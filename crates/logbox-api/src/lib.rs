//! The Logbox HTTP API (`/api/v1/...`), used by the web UI and by API tokens.

pub mod auth;
pub mod error;
pub mod errors;
pub mod inspector;
pub mod journeys;
pub mod live;
pub mod openapi;
pub mod overview;
pub mod projects;
pub mod query;
pub mod setup;
pub mod users;
pub mod views;

use std::ops::Deref;
use std::sync::Arc;

use axum::Router;
use logbox_core::Config;
use logbox_core::ch::ClickHouse;
use logbox_core::live::LiveBus;
use logbox_ingest::keys::KeyCache;
use sqlx::PgPool;
use uuid::Uuid;

use crate::auth::LoginLimiter;
use crate::error::{ApiError, ApiResult};

pub struct Inner {
    pub config: Config,
    pub ch: ClickHouse,
    pub pool: PgPool,
    pub bus: LiveBus,
    /// The ingest key cache, when ingest runs in this process (to drop stale
    /// entries after key, rule or settings changes).
    pub keys: Option<Arc<KeyCache>>,
    pub limiter: LoginLimiter,
    /// HTTP client for replays: no redirects, short timeouts.
    pub http: reqwest::Client,
    /// Cancelled when the server shuts down, so long-lived streams end.
    pub shutdown: tokio_util::sync::CancellationToken,
}

#[derive(Clone)]
pub struct AppState(Arc<Inner>);

impl Deref for AppState {
    type Target = Inner;
    fn deref(&self) -> &Inner {
        &self.0
    }
}

impl AppState {
    pub fn new(
        config: Config,
        ch: ClickHouse,
        pool: PgPool,
        bus: LiveBus,
        keys: Option<Arc<KeyCache>>,
        shutdown: tokio_util::sync::CancellationToken,
    ) -> anyhow::Result<Self> {
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(std::time::Duration::from_secs(5))
            .timeout(std::time::Duration::from_secs(30))
            .user_agent("logbox-replay/0.1")
            .build()?;
        Ok(Self(Arc::new(Inner {
            config,
            ch,
            pool,
            bus,
            keys,
            limiter: LoginLimiter::default(),
            http,
            shutdown,
        })))
    }

    /// Forgets cached ingest settings for a project.
    pub fn invalidate_keys(&self, project: Uuid) {
        if let Some(k) = &self.keys {
            k.invalidate_project(project);
        }
    }

    /// Ensures a project exists; returns its ID as stored in ClickHouse.
    pub async fn project_key(&self, project: Uuid) -> ApiResult<String> {
        let exists: bool =
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM projects WHERE id = $1)")
                .bind(project)
                .fetch_one(&self.pool)
                .await?;
        if exists {
            Ok(project.to_string())
        } else {
            Err(ApiError::not_found("project"))
        }
    }
}

/// All API routes, plus the OpenAPI document and docs page.
pub fn router(state: AppState) -> Router {
    let (api, doc) = openapi::routes().split_for_parts();
    Router::new()
        .merge(api)
        .merge(openapi::docs(doc))
        .with_state(state)
}

pub(crate) fn now_ms() -> i64 {
    (time::OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000) as i64
}
