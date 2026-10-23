//! Server-side redaction (defense in depth: SDKs redact too).
//!
//! Rules:
//! - header names (case-insensitive),
//! - body keys at any depth (case-insensitive, `_`/`-` ignored, so
//!   `accessToken` matches `access_token`), or dotted paths such as
//!   `user.profile.dob` where `*` matches any key or array index,
//! - regexes applied to string values, header values and the query string.
//!
//! Redacted values become `"[REDACTED]"`.

use std::collections::HashSet;
use std::net::IpAddr;
use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;

pub const REDACTED: &str = "[REDACTED]";

pub const DEFAULT_HEADERS: &[&str] = &["authorization", "cookie", "set-cookie", "x-api-key"];

pub const DEFAULT_BODY_KEYS: &[&str] = &[
    "password",
    "passwd",
    "secret",
    "token",
    "access_token",
    "refresh_token",
    "otp",
    "pin",
    "cvv",
    "card",
    "card_number",
    "iban",
    "ssn",
];

/// Matches `"key": value` pairs for the default keys in text that isn't valid
/// JSON (typically a body the SDK truncated mid-document).
static RAW_DEFAULT_KEYS: LazyLock<Option<Regex>> =
    LazyLock::new(|| raw_key_regex(DEFAULT_BODY_KEYS));

fn raw_key_regex(keys: &[&str]) -> Option<Regex> {
    if keys.is_empty() {
        return None;
    }
    let alts: Vec<String> = keys
        .iter()
        .map(|k| {
            // access_token also matches accessToken / access-token
            k.split('_')
                .map(regex::escape)
                .collect::<Vec<_>>()
                .join("[_-]?")
        })
        .collect();
    Regex::new(&format!(
        r#"(?i)("(?:{})"\s*:\s*)("(?:[^"\\]|\\.)*"?|[^,}}\]\s]+)"#,
        alts.join("|")
    ))
    .ok()
}

#[derive(Debug, Clone)]
pub struct Redactor {
    headers: HashSet<String>,
    keys: HashSet<String>,
    paths: Vec<Vec<String>>,
    regexes: Vec<Regex>,
    raw_custom_keys: Option<Regex>,
    mask_ip: bool,
}

impl Default for Redactor {
    fn default() -> Self {
        Self::new(&[], &[], &[], true)
    }
}

/// A project-defined rule, as stored in Postgres.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rule {
    Header(String),
    BodyKey(String),
    Regex(String),
}

impl Redactor {
    pub fn new(
        headers: &[String],
        body_keys: &[String],
        regexes: &[String],
        mask_ip: bool,
    ) -> Self {
        let mut hs: HashSet<String> = DEFAULT_HEADERS.iter().map(|h| h.to_string()).collect();
        hs.extend(headers.iter().map(|h| h.trim().to_ascii_lowercase()));
        let mut keys: HashSet<String> = DEFAULT_BODY_KEYS.iter().map(|k| norm_key(k)).collect();
        let mut paths = Vec::new();
        let mut custom_simple = Vec::new();
        for k in body_keys {
            let k = k.trim();
            if k.contains('.') {
                paths.push(k.split('.').map(norm_key).collect());
            } else if !k.is_empty() {
                keys.insert(norm_key(k));
                custom_simple.push(k.to_string());
            }
        }
        let regexes = regexes
            .iter()
            .filter_map(|r| match Regex::new(r) {
                Ok(re) => Some(re),
                Err(e) => {
                    tracing::warn!(pattern = %r, error = %e, "ignoring invalid redaction regex");
                    None
                }
            })
            .collect();
        let custom_refs: Vec<&str> = custom_simple.iter().map(String::as_str).collect();
        Self {
            headers: hs,
            keys,
            paths,
            regexes,
            raw_custom_keys: raw_key_regex(&custom_refs),
            mask_ip,
        }
    }

    pub fn from_rules(rules: &[Rule], mask_ip: bool) -> Self {
        let (mut h, mut k, mut r) = (Vec::new(), Vec::new(), Vec::new());
        for rule in rules {
            match rule {
                Rule::Header(s) => h.push(s.clone()),
                Rule::BodyKey(s) => k.push(s.clone()),
                Rule::Regex(s) => r.push(s.clone()),
            }
        }
        Self::new(&h, &k, &r, mask_ip)
    }

    pub fn headers(&self, headers: &mut [(String, String)]) {
        for (name, value) in headers.iter_mut() {
            if self.headers.contains(name.as_str()) {
                *value = REDACTED.to_string();
            } else {
                self.apply_regexes(value);
            }
        }
    }

    /// Redacts a body. JSON is walked structurally; urlencoded form bodies are
    /// handled pair by pair; anything else gets the raw-text fallbacks.
    pub fn body(&self, body: &mut String, content_type: &str) {
        if body.is_empty() {
            return;
        }
        let trimmed = body.trim_start();
        if trimmed.starts_with('{') || trimmed.starts_with('[') {
            if let Ok(mut v) = serde_json::from_str::<Value>(body) {
                let mut path = Vec::new();
                if self.walk(&mut v, &mut path) {
                    *body = v.to_string();
                }
                return;
            }
            self.raw_text(body);
            return;
        }
        if content_type.contains("x-www-form-urlencoded") {
            *body = self.urlencoded(body);
            return;
        }
        self.raw_text(body);
    }

    /// `a=1&token=x` -> `a=1&token=[REDACTED]`
    pub fn urlencoded(&self, s: &str) -> String {
        s.split('&')
            .map(|pair| match pair.split_once('=') {
                Some((k, _)) if self.keys.contains(&norm_key(&percent_decode(k))) => {
                    format!("{k}={REDACTED}")
                }
                Some((k, v)) => {
                    let mut v = v.to_string();
                    self.apply_regexes(&mut v);
                    format!("{k}={v}")
                }
                None => pair.to_string(),
            })
            .collect::<Vec<_>>()
            .join("&")
    }

    pub fn query(&self, query: &mut String) {
        if !query.is_empty() {
            let q = query.strip_prefix('?').unwrap_or(query);
            *query = self.urlencoded(q);
        }
    }

    /// Masks the last IPv4 octet, or everything after the /48 for IPv6.
    pub fn ip(&self, ip: &mut String) {
        if self.mask_ip {
            *ip = mask_ip(ip);
        }
    }

    pub fn apply_regexes(&self, s: &mut String) {
        for re in &self.regexes {
            if re.is_match(s) {
                *s = re.replace_all(s, REDACTED).into_owned();
            }
        }
    }

    fn raw_text(&self, body: &mut String) {
        for re in [RAW_DEFAULT_KEYS.as_ref(), self.raw_custom_keys.as_ref()]
            .into_iter()
            .flatten()
        {
            if re.is_match(body) {
                *body = re
                    .replace_all(body, format!("${{1}}\"{REDACTED}\""))
                    .into_owned();
            }
        }
        self.apply_regexes(body);
    }

    /// Returns true if anything changed.
    fn walk(&self, v: &mut Value, path: &mut Vec<String>) -> bool {
        let mut changed = false;
        match v {
            Value::Object(map) => {
                for (k, child) in map.iter_mut() {
                    let nk = norm_key(k);
                    path.push(nk.clone());
                    if self.keys.contains(&nk) || self.path_matches(path) {
                        if !matches!(child, Value::Null) && child.as_str() != Some(REDACTED) {
                            *child = Value::String(REDACTED.into());
                            changed = true;
                        }
                    } else {
                        changed |= self.walk(child, path);
                    }
                    path.pop();
                }
            }
            Value::Array(items) => {
                for child in items.iter_mut() {
                    path.push("*".into());
                    changed |= self.walk(child, path);
                    path.pop();
                }
            }
            Value::String(s) => {
                let before = s.len();
                let mut owned = std::mem::take(s);
                self.apply_regexes(&mut owned);
                changed |= owned.len() != before || owned.contains(REDACTED);
                *s = owned;
            }
            _ => {}
        }
        changed
    }

    fn path_matches(&self, path: &[String]) -> bool {
        self.paths.iter().any(|p| {
            p.len() == path.len()
                && p.iter()
                    .zip(path)
                    .all(|(want, got)| want == "*" || got == "*" || want == got)
        })
    }
}

fn norm_key(k: &str) -> String {
    k.chars()
        .filter(|c| *c != '_' && *c != '-')
        .flat_map(char::to_lowercase)
        .collect()
}

fn percent_decode(s: &str) -> String {
    url::form_urlencoded::parse(format!("{s}=").as_bytes())
        .next()
        .map_or_else(|| s.to_string(), |(k, _)| k.into_owned())
}

pub fn mask_ip(ip: &str) -> String {
    match ip.trim().parse::<IpAddr>() {
        Ok(IpAddr::V4(v4)) => {
            let [a, b, c, _] = v4.octets();
            format!("{a}.{b}.{c}.0")
        }
        Ok(IpAddr::V6(v6)) => {
            let s = v6.segments();
            format!("{:x}:{:x}:{:x}::", s[0], s[1], s[2])
        }
        Err(_) => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r() -> Redactor {
        Redactor::new(
            &["x-session".into()],
            &[
                "dob".into(),
                "user.profile.nickname".into(),
                "items.*.code".into(),
            ],
            &[r"\b\d{3}-\d{2}-\d{4}\b".into()],
            true,
        )
    }

    #[test]
    fn redacts_default_and_custom_headers() {
        let mut h = vec![
            ("authorization".to_string(), "Bearer abc".to_string()),
            ("x-session".to_string(), "s".to_string()),
            ("accept".to_string(), "application/json".to_string()),
        ];
        r().headers(&mut h);
        assert_eq!(h[0].1, REDACTED);
        assert_eq!(h[1].1, REDACTED);
        assert_eq!(h[2].1, "application/json");
    }

    #[test]
    fn redacts_json_keys_at_any_depth_and_paths() {
        let mut body =
            r#"{"Password":"p","nested":{"accessToken":"t","ok":1,"list":[{"CVV":"123"}]},
            "user":{"profile":{"nickname":"n","name":"keep"}},"items":[{"code":"x"},{"code":"y"}],
            "note":"ssn 123-45-6789 here","dob":"2000-01-01","pinned":true}"#
                .to_string();
        r().body(&mut body, "application/json");
        let v: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["Password"], REDACTED);
        assert_eq!(v["nested"]["accessToken"], REDACTED);
        assert_eq!(v["nested"]["ok"], 1);
        assert_eq!(v["nested"]["list"][0]["CVV"], REDACTED);
        assert_eq!(v["user"]["profile"]["nickname"], REDACTED);
        assert_eq!(v["user"]["profile"]["name"], "keep");
        assert_eq!(v["items"][1]["code"], REDACTED);
        assert_eq!(v["note"], "ssn [REDACTED] here");
        assert_eq!(v["dob"], REDACTED);
        // `pinned` is not `pin`
        assert_eq!(v["pinned"], true);
    }

    #[test]
    fn keeps_key_order() {
        let mut body = r#"{"z":1,"a":2,"token":"t"}"#.to_string();
        r().body(&mut body, "application/json");
        assert_eq!(body, r#"{"z":1,"a":2,"token":"[REDACTED]"}"#);
    }

    #[test]
    fn redacts_truncated_json_text() {
        let mut body =
            r#"{"user":"a","password": "hunter2", "refresh-token":"zz","dob":"x","more":"#
                .to_string();
        r().body(&mut body, "application/json");
        assert!(!body.contains("hunter2"), "{body}");
        assert!(!body.contains("zz"), "{body}");
        assert!(body.contains(r#""dob":"[REDACTED]""#), "{body}");
        assert!(body.contains(r#""user":"a""#));
    }

    #[test]
    fn redacts_urlencoded_and_query() {
        let red = r();
        let mut form = "user=a&password=b&Access_Token=c".to_string();
        red.body(&mut form, "application/x-www-form-urlencoded");
        assert_eq!(form, "user=a&password=[REDACTED]&Access_Token=[REDACTED]");
        let mut q = "?page=2&token=abc".to_string();
        red.query(&mut q);
        assert_eq!(q, "page=2&token=[REDACTED]");
    }

    #[test]
    fn masks_ips() {
        assert_eq!(mask_ip("203.0.113.77"), "203.0.113.0");
        assert_eq!(mask_ip("2001:db8:abcd:12::1"), "2001:db8:abcd::");
        assert_eq!(mask_ip("not an ip"), "");
        let mut ip = "10.1.2.3".to_string();
        Redactor::new(&[], &[], &[], false).ip(&mut ip);
        assert_eq!(ip, "10.1.2.3");
    }

    #[test]
    fn invalid_regex_is_ignored() {
        let red = Redactor::new(&[], &[], &["(".into()], true);
        let mut s = "abc".to_string();
        red.apply_regexes(&mut s);
        assert_eq!(s, "abc");
    }
}
