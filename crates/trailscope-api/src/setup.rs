//! First-run setup: create the owner, the first project and its ingest key.

use axum::Json;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::IntoResponse;
use axum_extra::extract::CookieJar;
use serde::{Deserialize, Serialize};
use trailscope_core::admin;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::AppState;
use crate::auth::{Role, User, check_password_strength, create_session, hash_password};
use crate::error::{ApiError, ApiResult};
use crate::users::{ClientIp, user_agent, valid_email};

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SetupStatus {
    pub needs_setup: bool,
    pub version: &'static str,
    /// Single sign-on, for the sign-in page.
    pub oidc: crate::users::OidcInfo,
}

/// Whether the install still needs its owner account.
#[utoipa::path(get, path = "/api/v1/setup", tag = "setup", responses((status = 200, body = SetupStatus)))]
pub async fn status(State(state): State<AppState>) -> ApiResult<Json<SetupStatus>> {
    let users: i64 = sqlx::query_scalar("SELECT count(*) FROM users")
        .fetch_one(&state.pool)
        .await?;
    Ok(Json(SetupStatus {
        needs_setup: users == 0,
        version: env!("CARGO_PKG_VERSION"),
        oidc: crate::users::OidcInfo {
            enabled: state.config.oidc.enabled(),
            display_name: state.config.oidc.display_name.clone(),
        },
    }))
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SetupRequest {
    pub email: String,
    #[serde(default)]
    pub name: String,
    pub password: String,
    pub project_name: String,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SetupResponse {
    pub user: User,
    pub project_id: Uuid,
    pub project_slug: String,
    /// The ingest key's plaintext. Shown once; store it now.
    pub ingest_key: String,
    pub csrf_token: String,
}

/// Creates the owner account and the first project. Only works once.
#[utoipa::path(post, path = "/api/v1/setup", tag = "setup", request_body = SetupRequest,
    responses((status = 200, body = SetupResponse), (status = 409, description = "already set up")))]
pub async fn complete(
    State(state): State<AppState>,
    ip: ClientIp,
    headers: HeaderMap,
    jar: CookieJar,
    Json(req): Json<SetupRequest>,
) -> ApiResult<impl IntoResponse> {
    let email = req.email.trim().to_lowercase();
    if !valid_email(&email) {
        return Err(ApiError::bad_request("enter a valid email address"));
    }
    check_password_strength(&req.password)?;
    let project_name = req.project_name.trim();
    if project_name.is_empty() || project_name.chars().count() > 100 {
        return Err(ApiError::bad_request(
            "give the project a name (up to 100 characters)",
        ));
    }
    let hash = hash_password(&req.password)?;

    // Serialize concurrent setup attempts; only the first may create the owner.
    let mut tx = state.pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(7340012)")
        .execute(&mut *tx)
        .await?;
    let users: i64 = sqlx::query_scalar("SELECT count(*) FROM users")
        .fetch_one(&mut *tx)
        .await?;
    if users > 0 {
        return Err(ApiError::conflict(
            "Trailscope is already set up; sign in instead",
        ));
    }
    let user_id = Uuid::now_v7();
    let name = req.name.trim().chars().take(100).collect::<String>();
    sqlx::query(
        "INSERT INTO users (id, email, name, password_hash, role, can_view_bodies) VALUES ($1, $2, $3, $4, 'owner', TRUE)",
    )
    .bind(user_id)
    .bind(&email)
    .bind(&name)
    .bind(&hash)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    let project = admin::create_project(&state.pool, project_name).await?;
    let key = admin::create_ingest_key(&state.pool, project.id, "default", Some(user_id)).await?;
    let session = create_session(&state, user_id, &user_agent(&headers), &ip.to_string()).await?;
    tracing::info!(%email, project = %project.slug, "setup complete");
    Ok((
        jar.add(session.cookie),
        Json(SetupResponse {
            user: User {
                id: user_id,
                email,
                name,
                role: Role::Owner,
                can_view_bodies: true,
            },
            project_id: project.id,
            project_slug: project.slug,
            ingest_key: key.token,
            csrf_token: session.csrf,
        }),
    ))
}
