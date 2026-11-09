//! ClickHouse row types (RowBinary), plus the compact summaries published to
//! the live tail and to the real-time error channel.
//!
//! `DateTime64(3)` columns are i64 milliseconds; `DateTime64(6)` is i64
//! microseconds. `Map` columns are `Vec<(String, String)>`.

use clickhouse::Row;
use serde::{Deserialize, Serialize};

pub type StrMap = Vec<(String, String)>;

#[derive(Debug, Clone, Default, PartialEq, Row, Serialize, Deserialize)]
pub struct RequestRow {
    pub project_id: String,
    pub environment: String,
    pub service: String,
    pub release: String,
    pub timestamp: i64,
    pub request_id: String,
    pub trace_id: String,
    pub span_id: String,
    pub method: String,
    pub route: String,
    pub path: String,
    pub query: String,
    pub status: u16,
    pub duration_ms: f32,
    pub req_size: u32,
    pub res_size: u32,
    pub user_id: String,
    pub client: String,
    pub client_version: String,
    pub ip: String,
    pub user_agent: String,
    pub req_headers: StrMap,
    pub res_headers: StrMap,
    pub req_body: String,
    pub res_body: String,
    pub body_truncated: u8,
    /// The request body was cut, so the request can't be replayed faithfully.
    pub req_body_truncated: u8,
    pub error_type: String,
    pub error_message: String,
    pub error_stack: String,
    pub error_fingerprint: u64,
    pub attributes: StrMap,
    pub correlation: StrMap,
    pub sample_rate: f32,
}

#[derive(Debug, Clone, Default, PartialEq, Row, Serialize, Deserialize)]
pub struct SpanRow {
    pub project_id: String,
    pub trace_id: String,
    pub span_id: String,
    pub parent_span_id: String,
    pub request_id: String,
    pub service: String,
    pub kind: String,
    pub name: String,
    /// Microseconds since the epoch.
    pub start: i64,
    pub duration_ms: f32,
    pub status: String,
    pub attributes: StrMap,
}

#[derive(Debug, Clone, Default, PartialEq, Row, Serialize, Deserialize)]
pub struct EventRow {
    pub project_id: String,
    pub environment: String,
    pub service: String,
    pub release: String,
    pub timestamp: i64,
    pub kind: String,
    pub name: String,
    pub level: String,
    pub message: String,
    pub status: String,
    pub duration_ms: f32,
    pub request_id: String,
    pub trace_id: String,
    pub user_id: String,
    pub attempt: u16,
    pub error_type: String,
    pub error_message: String,
    pub error_stack: String,
    pub error_fingerprint: u64,
    pub attributes: StrMap,
    pub correlation: StrMap,
}

/// What the live tail receives for each request: every filterable scalar plus
/// attributes and correlation, but no bodies, headers or stacks.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveRequest {
    pub project_id: String,
    pub environment: String,
    pub service: String,
    pub release: String,
    /// Milliseconds since the epoch.
    pub timestamp: i64,
    pub request_id: String,
    pub trace_id: String,
    pub method: String,
    pub route: String,
    pub path: String,
    pub query: String,
    pub status: u16,
    pub duration_ms: f32,
    pub req_size: u32,
    pub res_size: u32,
    pub user_id: String,
    pub client: String,
    pub client_version: String,
    pub ip: String,
    pub user_agent: String,
    pub error_type: String,
    pub error_message: String,
    /// Stringified so it survives JSON consumers that use f64 numbers.
    pub error_fingerprint: String,
    pub attributes: StrMap,
    pub correlation: StrMap,
    pub sample_rate: f32,
}

impl From<&RequestRow> for LiveRequest {
    fn from(r: &RequestRow) -> Self {
        Self {
            project_id: r.project_id.clone(),
            environment: r.environment.clone(),
            service: r.service.clone(),
            release: r.release.clone(),
            timestamp: r.timestamp,
            request_id: r.request_id.clone(),
            trace_id: r.trace_id.clone(),
            method: r.method.clone(),
            route: r.route.clone(),
            path: r.path.clone(),
            query: r.query.clone(),
            status: r.status,
            duration_ms: r.duration_ms,
            req_size: r.req_size,
            res_size: r.res_size,
            user_id: r.user_id.clone(),
            client: r.client.clone(),
            client_version: r.client_version.clone(),
            ip: r.ip.clone(),
            user_agent: r.user_agent.clone(),
            error_type: r.error_type.clone(),
            error_message: r.error_message.clone(),
            error_fingerprint: fingerprint_string(r.error_fingerprint),
            attributes: r.attributes.clone(),
            correlation: r.correlation.clone(),
            sample_rate: r.sample_rate,
        }
    }
}

/// A server error handed to the real-time alert engine. Deliberately excludes
/// bodies, headers and the client IP: these feed emails.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorSignal {
    pub project_id: String,
    pub environment: String,
    pub service: String,
    pub release: String,
    pub timestamp: i64,
    pub request_id: String,
    pub trace_id: String,
    pub user_id: String,
    pub method: String,
    pub route: String,
    pub status: u16,
    pub duration_ms: f32,
    pub error_type: String,
    pub error_message: String,
    pub error_stack: String,
    pub error_fingerprint: String,
    pub attributes: StrMap,
    pub correlation: StrMap,
}

impl From<&RequestRow> for ErrorSignal {
    fn from(r: &RequestRow) -> Self {
        Self {
            project_id: r.project_id.clone(),
            environment: r.environment.clone(),
            service: r.service.clone(),
            release: r.release.clone(),
            timestamp: r.timestamp,
            request_id: r.request_id.clone(),
            trace_id: r.trace_id.clone(),
            user_id: r.user_id.clone(),
            method: r.method.clone(),
            route: r.route.clone(),
            status: r.status,
            duration_ms: r.duration_ms,
            error_type: r.error_type.clone(),
            error_message: r.error_message.clone(),
            error_stack: r.error_stack.clone(),
            error_fingerprint: fingerprint_string(r.error_fingerprint),
            attributes: r.attributes.clone(),
            correlation: r.correlation.clone(),
        }
    }
}

/// Fingerprints are exposed as decimal strings (u64 does not fit in a JS number).
pub fn fingerprint_string(fp: u64) -> String {
    if fp == 0 {
        String::new()
    } else {
        fp.to_string()
    }
}
