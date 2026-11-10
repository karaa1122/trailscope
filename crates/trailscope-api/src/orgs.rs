//! Organizations (decision 070): list, create, rename and switch.
//!
//! A browser session works in one organization at a time; switching changes
//! the session's organization and is remembered for the next sign-in. API
//! tokens always act in the organization they were created in.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use trailscope_core::admin;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::AppState;
use crate::auth::{Auth, Method_, Role, Scope, user_orgs};
use crate::error::{ApiError, ApiResult};

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct OrgOut {
    pub id: Uuid,
    pub slug: String,
    pub name: String,
    /// Your role in it.
    pub role: Role,
    /// The organization this session works in.
    pub current: bool,
}

fn valid_name(name: &str) -> ApiResult<&str> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 100 {
        return Err(ApiError::bad_request(
            "give the organization a name (up to 100 characters)",
        ));
    }
    Ok(name)
}

#[utoipa::path(get, path = "/api/v1/orgs", tag = "organizations", responses((status = 200, body = Vec<OrgOut>)))]
pub async fn list(State(state): State<AppState>, auth: Auth) -> ApiResult<Json<Vec<OrgOut>>> {
    let current = auth.org.as_ref().map(|o| o.id);
    if let Method_::Token { .. } = auth.via {
        // A token sees only the organization it belongs to.
        return Ok(Json(
            auth.org
                .iter()
                .map(|o| OrgOut {
                    id: o.id,
                    slug: o.slug.clone(),
                    name: o.name.clone(),
                    role: auth.user.role,
                    current: true,
                })
                .collect(),
        ));
    }
    Ok(Json(
        user_orgs(&state, auth.user.id)
            .await?
            .into_iter()
            .map(|(o, role)| OrgOut {
                current: Some(o.id) == current,
                id: o.id,
                slug: o.slug,
                name: o.name,
                role,
            })
            .collect(),
    ))
}

#[derive(Deserialize, ToSchema)]
pub struct CreateOrg {
    pub name: String,
}

#[utoipa::path(post, path = "/api/v1/orgs", tag = "organizations", request_body = CreateOrg,
    responses((status = 200, body = OrgOut)))]
pub async fn create(
    State(state): State<AppState>,
    auth: Auth,
    Json(req): Json<CreateOrg>,
) -> ApiResult<Json<OrgOut>> {
    if matches!(auth.via, Method_::Token { .. }) {
        return Err(ApiError::forbidden("API tokens can't create organizations"));
    }
    // Until sign-up exists (milestone C2), instance admins create
    // organizations and invite their members.
    auth.require_instance_admin()?;
    let name = valid_name(&req.name)?;
    let org = admin::create_org(&state.pool, name).await?;
    admin::add_member(
        &state.pool,
        org.id,
        auth.user.id,
        Role::Owner.as_str(),
        true,
    )
    .await?;
    switch_to(&state, &auth, org.id).await?;
    Ok(Json(OrgOut {
        id: org.id,
        slug: org.slug,
        name: org.name,
        role: Role::Owner,
        current: true,
    }))
}

#[derive(Deserialize, ToSchema)]
pub struct UpdateOrg {
    pub name: String,
}

#[utoipa::path(patch, path = "/api/v1/orgs/current", tag = "organizations", request_body = UpdateOrg,
    responses((status = 204)))]
pub async fn update_current(
    State(state): State<AppState>,
    auth: Auth,
    Json(req): Json<UpdateOrg>,
) -> ApiResult<StatusCode> {
    auth.require(Role::Admin, Scope::Admin)?;
    let name = valid_name(&req.name)?;
    sqlx::query("UPDATE organizations SET name = $2 WHERE id = $1")
        .bind(auth.org_id()?)
        .bind(name)
        .execute(&state.pool)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(post, path = "/api/v1/orgs/{id}/switch", tag = "organizations", responses((status = 204), (status = 404)))]
pub async fn switch(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    if matches!(auth.via, Method_::Token { .. }) {
        return Err(ApiError::forbidden(
            "API tokens always act in their own organization",
        ));
    }
    let member: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM memberships WHERE org_id = $1 AND user_id = $2 AND NOT disabled)",
    )
    .bind(id)
    .bind(auth.user.id)
    .fetch_one(&state.pool)
    .await?;
    if !member {
        return Err(ApiError::not_found("organization"));
    }
    switch_to(&state, &auth, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Points the caller's session (and their next sign-in) at `org`.
pub(crate) async fn switch_to(state: &AppState, auth: &Auth, org: Uuid) -> ApiResult<()> {
    if let Method_::Session { id, .. } = &auth.via {
        sqlx::query("UPDATE sessions SET org_id = $2 WHERE id = $1")
            .bind(id)
            .bind(org)
            .execute(&state.pool)
            .await?;
    }
    sqlx::query("UPDATE users SET last_org_id = $2 WHERE id = $1")
        .bind(auth.user.id)
        .bind(org)
        .execute(&state.pool)
        .await?;
    Ok(())
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProjectUsage {
    pub id: Uuid,
    pub name: String,
    /// Requests, job runs and log lines stored this month; `null` when unknown.
    pub events_this_month: Option<i64>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct OrgUsage {
    pub plan: crate::plans::Plan,
    /// Accepted events this calendar month (UTC).
    pub events_this_month: i64,
    pub projects: Vec<ProjectUsage>,
    pub members: i64,
    pub suspended: bool,
    /// Whether the plan's limits are enforced on this server.
    pub enforced: bool,
}

/// The current organization's plan and this month's usage.
#[utoipa::path(get, path = "/api/v1/orgs/current/usage", tag = "organizations", responses((status = 200, body = OrgUsage)))]
pub async fn usage(State(state): State<AppState>, auth: Auth) -> ApiResult<Json<OrgUsage>> {
    auth.require(Role::Viewer, Scope::Read)?;
    let org = auth.org_id()?;
    let plan = crate::plans::org_plan(&state, org).await?;
    let events_this_month = crate::plans::month_events(&state, org).await?;
    let (members, suspended): (i64, bool) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM memberships WHERE org_id = $1 AND NOT disabled), \
                (SELECT suspended_at IS NOT NULL FROM organizations WHERE id = $1)",
    )
    .bind(org)
    .fetch_one(&state.pool)
    .await?;
    let rows: Vec<(Uuid, String)> =
        sqlx::query_as("SELECT id, name FROM projects WHERE id = ANY($1) ORDER BY created_at")
            .bind(auth.visible_projects())
            .fetch_all(&state.pool)
            .await?;
    let ids: Vec<Uuid> = rows.iter().map(|r| r.0).collect();
    let counts = crate::plans::project_month_events(&state, &ids).await;
    let known = !counts.is_empty();
    Ok(Json(OrgUsage {
        plan,
        events_this_month,
        projects: rows
            .into_iter()
            .map(|(id, name)| ProjectUsage {
                events_this_month: known.then(|| counts.get(&id).copied().unwrap_or(0)),
                id,
                name,
            })
            .collect(),
        members,
        suspended,
        enforced: state.config.cloud.enabled,
    }))
}
