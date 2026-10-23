//! `logbox demo`: realistic demo traffic, and `--load` for load testing.

pub mod generator;

use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use clap::Args;
use logbox_core::{Config, admin, pg};
use reqwest::StatusCode;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use crate::generator::{
    Generator, INCIDENT_ROUTE, Phase, RELEASE_BAD, RELEASE_BEFORE, RELEASE_FIX,
};

#[derive(Debug, Clone, Args)]
pub struct DemoArgs {
    /// Base URL of a Logbox server (ingest endpoints).
    #[arg(long, default_value = "http://localhost:8080", env = "LOGBOX_DEMO_URL")]
    pub url: String,
    /// Ingest key. Without one, a "Demo" project and key are created in Postgres.
    #[arg(long, env = "LOGBOX_DEMO_KEY")]
    pub key: Option<String>,
    /// Environment name to report.
    #[arg(long, default_value = "production")]
    pub environment: String,
    /// Hours of history to backfill before streaming live traffic.
    #[arg(long, default_value_t = 2.0)]
    pub backfill_hours: f64,
    /// Requests per second in the backfill (scaled by a daily curve).
    #[arg(long, default_value_t = 8.0)]
    pub backfill_rate: f64,
    /// Live requests per second.
    #[arg(long, default_value_t = 12.0)]
    pub rate: f64,
    /// Stop after this many seconds of live traffic (default: run until Ctrl-C).
    #[arg(long)]
    pub duration: Option<u64>,
    /// Seconds into the live run when the bad release deploys (0 disables).
    #[arg(long, default_value_t = 120)]
    pub incident_after: u64,
    /// How long the live incident lasts before the fix deploys, in seconds.
    #[arg(long, default_value_t = 240)]
    pub incident_length: u64,
    /// Random seed.
    #[arg(long, default_value_t = 1)]
    pub seed: u64,

    /// Load-test mode: send pre-built batches as fast as possible.
    #[arg(long)]
    pub load: bool,
    /// Concurrent senders in load mode.
    #[arg(long, default_value_t = 16)]
    pub concurrency: usize,
    /// Items per batch in load mode.
    #[arg(long, default_value_t = 1000)]
    pub batch_size: usize,
    /// Load test duration in seconds.
    #[arg(long, default_value_t = 30)]
    pub seconds: u64,
    /// Offered load in events/s for load mode (0 = as fast as possible).
    #[arg(long, default_value_t = 0)]
    pub target_eps: u64,
}

fn now_ms() -> i64 {
    (time::OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000) as i64
}

pub async fn run(
    args: DemoArgs,
    config: &Config,
    shutdown: CancellationToken,
) -> anyhow::Result<()> {
    let key = match &args.key {
        Some(k) => k.clone(),
        None => bootstrap_key(config).await?,
    };
    let client = Client::new(&args.url, &key)?;
    if args.load {
        return load(&args, &client, shutdown).await;
    }
    demo(&args, &client, shutdown).await
}

/// Ensures a "Demo" project exists and mints a fresh ingest key for it.
async fn bootstrap_key(config: &Config) -> anyhow::Result<String> {
    let pool = pg::connect(&config.postgres).await?;
    pg::migrate(&pool).await?;
    let project = match admin::project_by_slug(&pool, "demo").await? {
        Some(p) => p,
        None => admin::create_project(&pool, "Demo").await?,
    };
    let stamp = time::OffsetDateTime::now_utc().unix_timestamp();
    let key = admin::create_ingest_key(&pool, project.id, &format!("demo {stamp}"), None).await?;
    tracing::info!(project = %project.slug, key = %key.prefix, "created demo ingest key");
    Ok(key.token)
}

struct Client {
    http: reqwest::Client,
    ingest_url: String,
    release_url: String,
    key: String,
}

impl Client {
    fn new(base: &str, key: &str) -> anyhow::Result<Self> {
        let base = base.trim_end_matches('/');
        Ok(Self {
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .pool_max_idle_per_host(64)
                .build()?,
            ingest_url: format!("{base}/v1/ingest"),
            release_url: format!("{base}/v1/releases"),
            key: key.to_string(),
        })
    }

    /// Sends one gzipped NDJSON batch, honoring 429 + Retry-After.
    async fn send(&self, body: Vec<u8>) -> anyhow::Result<(StatusCode, Duration)> {
        for attempt in 0..8 {
            let started = Instant::now();
            let resp = self
                .http
                .post(&self.ingest_url)
                .bearer_auth(&self.key)
                .header("content-type", "application/x-ndjson")
                .header("content-encoding", "gzip")
                .body(body.clone())
                .send()
                .await?;
            let elapsed = started.elapsed();
            let status = resp.status();
            if status == StatusCode::TOO_MANY_REQUESTS {
                let wait = resp
                    .headers()
                    .get("retry-after")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(1u64);
                tokio::time::sleep(Duration::from_millis(wait * 1000 + 50 * attempt)).await;
                continue;
            }
            if !status.is_success() && status != StatusCode::MULTI_STATUS {
                let text = resp.text().await.unwrap_or_default();
                bail!("ingest returned {status}: {text}");
            }
            return Ok((status, elapsed));
        }
        bail!("ingest kept answering 429")
    }

    async fn release(
        &self,
        service: &str,
        version: &str,
        environment: &str,
        ts_ms: i64,
    ) -> anyhow::Result<()> {
        let resp = self
            .http
            .post(&self.release_url)
            .bearer_auth(&self.key)
            .json(&json!({
                "service": service,
                "version": version,
                "environment": environment,
                "timestamp": ts_ms,
                "commit": format!("{:x}", ts_ms.wrapping_mul(2_654_435_761)),
            }))
            .send()
            .await?;
        if !resp.status().is_success() {
            bail!("release returned {}", resp.status());
        }
        Ok(())
    }
}

fn gzip_ndjson(items: &[Value]) -> anyhow::Result<Vec<u8>> {
    let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    for item in items {
        serde_json::to_writer(&mut enc, item)?;
        enc.write_all(b"\n")?;
    }
    Ok(enc.finish()?)
}

/// Daily traffic curve: quiet at night, busy mid-day (UTC).
fn diurnal(ts_ms: i64) -> f64 {
    let hour = (ts_ms / 3_600_000).rem_euclid(24) as f64;
    0.55 + 0.45
        * ((hour - 9.0) / 24.0 * 2.0 * std::f64::consts::PI)
            .cos()
            .max(-0.6)
}

async fn demo(args: &DemoArgs, client: &Client, shutdown: CancellationToken) -> anyhow::Result<()> {
    let mut g = Generator::new(args.seed, &args.environment);
    let env = args.environment.as_str();

    // Backfill: a past incident 50 to 35 minutes ago, so charts have a story.
    if args.backfill_hours > 0.0 {
        let end = now_ms() - 5_000;
        let start = end - (args.backfill_hours * 3_600_000.0) as i64;
        let (inc_start, inc_end) = (end - 50 * 60_000, end - 35 * 60_000);
        let horizon = inc_start > start;
        client.release("api", RELEASE_BEFORE, env, start).await?;
        if horizon {
            client.release("api", RELEASE_BAD, env, inc_start).await?;
            client.release("api", RELEASE_FIX, env, inc_end).await?;
        }
        tracing::info!(hours = args.backfill_hours, "backfilling history");
        let mut buf = Vec::new();
        let mut sent = 0usize;
        let mut t = start;
        while t < end && !shutdown.is_cancelled() {
            let phase = if horizon && t >= inc_start && t < inc_end {
                Phase::Incident
            } else if horizon && t >= inc_end {
                Phase::Recovered
            } else {
                Phase::Normal
            };
            buf.extend(g.request(t, phase).items);
            let rate = args.backfill_rate * diurnal(t);
            t += (1000.0 / rate.max(0.01)) as i64;
            if buf.len() >= 2000 {
                sent += buf.len();
                client.send(gzip_ndjson(&buf)?).await?;
                buf.clear();
            }
        }
        if !buf.is_empty() {
            sent += buf.len();
            client.send(gzip_ndjson(&buf)?).await?;
        }
        tracing::info!(items = sent, "backfill done");
    }

    // Live traffic with a scripted incident.
    let started = Instant::now();
    let mut phase = Phase::Recovered;
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    let mut sent = 0usize;
    tracing::info!(rate = args.rate, "streaming live traffic (Ctrl-C to stop)");
    loop {
        tokio::select! {
            () = shutdown.cancelled() => break,
            _ = tick.tick() => {}
        }
        let elapsed = started.elapsed().as_secs();
        if args.duration.is_some_and(|d| elapsed >= d) {
            break;
        }
        if args.incident_after > 0 {
            let new_phase = if elapsed >= args.incident_after
                && elapsed < args.incident_after + args.incident_length
            {
                Phase::Incident
            } else {
                Phase::Recovered
            };
            if new_phase != phase {
                let version = new_phase.api_release();
                tracing::info!(version, route = INCIDENT_ROUTE, "deploying {version}");
                client.release("api", version, env, now_ms()).await?;
                phase = new_phase;
            }
        }
        // Poisson-ish arrivals spread across the second.
        let n = args.rate.max(0.0).floor() as usize
            + usize::from(rand::random::<f64>() < args.rate.fract());
        let base = now_ms();
        let mut items = Vec::new();
        for i in 0..n {
            items.extend(
                g.request(base + (i as i64 * 1000 / n.max(1) as i64), phase)
                    .items,
            );
        }
        if !items.is_empty() {
            sent += items.len();
            if let Err(e) = client.send(gzip_ndjson(&items)?).await {
                tracing::warn!(error = %e, "send failed");
            }
        }
        if elapsed.is_multiple_of(30) && elapsed > 0 {
            tracing::info!(items = sent, ?phase, "live");
        }
    }
    Ok(())
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct LoadReport {
    pub seconds: f64,
    pub events: u64,
    pub events_per_sec: f64,
    pub batches: u64,
    pub throttled: u64,
    pub errors: u64,
    pub latency_ms_p50: f64,
    pub latency_ms_p95: f64,
    pub latency_ms_p99: f64,
    pub latency_ms_max: f64,
}

async fn load(args: &DemoArgs, client: &Client, shutdown: CancellationToken) -> anyhow::Result<()> {
    let batch_size = args.batch_size.clamp(1, 10_000);
    // Pre-build payloads so the generator doesn't limit the server.
    let mut g = Generator::new(args.seed, &args.environment);
    let mut payloads = Vec::new();
    let now = now_ms();
    for b in 0..32 {
        let mut items = Vec::with_capacity(batch_size);
        while items.len() < batch_size {
            let generated = g.request(now - 60_000 + b, Phase::Normal);
            // Requests only: the target is defined in requests/s.
            items.extend(
                generated
                    .items
                    .into_iter()
                    .filter(|i| i["type"] == "request"),
            );
        }
        items.truncate(batch_size);
        payloads.push(gzip_ndjson(&items)?);
    }
    let payloads = Arc::new(payloads);
    let avg_kb =
        payloads.iter().map(Vec::len).sum::<usize>() as f64 / payloads.len() as f64 / 1024.0;
    tracing::info!(
        concurrency = args.concurrency,
        batch_size,
        seconds = args.seconds,
        avg_kb,
        "load test"
    );

    let events = Arc::new(AtomicU64::new(0));
    let throttled = Arc::new(AtomicU64::new(0));
    let errors = Arc::new(AtomicU64::new(0));
    let deadline = Instant::now() + Duration::from_secs(args.seconds);
    let started = Instant::now();
    let mut tasks = Vec::new();
    for w in 0..args.concurrency.max(1) {
        let (payloads, events, throttled, errors) = (
            payloads.clone(),
            events.clone(),
            throttled.clone(),
            errors.clone(),
        );
        let http = client.http.clone();
        let (url, key) = (client.ingest_url.clone(), client.key.clone());
        let stop = shutdown.clone();
        // Open-loop pacing: each worker sends on a fixed schedule so latency
        // is measured at the offered rate, not at saturation.
        let mut pace = (args.target_eps > 0).then(|| {
            let per_worker = args.target_eps as f64 / args.concurrency.max(1) as f64;
            let period = Duration::from_secs_f64(batch_size as f64 / per_worker.max(1.0));
            let start = tokio::time::Instant::now()
                + period.mul_f64(w as f64 / args.concurrency.max(1) as f64);
            let mut iv = tokio::time::interval_at(start, period);
            iv.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            iv
        });
        tasks.push(tokio::spawn(async move {
            let mut latencies = Vec::new();
            let mut i = w;
            while Instant::now() < deadline && !stop.is_cancelled() {
                if let Some(iv) = pace.as_mut() {
                    iv.tick().await;
                }
                let body = payloads[i % payloads.len()].clone();
                i += 1;
                let t = Instant::now();
                let res = http
                    .post(&url)
                    .bearer_auth(&key)
                    .header("content-type", "application/x-ndjson")
                    .header("content-encoding", "gzip")
                    .body(body)
                    .send()
                    .await;
                let elapsed = t.elapsed();
                match res {
                    Ok(r) if r.status().is_success() => {
                        latencies.push(elapsed.as_secs_f64() * 1000.0);
                        events.fetch_add(batch_size as u64, Ordering::Relaxed);
                    }
                    Ok(r) if r.status() == StatusCode::TOO_MANY_REQUESTS => {
                        throttled.fetch_add(1, Ordering::Relaxed);
                        tokio::time::sleep(Duration::from_millis(200)).await;
                    }
                    Ok(_) | Err(_) => {
                        errors.fetch_add(1, Ordering::Relaxed);
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                }
            }
            latencies
        }));
    }
    let mut lat = Vec::new();
    for t in tasks {
        lat.extend(t.await.context("load worker")?);
    }
    let secs = started.elapsed().as_secs_f64();
    lat.sort_by(f64::total_cmp);
    let pct = |p: f64| -> f64 {
        if lat.is_empty() {
            0.0
        } else {
            lat[((lat.len() as f64 - 1.0) * p).round() as usize]
        }
    };
    let ev = events.load(Ordering::Relaxed);
    let report = LoadReport {
        seconds: (secs * 10.0).round() / 10.0,
        events: ev,
        events_per_sec: (ev as f64 / secs).round(),
        batches: lat.len() as u64,
        throttled: throttled.load(Ordering::Relaxed),
        errors: errors.load(Ordering::Relaxed),
        latency_ms_p50: (pct(0.5) * 10.0).round() / 10.0,
        latency_ms_p95: (pct(0.95) * 10.0).round() / 10.0,
        latency_ms_p99: (pct(0.99) * 10.0).round() / 10.0,
        latency_ms_max: (lat.last().copied().unwrap_or(0.0) * 10.0).round() / 10.0,
    };
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
