//! Postgres pool and migrations (metadata only; telemetry is in ClickHouse).

use std::time::Duration;

use sqlx::postgres::{PgPool, PgPoolOptions};

use crate::config::PostgresConfig;
use crate::error::{Error, Result};

pub async fn connect(cfg: &PostgresConfig) -> Result<PgPool> {
    let mut delay = Duration::from_millis(250);
    let mut attempt = 0;
    loop {
        attempt += 1;
        match PgPoolOptions::new()
            .max_connections(cfg.max_connections)
            .acquire_timeout(Duration::from_secs(5))
            .connect(&cfg.url)
            .await
        {
            Ok(pool) => return Ok(pool),
            Err(e) if attempt < 20 => {
                tracing::warn!(attempt, error = %e, "waiting for postgres");
                tokio::time::sleep(delay).await;
                delay = (delay * 2).min(Duration::from_secs(5));
            }
            Err(e) => return Err(pg_err(e)),
        }
    }
}

pub async fn migrate(pool: &PgPool) -> Result<()> {
    sqlx::migrate!("../../migrations/postgres")
        .run(pool)
        .await
        .map_err(|e| Error::Other(format!("postgres migration failed: {e}")))
}

pub fn pg_err(e: sqlx::Error) -> Error {
    Error::Other(format!("postgres: {e}"))
}
