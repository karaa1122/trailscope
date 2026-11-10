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

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Org {
    pub id: Uuid,
    pub slug: String,
    pub name: String,
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

/// A slug derived from `name` that `exists_sql` reports as free. `exists_sql`
/// takes the candidate as `$1` and, when `scope` is set, the scope as `$2`.
async fn free_slug(
    pool: &PgPool,
    name: &str,
    fallback: &str,
    exists_sql: &'static str,
    scope: Option<Uuid>,
) -> Result<String, sqlx::Error> {
    let base = if name.chars().any(|c| c.is_ascii_alphanumeric()) {
        slugify(name)
    } else {
        fallback.to_string()
    };
    let mut slug = base.clone();
    for n in 2..1000 {
        let query = sqlx::query_scalar(exists_sql).bind(&slug);
        let taken: bool = match scope {
            Some(id) => query.bind(id).fetch_one(pool).await?,
            None => query.fetch_one(pool).await?,
        };
        if !taken {
            break;
        }
        slug = format!("{base}-{n}");
    }
    Ok(slug)
}

/// Creates an organization with a unique slug derived from its name.
pub async fn create_org(pool: &PgPool, name: &str) -> Result<Org, sqlx::Error> {
    let slug = free_slug(
        pool,
        name,
        "organization",
        "SELECT EXISTS (SELECT 1 FROM organizations WHERE slug = $1)",
        None,
    )
    .await?;
    sqlx::query_as(
        "INSERT INTO organizations (id, slug, name) VALUES ($1, $2, $3) RETURNING id, slug, name",
    )
    .bind(Uuid::now_v7())
    .bind(&slug)
    .bind(name.trim())
    .fetch_one(pool)
    .await
}

/// The instance's first organization, created ("Default") when there is none.
/// The demo, single sign-on's automatic accounts and tests use it.
pub async fn default_org(pool: &PgPool) -> Result<Org, sqlx::Error> {
    let first: Option<Org> =
        sqlx::query_as("SELECT id, slug, name FROM organizations ORDER BY created_at, id LIMIT 1")
            .fetch_optional(pool)
            .await?;
    match first {
        Some(org) => Ok(org),
        None => create_org(pool, "Default").await,
    }
}

/// Adds a user to an organization (or updates their role there).
pub async fn add_member(
    pool: &PgPool,
    org_id: Uuid,
    user_id: Uuid,
    role: &str,
    can_view_bodies: bool,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO memberships (org_id, user_id, role, can_view_bodies) VALUES ($1, $2, $3, $4) \
         ON CONFLICT (org_id, user_id) DO UPDATE SET role = excluded.role, \
         can_view_bodies = excluded.can_view_bodies, disabled = FALSE",
    )
    .bind(org_id)
    .bind(user_id)
    .bind(role)
    .bind(can_view_bodies)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn create_project(
    pool: &PgPool,
    org_id: Uuid,
    name: &str,
) -> Result<Project, sqlx::Error> {
    let slug = free_slug(
        pool,
        name,
        "project",
        "SELECT EXISTS (SELECT 1 FROM projects WHERE slug = $1 AND org_id = $2)",
        Some(org_id),
    )
    .await?;
    let project: Project = sqlx::query_as(
        "INSERT INTO projects (id, org_id, slug, name, settings) VALUES ($1, $2, $3, $4, $5) \
         RETURNING id, slug, name, settings, created_at",
    )
    .bind(Uuid::now_v7())
    .bind(org_id)
    .bind(&slug)
    .bind(name.trim())
    .bind(Json(ProjectSettings::default()))
    .fetch_one(pool)
    .await?;
    // Every project gets the built-in "Email on 5xx" rule. It sends nothing
    // until SMTP is configured and recipients are listed.
    sqlx::query("INSERT INTO email_rules (project_id) VALUES ($1) ON CONFLICT DO NOTHING")
        .bind(project.id)
        .execute(pool)
        .await?;
    Ok(project)
}

pub async fn project_by_slug(
    pool: &PgPool,
    org_id: Uuid,
    slug: &str,
) -> Result<Option<Project>, sqlx::Error> {
    sqlx::query_as(
        "SELECT id, slug, name, settings, created_at FROM projects WHERE org_id = $1 AND slug = $2",
    )
    .bind(org_id)
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
