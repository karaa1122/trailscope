//! Noise control for 5xx emails, grouped by error fingerprint. Pure logic
//! (no I/O), driven by the engine with explicit timestamps.
//!
//! - The first occurrence of a group is sent immediately.
//! - After that, at most one email per group per `throttle`: a digest, and
//!   only if the error kept happening.
//! - A group that was quiet for `regression` and comes back is sent at once.
//! - At most `cap` emails per project per clock hour; the first email over the
//!   cap is replaced by a single "paused" notice.

use std::collections::{HashMap, HashSet};

use uuid::Uuid;

use crate::render::Reason;

const MAX_TRACKED_USERS: usize = 10_000;

#[derive(Debug, Clone, Default)]
pub struct Group {
    pub first_seen: i64,
    pub last_seen: i64,
    pub last_sent: Option<i64>,
    pub total: u64,
    /// Occurrences since the last email.
    pub pending: u64,
    pub pending_users: HashSet<String>,
    /// `i64::MAX` mutes forever.
    pub muted_until: Option<i64>,
    /// Needs writing to Postgres.
    pub dirty: bool,
}

#[derive(Debug, Clone, Copy, Default)]
struct Cap {
    hour: i64,
    sent: u32,
    paused_notice_sent: bool,
    suppressed: u64,
}

/// What to do with an email that passed the per-group throttle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Admit {
    Send,
    /// Over the cap: send the one "paused" notice instead.
    SendPausedNotice,
    Suppress,
}

pub type Key = (Uuid, String);

#[derive(Debug)]
pub struct Throttle {
    pub groups: HashMap<Key, Group>,
    /// Mutes from Postgres, also for groups not in memory yet.
    mutes: HashMap<Key, i64>,
    caps: HashMap<Uuid, Cap>,
    throttle_ms: i64,
    regression_ms: i64,
    cap: u32,
}

impl Throttle {
    pub fn new(throttle_minutes: u32, regression_hours: u32, cap: u32) -> Self {
        Self {
            groups: HashMap::new(),
            mutes: HashMap::new(),
            caps: HashMap::new(),
            throttle_ms: i64::from(throttle_minutes.max(1)) * 60_000,
            regression_ms: i64::from(regression_hours.max(1)) * 3_600_000,
            cap,
        }
    }

    pub fn throttle_minutes(&self) -> u32 {
        (self.throttle_ms / 60_000) as u32
    }

    /// Records one occurrence; returns why to email now, if at all.
    pub fn observe(&mut self, key: &Key, user: &str, now: i64) -> Option<Reason> {
        let Some(g) = self.groups.get_mut(key) else {
            let muted_until = self.mutes.get(key).copied();
            self.groups.insert(
                key.clone(),
                Group {
                    first_seen: now,
                    last_seen: now,
                    total: 1,
                    muted_until,
                    dirty: true,
                    ..Group::default()
                },
            );
            if muted_until.is_some_and(|m| m > now) {
                return None;
            }
            return Some(Reason::First);
        };
        let quiet = now - g.last_seen;
        g.last_seen = now.max(g.last_seen);
        g.total += 1;
        g.dirty = true;
        if g.muted_until.is_some_and(|m| m > now) {
            return None;
        }
        if quiet >= self.regression_ms {
            g.pending = 0;
            g.pending_users.clear();
            return Some(Reason::Regression {
                quiet_hours: (quiet / 3_600_000) as u64,
            });
        }
        g.pending += 1;
        if !user.is_empty() && g.pending_users.len() < MAX_TRACKED_USERS {
            g.pending_users.insert(user.to_string());
        }
        if g.last_sent.is_none_or(|t| now - t >= self.throttle_ms) {
            return Some(self.digest(key));
        }
        None
    }

    fn digest(&self, key: &Key) -> Reason {
        let g = self.groups.get(key);
        Reason::Digest {
            count: g.map_or(0, |g| g.pending),
            users: g.map_or(0, |g| g.pending_users.len() as u64),
            minutes: self.throttle_minutes(),
        }
    }

    /// Groups whose throttle window ended with occurrences still pending.
    pub fn due_digests(&self, now: i64) -> Vec<(Key, Reason)> {
        self.groups
            .iter()
            .filter(|(_, g)| {
                g.pending > 0
                    && g.muted_until.is_none_or(|m| m <= now)
                    && g.last_sent.is_none_or(|t| now - t >= self.throttle_ms)
            })
            .map(|(k, _)| (k.clone(), self.digest(k)))
            .collect()
    }

    /// The project-wide hourly cap.
    pub fn admit(&mut self, project: Uuid, now: i64) -> Admit {
        let hour = now.div_euclid(3_600_000);
        let c = self.caps.entry(project).or_default();
        if c.hour != hour {
            *c = Cap {
                hour,
                ..Cap::default()
            };
        }
        if c.sent < self.cap {
            c.sent += 1;
            Admit::Send
        } else if !c.paused_notice_sent {
            c.paused_notice_sent = true;
            c.suppressed += 1;
            Admit::SendPausedNotice
        } else {
            c.suppressed += 1;
            Admit::Suppress
        }
    }

    /// The email for this group went out: start a new throttle window.
    pub fn mark_sent(&mut self, key: &Key, now: i64) {
        if let Some(g) = self.groups.get_mut(key) {
            g.last_sent = Some(now);
            g.pending = 0;
            g.pending_users.clear();
            g.dirty = true;
        }
    }

    /// Over the cap: keep the occurrence pending so a later digest covers it.
    pub fn mark_suppressed(&mut self, key: &Key, reason: &Reason) {
        if let Some(g) = self.groups.get_mut(key)
            && matches!(reason, Reason::First | Reason::Regression { .. })
        {
            g.pending += 1;
        }
    }

    /// Applies mutes loaded from Postgres (mute links may be clicked on any instance).
    pub fn set_mutes(&mut self, mutes: HashMap<Key, i64>) {
        for (k, g) in &mut self.groups {
            g.muted_until = mutes.get(k).copied();
        }
        self.mutes = mutes;
    }

    /// Forgets saved groups that have been quiet for `older_than_ms`. Keep it
    /// well above the regression window: a forgotten group that comes back is
    /// emailed as new rather than as a regression.
    pub fn prune(&mut self, now: i64, older_than_ms: i64) {
        self.groups
            .retain(|_, g| g.dirty || g.pending > 0 || now - g.last_seen < older_than_ms);
    }
}
