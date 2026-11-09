//! Validation and normalization: wire items in, ClickHouse rows out.
//!
//! Required fields and impossible values are rejected per item with a message
//! naming the field. Over-long strings are truncated, not rejected: dropping
//! a whole request because of a long user agent helps nobody.

use std::sync::LazyLock;

use logbox_core::rows::{EventRow, RequestRow, SpanRow, StrMap};
use logbox_core::wire::{WireEvent, WireMap, WireRequest, WireSpan};
use regex::Regex;

use crate::fingerprint::fingerprint;
use crate::redact::Redactor;

pub struct Limits {
    pub now_ms: i64,
    pub oldest_ms: i64,
    pub max_body_bytes: usize,
}

const MAX_FUTURE_MS: i64 = 60 * 60 * 1000;
const MAX_ATTRS: usize = 64;
const MAX_HEADERS: usize = 100;

pub type ItemResult<T> = Result<T, String>;

fn required(field: &str, v: &str) -> ItemResult<()> {
    if v.trim().is_empty() {
        Err(format!("{field}: required"))
    } else {
        Ok(())
    }
}

/// Truncates to at most `max` bytes on a char boundary.
pub fn clip(mut s: String, max: usize) -> String {
    if s.len() > max {
        let mut cut = max;
        while !s.is_char_boundary(cut) {
            cut -= 1;
        }
        s.truncate(cut);
    }
    s
}

fn clip_opt(s: Option<String>, max: usize) -> String {
    clip(s.unwrap_or_default().trim().to_string(), max)
}

fn timestamp_ms(
    t: Option<logbox_core::wire::WireTime>,
    lim: &Limits,
    field: &str,
) -> ItemResult<i64> {
    let ms = t.map_or(lim.now_ms, |t| t.millis());
    check_time(ms, lim, field)?;
    Ok(ms)
}

fn check_time(ms: i64, lim: &Limits, field: &str) -> ItemResult<()> {
    if ms > lim.now_ms + MAX_FUTURE_MS {
        return Err(format!("{field}: more than 1 hour in the future"));
    }
    if ms < lim.oldest_ms {
        return Err(format!("{field}: older than the retention period"));
    }
    Ok(())
}

fn map(m: WireMap, max_val: usize) -> StrMap {
    m.0.into_iter()
        .filter(|(k, _)| !k.is_empty())
        .take(MAX_ATTRS)
        .map(|(k, v)| (clip(k, 128), clip(v, max_val)))
        .collect()
}

fn headers(m: WireMap) -> StrMap {
    m.0.into_iter()
        .take(MAX_HEADERS)
        .map(|(k, v)| (clip(k.to_ascii_lowercase(), 128), clip(v, 4096)))
        .collect()
}

fn duration(ms: f64, field: &str) -> ItemResult<f32> {
    if !ms.is_finite() || ms < 0.0 {
        return Err(format!("{field}: must be a non-negative number"));
    }
    if ms > 86_400_000.0 {
        return Err(format!("{field}: longer than a day"));
    }
    Ok(ms as f32)
}

static UUID_SEGMENT: LazyLock<Regex> = LazyLock::new(|| {
    #[allow(clippy::expect_used)] // constant pattern, covered by tests
    Regex::new(r"(?i)^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$")
        .expect("static regex")
});

/// Whether a path segment looks like an identifier rather than a word.
/// Anything that isn't purely numeric or a UUID must contain a digit, so
/// `access_tokens` or `internationalization` stay literal.
fn is_id_segment(s: &str) -> bool {
    let has_digit = s.bytes().any(|b| b.is_ascii_digit());
    if s.bytes().all(|b| b.is_ascii_digit()) || UUID_SEGMENT.is_match(s) {
        return true;
    }
    if !has_digit
        || !s
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return false;
    }
    let long_hex = s.len() >= 16 && s.bytes().all(|b| b.is_ascii_hexdigit());
    // usr_8f2k1x, acc-9x8y7z
    let prefixed = s.split_once(['_', '-']).is_some_and(|(prefix, rest)| {
        (1..=8).contains(&prefix.len())
            && prefix.bytes().all(|b| b.is_ascii_alphabetic())
            && rest.len() >= 4
            && rest.bytes().any(|b| b.is_ascii_digit())
    });
    let opaque = s.len() >= 20 && s.bytes().all(|b| b.is_ascii_alphanumeric());
    long_hex || prefixed || opaque
}

/// `/users/42/files/9f8e...` -> `/users/:id/files/:id`. Used only when the
/// SDK didn't send a route pattern.
pub fn route_from_path(path: &str) -> String {
    let segs: Vec<&str> = path.split('/').collect();
    let mut out = Vec::with_capacity(segs.len());
    for s in segs {
        if !s.is_empty() && is_id_segment(s) {
            out.push(":id");
        } else {
            out.push(s);
        }
    }
    let r = out.join("/");
    if r.is_empty() { "/".into() } else { r }
}

/// Splits `https://host/a/b?x=1` or `/a/b?x=1` into `(/a/b, x=1)`.
fn split_path(raw: &str) -> (String, String) {
    let without_origin = match raw.find("://") {
        Some(i) => {
            let rest = &raw[i + 3..];
            rest.find('/').map_or("/", |j| &rest[j..])
        }
        None => raw,
    };
    let (p, q) = without_origin
        .split_once('?')
        .unwrap_or((without_origin, ""));
    let p = if p.starts_with('/') {
        p.to_string()
    } else {
        format!("/{p}")
    };
    (p, q.to_string())
}

const METHODS: &[&str] = &[
    "GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS", "TRACE", "CONNECT",
];

pub fn request(
    w: WireRequest,
    project_id: &str,
    redactor: &Redactor,
    lim: &Limits,
) -> ItemResult<RequestRow> {
    required("service", &w.service)?;
    required("method", &w.method)?;
    required("path", &w.path)?;
    let method = w.method.trim().to_ascii_uppercase();
    if !METHODS.contains(&method.as_str())
        && (method.len() > 16 || !method.chars().all(|c| c.is_ascii_alphabetic()))
    {
        return Err("method: not an HTTP method".into());
    }
    if !(100..=599).contains(&w.status) {
        return Err("status: must be between 100 and 599".into());
    }
    let duration_ms = duration(w.duration_ms, "durationMs")?;
    let timestamp = timestamp_ms(w.timestamp, lim, "timestamp")?;
    let sample_rate = match w.sample_rate {
        None => 1.0,
        Some(r) if r > 0.0 && r <= 1.0 => r as f32,
        Some(_) => return Err("sampleRate: must be in (0, 1]".into()),
    };

    let (path, path_query) = split_path(w.path.trim());
    let path = clip(path, 2048);
    let mut query = clip(
        w.query
            .filter(|q| !q.is_empty())
            .map(|q| q.trim_start_matches('?').to_string())
            .unwrap_or(path_query),
        2048,
    );
    let route = match w
        .route
        .map(|r| r.trim().to_string())
        .filter(|r| !r.is_empty())
    {
        Some(r) => clip(r, 512),
        None => clip(route_from_path(&path), 512),
    };

    let mut req_headers = headers(w.req_headers);
    let mut res_headers = headers(w.res_headers);
    let content_type = |h: &StrMap| {
        h.iter()
            .find(|(k, _)| k == "content-type")
            .map(|(_, v)| v.to_ascii_lowercase())
            .unwrap_or_default()
    };
    let (req_ct, res_ct) = (content_type(&req_headers), content_type(&res_headers));
    redactor.headers(&mut req_headers);
    redactor.headers(&mut res_headers);
    redactor.query(&mut query);

    let mut req_body = w.req_body.0;
    let mut res_body = w.res_body.0;
    redactor.body(&mut req_body, &req_ct);
    redactor.body(&mut res_body, &res_ct);
    // Without `reqBodyTruncated` (older SDKs), `bodyTruncated` may mean either body.
    let mut req_truncated = w.req_body_truncated.unwrap_or(w.body_truncated);
    let mut truncated = w.body_truncated || req_truncated;
    if req_body.len() > lim.max_body_bytes {
        req_body = clip(std::mem::take(&mut req_body), lim.max_body_bytes);
        req_truncated = true;
        truncated = true;
    }
    if res_body.len() > lim.max_body_bytes {
        res_body = clip(std::mem::take(&mut res_body), lim.max_body_bytes);
        truncated = true;
    }

    let mut ip = clip_opt(w.ip, 64);
    redactor.ip(&mut ip);

    let service = clip(w.service.trim().to_string(), 128);
    let mut error_type = clip_opt(w.error_type, 256);
    let mut error_message = clip_opt(w.error_message, 4096);
    redactor.apply_regexes(&mut error_message);
    let error_stack = clip_opt(w.error_stack, 32 * 1024);
    // A 5xx without error details still deserves a group.
    if w.status >= 500
        && error_type.is_empty()
        && error_message.is_empty()
        && error_stack.is_empty()
    {
        error_type = format!("HTTP {}", w.status);
        error_message = format!("{method} {route} returned {}", w.status);
    }
    let error_fingerprint = if error_type.is_empty() && error_message.is_empty() {
        0
    } else {
        fingerprint(&service, &error_type, &error_message, &error_stack)
    };

    let req_size = w.req_size.unwrap_or(req_body.len() as u32);
    let res_size = w.res_size.unwrap_or(res_body.len() as u32);

    Ok(RequestRow {
        project_id: project_id.to_string(),
        environment: env_or_default(w.environment),
        service,
        release: clip_opt(w.release, 128),
        timestamp,
        request_id: w
            .request_id
            .map(|s| clip(s.trim().to_string(), 128))
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| uuid::Uuid::now_v7().simple().to_string()),
        trace_id: clip_opt(w.trace_id, 64),
        span_id: clip_opt(w.span_id, 32),
        method,
        route,
        path,
        query,
        status: w.status,
        duration_ms,
        req_size,
        res_size,
        user_id: clip_opt(w.user_id, 256),
        client: clip_opt(w.client, 64),
        client_version: clip_opt(w.client_version, 64),
        ip,
        user_agent: clip_opt(w.user_agent, 1024),
        req_headers,
        res_headers,
        req_body,
        res_body,
        body_truncated: u8::from(truncated),
        req_body_truncated: u8::from(req_truncated),
        error_type,
        error_message,
        error_stack,
        error_fingerprint,
        attributes: map(w.attributes, 2048),
        correlation: map(w.correlation, 256),
        sample_rate,
    })
}

fn env_or_default(env: Option<String>) -> String {
    let e = clip_opt(env, 64);
    if e.is_empty() { "production".into() } else { e }
}

const SPAN_KINDS: &[&str] = &["http", "db", "cache", "queue", "fn", "external"];

pub fn span(w: WireSpan, project_id: &str, lim: &Limits) -> ItemResult<SpanRow> {
    required("traceId", &w.trace_id)?;
    required("spanId", &w.span_id)?;
    required("service", &w.service)?;
    required("name", &w.name)?;
    let start = w.start.map_or(lim.now_ms * 1000, |t| t.micros());
    check_time(start.div_euclid(1000), lim, "start")?;
    let kind = w
        .kind
        .map(|k| k.trim().to_ascii_lowercase())
        .filter(|k| SPAN_KINDS.contains(&k.as_str()))
        .unwrap_or_else(|| "fn".into());
    let status = match w.status.as_deref().map(str::trim) {
        Some(s) if s.eq_ignore_ascii_case("error") => "error",
        _ => "ok",
    };
    Ok(SpanRow {
        project_id: project_id.to_string(),
        trace_id: clip(w.trace_id.trim().to_string(), 64),
        span_id: clip(w.span_id.trim().to_string(), 32),
        parent_span_id: clip_opt(w.parent_span_id, 32),
        request_id: clip_opt(w.request_id, 128),
        service: clip(w.service.trim().to_string(), 128),
        kind,
        name: clip(w.name.trim().to_string(), 1024),
        start,
        duration_ms: duration(w.duration_ms, "durationMs")?,
        status: status.into(),
        attributes: map(w.attributes, 2048),
    })
}

const EVENT_KINDS: &[&str] = &["job", "log", "custom"];
const LEVELS: &[&str] = &["debug", "info", "warn", "error"];

pub fn event(
    w: WireEvent,
    project_id: &str,
    redactor: &Redactor,
    lim: &Limits,
) -> ItemResult<EventRow> {
    required("service", &w.service)?;
    required("name", &w.name)?;
    let kind = match w.kind.as_deref().map(|k| k.trim().to_ascii_lowercase()) {
        None => "custom".to_string(),
        Some(k) if EVENT_KINDS.contains(&k.as_str()) => k,
        Some(_) => return Err("kind: must be job, log or custom".into()),
    };
    let level = match w.level.as_deref().map(|l| l.trim().to_ascii_lowercase()) {
        None => "info".to_string(),
        Some(l) if l == "warning" => "warn".to_string(),
        Some(l) if l == "fatal" || l == "critical" => "error".to_string(),
        Some(l) if LEVELS.contains(&l.as_str()) => l,
        Some(_) => return Err("level: must be debug, info, warn or error".into()),
    };
    let status = match w.status.as_deref().map(|s| s.trim().to_ascii_lowercase()) {
        Some(s) if ["ok", "warn", "error"].contains(&s.as_str()) => s,
        Some(_) => return Err("status: must be ok, warn or error".into()),
        None => match level.as_str() {
            "error" => "error".into(),
            "warn" => "warn".into(),
            _ => "ok".into(),
        },
    };
    let duration_ms = match w.duration_ms {
        Some(d) => duration(d, "durationMs")?,
        None => 0.0,
    };
    let service = clip(w.service.trim().to_string(), 128);
    let name = clip(w.name.trim().to_string(), 256);
    let mut message = clip_opt(w.message, 16 * 1024);
    redactor.apply_regexes(&mut message);
    let error_type = clip_opt(w.error_type, 256);
    let mut error_message = clip_opt(w.error_message, 4096);
    redactor.apply_regexes(&mut error_message);
    let error_stack = clip_opt(w.error_stack, 32 * 1024);
    let error_fingerprint = if status == "error" {
        let msg = if error_message.is_empty() {
            &message
        } else {
            &error_message
        };
        // Group failed jobs by name when they carry no error details.
        let ty = if error_type.is_empty() && msg.is_empty() {
            &name
        } else {
            &error_type
        };
        fingerprint(&service, ty, msg, &error_stack)
    } else {
        0
    };
    Ok(EventRow {
        project_id: project_id.to_string(),
        environment: env_or_default(w.environment),
        service,
        release: clip_opt(w.release, 128),
        timestamp: timestamp_ms(w.timestamp, lim, "timestamp")?,
        kind,
        name,
        level,
        message,
        status,
        duration_ms,
        request_id: clip_opt(w.request_id, 128),
        trace_id: clip_opt(w.trace_id, 64),
        user_id: clip_opt(w.user_id, 256),
        attempt: w.attempt.unwrap_or(0),
        error_type,
        error_message,
        error_stack,
        error_fingerprint,
        attributes: map(w.attributes, 2048),
        correlation: map(w.correlation, 256),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use logbox_core::wire::{WireBody, WireTime};

    fn lim() -> Limits {
        Limits {
            now_ms: 1_800_000_000_000,
            oldest_ms: 1_800_000_000_000 - 30 * 86_400_000,
            max_body_bytes: 16,
        }
    }

    fn base() -> WireRequest {
        WireRequest {
            service: "api".into(),
            method: "get".into(),
            path: "/users/42".into(),
            status: 200,
            duration_ms: 12.0,
            ..Default::default()
        }
    }

    #[test]
    fn fills_defaults_and_route() {
        let r = request(base(), "p1", &Redactor::default(), &lim()).unwrap();
        assert_eq!(r.method, "GET");
        assert_eq!(r.route, "/users/:id");
        assert_eq!(r.environment, "production");
        assert_eq!(r.timestamp, lim().now_ms);
        assert_eq!(r.request_id.len(), 32);
        assert_eq!(r.sample_rate, 1.0);
        assert_eq!(r.error_fingerprint, 0);
    }

    #[test]
    fn route_templating() {
        assert_eq!(route_from_path("/"), "/");
        assert_eq!(
            route_from_path("/v1/files/4f1c8e5a-2b7d-4c1e-9a3f-0e5d6c7b8a90/parts/7"),
            "/v1/files/:id/parts/:id"
        );
        assert_eq!(
            route_from_path("/accounts/acc_9x8y7z6w/settings"),
            "/accounts/:id/settings"
        );
        assert_eq!(route_from_path("/search"), "/search");
        assert_eq!(
            route_from_path("/oauth/access_tokens"),
            "/oauth/access_tokens"
        );
        assert_eq!(
            route_from_path("/v2/internationalization"),
            "/v2/internationalization"
        );
        assert_eq!(route_from_path("/blobs/a1b2c3d4e5f60718"), "/blobs/:id");
    }

    #[test]
    fn splits_full_urls_and_query() {
        let mut w = base();
        w.path = "https://api.example.com/search?q=x&token=abc".into();
        let r = request(w, "p1", &Redactor::default(), &lim()).unwrap();
        assert_eq!(r.path, "/search");
        assert_eq!(r.query, "q=x&token=[REDACTED]");
    }

    #[test]
    fn rejects_bad_fields() {
        type Mutator = fn(&mut WireRequest);
        let cases: Vec<(Mutator, &str)> = vec![
            (|w| w.service = " ".into(), "service: required"),
            (|w| w.status = 42, "status: must be between 100 and 599"),
            (
                |w| w.duration_ms = -1.0,
                "durationMs: must be a non-negative number",
            ),
            (|w| w.method = "G E T".into(), "method: not an HTTP method"),
            (
                |w| w.sample_rate = Some(0.0),
                "sampleRate: must be in (0, 1]",
            ),
            (
                |w| w.timestamp = Some(WireTime::from_millis(1_800_000_000_000 + 7_200_000)),
                "timestamp: more than 1 hour in the future",
            ),
            (
                |w| w.timestamp = Some(WireTime::from_millis(1)),
                "timestamp: older than the retention period",
            ),
        ];
        for (mutate, msg) in cases {
            let mut w = base();
            mutate(&mut w);
            assert_eq!(
                request(w, "p", &Redactor::default(), &lim()).unwrap_err(),
                msg
            );
        }
    }

    #[test]
    fn truncates_bodies_after_redaction_and_masks_ip() {
        let mut w = base();
        w.req_body = WireBody(r#"{"password":"secret-value-here","x":"yyyyyyyyyyyyyy"}"#.into());
        w.ip = Some("198.51.100.23".into());
        let r = request(w, "p1", &Redactor::default(), &lim()).unwrap();
        assert_eq!(r.req_body.len(), 16);
        assert!(!r.req_body.contains("secret"));
        assert_eq!(r.body_truncated, 1);
        assert_eq!(r.ip, "198.51.100.0");
    }

    #[test]
    fn server_errors_always_get_a_group() {
        let mut w = base();
        w.status = 503;
        let r = request(w, "p1", &Redactor::default(), &lim()).unwrap();
        assert_eq!(r.error_type, "HTTP 503");
        assert_ne!(r.error_fingerprint, 0);
    }

    #[test]
    fn events_validate_kind_level_and_fingerprint_failures() {
        let w = WireEvent {
            service: "worker".into(),
            name: "report.generate".into(),
            kind: Some("job".into()),
            level: Some("WARNING".into()),
            ..Default::default()
        };
        let e = event(w.clone(), "p", &Redactor::default(), &lim()).unwrap();
        assert_eq!((e.level.as_str(), e.status.as_str()), ("warn", "warn"));
        assert_eq!(e.error_fingerprint, 0);

        let failed = WireEvent {
            status: Some("error".into()),
            ..w.clone()
        };
        assert_ne!(
            event(failed, "p", &Redactor::default(), &lim())
                .unwrap()
                .error_fingerprint,
            0
        );
        let bad = WireEvent {
            kind: Some("metric".into()),
            ..w
        };
        assert_eq!(
            event(bad, "p", &Redactor::default(), &lim()).unwrap_err(),
            "kind: must be job, log or custom"
        );
    }

    #[test]
    fn spans_default_kind_and_status() {
        let w = WireSpan {
            trace_id: "t".into(),
            span_id: "s".into(),
            service: "api".into(),
            name: "SELECT users".into(),
            kind: Some("weird".into()),
            duration_ms: 3.0,
            ..Default::default()
        };
        let s = span(w, "p", &lim()).unwrap();
        assert_eq!((s.kind.as_str(), s.status.as_str()), ("fn", "ok"));
        assert_eq!(s.start, lim().now_ms * 1000);
    }
}
