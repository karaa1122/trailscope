//! ClickHouse clients and the schema migrator.
//!
//! Two clients: `writer` (inserts, migrations, internal rollup reads) and
//! `reader`, which logs in as the restricted `logbox_reader` user. Every query
//! built from user input must go through `reader`.

use std::time::Duration;

use clickhouse::Client;
use serde::Deserialize;

use crate::config::{Config, QueryConfig};
use crate::error::{Error, Result};

/// Embedded migrations, applied in order. Add new files here.
const MIGRATIONS: &[(u32, &str, &str)] = &[
    (
        1,
        "init",
        include_str!("../../../migrations/clickhouse/0001_init.sql"),
    ),
    (
        2,
        "route_minute_order",
        include_str!("../../../migrations/clickhouse/0002_route_minute_order.sql"),
    ),
];

const READER_PROFILE: &str = "logbox_reader_profile";

#[derive(Clone)]
pub struct ClickHouse {
    pub writer: Client,
    pub reader: Client,
    pub database: String,
}

impl ClickHouse {
    pub fn new(config: &Config) -> Self {
        let ch = &config.clickhouse;
        let writer = Client::default()
            .with_url(&ch.url)
            .with_user(&ch.user)
            .with_password(&ch.password)
            .with_database(&ch.database);
        let reader = Client::default()
            .with_url(&ch.url)
            .with_user(&ch.reader_user)
            .with_password(&ch.reader_password)
            .with_database(&ch.database);
        Self {
            writer,
            reader: with_query_limits(reader, &config.query),
            database: ch.database.clone(),
        }
    }

    pub async fn ping(&self) -> Result<()> {
        self.writer
            .query("SELECT 1")
            .fetch_one::<u8>()
            .await
            .map(|_| ())
            .map_err(ch_err)
    }
}

/// Per-query limits for the reader. The settings profile caps them server-side
/// too, so a bug here can't raise them.
fn with_query_limits(client: Client, q: &QueryConfig) -> Client {
    client
        .with_setting("max_execution_time", q.max_execution_time_secs.to_string())
        .with_setting("max_rows_to_read", q.max_rows_to_read.to_string())
        .with_setting("max_result_rows", q.max_result_rows.to_string())
        .with_setting("max_memory_usage", q.max_memory_usage.to_string())
}

pub fn ch_err(e: clickhouse::error::Error) -> Error {
    Error::Other(format!("clickhouse: {e}"))
}

/// Splits a migration file into statements: strips `--` comment lines and
/// splits on `;` at the end of a line.
fn statements(sql: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for line in sql.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("--") || trimmed.is_empty() {
            continue;
        }
        cur.push_str(line);
        cur.push('\n');
        if trimmed.ends_with(';') {
            let stmt = cur.trim().trim_end_matches(';').trim().to_string();
            if !stmt.is_empty() {
                out.push(stmt);
            }
            cur.clear();
        }
    }
    let rest = cur.trim();
    if !rest.is_empty() {
        out.push(rest.to_string());
    }
    out
}

fn render(sql: &str, config: &Config) -> String {
    sql.replace("{{db}}", &config.clickhouse.database)
        .replace("{{days}}", &config.retention.days.to_string())
        .replace("{{body_days}}", &config.retention.body_days.to_string())
}

/// Escapes a value for a single-quoted ClickHouse string literal. Only used
/// for operator-supplied config (the reader password), never for user input.
fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\\', "\\\\").replace('\'', "\\'"))
}

#[derive(Debug, Deserialize, clickhouse::Row)]
struct MetaValue {
    value: String,
}

/// Applies pending migrations, (re)creates the reader user and syncs TTLs.
pub async fn migrate(config: &Config) -> Result<()> {
    let ch = &config.clickhouse;
    let db = &ch.database;
    let admin = Client::default()
        .with_url(&ch.url)
        .with_user(&ch.user)
        .with_password(&ch.password);

    wait_until_up(&admin).await?;

    exec(&admin, &format!("CREATE DATABASE IF NOT EXISTS {db}")).await?;
    exec(
        &admin,
        &format!(
            "CREATE TABLE IF NOT EXISTS {db}.schema_migrations \
             (version UInt32, name String, applied_at DateTime DEFAULT now()) \
             ENGINE = MergeTree ORDER BY version"
        ),
    )
    .await?;
    exec(
        &admin,
        &format!(
            "CREATE TABLE IF NOT EXISTS {db}.logbox_meta \
             (key String, value String, updated_at DateTime64(3) DEFAULT now64(3)) \
             ENGINE = ReplacingMergeTree(updated_at) ORDER BY key"
        ),
    )
    .await?;

    let applied: Vec<u32> = admin
        .query(&format!("SELECT version FROM {db}.schema_migrations"))
        .fetch_all::<u32>()
        .await
        .map_err(ch_err)?;
    let fresh = applied.is_empty();

    for (version, name, sql) in MIGRATIONS {
        if applied.contains(version) {
            continue;
        }
        tracing::info!(version, name, "applying clickhouse migration");
        for stmt in statements(&render(sql, config)) {
            exec(&admin, &stmt).await.map_err(|e| {
                Error::Other(format!("migration {version}_{name} failed: {e}\n{stmt}"))
            })?;
        }
        admin
            .query(&format!(
                "INSERT INTO {db}.schema_migrations (version, name) VALUES (?, ?)"
            ))
            .bind(*version)
            .bind(*name)
            .execute()
            .await
            .map_err(ch_err)?;
    }

    ensure_reader(&admin, config).await?;
    sync_ttl(&admin, config, fresh).await?;
    Ok(())
}

async fn wait_until_up(client: &Client) -> Result<()> {
    let mut delay = Duration::from_millis(250);
    for attempt in 1..=20 {
        match client.query("SELECT 1").fetch_one::<u8>().await {
            Ok(_) => return Ok(()),
            Err(e) if attempt < 20 => {
                tracing::warn!(attempt, error = %e, "waiting for clickhouse");
                tokio::time::sleep(delay).await;
                delay = (delay * 2).min(Duration::from_secs(5));
            }
            Err(e) => return Err(ch_err(e)),
        }
    }
    Err(Error::Other("clickhouse did not come up".into()))
}

async fn exec(client: &Client, sql: &str) -> Result<()> {
    client.query(sql).execute().await.map_err(ch_err)
}

/// Creates the restricted reader: readonly profile with hard caps, SELECT on
/// the Logbox database only.
async fn ensure_reader(admin: &Client, config: &Config) -> Result<()> {
    let ch = &config.clickhouse;
    let q = &config.query;
    let (db, user) = (&ch.database, &ch.reader_user);
    let pw = quote(&ch.reader_password);
    let profile = format!(
        "CREATE SETTINGS PROFILE OR REPLACE {READER_PROFILE} SETTINGS \
         readonly = 2, \
         max_execution_time = {t} MAX {t}, \
         max_rows_to_read = {r} MAX {r}, \
         max_result_rows = {res} MAX {res}, \
         max_memory_usage = {m} MAX {m}",
        t = q.max_execution_time_secs,
        r = q.max_rows_to_read,
        res = q.max_result_rows,
        m = q.max_memory_usage,
    );
    let stmts = [
        profile,
        format!("CREATE USER IF NOT EXISTS {user} IDENTIFIED WITH sha256_password BY {pw}"),
        format!(
            "ALTER USER {user} IDENTIFIED WITH sha256_password BY {pw} \
             SETTINGS PROFILE '{READER_PROFILE}'"
        ),
        format!("REVOKE ALL ON *.* FROM {user}"),
        format!("GRANT SELECT ON {db}.* TO {user}"),
    ];
    for stmt in &stmts {
        exec(admin, stmt).await.map_err(|e| {
            Error::Other(format!(
                "creating the ClickHouse reader user failed ({e}). The clickhouse.user account \
                 needs access management (CLICKHOUSE_DEFAULT_ACCESS_MANAGEMENT=1 in Docker)."
            ))
        })?;
    }
    Ok(())
}

/// Re-applies table and column TTLs when `retention.*` changed since the last run.
async fn sync_ttl(admin: &Client, config: &Config, fresh: bool) -> Result<()> {
    let db = &config.clickhouse.database;
    let (days, body_days) = (config.retention.days, config.retention.body_days);
    let wanted = format!("{days}/{body_days}");
    let current = admin
        .query(&format!(
            "SELECT value FROM {db}.logbox_meta FINAL WHERE key = 'ttl'"
        ))
        .fetch_optional::<MetaValue>()
        .await
        .map_err(ch_err)?
        .map(|m| m.value);

    if !fresh && current.as_deref() != Some(wanted.as_str()) {
        tracing::info!(from = ?current, to = %wanted, "retention changed; updating TTLs");
        let map = "Map(LowCardinality(String), String) CODEC(ZSTD(3))";
        let body_ttl = format!("TTL toDateTime(timestamp) + INTERVAL {body_days} DAY");
        let stmts = [
            format!(
                "ALTER TABLE {db}.requests MODIFY TTL toDateTime(timestamp) + INTERVAL {days} DAY"
            ),
            format!("ALTER TABLE {db}.requests MODIFY COLUMN req_headers {map} {body_ttl}"),
            format!("ALTER TABLE {db}.requests MODIFY COLUMN res_headers {map} {body_ttl}"),
            format!(
                "ALTER TABLE {db}.requests MODIFY COLUMN req_body String CODEC(ZSTD(3)) {body_ttl}"
            ),
            format!(
                "ALTER TABLE {db}.requests MODIFY COLUMN res_body String CODEC(ZSTD(3)) {body_ttl}"
            ),
            format!("ALTER TABLE {db}.spans MODIFY TTL toDateTime(start) + INTERVAL {days} DAY"),
            format!(
                "ALTER TABLE {db}.events MODIFY TTL toDateTime(timestamp) + INTERVAL {days} DAY"
            ),
            format!("ALTER TABLE {db}.route_minute MODIFY TTL minute + INTERVAL {days} DAY"),
            format!("ALTER TABLE {db}.error_minute MODIFY TTL minute + INTERVAL {days} DAY"),
            format!(
                "ALTER TABLE {db}.error_groups MODIFY TTL toDateTime(last_seen) + INTERVAL {days} DAY"
            ),
        ];
        for stmt in &stmts {
            // Don't rewrite existing parts now; new TTLs apply as parts merge.
            admin
                .query(stmt)
                .with_setting("materialize_ttl_after_modify", "0")
                .execute()
                .await
                .map_err(ch_err)?;
        }
    }
    if current.as_deref() != Some(wanted.as_str()) {
        admin
            .query(&format!(
                "INSERT INTO {db}.logbox_meta (key, value) VALUES ('ttl', ?)"
            ))
            .bind(&wanted)
            .execute()
            .await
            .map_err(ch_err)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_statements_and_strips_comments() {
        let sql = "-- header\nCREATE TABLE a\n(\n  x UInt8 -- trailing ok\n);\n\nSELECT 1;\n";
        let s = statements(sql);
        assert_eq!(s.len(), 2);
        assert!(s[0].starts_with("CREATE TABLE a"));
        assert_eq!(s[1], "SELECT 1");
    }

    #[test]
    fn renders_placeholders() {
        let cfg = Config::default();
        let out = render("{{db}}.t TTL {{days}} {{body_days}}", &cfg);
        assert_eq!(out, "logbox.t TTL 30 7");
    }

    #[test]
    fn migration_files_parse_into_statements() {
        let cfg = Config::default();
        for (_, _, sql) in MIGRATIONS {
            let stmts = statements(&render(sql, &cfg));
            assert!(stmts.len() >= 5);
            assert!(stmts.iter().all(|s| !s.contains("{{")));
        }
    }

    #[test]
    fn quotes_literals() {
        assert_eq!(quote(r"a'b\c"), r"'a\'b\\c'");
    }
}
