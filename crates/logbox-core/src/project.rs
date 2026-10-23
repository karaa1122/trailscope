//! Per-project settings stored in `projects.settings` (JSONB). SDKs can fetch
//! the sampling and capture defaults from `GET /v1/config`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ProjectSettings {
    /// Mask the last IPv4 octet (or IPv6 beyond /48) server-side.
    pub mask_ip: bool,
    /// `errors` (default), `always` or `never`.
    pub body_capture: BodyCapture,
    /// Requests slower than this count as slow (body capture, sampling, tiles).
    pub slow_ms: u32,
    /// Sample rate for healthy requests. Errors and slow requests are always kept.
    pub sample_rate: f64,
    /// Per-route overrides of `sample_rate`, keyed by route pattern.
    pub route_sample_rates: BTreeMap<String, f64>,
    /// Routes the SDKs don't report at all.
    pub ignore_routes: Vec<String>,
    /// Per-body capture limit in the SDKs, in bytes.
    pub max_body_bytes: u32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BodyCapture {
    #[default]
    Errors,
    Always,
    Never,
}

impl Default for ProjectSettings {
    fn default() -> Self {
        Self {
            mask_ip: true,
            body_capture: BodyCapture::Errors,
            slow_ms: 1000,
            sample_rate: 1.0,
            route_sample_rates: BTreeMap::new(),
            ignore_routes: ["/health", "/healthz", "/ready", "/readyz", "/livez"]
                .map(String::from)
                .to_vec(),
            max_body_bytes: 8 * 1024,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_fields_take_defaults() {
        let s: ProjectSettings = serde_json::from_str(r#"{"sampleRate":0.5}"#).unwrap();
        assert_eq!(s.sample_rate, 0.5);
        assert!(s.mask_ip);
        assert_eq!(s.body_capture, BodyCapture::Errors);
        assert!(s.ignore_routes.contains(&"/healthz".to_string()));
    }
}
