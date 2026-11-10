//! Process wiring for the long-running roles.

use std::time::Duration;

use anyhow::Context;
use axum::Router;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tower_http::compression::CompressionLayer;
use tower_http::cors::{AllowHeaders, AllowMethods, AllowOrigin, CorsLayer};
use tower_http::request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer};
use tower_http::trace::TraceLayer;
use trailscope_core::ch::{self, ClickHouse};
use trailscope_core::live::LiveBus;
use trailscope_core::{Config, pg};

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

    fn api(self) -> bool {
        matches!(self, Role::All | Role::Api)
    }

    fn alerts(self) -> bool {
        matches!(self, Role::All | Role::Alerts)
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

    let secrets = trailscope_alerts::Keys::load(&config, &pool)
        .await
        .context("loading the secret key")?;
    if !secrets.from_config && (role.api() || role.alerts()) {
        tracing::warn!(
            "TRAILSCOPE_SECRET_KEY is not set: stored secrets are encrypted with a key kept in Postgres; set it in production"
        );
    }
    let mut alerts = None;
    if role.alerts() {
        alerts = Some(trailscope_alerts::start(
            config.clone(),
            ch.clone(),
            pool.clone(),
            bus.clone(),
            secrets.clone(),
            shutdown.clone(),
        ));
    }

    let writer_stop = CancellationToken::new();
    let mut writer = None;
    let mut keys = None;
    let mut app = Router::new();
    if role.ingest() {
        let started =
            trailscope_ingest::start(&config, &ch, pool.clone(), bus.clone(), writer_stop.clone());
        app = app.merge(started.router);
        writer = Some(started.writer);
        keys = Some(started.keys);
    }
    if role.api() {
        let admin_token = config.admin_token.trim();
        if !admin_token.is_empty() && admin_token.len() < trailscope_api::console::MIN_ADMIN_TOKEN {
            tracing::warn!(
                "TRAILSCOPE_ADMIN_TOKEN is shorter than 32 characters; the management API stays off"
            );
        }
        let state = trailscope_api::AppState::new(
            config.clone(),
            ch.clone(),
            pool.clone(),
            bus.clone(),
            keys.clone(),
            secrets.clone(),
            shutdown.clone(),
        )?;
        let mut api = trailscope_api::router(state).layer(CompressionLayer::new());
        if !config.server.cors_origins.is_empty() {
            api = api.layer(cors(&config.server.cors_origins)?);
        }
        app = app.merge(api).merge(crate::ui::router());
    }

    let app = app
        .merge(ops::router(ch.clone(), pool.clone(), metrics))
        .layer(PropagateRequestIdLayer::x_request_id())
        .layer(TraceLayer::new_for_http())
        .layer(SetRequestIdLayer::x_request_id(MakeRequestUuid));

    let listener = TcpListener::bind(&config.server.listen)
        .await
        .with_context(|| format!("binding {}", config.server.listen))?;
    tracing::info!(addr = %config.server.listen, ?role, "trailscope listening");
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
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
    if let Some(mut set) = alerts {
        // The tasks watch the shutdown token; give them a moment to save state.
        let _ = tokio::time::timeout(Duration::from_secs(5), async {
            while set.join_next().await.is_some() {}
        })
        .await;
    }
    Ok(())
}

/// CORS for API clients on other origins (the embedded UI needs none).
fn cors(origins: &[String]) -> anyhow::Result<CorsLayer> {
    let list = origins
        .iter()
        .map(|o| {
            o.parse()
                .with_context(|| format!("invalid CORS origin {o}"))
        })
        .collect::<anyhow::Result<Vec<axum::http::HeaderValue>>>()?;
    Ok(CorsLayer::new()
        .allow_origin(AllowOrigin::list(list))
        .allow_methods(AllowMethods::mirror_request())
        .allow_headers(AllowHeaders::mirror_request())
        .allow_credentials(true))
}
