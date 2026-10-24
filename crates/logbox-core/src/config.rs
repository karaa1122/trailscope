//! Configuration, loaded from defaults, then `logbox.toml`, then `LOGBOX_*`
//! environment variables (highest precedence).
//!
//! Environment variables map onto sections with a single underscore after the
//! section name: `LOGBOX_CLICKHOUSE_URL` sets `clickhouse.url`,
//! `LOGBOX_INGEST_QUEUE_CAPACITY` sets `ingest.queue_capacity`.

use std::path::{Path, PathBuf};

use figment::{
    Figment,
    providers::{Env, Format, Serialized, Toml},
};
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// Top-level sections. Used to map flat env var names onto nested keys.
const SECTIONS: &[&str] = &[
    "server",
    "clickhouse",
    "postgres",
    "redis",
    "ingest",
    "retention",
    "query",
    "live",
    "alerts",
    "smtp",
    "oidc",
    "log",
];

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub server: ServerConfig,
    pub clickhouse: ClickHouseConfig,
    pub postgres: PostgresConfig,
    pub redis: RedisConfig,
    pub ingest: IngestConfig,
    pub retention: RetentionConfig,
    pub query: QueryConfig,
    pub live: LiveConfig,
    pub alerts: AlertsConfig,
    pub smtp: SmtpConfig,
    pub oidc: OidcConfig,
    pub log: LogConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ServerConfig {
    /// Address the HTTP server binds to.
    pub listen: String,
    /// Externally visible base URL, used in alert links and cookie security.
    pub public_url: String,
    /// Force the `Secure` cookie attribute. Defaults to on when `public_url` is https.
    pub secure_cookies: Option<bool>,
    /// Allowed CORS origins for the API (the embedded UI needs none).
    pub cors_origins: Vec<String>,
    /// Seconds to wait for in-flight work during shutdown.
    pub shutdown_timeout_secs: u64,
    /// Session lifetime in hours.
    pub session_ttl_hours: u64,
    /// Trust `X-Forwarded-For` for the client IP (only behind a reverse proxy you control).
    pub trust_proxy: bool,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            listen: "0.0.0.0:8080".into(),
            public_url: "http://localhost:8080".into(),
            secure_cookies: None,
            cors_origins: Vec::new(),
            shutdown_timeout_secs: 20,
            session_ttl_hours: 24 * 14,
            trust_proxy: false,
        }
    }
}

impl ServerConfig {
    pub fn cookies_secure(&self) -> bool {
        self.secure_cookies
            .unwrap_or_else(|| self.public_url.starts_with("https://"))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ClickHouseConfig {
    /// HTTP interface URL, e.g. `http://clickhouse:8123`.
    pub url: String,
    pub database: String,
    /// Read-write user used for migrations and inserts.
    pub user: String,
    #[serde(deserialize_with = "lenient_string")]
    pub password: String,
    /// Read-only user every user-driven query runs as. Created by `logbox migrate`.
    pub reader_user: String,
    #[serde(deserialize_with = "lenient_string")]
    pub reader_password: String,
}

impl Default for ClickHouseConfig {
    fn default() -> Self {
        Self {
            url: "http://localhost:8123".into(),
            database: "logbox".into(),
            user: "default".into(),
            password: String::new(),
            reader_user: "logbox_reader".into(),
            reader_password: "logbox_reader".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PostgresConfig {
    pub url: String,
    pub max_connections: u32,
}

impl Default for PostgresConfig {
    fn default() -> Self {
        Self {
            url: "postgres://logbox:logbox@localhost:5432/logbox".into(),
            max_connections: 10,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct RedisConfig {
    /// Redis URL. Empty means in-process live tail (only valid in `all` mode).
    pub url: String,
    pub channel: String,
}

impl RedisConfig {
    pub fn enabled(&self) -> bool {
        !self.url.trim().is_empty()
    }

    pub fn channel(&self) -> &str {
        if self.channel.is_empty() {
            "logbox:live"
        } else {
            &self.channel
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct IngestConfig {
    /// Maximum compressed request body size in bytes.
    pub max_body_bytes: usize,
    /// Maximum decompressed body size in bytes (zip-bomb guard).
    pub max_decompressed_bytes: usize,
    /// Maximum items per batch.
    pub max_items: usize,
    /// Rows the in-memory queue holds before ingest answers 429.
    pub queue_capacity: usize,
    /// Approximate bytes the queue holds before ingest answers 429.
    pub max_queue_bytes: usize,
    pub flush_interval_ms: u64,
    pub flush_rows: usize,
    /// ClickHouse inserts allowed in flight at once.
    pub writer_concurrency: usize,
    /// Batches decoded and validated at once; others wait. 0 = 2 x CPUs.
    pub max_concurrent_batches: usize,
    /// Request and response bodies longer than this are truncated server-side.
    pub max_stored_body_bytes: usize,
    /// Seconds suggested in `Retry-After` when the queue is full.
    pub retry_after_secs: u64,
}

impl Default for IngestConfig {
    fn default() -> Self {
        Self {
            max_body_bytes: 5 * 1024 * 1024,
            max_decompressed_bytes: 32 * 1024 * 1024,
            max_items: 10_000,
            queue_capacity: 200_000,
            max_queue_bytes: 256 * 1024 * 1024,
            flush_interval_ms: 1_000,
            flush_rows: 20_000,
            writer_concurrency: 4,
            max_concurrent_batches: 0,
            max_stored_body_bytes: 16 * 1024,
            retry_after_secs: 2,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RetentionConfig {
    /// Days raw rows are kept.
    pub days: u32,
    /// Days request/response bodies and headers are kept (column TTL).
    pub body_days: u32,
}

impl Default for RetentionConfig {
    fn default() -> Self {
        Self {
            days: 30,
            body_days: 7,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct QueryConfig {
    pub max_execution_time_secs: u64,
    pub max_rows_to_read: u64,
    pub max_result_rows: u64,
    pub max_memory_usage: u64,
    pub default_limit: u32,
    pub max_limit: u32,
}

impl Default for QueryConfig {
    fn default() -> Self {
        Self {
            max_execution_time_secs: 15,
            max_rows_to_read: 2_000_000_000,
            max_result_rows: 100_000,
            max_memory_usage: 2 * 1024 * 1024 * 1024,
            default_limit: 100,
            max_limit: 1_000,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LiveConfig {
    /// Per-client SSE cap; beyond it the stream emits `skipped` markers.
    pub max_events_per_sec: u32,
    /// Capacity of the in-process broadcast channel.
    pub channel_capacity: usize,
}

impl Default for LiveConfig {
    fn default() -> Self {
        Self {
            max_events_per_sec: 200,
            channel_capacity: 16_384,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AlertsConfig {
    pub interval_secs: u64,
    /// Default cooldown between notifications for the same rule, in seconds.
    pub default_cooldown_secs: u64,
}

impl Default for AlertsConfig {
    fn default() -> Self {
        Self {
            interval_secs: 60,
            default_cooldown_secs: 600,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SmtpConfig {
    /// SMTP host. Empty disables the email channel.
    pub host: String,
    pub port: u16,
    pub username: String,
    #[serde(deserialize_with = "lenient_string")]
    pub password: String,
    pub from: String,
    /// `starttls` (default), `tls` or `none`.
    pub security: String,
}

impl Default for SmtpConfig {
    fn default() -> Self {
        Self {
            host: String::new(),
            port: 587,
            username: String::new(),
            password: String::new(),
            from: "Logbox <logbox@localhost>".into(),
            security: "starttls".into(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct OidcConfig {
    /// Issuer URL. Empty disables OIDC login.
    pub issuer: String,
    #[serde(deserialize_with = "lenient_string")]
    pub client_id: String,
    #[serde(deserialize_with = "lenient_string")]
    pub client_secret: String,
    /// Label on the login button.
    pub display_name: String,
    /// Create a `viewer` member on first OIDC login when the email domain matches.
    pub auto_create_domains: Vec<String>,
}

impl OidcConfig {
    pub fn enabled(&self) -> bool {
        !self.issuer.is_empty() && !self.client_id.is_empty()
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogFormat {
    #[default]
    Pretty,
    Json,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LogConfig {
    pub level: String,
    pub format: LogFormat,
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            level: "info,logbox=info".into(),
            format: LogFormat::Pretty,
        }
    }
}

impl Config {
    /// Loads configuration. `path` (or `$LOGBOX_CONFIG`) must exist when given
    /// explicitly; the implicit `./logbox.toml` is optional.
    pub fn load(path: Option<&Path>) -> Result<Self> {
        let explicit = path
            .map(Path::to_path_buf)
            .or_else(|| std::env::var_os("LOGBOX_CONFIG").map(PathBuf::from));
        let file = explicit
            .clone()
            .unwrap_or_else(|| PathBuf::from("logbox.toml"));
        if explicit.is_some() && !file.exists() {
            return Err(Error::config(format!(
                "config file {} does not exist",
                file.display()
            )));
        }
        Self::figment(&file).extract::<Config>()?.validated()
    }

    fn figment(file: &Path) -> Figment {
        Figment::from(Serialized::defaults(Config::default()))
            .merge(Toml::file(file))
            .merge(
                Env::prefixed("LOGBOX_")
                    .ignore(&["config"])
                    .map(|key| map_env_key(key.as_str()).into()),
            )
    }

    fn validated(self) -> Result<Self> {
        if self.ingest.max_items == 0 || self.ingest.queue_capacity == 0 {
            return Err(Error::config(
                "ingest.max_items and ingest.queue_capacity must be > 0",
            ));
        }
        if self.ingest.flush_rows == 0 {
            return Err(Error::config("ingest.flush_rows must be > 0"));
        }
        if self.retention.body_days > self.retention.days {
            return Err(Error::config(
                "retention.body_days must be <= retention.days",
            ));
        }
        if self.retention.days == 0 {
            return Err(Error::config("retention.days must be > 0"));
        }
        if self.query.default_limit > self.query.max_limit {
            return Err(Error::config(
                "query.default_limit must be <= query.max_limit",
            ));
        }
        if !is_identifier(&self.clickhouse.database) || !is_identifier(&self.clickhouse.reader_user)
        {
            return Err(Error::config(
                "clickhouse.database and clickhouse.reader_user must be plain identifiers",
            ));
        }
        Ok(self)
    }
}

fn is_identifier(s: &str) -> bool {
    !s.is_empty()
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !s.starts_with(|c: char| c.is_ascii_digit())
}

/// Accepts strings and bare numbers, so `LOGBOX_CLICKHOUSE_PASSWORD=123456`
/// (which the env provider parses as an integer) still yields a string.
fn lenient_string<'de, D: serde::Deserializer<'de>>(de: D) -> Result<String, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Lenient {
        S(String),
        U(u64),
        I(i64),
        F(f64),
        B(bool),
    }
    Ok(match Lenient::deserialize(de)? {
        Lenient::S(s) => s,
        Lenient::U(n) => n.to_string(),
        Lenient::I(n) => n.to_string(),
        Lenient::F(n) => n.to_string(),
        Lenient::B(b) => b.to_string(),
    })
}

/// `clickhouse_reader_user` -> `clickhouse.reader_user`.
fn map_env_key(key: &str) -> String {
    let key = key.to_ascii_lowercase();
    for section in SECTIONS {
        if let Some(rest) = key.strip_prefix(section).and_then(|r| r.strip_prefix('_')) {
            return format!("{section}.{rest}");
        }
    }
    key
}

#[cfg(test)]
#[allow(clippy::result_large_err)] // figment::Jail closures return figment::Error
mod tests {
    use super::*;

    #[test]
    fn maps_env_keys_onto_sections() {
        assert_eq!(
            map_env_key("CLICKHOUSE_READER_USER"),
            "clickhouse.reader_user"
        );
        assert_eq!(
            map_env_key("ingest_queue_capacity"),
            "ingest.queue_capacity"
        );
        assert_eq!(map_env_key("unknown_key"), "unknown_key");
    }

    #[test]
    fn layers_defaults_file_and_env() {
        figment::Jail::expect_with(|jail| {
            jail.create_file(
                "logbox.toml",
                r#"
                [server]
                listen = "127.0.0.1:9999"
                [ingest]
                flush_rows = 5
                "#,
            )?;
            jail.set_env("LOGBOX_INGEST_FLUSH_ROWS", "7");
            jail.set_env("LOGBOX_CLICKHOUSE_PASSWORD", "s3cret");
            let cfg = Config::load(None).map_err(|e| e.to_string())?;
            assert_eq!(cfg.server.listen, "127.0.0.1:9999");
            assert_eq!(cfg.ingest.flush_rows, 7);
            assert_eq!(cfg.clickhouse.password, "s3cret");
            assert_eq!(cfg.retention.days, 30);
            Ok(())
        });
    }

    #[test]
    fn numeric_looking_secrets_stay_strings() {
        figment::Jail::expect_with(|jail| {
            jail.set_env("LOGBOX_POSTGRES_URL", "postgres://a:b@c/d");
            jail.set_env("LOGBOX_CLICKHOUSE_PASSWORD", "123456");
            let cfg = Config::load(None).map_err(|e| e.to_string())?;
            assert_eq!(cfg.clickhouse.password, "123456");
            Ok(())
        });
    }

    #[test]
    fn rejects_invalid_values() {
        figment::Jail::expect_with(|jail| {
            jail.set_env("LOGBOX_RETENTION_BODY_DAYS", "90");
            assert!(Config::load(None).is_err());
            Ok(())
        });
    }

    #[test]
    fn explicit_missing_file_is_an_error() {
        assert!(Config::load(Some(Path::new("/definitely/not/here.toml"))).is_err());
    }

    #[test]
    fn secure_cookies_follow_public_url() {
        let mut s = ServerConfig::default();
        assert!(!s.cookies_secure());
        s.public_url = "https://logs.example.com".into();
        assert!(s.cookies_secure());
    }
}
