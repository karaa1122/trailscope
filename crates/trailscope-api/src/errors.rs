//! Error groups: list with sparklines and new/spiking flags, and detail.

use std::collections::HashMap;

use axum::Json;
use axum::extract::{Path, Query, State};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use trailscope_query::compile::{ParamValue, Sql};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::AppState;
use crate::auth::{Auth, Role, Scope};
use crate::error::{ApiError, ApiResult};
use crate::overview::num;
use crate::query::{fetch_json, param};

/// A group is "new" when first seen within this many hours.
pub const NEW_HOURS: i64 = 24;
/// "Spiking": the rate over the last 15 minutes is at least this multiple of
/// the previous 24 hours' rate, with at least `SPIKE_MIN` occurrences.
pub const SPIKE_FACTOR: f64 = 3.0;
pub const SPIKE_MIN: f64 = 5.0;

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct ErrorsQuery {
    pub project: Uuid,
    pub environment: Option<String>,
    /// Groups seen since (ms; default 24 hours ago).
    pub from: Option<i64>,
    /// `all` (default), `new` or `spiking`.
    pub status: Option<String>,
    /// `recent` (default), `count`, `new`.
    pub sort: Option<String>,
    /// Substring of the error type or message.
    pub q: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Serialize, ToSchema, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ErrorGroup {
    /// Decimal string (u64).
    pub fingerprint: String,
    pub error_type: String,
    pub message: String,
    pub service: String,
    /// Route for requests, job/event name otherwise.
    pub route: String,
    /// `request`, `job`, `log` or `custom`.
    pub source: String,
    pub environment: String,
    pub first_seen: i64,
    pub last_seen: i64,
    /// All-time occurrences (within retention).
    pub total: u64,
    pub last_15m: u64,
    pub last_24h: u64,
    pub users: u64,
    pub first_release: String,
    pub sample_request_id: String,
    pub is_new: bool,
    pub spiking: bool,
    /// Occurrences per minute over the last 60 minutes, oldest first.
    pub sparkline: Vec<u64>,
}

/// `prev` counts occurrences before the last 15 minutes, within the 24-hour
/// baseline; `baseline_minutes` is how much of that window the group has
/// existed for, so a group first seen 3 hours ago is compared with its
/// 3-hour rate rather than a diluted 24-hour one.
pub fn spiking(last_15m: f64, prev: f64, baseline_minutes: f64) -> bool {
    if last_15m < SPIKE_MIN || prev <= 0.0 {
        return false;
    }
    let rate = last_15m / 15.0;
    let baseline = prev / baseline_minutes.clamp(15.0, 24.0 * 60.0);
    rate >= SPIKE_FACTOR * baseline
}

fn s(row: &Map<String, Value>, k: &str) -> String {
    row.get(k)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

pub async fn load_groups(
    state: &AppState,
    project_key: &str,
    env: Option<&str>,
    since_ms: i64,
    only: Option<u64>,
    limit: u32,
) -> ApiResult<Vec<ErrorGroup>> {
    let mut sql = Sql::default();
    let pid = param(
        &mut sql,
        "pid",
        "String",
        ParamValue::Str(project_key.to_string()),
    );
    let since = param(&mut sql, "since", "Int64", ParamValue::Int(since_ms));
    let mut filter = String::new();
    if let Some(e) = env.filter(|e| !e.is_empty()) {
        let e = param(&mut sql, "env", "String", ParamValue::Str(e.to_string()));
        filter.push_str(&format!(" AND environment = {e}"));
    }
    if let Some(fp) = only {
        let f = param(&mut sql, "fp", "UInt64", ParamValue::UInt(fp));
        filter.push_str(&format!(" AND error_fingerprint = {f}"));
    }
    let text = format!(
        "SELECT toString(error_fingerprint) AS fingerprint, any(environment) AS _c_environment, \
           toUnixTimestamp64Milli(min(first_seen)) AS _c_first_seen, toUnixTimestamp64Milli(max(last_seen)) AS _c_last_seen, \
           sum(count) AS total, anyLast(error_type) AS _c_error_type, anyLast(sample_message) AS message, \
           anyLast(service) AS _c_service, anyLast(route) AS _c_route, anyLast(source) AS _c_source, \
           argMaxMerge(sample_request_id) AS _c_sample_request_id, argMinMerge(first_release) AS _c_first_release, \
           uniqMerge(users) AS _c_users \
         FROM error_groups WHERE project_id = {pid}{filter} \
         GROUP BY error_fingerprint HAVING max(last_seen) >= fromUnixTimestamp64Milli({since}) \
         ORDER BY max(last_seen) DESC LIMIT {limit}"
    );
    let groups = fetch_json(state, &text, &sql.params).await?;
    if groups.rows.is_empty() {
        return Ok(Vec::new());
    }

    // Recent counts and per-minute sparklines from error_minute.
    let mut sql2 = Sql::default();
    let pid2 = param(
        &mut sql2,
        "pid",
        "String",
        ParamValue::Str(project_key.to_string()),
    );
    let mut filter2 = String::new();
    if let Some(e) = env.filter(|e| !e.is_empty()) {
        let e = param(&mut sql2, "env", "String", ParamValue::Str(e.to_string()));
        filter2.push_str(&format!(" AND environment = {e}"));
    }
    if let Some(fp) = only {
        let f = param(&mut sql2, "fp", "UInt64", ParamValue::UInt(fp));
        filter2.push_str(&format!(" AND error_fingerprint = {f}"));
    }
    let text2 = format!(
        "SELECT toString(error_fingerprint) AS fingerprint, \
           sumIf(count, minute >= now() - INTERVAL 15 MINUTE) AS c15, \
           sumIf(count, minute >= now() - INTERVAL 24 HOUR) AS c24, \
           sumIf(count, minute < now() - INTERVAL 15 MINUTE AND minute >= now() - INTERVAL 1455 MINUTE) AS prev, \
           groupArrayIf((toUnixTimestamp(minute), count), minute >= now() - INTERVAL 60 MINUTE) AS spark \
         FROM error_minute WHERE project_id = {pid2} AND minute >= now() - INTERVAL 1455 MINUTE{filter2} \
         GROUP BY error_fingerprint"
    );
    let recent = fetch_json(state, &text2, &sql2.params).await?;
    let by_fp: HashMap<String, &Map<String, Value>> = recent
        .rows
        .iter()
        .map(|r| (s(r, "fingerprint"), r))
        .collect();

    let now_s = crate::now_ms() / 1000;
    let spark_start = (now_s / 60) * 60 - 59 * 60;
    let mut out = Vec::with_capacity(groups.rows.len());
    for g in &groups.rows {
        let fp = s(g, "fingerprint");
        let r = by_fp.get(&fp);
        let (c15, c24, prev) = r.map_or((0.0, 0.0, 0.0), |r| {
            (num(r, "c15"), num(r, "c24"), num(r, "prev"))
        });
        let mut sparkline = vec![0u64; 60];
        if let Some(Value::Array(points)) = r.and_then(|r| r.get("spark")) {
            for p in points {
                if let Some([t, c]) = p
                    .as_array()
                    .map(|a| a.as_slice())
                    .and_then(|a| <&[Value; 2]>::try_from(a).ok())
                {
                    let t = t
                        .as_i64()
                        .or_else(|| t.as_str().and_then(|x| x.parse().ok()))
                        .unwrap_or(0);
                    let c = c
                        .as_u64()
                        .or_else(|| c.as_str().and_then(|x| x.parse().ok()))
                        .unwrap_or(0);
                    let idx = (t - spark_start) / 60;
                    if (0..60).contains(&idx) {
                        sparkline[idx as usize] += c;
                    }
                }
            }
        }
        let first_seen = num(g, "first_seen") as i64;
        out.push(ErrorGroup {
            error_type: s(g, "error_type"),
            message: s(g, "message"),
            service: s(g, "service"),
            route: s(g, "route"),
            source: s(g, "source"),
            environment: s(g, "environment"),
            first_seen,
            last_seen: num(g, "last_seen") as i64,
            total: num(g, "total") as u64,
            last_15m: c15 as u64,
            last_24h: c24 as u64,
            users: num(g, "users") as u64,
            first_release: s(g, "first_release"),
            sample_request_id: s(g, "sample_request_id"),
            is_new: first_seen >= crate::now_ms() - NEW_HOURS * 3_600_000,
            spiking: spiking(
                c15,
                prev,
                ((crate::now_ms() - first_seen) as f64 / 60_000.0) - 15.0,
            ),
            sparkline,
            fingerprint: fp,
        });
    }
    Ok(out)
}

#[derive(Serialize, ToSchema)]
pub struct ErrorsResponse {
    pub groups: Vec<ErrorGroup>,
}

/// Error groups with new/spiking flags and sparklines.
#[utoipa::path(get, path = "/api/v1/errors", tag = "errors", params(ErrorsQuery), responses((status = 200, body = ErrorsResponse)))]
pub async fn list(
    State(state): State<AppState>,
    auth: Auth,
    Query(q): Query<ErrorsQuery>,
) -> ApiResult<Json<ErrorsResponse>> {
    auth.require(Role::Viewer, Scope::Read)?;
    auth.check_project(q.project)?;
    let key = state.project_key(q.project).await?;
    let since = q.from.unwrap_or_else(|| crate::now_ms() - 24 * 3_600_000);
    let limit = q.limit.unwrap_or(50).clamp(1, 500);
    // Fetch extra so filtering by status still fills the page.
    let mut groups = load_groups(
        &state,
        &key,
        q.environment.as_deref(),
        since,
        None,
        (limit * 4).min(2000),
    )
    .await?;
    match q.status.as_deref() {
        Some("new") => groups.retain(|g| g.is_new),
        Some("spiking") => groups.retain(|g| g.spiking),
        Some("new_or_spiking") => groups.retain(|g| g.is_new || g.spiking),
        _ => {}
    }
    if let Some(text) =
        q.q.as_deref()
            .map(str::to_lowercase)
            .filter(|t| !t.is_empty())
    {
        groups.retain(|g| {
            g.error_type.to_lowercase().contains(&text) || g.message.to_lowercase().contains(&text)
        });
    }
    match q.sort.as_deref() {
        Some("count") => groups.sort_by_key(|g| std::cmp::Reverse(g.last_24h)),
        Some("new") => groups.sort_by_key(|g| std::cmp::Reverse(g.first_seen)),
        _ => groups.sort_by_key(|g| std::cmp::Reverse((g.spiking, g.last_seen))),
    }
    groups.truncate(limit as usize);
    Ok(Json(ErrorsResponse { groups }))
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ErrorDetail {
    pub group: ErrorGroup,
    /// Recent occurrences (requests or events), newest first.
    #[schema(value_type = Vec<Object>)]
    pub samples: Vec<Map<String, Value>>,
    /// Occurrences per hour over the last 7 days: `[t, count]`.
    pub hourly: Vec<[i64; 2]>,
    /// A stack trace from the latest sample.
    pub stack: String,
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct ErrorDetailQuery {
    pub project: Uuid,
    pub environment: Option<String>,
}

#[utoipa::path(get, path = "/api/v1/errors/{fingerprint}", tag = "errors", params(ErrorDetailQuery),
    responses((status = 200, body = ErrorDetail), (status = 404)))]
pub async fn detail(
    State(state): State<AppState>,
    auth: Auth,
    Path(fingerprint): Path<String>,
    Query(q): Query<ErrorDetailQuery>,
) -> ApiResult<Json<ErrorDetail>> {
    auth.require(Role::Viewer, Scope::Read)?;
    auth.check_project(q.project)?;
    let key = state.project_key(q.project).await?;
    let fp: u64 = fingerprint
        .parse()
        .map_err(|_| ApiError::not_found("error group"))?;
    let since = crate::now_ms() - i64::from(state.config.retention.days) * 86_400_000;
    let group = load_groups(&state, &key, q.environment.as_deref(), since, Some(fp), 1)
        .await?
        .pop()
        .ok_or_else(|| ApiError::not_found("error group"))?;

    let table = if group.source == "request" {
        "requests"
    } else {
        "events"
    };
    let cols = if table == "requests" {
        "toUnixTimestamp64Milli(timestamp) AS _c_timestamp, request_id, environment, service, release, method, route, path, \
         status, duration_ms, user_id, client, client_version, error_type, error_message, error_stack"
    } else {
        "toUnixTimestamp64Milli(timestamp) AS _c_timestamp, request_id, environment, service, release, kind, name, \
         status, duration_ms, user_id, attempt, error_type, error_message, error_stack, message"
    };
    let mut sql = Sql::default();
    let pid = param(&mut sql, "pid", "String", ParamValue::Str(key.clone()));
    let f = param(&mut sql, "fp", "UInt64", ParamValue::UInt(fp));
    let env = q.environment.clone().unwrap_or_default();
    let e = param(&mut sql, "env", "String", ParamValue::Str(env.clone()));
    let samples_sql = format!(
        "SELECT {cols} FROM {table} WHERE project_id = {pid} AND error_fingerprint = {f} \
           AND (empty({e}) OR environment = {e}) \
           AND timestamp >= now() - INTERVAL 7 DAY ORDER BY timestamp DESC LIMIT 25"
    );
    let samples = fetch_json(&state, &samples_sql, &sql.params).await?;

    let mut sql2 = Sql::default();
    let pid2 = param(&mut sql2, "pid", "String", ParamValue::Str(key));
    let f2 = param(&mut sql2, "fp", "UInt64", ParamValue::UInt(fp));
    let e2 = param(&mut sql2, "env", "String", ParamValue::Str(env));
    let hourly_sql = format!(
        "SELECT toUnixTimestamp(toStartOfHour(minute)) * 1000 AS t, sum(count) AS c FROM error_minute \
         WHERE project_id = {pid2} AND error_fingerprint = {f2} AND (empty({e2}) OR environment = {e2}) \
           AND minute >= now() - INTERVAL 7 DAY GROUP BY t ORDER BY t"
    );
    let hourly = fetch_json(&state, &hourly_sql, &sql2.params).await?;
    let stack = samples
        .rows
        .iter()
        .find_map(|r| {
            r.get("error_stack")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
        })
        .unwrap_or_default()
        .to_string();
    Ok(Json(ErrorDetail {
        group,
        stack,
        hourly: hourly
            .rows
            .iter()
            .map(|r| [num(r, "t") as i64, num(r, "c") as i64])
            .collect(),
        samples: samples.rows,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spike_detection() {
        // 30 in 15 min vs 24 in the previous day: 2/min vs ~0.017/min.
        assert!(spiking(30.0, 24.0, 1440.0));
        // Steady: 15 in 15 min vs 1440 in a day (1/min both).
        assert!(!spiking(15.0, 1440.0, 1440.0));
        // Steady over a 3-hour history must not look like a spike.
        assert!(!spiking(30.0, 330.0, 165.0));
        // Too few to call.
        assert!(!spiking(3.0, 1.0, 1440.0));
        // Brand new (no baseline) is "new", not "spiking".
        assert!(!spiking(50.0, 0.0, 0.0));
    }
}
