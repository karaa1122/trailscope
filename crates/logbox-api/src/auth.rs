//! Authentication and authorization.
//!
//! - Browser sessions: an opaque token in an HttpOnly, SameSite=Lax cookie
//!   (Secure when served over https). Only its SHA-256 is stored. State-
//!   changing requests must also send the session's CSRF token in
//!   `X-CSRF-Token`.
//! - API tokens: `Authorization: Bearer lbx_api_…` with scopes (`read`,
//!   `write`, `admin`), optionally bound to one project. No CSRF needed.
//! - Roles: owner > admin > member > viewer, plus `can_view_bodies`, which
//!   owners always have.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use argon2::Argon2;
use argon2::password_hash::{PasswordHasher, PasswordVerifier, phc::PasswordHash};
use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::{HeaderMap, Method, header};
use axum_extra::extract::cookie::{Cookie, SameSite};
use logbox_core::secrets::{API_TOKEN_PREFIX, generate_token, hash_token};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::AppState;
use crate::error::{ApiError, ApiResult};

pub const SESSION_COOKIE: &str = "logbox_session";
pub const CSRF_HEADER: &str = "x-csrf-token";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Viewer,
    Member,
    Admin,
    Owner,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Viewer => "viewer",
            Role::Member => "member",
            Role::Admin => "admin",
            Role::Owner => "owner",
        }
    }

    pub fn parse(s: &str) -> Option<Role> {
        match s {
            "viewer" => Some(Role::Viewer),
            "member" => Some(Role::Member),
            "admin" => Some(Role::Admin),
            "owner" => Some(Role::Owner),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Read,
    Write,
    Admin,
}

impl Scope {
    fn name(self) -> &'static str {
        match self {
            Scope::Read => "read",
            Scope::Write => "write",
            Scope::Admin => "admin",
        }
    }
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct User {
    pub id: Uuid,
    pub email: String,
    pub name: String,
    pub role: Role,
    pub can_view_bodies: bool,
}

#[derive(Debug, Clone)]
pub enum Method_ {
    Session { id: String, csrf: String },
    Token { scopes: Vec<String>, project: Option<Uuid> },
}

/// An authenticated caller.
#[derive(Debug, Clone)]
pub struct Auth {
    pub user: User,
    pub via: Method_,
}

impl Auth {
    /// Requires at least `role`, and for API tokens the matching scope.
    pub fn require(&self, role: Role, scope: Scope) -> ApiResult<()> {
        if self.user.role < role {
            return Err(ApiError::forbidden(format!(
                "this needs the {} role or higher",
                role.as_str()
            )));
        }
        if let Method_::Token { scopes, .. } = &self.via {
            let ok = scopes.iter().any(|s| s == scope.name() || s == "admin" || (s == "write" && scope == Scope::Read));
            if !ok {
                return Err(ApiError::forbidden(format!("this API token lacks the `{}` scope", scope.name())));
            }
        }
        Ok(())
    }

    pub fn can_view_bodies(&self) -> bool {
        self.user.role == Role::Owner || self.user.can_view_bodies
    }

    /// API tokens bound to a project can only see that project.
    pub fn check_project(&self, project: Uuid) -> ApiResult<()> {
        if let Method_::Token { project: Some(p), .. } = &self.via
            && *p != project
        {
            return Err(ApiError::forbidden("this API token is limited to another project"));
        }
        Ok(())
    }

    pub fn session_csrf(&self) -> Option<&str> {
        match &self.via {
            Method_::Session { csrf, .. } => Some(csrf),
            Method_::Token { .. } => None,
        }
    }
}

#[derive(sqlx::FromRow)]
struct UserRow {
    id: Uuid,
    email: String,
    name: String,
    role: String,
    can_view_bodies: bool,
    disabled: bool,
}

impl UserRow {
    fn into_user(self) -> Option<User> {
        if self.disabled {
            return None;
        }
        Some(User {
            id: self.id,
            email: self.email,
            name: self.name,
            role: Role::parse(&self.role)?,
            can_view_bodies: self.can_view_bodies,
        })
    }
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|c| Cookie::parse(c.trim().to_string()).ok())
        .find(|c| c.name() == name)
        .map(|c| c.value().to_string())
}

impl FromRequestParts<AppState> for Auth {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        if let Some(token) = parts
            .headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
        {
            return token_auth(state, token.trim()).await;
        }
        let Some(raw) = cookie_value(&parts.headers, SESSION_COOKIE) else {
            return Err(ApiError::unauthorized());
        };
        let row: Option<(String, String, UserRow)> = sqlx::query_as::<_, (String, String, Uuid, String, String, String, bool, bool)>(
            "SELECT s.id, s.csrf_token, u.id, u.email, u.name, u.role, u.can_view_bodies, u.disabled \
             FROM sessions s JOIN users u ON u.id = s.user_id \
             WHERE s.id = $1 AND s.expires_at > now()",
        )
        .bind(hash_token(&raw))
        .fetch_optional(&state.pool)
        .await?
        .map(|(sid, csrf, id, email, name, role, can_view_bodies, disabled)| {
            (sid, csrf, UserRow { id, email, name, role, can_view_bodies, disabled })
        });
        let Some((sid, csrf, user)) = row else {
            return Err(ApiError::unauthorized());
        };
        let Some(user) = user.into_user() else {
            return Err(ApiError::unauthorized());
        };
        if !matches!(parts.method, Method::GET | Method::HEAD | Method::OPTIONS) {
            let sent = parts.headers.get(CSRF_HEADER).and_then(|v| v.to_str().ok()).unwrap_or("");
            if !constant_time_eq(sent.as_bytes(), csrf.as_bytes()) {
                return Err(ApiError::forbidden("missing or invalid CSRF token"));
            }
        }
        Ok(Auth { user, via: Method_::Session { id: sid, csrf } })
    }
}

async fn token_auth(state: &AppState, token: &str) -> ApiResult<Auth> {
    if !token.starts_with(API_TOKEN_PREFIX) {
        return Err(ApiError::unauthorized());
    }
    let row = sqlx::query_as::<_, (Uuid, Vec<String>, Option<Uuid>, Uuid, String, String, String, bool, bool)>(
        "SELECT t.id, t.scopes, t.project_id, u.id, u.email, u.name, u.role, u.can_view_bodies, u.disabled \
         FROM api_tokens t JOIN users u ON u.id = t.user_id \
         WHERE t.token_hash = $1 AND t.revoked_at IS NULL AND (t.expires_at IS NULL OR t.expires_at > now())",
    )
    .bind(hash_token(token))
    .fetch_optional(&state.pool)
    .await?;
    let Some((token_id, scopes, project, id, email, name, role, can_view_bodies, disabled)) = row else {
        return Err(ApiError::unauthorized());
    };
    let user = UserRow { id, email, name, role, can_view_bodies, disabled }
        .into_user()
        .ok_or_else(ApiError::unauthorized)?;
    let pool = state.pool.clone();
    tokio::spawn(async move {
        let _ = sqlx::query("UPDATE api_tokens SET last_used_at = now() WHERE id = $1")
            .bind(token_id)
            .execute(&pool)
            .await;
    });
    Ok(Auth { user, via: Method_::Token { scopes, project } })
}

// ---- passwords ----------------------------------------------------------------

pub const MIN_PASSWORD_LEN: usize = 10;

pub fn hash_password(password: &str) -> ApiResult<String> {
    Argon2::default()
        .hash_password(password.as_bytes())
        .map(|h| h.to_string())
        .map_err(ApiError::internal)
}

pub fn verify_password(password: &str, phc: &str) -> bool {
    PasswordHash::new(phc)
        .map(|h| Argon2::default().verify_password(password.as_bytes(), &h).is_ok())
        .unwrap_or(false)
}

/// A fixed hash to verify against when the user doesn't exist, so login takes
/// the same time either way.
pub fn dummy_verify(password: &str) {
    static DUMMY: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    let hash = DUMMY.get_or_init(|| hash_password("logbox-timing-dummy").unwrap_or_default());
    let _ = verify_password(password, hash);
}

pub fn check_password_strength(password: &str) -> ApiResult<()> {
    if password.chars().count() < MIN_PASSWORD_LEN {
        return Err(ApiError::bad_request(format!(
            "passwords need at least {MIN_PASSWORD_LEN} characters"
        )));
    }
    if password.len() > 1024 {
        return Err(ApiError::bad_request("that password is too long"));
    }
    Ok(())
}

// ---- sessions ------------------------------------------------------------------

pub struct NewSession {
    pub cookie: Cookie<'static>,
    pub csrf: String,
}

pub async fn create_session(state: &AppState, user_id: Uuid, user_agent: &str, ip: &str) -> ApiResult<NewSession> {
    let token = generate_token("");
    let csrf = generate_token("");
    let ttl = state.config.server.session_ttl_hours;
    sqlx::query(
        "INSERT INTO sessions (id, user_id, csrf_token, user_agent, ip, expires_at) \
         VALUES ($1, $2, $3, $4, $5, now() + make_interval(hours => $6))",
    )
    .bind(hash_token(&token))
    .bind(user_id)
    .bind(&csrf)
    .bind(user_agent.chars().take(512).collect::<String>())
    .bind(ip)
    .bind(i32::try_from(ttl).unwrap_or(i32::MAX))
    .execute(&state.pool)
    .await?;
    sqlx::query("UPDATE users SET last_login_at = now() WHERE id = $1")
        .bind(user_id)
        .execute(&state.pool)
        .await?;
    // Opportunistic cleanup of expired sessions.
    let _ = sqlx::query("DELETE FROM sessions WHERE expires_at < now()")
        .execute(&state.pool)
        .await;
    Ok(NewSession {
        cookie: session_cookie(state, token, Duration::from_secs(ttl * 3600)),
        csrf,
    })
}

pub fn session_cookie(state: &AppState, value: String, max_age: Duration) -> Cookie<'static> {
    Cookie::build((SESSION_COOKIE, value))
        .path("/")
        .http_only(true)
        .same_site(SameSite::Lax)
        .secure(state.config.server.cookies_secure())
        .max_age(time::Duration::seconds(i64::try_from(max_age.as_secs()).unwrap_or(i64::MAX)))
        .build()
}

pub fn clear_cookie(state: &AppState) -> Cookie<'static> {
    session_cookie(state, String::new(), Duration::ZERO)
}

pub fn new_api_token() -> String {
    generate_token(API_TOKEN_PREFIX)
}

// ---- login rate limiting -----------------------------------------------------------

/// Sliding-window limits on failed logins, per email and per client IP.
pub struct LoginLimiter {
    failures: Mutex<HashMap<String, Vec<Instant>>>,
}

const WINDOW: Duration = Duration::from_secs(15 * 60);
const PER_EMAIL: usize = 8;
const PER_IP: usize = 30;

impl Default for LoginLimiter {
    fn default() -> Self {
        Self {
            failures: Mutex::new(HashMap::new()),
        }
    }
}

impl LoginLimiter {
    fn keys(email: &str, ip: Option<IpAddr>) -> [(String, usize); 2] {
        [
            (format!("e:{}", email.to_lowercase()), PER_EMAIL),
            (format!("i:{}", ip.map(|i| i.to_string()).unwrap_or_default()), PER_IP),
        ]
    }

    /// Seconds until another attempt is allowed, if currently blocked.
    pub fn blocked(&self, email: &str, ip: Option<IpAddr>) -> Option<u64> {
        let Ok(mut map) = self.failures.lock() else { return None };
        let now = Instant::now();
        let mut wait = None;
        for (key, limit) in Self::keys(email, ip) {
            if let Some(times) = map.get_mut(&key) {
                times.retain(|t| now.duration_since(*t) < WINDOW);
                if times.len() >= limit
                    && let Some(oldest) = times.first()
                {
                    let secs = (WINDOW - now.duration_since(*oldest)).as_secs().max(1);
                    wait = Some(wait.map_or(secs, |w: u64| w.max(secs)));
                }
            }
        }
        wait
    }

    pub fn record_failure(&self, email: &str, ip: Option<IpAddr>) {
        if let Ok(mut map) = self.failures.lock() {
            if map.len() > 100_000 {
                map.clear();
            }
            for (key, _) in Self::keys(email, ip) {
                map.entry(key).or_default().push(Instant::now());
            }
        }
    }

    pub fn reset(&self, email: &str) {
        if let Ok(mut map) = self.failures.lock() {
            map.remove(&format!("e:{}", email.to_lowercase()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passwords_hash_and_verify() {
        let h = hash_password("correct horse battery").unwrap();
        assert!(h.starts_with("$argon2id$"));
        assert!(verify_password("correct horse battery", &h));
        assert!(!verify_password("wrong", &h));
        assert!(!verify_password("x", "not a hash"));
    }

    #[test]
    fn limiter_blocks_after_repeated_failures() {
        let l = LoginLimiter::default();
        let ip: Option<IpAddr> = "203.0.113.1".parse().ok();
        for _ in 0..PER_EMAIL {
            assert!(l.blocked("a@example.com", ip).is_none());
            l.record_failure("A@example.com", ip);
        }
        assert!(l.blocked("a@example.com", ip).is_some());
        assert!(l.blocked("b@example.com", "203.0.113.2".parse().ok()).is_none());
        l.reset("a@example.com");
        assert!(l.blocked("a@example.com", "198.51.100.1".parse().ok()).is_none());
    }

    #[test]
    fn roles_are_ordered() {
        assert!(Role::Owner > Role::Admin && Role::Admin > Role::Member && Role::Member > Role::Viewer);
        assert_eq!(Role::parse("admin"), Some(Role::Admin));
    }

    #[test]
    fn csrf_compare() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
    }
}
