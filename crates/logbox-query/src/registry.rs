//! The column registry: the single source of truth for what users can query.
//!
//! The compiler only ever emits column names from this list, the UI's
//! autocomplete and docs are generated from it (`GET /api/v1/schema`), and the
//! evaluator uses it to read live events.

use serde::Serialize;

/// The tables a query can target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Table {
    Requests,
    Events,
}

impl Table {
    pub fn sql_name(self) -> &'static str {
        match self {
            Table::Requests => "requests",
            Table::Events => "events",
        }
    }

    pub fn columns(self) -> &'static [Column] {
        match self {
            Table::Requests => REQUESTS,
            Table::Events => EVENTS,
        }
    }

    pub fn column(self, name: &str) -> Option<&'static Column> {
        let name = name.to_ascii_lowercase();
        self.columns().iter().find(|c| c.name == name)
    }

    /// Columns matched by search-syntax free text.
    pub fn free_text_columns(self) -> impl Iterator<Item = &'static Column> {
        self.columns().iter().filter(|c| c.free_text)
    }
}

/// How a column behaves in expressions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    String,
    Number,
    /// `UInt64` identifiers (fingerprints): compared exactly, never as floats.
    U64,
    Bool,
    Time,
    /// `Map(String, String)`, only usable through `col['key']`.
    Map,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Column {
    pub name: &'static str,
    /// The ClickHouse type, for display.
    pub ch_type: &'static str,
    pub kind: Kind,
    pub description: &'static str,
    pub filterable: bool,
    pub facetable: bool,
    pub groupable: bool,
    /// Bodies and headers: only for members with `can_view_bodies`.
    pub sensitive: bool,
    /// Matched by free text in search syntax.
    pub free_text: bool,
    /// Present in the live-tail summary, so usable in live filters.
    pub live: bool,
}

const fn col(
    name: &'static str,
    ch_type: &'static str,
    kind: Kind,
    description: &'static str,
) -> Column {
    Column {
        name,
        ch_type,
        kind,
        description,
        filterable: true,
        facetable: false,
        groupable: false,
        sensitive: false,
        free_text: false,
        live: true,
    }
}

impl Column {
    const fn facet(mut self) -> Self {
        self.facetable = true;
        self.groupable = true;
        self
    }
    const fn group(mut self) -> Self {
        self.groupable = true;
        self
    }
    const fn sensitive(mut self) -> Self {
        self.sensitive = true;
        self.live = false;
        self
    }
    const fn text(mut self) -> Self {
        self.free_text = true;
        self
    }
    const fn not_live(mut self) -> Self {
        self.live = false;
        self
    }
}

use Kind::{Bool, Map, Number, String as Str, Time, U64};

pub static REQUESTS: &[Column] = &[
    col(
        "timestamp",
        "DateTime64(3)",
        Time,
        "When the request started (UTC)",
    ),
    col(
        "environment",
        "LowCardinality(String)",
        Str,
        "Deployment environment, e.g. production",
    )
    .facet(),
    col(
        "service",
        "LowCardinality(String)",
        Str,
        "Service that handled the request",
    )
    .facet(),
    col(
        "release",
        "LowCardinality(String)",
        Str,
        "Version of the service",
    )
    .facet(),
    col("request_id", "String", Str, "Request ID (X-Request-Id)"),
    col("trace_id", "String", Str, "W3C trace ID"),
    col("span_id", "String", Str, "Span ID of the request").not_live(),
    col("method", "LowCardinality(String)", Str, "HTTP method").facet(),
    col(
        "route",
        "LowCardinality(String)",
        Str,
        "Route pattern, e.g. /users/:id",
    )
    .facet(),
    col("path", "String", Str, "Raw request path").text(),
    col("query", "String", Str, "Query string, without the ?"),
    col("status", "UInt16", Number, "HTTP status code").facet(),
    col(
        "duration_ms",
        "Float32",
        Number,
        "Server-side duration in milliseconds",
    ),
    col("req_size", "UInt32", Number, "Request body size in bytes"),
    col("res_size", "UInt32", Number, "Response body size in bytes"),
    col("user_id", "String", Str, "Authenticated user").facet(),
    col(
        "client",
        "LowCardinality(String)",
        Str,
        "Client type, e.g. web, ios, android, server",
    )
    .facet(),
    col(
        "client_version",
        "LowCardinality(String)",
        Str,
        "Client app version",
    )
    .facet(),
    col("ip", "String", Str, "Client IP (masked)").group(),
    col("user_agent", "String", Str, "User-Agent header").group(),
    col(
        "req_headers",
        "Map(String, String)",
        Map,
        "Request headers, e.g. req_headers['accept']",
    )
    .sensitive(),
    col(
        "res_headers",
        "Map(String, String)",
        Map,
        "Response headers",
    )
    .sensitive(),
    col(
        "req_body",
        "String",
        Str,
        "Request body (redacted, truncated)",
    )
    .sensitive(),
    col(
        "res_body",
        "String",
        Str,
        "Response body (redacted, truncated)",
    )
    .sensitive(),
    col(
        "body_truncated",
        "UInt8",
        Bool,
        "Whether a body was truncated",
    )
    .not_live(),
    col(
        "req_body_truncated",
        "UInt8",
        Bool,
        "Whether the request body was truncated (such requests can't be replayed)",
    )
    .not_live(),
    col(
        "error_type",
        "LowCardinality(String)",
        Str,
        "Exception class, e.g. TimeoutError",
    )
    .facet(),
    col("error_message", "String", Str, "Exception message").text(),
    col("error_stack", "String", Str, "Stack trace").not_live(),
    col("error_fingerprint", "UInt64", U64, "Error group ID").group(),
    col(
        "attributes",
        "Map(String, String)",
        Map,
        "Free-form attributes: attr['key']",
    ),
    col(
        "correlation",
        "Map(String, String)",
        Map,
        "Journey keys: correlation['account_id']",
    ),
    col(
        "sample_rate",
        "Float32",
        Number,
        "Fraction of similar requests that were kept",
    ),
];

pub static EVENTS: &[Column] = &[
    col(
        "timestamp",
        "DateTime64(3)",
        Time,
        "When the event happened (UTC)",
    ),
    col(
        "environment",
        "LowCardinality(String)",
        Str,
        "Deployment environment",
    )
    .facet(),
    col(
        "service",
        "LowCardinality(String)",
        Str,
        "Service or worker that emitted the event",
    )
    .facet(),
    col(
        "release",
        "LowCardinality(String)",
        Str,
        "Version of the service",
    )
    .facet(),
    col("kind", "LowCardinality(String)", Str, "job, log or custom").facet(),
    col(
        "name",
        "LowCardinality(String)",
        Str,
        "Job name, logger or event name",
    )
    .facet()
    .text(),
    col(
        "level",
        "LowCardinality(String)",
        Str,
        "debug, info, warn or error",
    )
    .facet(),
    col("message", "String", Str, "Log message").text(),
    col("status", "LowCardinality(String)", Str, "ok, warn or error").facet(),
    col("duration_ms", "Float32", Number, "Duration in milliseconds"),
    col(
        "request_id",
        "String",
        Str,
        "Request that triggered the event",
    ),
    col("trace_id", "String", Str, "W3C trace ID"),
    col("user_id", "String", Str, "User").facet(),
    col("attempt", "UInt16", Number, "Job attempt (1-based)").group(),
    col(
        "error_type",
        "LowCardinality(String)",
        Str,
        "Exception class",
    )
    .facet(),
    col("error_message", "String", Str, "Exception message").text(),
    col("error_stack", "String", Str, "Stack trace"),
    col("error_fingerprint", "UInt64", U64, "Error group ID").group(),
    col(
        "attributes",
        "Map(String, String)",
        Map,
        "Free-form attributes: attr['key']",
    ),
    col("correlation", "Map(String, String)", Map, "Journey keys"),
];

/// Names users may write for map columns.
pub fn map_alias(name: &str) -> Option<&'static str> {
    match name.to_ascii_lowercase().as_str() {
        "attr" | "attrs" | "attributes" => Some("attributes"),
        "correlation" | "corr" => Some("correlation"),
        "req_headers" => Some("req_headers"),
        "res_headers" => Some("res_headers"),
        _ => None,
    }
}

/// Whitelisted functions, for docs and autocomplete.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct FunctionDoc {
    pub name: &'static str,
    pub signature: &'static str,
    pub description: &'static str,
}

pub static FUNCTIONS: &[FunctionDoc] = &[
    FunctionDoc {
        name: "lower",
        signature: "lower(s) -> String",
        description: "Lower-cases a string (Unicode)",
    },
    FunctionDoc {
        name: "upper",
        signature: "upper(s) -> String",
        description: "Upper-cases a string (Unicode)",
    },
    FunctionDoc {
        name: "length",
        signature: "length(s) -> Number",
        description: "Length in characters",
    },
    FunctionDoc {
        name: "startsWith",
        signature: "startsWith(s, prefix) -> Bool",
        description: "Whether s starts with prefix",
    },
    FunctionDoc {
        name: "endsWith",
        signature: "endsWith(s, suffix) -> Bool",
        description: "Whether s ends with suffix",
    },
    FunctionDoc {
        name: "position",
        signature: "position(s, needle) -> Number",
        description: "1-based character position of needle in s, or 0",
    },
    FunctionDoc {
        name: "match",
        signature: "match(s, 'regex') -> Bool",
        description: "RE2 regular expression match (pattern up to 200 characters)",
    },
    FunctionDoc {
        name: "now",
        signature: "now() -> Time",
        description: "The current time; combine with INTERVAL n SECOND|MINUTE|HOUR|DAY",
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_unique_lowercase_identifiers() {
        for table in [Table::Requests, Table::Events] {
            let mut seen = std::collections::HashSet::new();
            for c in table.columns() {
                assert!(seen.insert(c.name), "duplicate {}", c.name);
                assert!(
                    c.name
                        .chars()
                        .all(|ch| ch.is_ascii_lowercase() || ch == '_'),
                    "{} must be a plain identifier: it is emitted into SQL verbatim",
                    c.name
                );
            }
        }
    }

    #[test]
    fn lookups() {
        assert_eq!(
            Table::Requests.column("STATUS").map(|c| c.kind),
            Some(Kind::Number)
        );
        assert_eq!(
            Table::Events.column("status").map(|c| c.kind),
            Some(Kind::String)
        );
        assert!(Table::Requests.column("system").is_none());
        assert_eq!(map_alias("attr"), Some("attributes"));
    }
}
