//! Trailscope ingest: `POST /v1/ingest`, `POST /v1/releases`, `GET /v1/config`,
//! and the OTLP/HTTP receiver at `/v1/otlp/v1/traces` and `/v1/otlp/v1/logs`.
//!
//! Batches are authenticated, decompressed, split into items, validated and
//! redacted item by item, fingerprinted, then admitted to a bounded queue that
//! a writer task flushes to ClickHouse.

pub mod decode;
pub mod fingerprint;
pub mod keys;
pub mod normalize;
pub mod otlp;
pub mod pipeline;
pub mod redact;

use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::{DefaultBodyLimit, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use bytes::Bytes;
use serde_json::json;
use sqlx::PgPool;
use tokio::sync::Semaphore;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use trailscope_core::Config;
use trailscope_core::ch::ClickHouse;
use trailscope_core::live::LiveBus;
use trailscope_core::wire::{IngestResponse, ItemError, WireItem, WireRelease};

use crate::keys::{KeyCache, ProjectCtx};
use crate::normalize::Limits;
use crate::pipeline::{Batch, ClickHouseSink, PushError, Queue, WriterConfig};

/// Bodies above this are processed on the blocking pool so big batches don't
/// stall other requests on the same worker thread.
const BLOCKING_THRESHOLD: usize = 64 * 1024;

#[derive(Clone)]
pub struct IngestState {
    queue: Arc<Queue>,
    keys: Arc<KeyCache>,
    pool: PgPool,
    cfg: trailscope_core::config::IngestConfig,
    retention_days: u32,
    /// Bounds how many batches are decoded at once, which bounds the memory
    /// held by in-flight requests.
    processing: Arc<Semaphore>,
}

pub struct Started {
    pub router: Router,
    pub keys: Arc<KeyCache>,
    pub writer: JoinHandle<()>,
}

/// Builds the ingest router and spawns the writer. Cancel `writer_stop` only
/// after the HTTP server has stopped accepting requests, then await `writer`.
pub fn start(
    config: &Config,
    ch: &ClickHouse,
    pool: PgPool,
    bus: LiveBus,
    writer_stop: CancellationToken,
) -> Started {
    let (queue, rx) = Queue::new(config.ingest.queue_capacity, config.ingest.max_queue_bytes);
    let keys = Arc::new(KeyCache::new(pool.clone()));
    let writer = tokio::spawn(pipeline::run_writer(
        ClickHouseSink {
            // Row types are covered by integration tests; skip per-insert schema checks.
            client: ch.writer.clone().with_validation(false),
        },
        queue.clone(),
        rx,
        bus,
        WriterConfig {
            flush_interval: Duration::from_millis(config.ingest.flush_interval_ms.max(10)),
            flush_rows: config.ingest.flush_rows,
            concurrency: config.ingest.writer_concurrency,
            shutdown_grace: Duration::from_secs(config.server.shutdown_timeout_secs),
        },
        writer_stop,
    ));
    let state = IngestState {
        queue,
        keys: keys.clone(),
        pool,
        cfg: config.ingest.clone(),
        retention_days: config.retention.days,
        processing: Arc::new(Semaphore::new(batch_permits(
            config.ingest.max_concurrent_batches,
        ))),
    };
    Started {
        router: router(state, config.ingest.max_body_bytes),
        keys,
        writer,
    }
}

fn batch_permits(configured: usize) -> usize {
    if configured > 0 {
        return configured;
    }
    std::thread::available_parallelism().map_or(4, |n| n.get() * 2)
}

fn router(state: IngestState, max_body: usize) -> Router {
    Router::new()
        .route(
            "/v1/ingest",
            post(ingest).layer(DefaultBodyLimit::max(max_body)),
        )
        .route(
            "/v1/releases",
            post(release).layer(DefaultBodyLimit::max(64 * 1024)),
        )
        .route("/v1/config", get(sdk_config))
        .route(
            "/v1/otlp/v1/traces",
            post(otlp_traces).layer(DefaultBodyLimit::max(max_body)),
        )
        .route(
            "/v1/otlp/v1/logs",
            post(otlp_logs).layer(DefaultBodyLimit::max(max_body)),
        )
        .with_state(state)
}

/// An error response: status, message and an optional `Retry-After`.
#[derive(Debug)]
pub struct IngestError {
    status: StatusCode,
    message: String,
    retry_after: Option<u64>,
}

impl IntoResponse for IngestError {
    fn into_response(self) -> Response {
        let mut resp = (self.status, Json(json!({ "error": self.message }))).into_response();
        if let Some(secs) = self.retry_after
            && let Ok(v) = HeaderValue::from_str(&secs.to_string())
        {
            resp.headers_mut().insert(header::RETRY_AFTER, v);
        }
        resp
    }
}

fn error(status: StatusCode, msg: impl Into<String>) -> IngestError {
    IngestError {
        status,
        message: msg.into(),
        retry_after: None,
    }
}

async fn authenticate(
    state: &IngestState,
    headers: &HeaderMap,
) -> Result<Arc<ProjectCtx>, IngestError> {
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim);
    let Some(token) = token else {
        return Err(error(
            StatusCode::UNAUTHORIZED,
            "missing Authorization: Bearer <ingest key>",
        ));
    };
    match state.keys.resolve(token).await {
        Ok(Some(ctx)) => Ok(ctx),
        Ok(None) => Err(error(
            StatusCode::UNAUTHORIZED,
            "invalid or revoked ingest key",
        )),
        Err(e) => {
            tracing::error!(error = %e, "ingest key lookup failed");
            Err(error(
                StatusCode::SERVICE_UNAVAILABLE,
                "key lookup unavailable",
            ))
        }
    }
}

fn too_busy(retry_after: u64) -> IngestError {
    metrics::counter!("trailscope_ingest_throttled_total").increment(1);
    IngestError {
        retry_after: Some(retry_after),
        ..error(
            StatusCode::TOO_MANY_REQUESTS,
            "ingest queue is full; retry later",
        )
    }
}

async fn ingest(State(state): State<IngestState>, headers: HeaderMap, body: Bytes) -> Response {
    let started = Instant::now();
    let resp = ingest_inner(&state, &headers, body).await;
    let status = resp.status().as_u16().to_string();
    metrics::counter!("trailscope_ingest_requests_total", "status" => status).increment(1);
    metrics::histogram!("trailscope_ingest_request_seconds")
        .record(started.elapsed().as_secs_f64());
    resp
}

async fn ingest_inner(state: &IngestState, headers: &HeaderMap, body: Bytes) -> Response {
    let ctx = match authenticate(state, headers).await {
        Ok(c) => c,
        Err(r) => return r.into_response(),
    };
    // Cheap early refusal before spending CPU on a batch we can't take.
    if state.queue.is_full() {
        return too_busy(state.cfg.retry_after_secs).into_response();
    }
    let Ok(_permit) = state.processing.acquire().await else {
        return error(StatusCode::SERVICE_UNAVAILABLE, "shutting down").into_response();
    };
    let encoding = headers
        .get(header::CONTENT_ENCODING)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);

    let big = body.len() > BLOCKING_THRESHOLD;
    let work = {
        let ctx = ctx.clone();
        let cfg = state.cfg.clone();
        let days = state.retention_days;
        move || process(&ctx, &cfg, days, encoding.as_deref(), &body)
    };
    let outcome = if big {
        match tokio::task::spawn_blocking(work).await {
            Ok(o) => o,
            Err(e) => {
                tracing::error!(error = %e, "ingest worker panicked");
                return error(StatusCode::INTERNAL_SERVER_ERROR, "internal error").into_response();
            }
        }
    } else {
        work()
    };
    let (batch, response) = match outcome {
        Ok(v) => v,
        Err(e) => return e.into_response(),
    };

    let accepted = batch.len();
    match state.queue.try_push(batch) {
        Ok(()) => {}
        Err(PushError::Full) => return too_busy(state.cfg.retry_after_secs).into_response(),
        Err(PushError::Closed) => {
            return error(StatusCode::SERVICE_UNAVAILABLE, "shutting down").into_response();
        }
    }
    metrics::counter!("trailscope_ingest_items_total", "result" => "accepted")
        .increment(accepted as u64);
    metrics::counter!("trailscope_ingest_items_total", "result" => "rejected")
        .increment(response.rejected as u64);

    let status = if response.rejected == 0 {
        StatusCode::OK
    } else if response.accepted == 0 {
        StatusCode::BAD_REQUEST
    } else {
        StatusCode::MULTI_STATUS
    };
    (status, Json(response)).into_response()
}

fn now_ms() -> i64 {
    (time::OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000) as i64
}

/// The CPU-bound part of ingest: decode, parse, validate, redact, fingerprint.
fn process(
    ctx: &ProjectCtx,
    cfg: &trailscope_core::config::IngestConfig,
    retention_days: u32,
    encoding: Option<&str>,
    body: &[u8],
) -> Result<(Batch, IngestResponse), IngestError> {
    let buf = decode::decompress(encoding, body, cfg.max_decompressed_bytes).map_err(|e| {
        let status = match e {
            decode::DecodeError::TooLarge => StatusCode::PAYLOAD_TOO_LARGE,
            decode::DecodeError::UnsupportedEncoding(_) => StatusCode::UNSUPPORTED_MEDIA_TYPE,
            _ => StatusCode::BAD_REQUEST,
        };
        error(status, e.to_string())
    })?;
    let items =
        decode::split_items(&buf).map_err(|e| error(StatusCode::BAD_REQUEST, e.to_string()))?;
    if items.len() > cfg.max_items {
        return Err(error(
            StatusCode::PAYLOAD_TOO_LARGE,
            format!(
                "batch has {} items; the limit is {}",
                items.len(),
                cfg.max_items
            ),
        ));
    }

    let now = now_ms();
    let lim = Limits {
        now_ms: now,
        oldest_ms: now - i64::from(retention_days) * 86_400_000,
        max_body_bytes: cfg.max_stored_body_bytes,
    };
    let mut batch = Batch::default();
    let mut errors = Vec::new();
    let pid = ctx.project_key.as_str();
    for (index, raw) in items.into_iter().enumerate() {
        let item = match serde_json::from_str::<WireItem>(raw) {
            Ok(i) => i,
            Err(e) => {
                errors.push(ItemError {
                    index,
                    message: item_parse_error(&e),
                });
                continue;
            }
        };
        let res = match item {
            WireItem::Request(r) => {
                normalize::request(*r, pid, &ctx.redactor, &lim).map(|row| batch.requests.push(row))
            }
            WireItem::Span(s) => normalize::span(*s, pid, &lim).map(|row| batch.spans.push(row)),
            WireItem::Event(e) => {
                normalize::event(*e, pid, &ctx.redactor, &lim).map(|row| batch.events.push(row))
            }
        };
        if let Err(message) = res {
            errors.push(ItemError { index, message });
        }
    }
    let response = IngestResponse {
        accepted: batch.len(),
        rejected: errors.len(),
        errors,
    };
    Ok((batch, response))
}

fn item_parse_error(e: &serde_json::Error) -> String {
    let msg = e.to_string();
    // serde's messages end with "at line 1 column 42"; the column is noise here.
    match msg.rfind(" at line ") {
        Some(i) => msg[..i].to_string(),
        None => msg,
    }
}

async fn release(State(state): State<IngestState>, headers: HeaderMap, body: Bytes) -> Response {
    let ctx = match authenticate(&state, &headers).await {
        Ok(c) => c,
        Err(r) => return r.into_response(),
    };
    let rel: WireRelease = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(e) => {
            return error(StatusCode::BAD_REQUEST, format!("invalid JSON: {e}")).into_response();
        }
    };
    if rel.service.trim().is_empty() || rel.version.trim().is_empty() {
        return error(StatusCode::BAD_REQUEST, "service and version are required").into_response();
    }
    let released_at = rel
        .timestamp
        .and_then(|t| {
            time::OffsetDateTime::from_unix_timestamp_nanos(i128::from(t.micros()) * 1000).ok()
        })
        .unwrap_or_else(time::OffsetDateTime::now_utc);
    let id = uuid::Uuid::now_v7();
    let res = sqlx::query(
        "INSERT INTO releases (id, project_id, environment, service, version, commit_sha, url, released_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .bind(id)
    .bind(ctx.project_id)
    .bind(rel.environment.unwrap_or_default().trim().to_string())
    .bind(rel.service.trim())
    .bind(rel.version.trim())
    .bind(rel.commit.filter(|c| !c.is_empty()))
    .bind(rel.url.filter(|u| !u.is_empty()))
    .bind(released_at)
    .execute(&state.pool)
    .await;
    match res {
        Ok(_) => (StatusCode::CREATED, Json(json!({ "id": id }))).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "inserting release");
            error(StatusCode::SERVICE_UNAVAILABLE, "could not record release").into_response()
        }
    }
}

/// Sampling and capture defaults for SDKs, so they can be tuned from the UI.
async fn sdk_config(State(state): State<IngestState>, headers: HeaderMap) -> Response {
    match authenticate(&state, &headers).await {
        Ok(ctx) => Json(&ctx.settings).into_response(),
        Err(r) => r.into_response(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Signal {
    Traces,
    Logs,
}

impl Signal {
    fn as_str(self) -> &'static str {
        match self {
            Signal::Traces => "traces",
            Signal::Logs => "logs",
        }
    }
}

/// `POST /v1/otlp/v1/traces` (OTLP/HTTP, protobuf or JSON).
async fn otlp_traces(
    State(state): State<IngestState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    otlp(&state, &headers, body, Signal::Traces).await
}

/// `POST /v1/otlp/v1/logs` (OTLP/HTTP, protobuf or JSON).
async fn otlp_logs(State(state): State<IngestState>, headers: HeaderMap, body: Bytes) -> Response {
    otlp(&state, &headers, body, Signal::Logs).await
}

async fn otlp(state: &IngestState, headers: &HeaderMap, body: Bytes, signal: Signal) -> Response {
    let started = Instant::now();
    let resp = otlp_inner(state, headers, body, signal).await;
    let status = resp.status().as_u16().to_string();
    metrics::counter!("trailscope_otlp_requests_total", "signal" => signal.as_str(), "status" => status)
        .increment(1);
    metrics::histogram!("trailscope_ingest_request_seconds")
        .record(started.elapsed().as_secs_f64());
    resp
}

async fn otlp_inner(
    state: &IngestState,
    headers: &HeaderMap,
    body: Bytes,
    signal: Signal,
) -> Response {
    let ctx = match authenticate(state, headers).await {
        Ok(c) => c,
        Err(r) => return r.into_response(),
    };
    if state.queue.is_full() {
        return too_busy(state.cfg.retry_after_secs).into_response();
    }
    let Ok(_permit) = state.processing.acquire().await else {
        return error(StatusCode::SERVICE_UNAVAILABLE, "shutting down").into_response();
    };
    let encoding = headers
        .get(header::CONTENT_ENCODING)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let json = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ct| ct.to_ascii_lowercase().contains("json"));

    let big = body.len() > BLOCKING_THRESHOLD;
    let work = {
        let ctx = ctx.clone();
        let cfg = state.cfg.clone();
        let days = state.retention_days;
        move || process_otlp(&ctx, &cfg, days, encoding.as_deref(), json, signal, &body)
    };
    let outcome = if big {
        match tokio::task::spawn_blocking(work).await {
            Ok(o) => o,
            Err(e) => {
                tracing::error!(error = %e, "otlp worker panicked");
                return error(StatusCode::INTERNAL_SERVER_ERROR, "internal error").into_response();
            }
        }
    } else {
        work()
    };
    let (batch, rejected, message) = match outcome {
        Ok(v) => v,
        Err(e) => return e.into_response(),
    };
    let accepted = batch.len();
    if accepted > 0 {
        match state.queue.try_push(batch) {
            Ok(()) => {}
            Err(PushError::Full) => return too_busy(state.cfg.retry_after_secs).into_response(),
            Err(PushError::Closed) => {
                return error(StatusCode::SERVICE_UNAVAILABLE, "shutting down").into_response();
            }
        }
    }
    metrics::counter!("trailscope_ingest_items_total", "result" => "accepted")
        .increment(accepted as u64);
    metrics::counter!("trailscope_ingest_items_total", "result" => "rejected")
        .increment(rejected as u64);
    otlp_response(json, signal, rejected, &message)
}

/// An `Export*ServiceResponse` in the request's encoding, with `partialSuccess`
/// when items were rejected.
fn otlp_response(json: bool, signal: Signal, rejected: usize, message: &str) -> Response {
    if json {
        let body = if rejected == 0 {
            json!({})
        } else {
            let field = if signal == Signal::Traces {
                "rejectedSpans"
            } else {
                "rejectedLogRecords"
            };
            json!({ "partialSuccess": { field: rejected.to_string(), "errorMessage": message } })
        };
        return (StatusCode::OK, Json(body)).into_response();
    }
    let msg = otlp::ExportResponse {
        partial_success: (rejected > 0).then(|| otlp::PartialSuccess {
            rejected: rejected as i64,
            error_message: message.to_string(),
        }),
    };
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/x-protobuf")],
        prost::Message::encode_to_vec(&msg),
    )
        .into_response()
}

/// Decode, map and normalize one OTLP export request.
fn process_otlp(
    ctx: &ProjectCtx,
    cfg: &trailscope_core::config::IngestConfig,
    retention_days: u32,
    encoding: Option<&str>,
    json: bool,
    signal: Signal,
    body: &[u8],
) -> Result<(Batch, usize, String), IngestError> {
    use prost::Message;

    let buf = decode::decompress(encoding, body, cfg.max_decompressed_bytes).map_err(|e| {
        let status = match e {
            decode::DecodeError::TooLarge => StatusCode::PAYLOAD_TOO_LARGE,
            decode::DecodeError::UnsupportedEncoding(_) => StatusCode::UNSUPPORTED_MEDIA_TYPE,
            _ => StatusCode::BAD_REQUEST,
        };
        error(status, e.to_string())
    })?;
    let bad = |e: String| {
        error(
            StatusCode::BAD_REQUEST,
            format!("invalid OTLP {} payload: {e}", signal.as_str()),
        )
    };
    let mapped = match (signal, json) {
        (Signal::Traces, true) => serde_json::from_slice::<otlp::ExportTraceServiceRequest>(&buf)
            .map(otlp::map_traces)
            .map_err(|e| bad(e.to_string()))?,
        (Signal::Traces, false) => otlp::ExportTraceServiceRequest::decode(buf.as_slice())
            .map(otlp::map_traces)
            .map_err(|e| bad(e.to_string()))?,
        (Signal::Logs, true) => serde_json::from_slice::<otlp::ExportLogsServiceRequest>(&buf)
            .map(otlp::map_logs)
            .map_err(|e| bad(e.to_string()))?,
        (Signal::Logs, false) => otlp::ExportLogsServiceRequest::decode(buf.as_slice())
            .map(otlp::map_logs)
            .map_err(|e| bad(e.to_string()))?,
    };
    let total = mapped.requests.len() + mapped.spans.len() + mapped.events.len();
    if total > cfg.max_items {
        return Err(error(
            StatusCode::PAYLOAD_TOO_LARGE,
            format!("export has {total} items; the limit is {}", cfg.max_items),
        ));
    }
    let now = now_ms();
    let lim = Limits {
        now_ms: now,
        oldest_ms: now - i64::from(retention_days) * 86_400_000,
        max_body_bytes: cfg.max_stored_body_bytes,
    };
    let pid = ctx.project_key.as_str();
    let mut batch = Batch::default();
    let mut rejected = 0;
    let mut message = String::new();
    let mut fail = |e: String| {
        rejected += 1;
        if message.is_empty() {
            message = e;
        }
    };
    for r in mapped.requests {
        match normalize::request(r, pid, &ctx.redactor, &lim) {
            Ok(row) => batch.requests.push(row),
            // The span itself is still stored; count it once, under spans.
            Err(e) => tracing::debug!(error = %e, "otlp request mapping rejected"),
        }
    }
    for s in mapped.spans {
        match normalize::span(s, pid, &lim) {
            Ok(row) => batch.spans.push(row),
            Err(e) => fail(e),
        }
    }
    for ev in mapped.events {
        match normalize::event(ev, pid, &ctx.redactor, &lim) {
            Ok(row) => batch.events.push(row),
            Err(e) => fail(e),
        }
    }
    Ok((batch, rejected, message))
}
