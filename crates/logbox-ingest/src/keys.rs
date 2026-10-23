//! Ingest key resolution with a small in-memory cache, so the hot path does
//! not hit Postgres for every batch.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use logbox_core::project::ProjectSettings;
use logbox_core::secrets::{INGEST_KEY_PREFIX, hash_token};
use sqlx::PgPool;
use sqlx::types::Json;
use uuid::Uuid;

use crate::redact::{Redactor, Rule};

const POSITIVE_TTL: Duration = Duration::from_secs(30);
const NEGATIVE_TTL: Duration = Duration::from_secs(10);
const MAX_ENTRIES: usize = 10_000;

/// Everything ingest needs to know about the project a key belongs to.
#[derive(Debug)]
pub struct ProjectCtx {
    pub project_id: Uuid,
    /// `project_id` as stored in ClickHouse.
    pub project_key: String,
    pub key_id: Uuid,
    pub settings: ProjectSettings,
    pub redactor: Redactor,
}

struct Entry {
    ctx: Option<Arc<ProjectCtx>>,
    at: Instant,
}

pub struct KeyCache {
    pool: PgPool,
    entries: RwLock<HashMap<String, Entry>>,
}

impl KeyCache {
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool,
            entries: RwLock::new(HashMap::new()),
        }
    }

    /// Resolves a plaintext ingest key. `Ok(None)` means unknown or revoked.
    pub async fn resolve(&self, token: &str) -> Result<Option<Arc<ProjectCtx>>, sqlx::Error> {
        if !token.starts_with(INGEST_KEY_PREFIX) || token.len() > 128 {
            return Ok(None);
        }
        let hash = hash_token(token);
        if let Some(hit) = self.cached(&hash) {
            return Ok(hit);
        }
        let ctx = self.load(&hash).await?;
        if let Ok(mut map) = self.entries.write() {
            if map.len() >= MAX_ENTRIES {
                map.clear();
            }
            map.insert(
                hash,
                Entry {
                    ctx: ctx.clone(),
                    at: Instant::now(),
                },
            );
        }
        Ok(ctx)
    }

    /// Drops cached entries for a project (after its keys, rules or settings change).
    pub fn invalidate_project(&self, project_id: Uuid) {
        if let Ok(mut map) = self.entries.write() {
            map.retain(|_, e| e.ctx.as_ref().is_none_or(|c| c.project_id != project_id));
        }
    }

    fn cached(&self, hash: &str) -> Option<Option<Arc<ProjectCtx>>> {
        let map = self.entries.read().ok()?;
        let e = map.get(hash)?;
        let ttl = if e.ctx.is_some() {
            POSITIVE_TTL
        } else {
            NEGATIVE_TTL
        };
        (e.at.elapsed() < ttl).then(|| e.ctx.clone())
    }

    async fn load(&self, hash: &str) -> Result<Option<Arc<ProjectCtx>>, sqlx::Error> {
        let row: Option<(Uuid, Uuid, Json<ProjectSettings>)> = sqlx::query_as(
            "SELECT k.id, k.project_id, p.settings \
             FROM ingest_keys k JOIN projects p ON p.id = k.project_id \
             WHERE k.key_hash = $1 AND k.revoked_at IS NULL",
        )
        .bind(hash)
        .fetch_optional(&self.pool)
        .await?;
        let Some((key_id, project_id, Json(settings))) = row else {
            return Ok(None);
        };
        let rules: Vec<(String, String)> =
            sqlx::query_as("SELECT kind, pattern FROM redaction_rules WHERE project_id = $1")
                .bind(project_id)
                .fetch_all(&self.pool)
                .await?;
        let rules: Vec<Rule> = rules
            .into_iter()
            .filter_map(|(kind, pattern)| match kind.as_str() {
                "header" => Some(Rule::Header(pattern)),
                "body_key" => Some(Rule::BodyKey(pattern)),
                "regex" => Some(Rule::Regex(pattern)),
                _ => None,
            })
            .collect();
        // Best effort; never fail ingest over bookkeeping.
        let pool = self.pool.clone();
        tokio::spawn(async move {
            let _ = sqlx::query("UPDATE ingest_keys SET last_used_at = now() WHERE id = $1")
                .bind(key_id)
                .execute(&pool)
                .await;
        });
        Ok(Some(Arc::new(ProjectCtx {
            project_id,
            project_key: project_id.to_string(),
            key_id,
            redactor: Redactor::from_rules(&rules, settings.mask_ip),
            settings,
        })))
    }
}
