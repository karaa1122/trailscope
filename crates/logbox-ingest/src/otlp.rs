//! OTLP/HTTP receiver: traces and logs, as protobuf or JSON, mapped onto the
//! ingest wire types so they are validated, redacted and stored like SDK data.
//!
//! - Every span becomes a `spans` row. A server span with an HTTP method also
//!   becomes a `requests` row (request ID = span ID), so services instrumented
//!   with OpenTelemetry show up in Live tail, Explore and the error views.
//! - Every log record becomes a `log` event.
//!
//! The message types are the subset of `opentelemetry-proto` the mapping
//! reads; unknown fields are skipped by both decoders.

use std::collections::BTreeMap;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use logbox_core::wire::{WireEvent, WireMap, WireRequest, WireSpan, WireTime};
use serde::{Deserialize, Deserializer};

// ---- messages ------------------------------------------------------------------

#[derive(Clone, PartialEq, prost::Message, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ExportTraceServiceRequest {
    #[prost(message, repeated, tag = "1")]
    pub resource_spans: Vec<ResourceSpans>,
}

#[derive(Clone, PartialEq, prost::Message, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ResourceSpans {
    #[prost(message, optional, tag = "1")]
    pub resource: Option<Resource>,
    #[prost(message, repeated, tag = "2")]
    pub scope_spans: Vec<ScopeSpans>,
}

#[derive(Clone, PartialEq, prost::Message, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ScopeSpans {
    #[prost(message, optional, tag = "1")]
    pub scope: Option<InstrumentationScope>,
    #[prost(message, repeated, tag = "2")]
    pub spans: Vec<Span>,
}

#[derive(Clone, PartialEq, prost::Message, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Span {
    #[prost(bytes = "vec", tag = "1")]
    #[serde(deserialize_with = "de_hex")]
    pub trace_id: Vec<u8>,
    #[prost(bytes = "vec", tag = "2")]
    #[serde(deserialize_with = "de_hex")]
    pub span_id: Vec<u8>,
    #[prost(bytes = "vec", tag = "4")]
    #[serde(deserialize_with = "de_hex")]
    pub parent_span_id: Vec<u8>,
    #[prost(string, tag = "5")]
    pub name: String,
    #[prost(int32, tag = "6")]
    #[serde(deserialize_with = "de_i32")]
    pub kind: i32,
    #[prost(fixed64, tag = "7")]
    #[serde(deserialize_with = "de_u64")]
    pub start_time_unix_nano: u64,
    #[prost(fixed64, tag = "8")]
    #[serde(deserialize_with = "de_u64")]
    pub end_time_unix_nano: u64,
    #[prost(message, repeated, tag = "9")]
    pub attributes: Vec<KeyValue>,
    #[prost(message, repeated, tag = "11")]
    pub events: Vec<SpanEvent>,
    #[prost(message, optional, tag = "15")]
    pub status: Option<Status>,
}

#[derive(Clone, PartialEq, prost::Message, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SpanEvent {
    #[prost(fixed64, tag = "1")]
    #[serde(deserialize_with = "de_u64")]
    pub time_unix_nano: u64,
    #[prost(string, tag = "2")]
    pub name: String,
    #[prost(message, repeated, tag = "3")]
    pub attributes: Vec<KeyValue>,
}

#[derive(Clone, PartialEq, prost::Message, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Status {
    #[prost(string, tag = "2")]
    pub message: String,
    #[prost(int32, tag = "3")]
    #[serde(deserialize_with = "de_i32")]
    pub code: i32,
}

#[derive(Clone, PartialEq, prost::Message, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ExportLogsServiceRequest {
    #[prost(message, repeated, tag = "1")]
    pub resource_logs: Vec<ResourceLogs>,
}

#[derive(Clone, PartialEq, prost::Message, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ResourceLogs {
    #[prost(message, optional, tag = "1")]
    pub resource: Option<Resource>,
    #[prost(message, repeated, tag = "2")]
    pub scope_logs: Vec<ScopeLogs>,
}

#[derive(Clone, PartialEq, prost::Message, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ScopeLogs {
    #[prost(message, optional, tag = "1")]
    pub scope: Option<InstrumentationScope>,
    #[prost(message, repeated, tag = "2")]
    pub log_records: Vec<LogRecord>,
}

#[derive(Clone, PartialEq, prost::Message, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct LogRecord {
    #[prost(fixed64, tag = "1")]
    #[serde(deserialize_with = "de_u64")]
    pub time_unix_nano: u64,
    #[prost(int32, tag = "2")]
    #[serde(deserialize_with = "de_i32")]
    pub severity_number: i32,
    #[prost(string, tag = "3")]
    pub severity_text: String,
    #[prost(message, optional, tag = "5")]
    pub body: Option<AnyValue>,
    #[prost(message, repeated, tag = "6")]
    pub attributes: Vec<KeyValue>,
    #[prost(bytes = "vec", tag = "9")]
    #[serde(deserialize_with = "de_hex")]
    pub trace_id: Vec<u8>,
    #[prost(bytes = "vec", tag = "10")]
    #[serde(deserialize_with = "de_hex")]
    pub span_id: Vec<u8>,
    #[prost(fixed64, tag = "11")]
    #[serde(deserialize_with = "de_u64")]
    pub observed_time_unix_nano: u64,
    #[prost(string, tag = "12")]
    pub event_name: String,
}

#[derive(Clone, PartialEq, prost::Message, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Resource {
    #[prost(message, repeated, tag = "1")]
    pub attributes: Vec<KeyValue>,
}

#[derive(Clone, PartialEq, prost::Message, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct InstrumentationScope {
    #[prost(string, tag = "1")]
    pub name: String,
    #[prost(string, tag = "2")]
    pub version: String,
}

#[derive(Clone, PartialEq, prost::Message, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct KeyValue {
    #[prost(string, tag = "1")]
    pub key: String,
    #[prost(message, optional, tag = "2")]
    pub value: Option<AnyValue>,
}

#[derive(Clone, PartialEq, prost::Message, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AnyValue {
    #[prost(oneof = "Value", tags = "1, 2, 3, 4, 5, 6, 7")]
    #[serde(flatten)]
    pub value: Option<Value>,
}

#[derive(Clone, PartialEq, prost::Oneof, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Value {
    #[prost(string, tag = "1")]
    StringValue(String),
    #[prost(bool, tag = "2")]
    BoolValue(bool),
    #[prost(int64, tag = "3")]
    IntValue(#[serde(deserialize_with = "de_i64")] i64),
    #[prost(double, tag = "4")]
    DoubleValue(f64),
    #[prost(message, tag = "5")]
    ArrayValue(ArrayValue),
    #[prost(message, tag = "6")]
    KvlistValue(KeyValueList),
    #[prost(bytes = "vec", tag = "7")]
    BytesValue(#[serde(deserialize_with = "de_base64")] Vec<u8>),
}

#[derive(Clone, PartialEq, prost::Message, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ArrayValue {
    #[prost(message, repeated, tag = "1")]
    pub values: Vec<AnyValue>,
}

#[derive(Clone, PartialEq, prost::Message, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct KeyValueList {
    #[prost(message, repeated, tag = "1")]
    pub values: Vec<KeyValue>,
}

/// `Export{Trace,Logs}ServiceResponse`: both have `partial_success` at tag 1,
/// holding the rejected count (tag 1) and an error message (tag 2).
#[derive(Clone, PartialEq, prost::Message)]
pub struct ExportResponse {
    #[prost(message, optional, tag = "1")]
    pub partial_success: Option<PartialSuccess>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct PartialSuccess {
    #[prost(int64, tag = "1")]
    pub rejected: i64,
    #[prost(string, tag = "2")]
    pub error_message: String,
}

// ---- JSON encoding quirks ---------------------------------------------------------

/// Trace and span IDs are hex strings in OTLP/JSON.
fn de_hex<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
    let s = Option::<String>::deserialize(d)?.unwrap_or_default();
    hex::decode(s.trim()).map_err(serde::de::Error::custom)
}

fn de_base64<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
    let s = String::deserialize(d)?;
    STANDARD.decode(s.trim()).map_err(serde::de::Error::custom)
}

/// 64-bit integers may be JSON numbers or decimal strings.
#[derive(Deserialize)]
#[serde(untagged)]
enum Num {
    I(i64),
    U(u64),
    F(f64),
    S(String),
}

impl Num {
    fn as_i128(&self) -> i128 {
        match self {
            Num::I(i) => i128::from(*i),
            Num::U(u) => i128::from(*u),
            Num::F(f) => *f as i128,
            Num::S(s) => s.trim().parse().unwrap_or(0),
        }
    }
}

fn de_u64<'de, D: Deserializer<'de>>(d: D) -> Result<u64, D::Error> {
    Ok(Option::<Num>::deserialize(d)?
        .map_or(0, |n| n.as_i128().clamp(0, i128::from(u64::MAX)) as u64))
}

fn de_i64<'de, D: Deserializer<'de>>(d: D) -> Result<i64, D::Error> {
    Ok(Num::deserialize(d)?
        .as_i128()
        .clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64)
}

/// Enums are integers in OTLP/JSON; anything else counts as unspecified.
fn de_i32<'de, D: Deserializer<'de>>(d: D) -> Result<i32, D::Error> {
    Ok(Option::<Num>::deserialize(d)?
        .map_or(0, |n| n.as_i128().clamp(0, i128::from(i32::MAX)) as i32))
}

// ---- mapping ----------------------------------------------------------------------

/// What one export request turned into.
#[derive(Debug, Default)]
pub struct Mapped {
    pub requests: Vec<WireRequest>,
    pub spans: Vec<WireSpan>,
    pub events: Vec<WireEvent>,
}

const SPAN_KIND_SERVER: i32 = 2;
const SPAN_KIND_CLIENT: i32 = 3;
const SPAN_KIND_PRODUCER: i32 = 4;
const SPAN_KIND_CONSUMER: i32 = 5;
const STATUS_ERROR: i32 = 2;

fn value_string(v: &AnyValue) -> Option<String> {
    Some(match v.value.as_ref()? {
        Value::StringValue(s) => s.clone(),
        Value::BoolValue(b) => b.to_string(),
        Value::IntValue(i) => i.to_string(),
        Value::DoubleValue(f) => f.to_string(),
        Value::BytesValue(b) => STANDARD.encode(b),
        Value::ArrayValue(_) | Value::KvlistValue(_) => value_json(v).to_string(),
    })
}

fn value_json(v: &AnyValue) -> serde_json::Value {
    match &v.value {
        None => serde_json::Value::Null,
        Some(Value::StringValue(s)) => s.clone().into(),
        Some(Value::BoolValue(b)) => (*b).into(),
        Some(Value::IntValue(i)) => (*i).into(),
        Some(Value::DoubleValue(f)) => (*f).into(),
        Some(Value::BytesValue(b)) => STANDARD.encode(b).into(),
        Some(Value::ArrayValue(a)) => a.values.iter().map(value_json).collect(),
        Some(Value::KvlistValue(kv)) => kv
            .values
            .iter()
            .map(|e| {
                (
                    e.key.clone(),
                    e.value.as_ref().map_or(serde_json::Value::Null, value_json),
                )
            })
            .collect::<serde_json::Map<_, _>>()
            .into(),
    }
}

fn attrs(kvs: &[KeyValue]) -> BTreeMap<String, String> {
    kvs.iter()
        .filter_map(|kv| Some((kv.key.clone(), value_string(kv.value.as_ref()?)?)))
        .collect()
}

fn hex_id(b: &[u8]) -> String {
    if b.iter().all(|x| *x == 0) {
        String::new()
    } else {
        hex::encode(b)
    }
}

fn time_ns(ns: u64) -> Option<WireTime> {
    (ns > 0).then(|| WireTime((ns / 1000).min(i64::MAX as u64) as i64))
}

/// Takes the first of `keys` present in `map`, removing them all.
fn take(map: &mut BTreeMap<String, String>, keys: &[&str]) -> Option<String> {
    let mut found = None;
    for k in keys {
        if let Some(v) = map.remove(*k)
            && found.is_none()
            && !v.is_empty()
        {
            found = Some(v);
        }
    }
    found
}

/// Service, environment and release from resource attributes (semantic conventions).
struct Origin {
    service: String,
    environment: Option<String>,
    release: Option<String>,
}

fn origin(resource: Option<&Resource>) -> Origin {
    let mut a = resource.map(|r| attrs(&r.attributes)).unwrap_or_default();
    Origin {
        service: take(&mut a, &["service.name"]).unwrap_or_else(|| "unknown_service".into()),
        environment: take(
            &mut a,
            &["deployment.environment.name", "deployment.environment"],
        ),
        release: take(&mut a, &["service.version"]),
    }
}

fn span_kind(kind: i32, a: &BTreeMap<String, String>) -> &'static str {
    let has = |k: &str| a.contains_key(k);
    if has("db.system") || has("db.system.name") {
        "db"
    } else if has("messaging.system") || kind == SPAN_KIND_PRODUCER || kind == SPAN_KIND_CONSUMER {
        "queue"
    } else if kind == SPAN_KIND_CLIENT {
        "external"
    } else if has("http.request.method") || has("http.method") {
        "http"
    } else {
        "fn"
    }
}

/// `exception.*` attributes (span events named `exception`, or log records).
fn exception(a: &mut BTreeMap<String, String>) -> (Option<String>, Option<String>, Option<String>) {
    (
        take(a, &["exception.type"]),
        take(a, &["exception.message"]),
        take(a, &["exception.stacktrace"]),
    )
}

pub fn map_traces(req: ExportTraceServiceRequest) -> Mapped {
    let mut out = Mapped::default();
    for rs in req.resource_spans {
        let o = origin(rs.resource.as_ref());
        for ss in rs.scope_spans {
            for span in ss.spans {
                map_span(&o, span, &mut out);
            }
        }
    }
    out
}

fn map_span(o: &Origin, span: Span, out: &mut Mapped) {
    let trace_id = hex_id(&span.trace_id);
    let span_id = hex_id(&span.span_id);
    let parent = hex_id(&span.parent_span_id);
    let duration_ms = span
        .end_time_unix_nano
        .saturating_sub(span.start_time_unix_nano) as f64
        / 1e6;
    let a = attrs(&span.attributes);
    let failed = span.status.as_ref().is_some_and(|s| s.code == STATUS_ERROR);
    let kind = span_kind(span.kind, &a);
    let is_request = span.kind == SPAN_KIND_SERVER
        && (a.contains_key("http.request.method") || a.contains_key("http.method"));
    out.spans.push(WireSpan {
        trace_id: trace_id.clone(),
        span_id: span_id.clone(),
        parent_span_id: (!parent.is_empty()).then(|| parent.clone()),
        request_id: is_request.then(|| span_id.clone()),
        service: o.service.clone(),
        kind: Some(kind.into()),
        name: span.name.clone(),
        start: time_ns(span.start_time_unix_nano),
        duration_ms,
        status: Some(if failed { "error" } else { "ok" }.into()),
        attributes: WireMap(a.clone()),
    });
    if !is_request {
        return;
    }
    let mut a = a;
    let method =
        take(&mut a, &["http.request.method", "http.method"]).unwrap_or_else(|| "GET".into());
    let status = take(&mut a, &["http.response.status_code", "http.status_code"])
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(if failed { 500 } else { 200 });
    let route = take(&mut a, &["http.route"]).map(|r| route_template(&r));
    let mut query = take(&mut a, &["url.query"]);
    let mut path = take(&mut a, &["url.path"]);
    if let Some(target) = take(&mut a, &["http.target"]) {
        let (p, q) = target.split_once('?').unwrap_or((target.as_str(), ""));
        path.get_or_insert_with(|| p.to_string());
        if query.is_none() && !q.is_empty() {
            query = Some(q.to_string());
        }
    }
    if let Some(full) = take(&mut a, &["url.full", "http.url"])
        && let Ok(u) = url::Url::parse(&full)
    {
        path.get_or_insert_with(|| u.path().to_string());
        if query.is_none() {
            query = u.query().map(str::to_string);
        }
    }
    let user_agent = take(&mut a, &["user_agent.original", "http.user_agent"]);
    let ip = take(
        &mut a,
        &[
            "client.address",
            "http.client_ip",
            "net.sock.peer.addr",
            "net.peer.ip",
        ],
    );
    let user_id = take(&mut a, &["enduser.id", "user.id"]);
    // The first `exception` event carries the error, as recorded by OpenTelemetry SDKs.
    let (mut error_type, mut error_message, mut error_stack) = (None, None, None);
    if let Some(ev) = span.events.iter().find(|e| e.name == "exception") {
        let mut ea = attrs(&ev.attributes);
        (error_type, error_message, error_stack) = exception(&mut ea);
    }
    if failed && error_message.is_none() {
        error_message = span
            .status
            .as_ref()
            .map(|s| s.message.clone())
            .filter(|m| !m.is_empty());
    }
    out.requests.push(WireRequest {
        timestamp: time_ns(span.start_time_unix_nano),
        environment: o.environment.clone(),
        service: o.service.clone(),
        release: o.release.clone(),
        request_id: Some(span_id.clone()),
        trace_id: Some(trace_id),
        span_id: Some(span_id),
        method,
        route,
        path: path.unwrap_or_else(|| "/".into()),
        query,
        status,
        duration_ms,
        user_id,
        ip,
        user_agent,
        error_type,
        error_message,
        error_stack,
        attributes: WireMap(a),
        ..WireRequest::default()
    });
}

/// Framework route syntaxes (`/items/{id}`, `/items/{id:int}`, `/items/<int:id>`)
/// become Logbox's `/items/:id`, so OTLP routes group with SDK routes.
fn route_template(route: &str) -> String {
    static PARAM: std::sync::LazyLock<Option<regex::Regex>> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"\{([A-Za-z_][\w]*)(?::[^}]*)?\}|<(?:[^>:]+:)?([A-Za-z_][\w]*)>").ok()
    });
    match PARAM.as_ref() {
        Some(re) => re
            .replace_all(route, |c: &regex::Captures<'_>| {
                format!(
                    ":{}",
                    c.get(1).or_else(|| c.get(2)).map_or("", |m| m.as_str())
                )
            })
            .into_owned(),
        None => route.to_string(),
    }
}

/// OpenTelemetry severity numbers: 1–8 trace/debug, 9–12 info, 13–16 warn, 17–24 error/fatal.
fn level(number: i32, text: &str) -> &'static str {
    match number {
        1..=8 => "debug",
        9..=12 => "info",
        13..=16 => "warn",
        17..=24 => "error",
        _ => match text.trim().to_ascii_lowercase().as_str() {
            "trace" | "debug" => "debug",
            "warn" | "warning" => "warn",
            "error" | "fatal" | "critical" => "error",
            _ => "info",
        },
    }
}

pub fn map_logs(req: ExportLogsServiceRequest) -> Mapped {
    let mut out = Mapped::default();
    for rl in req.resource_logs {
        let o = origin(rl.resource.as_ref());
        for sl in rl.scope_logs {
            let scope = sl
                .scope
                .as_ref()
                .map(|s| s.name.clone())
                .filter(|n| !n.is_empty());
            for rec in sl.log_records {
                let mut a = attrs(&rec.attributes);
                let (error_type, error_message, error_stack) = exception(&mut a);
                let name = Some(rec.event_name.clone())
                    .filter(|n| !n.is_empty())
                    .or_else(|| take(&mut a, &["event.name"]))
                    .or_else(|| scope.clone())
                    .unwrap_or_else(|| "log".into());
                let message = rec
                    .body
                    .as_ref()
                    .and_then(value_string)
                    .or_else(|| error_message.clone());
                let trace_id = hex_id(&rec.trace_id);
                out.events.push(WireEvent {
                    timestamp: time_ns(rec.time_unix_nano)
                        .or_else(|| time_ns(rec.observed_time_unix_nano)),
                    environment: o.environment.clone(),
                    service: o.service.clone(),
                    release: o.release.clone(),
                    kind: Some("log".into()),
                    name,
                    level: Some(level(rec.severity_number, &rec.severity_text).into()),
                    message,
                    trace_id: (!trace_id.is_empty()).then_some(trace_id),
                    user_id: take(&mut a, &["enduser.id", "user.id"]),
                    error_type,
                    error_message,
                    error_stack,
                    attributes: WireMap(a),
                    ..WireEvent::default()
                });
            }
        }
    }
    out
}
