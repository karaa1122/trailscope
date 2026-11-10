//! The admin console (decision 071): every organization on the server, their
//! plans, usage and invoices. Only signed-in instance admins can use it; API
//! tokens can't, since they act inside one organization.

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use time::macros::format_description;
use trailscope_core::admin;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::AppState;
use crate::auth::{Auth, Method_, Role};
use crate::error::{ApiError, ApiResult};
use crate::plans::{MONTH_START, PLAN_COLUMNS, Plan, project_month_events};
use crate::users::{InviteCreated, issue_invite};

fn require_console(auth: &Auth) -> ApiResult<()> {
    if matches!(auth.via, Method_::Token { .. }) {
        return Err(ApiError::forbidden(
            "the admin console needs a signed-in instance admin; API tokens can't use it",
        ));
    }
    if !auth.instance_admin {
        return Err(ApiError::forbidden(
            "only instance admins can open the admin console",
        ));
    }
    Ok(())
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
    /// Only instance admins see it.
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
    pub open_invoices: i64,
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
                      ORDER BY m.created_at) AS owners, \
                (SELECT count(*) FROM invoices i WHERE i.org_id = o.id AND i.status = 'open') AS open_invoices \
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
    auth: Auth,
) -> ApiResult<Json<Vec<AdminOrg>>> {
    require_console(&auth)?;
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

/// Creates an organization without making you a member, and optionally the
/// owner invite link to send its first owner.
#[utoipa::path(post, path = "/api/v1/admin/orgs", tag = "admin", request_body = CreateOrg,
    responses((status = 200, body = OrgCreated)))]
pub async fn create_org(
    State(state): State<AppState>,
    auth: Auth,
    Json(req): Json<CreateOrg>,
) -> ApiResult<Json<OrgCreated>> {
    require_console(&auth)?;
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
        Some(email) => {
            match issue_invite(&state, org.id, email, Role::Owner, true, auth.user.id).await {
                Ok(i) => Some(i),
                Err(e) => {
                    // Don't leave an organization nobody can get into.
                    sqlx::query("DELETE FROM organizations WHERE id = $1")
                        .bind(org.id)
                        .execute(&state.pool)
                        .await?;
                    return Err(e);
                }
            }
        }
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
    pub invoices: Vec<InvoiceOut>,
}

#[utoipa::path(get, path = "/api/v1/admin/orgs/{id}", tag = "admin", responses((status = 200, body = AdminOrgDetail)))]
pub async fn get_org(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<AdminOrgDetail>> {
    require_console(&auth)?;
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
    if !counts.is_empty() || ids.is_empty() {
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
    let invoices = list_invoices_for(&state, Some(id), None).await?;
    Ok(Json(AdminOrgDetail {
        org,
        members,
        projects,
        invites,
        usage,
        invoices,
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
    auth: Auth,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateOrg>,
) -> ApiResult<Json<AdminOrg>> {
    require_console(&auth)?;
    let current = load_org(&state, id).await?;
    let mut tx = state.pool.begin().await?;
    if let Some(name) = &req.name {
        sqlx::query("UPDATE organizations SET name = $2 WHERE id = $1")
            .bind(id)
            .bind(valid_org_name(name)?)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(plan) = &req.plan_id {
        assignable_plan(&state, plan, Some(&current.plan_id)).await?;
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
    auth: Auth,
    Path(id): Path<Uuid>,
    Json(req): Json<AdminInviteReq>,
) -> ApiResult<Json<InviteCreated>> {
    require_console(&auth)?;
    load_org(&state, id).await?;
    Ok(Json(
        issue_invite(
            &state,
            id,
            &req.email,
            req.role,
            req.can_view_bodies,
            auth.user.id,
        )
        .await?,
    ))
}

/// Makes you an owner of the organization and switches your session to it
/// (to help a customer from inside their account).
#[utoipa::path(post, path = "/api/v1/admin/orgs/{id}/join", tag = "admin", responses((status = 204)))]
pub async fn join(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    require_console(&auth)?;
    load_org(&state, id).await?;
    admin::add_member(&state.pool, id, auth.user.id, Role::Owner.as_str(), true).await?;
    crate::orgs::switch_to(&state, &auth, id).await?;
    Ok(StatusCode::NO_CONTENT)
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
    auth: Auth,
) -> ApiResult<Json<Vec<PlanOut>>> {
    require_console(&auth)?;
    Ok(Json(load_plans(&state, None).await?))
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PlanIn {
    pub name: String,
    #[serde(default)]
    pub price_iqd: i64,
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
    if p.price_iqd < 0 || p.events_per_month.is_some_and(|n| n < 0) {
        return Err(ApiError::bad_request("prices and limits can't be negative"));
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
    auth: Auth,
    Json(req): Json<CreatePlan>,
) -> ApiResult<Json<PlanOut>> {
    require_console(&auth)?;
    let id = req.id.trim().to_string();
    if !valid_plan_id(&id) {
        return Err(ApiError::bad_request(
            "the plan ID uses lowercase letters, digits and dashes (up to 40)",
        ));
    }
    validate_plan(&req.plan)?;
    let p = &req.plan;
    let inserted = sqlx::query(
        "INSERT INTO plans (id, name, price_iqd, events_per_month, hard_limit, max_projects, max_members, archived, position) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, (SELECT COALESCE(MAX(position), 0) + 1 FROM plans)) \
         ON CONFLICT (id) DO NOTHING",
    )
    .bind(&id)
    .bind(p.name.trim())
    .bind(p.price_iqd)
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

/// Replaces a plan's settings. Plans aren't deleted; archive them instead.
#[utoipa::path(put, path = "/api/v1/admin/plans/{id}", tag = "admin", request_body = PlanIn,
    responses((status = 200, body = PlanOut)))]
pub async fn update_plan(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
    Json(p): Json<PlanIn>,
) -> ApiResult<Json<PlanOut>> {
    require_console(&auth)?;
    validate_plan(&p)?;
    let updated = sqlx::query(
        "UPDATE plans SET name = $2, price_iqd = $3, events_per_month = $4, hard_limit = $5, \
         max_projects = $6, max_members = $7, archived = $8 WHERE id = $1",
    )
    .bind(&id)
    .bind(p.name.trim())
    .bind(p.price_iqd)
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

// ---- invoices ----------------------------------------------------------------

#[derive(Serialize, ToSchema, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct InvoiceOut {
    pub id: Uuid,
    pub org_id: Uuid,
    pub org_name: String,
    pub plan_id: String,
    pub plan_name: String,
    /// `YYYY-MM-DD`.
    pub period_start: String,
    /// `YYYY-MM-DD`, inclusive.
    pub period_end: String,
    pub amount_iqd: i64,
    /// `open`, `paid` or `void`.
    pub status: String,
    pub note: String,
    #[schema(value_type = Option<String>)]
    #[serde(with = "time::serde::rfc3339::option")]
    pub paid_at: Option<OffsetDateTime>,
    #[schema(value_type = String)]
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

const INVOICE_SQL: &str = "SELECT i.id, i.org_id, o.name AS org_name, i.plan_id, p.name AS plan_name, \
       to_char(i.period_start, 'YYYY-MM-DD') AS period_start, \
       to_char(i.period_end, 'YYYY-MM-DD') AS period_end, \
       i.amount_iqd, i.status, i.note, i.paid_at, i.created_at \
     FROM invoices i JOIN organizations o ON o.id = i.org_id JOIN plans p ON p.id = i.plan_id \
     WHERE ($1::uuid IS NULL OR i.org_id = $1) AND ($2::text IS NULL OR i.status = $2) \
       AND ($3::uuid IS NULL OR i.id = $3) \
     ORDER BY i.created_at DESC LIMIT 500";

async fn list_invoices_for(
    state: &AppState,
    org: Option<Uuid>,
    status: Option<&str>,
) -> ApiResult<Vec<InvoiceOut>> {
    Ok(sqlx::query_as(INVOICE_SQL)
        .bind(org)
        .bind(status)
        .bind(None::<Uuid>)
        .fetch_all(&state.pool)
        .await?)
}

async fn load_invoice(state: &AppState, id: Uuid) -> ApiResult<InvoiceOut> {
    sqlx::query_as(INVOICE_SQL)
        .bind(None::<Uuid>)
        .bind(None::<String>)
        .bind(id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| ApiError::not_found("invoice"))
}

#[derive(Deserialize, utoipa::IntoParams)]
pub struct InvoiceQuery {
    pub org: Option<Uuid>,
    /// `open`, `paid` or `void`.
    pub status: Option<String>,
}

#[utoipa::path(get, path = "/api/v1/admin/invoices", tag = "admin", params(InvoiceQuery),
    responses((status = 200, body = Vec<InvoiceOut>)))]
pub async fn list_invoices(
    State(state): State<AppState>,
    auth: Auth,
    Query(q): Query<InvoiceQuery>,
) -> ApiResult<Json<Vec<InvoiceOut>>> {
    require_console(&auth)?;
    Ok(Json(
        list_invoices_for(&state, q.org, q.status.as_deref().filter(|s| !s.is_empty())).await?,
    ))
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreateInvoice {
    pub org_id: Uuid,
    pub plan_id: String,
    /// `YYYY-MM-DD`.
    pub period_start: String,
    /// `YYYY-MM-DD`, inclusive.
    pub period_end: String,
    pub amount_iqd: i64,
    #[serde(default)]
    pub note: String,
}

fn parse_day(s: &str, what: &str) -> ApiResult<time::Date> {
    time::Date::parse(s.trim(), format_description!("[year]-[month]-[day]"))
        .map_err(|_| ApiError::bad_request(format!("{what} must be a date like 2026-12-01")))
}

/// Records what an organization owes for a period (paid by FIB transfer and
/// marked paid by hand until FIB payments are wired in).
#[utoipa::path(post, path = "/api/v1/admin/invoices", tag = "admin", request_body = CreateInvoice,
    responses((status = 200, body = InvoiceOut)))]
pub async fn create_invoice(
    State(state): State<AppState>,
    auth: Auth,
    Json(req): Json<CreateInvoice>,
) -> ApiResult<Json<InvoiceOut>> {
    require_console(&auth)?;
    load_org(&state, req.org_id).await?;
    let start = parse_day(&req.period_start, "the period start")?;
    let end = parse_day(&req.period_end, "the period end")?;
    if end < start {
        return Err(ApiError::bad_request("the period ends before it starts"));
    }
    if req.amount_iqd < 0 {
        return Err(ApiError::bad_request("the amount can't be negative"));
    }
    let plan_exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM plans WHERE id = $1)")
        .bind(&req.plan_id)
        .fetch_one(&state.pool)
        .await?;
    if !plan_exists {
        return Err(ApiError::bad_request(format!(
            "there is no plan `{}`",
            req.plan_id
        )));
    }
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO invoices (id, org_id, plan_id, period_start, period_end, amount_iqd, note, created_by) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .bind(id)
    .bind(req.org_id)
    .bind(&req.plan_id)
    .bind(start)
    .bind(end)
    .bind(req.amount_iqd)
    .bind(req.note.trim().chars().take(1000).collect::<String>())
    .bind(auth.user.id)
    .execute(&state.pool)
    .await?;
    Ok(Json(load_invoice(&state, id).await?))
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInvoice {
    /// `open`, `paid` or `void`.
    pub status: Option<String>,
    pub note: Option<String>,
    /// When marking it paid: also move the organization to the invoice's plan.
    #[serde(default)]
    pub apply_plan: bool,
}

#[utoipa::path(patch, path = "/api/v1/admin/invoices/{id}", tag = "admin", request_body = UpdateInvoice,
    responses((status = 200, body = InvoiceOut)))]
pub async fn update_invoice(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateInvoice>,
) -> ApiResult<Json<InvoiceOut>> {
    require_console(&auth)?;
    let inv = load_invoice(&state, id).await?;
    let mut tx = state.pool.begin().await?;
    if let Some(status) = req.status.as_deref() {
        if !matches!(status, "open" | "paid" | "void") {
            return Err(ApiError::bad_request("status is open, paid or void"));
        }
        sqlx::query(
            "UPDATE invoices SET status = $2, \
             paid_at = CASE WHEN $2 = 'paid' THEN COALESCE(paid_at, now()) END WHERE id = $1",
        )
        .bind(id)
        .bind(status)
        .execute(&mut *tx)
        .await?;
        if status == "paid" && req.apply_plan {
            sqlx::query("UPDATE organizations SET plan_id = $2 WHERE id = $1")
                .bind(inv.org_id)
                .bind(&inv.plan_id)
                .execute(&mut *tx)
                .await?;
        }
    }
    if let Some(note) = &req.note {
        sqlx::query("UPDATE invoices SET note = $2 WHERE id = $1")
            .bind(id)
            .bind(note.trim().chars().take(1000).collect::<String>())
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(Json(load_invoice(&state, id).await?))
}
