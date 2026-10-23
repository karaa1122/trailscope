//! Metadata operations shared by the API, the demo and tests.

use serde::Serialize;
use sqlx::PgPool;
use sqlx::types::Json;
use uuid::Uuid;

use crate::project::ProjectSettings;
use crate::secrets::{INGEST_KEY_PREFIX, display_prefix, generate_token, hash_token};

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Project {
    pub id: Uuid,
    pub slug: String,
    pub name: String,
    #[sqlx(json)]
    pub settings: ProjectSettings,
    pub created_at: time::OffsetDateTime,
}

/// A newly created ingest key. `token` is the only time the plaintext exists.
#[derive(Debug, Clone, Serialize)]
pub struct NewKey {
    pub id: Uuid,
    pub token: String,
    pub prefix: String,
}

/// `My Project!` -> `my-project`
pub fn slugify(name: &str) -> String {
    let mut out = String::new();
    for c in name.trim().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let out = out.trim_end_matches('-').to_string();
    if out.is_empty() {
        "project".into()
    } else {
        out
    }
}

pub async fn create_project(pool: &PgPool, name: &str) -> Result<Project, sqlx::Error> {
    let base = slugify(name);
    let mut slug = base.clone();
    for n in 2..100 {
        let taken: bool =
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM projects WHERE slug = $1)")
                .bind(&slug)
                .fetch_one(pool)
                .await?;
        if !taken {
            break;
        }
        slug = format!("{base}-{n}");
    }
    sqlx::query_as(
        "INSERT INTO projects (id, slug, name, settings) VALUES ($1, $2, $3, $4) \
         RETURNING id, slug, name, settings, created_at",
    )
    .bind(Uuid::now_v7())
    .bind(&slug)
    .bind(name.trim())
    .bind(Json(ProjectSettings::default()))
    .fetch_one(pool)
    .await
}

pub async fn project_by_slug(pool: &PgPool, slug: &str) -> Result<Option<Project>, sqlx::Error> {
    sqlx::query_as("SELECT id, slug, name, settings, created_at FROM projects WHERE slug = $1")
        .bind(slug)
        .fetch_optional(pool)
        .await
}

pub async fn create_ingest_key(
    pool: &PgPool,
    project_id: Uuid,
    name: &str,
    created_by: Option<Uuid>,
) -> Result<NewKey, sqlx::Error> {
    let token = generate_token(INGEST_KEY_PREFIX);
    let id = Uuid::now_v7();
    let prefix = display_prefix(&token);
    sqlx::query(
        "INSERT INTO ingest_keys (id, project_id, name, key_hash, prefix, created_by) \
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(id)
    .bind(project_id)
    .bind(name)
    .bind(hash_token(&token))
    .bind(&prefix)
    .bind(created_by)
    .execute(pool)
    .await?;
    Ok(NewKey { id, token, prefix })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs() {
        assert_eq!(slugify("My Project!"), "my-project");
        assert_eq!(slugify("  Public  API v2 "), "public-api-v2");
        assert_eq!(slugify("***"), "project");
    }
}
