#![allow(clippy::unwrap_used, clippy::expect_used)] // test code
//! End-to-end ingest against real ClickHouse and Postgres (skipped when
//! `TRAILSCOPE_TEST_*` is unset).

use std::io::Write;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;
use trailscope_core::ch::{self, ClickHouse};
use trailscope_core::config::Config;
use trailscope_core::live::LiveBus;
use trailscope_core::{admin, pg, testing};

struct Harness {
    router: axum::Router,
    key: String,
    ch: ClickHouse,
    bus: LiveBus,
    stop: CancellationToken,
    writer: tokio::task::JoinHandle<()>,
}

async fn harness(tweak: impl FnOnce(&mut Config)) -> Option<Harness> {
    let mut env = testing::env().await?;
    env.config.ingest.flush_interval_ms = 50;
    tweak(&mut env.config);
    let cfg = env.config;
    ch::migrate(&cfg).await.unwrap();
    let pool = pg::connect(&cfg.postgres).await.unwrap();
    pg::migrate(&pool).await.unwrap();
    let org = admin::default_org(&pool).await.unwrap();
    let project = admin::create_project(&pool, org.id, "Test").await.unwrap();
    let key = admin::create_ingest_key(&pool, project.id, "test", None)
        .await
        .unwrap()
        .token;
    let ch = ClickHouse::new(&cfg);
    let bus = LiveBus::local(&cfg.live);
    let stop = CancellationToken::new();
    let started = trailscope_ingest::start(&cfg, &ch, pool, bus.clone(), stop.clone());
    Some(Harness {
        router: started.router,
        key,
        ch,
        bus,
        stop,
        writer: started.writer,
    })
}

async fn post(
    h: &Harness,
    body: Vec<u8>,
    gzip: bool,
    key: Option<&str>,
) -> (StatusCode, Value, Option<String>) {
    let mut req = Request::post("/v1/ingest").header(header::CONTENT_TYPE, "application/x-ndjson");
    if let Some(k) = key.or(Some(&h.key)) {
        req = req.header(header::AUTHORIZATION, format!("Bearer {k}"));
    }
    let body = if gzip {
        req = req.header(header::CONTENT_ENCODING, "gzip");
        let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        e.write_all(&body).unwrap();
        e.finish().unwrap()
    } else {
        body
    };
    let resp = h
        .router
        .clone()
        .oneshot(req.body(Body::from(body)).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let retry = resp
        .headers()
        .get(header::RETRY_AFTER)
        .map(|v| v.to_str().unwrap().to_string());
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        retry,
    )
}

fn ndjson(items: &[Value]) -> Vec<u8> {
    items
        .iter()
        .map(|v| v.to_string())
        .collect::<Vec<_>>()
        .join("\n")
        .into_bytes()
}

async fn wait_for_rows(ch: &ClickHouse, table: &str, n: u64) -> u64 {
    let mut got = 0;
    for _ in 0..100 {
        got = ch
            .writer
            .query(&format!("SELECT count() FROM {table}"))
            .fetch_one::<u64>()
            .await
            .unwrap();
        if got >= n {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    got
}

#[tokio::test]
async fn ingests_validates_redacts_and_publishes() {
    let Some(h) = harness(|_| {}).await else {
        eprintln!("skipping: TRAILSCOPE_TEST_* not set");
        return;
    };
    let mut live = h.bus.subscribe_requests();
    let mut errors = h.bus.subscribe_errors();

    let items = vec![
        json!({"type":"request","service":"api","method":"POST","path":"/auth/login","route":"/auth/login",
               "status":200,"durationMs":42.5,"userId":"u_1","ip":"203.0.113.9",
               "reqHeaders":{"Authorization":"Bearer x","Content-Type":"application/json"},
               "reqBody":{"email":"a@example.com","password":"hunter2"},
               "attributes":{"tenant":"t1"},"correlation":{"account_id":"acc_1"}}),
        json!({"type":"request","service":"api","method":"GET","path":"/users/123",
               "status":503,"durationMs":3000,"errorType":"TimeoutError","errorMessage":"upstream timed out",
               "errorStack":"TimeoutError: upstream timed out\n    at call (/app/src/a.js:1:2)"}),
        json!({"type":"request","service":"api","method":"GET","path":"/x","status":42,"durationMs":1}),
        json!({"type":"span","traceId":"t1","spanId":"s1","service":"api","name":"SELECT 1","kind":"db","durationMs":2}),
        json!({"type":"event","service":"worker","name":"email.send","kind":"job","status":"error","attempt":2,
               "errorType":"SMTPError","errorMessage":"connection refused"}),
        json!({"type":"nope"}),
    ];
    let (status, body, _) = post(&h, ndjson(&items), true, None).await;
    assert_eq!(status, StatusCode::MULTI_STATUS, "{body}");
    assert_eq!(body["accepted"], 4);
    assert_eq!(body["rejected"], 2);
    assert_eq!(body["errors"][0]["index"], 2);
    assert_eq!(
        body["errors"][0]["message"],
        "status: must be between 100 and 599"
    );
    assert_eq!(body["errors"][1]["index"], 5);

    assert_eq!(wait_for_rows(&h.ch, "requests", 2).await, 2);
    assert_eq!(wait_for_rows(&h.ch, "spans", 1).await, 1);
    assert_eq!(wait_for_rows(&h.ch, "events", 1).await, 1);

    #[derive(Debug, serde::Deserialize, clickhouse::Row)]
    struct Check {
        route: String,
        ip: String,
        req_body: String,
        auth: String,
        fp: u64,
    }
    let rows: Vec<Check> = h
        .ch
        .writer
        .query(
            "SELECT route, ip, req_body, req_headers['authorization'] AS auth, error_fingerprint AS fp \
             FROM requests ORDER BY status",
        )
        .fetch_all()
        .await
        .unwrap();
    assert_eq!(rows[0].route, "/auth/login");
    assert_eq!(rows[0].ip, "203.0.113.0");
    assert!(
        rows[0].req_body.contains(r#""password":"[REDACTED]""#),
        "{}",
        rows[0].req_body
    );
    assert_eq!(rows[0].auth, "[REDACTED]");
    assert_eq!(rows[0].fp, 0);
    assert_eq!(rows[1].route, "/users/:id");
    assert_ne!(rows[1].fp, 0);

    // Rollups and error groups fill through the materialized views.
    assert_eq!(wait_for_rows(&h.ch, "route_minute", 2).await, 2);
    assert!(wait_for_rows(&h.ch, "error_groups", 2).await >= 2);

    // Live summaries and the 5xx error signal were published after the flush.
    let a = live.recv().await.unwrap();
    let b = live.recv().await.unwrap();
    assert_eq!(a.status.max(b.status), 503);
    let e = errors.recv().await.unwrap();
    assert_eq!((e.status, e.route.as_str()), (503, "/users/:id"));

    h.stop.cancel();
    h.writer.await.unwrap();
}

#[tokio::test]
async fn rejects_bad_auth_and_bodies() {
    let Some(h) = harness(|c| c.ingest.max_items = 3).await else {
        return;
    };
    let one = ndjson(&[json!({"type":"event","service":"s","name":"n"})]);
    let (s, _, _) = post(&h, one.clone(), false, Some("trs_ingest_nope")).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    let (s, _, _) = post(&h, one.clone(), false, Some("not-a-key")).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    let (s, body, _) = post(&h, b"[{".to_vec(), false, None).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{body}");
    let four = ndjson(&vec![json!({"type":"event","service":"s","name":"n"}); 4]);
    let (s, body, _) = post(&h, four, false, None).await;
    assert_eq!(s, StatusCode::PAYLOAD_TOO_LARGE, "{body}");
    let (s, body, _) = post(&h, ndjson(&[json!({"type":"event"})]), false, None).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert_eq!(body["errors"][0]["message"], "service: required");
    h.stop.cancel();
    h.writer.await.unwrap();
}

#[tokio::test]
async fn answers_429_when_the_queue_is_full() {
    // A long flush interval keeps rows queued, so the second batch can't fit.
    let Some(h) = harness(|c| {
        c.ingest.queue_capacity = 5;
        c.ingest.retry_after_secs = 7;
        c.ingest.flush_interval_ms = 60_000;
    })
    .await
    else {
        return;
    };
    let five = ndjson(&vec![json!({"type":"event","service":"s","name":"n"}); 5]);
    let (s, _, _) = post(&h, five.clone(), false, None).await;
    assert_eq!(s, StatusCode::OK);
    let (s, body, retry) = post(&h, five, false, None).await;
    assert_eq!(s, StatusCode::TOO_MANY_REQUESTS, "{body}");
    assert_eq!(retry.as_deref(), Some("7"));
    // Shutdown still drains what was admitted.
    h.stop.cancel();
    h.writer.await.unwrap();
    assert_eq!(wait_for_rows(&h.ch, "events", 5).await, 5);
}
