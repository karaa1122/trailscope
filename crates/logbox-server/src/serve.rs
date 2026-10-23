//! Process wiring for the long-running roles.

use std::time::Duration;

use anyhow::Context;
use axum::Router;
use logbox_core::ch::{self, ClickHouse};
use logbox_core::live::LiveBus;
use logbox_core::{Config, pg};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tower_http::request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer};
use tower_http::trace::TraceLayer;

use crate::ops;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    All,
    Ingest,
    Api,
    Alerts,
}

impl Role {
    fn ingest(self) -> bool {
        matches!(self, Role::All | Role::Ingest)
    }
}

pub async fn migrate(config: &Config) -> anyhow::Result<()> {
    ch::migrate(config).await.context("clickhouse migrations")?;
    let pool = pg::connect(&config.postgres)
        .await
        .context("connecting to postgres")?;
    pg::migrate(&pool).await.context("postgres migrations")?;
    tracing::info!("migrations applied");
    Ok(())
}

pub async fn run(config: Config, role: Role, shutdown: CancellationToken) -> anyhow::Result<()> {
    let metrics = ops::install_metrics()?;
    if role == Role::All {
        migrate(&config).await?;
    }
    let ch = ClickHouse::new(&config);
    let pool = pg::connect(&config.postgres)
        .await
        .context("connecting to postgres")?;
    let bus = if config.redis.enabled() {
        LiveBus::redis(&config.live, &config.redis, shutdown.clone())
            .await
            .context("connecting to redis")?
    } else {
        if role != Role::All {
            tracing::warn!(
                ?role,
                "redis.url is not set: live tail and real-time alerts only work in `all` mode"
            );
        }
        LiveBus::local(&config.live)
    };

    let writer_stop = CancellationToken::new();
    let mut writer = None;
    let mut app = Router::new();
    if role.ingest() {
        let started =
            logbox_ingest::start(&config, &ch, pool.clone(), bus.clone(), writer_stop.clone());
        app = app.merge(started.router);
        writer = Some(started.writer);
    }

    let app = app
        .merge(ops::router(ch.clone(), pool.clone(), metrics))
        .layer(PropagateRequestIdLayer::x_request_id())
        .layer(TraceLayer::new_for_http())
        .layer(SetRequestIdLayer::x_request_id(MakeRequestUuid));

    let listener = TcpListener::bind(&config.server.listen)
        .await
        .with_context(|| format!("binding {}", config.server.listen))?;
    tracing::info!(addr = %config.server.listen, ?role, "logbox listening");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown.clone().cancelled_owned())
        .await?;

    // HTTP has stopped accepting and in-flight requests are done: drain.
    tracing::info!("shutting down; draining ingest queue");
    writer_stop.cancel();
    if let Some(w) = writer {
        let grace = Duration::from_secs(config.server.shutdown_timeout_secs + 5);
        if tokio::time::timeout(grace, w).await.is_err() {
            tracing::error!("ingest writer did not finish in time");
        }
    }
    Ok(())
}
