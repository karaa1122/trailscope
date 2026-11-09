//! Journeys: everything that happened for one correlation key (or user,
//! request or trace) across services and background jobs, as one timeline.

use std::collections::{BTreeMap, HashMap};

use axum::Json;
use axum::extract::{Query, State};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use trailscope_query::compile::{ParamValue, Sql};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::auth::{Auth, Role, Scope};
use crate::error::{ApiError, ApiResult};
use crate::overview::num;
use crate::query::{fetch_json, param};
use crate::{AppState, now_ms};

/// Runs longer than this (same lane, route and status) collapse into one item.
const COLLAPSE_AFTER: usize = 5;
const MAX_ITEMS: u32 = 2000;

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct JourneyQuery {
    pub project: Uuid,
    /// A correlation key (`account_id`, `session_id`, …) or `user_id`,
    /// `request_id`, `trace_id`.
    pub key: String,
    pub value: String,
    /// Window start (ms; default 7 days ago).
    pub from: Option<i64>,
    pub to: Option<i64>,
    pub environment: Option<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct JourneyItem {
    /// ms since the epoch.
    pub t: i64,
    /// `request`, `job`, `log` or `custom`.
    pub kind: String,
    /// Lane: the service, or `jobs · <service>` for background jobs.
    pub lane: String,
    /// `ok`, `warn` or `error`.
    pub status: String,
    pub title: String,
    pub detail: String,
    pub duration_ms: f64,
    pub request_id: String,
    pub trace_id: String,
    pub fingerprint: String,
    /// More than one when repetitive items were collapsed into this one.
    pub similar: usize,
    /// Last timestamp covered by a collapsed run.
    pub until: i64,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct JourneySummary {
    pub first: i64,
    pub last: i64,
    pub requests: usize,
    pub jobs: usize,
    pub errors: usize,
    pub warnings: usize,
    pub services: Vec<String>,
    /// Activity within the last 5 minutes.
    pub in_progress: bool,
    /// Items folded into "N similar" rows.
    pub collapsed: usize,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RelatedError {
    pub fingerprint: String,
    pub error_type: String,
    pub count: usize,
    pub status: String,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RelatedTrace {
    pub trace_id: String,
    pub spans: u64,
    pub services: Vec<String>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Journey {
    pub key: String,
    pub value: String,
    pub summary: JourneySummary,
    pub lanes: Vec<String>,
    pub items: Vec<JourneyItem>,
    /// Most common correlation values, attributes, users and clients seen.
    #[schema(value_type = Object)]
    pub context: BTreeMap<String, BTreeMap<String, String>>,
    pub errors: Vec<RelatedError>,
    pub traces: Vec<RelatedTrace>,
    pub truncated: bool,
}

fn s(row: &Map<String, Value>, k: &str) -> String {
    row.get(k)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn map_of(row: &Map<String, Value>, k: &str) -> Vec<(String, String)> {
    row.get(k)
        .and_then(Value::as_object)
        .map(|m| {
            m.iter()
                .map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_string()))
                .collect()
        })
        .unwrap_or_default()
}

/// The WHERE fragment selecting rows that belong to the journey.
fn key_condition(sql: &mut Sql, key: &str, value: &str) -> ApiResult<String> {
    let v = param(sql, "jv", "String", ParamValue::Str(value.to_string()));
    match key {
        "user_id" | "request_id" | "trace_id" => Ok(format!("{key} = {v}")),
        k => {
            if k.is_empty() || k.len() > 128 {
                return Err(ApiError::bad_request(
                    "journey keys are 1 to 128 characters",
                ));
            }
            let kp = param(sql, "jk", "String", ParamValue::Str(k.to_string()));
            Ok(format!("correlation[{kp}] = {v}"))
        }
    }
}

/// Collapses runs of more than `COLLAPSE_AFTER` items with the same lane and title.
pub fn collapse(items: Vec<JourneyItem>) -> (Vec<JourneyItem>, usize) {
    let mut out: Vec<JourneyItem> = Vec::with_capacity(items.len());
    let mut collapsed = 0;
    let mut i = 0;
    while i < items.len() {
        let mut j = i + 1;
        while j < items.len()
            && items[j].lane == items[i].lane
            && items[j].title == items[i].title
            && items[j].status == items[i].status
        {
            j += 1;
        }
        let run = j - i;
        if run > COLLAPSE_AFTER {
            let mut first = items[i].clone();
            first.similar = run;
            first.until = items[j - 1].t;
            first.duration_ms = items[i..j].iter().map(|x| x.duration_ms).sum::<f64>() / run as f64;
            collapsed += run - 1;
            out.push(first);
        } else {
            out.extend(items[i..j].iter().cloned());
        }
        i = j;
    }
    (out, collapsed)
}

#[utoipa::path(get, path = "/api/v1/journeys", tag = "journeys", params(JourneyQuery), responses((status = 200, body = Journey)))]
pub async fn journey(
    State(state): State<AppState>,
    auth: Auth,
    Query(q): Query<JourneyQuery>,
) -> ApiResult<Json<Journey>> {
    auth.require(Role::Viewer, Scope::Read)?;
    auth.check_project(q.project)?;
    let project = state.project_key(q.project).await?;
    let key = q.key.trim().to_string();
    let value = q.value.trim().to_string();
    if value.is_empty() || value.len() > 256 {
        return Err(ApiError::bad_request(
            "give a value to follow, e.g. account_id:acc_1",
        ));
    }
    let to = q.to.unwrap_or_else(now_ms);
    let from = q.from.unwrap_or(to - 7 * 86_400_000);

    let scope = |sql: &mut Sql| -> ApiResult<String> {
        let pid = param(sql, "pid", "String", ParamValue::Str(project.clone()));
        let f = param(sql, "from", "Int64", ParamValue::Int(from));
        let t = param(sql, "to", "Int64", ParamValue::Int(to));
        let mut text = format!(
            "project_id = {pid} AND timestamp BETWEEN fromUnixTimestamp64Milli({f}) AND fromUnixTimestamp64Milli({t})"
        );
        if let Some(e) = q.environment.as_deref().filter(|e| !e.is_empty()) {
            let e = param(sql, "env", "String", ParamValue::Str(e.to_string()));
            text.push_str(&format!(" AND environment = {e}"));
        }
        Ok(text)
    };

    // Requests.
    let mut rq = Sql::default();
    let base = scope(&mut rq)?;
    let cond = key_condition(&mut rq, &key, &value)?;
    let req_sql = format!(
        "SELECT toUnixTimestamp64Milli(timestamp) AS _c_timestamp, request_id, trace_id, service, method, route, status, \
           duration_ms, user_id, client, client_version, error_type, error_message, if(error_fingerprint = 0, '', toString(error_fingerprint)) AS _c_fp, \
           correlation, attributes \
         FROM requests WHERE {base} AND {cond} ORDER BY timestamp LIMIT {MAX_ITEMS}"
    );
    let requests = fetch_json(&state, &req_sql, &rq.params).await?;
    let request_ids: Vec<String> = requests
        .rows
        .iter()
        .map(|r| s(r, "request_id"))
        .filter(|x| !x.is_empty())
        .take(1000)
        .collect();
    let trace_ids: Vec<String> = {
        let mut v: Vec<String> = requests
            .rows
            .iter()
            .map(|r| s(r, "trace_id"))
            .filter(|x| !x.is_empty())
            .collect();
        v.sort();
        v.dedup();
        v.truncate(200);
        v
    };

    // Events: matching the key, or triggered by one of the requests.
    let mut ev = Sql::default();
    let base = scope(&mut ev)?;
    let cond = key_condition(&mut ev, &key, &value)?;
    let ids = param(
        &mut ev,
        "rids",
        "Array(String)",
        ParamValue::Strs(request_ids),
    );
    let ev_sql = format!(
        "SELECT toUnixTimestamp64Milli(timestamp) AS _c_timestamp, service, kind, name, level, message, status, duration_ms, \
           request_id, trace_id, attempt, user_id, error_type, error_message, if(error_fingerprint = 0, '', toString(error_fingerprint)) AS _c_fp, correlation, attributes \
         FROM events WHERE {base} AND ({cond} OR (notEmpty(request_id) AND has({ids}, request_id))) \
         ORDER BY timestamp LIMIT {MAX_ITEMS}"
    );
    let events = fetch_json(&state, &ev_sql, &ev.params).await?;

    // Traces of those requests.
    let traces = if trace_ids.is_empty() {
        Vec::new()
    } else {
        let mut sp = Sql::default();
        let pid = param(&mut sp, "pid", "String", ParamValue::Str(project.clone()));
        let tids = param(
            &mut sp,
            "tids",
            "Array(String)",
            ParamValue::Strs(trace_ids),
        );
        let sp_sql = format!(
            "SELECT trace_id, count() AS spans, groupUniqArray(service) AS services FROM spans \
             WHERE project_id = {pid} AND has({tids}, trace_id) GROUP BY trace_id ORDER BY spans DESC LIMIT 20"
        );
        fetch_json(&state, &sp_sql, &sp.params)
            .await?
            .rows
            .iter()
            .map(|r| RelatedTrace {
                trace_id: s(r, "trace_id"),
                spans: num(r, "spans") as u64,
                services: r
                    .get("services")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(Value::as_str)
                            .map(str::to_string)
                            .collect()
                    })
                    .unwrap_or_default(),
            })
            .collect()
    };

    let truncated =
        requests.rows.len() as u32 >= MAX_ITEMS || events.rows.len() as u32 >= MAX_ITEMS;
    let mut items = Vec::with_capacity(requests.rows.len() + events.rows.len());
    let mut context: BTreeMap<String, HashMap<String, usize>> = BTreeMap::new();
    let mut bump = |section: &str, k: &str, v: &str| {
        if !v.is_empty() {
            *context
                .entry(format!("{section}\u{1f}{k}"))
                .or_default()
                .entry(v.to_string())
                .or_insert(0) += 1;
        }
    };
    for r in &requests.rows {
        let status = num(r, "status") as u16;
        for (k, v) in map_of(r, "correlation") {
            bump("correlation", &k, &v);
        }
        for (k, v) in map_of(r, "attributes") {
            bump("attributes", &k, &v);
        }
        bump("who", "user", &s(r, "user_id"));
        bump(
            "who",
            "client",
            [s(r, "client"), s(r, "client_version")].join(" ").trim(),
        );
        let error = s(r, "error_message");
        items.push(JourneyItem {
            t: num(r, "timestamp") as i64,
            kind: "request".into(),
            lane: s(r, "service"),
            status: if status >= 500 {
                "error"
            } else if status >= 400 {
                "warn"
            } else {
                "ok"
            }
            .into(),
            title: format!("{} {} → {status}", s(r, "method"), s(r, "route")),
            detail: if error.is_empty() {
                String::new()
            } else {
                format!("{}: {error}", s(r, "error_type"))
            },
            duration_ms: num(r, "duration_ms"),
            request_id: s(r, "request_id"),
            trace_id: s(r, "trace_id"),
            fingerprint: s(r, "fp"),
            similar: 1,
            until: num(r, "timestamp") as i64,
        });
    }
    for r in &events.rows {
        let kind = s(r, "kind");
        for (k, v) in map_of(r, "correlation") {
            bump("correlation", &k, &v);
        }
        let attempt = num(r, "attempt") as u32;
        let name = s(r, "name");
        let error = s(r, "error_message");
        let message = s(r, "message");
        items.push(JourneyItem {
            t: num(r, "timestamp") as i64,
            lane: if kind == "job" {
                format!("jobs · {}", s(r, "service"))
            } else {
                s(r, "service")
            },
            status: s(r, "status"),
            title: if kind == "job" && attempt > 0 {
                format!("{name} · attempt {attempt}")
            } else {
                name
            },
            detail: if error.is_empty() {
                message
            } else {
                format!("{}: {error}", s(r, "error_type"))
            },
            duration_ms: num(r, "duration_ms"),
            request_id: s(r, "request_id"),
            trace_id: s(r, "trace_id"),
            fingerprint: s(r, "fp"),
            similar: 1,
            until: num(r, "timestamp") as i64,
            kind,
        });
    }
    items.sort_by_key(|i| i.t);

    // Related errors before collapsing, so counts are exact.
    let mut errs: BTreeMap<String, RelatedError> = BTreeMap::new();
    for i in items
        .iter()
        .filter(|i| !i.fingerprint.is_empty() && i.status != "ok")
    {
        let e = errs
            .entry(i.fingerprint.clone())
            .or_insert_with(|| RelatedError {
                fingerprint: i.fingerprint.clone(),
                error_type: i.detail.split(':').next().unwrap_or_default().to_string(),
                count: 0,
                status: i.status.clone(),
            });
        e.count += 1;
    }
    let mut errors: Vec<RelatedError> = errs.into_values().collect();
    errors.sort_by_key(|e| std::cmp::Reverse(e.count));

    let summary_counts = (
        items.iter().filter(|i| i.kind == "request").count(),
        items.iter().filter(|i| i.kind == "job").count(),
        items.iter().filter(|i| i.status == "error").count(),
        items.iter().filter(|i| i.status == "warn").count(),
    );
    let (items, collapsed) = collapse(items);
    let mut lanes: Vec<String> = Vec::new();
    for i in &items {
        if !lanes.contains(&i.lane) {
            lanes.push(i.lane.clone());
        }
    }
    let services: Vec<String> = {
        let mut v: Vec<String> = lanes
            .iter()
            .map(|l| l.trim_start_matches("jobs · ").to_string())
            .collect();
        v.sort();
        v.dedup();
        v
    };
    let first = items.first().map_or(0, |i| i.t);
    let last = items.last().map_or(0, |i| i.until.max(i.t));

    // Keep the most common value per context key.
    let mut ctx: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    for (k, values) in context {
        let (section, name) = k.split_once('\u{1f}').unwrap_or(("", &k));
        if let Some((v, _)) = values.into_iter().max_by_key(|(_, n)| *n) {
            ctx.entry(section.to_string())
                .or_default()
                .insert(name.to_string(), v);
        }
    }

    Ok(Json(Journey {
        key,
        value,
        summary: JourneySummary {
            first,
            last,
            requests: summary_counts.0,
            jobs: summary_counts.1,
            errors: summary_counts.2,
            warnings: summary_counts.3,
            services,
            in_progress: last > 0 && now_ms() - last < 5 * 60_000,
            collapsed,
        },
        lanes,
        items,
        context: ctx,
        errors,
        traces,
        truncated,
    }))
}
