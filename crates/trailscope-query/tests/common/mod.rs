#![allow(dead_code)] // each test binary uses a subset

pub mod strategies;

use trailscope_query::compile::Sql;
use trailscope_query::registry::{EVENTS, REQUESTS};

/// Words that may appear in compiled SQL besides registry column names.
const ALLOWED_WORDS: &[&str] = &[
    "AND",
    "OR",
    "NOT",
    "IN",
    "BETWEEN",
    "like",
    "notLike",
    "ilike",
    "notILike",
    "empty",
    "startsWith",
    "endsWith",
    "match",
    "lowerUTF8",
    "upperUTF8",
    "lengthUTF8",
    "positionUTF8",
    "now64",
    "toIntervalSecond",
    "toIntervalMinute",
    "toIntervalHour",
    "toIntervalDay",
    "fromUnixTimestamp64Milli",
    "toUnixTimestamp64Milli",
    "project_id",
];

/// Asserts compiled SQL is built only from trusted pieces: no quotes,
/// comments or statement separators, every word is a registry column or an
/// allowed keyword/function, and every value is a `{pN:Type}` placeholder.
pub fn assert_safe(sql: &Sql) {
    let text = &sql.text;
    for bad in ["'", "\"", "`", ";", "--", "/*", "*/", "\\", "\0"] {
        assert!(!text.contains(bad), "compiled SQL contains {bad:?}: {text}");
    }
    let columns: Vec<&str> = REQUESTS.iter().chain(EVENTS).map(|c| c.name).collect();
    let mut rest = text.as_str();
    while !rest.is_empty() {
        let c = rest.chars().next().unwrap_or(' ');
        if c == '{' {
            let end = rest
                .find('}')
                .unwrap_or_else(|| panic!("unclosed placeholder in {text}"));
            let ph = &rest[1..end];
            let (name, ty) = ph
                .split_once(':')
                .unwrap_or_else(|| panic!("bad placeholder {ph}"));
            assert!(
                name.starts_with('p') && name[1..].chars().all(|d| d.is_ascii_digit()),
                "bad placeholder name {name} in {text}"
            );
            assert!(
                ["String", "Int64", "Float64", "UInt64", "UInt8"].contains(&ty),
                "bad placeholder type {ty} in {text}"
            );
            rest = &rest[end + 1..];
        } else if c.is_ascii_alphabetic() || c == '_' {
            let end = rest
                .find(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_'))
                .unwrap_or(rest.len());
            let word = &rest[..end];
            assert!(
                ALLOWED_WORDS.contains(&word) || columns.contains(&word),
                "unexpected word `{word}` in compiled SQL: {text}"
            );
            rest = &rest[end..];
        } else if c.is_ascii_digit() {
            // Only the `3` in now64(3).
            assert!(
                rest.starts_with("3)"),
                "bare number in compiled SQL: {text}"
            );
            rest = &rest[1..];
        } else {
            assert!(
                " ()[],=!<>+-".contains(c),
                "unexpected character {c:?} in compiled SQL: {text}"
            );
            rest = &rest[c.len_utf8()..];
        }
    }
    let placeholders = text.matches('{').count();
    assert_eq!(
        placeholders,
        sql.params.len(),
        "placeholder/param mismatch in {text}"
    );
}
