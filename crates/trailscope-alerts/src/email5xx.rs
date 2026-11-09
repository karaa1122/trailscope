//! The built-in "Email on 5xx" rule. Runs in real time on the 5xx signals
//! ingest publishes after each flush (in-process in `all` mode, Redis in
//! split mode), so an email goes out within seconds of the error.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use serde_json::to_value;
use sqlx::PgPool;
use tokio::sync::broadcast::error::RecvError;
use tokio_util::sync::CancellationToken;
use trailscope_core::Config;
use trailscope_core::ch::ClickHouse;
use trailscope_core::live::LiveBus;
use trailscope_core::rows::ErrorSignal;
use trailscope_query::{Ctx, Prepared, Table};
use uuid::Uuid;

use crate::channels::{self, ChannelKind};
use crate::crypto::Keys;
use crate::mute::{self, MuteToken};
use crate::outbox::{NewNotification, Outbox};
use crate::render::{self, ErrorEmail, Reason};
use crate::smtp::{self, Email};
use crate::throttle::{Admit, Key, Throttle};

/// The filter language as used by this rule: live columns of the requests table.
pub fn filter_ctx() -> Ctx {
    Ctx {
        table: Table::Requests,
        allow_sensitive: false,
        live: true,
    }
}

/// Parses an "Email on 5xx" filter; empty means every 5xx.
pub fn parse_filter(text: &str) -> Result<Option<Prepared>, String> {
    let expr = trailscope_query::parse(text, &filter_ctx()).map_err(|e| e.message)?;
    expr.map(|e| Prepared::new(&e)).transpose()
}

struct ActiveRule {
    project_name: String,
    filter: Option<Prepared>,
    recipients: Vec<String>,
}

#[derive(sqlx::FromRow)]
struct RuleRow {
    project_id: Uuid,
    project_name: String,
    filter: String,
    member_ids: Vec<Uuid>,
    emails: Vec<String>,
}

pub struct Engine {
    config: Config,
    ch: ClickHouse,
    pool: PgPool,
    keys: Keys,
    outbox: Outbox,
    throttle: Throttle,
    rules: HashMap<Uuid, ActiveRule>,
    /// Latest signal per group, used for digests sent from the timer.
    latest: HashMap<Key, Arc<ErrorSignal>>,
}

impl Engine {
    pub fn new(config: Config, ch: ClickHouse, pool: PgPool, keys: Keys, outbox: Outbox) -> Self {
        let a = &config.alerts;
        let throttle = Throttle::new(
            a.email_throttle_minutes,
            a.regression_hours,
            a.email_hourly_cap,
        );
        Self {
            config,
            ch,
            pool,
            keys,
            outbox,
            throttle,
            rules: HashMap::new(),
            latest: HashMap::new(),
        }
    }

    pub async fn run(mut self, bus: LiveBus, shutdown: CancellationToken) {
        let mut rx = bus.subscribe_errors();
        if let Err(e) = self.load_groups().await {
            tracing::warn!(error = %e, "loading 5xx email state");
        }
        self.refresh().await;
        let mut tick = tokio::time::interval(Duration::from_secs(15));
        let mut ticks: u64 = 0;
        loop {
            tokio::select! {
                () = shutdown.cancelled() => break,
                msg = rx.recv() => match msg {
                    Ok(sig) => self.handle(sig).await,
                    Err(RecvError::Lagged(n)) => tracing::warn!(skipped = n, "5xx email engine fell behind"),
                    Err(RecvError::Closed) => break,
                },
                _ = tick.tick() => {
                    ticks += 1;
                    if ticks.is_multiple_of(2) {
                        self.refresh().await;
                    }
                    self.send_due_digests().await;
                    if let Err(e) = self.persist().await {
                        tracing::warn!(error = %e, "saving 5xx email state");
                    }
                    // Same horizon as `load_groups`; digests only need the latest
                    // signal while occurrences are pending.
                    self.throttle.prune(now_ms(), 30 * 86_400_000);
                    let groups = &self.throttle.groups;
                    self.latest.retain(|k, _| groups.get(k).is_some_and(|g| g.pending > 0));
                }
            }
        }
        if let Err(e) = self.persist().await {
            tracing::warn!(error = %e, "saving 5xx email state");
        }
    }

    /// Rules with SMTP configured and at least one recipient, plus mutes.
    async fn refresh(&mut self) {
        match self.load_rules().await {
            Ok(rules) => self.rules = rules,
            Err(e) => tracing::warn!(error = %e, "loading email rules"),
        }
        match self.load_mutes().await {
            Ok(m) => self.throttle.set_mutes(m),
            Err(e) => tracing::warn!(error = %e, "loading mutes"),
        }
    }

    async fn load_rules(&self) -> anyhow_lite::Result<HashMap<Uuid, ActiveRule>> {
        let settings = smtp::load(&self.config, &self.pool, &self.keys).await?;
        if !settings.configured() {
            return Ok(HashMap::new());
        }
        let rows: Vec<RuleRow> = sqlx::query_as(
            "SELECT r.project_id, p.name AS project_name, r.filter, r.member_ids, r.emails \
             FROM email_rules r JOIN projects p ON p.id = r.project_id WHERE r.enabled",
        )
        .fetch_all(&self.pool)
        .await?;
        let mut out = HashMap::new();
        for r in rows {
            let recipients = channels::recipients(&self.pool, &r.member_ids, &r.emails).await?;
            if recipients.is_empty() {
                continue;
            }
            let filter = match parse_filter(&r.filter) {
                Ok(f) => f,
                Err(e) => {
                    tracing::warn!(project = %r.project_id, error = %e, "invalid Email on 5xx filter; rule skipped");
                    continue;
                }
            };
            out.insert(
                r.project_id,
                ActiveRule {
                    project_name: r.project_name,
                    filter,
                    recipients,
                },
            );
        }
        Ok(out)
    }

    async fn load_mutes(&self) -> Result<HashMap<Key, i64>, sqlx::Error> {
        let rows: Vec<(Uuid, String, Option<i64>)> = sqlx::query_as(
            "SELECT project_id, fingerprint, \
               CASE WHEN muted_until = 'infinity' THEN NULL ELSE (extract(epoch FROM muted_until) * 1000)::bigint END \
             FROM email_groups WHERE muted_until > now()",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(p, f, until)| ((p, f), until.unwrap_or(i64::MAX)))
            .collect())
    }

    async fn load_groups(&mut self) -> Result<(), sqlx::Error> {
        let rows: Vec<(Uuid, String, i64, i64, Option<i64>, i64)> = sqlx::query_as(
            "SELECT project_id, fingerprint, (extract(epoch FROM first_seen_at) * 1000)::bigint, \
               (extract(epoch FROM last_seen_at) * 1000)::bigint, (extract(epoch FROM last_sent_at) * 1000)::bigint, total \
             FROM email_groups WHERE last_seen_at > now() - interval '30 days'",
        )
        .fetch_all(&self.pool)
        .await?;
        for (p, f, first, last, sent, total) in rows {
            self.throttle.groups.insert(
                (p, f),
                crate::throttle::Group {
                    first_seen: first,
                    last_seen: last,
                    last_sent: sent,
                    total: total.max(0) as u64,
                    ..Default::default()
                },
            );
        }
        Ok(())
    }

    async fn persist(&mut self) -> Result<(), sqlx::Error> {
        let mut projects = Vec::new();
        let mut fps = Vec::new();
        let mut firsts = Vec::new();
        let mut lasts = Vec::new();
        let mut sents: Vec<Option<i64>> = Vec::new();
        let mut totals = Vec::new();
        for ((p, f), g) in &mut self.throttle.groups {
            if !g.dirty {
                continue;
            }
            g.dirty = false;
            projects.push(*p);
            fps.push(f.clone());
            firsts.push(g.first_seen);
            lasts.push(g.last_seen);
            sents.push(g.last_sent);
            totals.push(g.total as i64);
        }
        if projects.is_empty() {
            return Ok(());
        }
        sqlx::query(
            "INSERT INTO email_groups (project_id, fingerprint, first_seen_at, last_seen_at, last_sent_at, total) \
             SELECT p, f, to_timestamp(a / 1000.0), to_timestamp(b / 1000.0), to_timestamp(c / 1000.0), t \
             FROM unnest($1::uuid[], $2::text[], $3::bigint[], $4::bigint[], $5::bigint[], $6::bigint[]) AS x(p, f, a, b, c, t) \
             ON CONFLICT (project_id, fingerprint) DO UPDATE SET last_seen_at = EXCLUDED.last_seen_at, \
               last_sent_at = COALESCE(EXCLUDED.last_sent_at, email_groups.last_sent_at), total = EXCLUDED.total",
        )
        .bind(&projects)
        .bind(&fps)
        .bind(&firsts)
        .bind(&lasts)
        .bind(&sents)
        .bind(&totals)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn handle(&mut self, sig: Arc<ErrorSignal>) {
        let Ok(project) = sig.project_id.parse::<Uuid>() else {
            return;
        };
        let Some(rule) = self.rules.get(&project) else {
            return;
        };
        if sig.status < 500 {
            return;
        }
        let now = now_ms();
        if let Some(f) = &rule.filter
            && !f.matches(sig.as_ref(), now)
        {
            return;
        }
        // Errors without a fingerprint still group by route and status.
        let fp = if sig.error_fingerprint.is_empty() || sig.error_fingerprint == "0" {
            format!("{} {} {}", sig.status, sig.method, sig.route)
        } else {
            sig.error_fingerprint.clone()
        };
        let key: Key = (project, fp);
        self.latest.insert(key.clone(), sig.clone());
        if let Some(reason) = self.throttle.observe(&key, &sig.user_id, now) {
            self.dispatch(&key, reason, &sig, now).await;
        }
    }

    async fn send_due_digests(&mut self) {
        let now = now_ms();
        for (key, reason) in self.throttle.due_digests(now) {
            if !self.rules.contains_key(&key.0) {
                continue;
            }
            if let Some(sig) = self.latest.get(&key).cloned() {
                self.dispatch(&key, reason, &sig, now).await;
            }
        }
    }

    async fn dispatch(&mut self, key: &Key, reason: Reason, sig: &ErrorSignal, now: i64) {
        let project = key.0;
        let Some(rule) = self.rules.get(&project) else {
            return;
        };
        let admit = self.throttle.admit(project, now);
        let rendered = match admit {
            Admit::Suppress => {
                self.throttle.mark_suppressed(key, &reason);
                return;
            }
            Admit::SendPausedNotice => {
                self.throttle.mark_suppressed(key, &reason);
                let resumes = (now.div_euclid(3_600_000) + 1) * 3_600_000;
                render::paused_email(
                    &rule.project_name,
                    self.config.alerts.email_hourly_cap,
                    resumes,
                    &self.config.server.public_url,
                )
            }
            Admit::Send => {
                self.throttle.mark_sent(key, now);
                let totals = self.totals(&sig.project_id, &key.1).await;
                let expiry = now / 1000 + i64::from(self.config.alerts.mute_link_days) * 86_400;
                let mute_links = [("1 hour", 1), ("24 hours", 24), ("forever", 0)]
                    .into_iter()
                    .map(|(label, h)| {
                        let t = MuteToken {
                            p: project,
                            f: key.1.clone(),
                            h,
                            e: expiry,
                        };
                        (
                            label,
                            mute::link(&self.config.server.public_url, &self.keys, &t),
                        )
                    })
                    .collect();
                render::error_email(&ErrorEmail {
                    sig,
                    project_name: &rule.project_name,
                    reason: &reason,
                    totals,
                    public_url: &self.config.server.public_url,
                    mute_links,
                })
            }
        };
        let reason_key = if admit == Admit::SendPausedNotice {
            "paused"
        } else {
            reason.key()
        };
        let items: Vec<NewNotification> = rule
            .recipients
            .iter()
            .map(|to| NewNotification {
                project_id: Some(project),
                source: "email_5xx",
                rule_id: None,
                rule_name: "Email on 5xx".into(),
                channel_id: None,
                channel_kind: ChannelKind::Email,
                target: to.clone(),
                reason: reason_key,
                fingerprint: key.1.clone(),
                subject: rendered.subject.clone(),
                payload: to_value(Email {
                    to: to.clone(),
                    subject: rendered.subject.clone(),
                    text: rendered.text.clone(),
                    html: rendered.html.clone(),
                })
                .unwrap_or_default(),
            })
            .collect();
        if let Err(e) = self.outbox.enqueue(items).await {
            tracing::error!(error = %e, "queueing 5xx email");
        }
    }

    /// Occurrences and users affected so far, from the error group rollup.
    async fn totals(&self, project: &str, fingerprint: &str) -> (u64, u64) {
        #[derive(clickhouse::Row, serde::Deserialize)]
        struct T {
            c: u64,
            u: u64,
        }
        let Ok(fp) = fingerprint.parse::<u64>() else {
            return (0, 0);
        };
        let q = self
            .ch
            .reader
            .query(
                "SELECT toUInt64(sum(count)) AS c, toUInt64(uniqMerge(users)) AS u FROM error_groups \
                 WHERE project_id = {pid:String} AND error_fingerprint = {fp:UInt64}",
            )
            .param("pid", project)
            .param("fp", fp);
        match q.fetch_one::<T>().await {
            Ok(t) => (t.c, t.u),
            Err(e) => {
                tracing::warn!(error = %e, "error group totals");
                (0, 0)
            }
        }
    }
}

pub fn now_ms() -> i64 {
    (time::OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000) as i64
}

/// A tiny error type so engine helpers can use `?` across sqlx and crypto errors.
mod anyhow_lite {
    pub type Result<T> = std::result::Result<T, Error>;

    #[derive(Debug, thiserror::Error)]
    pub enum Error {
        #[error(transparent)]
        Db(#[from] sqlx::Error),
        #[error(transparent)]
        Crypto(#[from] crate::crypto::CryptoError),
    }
}
