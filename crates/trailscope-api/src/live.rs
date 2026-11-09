//! Live tail over Server-Sent Events.
//!
//! The filter uses the same language as Explore and is evaluated in memory
//! per event. Each client gets at most `live.max_events_per_sec` events per
//! second; beyond that (and when the client falls behind) the stream sends a
//! `skipped` event with the number of matching events it dropped.

use std::convert::Infallible;
use std::time::{Duration, Instant};

use axum::extract::{Query, State};
use axum::response::sse::{Event, KeepAlive, Sse};
use futures::Stream;
use serde::Deserialize;
use serde_json::json;
use tokio::sync::broadcast::error::RecvError;
use trailscope_query::{Ctx, Prepared, QueryInput, Table};
use uuid::Uuid;

use crate::auth::{Auth, Role, Scope};
use crate::error::{ApiError, ApiResult};
use crate::{AppState, now_ms};

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct LiveQuery {
    pub project: Uuid,
    /// The filter text.
    #[serde(default)]
    pub filter: String,
    /// `sql` (default) or `search`.
    pub mode: Option<String>,
    pub environment: Option<String>,
}

/// Streams matching requests as `request` events (JSON), plus `skipped` and
/// a first `ready` event.
#[utoipa::path(get, path = "/api/v1/live", tag = "live", params(LiveQuery),
    responses((status = 200, description = "text/event-stream"), (status = 422, description = "invalid filter")))]
pub async fn live(
    State(state): State<AppState>,
    auth: Auth,
    Query(q): Query<LiveQuery>,
) -> ApiResult<Sse<impl Stream<Item = Result<Event, Infallible>>>> {
    auth.require(Role::Viewer, Scope::Read)?;
    auth.check_project(q.project)?;
    let project = state.project_key(q.project).await?;
    let ctx = Ctx {
        table: Table::Requests,
        allow_sensitive: auth.can_view_bodies(),
        live: true,
    };
    let input = match q.mode.as_deref() {
        Some("search") => QueryInput::Search {
            text: q.filter.clone(),
        },
        _ => QueryInput::Sql {
            text: q.filter.clone(),
        },
    };
    let expr = input.to_expr(&ctx).map_err(ApiError::invalid_query)?;
    let prepared = match &expr {
        Some(e) => Some(Prepared::new(e).map_err(ApiError::bad_request)?),
        None => None,
    };
    let env = q.environment.filter(|e| !e.is_empty());
    let cap = u64::from(state.config.live.max_events_per_sec.max(1));
    let mut rx = state.bus.subscribe_requests();
    let stop = state.shutdown.clone();
    metrics::gauge!("trailscope_live_clients").increment(1.0);

    let stream = async_stream::stream! {
        let _guard = ClientGuard;
        yield Ok(Event::default().event("ready").data("{}"));
        let mut window = Instant::now();
        let mut sent = 0u64;
        let mut skipped = 0u64;
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        loop {
            tokio::select! {
                () = stop.cancelled() => break,
                msg = rx.recv() => match msg {
                    Ok(ev) => {
                        if ev.project_id != project || env.as_ref().is_some_and(|e| *e != ev.environment) {
                            continue;
                        }
                        if prepared.as_ref().is_some_and(|p| !p.matches(&*ev, now_ms())) {
                            continue;
                        }
                        if window.elapsed() >= Duration::from_secs(1) {
                            window = Instant::now();
                            sent = 0;
                        }
                        if sent < cap {
                            sent += 1;
                            if let Ok(e) = Event::default().event("request").json_data(&*ev) {
                                yield Ok(e);
                            }
                        } else {
                            skipped += 1;
                        }
                    }
                    Err(RecvError::Lagged(n)) => skipped += n,
                    Err(RecvError::Closed) => break,
                },
                _ = tick.tick() => {
                    if skipped > 0 {
                        yield Ok(Event::default().event("skipped").data(json!({ "count": skipped }).to_string()));
                        skipped = 0;
                    }
                }
            }
        }
    };
    Ok(Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15))))
}

struct ClientGuard;

impl Drop for ClientGuard {
    fn drop(&mut self) {
        metrics::gauge!("trailscope_live_clients").decrement(1.0);
    }
}
