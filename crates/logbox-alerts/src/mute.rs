//! Signed, expiring "mute this error" links that work without signing in.

use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use crate::crypto::Keys;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MuteToken {
    /// Project ID.
    pub p: Uuid,
    /// Error fingerprint.
    pub f: String,
    /// Mute duration in hours; 0 mutes forever.
    pub h: u32,
    /// Link expiry, unix seconds.
    pub e: i64,
}

impl MuteToken {
    pub fn label(&self) -> &'static str {
        match self.h {
            0 => "forever",
            1 => "for 1 hour",
            24 => "for 24 hours",
            _ => "for a while",
        }
    }
}

pub fn token(keys: &Keys, t: &MuteToken) -> String {
    keys.sign_token(&serde_json::to_vec(t).unwrap_or_default())
}

/// A valid, unexpired token.
pub fn open(keys: &Keys, token: &str, now_secs: i64) -> Option<MuteToken> {
    let payload = keys.open_token(token)?;
    let t: MuteToken = serde_json::from_slice(&payload).ok()?;
    (t.e >= now_secs).then_some(t)
}

pub fn link(public_url: &str, keys: &Keys, t: &MuteToken) -> String {
    format!(
        "{}/api/v1/alerts/mute?t={}",
        public_url.trim_end_matches('/'),
        token(keys, t)
    )
}

pub async fn apply(pool: &PgPool, t: &MuteToken) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO email_groups (project_id, fingerprint, first_seen_at, last_seen_at, muted_until) \
         VALUES ($1, $2, now(), now(), CASE WHEN $3 = 0 THEN 'infinity'::timestamptz ELSE now() + make_interval(hours => $3) END) \
         ON CONFLICT (project_id, fingerprint) DO UPDATE SET muted_until = EXCLUDED.muted_until",
    )
    .bind(t.p)
    .bind(&t.f)
    .bind(t.h as i32)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn unmute(pool: &PgPool, project: Uuid, fingerprint: &str) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE email_groups SET muted_until = NULL WHERE project_id = $1 AND fingerprint = $2",
    )
    .bind(project)
    .bind(fingerprint)
    .execute(pool)
    .await?;
    Ok(())
}
