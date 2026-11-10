//! Sign-in, the current user, members, invites and API tokens.

use std::net::{IpAddr, SocketAddr};

use axum::Json;
use axum::extract::{ConnectInfo, FromRequestParts, Path, State};
use axum::http::request::Parts;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::IntoResponse;
use axum_extra::extract::CookieJar;
use serde::{Deserialize, Serialize};
use trailscope_core::secrets::{display_prefix, generate_token, hash_token};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::AppState;
use crate::auth::{
    Auth, Method_, OrgInfo, Role, SESSION_COOKIE, Scope, User, check_password_strength,
    clear_cookie, create_session, dummy_verify, hash_password, load_caller, new_api_token,
    user_orgs, verify_password,
};
use crate::error::{ApiError, ApiResult};

/// The client's IP: the socket peer, or the first `X-Forwarded-For` hop when
/// `server.trust_proxy` is on.
#[derive(Debug, Clone, Copy)]
pub struct ClientIp(pub Option<IpAddr>);

impl std::fmt::Display for ClientIp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0 {
            Some(ip) => write!(f, "{ip}"),
            None => Ok(()),
        }
    }
}

impl FromRequestParts<AppState> for ClientIp {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        if state.config.server.trust_proxy
            && let Some(ip) = parts
                .headers
                .get("x-forwarded-for")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.split(',').next())
                .and_then(|v| v.trim().parse().ok())
        {
            return Ok(ClientIp(Some(ip)));
        }
        Ok(ClientIp(
            parts
                .extensions
                .get::<ConnectInfo<SocketAddr>>()
                .map(|c| c.0.ip()),
        ))
    }
}

pub fn user_agent(headers: &HeaderMap) -> String {
    headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string()
}

pub fn valid_email(email: &str) -> bool {
    let Some((local, domain)) = email.split_once('@') else {
        return false;
    };
    !local.is_empty()
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && email.len() <= 254
        && !email.chars().any(|c| c.is_whitespace() || c.is_control())
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SessionInfo {
    pub user: User,
    /// The organization this session works in.
    pub org: Option<OrgInfo>,
    pub instance_admin: bool,
    /// Send this as `X-CSRF-Token` on every state-changing request.
    pub csrf_token: String,
}

#[derive(Deserialize, ToSchema)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
}

#[derive(sqlx::FromRow)]
struct LoginRow {
    id: Uuid,
    disabled: bool,
    password_hash: Option<String>,
}

#[utoipa::path(post, path = "/api/v1/auth/login", tag = "auth", request_body = LoginRequest,
    responses((status = 200, body = SessionInfo), (status = 401), (status = 429)))]
pub async fn login(
    State(state): State<AppState>,
    ip: ClientIp,
    headers: HeaderMap,
    jar: CookieJar,
    Json(req): Json<LoginRequest>,
) -> ApiResult<impl IntoResponse> {
    let email = req.email.trim().to_lowercase();
    if let Some(wait) = state.limiter.blocked(&email, ip.0) {
        return Err(ApiError::too_many(format!(
            "too many failed sign-ins; try again in {} minutes",
            wait.div_ceil(60)
        )));
    }
    let row: Option<LoginRow> =
        sqlx::query_as("SELECT id, disabled, password_hash FROM users WHERE lower(email) = $1")
            .bind(&email)
            .fetch_optional(&state.pool)
            .await?;
    let ok = match &row {
        Some(r) => match &r.password_hash {
            Some(h) => verify_password(&req.password, h) && !r.disabled,
            None => false,
        },
        None => {
            dummy_verify(&req.password);
            false
        }
    };
    let Some(r) = row.filter(|_| ok) else {
        state.limiter.record_failure(&email, ip.0);
        return Err(ApiError::new(
            StatusCode::UNAUTHORIZED,
            "invalid_credentials",
            "wrong email or password",
        ));
    };
    state.limiter.reset(&email);
    let session =
        create_session(&state, r.id, None, &user_agent(&headers), &ip.to_string()).await?;
    let (user, org, instance_admin) = load_caller(&state, r.id, None).await?;
    Ok((
        jar.add(session.cookie),
        Json(SessionInfo {
            user,
            org,
            instance_admin,
            csrf_token: session.csrf,
        }),
    ))
}

#[utoipa::path(post, path = "/api/v1/auth/logout", tag = "auth", responses((status = 204)))]
pub async fn logout(
    State(state): State<AppState>,
    auth: Auth,
    jar: CookieJar,
) -> ApiResult<impl IntoResponse> {
    if let Method_::Session { id, .. } = &auth.via {
        sqlx::query("DELETE FROM sessions WHERE id = $1")
            .bind(id)
            .execute(&state.pool)
            .await?;
    }
    Ok((
        jar.remove(clear_cookie(&state)).add(clear_cookie(&state)),
        StatusCode::NO_CONTENT,
    ))
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct MeResponse {
    pub user: User,
    /// The organization requests act in; `None` when the user belongs to none.
    pub org: Option<OrgInfo>,
    /// Every organization the user belongs to, for the switcher.
    pub orgs: Vec<OrgMembership>,
    /// May manage server-wide settings and create organizations.
    pub instance_admin: bool,
    pub csrf_token: Option<String>,
    pub oidc: OidcInfo,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct OrgMembership {
    pub id: Uuid,
    pub slug: String,
    pub name: String,
    pub role: Role,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct OidcInfo {
    pub enabled: bool,
    pub display_name: String,
}

#[utoipa::path(get, path = "/api/v1/auth/me", tag = "auth", responses((status = 200, body = MeResponse), (status = 401)))]
pub async fn me(State(state): State<AppState>, auth: Auth) -> ApiResult<Json<MeResponse>> {
    let mut user = auth.user.clone();
    user.can_view_bodies = auth.org.is_some() && auth.can_view_bodies();
    let orgs = match &auth.via {
        // A token acts in its own organization only.
        Method_::Token { .. } => Vec::new(),
        Method_::Session { .. } => user_orgs(&state, auth.user.id)
            .await?
            .into_iter()
            .map(|(o, role)| OrgMembership {
                id: o.id,
                slug: o.slug,
                name: o.name,
                role,
            })
            .collect(),
    };
    Ok(Json(MeResponse {
        csrf_token: auth.session_csrf().map(str::to_string),
        org: auth.org.clone(),
        orgs,
        instance_admin: auth.instance_admin,
        user,
        oidc: OidcInfo {
            enabled: state.config.oidc.enabled(),
            display_name: state.config.oidc.display_name.clone(),
        },
    }))
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ChangePassword {
    pub current_password: String,
    pub new_password: String,
}

#[utoipa::path(post, path = "/api/v1/auth/password", tag = "auth", request_body = ChangePassword, responses((status = 204)))]
pub async fn change_password(
    State(state): State<AppState>,
    auth: Auth,
    Json(req): Json<ChangePassword>,
) -> ApiResult<StatusCode> {
    let current: Option<String> =
        sqlx::query_scalar("SELECT password_hash FROM users WHERE id = $1")
            .bind(auth.user.id)
            .fetch_one(&state.pool)
            .await?;
    if let Some(h) = current
        && !verify_password(&req.current_password, &h)
    {
        return Err(ApiError::bad_request("the current password is wrong"));
    }
    check_password_strength(&req.new_password)?;
    sqlx::query("UPDATE users SET password_hash = $1 WHERE id = $2")
        .bind(hash_password(&req.new_password)?)
        .bind(auth.user.id)
        .execute(&state.pool)
        .await?;
    // Sign out other sessions.
    let keep = match &auth.via {
        Method_::Session { id, .. } => id.clone(),
        Method_::Token { .. } => String::new(),
    };
    sqlx::query("DELETE FROM sessions WHERE user_id = $1 AND id != $2")
        .bind(auth.user.id)
        .bind(keep)
        .execute(&state.pool)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---- members -------------------------------------------------------------------

#[derive(Serialize, ToSchema, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct Member {
    pub id: Uuid,
    pub email: String,
    pub name: String,
    pub role: String,
    pub can_view_bodies: bool,
    pub disabled: bool,
    #[schema(value_type = String)]
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: time::OffsetDateTime,
    #[schema(value_type = Option<String>)]
    #[serde(with = "time::serde::rfc3339::option")]
    pub last_login_at: Option<time::OffsetDateTime>,
}

#[utoipa::path(get, path = "/api/v1/members", tag = "members", responses((status = 200, body = Vec<Member>)))]
pub async fn list_members(
    State(state): State<AppState>,
    auth: Auth,
) -> ApiResult<Json<Vec<Member>>> {
    auth.require(Role::Viewer, Scope::Read)?;
    let members = sqlx::query_as(
        "SELECT u.id, u.email, u.name, m.role, m.can_view_bodies, m.disabled, m.created_at, u.last_login_at \
         FROM memberships m JOIN users u ON u.id = m.user_id WHERE m.org_id = $1 ORDER BY m.created_at, u.id",
    )
    .bind(auth.org_id()?)
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(members))
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UpdateMember {
    pub role: Option<Role>,
    pub can_view_bodies: Option<bool>,
    /// Suspends the member in this organization only.
    pub disabled: Option<bool>,
    /// Only your own name can be changed (it is shown in every organization).
    pub name: Option<String>,
}

async fn member_role(state: &AppState, org: Uuid, user: Uuid) -> ApiResult<Role> {
    let role: String =
        sqlx::query_scalar("SELECT role FROM memberships WHERE org_id = $1 AND user_id = $2")
            .bind(org)
            .bind(user)
            .fetch_optional(&state.pool)
            .await?
            .ok_or_else(|| ApiError::not_found("member"))?;
    Ok(Role::parse(&role).unwrap_or(Role::Viewer))
}

async fn active_owners(state: &AppState, org: Uuid) -> ApiResult<i64> {
    Ok(sqlx::query_scalar(
        "SELECT count(*) FROM memberships m JOIN users u ON u.id = m.user_id \
         WHERE m.org_id = $1 AND m.role = 'owner' AND NOT m.disabled AND NOT u.disabled",
    )
    .bind(org)
    .fetch_one(&state.pool)
    .await?)
}

#[utoipa::path(patch, path = "/api/v1/members/{id}", tag = "members", request_body = UpdateMember, responses((status = 204)))]
pub async fn update_member(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateMember>,
) -> ApiResult<StatusCode> {
    let org = auth.org_id()?;
    let editing_membership =
        req.role.is_some() || req.can_view_bodies.is_some() || req.disabled.is_some();
    if editing_membership {
        auth.require(Role::Admin, Scope::Admin)?;
    } else {
        auth.require(Role::Viewer, Scope::Write)?;
    }
    let current = member_role(&state, org, id).await?;
    if req.name.is_some() && id != auth.user.id {
        return Err(ApiError::forbidden("you can only change your own name"));
    }
    // Only owners can change owners or make new ones.
    if (current == Role::Owner || req.role == Some(Role::Owner)) && auth.user.role != Role::Owner {
        return Err(ApiError::forbidden("only an owner can change owners"));
    }
    if current == Role::Owner
        && (req.role.is_some_and(|r| r != Role::Owner) || req.disabled == Some(true))
        && active_owners(&state, org).await? <= 1
    {
        return Err(ApiError::conflict(
            "an organization needs at least one active owner",
        ));
    }
    sqlx::query(
        "UPDATE memberships SET role = COALESCE($3, role), can_view_bodies = COALESCE($4, can_view_bodies), \
         disabled = COALESCE($5, disabled) WHERE org_id = $1 AND user_id = $2",
    )
    .bind(org)
    .bind(id)
    .bind(req.role.map(Role::as_str))
    .bind(req.can_view_bodies)
    .bind(req.disabled)
    .execute(&state.pool)
    .await?;
    if let Some(name) = req.name {
        sqlx::query("UPDATE users SET name = $2 WHERE id = $1")
            .bind(id)
            .bind(name.trim().chars().take(100).collect::<String>())
            .execute(&state.pool)
            .await?;
    }
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(delete, path = "/api/v1/members/{id}", tag = "members", responses((status = 204)))]
pub async fn delete_member(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    auth.require(Role::Admin, Scope::Admin)?;
    let org = auth.org_id()?;
    if id == auth.user.id {
        return Err(ApiError::bad_request("you can't remove yourself"));
    }
    if member_role(&state, org, id).await? == Role::Owner {
        if auth.user.role != Role::Owner {
            return Err(ApiError::forbidden("only an owner can remove an owner"));
        }
        if active_owners(&state, org).await? <= 1 {
            return Err(ApiError::conflict(
                "an organization needs at least one active owner",
            ));
        }
    }
    let mut tx = state.pool.begin().await?;
    sqlx::query("DELETE FROM memberships WHERE org_id = $1 AND user_id = $2")
        .bind(org)
        .bind(id)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "UPDATE api_tokens SET revoked_at = now() WHERE org_id = $1 AND user_id = $2 AND revoked_at IS NULL",
    )
    .bind(org)
    .bind(id)
    .execute(&mut *tx)
    .await?;
    // Someone who belongs nowhere any more is removed entirely, as before
    // organizations existed (instance admins are kept).
    sqlx::query(
        "DELETE FROM users WHERE id = $1 AND NOT instance_admin \
         AND NOT EXISTS (SELECT 1 FROM memberships WHERE user_id = $1)",
    )
    .bind(id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---- invites -------------------------------------------------------------------

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreateInvite {
    pub email: String,
    pub role: Role,
    #[serde(default)]
    pub can_view_bodies: bool,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct InviteCreated {
    pub id: String,
    /// The invite link. Shown once.
    pub url: String,
    pub token: String,
}

#[utoipa::path(post, path = "/api/v1/invites", tag = "members", request_body = CreateInvite, responses((status = 200, body = InviteCreated)))]
pub async fn create_invite(
    State(state): State<AppState>,
    auth: Auth,
    Json(req): Json<CreateInvite>,
) -> ApiResult<Json<InviteCreated>> {
    auth.require(Role::Admin, Scope::Admin)?;
    let org = auth.org_id()?;
    let email = req.email.trim().to_lowercase();
    if !valid_email(&email) {
        return Err(ApiError::bad_request("enter a valid email address"));
    }
    if req.role == Role::Owner {
        return Err(ApiError::bad_request(
            "invite as admin, then promote to owner",
        ));
    }
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM memberships m JOIN users u ON u.id = m.user_id \
         WHERE m.org_id = $1 AND lower(u.email) = $2)",
    )
    .bind(org)
    .bind(&email)
    .fetch_one(&state.pool)
    .await?;
    if exists {
        return Err(ApiError::conflict("that person is already a member"));
    }
    let token = generate_token("");
    let hash = hash_token(&token);
    sqlx::query(
        "INSERT INTO invites (token_hash, org_id, email, role, can_view_bodies, created_by, expires_at) \
         VALUES ($1, $2, $3, $4, $5, $6, now() + interval '7 days')",
    )
    .bind(&hash)
    .bind(org)
    .bind(&email)
    .bind(req.role.as_str())
    .bind(req.can_view_bodies)
    .bind(auth.user.id)
    .execute(&state.pool)
    .await?;
    let base = state.config.server.public_url.trim_end_matches('/');
    Ok(Json(InviteCreated {
        id: hash[..16].to_string(),
        url: format!("{base}/invite/{token}"),
        token,
    }))
}

#[derive(Serialize, ToSchema, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct Invite {
    pub id: String,
    pub email: String,
    pub role: String,
    pub can_view_bodies: bool,
    #[schema(value_type = String)]
    #[serde(with = "time::serde::rfc3339")]
    pub expires_at: time::OffsetDateTime,
}

#[utoipa::path(get, path = "/api/v1/invites", tag = "members", responses((status = 200, body = Vec<Invite>)))]
pub async fn list_invites(
    State(state): State<AppState>,
    auth: Auth,
) -> ApiResult<Json<Vec<Invite>>> {
    auth.require(Role::Admin, Scope::Admin)?;
    let rows = sqlx::query_as(
        "SELECT left(token_hash, 16) AS id, email, role, can_view_bodies, expires_at FROM invites \
         WHERE org_id = $1 AND accepted_at IS NULL AND expires_at > now() ORDER BY created_at DESC",
    )
    .bind(auth.org_id()?)
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(rows))
}

#[utoipa::path(delete, path = "/api/v1/invites/{id}", tag = "members", responses((status = 204)))]
pub async fn delete_invite(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    auth.require(Role::Admin, Scope::Admin)?;
    if id.len() != 16 || !id.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(ApiError::not_found("invite"));
    }
    sqlx::query("DELETE FROM invites WHERE org_id = $1 AND left(token_hash, 16) = $2")
        .bind(auth.org_id()?)
        .bind(id)
        .execute(&state.pool)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize, ToSchema, utoipa::IntoParams)]
pub struct InviteToken {
    pub token: String,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct InviteInfo {
    pub email: String,
    pub role: String,
    pub organization: String,
    /// The email already has an account: accepting asks for its password.
    pub existing_account: bool,
}

#[utoipa::path(get, path = "/api/v1/invites/lookup", tag = "members", params(InviteToken),
    responses((status = 200, body = InviteInfo), (status = 404)))]
pub async fn lookup_invite(
    State(state): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<InviteToken>,
) -> ApiResult<Json<InviteInfo>> {
    let row: Option<(String, String, String, bool)> = sqlx::query_as(
        "SELECT i.email, i.role, o.name, EXISTS (SELECT 1 FROM users u WHERE lower(u.email) = lower(i.email)) \
         FROM invites i JOIN organizations o ON o.id = i.org_id \
         WHERE i.token_hash = $1 AND i.accepted_at IS NULL AND i.expires_at > now()",
    )
    .bind(hash_token(q.token.trim()))
    .fetch_optional(&state.pool)
    .await?;
    let (email, role, organization, existing_account) =
        row.ok_or_else(|| ApiError::not_found("invite (it may have expired or been used)"))?;
    Ok(Json(InviteInfo {
        email,
        role,
        organization,
        existing_account,
    }))
}

#[derive(Deserialize, ToSchema)]
pub struct AcceptInvite {
    pub token: String,
    #[serde(default)]
    pub name: String,
    /// A new password, or the existing account's password.
    pub password: String,
}

#[utoipa::path(post, path = "/api/v1/invites/accept", tag = "members", request_body = AcceptInvite, responses((status = 200, body = SessionInfo)))]
pub async fn accept_invite(
    State(state): State<AppState>,
    ip: ClientIp,
    headers: HeaderMap,
    jar: CookieJar,
    Json(req): Json<AcceptInvite>,
) -> ApiResult<impl IntoResponse> {
    let mut tx = state.pool.begin().await?;
    let invite: Option<(String, String, bool, Uuid)> = sqlx::query_as(
        "UPDATE invites SET accepted_at = now() WHERE token_hash = $1 AND accepted_at IS NULL AND expires_at > now() \
         RETURNING email, role, can_view_bodies, org_id",
    )
    .bind(hash_token(req.token.trim()))
    .fetch_optional(&mut *tx)
    .await?;
    let Some((email, role, can_view_bodies, org)) = invite else {
        return Err(ApiError::not_found(
            "invite (it may have expired or been used)",
        ));
    };
    let existing: Option<(Uuid, Option<String>, bool)> =
        sqlx::query_as("SELECT id, password_hash, disabled FROM users WHERE lower(email) = $1")
            .bind(&email)
            .fetch_optional(&mut *tx)
            .await?;
    let user_id = match existing {
        // Someone who already has an account joins with its password (the
        // invite stays unused when it is wrong).
        Some((id, hash, disabled)) => {
            if let Some(wait) = state.limiter.blocked(&email, ip.0) {
                return Err(ApiError::too_many(format!(
                    "too many failed attempts; try again in {} minutes",
                    wait.div_ceil(60)
                )));
            }
            let ok = hash
                .as_deref()
                .is_some_and(|h| verify_password(&req.password, h));
            if !ok || disabled {
                state.limiter.record_failure(&email, ip.0);
                return Err(ApiError::new(
                    StatusCode::UNAUTHORIZED,
                    "invalid_credentials",
                    "that isn't the password of the existing account for this email",
                ));
            }
            state.limiter.reset(&email);
            id
        }
        None => {
            check_password_strength(&req.password)?;
            let id = Uuid::now_v7();
            let name = req.name.trim().chars().take(100).collect::<String>();
            sqlx::query(
                "INSERT INTO users (id, email, name, password_hash) VALUES ($1, $2, $3, $4)",
            )
            .bind(id)
            .bind(&email)
            .bind(&name)
            .bind(hash_password(&req.password)?)
            .execute(&mut *tx)
            .await?;
            id
        }
    };
    sqlx::query(
        "INSERT INTO memberships (org_id, user_id, role, can_view_bodies) VALUES ($1, $2, $3, $4) \
         ON CONFLICT (org_id, user_id) DO UPDATE SET role = excluded.role, \
         can_view_bodies = excluded.can_view_bodies, disabled = FALSE",
    )
    .bind(org)
    .bind(user_id)
    .bind(&role)
    .bind(can_view_bodies)
    .execute(&mut *tx)
    .await?;
    sqlx::query("UPDATE users SET last_org_id = $2 WHERE id = $1")
        .bind(user_id)
        .bind(org)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    let session = create_session(
        &state,
        user_id,
        Some(org),
        &user_agent(&headers),
        &ip.to_string(),
    )
    .await?;
    let (user, org, instance_admin) = load_caller(&state, user_id, Some(org)).await?;
    Ok((
        jar.add(session.cookie),
        Json(SessionInfo {
            user,
            org,
            instance_admin,
            csrf_token: session.csrf,
        }),
    ))
}

// ---- API tokens ------------------------------------------------------------------

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreateToken {
    pub name: String,
    /// Any of `read`, `write`, `admin`.
    pub scopes: Vec<String>,
    pub project_id: Option<Uuid>,
    /// Days until expiry; omit for no expiry.
    pub expires_in_days: Option<u32>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TokenCreated {
    pub id: Uuid,
    /// The token's plaintext. Shown once.
    pub token: String,
    pub prefix: String,
}

#[derive(Serialize, ToSchema, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct ApiToken {
    pub id: Uuid,
    pub name: String,
    pub prefix: String,
    pub scopes: Vec<String>,
    pub project_id: Option<Uuid>,
    #[schema(value_type = String)]
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: time::OffsetDateTime,
    #[schema(value_type = Option<String>)]
    #[serde(with = "time::serde::rfc3339::option")]
    pub last_used_at: Option<time::OffsetDateTime>,
    #[schema(value_type = Option<String>)]
    #[serde(with = "time::serde::rfc3339::option")]
    pub expires_at: Option<time::OffsetDateTime>,
}

#[utoipa::path(get, path = "/api/v1/tokens", tag = "tokens", responses((status = 200, body = Vec<ApiToken>)))]
pub async fn list_tokens(
    State(state): State<AppState>,
    auth: Auth,
) -> ApiResult<Json<Vec<ApiToken>>> {
    let rows = sqlx::query_as(
        "SELECT id, name, prefix, scopes, project_id, created_at, last_used_at, expires_at FROM api_tokens \
         WHERE user_id = $1 AND org_id = $2 AND revoked_at IS NULL ORDER BY created_at DESC",
    )
    .bind(auth.user.id)
    .bind(auth.org_id()?)
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(rows))
}

#[utoipa::path(post, path = "/api/v1/tokens", tag = "tokens", request_body = CreateToken, responses((status = 200, body = TokenCreated)))]
pub async fn create_token(
    State(state): State<AppState>,
    auth: Auth,
    Json(req): Json<CreateToken>,
) -> ApiResult<Json<TokenCreated>> {
    if matches!(auth.via, Method_::Token { .. }) {
        return Err(ApiError::forbidden("API tokens can't create other tokens"));
    }
    auth.require(Role::Viewer, Scope::Read)?;
    let org = auth.org_id()?;
    let name = req.name.trim();
    if name.is_empty() || name.chars().count() > 100 {
        return Err(ApiError::bad_request("give the token a name"));
    }
    if req.scopes.is_empty()
        || req
            .scopes
            .iter()
            .any(|s| !["read", "write", "admin"].contains(&s.as_str()))
    {
        return Err(ApiError::bad_request(
            "scopes must be read, write and/or admin",
        ));
    }
    if req.scopes.iter().any(|s| s == "admin") && auth.user.role < Role::Admin {
        return Err(ApiError::forbidden("only admins can create admin tokens"));
    }
    if let Some(p) = req.project_id {
        auth.check_project(p)?;
    }
    let token = new_api_token();
    let id = Uuid::now_v7();
    let prefix = display_prefix(&token);
    sqlx::query(
        "INSERT INTO api_tokens (id, user_id, project_id, name, token_hash, prefix, scopes, expires_at, org_id) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, CASE WHEN $8::int IS NULL THEN NULL ELSE now() + make_interval(days => $8::int) END, $9)",
    )
    .bind(id)
    .bind(auth.user.id)
    .bind(req.project_id)
    .bind(name)
    .bind(hash_token(&token))
    .bind(&prefix)
    .bind(&req.scopes)
    .bind(req.expires_in_days.map(|d| i32::try_from(d.min(3650)).unwrap_or(3650)))
    .bind(org)
    .execute(&state.pool)
    .await?;
    Ok(Json(TokenCreated { id, token, prefix }))
}

#[utoipa::path(delete, path = "/api/v1/tokens/{id}", tag = "tokens", responses((status = 204)))]
pub async fn revoke_token(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let res = sqlx::query(
        "UPDATE api_tokens SET revoked_at = now() WHERE id = $1 AND org_id = $4 AND (user_id = $2 OR $3)",
    )
    .bind(id)
    .bind(auth.user.id)
    .bind(auth.user.role >= Role::Admin)
    .bind(auth.org_id()?)
    .execute(&state.pool)
    .await?;
    if res.rows_affected() == 0 {
        return Err(ApiError::not_found("token"));
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Used by tests and the CLI.
pub fn session_cookie_name() -> &'static str {
    SESSION_COOKIE
}
