//! The request inspector: one request with its spans, related events and
//! error group; cURL export; and replay against configured targets.

use std::time::Instant;

use axum::Json;
use axum::extract::{Path, Query, State};
use logbox_query::compile::{ParamValue, Sql};
use logbox_query::registry::Table;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::AppState;
use crate::auth::{Auth, Role, Scope};
use crate::error::{ApiError, ApiResult};
use crate::query::{fetch_json, param, select_expr};

#[derive(Deserialize, utoipa::IntoParams)]
pub struct ProjectParam {
    pub project: Uuid,
}

const REDACTED: &str = "[REDACTED]";
/// Headers that are never replayed or exported with their values.
const AUTH_HEADERS: &[&str] = &[
    "authorization",
    "cookie",
    "x-api-key",
    "proxy-authorization",
];
/// Hop-by-hop and transport headers that don't belong in a replay or cURL.
const SKIP_HEADERS: &[&str] = &[
    "host",
    "content-length",
    "connection",
    "keep-alive",
    "transfer-encoding",
    "te",
    "trailer",
    "upgrade",
    "accept-encoding",
    "x-request-id",
    "traceparent",
    "tracestate",
];

async fn request_row(
    state: &AppState,
    auth: &Auth,
    project: Uuid,
    request_id: &str,
) -> ApiResult<Map<String, Value>> {
    auth.require(Role::Viewer, Scope::Read)?;
    auth.check_project(project)?;
    let key = state.project_key(project).await?;
    if request_id.is_empty() || request_id.len() > 128 {
        return Err(ApiError::not_found("request"));
    }
    let cols: Vec<String> = Table::Requests
        .columns()
        .iter()
        .filter(|c| !c.sensitive || auth.can_view_bodies())
        .map(select_expr)
        .collect();
    let mut sql = Sql::default();
    let pid = param(&mut sql, "pid", "String", ParamValue::Str(key));
    let rid = param(
        &mut sql,
        "rid",
        "String",
        ParamValue::Str(request_id.to_string()),
    );
    let text = format!(
        "SELECT {} FROM requests WHERE project_id = {pid} AND request_id = {rid} ORDER BY timestamp DESC LIMIT 1",
        cols.join(", ")
    );
    let mut r = fetch_json(state, &text, &sql.params).await?;
    r.rows.pop().ok_or_else(|| ApiError::not_found("request"))
}

fn s<'a>(row: &'a Map<String, Value>, k: &str) -> &'a str {
    row.get(k).and_then(Value::as_str).unwrap_or("")
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RequestDetail {
    #[schema(value_type = Object)]
    pub request: Map<String, Value>,
    #[schema(value_type = Vec<Object>)]
    pub spans: Vec<Map<String, Value>>,
    #[schema(value_type = Vec<Object>)]
    pub events: Vec<Map<String, Value>>,
    #[schema(value_type = Option<Object>)]
    pub error_group: Option<Map<String, Value>>,
    /// True when bodies and headers were withheld (no *can view bodies*).
    pub bodies_hidden: bool,
}

#[utoipa::path(get, path = "/api/v1/requests/{request_id}", tag = "inspector", params(ProjectParam),
    responses((status = 200, body = RequestDetail), (status = 404)))]
pub async fn detail(
    State(state): State<AppState>,
    auth: Auth,
    Path(request_id): Path<String>,
    Query(q): Query<ProjectParam>,
) -> ApiResult<Json<RequestDetail>> {
    let row = request_row(&state, &auth, q.project, &request_id).await?;
    let key = state.project_key(q.project).await?;
    let ts = row.get("timestamp").and_then(Value::as_i64).unwrap_or(0);
    let trace = s(&row, "trace_id").to_string();
    let fp = s(&row, "error_fingerprint").to_string();
    let env = s(&row, "environment").to_string();

    let mut sp = Sql::default();
    let pid = param(&mut sp, "pid", "String", ParamValue::Str(key.clone()));
    let rid = param(
        &mut sp,
        "rid",
        "String",
        ParamValue::Str(request_id.clone()),
    );
    let tid = param(&mut sp, "tid", "String", ParamValue::Str(trace.clone()));
    let lo = param(
        &mut sp,
        "lo",
        "Int64",
        ParamValue::Int((ts - 3_600_000) * 1000),
    );
    let hi = param(
        &mut sp,
        "hi",
        "Int64",
        ParamValue::Int((ts + 3_600_000) * 1000),
    );
    let spans_sql = format!(
        "SELECT trace_id, span_id, parent_span_id, request_id, service, kind, name, \
           toUnixTimestamp64Micro(start) / 1000 AS _c_start, duration_ms, status, attributes \
         FROM spans WHERE project_id = {pid} AND (request_id = {rid} OR (notEmpty({tid}) AND trace_id = {tid})) \
           AND start BETWEEN fromUnixTimestamp64Micro({lo}) AND fromUnixTimestamp64Micro({hi}) \
         ORDER BY start LIMIT 500"
    );

    let mut ev = Sql::default();
    let pid2 = param(&mut ev, "pid", "String", ParamValue::Str(key.clone()));
    let rid2 = param(
        &mut ev,
        "rid",
        "String",
        ParamValue::Str(request_id.clone()),
    );
    let tid2 = param(&mut ev, "tid", "String", ParamValue::Str(trace));
    let lo2 = param(&mut ev, "lo", "Int64", ParamValue::Int(ts - 3_600_000));
    let hi2 = param(&mut ev, "hi", "Int64", ParamValue::Int(ts + 6 * 3_600_000));
    let event_cols: Vec<String> = Table::Events.columns().iter().map(select_expr).collect();
    let events_sql = format!(
        "SELECT {} FROM events WHERE project_id = {pid2} AND (request_id = {rid2} OR (notEmpty({tid2}) AND trace_id = {tid2})) \
           AND timestamp BETWEEN fromUnixTimestamp64Milli({lo2}) AND fromUnixTimestamp64Milli({hi2}) \
         ORDER BY timestamp LIMIT 200",
        event_cols.join(", ")
    );

    let (spans, events, group) = tokio::join!(
        fetch_json(&state, &spans_sql, &sp.params),
        fetch_json(&state, &events_sql, &ev.params),
        error_group(&state, &key, &env, &fp),
    );
    Ok(Json(RequestDetail {
        request: row,
        spans: spans?.rows,
        events: events?.rows,
        error_group: group?,
        bodies_hidden: !auth.can_view_bodies(),
    }))
}

/// Summary of an error group, with users affected and the first-seen release.
pub async fn error_group(
    state: &AppState,
    project_key: &str,
    env: &str,
    fingerprint: &str,
) -> ApiResult<Option<Map<String, Value>>> {
    let Ok(fp) = fingerprint.parse::<u64>() else {
        return Ok(None);
    };
    if fp == 0 {
        return Ok(None);
    }
    let mut sql = Sql::default();
    let pid = param(
        &mut sql,
        "pid",
        "String",
        ParamValue::Str(project_key.to_string()),
    );
    let env = param(&mut sql, "env", "String", ParamValue::Str(env.to_string()));
    let f = param(&mut sql, "fp", "UInt64", ParamValue::UInt(fp));
    let text = format!(
        "SELECT toString(error_fingerprint) AS fingerprint, any(environment) AS _c_environment, \
           toUnixTimestamp64Milli(min(first_seen)) AS _c_first_seen, toUnixTimestamp64Milli(max(last_seen)) AS _c_last_seen, \
           sum(count) AS _c_count, anyLast(error_type) AS _c_error_type, anyLast(sample_message) AS message, \
           anyLast(service) AS _c_service, anyLast(route) AS _c_route, anyLast(source) AS _c_source, \
           argMaxMerge(sample_request_id) AS _c_sample_request_id, argMinMerge(first_release) AS _c_first_release, \
           uniqMerge(users) AS _c_users \
         FROM error_groups WHERE project_id = {pid} AND error_fingerprint = {f} AND (empty({env}) OR environment = {env}) \
         GROUP BY error_fingerprint"
    );
    let mut r = fetch_json(state, &text, &sql.params).await?;
    Ok(r.rows.pop())
}

// ---- cURL -------------------------------------------------------------------------

#[derive(Deserialize, utoipa::IntoParams)]
pub struct CurlQuery {
    pub project: Uuid,
    /// Base URL to target, e.g. `https://staging.example.com` (default: the original host).
    pub target: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct CurlResponse {
    pub command: String,
    /// Redacted values that appear as placeholders, e.g. `<authorization>`.
    pub placeholders: Vec<String>,
}

/// Single-quotes a string for POSIX shells.
pub fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

fn headers_of(row: &Map<String, Value>, key: &str) -> Vec<(String, String)> {
    row.get(key)
        .and_then(Value::as_object)
        .map(|m| {
            let mut v: Vec<(String, String)> = m
                .iter()
                .map(|(k, v)| {
                    (
                        k.to_ascii_lowercase(),
                        v.as_str().unwrap_or_default().to_string(),
                    )
                })
                .collect();
            v.sort();
            v
        })
        .unwrap_or_default()
}

pub fn build_curl(row: &Map<String, Value>, target: Option<&str>) -> CurlResponse {
    let headers = headers_of(row, "req_headers");
    let base = target
        .map(|t| t.trim_end_matches('/').to_string())
        .or_else(|| {
            headers
                .iter()
                .find(|(k, _)| k == "host")
                .map(|(_, h)| format!("https://{h}"))
        })
        .unwrap_or_else(|| "http://localhost".to_string());
    let path = s(row, "path");
    let query = s(row, "query");
    let url = if query.is_empty() {
        format!("{base}{path}")
    } else {
        format!("{base}{path}?{query}")
    };
    let mut parts = vec![format!(
        "curl -X {} {}",
        s(row, "method"),
        shell_quote(&url)
    )];
    let mut placeholders = Vec::new();
    for (k, v) in &headers {
        if SKIP_HEADERS.contains(&k.as_str()) {
            continue;
        }
        let value = if v == REDACTED || AUTH_HEADERS.contains(&k.as_str()) {
            let ph = format!("<{k}>");
            placeholders.push(ph.clone());
            ph
        } else {
            v.clone()
        };
        parts.push(format!("-H {}", shell_quote(&format!("{k}: {value}"))));
    }
    let body = s(row, "req_body");
    if !body.is_empty() {
        let body = body.replace(&format!("\"{REDACTED}\""), "\"<redacted>\"");
        if body.contains("<redacted>") {
            placeholders.push("<redacted>".into());
        }
        parts.push(format!("--data-raw {}", shell_quote(&body)));
    }
    CurlResponse {
        command: parts.join(" \\\n  "),
        placeholders,
    }
}

#[utoipa::path(get, path = "/api/v1/requests/{request_id}/curl", tag = "inspector", params(CurlQuery),
    responses((status = 200, body = CurlResponse)))]
pub async fn curl(
    State(state): State<AppState>,
    auth: Auth,
    Path(request_id): Path<String>,
    Query(q): Query<CurlQuery>,
) -> ApiResult<Json<CurlResponse>> {
    let row = request_row(&state, &auth, q.project, &request_id).await?;
    if let Some(t) = &q.target {
        crate::projects::normalize_base_url(t)?;
    }
    Ok(Json(build_curl(&row, q.target.as_deref())))
}

// ---- replay ------------------------------------------------------------------------

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReplayRequest {
    pub project: Uuid,
    pub target_id: Uuid,
    /// Extra headers (e.g. a fresh `authorization`). Auth headers from the
    /// original request are never replayed.
    #[serde(default)]
    pub headers: std::collections::BTreeMap<String, String>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReplayResult {
    pub id: Uuid,
    pub url: String,
    pub status: Option<u16>,
    #[schema(value_type = Object)]
    pub headers: Value,
    pub body: String,
    pub body_truncated: bool,
    pub duration_ms: f64,
    pub error: Option<String>,
}

const REPLAY_BODY_LIMIT: usize = 64 * 1024;

#[utoipa::path(post, path = "/api/v1/requests/{request_id}/replay", tag = "inspector", request_body = ReplayRequest,
    responses((status = 200, body = ReplayResult)))]
pub async fn replay(
    State(state): State<AppState>,
    auth: Auth,
    Path(request_id): Path<String>,
    Json(req): Json<ReplayRequest>,
) -> ApiResult<Json<ReplayResult>> {
    auth.require(Role::Member, Scope::Write)?;
    if !auth.can_view_bodies() {
        return Err(ApiError::forbidden(
            "replaying needs the *can view bodies* permission",
        ));
    }
    let row = request_row(&state, &auth, req.project, &request_id).await?;
    let target: (String,) =
        sqlx::query_as("SELECT base_url FROM replay_targets WHERE id = $1 AND project_id = $2")
            .bind(req.target_id)
            .bind(req.project)
            .fetch_optional(&state.pool)
            .await?
            .ok_or_else(|| ApiError::not_found("replay target"))?;
    if row
        .get("req_body_truncated")
        .and_then(Value::as_u64)
        .unwrap_or(0)
        == 1
    {
        return Err(ApiError::bad_request(
            "the stored request body was truncated, so it can't be replayed faithfully",
        ));
    }
    let method = reqwest::Method::from_bytes(s(&row, "method").as_bytes())
        .map_err(|_| ApiError::bad_request("unsupported method"))?;
    let (path, query) = (s(&row, "path"), s(&row, "query"));
    let url = if query.is_empty() {
        format!("{}{path}", target.0)
    } else {
        format!("{}{path}?{query}", target.0)
    };

    let mut builder = state.http.request(method.clone(), &url);
    for (k, v) in headers_of(&row, "req_headers") {
        if SKIP_HEADERS.contains(&k.as_str()) || AUTH_HEADERS.contains(&k.as_str()) || v == REDACTED
        {
            continue;
        }
        builder = builder.header(k, v);
    }
    for (k, v) in &req.headers {
        let k = k.trim().to_ascii_lowercase();
        if k.is_empty() || SKIP_HEADERS.contains(&k.as_str()) {
            continue;
        }
        builder = builder.header(k, v);
    }
    builder = builder.header("x-logbox-replay", request_id.clone());
    let body = s(&row, "req_body");
    if !body.is_empty() {
        builder = builder.body(body.to_string());
    }

    let started = Instant::now();
    let (status, headers, body, truncated, error) = match builder.send().await {
        Ok(resp) => {
            let status = resp.status().as_u16();
            let headers: Map<String, Value> = resp
                .headers()
                .iter()
                .map(|(k, v)| (k.to_string(), json!(v.to_str().unwrap_or(""))))
                .collect();
            let bytes = resp.bytes().await.unwrap_or_default();
            let truncated = bytes.len() > REPLAY_BODY_LIMIT;
            let body =
                String::from_utf8_lossy(&bytes[..bytes.len().min(REPLAY_BODY_LIMIT)]).into_owned();
            (Some(status), Value::Object(headers), body, truncated, None)
        }
        Err(e) => (None, json!({}), String::new(), false, Some(e.to_string())),
    };
    let duration_ms = (started.elapsed().as_secs_f64() * 10_000.0).round() / 10.0;
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO replays (id, project_id, request_id, target_id, user_id, method, url, response_status, response_headers, response_body, duration_ms, error) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
    )
    .bind(id)
    .bind(req.project)
    .bind(&request_id)
    .bind(req.target_id)
    .bind(auth.user.id)
    .bind(method.as_str())
    .bind(&url)
    .bind(status.map(i32::from))
    .bind(sqlx::types::Json(&headers))
    .bind(&body)
    .bind(duration_ms)
    .bind(&error)
    .execute(&state.pool)
    .await?;
    Ok(Json(ReplayResult {
        id,
        url,
        status,
        headers,
        body,
        body_truncated: truncated,
        duration_ms,
        error,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curl_uses_placeholders_and_quotes() {
        let row: Map<String, Value> = serde_json::from_value(json!({
            "method": "POST",
            "path": "/projects/prj_1/tasks",
            "query": "dry_run=true",
            "req_headers": {"authorization": "[REDACTED]", "content-type": "application/json", "host": "api.example.com", "x-note": "it's"},
            "req_body": "{\"title\":\"x\",\"password\":\"[REDACTED]\"}"
        }))
        .unwrap();
        let c = build_curl(&row, Some("https://staging.example.com/"));
        assert!(c.command.starts_with(
            "curl -X POST 'https://staging.example.com/projects/prj_1/tasks?dry_run=true'"
        ));
        assert!(c.command.contains("-H 'authorization: <authorization>'"));
        assert!(c.command.contains("-H 'x-note: it'\\''s'"));
        assert!(!c.command.contains("host:"));
        assert!(c.command.contains("\"password\":\"<redacted>\""));
        assert_eq!(c.placeholders, vec!["<authorization>", "<redacted>"]);
        let default_host = build_curl(&row, None);
        assert!(
            default_host
                .command
                .contains("'https://api.example.com/projects")
        );
    }
}
