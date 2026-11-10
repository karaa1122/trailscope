//! Usage metering and plan limits at ingest (decision 071).
//!
//! Accepted events are counted per organization in memory and added to
//! Postgres `usage_daily` every few seconds, so the hot path never waits on the
//! database. With `cloud.enabled`, a second loop reads which organizations are
//! suspended or past a hard monthly limit, and ingest refuses their batches
//! from memory. An event is a request, a job run or a log line; spans are free.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use sqlx::PgPool;
use time::{Date, OffsetDateTime, Time};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const FLUSH_EVERY: Duration = Duration::from_secs(10);
const REFRESH_EVERY: Duration = Duration::from_secs(5);

/// Why an organization's batches are refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Block {
    Suspended,
    /// Its plan stops at the monthly limit, and the limit is reached.
    OverLimit,
}

#[derive(Debug, Default, Clone, Copy)]
struct Counts {
    events: u64,
    bytes: u64,
}

pub struct Usage {
    pool: PgPool,
    enforce: bool,
    pending: Mutex<HashMap<Uuid, Counts>>,
    blocked: RwLock<HashMap<Uuid, Block>>,
}

impl Usage {
    pub fn new(pool: PgPool, enforce: bool) -> Arc<Self> {
        Arc::new(Self {
            pool,
            enforce,
            pending: Mutex::new(HashMap::new()),
            blocked: RwLock::new(HashMap::new()),
        })
    }

    /// Counts an accepted batch.
    pub fn record(&self, org: Uuid, events: u64, bytes: u64) {
        if events == 0 && bytes == 0 {
            return;
        }
        if let Ok(mut map) = self.pending.lock() {
            let c = map.entry(org).or_default();
            c.events += events;
            c.bytes += bytes;
        }
    }

    /// Whether the organization's batches must be refused. Always `None` when
    /// limits aren't enforced.
    pub fn check(&self, org: Uuid) -> Option<Block> {
        if !self.enforce {
            return None;
        }
        self.blocked.read().ok()?.get(&org).copied()
    }

    /// Adds the pending counts to `usage_daily`. On failure they are kept for
    /// the next attempt.
    pub async fn flush(&self) -> Result<(), sqlx::Error> {
        let taken = match self.pending.lock() {
            Ok(mut map) => std::mem::take(&mut *map),
            Err(_) => return Ok(()),
        };
        if taken.is_empty() {
            return Ok(());
        }
        let mut orgs = Vec::with_capacity(taken.len());
        let mut events = Vec::with_capacity(taken.len());
        let mut bytes = Vec::with_capacity(taken.len());
        for (org, c) in &taken {
            orgs.push(*org);
            events.push(i64::try_from(c.events).unwrap_or(i64::MAX));
            bytes.push(i64::try_from(c.bytes).unwrap_or(i64::MAX));
        }
        let res = sqlx::query(
            "INSERT INTO usage_daily (org_id, day, events, bytes) \
             SELECT t.org, (now() AT TIME ZONE 'UTC')::date, t.events, t.bytes \
             FROM UNNEST($1::uuid[], $2::bigint[], $3::bigint[]) AS t (org, events, bytes) \
             WHERE EXISTS (SELECT 1 FROM organizations o WHERE o.id = t.org) \
             ON CONFLICT (org_id, day) DO UPDATE \
             SET events = usage_daily.events + EXCLUDED.events, \
                 bytes = usage_daily.bytes + EXCLUDED.bytes",
        )
        .bind(&orgs)
        .bind(&events)
        .bind(&bytes)
        .execute(&self.pool)
        .await;
        if let Err(e) = res {
            if let Ok(mut map) = self.pending.lock() {
                for (org, c) in taken {
                    let p = map.entry(org).or_default();
                    p.events += c.events;
                    p.bytes += c.bytes;
                }
            }
            return Err(e);
        }
        Ok(())
    }

    /// Reloads which organizations are suspended or past a hard limit.
    pub async fn refresh(&self) -> Result<(), sqlx::Error> {
        let rows: Vec<(Uuid, bool, bool)> = sqlx::query_as(
            "SELECT o.id, o.suspended_at IS NOT NULL, \
                    p.hard_limit AND p.events_per_month IS NOT NULL AND \
                    COALESCE((SELECT SUM(u.events) FROM usage_daily u \
                              WHERE u.org_id = o.id \
                                AND u.day >= date_trunc('month', now() AT TIME ZONE 'UTC')::date), 0) \
                      >= p.events_per_month \
             FROM organizations o JOIN plans p ON p.id = o.plan_id \
             WHERE o.suspended_at IS NOT NULL OR (p.hard_limit AND p.events_per_month IS NOT NULL)",
        )
        .fetch_all(&self.pool)
        .await?;
        let next: HashMap<Uuid, Block> = rows
            .into_iter()
            .filter_map(|(org, suspended, over)| {
                if suspended {
                    Some((org, Block::Suspended))
                } else if over {
                    Some((org, Block::OverLimit))
                } else {
                    None
                }
            })
            .collect();
        if let Ok(mut map) = self.blocked.write() {
            *map = next;
        }
        Ok(())
    }
}

/// Flushes usage every few seconds (and once more on `stop`), and refreshes
/// the blocked organizations when limits are enforced.
pub async fn run(usage: Arc<Usage>, stop: CancellationToken) {
    let mut flush = tokio::time::interval(FLUSH_EVERY);
    let mut refresh = tokio::time::interval(REFRESH_EVERY);
    flush.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    refresh.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            () = stop.cancelled() => {
                if let Err(e) = usage.flush().await {
                    tracing::warn!(error = %e, "final usage flush failed");
                }
                return;
            }
            _ = flush.tick() => {
                if let Err(e) = usage.flush().await {
                    tracing::warn!(error = %e, "usage flush failed; retrying");
                }
            }
            _ = refresh.tick(), if usage.enforce => {
                if let Err(e) = usage.refresh().await {
                    tracing::warn!(error = %e, "refreshing plan limits failed");
                }
            }
        }
    }
}

/// Seconds until the next calendar month starts (UTC), for `Retry-After`.
pub fn secs_to_next_month(now: OffsetDateTime) -> u64 {
    let (year, month) = match now.month() {
        time::Month::December => (now.year() + 1, time::Month::January),
        m => (now.year(), m.next()),
    };
    let Ok(first) = Date::from_calendar_date(year, month, 1) else {
        return 3600;
    };
    let next = first.with_time(Time::MIDNIGHT).assume_utc();
    u64::try_from((next - now).whole_seconds())
        .unwrap_or(3600)
        .max(60)
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    #[test]
    fn next_month() {
        assert_eq!(secs_to_next_month(datetime!(2026-11-30 23:00 UTC)), 3600);
        assert_eq!(secs_to_next_month(datetime!(2026-12-31 23:59 UTC)), 60);
    }
}
