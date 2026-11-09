//! Query endpoints: validate, rows, histogram, facets, group-by, schema and
//! autocomplete values.
//!
//! Every query here runs as the restricted reader user, and is built from
//! `trailscope_query::where_clause` plus fragments whose identifiers come from the
//! column registry. Values (including map keys, prefixes and cursors) are
//! always parameters.

use std::time::Instant;

use axum::Json;
use axum::extract::{Query, State};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use trailscope_query::ast::{CmpOp, Expr, Literal, Operand};
use trailscope_query::compile::{Param, ParamValue, Sql, where_clause};
use trailscope_query::registry::{Column, FUNCTIONS, FunctionDoc, Kind, Table, map_alias};
use trailscope_query::{Ctx, QueryInput, expr_to_chips, to_search};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::auth::{Auth, Role, Scope};
use crate::error::{ApiError, ApiResult};
use crate::{AppState, now_ms};

const MAX_RANGE_DAYS: i64 = 400;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum TableName {
    #[default]
    Requests,
    Events,
}

impl From<TableName> for Table {
    fn from(t: TableName) -> Table {
        match t {
            TableName::Requests => Table::Requests,
            TableName::Events => Table::Events,
        }
    }
}

/// The filter part shared by every query endpoint.
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Scope_ {
    pub project: Uuid,
    #[serde(default)]
    pub table: TableName,
    /// `{ "mode": "sql", "text": "status >= 500" }`, `{ "mode": "search", "text": "status:5xx" }`
    /// or `{ "mode": "filters", "chips": [...] }`.
    #[schema(value_type = Option<Object>)]
    pub query: Option<QueryInput>,
    /// Range start, ms since the epoch (default: 15 minutes ago).
    pub from: Option<i64>,
    /// Range end, ms since the epoch (default: now).
    pub to: Option<i64>,
    /// Restrict to one environment (the environment switcher).
    pub environment: Option<String>,
}

/// A parsed and authorized query scope.
pub struct Prepared {
    pub table: Table,
    pub project_key: String,
    pub from: i64,
    pub to: i64,
    pub filter: Option<Expr>,
    pub ctx: Ctx,
}

impl Prepared {
    pub fn where_sql(&self) -> Sql {
        where_clause(
            self.table,
            &self.project_key,
            self.from,
            self.to,
            self.filter.as_ref(),
        )
    }
}

pub fn env_condition(table: Table, env: &str) -> Option<Expr> {
    let col = table.column("environment")?;
    Some(Expr::Compare {
        left: Operand::Column(col),
        op: CmpOp::Eq,
        right: Operand::Literal(Literal::Str(env.to_string())),
    })
}

pub fn and(a: Option<Expr>, b: Option<Expr>) -> Option<Expr> {
    match (a, b) {
        (None, x) | (x, None) => x,
        (Some(Expr::And(mut xs)), Some(y)) => {
            xs.push(y);
            Some(Expr::And(xs))
        }
        (Some(x), Some(y)) => Some(Expr::And(vec![x, y])),
    }
}

pub async fn prepare(state: &AppState, auth: &Auth, s: &Scope_) -> ApiResult<Prepared> {
    auth.require(Role::Viewer, Scope::Read)?;
    auth.check_project(s.project)?;
    let project_key = state.project_key(s.project).await?;
    let now = now_ms();
    let to = s.to.unwrap_or(now);
    let from = s.from.unwrap_or(to - 15 * 60_000);
    if from > to {
        return Err(ApiError::bad_request(
            "the time range ends before it starts",
        ));
    }
    if to - from > MAX_RANGE_DAYS * 86_400_000 {
        return Err(ApiError::bad_request("the time range is too long"));
    }
    let table: Table = s.table.into();
    let ctx = Ctx {
        table,
        allow_sensitive: auth.can_view_bodies(),
        live: false,
    };
    let filter = match &s.query {
        Some(q) => q.to_expr(&ctx).map_err(ApiError::invalid_query)?,
        None => None,
    };
    let env = s
        .environment
        .as_deref()
        .filter(|e| !e.is_empty())
        .and_then(|e| env_condition(table, e));
    Ok(Prepared {
        table,
        project_key,
        from,
        to,
        filter: and(env, filter),
        ctx,
    })
}

/// Adds a named parameter (outside the `p` namespace used by the compiler).
pub fn param(sql: &mut Sql, name: &str, ty: &'static str, value: ParamValue) -> String {
    sql.params.push(Param {
        name: name.to_string(),
        ty,
        value,
    });
    format!("{{{name}:{ty}}}")
}

/// Output aliases that would collide with column names use this prefix;
/// `fetch_json` strips it.
pub const ALIAS_PREFIX: &str = "_c_";

pub struct JsonRows {
    pub rows: Vec<Map<String, Value>>,
    pub read_rows: u64,
    pub elapsed_ms: f64,
}

/// Runs `sql` as the reader and parses `JSONEachRow` output.
pub async fn fetch_json(state: &AppState, sql: &str, params: &[Param]) -> ApiResult<JsonRows> {
    let started = Instant::now();
    let mut q = state.ch.reader.query(sql);
    for p in params {
        q = match &p.value {
            ParamValue::Str(s) => q.param(&p.name, s),
            ParamValue::Int(i) => q.param(&p.name, i),
            ParamValue::UInt(u) => q.param(&p.name, u),
            ParamValue::Float(f) => q.param(&p.name, f),
            ParamValue::Strs(v) => q.param(&p.name, v),
        };
    }
    let mut cursor = q.fetch_bytes("JSONEachRow")?;
    let body = cursor.collect().await?;
    let read_rows = cursor.summary().and_then(|s| s.read_rows()).unwrap_or(0);
    let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
    metrics::histogram!("trailscope_query_seconds").record(elapsed_ms / 1000.0);
    let mut rows = Vec::new();
    for line in body.split(|b| *b == b'\n').filter(|l| !l.is_empty()) {
        let v: Map<String, Value> = serde_json::from_slice(line).map_err(ApiError::internal)?;
        rows.push(
            v.into_iter()
                .map(|(k, v)| match k.strip_prefix(ALIAS_PREFIX) {
                    Some(name) => (name.to_string(), v),
                    None => (k, v),
                })
                .collect(),
        );
    }
    Ok(JsonRows {
        rows,
        read_rows,
        elapsed_ms: (elapsed_ms * 10.0).round() / 10.0,
    })
}

// ---- validate ------------------------------------------------------------------

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ValidateRequest {
    #[serde(default)]
    pub table: TableName,
    #[schema(value_type = Object)]
    pub query: QueryInput,
    /// Validate for the live tail (only live columns allowed).
    #[serde(default)]
    pub live: bool,
    /// With a project and range, the response includes an estimate of rows to scan.
    pub project: Option<Uuid>,
    pub from: Option<i64>,
    pub to: Option<i64>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ValidateResponse {
    pub ok: bool,
    #[schema(value_type = Vec<Object>)]
    pub errors: Vec<trailscope_query::QueryError>,
    /// The query as chips, when it can be expressed as chips.
    #[schema(value_type = Option<Vec<Object>>)]
    pub chips: Option<Vec<trailscope_query::Chip>>,
    /// The query in search syntax, when representable.
    pub search: Option<String>,
    /// The canonical SQL WHERE text.
    pub sql: Option<String>,
    pub conditions: usize,
    pub estimated_rows: Option<u64>,
}

#[utoipa::path(post, path = "/api/v1/query/validate", tag = "query", request_body = ValidateRequest,
    responses((status = 200, body = ValidateResponse)))]
pub async fn validate(
    State(state): State<AppState>,
    auth: Auth,
    Json(req): Json<ValidateRequest>,
) -> ApiResult<Json<ValidateResponse>> {
    auth.require(Role::Viewer, Scope::Read)?;
    let ctx = Ctx {
        table: req.table.into(),
        allow_sensitive: auth.can_view_bodies(),
        live: req.live,
    };
    let expr = match req.query.to_expr(&ctx) {
        Ok(e) => e,
        Err(err) => {
            return Ok(Json(ValidateResponse {
                ok: false,
                errors: vec![err],
                chips: None,
                search: None,
                sql: None,
                conditions: 0,
                estimated_rows: None,
            }));
        }
    };
    let mut estimated_rows = None;
    if let (Some(project), false) = (req.project, req.live) {
        let scope = Scope_ {
            project,
            table: req.table,
            query: Some(req.query.clone()),
            from: req.from,
            to: req.to,
            environment: None,
        };
        if let Ok(p) = prepare(&state, &auth, &scope).await {
            let w = p.where_sql();
            let sql = format!(
                "EXPLAIN ESTIMATE SELECT 1 FROM {} WHERE {}",
                p.table.sql_name(),
                w.text
            );
            if let Ok(r) = fetch_json(&state, &sql, &w.params).await {
                estimated_rows = Some(
                    r.rows
                        .iter()
                        .filter_map(|row| row.get("rows").and_then(json_u64))
                        .sum(),
                );
            }
        }
    }
    Ok(Json(ValidateResponse {
        ok: true,
        errors: vec![],
        chips: expr.as_ref().map_or(Some(vec![]), expr_to_chips),
        search: expr
            .as_ref()
            .map_or(Some(String::new()), |e| to_search(e, &ctx)),
        sql: Some(expr.as_ref().map(|e| e.to_string()).unwrap_or_default()),
        conditions: expr.as_ref().map_or(0, Expr::condition_count),
        estimated_rows,
    }))
}

fn json_u64(v: &Value) -> Option<u64> {
    v.as_u64()
        .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
}

fn json_f64(v: &Value) -> Option<f64> {
    v.as_f64()
        .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
}

// ---- rows -------------------------------------------------------------------------

const DEFAULT_REQUEST_COLUMNS: &[&str] = &[
    "timestamp",
    "request_id",
    "trace_id",
    "environment",
    "service",
    "release",
    "method",
    "status",
    "route",
    "path",
    "duration_ms",
    "user_id",
    "client",
    "client_version",
    "error_type",
    "error_message",
    "error_fingerprint",
    "sample_rate",
];

const DEFAULT_EVENT_COLUMNS: &[&str] = &[
    "timestamp",
    "environment",
    "service",
    "release",
    "kind",
    "name",
    "level",
    "status",
    "message",
    "duration_ms",
    "request_id",
    "trace_id",
    "user_id",
    "attempt",
    "error_type",
    "error_message",
    "error_fingerprint",
];

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RowsRequest {
    #[serde(flatten)]
    pub scope: Scope_,
    /// Columns to return (default: a summary set). Bodies and headers need *can view bodies*.
    pub columns: Option<Vec<String>>,
    pub limit: Option<u32>,
    /// `nextCursor` from the previous page.
    pub cursor: Option<String>,
    /// `desc` (newest first, default) or `asc`.
    pub order: Option<String>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RowsResponse {
    #[schema(value_type = Vec<Object>)]
    pub rows: Vec<Map<String, Value>>,
    pub next_cursor: Option<String>,
    pub elapsed_ms: f64,
    pub read_rows: u64,
}

#[derive(Serialize, Deserialize)]
struct Cursor {
    t: i64,
    k: String,
}

/// The SELECT expression for a registry column.
pub fn select_expr(col: &Column) -> String {
    match col.kind {
        // Aliased under `_c_` (stripped in `fetch_json`): an alias that
        // shadows the column would change what WHERE compares against.
        Kind::Time => format!("toUnixTimestamp64Milli({0}) AS _c_{0}", col.name),
        Kind::U64 => format!("toString({0}) AS _c_{0}", col.name),
        _ => col.name.to_string(),
    }
}

/// Resolves requested output columns, enforcing the sensitive-column rule.
pub fn output_columns(
    table: Table,
    requested: Option<&[String]>,
    auth: &Auth,
) -> ApiResult<Vec<&'static Column>> {
    let defaults = match table {
        Table::Requests => DEFAULT_REQUEST_COLUMNS,
        Table::Events => DEFAULT_EVENT_COLUMNS,
    };
    let names: Vec<String> = match requested {
        Some(r) if !r.is_empty() => r.to_vec(),
        _ => defaults.iter().map(|s| s.to_string()).collect(),
    };
    let mut out = Vec::new();
    for n in &names {
        let col = table
            .column(n)
            .ok_or_else(|| ApiError::bad_request(format!("unknown column `{n}`")))?;
        if col.sensitive && !auth.can_view_bodies() {
            return Err(ApiError::forbidden(format!(
                "`{}` needs the *can view bodies* permission",
                col.name
            )));
        }
        if !out.iter().any(|c: &&Column| c.name == col.name) {
            out.push(col);
        }
    }
    if !out.iter().any(|c| c.name == "timestamp")
        && let Some(ts) = table.column("timestamp")
    {
        out.insert(0, ts);
    }
    Ok(out)
}

#[utoipa::path(post, path = "/api/v1/query/requests", tag = "query", request_body = RowsRequest,
    responses((status = 200, body = RowsResponse), (status = 422, description = "invalid query")))]
pub async fn rows(
    State(state): State<AppState>,
    auth: Auth,
    Json(req): Json<RowsRequest>,
) -> ApiResult<Json<RowsResponse>> {
    let p = prepare(&state, &auth, &req.scope).await?;
    let cols = output_columns(p.table, req.columns.as_deref(), &auth)?;
    let limit = req
        .limit
        .unwrap_or(state.config.query.default_limit)
        .clamp(1, state.config.query.max_limit);
    let asc = req.order.as_deref() == Some("asc");
    let (dir, cmp) = if asc { ("ASC", ">") } else { ("DESC", "<") };
    // A unique tiebreaker for keyset pagination.
    let key = match p.table {
        Table::Requests => "request_id",
        Table::Events => {
            "toString(cityHash64(service, name, message, request_id, attempt, timestamp))"
        }
    };
    let mut w = p.where_sql();
    if let Some(c) = &req.cursor {
        let cur: Cursor = URL_SAFE_NO_PAD
            .decode(c)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .ok_or_else(|| ApiError::bad_request("invalid cursor"))?;
        let t = param(&mut w, "cursor_t", "Int64", ParamValue::Int(cur.t));
        let k = param(&mut w, "cursor_k", "String", ParamValue::Str(cur.k));
        w.text = format!(
            "{} AND (timestamp, {key}) {cmp} (fromUnixTimestamp64Milli({t}), {k})",
            w.text
        );
    }
    let select: Vec<String> = cols.iter().map(|c| select_expr(c)).collect();
    let sql = format!(
        "SELECT {}, {key} AS _key FROM {} WHERE {} ORDER BY timestamp {dir}, _key {dir} LIMIT {}",
        select.join(", "),
        p.table.sql_name(),
        w.text,
        limit + 1
    );
    let mut res = fetch_json(&state, &sql, &w.params).await?;
    let more = res.rows.len() > limit as usize;
    res.rows.truncate(limit as usize);
    let next_cursor = if more {
        res.rows.last().and_then(|r| {
            let cur = Cursor {
                t: r.get("timestamp").and_then(Value::as_i64)?,
                k: r.get("_key").and_then(Value::as_str)?.to_string(),
            };
            serde_json::to_vec(&cur)
                .ok()
                .map(|b| URL_SAFE_NO_PAD.encode(b))
        })
    } else {
        None
    };
    for r in &mut res.rows {
        r.remove("_key");
    }
    Ok(Json(RowsResponse {
        rows: res.rows,
        next_cursor,
        elapsed_ms: res.elapsed_ms,
        read_rows: res.read_rows,
    }))
}

// ---- histogram ---------------------------------------------------------------------

const BUCKETS_SECS: &[i64] = &[
    1, 5, 10, 15, 30, 60, 120, 300, 600, 900, 1800, 3600, 7200, 10800, 21600, 43200, 86400,
];

/// The smallest "nice" bucket that yields at most `target` buckets.
pub fn bucket_secs(from: i64, to: i64, target: i64) -> i64 {
    let span = ((to - from) / 1000).max(1);
    BUCKETS_SECS
        .iter()
        .copied()
        .find(|b| span / b <= target)
        .unwrap_or(86400)
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct HistogramRequest {
    #[serde(flatten)]
    pub scope: Scope_,
    /// Approximate number of buckets (default 60, max 400).
    pub buckets: Option<i64>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct HistogramResponse {
    pub bucket_ms: i64,
    /// Series names, in stacking order.
    pub series: Vec<&'static str>,
    /// `[t, count per series...]`, counts scaled by sampling.
    pub points: Vec<Vec<f64>>,
    pub total: f64,
    pub elapsed_ms: f64,
}

#[utoipa::path(post, path = "/api/v1/query/histogram", tag = "query", request_body = HistogramRequest,
    responses((status = 200, body = HistogramResponse)))]
pub async fn histogram(
    State(state): State<AppState>,
    auth: Auth,
    Json(req): Json<HistogramRequest>,
) -> ApiResult<Json<HistogramResponse>> {
    let p = prepare(&state, &auth, &req.scope).await?;
    let target = req.buckets.unwrap_or(60).clamp(1, 400);
    let b = bucket_secs(p.from, p.to, target);
    let mut w = p.where_sql();
    let bucket = param(&mut w, "bucket", "Int64", ParamValue::Int(b));
    let (series, sums): (Vec<&'static str>, &str) = match p.table {
        Table::Requests => (
            vec!["2xx", "3xx", "4xx", "5xx"],
            "round(sumIf(1 / greatest(sample_rate, 0.0001), status < 300)) AS s0, \
             round(sumIf(1 / greatest(sample_rate, 0.0001), status >= 300 AND status < 400)) AS s1, \
             round(sumIf(1 / greatest(sample_rate, 0.0001), status >= 400 AND status < 500)) AS s2, \
             round(sumIf(1 / greatest(sample_rate, 0.0001), status >= 500)) AS s3",
        ),
        Table::Events => (
            vec!["ok", "warn", "error"],
            "countIf(status = 'ok') AS s0, countIf(status = 'warn') AS s1, countIf(status = 'error') AS s2",
        ),
    };
    let sql = format!(
        "SELECT toUnixTimestamp(toStartOfInterval(timestamp, toIntervalSecond({bucket}))) * 1000 AS t, {sums} \
         FROM {} WHERE {} GROUP BY t ORDER BY t",
        p.table.sql_name(),
        w.text
    );
    let res = fetch_json(&state, &sql, &w.params).await?;
    let mut total = 0.0;
    let points = res
        .rows
        .iter()
        .map(|r| {
            let mut pt = vec![r.get("t").and_then(json_f64).unwrap_or(0.0)];
            for i in 0..series.len() {
                let v = r.get(&format!("s{i}")).and_then(json_f64).unwrap_or(0.0);
                total += v;
                pt.push(v);
            }
            pt
        })
        .collect();
    Ok(Json(HistogramResponse {
        bucket_ms: b * 1000,
        series,
        points,
        total,
        elapsed_ms: res.elapsed_ms,
    }))
}

// ---- facets and group-by ----------------------------------------------------------

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct FieldRef {
    pub column: String,
    /// For map columns (`attributes`, `correlation`).
    pub key: Option<String>,
}

/// The SQL for a facet/group field, and its display name.
fn field_sql(
    table: Table,
    f: &FieldRef,
    w: &mut Sql,
    n: usize,
    auth: &Auth,
    purpose: &str,
) -> ApiResult<(String, String)> {
    let name = map_alias(&f.column).unwrap_or(&f.column);
    let col = table
        .column(name)
        .ok_or_else(|| ApiError::bad_request(format!("unknown column `{}`", f.column)))?;
    if col.sensitive && !auth.can_view_bodies() {
        return Err(ApiError::forbidden(format!(
            "`{}` needs the *can view bodies* permission",
            col.name
        )));
    }
    if col.kind == Kind::Map {
        let key = f
            .key
            .as_deref()
            .filter(|k| !k.is_empty() && k.chars().count() <= 128)
            .ok_or_else(|| ApiError::bad_request(format!("`{}` needs a key", col.name)))?;
        let k = param(
            w,
            &format!("fk{n}"),
            "String",
            ParamValue::Str(key.to_string()),
        );
        return Ok((format!("{}[{k}]", col.name), format!("{}.{key}", col.name)));
    }
    if !(col.groupable || col.facetable) {
        return Err(ApiError::bad_request(format!(
            "`{}` can't be used for {purpose}",
            col.name
        )));
    }
    let expr = match col.kind {
        Kind::Time => format!("toUnixTimestamp64Milli({})", col.name),
        _ => format!("toString({})", col.name),
    };
    Ok((expr, col.name.to_string()))
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct FacetsRequest {
    #[serde(flatten)]
    pub scope: Scope_,
    pub fields: Vec<FieldRef>,
    /// Values per field (default 10, max 50).
    pub limit: Option<u32>,
}

#[derive(Serialize, ToSchema)]
pub struct FacetValue {
    pub value: String,
    pub count: u64,
}

#[derive(Serialize, ToSchema)]
pub struct Facet {
    pub field: String,
    pub values: Vec<FacetValue>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct FacetsResponse {
    pub facets: Vec<Facet>,
    pub elapsed_ms: f64,
}

#[utoipa::path(post, path = "/api/v1/query/facets", tag = "query", request_body = FacetsRequest,
    responses((status = 200, body = FacetsResponse)))]
pub async fn facets(
    State(state): State<AppState>,
    auth: Auth,
    Json(req): Json<FacetsRequest>,
) -> ApiResult<Json<FacetsResponse>> {
    if req.fields.len() > 20 {
        return Err(ApiError::bad_request("ask for at most 20 facets at once"));
    }
    let p = prepare(&state, &auth, &req.scope).await?;
    let limit = req.limit.unwrap_or(10).clamp(1, 50);
    let started = Instant::now();
    let mut jobs = Vec::new();
    for (i, f) in req.fields.iter().enumerate() {
        let mut w = p.where_sql();
        let (expr, name) = field_sql(p.table, f, &mut w, i, &auth, "facets")?;
        let sql = format!(
            "SELECT {expr} AS value, count() AS count FROM {} WHERE {} GROUP BY value ORDER BY count DESC LIMIT {limit}",
            p.table.sql_name(),
            w.text
        );
        let state = state.clone();
        jobs.push(async move {
            let r = fetch_json(&state, &sql, &w.params).await?;
            let values = r
                .rows
                .iter()
                .map(|row| FacetValue {
                    value: match row.get("value") {
                        Some(Value::String(s)) => s.clone(),
                        Some(v) => v.to_string(),
                        None => String::new(),
                    },
                    count: row.get("count").and_then(json_u64).unwrap_or(0),
                })
                .collect();
            Ok::<_, ApiError>(Facet {
                field: name,
                values,
            })
        });
    }
    let facets = futures::future::try_join_all(jobs).await?;
    Ok(Json(FacetsResponse {
        facets,
        elapsed_ms: (started.elapsed().as_secs_f64() * 10_000.0).round() / 10.0,
    }))
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct GroupRequest {
    #[serde(flatten)]
    pub scope: Scope_,
    pub by: FieldRef,
    /// Groups to return (default 50, max 500).
    pub limit: Option<u32>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct GroupRow {
    pub value: String,
    /// Estimated count (scaled by sampling).
    pub count: f64,
    /// Share of 5xx (requests) or failed events.
    pub error_rate: f64,
    /// Share of 4xx and 5xx (requests only).
    pub client_error_rate: Option<f64>,
    pub p50: f64,
    pub p95: f64,
    pub p99: f64,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct GroupResponse {
    pub field: String,
    pub groups: Vec<GroupRow>,
    pub elapsed_ms: f64,
}

#[utoipa::path(post, path = "/api/v1/query/group", tag = "query", request_body = GroupRequest,
    responses((status = 200, body = GroupResponse)))]
pub async fn group(
    State(state): State<AppState>,
    auth: Auth,
    Json(req): Json<GroupRequest>,
) -> ApiResult<Json<GroupResponse>> {
    let p = prepare(&state, &auth, &req.scope).await?;
    let limit = req.limit.unwrap_or(50).clamp(1, 500);
    let mut w = p.where_sql();
    let (expr, name) = field_sql(p.table, &req.by, &mut w, 0, &auth, "grouping")?;
    let metrics = match p.table {
        Table::Requests => {
            "sum(1 / greatest(sample_rate, 0.0001)) AS count, countIf(status >= 500) / count() AS error_rate, \
             countIf(status >= 400) / count() AS client_error_rate, \
             quantiles(0.5, 0.95, 0.99)(duration_ms) AS q"
        }
        Table::Events => {
            "count() AS count, countIf(status = 'error') / count() AS error_rate, \
             NULL AS client_error_rate, quantiles(0.5, 0.95, 0.99)(duration_ms) AS q"
        }
    };
    let sql = format!(
        "SELECT {expr} AS value, {metrics} FROM {} WHERE {} GROUP BY value ORDER BY count DESC LIMIT {limit}",
        p.table.sql_name(),
        w.text
    );
    let r = fetch_json(&state, &sql, &w.params).await?;
    let round = |x: f64| (x * 100.0).round() / 100.0;
    let rate = |x: f64| (x * 10_000.0).round() / 10_000.0;
    let groups = r
        .rows
        .iter()
        .map(|row| {
            let q: Vec<f64> = row
                .get("q")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(json_f64).collect())
                .unwrap_or_default();
            GroupRow {
                value: match row.get("value") {
                    Some(Value::String(s)) => s.clone(),
                    Some(v) => v.to_string(),
                    None => String::new(),
                },
                count: row.get("count").and_then(json_f64).unwrap_or(0.0).round(),
                error_rate: rate(row.get("error_rate").and_then(json_f64).unwrap_or(0.0)),
                client_error_rate: row.get("client_error_rate").and_then(json_f64).map(rate),
                p50: round(q.first().copied().unwrap_or(0.0)),
                p95: round(q.get(1).copied().unwrap_or(0.0)),
                p99: round(q.get(2).copied().unwrap_or(0.0)),
            }
        })
        .collect();
    Ok(Json(GroupResponse {
        field: name,
        groups,
        elapsed_ms: r.elapsed_ms,
    }))
}

// ---- schema and autocomplete --------------------------------------------------------

#[derive(Serialize, ToSchema)]
pub struct SchemaResponse {
    #[schema(value_type = Object)]
    pub tables: Value,
    #[schema(value_type = Vec<Object>)]
    pub functions: &'static [FunctionDoc],
    pub keywords: Vec<&'static str>,
}

#[utoipa::path(get, path = "/api/v1/schema", tag = "query", responses((status = 200, body = SchemaResponse)))]
pub async fn schema(auth: Auth) -> ApiResult<Json<SchemaResponse>> {
    auth.require(Role::Viewer, Scope::Read)?;
    Ok(Json(SchemaResponse {
        tables: json!({
            "requests": Table::Requests.columns(),
            "events": Table::Events.columns(),
        }),
        functions: FUNCTIONS,
        keywords: vec![
            "AND", "OR", "NOT", "IN", "BETWEEN", "LIKE", "ILIKE", "IS", "NULL", "INTERVAL",
            "SECOND", "MINUTE", "HOUR", "DAY", "TRUE", "FALSE",
        ],
    }))
}

#[derive(Deserialize, ToSchema, utoipa::IntoParams)]
#[serde(rename_all = "camelCase")]
pub struct ValuesQuery {
    pub project: Uuid,
    #[serde(default)]
    pub table: TableName,
    pub column: String,
    pub key: Option<String>,
    #[serde(default)]
    pub prefix: String,
    pub environment: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct ValuesResponse {
    pub values: Vec<FacetValue>,
}

/// Top values of a column over the last hour, for autocomplete.
#[utoipa::path(get, path = "/api/v1/values", tag = "query", params(ValuesQuery), responses((status = 200, body = ValuesResponse)))]
pub async fn values(
    State(state): State<AppState>,
    auth: Auth,
    Query(q): Query<ValuesQuery>,
) -> ApiResult<Json<ValuesResponse>> {
    let scope = Scope_ {
        project: q.project,
        table: q.table,
        query: None,
        from: Some(now_ms() - 3_600_000),
        to: None,
        environment: q.environment.clone(),
    };
    let p = prepare(&state, &auth, &scope).await?;
    let mut w = p.where_sql();
    let (expr, _) = field_sql(
        p.table,
        &FieldRef {
            column: q.column.clone(),
            key: q.key.clone(),
        },
        &mut w,
        0,
        &auth,
        "autocomplete",
    )
    .or_else(|e| {
        // Any filterable string column works for autocomplete, not just facets.
        let col = p
            .table
            .column(&q.column)
            .filter(|c| c.kind == Kind::String && (!c.sensitive || auth.can_view_bodies()));
        col.map(|c| (format!("toString({})", c.name), c.name.to_string()))
            .ok_or(e)
    })?;
    let prefix = param(
        &mut w,
        "prefix",
        "String",
        ParamValue::Str(q.prefix.chars().take(200).collect()),
    );
    let sql = format!(
        "SELECT {expr} AS value, count() AS count FROM {} WHERE {} AND startsWith({expr}, {prefix}) AND notEmpty(value) \
         GROUP BY value ORDER BY count DESC LIMIT 20",
        p.table.sql_name(),
        w.text
    );
    let r = fetch_json(&state, &sql, &w.params).await?;
    Ok(Json(ValuesResponse {
        values: r
            .rows
            .iter()
            .map(|row| FacetValue {
                value: row
                    .get("value")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                count: row.get("count").and_then(json_u64).unwrap_or(0),
            })
            .collect(),
    }))
}

#[derive(Deserialize, ToSchema, utoipa::IntoParams)]
#[serde(rename_all = "camelCase")]
pub struct KeysQuery {
    pub project: Uuid,
    #[serde(default)]
    pub table: TableName,
    /// `attributes` or `correlation`.
    pub map: String,
}

/// Map keys seen in the last hour (for `attr['…']` autocomplete).
#[utoipa::path(get, path = "/api/v1/keys", tag = "query", params(KeysQuery), responses((status = 200, body = ValuesResponse)))]
pub async fn map_keys(
    State(state): State<AppState>,
    auth: Auth,
    Query(q): Query<KeysQuery>,
) -> ApiResult<Json<ValuesResponse>> {
    let scope = Scope_ {
        project: q.project,
        table: q.table,
        query: None,
        from: Some(now_ms() - 3_600_000),
        to: None,
        environment: None,
    };
    let p = prepare(&state, &auth, &scope).await?;
    let col = map_alias(&q.map)
        .and_then(|m| p.table.column(m))
        .filter(|c| c.kind == Kind::Map && (!c.sensitive || auth.can_view_bodies()))
        .ok_or_else(|| ApiError::bad_request("map must be attributes or correlation"))?;
    let w = p.where_sql();
    let sql = format!(
        "SELECT arrayJoin(mapKeys({})) AS value, count() AS count FROM {} WHERE {} GROUP BY value ORDER BY count DESC LIMIT 50",
        col.name,
        p.table.sql_name(),
        w.text
    );
    let r = fetch_json(&state, &sql, &w.params).await?;
    Ok(Json(ValuesResponse {
        values: r
            .rows
            .iter()
            .map(|row| FacetValue {
                value: row
                    .get("value")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                count: row.get("count").and_then(json_u64).unwrap_or(0),
            })
            .collect(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buckets_are_nice() {
        assert_eq!(bucket_secs(0, 15 * 60_000, 60), 15);
        assert_eq!(bucket_secs(0, 3_600_000, 60), 60);
        assert_eq!(bucket_secs(0, 86_400_000, 60), 1800);
        assert_eq!(bucket_secs(0, 30 * 86_400_000, 60), 43200);
        assert_eq!(bucket_secs(0, 365 * 86_400_000, 60), 86400);
    }
}
