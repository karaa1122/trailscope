//! Overview tiles and the route heatmap, served from the `route_minute`
//! rollup so they never scan raw rows.

use std::collections::BTreeMap;

use axum::Json;
use axum::extract::{Query, State};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use trailscope_query::compile::{ParamValue, Sql};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::AppState;
use crate::auth::{Auth, Role, Scope};
use crate::error::ApiResult;
use crate::query::{fetch_json, param};

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct OverviewQuery {
    pub project: Uuid,
    pub environment: Option<String>,
    pub service: Option<String>,
    /// Window in minutes (default 30, max 1440).
    pub minutes: Option<u32>,
}

pub fn num(row: &Map<String, Value>, key: &str) -> f64 {
    match row.get(key) {
        Some(Value::Number(n)) => n.as_f64().unwrap_or(0.0),
        Some(Value::String(s)) => s.parse().unwrap_or(0.0),
        _ => 0.0,
    }
}

/// Scope SQL for rollup queries: project, optional environment and service,
/// and minutes since `now - window`.
fn rollup_scope(
    sql: &mut Sql,
    project: &str,
    env: Option<&str>,
    service: Option<&str>,
    window_minutes: u32,
) -> String {
    let pid = param(sql, "pid", "String", ParamValue::Str(project.to_string()));
    let m = param(
        sql,
        "win",
        "Int64",
        ParamValue::Int(i64::from(window_minutes)),
    );
    let mut text =
        format!("project_id = {pid} AND minute >= toStartOfMinute(now()) - toIntervalMinute({m})");
    if let Some(e) = env.filter(|e| !e.is_empty()) {
        let e = param(sql, "env", "String", ParamValue::Str(e.to_string()));
        text.push_str(&format!(" AND environment = {e}"));
    }
    if let Some(s) = service.filter(|s| !s.is_empty()) {
        let s = param(sql, "svc", "String", ParamValue::Str(s.to_string()));
        text.push_str(&format!(" AND service = {s}"));
    }
    text
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Tile {
    pub value: f64,
    /// The same metric over the preceding window, for the delta.
    pub previous: f64,
    /// One value per minute over the window.
    pub series: Vec<f64>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct OverviewResponse {
    pub minutes: u32,
    /// Minute timestamps (ms) for the series.
    pub times: Vec<i64>,
    pub requests_per_min: Tile,
    /// Share of 5xx, 0..1.
    pub error_rate: Tile,
    /// Milliseconds.
    pub p95: Tile,
    /// Requests slower than the project's `slowMs`.
    pub slow: Tile,
    pub slow_ms: u32,
}

/// Tiles for the live tail: requests/min, error rate, p95 and slow requests.
#[utoipa::path(get, path = "/api/v1/overview", tag = "overview", params(OverviewQuery),
    responses((status = 200, body = OverviewResponse)))]
pub async fn overview(
    State(state): State<AppState>,
    auth: Auth,
    Query(q): Query<OverviewQuery>,
) -> ApiResult<Json<OverviewResponse>> {
    auth.require(Role::Viewer, Scope::Read)?;
    auth.check_project(q.project)?;
    let key = state.project_key(q.project).await?;
    let minutes = q.minutes.unwrap_or(30).clamp(5, 1440);
    let slow_ms = project_slow_ms(&state, q.project).await;

    // Per minute, over two windows (current and previous, for deltas).
    let mut sql = Sql::default();
    let scope = rollup_scope(
        &mut sql,
        &key,
        q.environment.as_deref(),
        q.service.as_deref(),
        minutes * 2,
    );
    let text = format!(
        "SELECT toUnixTimestamp(minute) * 1000 AS t, sumMerge(weighted) AS req, countMerge(requests) AS raw, \
           countIfMerge(errors_5xx) AS e5, countIfMerge(slow) AS slow, \
           quantilesTDigestMerge(0.5, 0.95, 0.99)(latency)[2] AS p95 \
         FROM route_minute WHERE {scope} GROUP BY t ORDER BY t"
    );

    // Window totals, so p95 is a true quantile rather than an average of minutes.
    let mut sql2 = Sql::default();
    let scope2 = rollup_scope(
        &mut sql2,
        &key,
        q.environment.as_deref(),
        q.service.as_deref(),
        minutes * 2,
    );
    let cut = param(
        &mut sql2,
        "cut",
        "Int64",
        ParamValue::Int(i64::from(minutes)),
    );
    let text2 = format!(
        "SELECT minute >= toStartOfMinute(now()) - toIntervalMinute({cut}) AS cur, sumMerge(weighted) AS req, \
           countIfMerge(errors_5xx) AS e5, countIfMerge(slow) AS slow, \
           quantilesTDigestMerge(0.5, 0.95, 0.99)(latency)[2] AS p95 \
         FROM route_minute WHERE {scope2} GROUP BY cur"
    );
    let (per_minute, windows) = tokio::try_join!(
        fetch_json(&state, &text, &sql.params),
        fetch_json(&state, &text2, &sql2.params),
    )?;

    let now_min = (crate::now_ms() / 60_000) * 60_000;
    let start = now_min - i64::from(minutes - 1) * 60_000;
    let times: Vec<i64> = (0..minutes)
        .map(|i| start + i64::from(i) * 60_000)
        .collect();
    let by_t: BTreeMap<i64, &Map<String, Value>> = per_minute
        .rows
        .iter()
        .map(|r| (num(r, "t") as i64, r))
        .collect();
    let series = |f: &dyn Fn(&Map<String, Value>) -> f64| -> Vec<f64> {
        times
            .iter()
            .map(|t| by_t.get(t).map_or(0.0, |r| f(r)))
            .collect()
    };
    let rate = |r: &Map<String, Value>| {
        let req = num(r, "req");
        if req > 0.0 { num(r, "e5") / req } else { 0.0 }
    };
    let window = |cur: bool| windows.rows.iter().find(|r| (num(r, "cur") > 0.0) == cur);
    let (cur, prev) = (window(true), window(false));
    let w = |r: Option<&&Map<String, Value>>, f: &dyn Fn(&Map<String, Value>) -> f64| {
        r.map_or(0.0, |r| f(r))
    };
    let round = |x: f64, d: i32| (x * 10f64.powi(d)).round() / 10f64.powi(d);
    Ok(Json(OverviewResponse {
        minutes,
        requests_per_min: Tile {
            value: round(w(cur.as_ref(), &|r| num(r, "req")) / f64::from(minutes), 1),
            previous: round(w(prev.as_ref(), &|r| num(r, "req")) / f64::from(minutes), 1),
            series: series(&|r| num(r, "req").round()),
        },
        error_rate: Tile {
            value: round(w(cur.as_ref(), &rate), 4),
            previous: round(w(prev.as_ref(), &rate), 4),
            series: series(&|r| round(rate(r), 4)),
        },
        p95: Tile {
            value: round(w(cur.as_ref(), &|r| num(r, "p95")), 1),
            previous: round(w(prev.as_ref(), &|r| num(r, "p95")), 1),
            series: series(&|r| round(num(r, "p95"), 1)),
        },
        slow: Tile {
            value: w(cur.as_ref(), &|r| num(r, "slow")),
            previous: w(prev.as_ref(), &|r| num(r, "slow")),
            series: series(&|r| num(r, "slow")),
        },
        times,
        slow_ms,
    }))
}

async fn project_slow_ms(state: &AppState, project: Uuid) -> u32 {
    sqlx::query_scalar::<_, sqlx::types::Json<trailscope_core::project::ProjectSettings>>(
        "SELECT settings FROM projects WHERE id = $1",
    )
    .bind(project)
    .fetch_one(&state.pool)
    .await
    .map(|s| s.0.slow_ms)
    .unwrap_or(1000)
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct HeatmapQuery {
    pub project: Uuid,
    pub environment: Option<String>,
    pub service: Option<String>,
    /// Window in minutes (default 30, max 1440).
    pub minutes: Option<u32>,
    /// Number of routes (default 12, max 100), busiest first.
    pub limit: Option<u32>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct HeatCell {
    pub requests: f64,
    pub errors: f64,
    /// Share of 5xx, 0..1.
    pub error_rate: f64,
    pub p95: f64,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct HeatRoute {
    pub route: String,
    pub services: Vec<String>,
    /// One cell per minute, aligned with `times`; `null` when there was no traffic.
    pub cells: Vec<Option<HeatCell>>,
    /// p95 / error rate / volume over the last 5 minutes.
    pub current: HeatCell,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct HeatmapResponse {
    pub times: Vec<i64>,
    pub routes: Vec<HeatRoute>,
    /// Latency the colour scale treats as "over SLO" (the project's `slowMs`).
    pub slo_ms: u32,
}

/// Routes × minutes, with p95, errors and volume per cell.
#[utoipa::path(get, path = "/api/v1/routes/heatmap", tag = "overview", params(HeatmapQuery),
    responses((status = 200, body = HeatmapResponse)))]
pub async fn heatmap(
    State(state): State<AppState>,
    auth: Auth,
    Query(q): Query<HeatmapQuery>,
) -> ApiResult<Json<HeatmapResponse>> {
    auth.require(Role::Viewer, Scope::Read)?;
    auth.check_project(q.project)?;
    let key = state.project_key(q.project).await?;
    let minutes = q.minutes.unwrap_or(30).clamp(5, 1440);
    let limit = q.limit.unwrap_or(12).clamp(1, 100) as usize;
    let slo_ms = project_slow_ms(&state, q.project).await;
    // Coarser buckets for long windows keep the grid readable.
    let bucket = match minutes {
        0..=60 => 1,
        61..=180 => 5,
        181..=720 => 15,
        _ => 30,
    };
    let mut sql = Sql::default();
    let scope = rollup_scope(
        &mut sql,
        &key,
        q.environment.as_deref(),
        q.service.as_deref(),
        minutes,
    );
    let b = param(&mut sql, "bucket", "Int64", ParamValue::Int(bucket));
    let text = format!(
        "SELECT route, groupUniqArray(service) AS services, \
           toUnixTimestamp(toStartOfInterval(minute, toIntervalMinute({b}))) * 1000 AS t, \
           sumMerge(weighted) AS req, countIfMerge(errors_5xx) AS e5, \
           quantilesTDigestMerge(0.5, 0.95, 0.99)(latency)[2] AS p95 \
         FROM route_minute WHERE {scope} GROUP BY route, t"
    );
    let rows = fetch_json(&state, &text, &sql.params).await?;

    let step = bucket * 60_000;
    let now_b = (crate::now_ms() / step) * step;
    let count = (i64::from(minutes) / bucket).max(1);
    let times: Vec<i64> = (0..count).map(|i| now_b - (count - 1 - i) * step).collect();
    let index: BTreeMap<i64, usize> = times.iter().enumerate().map(|(i, t)| (*t, i)).collect();

    struct Acc {
        services: Vec<String>,
        cells: Vec<Option<HeatCell>>,
        total: f64,
    }
    let mut routes: BTreeMap<String, Acc> = BTreeMap::new();
    for r in &rows.rows {
        let route = r
            .get("route")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let acc = routes.entry(route).or_insert_with(|| Acc {
            services: Vec::new(),
            cells: (0..times.len()).map(|_| None).collect(),
            total: 0.0,
        });
        if let Some(Value::Array(svcs)) = r.get("services") {
            for s in svcs.iter().filter_map(Value::as_str) {
                if !acc.services.iter().any(|x| x == s) {
                    acc.services.push(s.to_string());
                }
            }
        }
        let (req, e5) = (num(r, "req"), num(r, "e5"));
        acc.total += req;
        if let Some(i) = index.get(&(num(r, "t") as i64)) {
            acc.cells[*i] = Some(HeatCell {
                requests: req.round(),
                errors: e5,
                error_rate: if req > 0.0 {
                    (e5 / req * 10_000.0).round() / 10_000.0
                } else {
                    0.0
                },
                p95: (num(r, "p95") * 10.0).round() / 10.0,
            });
        }
    }
    let mut list: Vec<(String, Acc)> = routes.into_iter().collect();
    list.sort_by(|a, b| b.1.total.total_cmp(&a.1.total));
    list.truncate(limit);
    // Buckets covering the last 5 minutes.
    let recent = times
        .len()
        .saturating_sub(((5 + bucket - 1) / bucket) as usize);
    let routes = list
        .into_iter()
        .map(|(route, acc)| {
            let tail: Vec<&HeatCell> = acc.cells[recent..].iter().flatten().collect();
            let req: f64 = tail.iter().map(|c| c.requests).sum();
            let errors: f64 = tail.iter().map(|c| c.errors).sum();
            let p95 = tail.iter().map(|c| c.p95).fold(0.0, f64::max);
            HeatRoute {
                route,
                services: acc.services,
                current: HeatCell {
                    requests: req,
                    errors,
                    error_rate: if req > 0.0 {
                        (errors / req * 10_000.0).round() / 10_000.0
                    } else {
                        0.0
                    },
                    p95,
                },
                cells: acc.cells,
            }
        })
        .collect();
    Ok(Json(HeatmapResponse {
        times,
        routes,
        slo_ms,
    }))
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct RouteSeriesQuery {
    pub project: Uuid,
    pub route: String,
    pub environment: Option<String>,
    pub service: Option<String>,
    /// Window in minutes (default 60, max 10080).
    pub minutes: Option<u32>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RouteSeries {
    pub route: String,
    pub bucket_minutes: i64,
    pub times: Vec<i64>,
    pub requests: Vec<f64>,
    pub errors_5xx: Vec<f64>,
    pub errors_4xx: Vec<f64>,
    pub p50: Vec<f64>,
    pub p95: Vec<f64>,
    pub p99: Vec<f64>,
    /// Requests per method over the window.
    pub methods: BTreeMap<String, f64>,
    pub slo_ms: u32,
}

/// Latency percentiles, volume and errors over time for one route.
#[utoipa::path(get, path = "/api/v1/routes/series", tag = "overview", params(RouteSeriesQuery),
    responses((status = 200, body = RouteSeries)))]
pub async fn route_series(
    State(state): State<AppState>,
    auth: Auth,
    Query(q): Query<RouteSeriesQuery>,
) -> ApiResult<Json<RouteSeries>> {
    auth.require(Role::Viewer, Scope::Read)?;
    auth.check_project(q.project)?;
    let key = state.project_key(q.project).await?;
    let minutes = q.minutes.unwrap_or(60).clamp(5, 10_080);
    let slo_ms = project_slow_ms(&state, q.project).await;
    let bucket: i64 = match minutes {
        0..=120 => 1,
        121..=720 => 5,
        721..=2880 => 15,
        _ => 60,
    };
    let mut sql = Sql::default();
    let scope = rollup_scope(
        &mut sql,
        &key,
        q.environment.as_deref(),
        q.service.as_deref(),
        minutes,
    );
    let r = param(
        &mut sql,
        "route",
        "String",
        ParamValue::Str(q.route.clone()),
    );
    let b = param(&mut sql, "bucket", "Int64", ParamValue::Int(bucket));
    let text = format!(
        "SELECT toUnixTimestamp(toStartOfInterval(minute, toIntervalMinute({b}))) * 1000 AS t, method, \
           sumMerge(weighted) AS req, countIfMerge(errors_5xx) AS e5, countIfMerge(errors_4xx) AS e4, \
           quantilesTDigestMerge(0.5, 0.95, 0.99)(latency) AS q \
         FROM route_minute WHERE {scope} AND route = {r} GROUP BY t, method"
    );
    let rows = fetch_json(&state, &text, &sql.params).await?;

    let step = bucket * 60_000;
    let now_b = (crate::now_ms() / step) * step;
    let count = (i64::from(minutes) / bucket).max(1);
    let times: Vec<i64> = (0..count).map(|i| now_b - (count - 1 - i) * step).collect();
    let index: BTreeMap<i64, usize> = times.iter().enumerate().map(|(i, t)| (*t, i)).collect();
    let n = times.len();
    let mut out = RouteSeries {
        route: q.route,
        bucket_minutes: bucket,
        times,
        requests: vec![0.0; n],
        errors_5xx: vec![0.0; n],
        errors_4xx: vec![0.0; n],
        p50: vec![0.0; n],
        p95: vec![0.0; n],
        p99: vec![0.0; n],
        methods: BTreeMap::new(),
        slo_ms,
    };
    for row in &rows.rows {
        let req = num(row, "req");
        let method = row
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        *out.methods.entry(method).or_insert(0.0) += req.round();
        let Some(&i) = index.get(&(num(row, "t") as i64)) else {
            continue;
        };
        let qs: Vec<f64> = row
            .get("q")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_f64).collect())
            .unwrap_or_default();
        // Methods share a bucket: keep the worst latency, sum the counts.
        let round = |v: f64| (v * 10.0).round() / 10.0;
        for (slot, v) in [&mut out.p50, &mut out.p95, &mut out.p99]
            .into_iter()
            .zip(qs)
        {
            slot[i] = slot[i].max(round(v));
        }
        out.requests[i] += req.round();
        out.errors_5xx[i] += num(row, "e5");
        out.errors_4xx[i] += num(row, "e4");
    }
    Ok(Json(out))
}
