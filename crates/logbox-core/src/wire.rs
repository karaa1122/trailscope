//! The ingest wire format: what SDKs send to `POST /v1/ingest`.
//!
//! Field names are camelCase on the wire. Every field except the handful
//! needed to make sense of an item is optional, so SDKs can stay small.

use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer, Serialize};

/// One item in an ingest batch.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum WireItem {
    Request(Box<WireRequest>),
    Span(Box<WireSpan>),
    Event(Box<WireEvent>),
}

/// A timestamp: RFC 3339 string, or a number of milliseconds since the epoch.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(transparent)]
pub struct WireTime(
    /// Microseconds since the Unix epoch.
    pub i64,
);

impl WireTime {
    pub fn from_millis(ms: i64) -> Self {
        Self(ms.saturating_mul(1000))
    }

    pub fn micros(self) -> i64 {
        self.0
    }

    pub fn millis(self) -> i64 {
        self.0.div_euclid(1000)
    }
}

impl<'de> Deserialize<'de> for WireTime {
    fn deserialize<D: Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Num(f64),
            Str(String),
        }
        match Raw::deserialize(de)? {
            Raw::Num(ms) if ms.is_finite() => Ok(WireTime((ms * 1000.0).round() as i64)),
            Raw::Num(_) => Err(serde::de::Error::custom("timestamp must be finite")),
            Raw::Str(s) => parse_rfc3339_micros(&s)
                .map(WireTime)
                .ok_or_else(|| serde::de::Error::custom("timestamp must be RFC 3339 or epoch ms")),
        }
    }
}

fn parse_rfc3339_micros(s: &str) -> Option<i64> {
    let t = time::OffsetDateTime::parse(s, &time::format_description::well_known::Rfc3339).ok()?;
    i64::try_from(t.unix_timestamp_nanos() / 1000).ok()
}

/// A body: SDKs may send a string, or a JSON value that the server serializes.
#[derive(Debug, Clone, PartialEq, Serialize, Default)]
#[serde(transparent)]
pub struct WireBody(pub String);

impl<'de> Deserialize<'de> for WireBody {
    fn deserialize<D: Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        let v = serde_json::Value::deserialize(de)?;
        Ok(WireBody(match v {
            serde_json::Value::Null => String::new(),
            serde_json::Value::String(s) => s,
            other => other.to_string(),
        }))
    }
}

/// Attribute maps accept any scalar value and store it as a string.
#[derive(Debug, Clone, PartialEq, Serialize, Default)]
#[serde(transparent)]
pub struct WireMap(pub BTreeMap<String, String>);

impl<'de> Deserialize<'de> for WireMap {
    fn deserialize<D: Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        let raw = Option::<BTreeMap<String, serde_json::Value>>::deserialize(de)?;
        let mut out = BTreeMap::new();
        for (k, v) in raw.unwrap_or_default() {
            let s = match v {
                serde_json::Value::Null => continue,
                serde_json::Value::String(s) => s,
                other => other.to_string(),
            };
            out.insert(k, s);
        }
        Ok(WireMap(out))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct WireRequest {
    pub timestamp: Option<WireTime>,
    pub environment: Option<String>,
    pub service: String,
    pub release: Option<String>,

    pub request_id: Option<String>,
    pub trace_id: Option<String>,
    pub span_id: Option<String>,

    pub method: String,
    pub route: Option<String>,
    pub path: String,
    pub query: Option<String>,
    pub status: u16,
    pub duration_ms: f64,
    pub req_size: Option<u32>,
    pub res_size: Option<u32>,

    pub user_id: Option<String>,
    pub client: Option<String>,
    pub client_version: Option<String>,
    pub ip: Option<String>,
    pub user_agent: Option<String>,

    pub req_headers: WireMap,
    pub res_headers: WireMap,
    pub req_body: WireBody,
    pub res_body: WireBody,
    pub body_truncated: bool,

    pub error_type: Option<String>,
    pub error_message: Option<String>,
    pub error_stack: Option<String>,

    pub attributes: WireMap,
    pub correlation: WireMap,
    pub sample_rate: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct WireSpan {
    pub trace_id: String,
    pub span_id: String,
    pub parent_span_id: Option<String>,
    pub request_id: Option<String>,
    pub service: String,
    pub kind: Option<String>,
    pub name: String,
    pub start: Option<WireTime>,
    pub duration_ms: f64,
    pub status: Option<String>,
    pub attributes: WireMap,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct WireEvent {
    pub timestamp: Option<WireTime>,
    pub environment: Option<String>,
    pub service: String,
    pub release: Option<String>,
    pub kind: Option<String>,
    pub name: String,
    pub level: Option<String>,
    pub message: Option<String>,
    pub status: Option<String>,
    pub duration_ms: Option<f64>,
    pub request_id: Option<String>,
    pub trace_id: Option<String>,
    pub user_id: Option<String>,
    pub attempt: Option<u16>,
    pub error_type: Option<String>,
    pub error_message: Option<String>,
    pub error_stack: Option<String>,
    pub attributes: WireMap,
    pub correlation: WireMap,
}

/// `POST /v1/releases` body.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct WireRelease {
    pub service: String,
    pub version: String,
    pub environment: Option<String>,
    pub commit: Option<String>,
    pub url: Option<String>,
    pub timestamp: Option<WireTime>,
}

/// Response to `POST /v1/ingest`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct IngestResponse {
    pub accepted: usize,
    pub rejected: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<ItemError>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ItemError {
    pub index: usize,
    pub message: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tagged_items() {
        let json = r#"[
          {"type":"request","service":"api","method":"GET","path":"/users/42","route":"/users/:id",
           "status":200,"durationMs":12.5,"timestamp":"2026-01-02T03:04:05.678Z",
           "attributes":{"tenant":"acme","retries":2,"beta":true,"none":null},
           "reqBody":{"a":1}},
          {"type":"span","traceId":"t","spanId":"s","service":"api","name":"SELECT","durationMs":3,"start":1767323045678},
          {"type":"event","service":"worker","name":"report.generate","kind":"job","attempt":2}
        ]"#;
        let items: Vec<WireItem> = serde_json::from_str(json).unwrap();
        let WireItem::Request(r) = &items[0] else {
            panic!("not a request")
        };
        assert_eq!(r.route.as_deref(), Some("/users/:id"));
        assert_eq!(r.timestamp.unwrap().millis(), 1_767_323_045_678);
        assert_eq!(r.attributes.0["retries"], "2");
        assert_eq!(r.attributes.0["beta"], "true");
        assert!(!r.attributes.0.contains_key("none"));
        assert_eq!(r.req_body.0, r#"{"a":1}"#);
        let WireItem::Span(s) = &items[1] else {
            panic!("not a span")
        };
        assert_eq!(s.start.unwrap().millis(), 1_767_323_045_678);
        assert!(matches!(&items[2], WireItem::Event(e) if e.attempt == Some(2)));
    }

    #[test]
    fn rejects_unknown_type_and_bad_time() {
        assert!(serde_json::from_str::<WireItem>(r#"{"type":"metric"}"#).is_err());
        assert!(
            serde_json::from_str::<WireItem>(r#"{"type":"event","timestamp":"yesterday"}"#)
                .is_err()
        );
    }
}
