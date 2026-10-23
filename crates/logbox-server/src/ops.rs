//! Operational endpoints: `/healthz`, `/readyz`, `/metrics`.

use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use logbox_core::ch::ClickHouse;
use metrics_exporter_prometheus::{Matcher, PrometheusBuilder, PrometheusHandle};
use serde_json::json;
use sqlx::PgPool;

#[derive(Clone)]
struct OpsState {
    ch: ClickHouse,
    pool: PgPool,
    metrics: PrometheusHandle,
}

const LATENCY_BUCKETS: &[f64] = &[
    0.001, 0.0025, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 15.0,
];

pub fn install_metrics() -> anyhow::Result<PrometheusHandle> {
    let handle = PrometheusBuilder::new()
        .set_buckets_for_metric(Matcher::Suffix("_seconds".into()), LATENCY_BUCKETS)?
        .install_recorder()?;
    Ok(handle)
}

pub fn router(ch: ClickHouse, pool: PgPool, metrics: PrometheusHandle) -> Router {
    Router::new()
        .route(
            "/healthz",
            get(|| async { Json(json!({ "status": "ok" })) }),
        )
        .route("/readyz", get(readyz))
        .route("/metrics", get(render_metrics))
        .with_state(OpsState { ch, pool, metrics })
}

async fn readyz(State(s): State<OpsState>) -> impl IntoResponse {
    let ch = s.ch.ping().await.map_err(|e| e.to_string());
    let pg = sqlx::query_scalar::<_, i32>("SELECT 1")
        .fetch_one(&s.pool)
        .await
        .map(|_| ())
        .map_err(|e| e.to_string());
    let status = if ch.is_ok() && pg.is_ok() {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    let show = |r: &Result<(), String>| match r {
        Ok(()) => json!("ok"),
        Err(e) => json!(e),
    };
    (
        status,
        Json(json!({ "clickhouse": show(&ch), "postgres": show(&pg) })),
    )
}

async fn render_metrics(State(s): State<OpsState>) -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/plain; version=0.0.4")],
        s.metrics.render(),
    )
}
