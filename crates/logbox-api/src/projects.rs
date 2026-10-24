//! Projects and their configuration: ingest keys, redaction rules, replay
//! targets, settings, plus discovered environments and services, releases
//! and the (read-only) retention settings.

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use logbox_core::admin::{self, Project};
use logbox_core::project::ProjectSettings;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::types::Json as SqlJson;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::auth::{Auth, Role, Scope};
use crate::error::{ApiError, ApiResult};
use crate::query::{fetch_json, param};
use crate::{AppState, now_ms};
use logbox_query::compile::{ParamValue, Sql};

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProjectOut {
    pub id: Uuid,
    pub slug: String,
    pub name: String,
    #[schema(value_type = Object)]
    pub settings: ProjectSettings,
    #[schema(value_type = String)]
    pub created_at: time::OffsetDateTime,
}

impl From<Project> for ProjectOut {
    fn from(p: Project) -> Self {
        Self {
            id: p.id,
            slug: p.slug,
            name: p.name,
            settings: p.settings,
            created_at: p.created_at,
        }
    }
}

async fn load_project(state: &AppState, id: Uuid) -> ApiResult<Project> {
    sqlx::query_as("SELECT id, slug, name, settings, created_at FROM projects WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| ApiError::not_found("project"))
}

#[utoipa::path(get, path = "/api/v1/projects", tag = "projects", responses((status = 200, body = Vec<ProjectOut>)))]
pub async fn list(State(state): State<AppState>, auth: Auth) -> ApiResult<Json<Vec<ProjectOut>>> {
    auth.require(Role::Viewer, Scope::Read)?;
    let rows: Vec<Project> = sqlx::query_as(
        "SELECT id, slug, name, settings, created_at FROM projects ORDER BY created_at",
    )
    .fetch_all(&state.pool)
    .await?;
    let only = match &auth.via {
        crate::auth::Method_::Token { project, .. } => *project,
        crate::auth::Method_::Session { .. } => None,
    };
    Ok(Json(
        rows.into_iter()
            .filter(|p| only.is_none_or(|o| o == p.id))
            .map(ProjectOut::from)
            .collect(),
    ))
}

#[derive(Deserialize, ToSchema)]
pub struct CreateProject {
    pub name: String,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProjectCreated {
    pub project: ProjectOut,
    /// The first ingest key's plaintext. Shown once.
    pub ingest_key: String,
}

#[utoipa::path(post, path = "/api/v1/projects", tag = "projects", request_body = CreateProject,
    responses((status = 200, body = ProjectCreated)))]
pub async fn create(
    State(state): State<AppState>,
    auth: Auth,
    Json(req): Json<CreateProject>,
) -> ApiResult<Json<ProjectCreated>> {
    auth.require(Role::Admin, Scope::Admin)?;
    let name = req.name.trim();
    if name.is_empty() || name.chars().count() > 100 {
        return Err(ApiError::bad_request(
            "give the project a name (up to 100 characters)",
        ));
    }
    let project = admin::create_project(&state.pool, name).await?;
    let key =
        admin::create_ingest_key(&state.pool, project.id, "default", Some(auth.user.id)).await?;
    Ok(Json(ProjectCreated {
        project: project.into(),
        ingest_key: key.token,
    }))
}

#[utoipa::path(get, path = "/api/v1/projects/{id}", tag = "projects", responses((status = 200, body = ProjectOut)))]
pub async fn get(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<ProjectOut>> {
    auth.require(Role::Viewer, Scope::Read)?;
    auth.check_project(id)?;
    Ok(Json(load_project(&state, id).await?.into()))
}

#[derive(Deserialize, ToSchema)]
pub struct UpdateProject {
    pub name: Option<String>,
    #[schema(value_type = Option<Object>)]
    pub settings: Option<ProjectSettings>,
}

#[utoipa::path(patch, path = "/api/v1/projects/{id}", tag = "projects", request_body = UpdateProject,
    responses((status = 200, body = ProjectOut)))]
pub async fn update(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateProject>,
) -> ApiResult<Json<ProjectOut>> {
    auth.require(Role::Admin, Scope::Admin)?;
    auth.check_project(id)?;
    if let Some(s) = &req.settings {
        if !(0.0..=1.0).contains(&s.sample_rate)
            || s.route_sample_rates
                .values()
                .any(|r| !(0.0..=1.0).contains(r))
        {
            return Err(ApiError::bad_request(
                "sample rates must be between 0 and 1",
            ));
        }
        if s.max_body_bytes > 1024 * 1024 {
            return Err(ApiError::bad_request("maxBodyBytes is limited to 1 MB"));
        }
    }
    let name = req.name.as_deref().map(str::trim).filter(|n| !n.is_empty());
    sqlx::query("UPDATE projects SET name = COALESCE($2, name), settings = COALESCE($3, settings) WHERE id = $1")
        .bind(id)
        .bind(name)
        .bind(req.settings.map(SqlJson))
        .execute(&state.pool)
        .await?;
    state.invalidate_keys(id);
    Ok(Json(load_project(&state, id).await?.into()))
}

#[utoipa::path(delete, path = "/api/v1/projects/{id}", tag = "projects", responses((status = 204)))]
pub async fn delete(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    auth.require(Role::Owner, Scope::Admin)?;
    let key = state.project_key(id).await?;
    sqlx::query("DELETE FROM projects WHERE id = $1")
        .bind(id)
        .execute(&state.pool)
        .await?;
    state.invalidate_keys(id);
    // Telemetry deletion can take a while; do it in the background.
    let ch = state.ch.writer.clone();
    tokio::spawn(async move {
        for table in [
            "requests",
            "spans",
            "events",
            "route_minute",
            "error_groups",
            "error_minute",
        ] {
            let res = ch
                .query(&format!("ALTER TABLE {table} DELETE WHERE project_id = ?"))
                .bind(&key)
                .execute()
                .await;
            if let Err(e) = res {
                tracing::warn!(table, error = %e, "deleting project telemetry");
            }
        }
    });
    Ok(StatusCode::NO_CONTENT)
}

// ---- ingest keys -------------------------------------------------------------------

#[derive(Serialize, ToSchema, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct IngestKey {
    pub id: Uuid,
    pub name: String,
    pub prefix: String,
    #[schema(value_type = String)]
    pub created_at: time::OffsetDateTime,
    #[schema(value_type = Option<String>)]
    pub last_used_at: Option<time::OffsetDateTime>,
    #[schema(value_type = Option<String>)]
    pub revoked_at: Option<time::OffsetDateTime>,
}

#[utoipa::path(get, path = "/api/v1/projects/{id}/keys", tag = "projects", responses((status = 200, body = Vec<IngestKey>)))]
pub async fn list_keys(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Vec<IngestKey>>> {
    auth.require(Role::Admin, Scope::Admin)?;
    auth.check_project(id)?;
    let rows = sqlx::query_as(
        "SELECT id, name, prefix, created_at, last_used_at, revoked_at FROM ingest_keys WHERE project_id = $1 ORDER BY created_at",
    )
    .bind(id)
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(rows))
}

#[derive(Deserialize, ToSchema)]
pub struct CreateKey {
    pub name: String,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct KeyCreated {
    pub id: Uuid,
    /// Shown once.
    pub token: String,
    pub prefix: String,
}

#[utoipa::path(post, path = "/api/v1/projects/{id}/keys", tag = "projects", request_body = CreateKey,
    responses((status = 200, body = KeyCreated)))]
pub async fn create_key(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
    Json(req): Json<CreateKey>,
) -> ApiResult<Json<KeyCreated>> {
    auth.require(Role::Admin, Scope::Admin)?;
    auth.check_project(id)?;
    state.project_key(id).await?;
    let name = req.name.trim();
    if name.is_empty() || name.chars().count() > 100 {
        return Err(ApiError::bad_request("give the key a name"));
    }
    let k = admin::create_ingest_key(&state.pool, id, name, Some(auth.user.id)).await?;
    Ok(Json(KeyCreated {
        id: k.id,
        token: k.token,
        prefix: k.prefix,
    }))
}

#[utoipa::path(delete, path = "/api/v1/projects/{id}/keys/{key_id}", tag = "projects", responses((status = 204)))]
pub async fn revoke_key(
    State(state): State<AppState>,
    auth: Auth,
    Path((id, key_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<StatusCode> {
    auth.require(Role::Admin, Scope::Admin)?;
    auth.check_project(id)?;
    let res = sqlx::query("UPDATE ingest_keys SET revoked_at = now() WHERE id = $1 AND project_id = $2 AND revoked_at IS NULL")
        .bind(key_id)
        .bind(id)
        .execute(&state.pool)
        .await?;
    if res.rows_affected() == 0 {
        return Err(ApiError::not_found("key"));
    }
    state.invalidate_keys(id);
    Ok(StatusCode::NO_CONTENT)
}

// ---- redaction rules ------------------------------------------------------------------

#[derive(Serialize, Deserialize, ToSchema, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct RedactionRule {
    #[serde(default)]
    pub id: Uuid,
    /// `header`, `body_key` or `regex`.
    pub kind: String,
    pub pattern: String,
}

#[utoipa::path(get, path = "/api/v1/projects/{id}/redaction-rules", tag = "projects", responses((status = 200, body = Vec<RedactionRule>)))]
pub async fn list_rules(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Vec<RedactionRule>>> {
    auth.require(Role::Viewer, Scope::Read)?;
    auth.check_project(id)?;
    let rows = sqlx::query_as(
        "SELECT id, kind, pattern FROM redaction_rules WHERE project_id = $1 ORDER BY created_at",
    )
    .bind(id)
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(rows))
}

#[utoipa::path(post, path = "/api/v1/projects/{id}/redaction-rules", tag = "projects", request_body = RedactionRule,
    responses((status = 200, body = RedactionRule)))]
pub async fn create_rule(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
    Json(req): Json<RedactionRule>,
) -> ApiResult<Json<RedactionRule>> {
    auth.require(Role::Admin, Scope::Admin)?;
    auth.check_project(id)?;
    state.project_key(id).await?;
    let pattern = req.pattern.trim().to_string();
    if pattern.is_empty() || pattern.len() > 500 {
        return Err(ApiError::bad_request(
            "the pattern must be 1 to 500 characters",
        ));
    }
    match req.kind.as_str() {
        "header" | "body_key" => {}
        "regex" => {
            regex::Regex::new(&pattern)
                .map_err(|e| ApiError::bad_request(format!("invalid regex: {e}")))?;
        }
        _ => {
            return Err(ApiError::bad_request(
                "kind must be header, body_key or regex",
            ));
        }
    }
    let rule = RedactionRule {
        id: Uuid::now_v7(),
        kind: req.kind,
        pattern,
    };
    sqlx::query(
        "INSERT INTO redaction_rules (id, project_id, kind, pattern) VALUES ($1, $2, $3, $4)",
    )
    .bind(rule.id)
    .bind(id)
    .bind(&rule.kind)
    .bind(&rule.pattern)
    .execute(&state.pool)
    .await?;
    state.invalidate_keys(id);
    Ok(Json(rule))
}

#[utoipa::path(delete, path = "/api/v1/projects/{id}/redaction-rules/{rule_id}", tag = "projects", responses((status = 204)))]
pub async fn delete_rule(
    State(state): State<AppState>,
    auth: Auth,
    Path((id, rule_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<StatusCode> {
    auth.require(Role::Admin, Scope::Admin)?;
    auth.check_project(id)?;
    sqlx::query("DELETE FROM redaction_rules WHERE id = $1 AND project_id = $2")
        .bind(rule_id)
        .bind(id)
        .execute(&state.pool)
        .await?;
    state.invalidate_keys(id);
    Ok(StatusCode::NO_CONTENT)
}

// ---- replay targets --------------------------------------------------------------------

#[derive(Serialize, Deserialize, ToSchema, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct ReplayTarget {
    #[serde(default)]
    pub id: Uuid,
    pub name: String,
    pub base_url: String,
}

/// Normalizes and checks a replay target URL: http(s), a host, no
/// credentials, query or fragment.
pub fn normalize_base_url(raw: &str) -> ApiResult<String> {
    let u = url::Url::parse(raw.trim())
        .map_err(|_| ApiError::bad_request("enter a full URL, e.g. https://staging.example.com"))?;
    if !matches!(u.scheme(), "http" | "https") || u.host_str().is_none() {
        return Err(ApiError::bad_request(
            "replay targets must be http or https URLs",
        ));
    }
    if !u.username().is_empty()
        || u.password().is_some()
        || u.query().is_some()
        || u.fragment().is_some()
    {
        return Err(ApiError::bad_request(
            "replay targets can't contain credentials, a query or a fragment",
        ));
    }
    Ok(u.as_str().trim_end_matches('/').to_string())
}

#[utoipa::path(get, path = "/api/v1/projects/{id}/replay-targets", tag = "projects", responses((status = 200, body = Vec<ReplayTarget>)))]
pub async fn list_targets(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Vec<ReplayTarget>>> {
    auth.require(Role::Viewer, Scope::Read)?;
    auth.check_project(id)?;
    let rows = sqlx::query_as(
        "SELECT id, name, base_url FROM replay_targets WHERE project_id = $1 ORDER BY name",
    )
    .bind(id)
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(rows))
}

#[utoipa::path(post, path = "/api/v1/projects/{id}/replay-targets", tag = "projects", request_body = ReplayTarget,
    responses((status = 200, body = ReplayTarget)))]
pub async fn create_target(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
    Json(req): Json<ReplayTarget>,
) -> ApiResult<Json<ReplayTarget>> {
    auth.require(Role::Admin, Scope::Admin)?;
    auth.check_project(id)?;
    state.project_key(id).await?;
    let name = req.name.trim().to_string();
    if name.is_empty() || name.chars().count() > 100 {
        return Err(ApiError::bad_request("give the target a name"));
    }
    let t = ReplayTarget {
        id: Uuid::now_v7(),
        name,
        base_url: normalize_base_url(&req.base_url)?,
    };
    sqlx::query(
        "INSERT INTO replay_targets (id, project_id, name, base_url) VALUES ($1, $2, $3, $4)",
    )
    .bind(t.id)
    .bind(id)
    .bind(&t.name)
    .bind(&t.base_url)
    .execute(&state.pool)
    .await?;
    Ok(Json(t))
}

#[utoipa::path(delete, path = "/api/v1/projects/{id}/replay-targets/{target_id}", tag = "projects", responses((status = 204)))]
pub async fn delete_target(
    State(state): State<AppState>,
    auth: Auth,
    Path((id, target_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<StatusCode> {
    auth.require(Role::Admin, Scope::Admin)?;
    auth.check_project(id)?;
    sqlx::query("DELETE FROM replay_targets WHERE id = $1 AND project_id = $2")
        .bind(target_id)
        .bind(id)
        .execute(&state.pool)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---- discovered environments and services ----------------------------------------------

fn pid_sql(project: &str) -> Sql {
    let mut sql = Sql::default();
    param(
        &mut sql,
        "pid",
        "String",
        ParamValue::Str(project.to_string()),
    );
    sql
}

#[utoipa::path(get, path = "/api/v1/projects/{id}/environments", tag = "projects", responses((status = 200, body = Vec<String>)))]
pub async fn environments(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Vec<String>>> {
    auth.require(Role::Viewer, Scope::Read)?;
    auth.check_project(id)?;
    let key = state.project_key(id).await?;
    let p = pid_sql(&key);
    let sql = "SELECT DISTINCT environment AS e FROM ( \
                 SELECT environment FROM route_minute WHERE project_id = {pid:String} AND minute > now() - INTERVAL 30 DAY \
                 UNION ALL \
                 SELECT environment FROM events WHERE project_id = {pid:String} AND timestamp > now() - INTERVAL 7 DAY \
               ) WHERE notEmpty(environment) ORDER BY e";
    let r = fetch_json(&state, sql, &p.params).await?;
    Ok(Json(
        r.rows
            .iter()
            .filter_map(|row| row.get("e").and_then(Value::as_str).map(str::to_string))
            .collect(),
    ))
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ServiceHealth {
    pub service: String,
    /// Last minute with traffic, ms since the epoch.
    pub last_seen: i64,
    /// Requests in the last 15 minutes.
    pub requests: u64,
    /// Share of 5xx in the last 15 minutes.
    pub error_rate: f64,
    /// `ok`, `degraded` (≥ 1% 5xx), `failing` (≥ 10%) or `idle`.
    pub health: &'static str,
}

#[derive(Deserialize, utoipa::IntoParams)]
pub struct EnvParam {
    pub environment: Option<String>,
}

#[utoipa::path(get, path = "/api/v1/projects/{id}/services", tag = "projects", params(EnvParam),
    responses((status = 200, body = Vec<ServiceHealth>)))]
pub async fn services(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
    Query(q): Query<EnvParam>,
) -> ApiResult<Json<Vec<ServiceHealth>>> {
    auth.require(Role::Viewer, Scope::Read)?;
    auth.check_project(id)?;
    let key = state.project_key(id).await?;
    let mut p = pid_sql(&key);
    let env_clause = match q.environment.as_deref().filter(|e| !e.is_empty()) {
        Some(e) => {
            let ph = param(&mut p, "env", "String", ParamValue::Str(e.to_string()));
            format!(" AND environment = {ph}")
        }
        None => String::new(),
    };
    let sql = format!(
        "SELECT service, toUnixTimestamp(max(minute)) * 1000 AS last_seen, \
           countMergeIf(requests, minute >= now() - INTERVAL 15 MINUTE) AS recent, \
           countIfMergeIf(errors_5xx, minute >= now() - INTERVAL 15 MINUTE) AS errors \
         FROM route_minute WHERE project_id = {{pid:String}} AND minute > now() - INTERVAL 7 DAY{env_clause} \
         GROUP BY service ORDER BY service"
    );
    let r = fetch_json(&state, &sql, &p.params).await?;
    let num = |v: Option<&Value>| {
        v.and_then(|v| {
            v.as_u64()
                .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
        })
        .unwrap_or(0)
    };
    let out = r
        .rows
        .iter()
        .map(|row| {
            let requests = num(row.get("recent"));
            let errors = num(row.get("errors"));
            let error_rate = if requests > 0 {
                errors as f64 / requests as f64
            } else {
                0.0
            };
            ServiceHealth {
                service: row
                    .get("service")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                last_seen: i64::try_from(num(row.get("last_seen"))).unwrap_or(0),
                requests,
                error_rate: (error_rate * 10_000.0).round() / 10_000.0,
                health: if requests == 0 {
                    "idle"
                } else if error_rate >= 0.10 {
                    "failing"
                } else if error_rate >= 0.01 {
                    "degraded"
                } else {
                    "ok"
                },
            }
        })
        .collect();
    Ok(Json(out))
}

// ---- releases ------------------------------------------------------------------------

#[derive(Serialize, ToSchema, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct Release {
    pub id: Uuid,
    pub environment: String,
    pub service: String,
    pub version: String,
    pub commit_sha: Option<String>,
    pub url: Option<String>,
    /// ms since the epoch.
    pub at: i64,
}

#[derive(Deserialize, utoipa::IntoParams)]
pub struct ReleasesQuery {
    pub from: Option<i64>,
    pub to: Option<i64>,
    pub service: Option<String>,
    pub environment: Option<String>,
}

/// Deploy markers for time charts.
#[utoipa::path(get, path = "/api/v1/projects/{id}/releases", tag = "projects", params(ReleasesQuery),
    responses((status = 200, body = Vec<Release>)))]
pub async fn releases(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
    Query(q): Query<ReleasesQuery>,
) -> ApiResult<Json<Vec<Release>>> {
    auth.require(Role::Viewer, Scope::Read)?;
    auth.check_project(id)?;
    let to = q.to.unwrap_or_else(now_ms);
    let from = q.from.unwrap_or(to - 7 * 86_400_000);
    let rows = sqlx::query_as(
        "SELECT id, environment, service, version, commit_sha, url, \
           (extract(epoch FROM released_at) * 1000)::bigint AS at \
         FROM releases WHERE project_id = $1 \
           AND released_at BETWEEN to_timestamp($2::double precision / 1000) AND to_timestamp($3::double precision / 1000) \
           AND ($4::text IS NULL OR service = $4) \
           AND ($5::text IS NULL OR environment = '' OR environment = $5) \
         ORDER BY released_at LIMIT 500",
    )
    .bind(id)
    .bind(from)
    .bind(to)
    .bind(q.service.filter(|s| !s.is_empty()))
    .bind(q.environment.filter(|s| !s.is_empty()))
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(rows))
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Retention {
    pub days: u32,
    pub body_days: u32,
}

/// Retention comes from the server config and is shown read-only.
#[utoipa::path(get, path = "/api/v1/settings/retention", tag = "projects", responses((status = 200, body = Retention)))]
pub async fn retention(State(state): State<AppState>, auth: Auth) -> ApiResult<Json<Retention>> {
    auth.require(Role::Viewer, Scope::Read)?;
    Ok(Json(Retention {
        days: state.config.retention.days,
        body_days: state.config.retention.body_days,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replay_target_urls() {
        assert_eq!(
            normalize_base_url("https://staging.example.com/").unwrap(),
            "https://staging.example.com"
        );
        assert_eq!(
            normalize_base_url("http://10.0.0.5:8080/api").unwrap(),
            "http://10.0.0.5:8080/api"
        );
        for bad in [
            "ftp://x",
            "https://user:pw@x.com",
            "https://x.com?a=1",
            "not a url",
            "file:///etc/passwd",
        ] {
            assert!(normalize_base_url(bad).is_err(), "{bad}");
        }
    }
}
