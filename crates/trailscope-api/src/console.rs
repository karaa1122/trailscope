//! The management API (decisions 071, 072): every organization on the server,
//! their plans (limit tiers) and usage, for the operator's own tools such as a
//! separate admin dashboard. It has no screens in Trailscope itself, and only
//! answers `Authorization: Bearer <TRAILSCOPE_ADMIN_TOKEN>`; sessions and API
//! tokens, which act inside one organization, never reach it.

use axum::Json;
use axum::extract::{FromRequestParts, Path, State};
use axum::http::request::Parts;
use axum::http::{StatusCode, header};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use time::OffsetDateTime;
use trailscope_core::admin;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::AppState;
use crate::auth::Role;
use crate::error::{ApiError, ApiResult};
use crate::plans::{MONTH_START, PLAN_COLUMNS, Plan, project_month_events};
use crate::users::{InviteCreated, issue_invite};

/// Shortest admin token the server accepts.
pub const MIN_ADMIN_TOKEN: usize = 32;

/// Proof that the request carried the admin token.
pub struct AdminAuth;

impl FromRequestParts<AppState> for AdminAuth {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let expected = state.config.admin_token.trim();
        if expected.len() < MIN_ADMIN_TOKEN {
            return Err(ApiError::new(
                StatusCode::NOT_FOUND,
                "not_found",
                "the management API is off; set TRAILSCOPE_ADMIN_TOKEN (32 characters or more)",
            ));
        }
        let given = parts
            .headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .map(str::trim);
        match given {
            None => Err(ApiError::unauthorized()),
            Some(t) if same(t, expected) => Ok(AdminAuth),
            Some(_) => Err(ApiError::forbidden("that isn't the admin token")),
        }
    }
}

/// Compares digests in constant time, so the comparison can't leak how much
/// of the token matched.
fn same(a: &str, b: &str) -> bool {
    let (x, y) = (Sha256::digest(a.as_bytes()), Sha256::digest(b.as_bytes()));
    x.iter()
        .zip(y.iter())
        .fold(0u8, |acc, (p, q)| acc | (p ^ q))
        == 0
}

// ---- organizations -----------------------------------------------------------

#[derive(Serialize, ToSchema, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct AdminOrg {
    pub id: Uuid,
    pub slug: String,
    pub name: String,
    pub plan_id: String,
    pub plan_name: String,
    pub suspended: bool,
    #[schema(value_type = String)]
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    /// The operator's private note.
    pub note: String,
    pub members: i64,
    pub projects: i64,
    /// Accepted events this calendar month (UTC).
    pub events_this_month: i64,
    /// The plan's monthly events; `null` = unlimited.
    pub events_limit: Option<i64>,
    pub hard_limit: bool,
    /// Owners' email addresses.
    pub owners: Vec<String>,
}

fn org_sql(filter: &str) -> String {
    format!(
        "SELECT o.id, o.slug, o.name, o.plan_id, p.name AS plan_name, \
                o.suspended_at IS NOT NULL AS suspended, o.created_at, o.admin_note AS note, \
                (SELECT count(*) FROM memberships m WHERE m.org_id = o.id AND NOT m.disabled) AS members, \
                (SELECT count(*) FROM projects pr WHERE pr.org_id = o.id) AS projects, \
                COALESCE((SELECT SUM(u.events) FROM usage_daily u \
                          WHERE u.org_id = o.id AND u.day >= {MONTH_START}), 0)::bigint AS events_this_month, \
                p.events_per_month AS events_limit, p.hard_limit, \
                ARRAY(SELECT us.email FROM memberships m JOIN users us ON us.id = m.user_id \
                      WHERE m.org_id = o.id AND m.role = 'owner' AND NOT m.disabled \
                      ORDER BY m.created_at) AS owners \
         FROM organizations o JOIN plans p ON p.id = o.plan_id {filter} ORDER BY o.created_at, o.id"
    )
}

async fn load_org(state: &AppState, id: Uuid) -> ApiResult<AdminOrg> {
    sqlx::query_as::<_, AdminOrg>(sqlx::AssertSqlSafe(org_sql("WHERE o.id = $1")))
        .bind(id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| ApiError::not_found("organization"))
}

#[utoipa::path(get, path = "/api/v1/admin/orgs", tag = "admin", responses((status = 200, body = Vec<AdminOrg>)))]
pub async fn list_orgs(
    State(state): State<AppState>,
    _: AdminAuth,
) -> ApiResult<Json<Vec<AdminOrg>>> {
    Ok(Json(
        sqlx::query_as(sqlx::AssertSqlSafe(org_sql("")))
            .fetch_all(&state.pool)
            .await?,
    ))
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreateOrg {
    pub name: String,
    /// Defaults to `free`.
    pub plan_id: Option<String>,
    /// Creates an owner invite link for this address.
    pub owner_email: Option<String>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct OrgCreated {
    pub org: AdminOrg,
    /// The owner's invite link (shown once), when an email was given.
    pub invite: Option<InviteCreated>,
}

fn valid_org_name(name: &str) -> ApiResult<&str> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 100 {
        return Err(ApiError::bad_request(
            "give the organization a name (up to 100 characters)",
        ));
    }
    Ok(name)
}

/// A plan that can be given to an organization (exists, not archived unless
/// it already has it).
async fn assignable_plan(state: &AppState, plan: &str, current: Option<&str>) -> ApiResult<()> {
    let archived: Option<bool> = sqlx::query_scalar("SELECT archived FROM plans WHERE id = $1")
        .bind(plan)
        .fetch_optional(&state.pool)
        .await?;
    match archived {
        None => Err(ApiError::bad_request(format!("there is no plan `{plan}`"))),
        Some(true) if current != Some(plan) => Err(ApiError::bad_request(format!(
            "the plan `{plan}` is archived"
        ))),
        Some(_) => Ok(()),
    }
}

/// Creates an organization, optionally with the owner invite link to send its
/// first owner.
#[utoipa::path(post, path = "/api/v1/admin/orgs", tag = "admin", request_body = CreateOrg,
    responses((status = 200, body = OrgCreated)))]
pub async fn create_org(
    State(state): State<AppState>,
    _: AdminAuth,
    Json(req): Json<CreateOrg>,
) -> ApiResult<Json<OrgCreated>> {
    let name = valid_org_name(&req.name)?;
    let plan = req
        .plan_id
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .unwrap_or("free")
        .to_string();
    assignable_plan(&state, &plan, None).await?;
    let owner = req
        .owner_email
        .as_deref()
        .map(str::trim)
        .filter(|e| !e.is_empty());
    if let Some(e) = owner
        && !crate::users::valid_email(&e.to_lowercase())
    {
        return Err(ApiError::bad_request("enter a valid email for the owner"));
    }
    let org = admin::create_org(&state.pool, name).await?;
    sqlx::query("UPDATE organizations SET plan_id = $2 WHERE id = $1")
        .bind(org.id)
        .bind(&plan)
        .execute(&state.pool)
        .await?;
    let invite = match owner {
        Some(email) => match issue_invite(&state, org.id, email, Role::Owner, true, None).await {
            Ok(i) => Some(i),
            Err(e) => {
                // Don't leave an organization nobody can get into.
                sqlx::query("DELETE FROM organizations WHERE id = $1")
                    .bind(org.id)
                    .execute(&state.pool)
                    .await?;
                return Err(e);
            }
        },
        None => None,
    };
    Ok(Json(OrgCreated {
        org: load_org(&state, org.id).await?,
        invite,
    }))
}

#[derive(Serialize, ToSchema, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct AdminMember {
    pub id: Uuid,
    pub email: String,
    pub name: String,
    pub role: String,
    pub disabled: bool,
    #[schema(value_type = Option<String>)]
    #[serde(with = "time::serde::rfc3339::option")]
    pub last_login_at: Option<OffsetDateTime>,
}

#[derive(Serialize, ToSchema, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct AdminProject {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    #[schema(value_type = String)]
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    /// Requests, job runs and log lines stored this month (from ClickHouse).
    #[sqlx(skip)]
    pub events_this_month: Option<i64>,
}

#[derive(Serialize, ToSchema, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct AdminInvite {
    pub email: String,
    pub role: String,
    #[schema(value_type = String)]
    #[serde(with = "time::serde::rfc3339")]
    pub expires_at: OffsetDateTime,
}

#[derive(Serialize, ToSchema, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct UsageDay {
    /// `YYYY-MM-DD` (UTC).
    pub day: String,
    pub events: i64,
    pub bytes: i64,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AdminOrgDetail {
    pub org: AdminOrg,
    pub members: Vec<AdminMember>,
    pub projects: Vec<AdminProject>,
    pub invites: Vec<AdminInvite>,
    /// The last 31 days.
    pub usage: Vec<UsageDay>,
}

#[utoipa::path(get, path = "/api/v1/admin/orgs/{id}", tag = "admin", responses((status = 200, body = AdminOrgDetail)))]
pub async fn get_org(
    State(state): State<AppState>,
    _: AdminAuth,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<AdminOrgDetail>> {
    let org = load_org(&state, id).await?;
    let members = sqlx::query_as(
        "SELECT u.id, u.email, u.name, m.role, m.disabled, u.last_login_at \
         FROM memberships m JOIN users u ON u.id = m.user_id WHERE m.org_id = $1 \
         ORDER BY m.created_at",
    )
    .bind(id)
    .fetch_all(&state.pool)
    .await?;
    let mut projects: Vec<AdminProject> = sqlx::query_as(
        "SELECT id, name, slug, created_at FROM projects WHERE org_id = $1 ORDER BY created_at",
    )
    .bind(id)
    .fetch_all(&state.pool)
    .await?;
    let ids: Vec<Uuid> = projects.iter().map(|p| p.id).collect();
    let counts = project_month_events(&state, &ids).await;
    if !counts.is_empty() {
        for p in &mut projects {
            p.events_this_month = Some(counts.get(&p.id).copied().unwrap_or(0));
        }
    }
    let invites = sqlx::query_as(
        "SELECT email, role, expires_at FROM invites \
         WHERE org_id = $1 AND accepted_at IS NULL AND expires_at > now() ORDER BY created_at DESC",
    )
    .bind(id)
    .fetch_all(&state.pool)
    .await?;
    let usage = sqlx::query_as(
        "SELECT to_char(day, 'YYYY-MM-DD') AS day, events, bytes FROM usage_daily \
         WHERE org_id = $1 AND day > (now() AT TIME ZONE 'UTC')::date - 31 ORDER BY day",
    )
    .bind(id)
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(AdminOrgDetail {
        org,
        members,
        projects,
        invites,
        usage,
    }))
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UpdateOrg {
    pub name: Option<String>,
    pub plan_id: Option<String>,
    /// Suspended organizations' ingest keys are refused (with `cloud.enabled`).
    pub suspended: Option<bool>,
    pub note: Option<String>,
}

#[utoipa::path(patch, path = "/api/v1/admin/orgs/{id}", tag = "admin", request_body = UpdateOrg,
    responses((status = 200, body = AdminOrg)))]
pub async fn update_org(
    State(state): State<AppState>,
    _: AdminAuth,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateOrg>,
) -> ApiResult<Json<AdminOrg>> {
    let current = load_org(&state, id).await?;
    if let Some(plan) = &req.plan_id {
        assignable_plan(&state, plan, Some(&current.plan_id)).await?;
    }
    let name = req.name.as_deref().map(valid_org_name).transpose()?;
    let mut tx = state.pool.begin().await?;
    if let Some(name) = name {
        sqlx::query("UPDATE organizations SET name = $2 WHERE id = $1")
            .bind(id)
            .bind(name)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(plan) = &req.plan_id {
        sqlx::query("UPDATE organizations SET plan_id = $2 WHERE id = $1")
            .bind(id)
            .bind(plan)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(suspended) = req.suspended {
        sqlx::query(
            "UPDATE organizations SET suspended_at = CASE WHEN $2 THEN COALESCE(suspended_at, now()) END \
             WHERE id = $1",
        )
        .bind(id)
        .bind(suspended)
        .execute(&mut *tx)
        .await?;
    }
    if let Some(note) = &req.note {
        sqlx::query("UPDATE organizations SET admin_note = $2 WHERE id = $1")
            .bind(id)
            .bind(note.chars().take(4000).collect::<String>())
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(Json(load_org(&state, id).await?))
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AdminInviteReq {
    pub email: String,
    pub role: Role,
    #[serde(default)]
    pub can_view_bodies: bool,
}

/// An invite link to any organization, owners included.
#[utoipa::path(post, path = "/api/v1/admin/orgs/{id}/invites", tag = "admin", request_body = AdminInviteReq,
    responses((status = 200, body = InviteCreated)))]
pub async fn invite(
    State(state): State<AppState>,
    _: AdminAuth,
    Path(id): Path<Uuid>,
    Json(req): Json<AdminInviteReq>,
) -> ApiResult<Json<InviteCreated>> {
    load_org(&state, id).await?;
    Ok(Json(
        issue_invite(&state, id, &req.email, req.role, req.can_view_bodies, None).await?,
    ))
}

// ---- plans -------------------------------------------------------------------

#[derive(Serialize, ToSchema, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct PlanOut {
    #[serde(flatten)]
    #[sqlx(flatten)]
    pub plan: Plan,
    /// Organizations on this plan.
    pub orgs: i64,
}

async fn load_plans(state: &AppState, id: Option<&str>) -> ApiResult<Vec<PlanOut>> {
    let filter = if id.is_some() { "WHERE p.id = $1" } else { "" };
    let sql = format!(
        "SELECT {PLAN_COLUMNS}, (SELECT count(*) FROM organizations o WHERE o.plan_id = p.id) AS orgs \
         FROM plans p {filter} ORDER BY p.position, p.created_at"
    );
    let mut q = sqlx::query_as::<_, PlanOut>(sqlx::AssertSqlSafe(sql));
    if let Some(id) = id {
        q = q.bind(id.to_string());
    }
    Ok(q.fetch_all(&state.pool).await?)
}

#[utoipa::path(get, path = "/api/v1/admin/plans", tag = "admin", responses((status = 200, body = Vec<PlanOut>)))]
pub async fn list_plans(
    State(state): State<AppState>,
    _: AdminAuth,
) -> ApiResult<Json<Vec<PlanOut>>> {
    Ok(Json(load_plans(&state, None).await?))
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PlanIn {
    pub name: String,
    /// `null` = unlimited.
    pub events_per_month: Option<i64>,
    #[serde(default)]
    pub hard_limit: bool,
    pub max_projects: Option<i32>,
    pub max_members: Option<i32>,
    #[serde(default)]
    pub archived: bool,
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreatePlan {
    /// Lowercase letters, digits and dashes, e.g. `team-yearly`.
    pub id: String,
    #[serde(flatten)]
    pub plan: PlanIn,
}

fn validate_plan(p: &PlanIn) -> ApiResult<()> {
    let name = p.name.trim();
    if name.is_empty() || name.chars().count() > 60 {
        return Err(ApiError::bad_request(
            "give the plan a name (up to 60 characters)",
        ));
    }
    if p.events_per_month.is_some_and(|n| n < 0) {
        return Err(ApiError::bad_request("limits can't be negative"));
    }
    if p.max_projects.is_some_and(|n| n < 1) || p.max_members.is_some_and(|n| n < 1) {
        return Err(ApiError::bad_request(
            "project and member caps are at least 1 (leave empty for unlimited)",
        ));
    }
    Ok(())
}

fn valid_plan_id(id: &str) -> bool {
    let mut chars = id.chars();
    id.len() <= 40
        && chars
            .next()
            .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

#[utoipa::path(post, path = "/api/v1/admin/plans", tag = "admin", request_body = CreatePlan,
    responses((status = 200, body = PlanOut), (status = 409)))]
pub async fn create_plan(
    State(state): State<AppState>,
    _: AdminAuth,
    Json(req): Json<CreatePlan>,
) -> ApiResult<Json<PlanOut>> {
    let id = req.id.trim().to_string();
    if !valid_plan_id(&id) {
        return Err(ApiError::bad_request(
            "the plan ID uses lowercase letters, digits and dashes (up to 40)",
        ));
    }
    validate_plan(&req.plan)?;
    let p = &req.plan;
    let inserted = sqlx::query(
        "INSERT INTO plans (id, name, events_per_month, hard_limit, max_projects, max_members, archived, position) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, (SELECT COALESCE(MAX(position), 0) + 1 FROM plans)) \
         ON CONFLICT (id) DO NOTHING",
    )
    .bind(&id)
    .bind(p.name.trim())
    .bind(p.events_per_month)
    .bind(p.hard_limit)
    .bind(p.max_projects)
    .bind(p.max_members)
    .bind(p.archived)
    .execute(&state.pool)
    .await?;
    if inserted.rows_affected() == 0 {
        return Err(ApiError::conflict(format!("a plan `{id}` already exists")));
    }
    first_plan(&state, &id).await
}

async fn first_plan(state: &AppState, id: &str) -> ApiResult<Json<PlanOut>> {
    load_plans(state, Some(id))
        .await?
        .into_iter()
        .next()
        .map(Json)
        .ok_or_else(|| ApiError::not_found("plan"))
}

/// Replaces a plan's limits. Plans aren't deleted; archive them instead.
#[utoipa::path(put, path = "/api/v1/admin/plans/{id}", tag = "admin", request_body = PlanIn,
    responses((status = 200, body = PlanOut)))]
pub async fn update_plan(
    State(state): State<AppState>,
    _: AdminAuth,
    Path(id): Path<String>,
    Json(p): Json<PlanIn>,
) -> ApiResult<Json<PlanOut>> {
    validate_plan(&p)?;
    let updated = sqlx::query(
        "UPDATE plans SET name = $2, events_per_month = $3, hard_limit = $4, \
         max_projects = $5, max_members = $6, archived = $7 WHERE id = $1",
    )
    .bind(&id)
    .bind(p.name.trim())
    .bind(p.events_per_month)
    .bind(p.hard_limit)
    .bind(p.max_projects)
    .bind(p.max_members)
    .bind(p.archived)
    .execute(&state.pool)
    .await?;
    if updated.rows_affected() == 0 {
        return Err(ApiError::not_found("plan"));
    }
    first_plan(&state, &id).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_tokens() {
        assert!(same("a".repeat(40).as_str(), "a".repeat(40).as_str()));
        assert!(!same("a".repeat(40).as_str(), "b".repeat(40).as_str()));
        assert!(!same("short", "a".repeat(40).as_str()));
    }
}
