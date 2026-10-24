//! The notification outbox: every message is a row in `notifications`, sent
//! by a worker with retries (exponential backoff, 5 attempts). The same table
//! is the send history shown under Alerts → History.

use std::sync::Arc;
use std::time::Duration;

use logbox_core::Config;
use serde_json::Value;
use sqlx::PgPool;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::channels::{self, ChannelKind};
use crate::crypto::Keys;
use crate::smtp::{self, Email};

pub const MAX_ATTEMPTS: i32 = 5;
const BATCH: i64 = 20;
/// A claimed row is retried after this long if the worker dies mid-send.
const LEASE_SECS: i64 = 120;

#[derive(Debug, Clone)]
pub struct NewNotification {
    pub project_id: Option<Uuid>,
    pub source: &'static str,
    pub rule_id: Option<Uuid>,
    pub rule_name: String,
    pub channel_id: Option<Uuid>,
    pub channel_kind: ChannelKind,
    pub target: String,
    pub reason: &'static str,
    pub fingerprint: String,
    pub subject: String,
    /// Email: an [`Email`]. Others: `{text, html, body}`.
    pub payload: Value,
}

/// Enqueues notifications and wakes the worker.
#[derive(Clone)]
pub struct Outbox {
    pool: PgPool,
    wake: Arc<Notify>,
}

impl Outbox {
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool,
            wake: Arc::new(Notify::new()),
        }
    }

    pub async fn enqueue(&self, items: Vec<NewNotification>) -> Result<(), sqlx::Error> {
        if items.is_empty() {
            return Ok(());
        }
        let mut tx = self.pool.begin().await?;
        for n in items {
            sqlx::query(
                "INSERT INTO notifications (id, project_id, source, rule_id, rule_name, channel_id, channel_kind, \
                   target, reason, fingerprint, subject, payload) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
            )
            .bind(Uuid::now_v7())
            .bind(n.project_id)
            .bind(n.source)
            .bind(n.rule_id)
            .bind(&n.rule_name)
            .bind(n.channel_id)
            .bind(n.channel_kind.as_str())
            .bind(&n.target)
            .bind(n.reason)
            .bind(&n.fingerprint)
            .bind(&n.subject)
            .bind(&n.payload)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        self.wake.notify_one();
        Ok(())
    }

    /// Records a message that was already sent synchronously (test sends).
    pub async fn record(
        &self,
        n: NewNotification,
        result: &Result<String, String>,
    ) -> Result<(), sqlx::Error> {
        let (status, response, error) = match result {
            Ok(r) => ("sent", Some(r.as_str()), None),
            Err(e) => ("failed", None, Some(e.as_str())),
        };
        sqlx::query(
            "INSERT INTO notifications (id, project_id, source, rule_id, rule_name, channel_id, channel_kind, \
               target, reason, fingerprint, subject, payload, status, attempts, response, last_error, sent_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, 1, $14, $15, \
               CASE WHEN $13 = 'sent' THEN now() END)",
        )
        .bind(Uuid::now_v7())
        .bind(n.project_id)
        .bind(n.source)
        .bind(n.rule_id)
        .bind(&n.rule_name)
        .bind(n.channel_id)
        .bind(n.channel_kind.as_str())
        .bind(&n.target)
        .bind(n.reason)
        .bind(&n.fingerprint)
        .bind(&n.subject)
        .bind(&n.payload)
        .bind(status)
        .bind(response)
        .bind(error)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

#[derive(sqlx::FromRow)]
struct Due {
    id: Uuid,
    channel_id: Option<Uuid>,
    channel_kind: String,
    payload: Value,
    attempts: i32,
}

/// Delivers due notifications until `shutdown`.
pub async fn run_worker(
    config: Config,
    pool: PgPool,
    keys: Keys,
    outbox: Outbox,
    shutdown: CancellationToken,
) {
    let http = channels::http_client();
    loop {
        match deliver_due(&config, &pool, &keys, &http).await {
            Ok(n) if n as i64 == BATCH => continue,
            Ok(_) => {}
            Err(e) => tracing::warn!(error = %e, "notification outbox"),
        }
        tokio::select! {
            () = shutdown.cancelled() => break,
            () = outbox.wake.notified() => {}
            () = tokio::time::sleep(Duration::from_secs(5)) => {}
        }
    }
}

async fn deliver_due(
    config: &Config,
    pool: &PgPool,
    keys: &Keys,
    http: &reqwest::Client,
) -> Result<usize, sqlx::Error> {
    // Claim a batch with a lease so concurrent workers never double-send.
    let due: Vec<Due> = sqlx::query_as(
        "UPDATE notifications SET next_attempt_at = now() + make_interval(secs => $2) \
         WHERE id IN (SELECT id FROM notifications WHERE status = 'pending' AND next_attempt_at <= now() \
                      ORDER BY next_attempt_at LIMIT $1 FOR UPDATE SKIP LOCKED) \
         RETURNING id, channel_id, channel_kind, payload, attempts",
    )
    .bind(BATCH)
    .bind(LEASE_SECS as f64)
    .fetch_all(pool)
    .await?;
    if due.is_empty() {
        return Ok(0);
    }
    let smtp_settings = smtp::load(config, pool, keys).await;
    for d in &due {
        let result = match ChannelKind::parse(&d.channel_kind) {
            Some(ChannelKind::Email) => match (
                &smtp_settings,
                serde_json::from_value::<Email>(d.payload.clone()),
            ) {
                (Ok(s), Ok(email)) => smtp::send(s, &email).await,
                (Err(e), _) => Err(e.to_string()),
                (_, Err(e)) => Err(format!("bad stored message: {e}")),
            },
            Some(_) => match d.channel_id {
                Some(cid) => match channels::load(pool, &[cid]).await?.into_iter().next() {
                    Some(ch) => channels::send(http, keys, &ch, &d.payload).await,
                    None => Err("the channel was deleted".into()),
                },
                None => Err("no channel".into()),
            },
            None => Err(format!("unknown channel kind {}", d.channel_kind)),
        };
        let attempts = d.attempts + 1;
        match result {
            Ok(response) => {
                sqlx::query(
                    "UPDATE notifications SET status = 'sent', attempts = $2, response = $3, last_error = NULL, \
                       sent_at = now() WHERE id = $1",
                )
                .bind(d.id)
                .bind(attempts)
                .bind(response)
                .execute(pool)
                .await?;
                metrics::counter!("logbox_notifications_sent_total", "kind" => d.channel_kind.clone()).increment(1);
            }
            Err(error) => {
                let failed = attempts >= MAX_ATTEMPTS;
                // 30 s, 1 min, 2 min, 4 min.
                let backoff = 30.0 * f64::from(2_i32.pow((attempts - 1).clamp(0, 10) as u32));
                sqlx::query(
                    "UPDATE notifications SET status = CASE WHEN $3 THEN 'failed' ELSE 'pending' END, attempts = $2, \
                       last_error = $4, next_attempt_at = now() + make_interval(secs => $5) WHERE id = $1",
                )
                .bind(d.id)
                .bind(attempts)
                .bind(failed)
                .bind(&error)
                .bind(backoff)
                .execute(pool)
                .await?;
                tracing::warn!(id = %d.id, attempts, failed, %error, "notification not delivered");
                metrics::counter!("logbox_notifications_failed_total", "kind" => d.channel_kind.clone()).increment(1);
            }
        }
    }
    Ok(due.len())
}
