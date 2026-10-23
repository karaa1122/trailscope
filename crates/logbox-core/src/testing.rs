//! Helpers for integration tests that need real ClickHouse and Postgres.
//!
//! Tests read `LOGBOX_TEST_CLICKHOUSE_URL` (+ `_USER`, `_PASSWORD`) and
//! `LOGBOX_TEST_POSTGRES_URL`. When they are unset the helpers return `None`
//! and the test should return early, so `cargo test` stays green without
//! services. Each call gets its own ClickHouse database, reader user and
//! Postgres database, so tests can run in parallel.

use rand::distr::{Alphanumeric, SampleString};
use sqlx::{Connection, Executor, PgConnection};

use crate::config::Config;

pub struct TestEnv {
    pub config: Config,
}

/// Returns a config pointing at fresh, empty test databases, or `None` when
/// the test services are not configured.
pub async fn env() -> Option<TestEnv> {
    let ch_url = std::env::var("LOGBOX_TEST_CLICKHOUSE_URL").ok()?;
    let pg_url = std::env::var("LOGBOX_TEST_POSTGRES_URL").ok()?;
    let suffix = Alphanumeric
        .sample_string(&mut rand::rng(), 10)
        .to_ascii_lowercase();

    let mut config = Config::default();
    config.clickhouse.url = ch_url;
    config.clickhouse.user =
        std::env::var("LOGBOX_TEST_CLICKHOUSE_USER").unwrap_or_else(|_| "default".into());
    config.clickhouse.password =
        std::env::var("LOGBOX_TEST_CLICKHOUSE_PASSWORD").unwrap_or_default();
    config.clickhouse.database = format!("logbox_test_{suffix}");
    config.clickhouse.reader_user = format!("logbox_reader_{suffix}");
    config.clickhouse.reader_password = format!("reader_{suffix}");

    let db_name = format!("logbox_test_{suffix}");
    let mut admin = PgConnection::connect(&pg_url).await.ok()?;
    admin
        // db_name is generated above from [a-z0-9] only.
        .execute(sqlx::AssertSqlSafe(format!("CREATE DATABASE {db_name}")))
        .await
        .ok()?;
    let base = pg_url.rsplit_once('/').map_or(pg_url.as_str(), |(b, _)| b);
    config.postgres.url = format!("{base}/{db_name}");
    config.postgres.max_connections = 5;
    Some(TestEnv { config })
}
