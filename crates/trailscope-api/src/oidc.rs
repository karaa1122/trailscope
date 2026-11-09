//! Optional single sign-on with OpenID Connect: the authorization code flow
//! with PKCE. Users are matched by their OIDC subject, then by verified email;
//! unknown users from `oidc.auto_create_domains` get a viewer account.

use std::time::{Duration, Instant};

use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Redirect, Response};
use axum_extra::extract::CookieJar;
use axum_extra::extract::cookie::{Cookie, SameSite};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use jsonwebtoken::jwk::JwkSet;
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::RwLock;
use trailscope_core::config::OidcConfig;
use trailscope_core::secrets::generate_token;
use uuid::Uuid;

use crate::AppState;
use crate::auth::create_session;
use crate::error::ApiError;
use crate::users::{ClientIp, user_agent};

const COOKIE: &str = "trailscope_oidc";
const COOKIE_PATH: &str = "/api/v1/auth/oidc";
/// How long the browser has to come back from the identity provider.
const PENDING_SECS: i64 = 600;
/// Discovery and keys are refreshed after this long, or when a token uses an unknown key.
const CACHE_TTL: Duration = Duration::from_secs(3600);

#[derive(Clone, Deserialize)]
struct Provider {
    issuer: String,
    authorization_endpoint: String,
    token_endpoint: String,
    jwks_uri: String,
    #[serde(default)]
    userinfo_endpoint: Option<String>,
    #[serde(default)]
    token_endpoint_auth_methods_supported: Vec<String>,
}

#[derive(Clone)]
struct Discovered {
    provider: Provider,
    jwks: JwkSet,
    fetched: Instant,
}

/// The discovery document and signing keys, cached.
pub struct Oidc {
    http: reqwest::Client,
    cache: RwLock<Option<Discovered>>,
}

impl Default for Oidc {
    fn default() -> Self {
        Self {
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .redirect(reqwest::redirect::Policy::none())
                .user_agent(concat!("Trailscope/", env!("CARGO_PKG_VERSION")))
                .build()
                .unwrap_or_default(),
            cache: RwLock::new(None),
        }
    }
}

impl Oidc {
    async fn discovered(&self, cfg: &OidcConfig, refresh: bool) -> Result<Discovered, String> {
        if !refresh
            && let Some(d) = self.cache.read().await.as_ref()
            && d.fetched.elapsed() < CACHE_TTL
        {
            return Ok(d.clone());
        }
        let url = format!(
            "{}/.well-known/openid-configuration",
            cfg.issuer.trim_end_matches('/')
        );
        let provider: Provider = self.get_json(&url).await?;
        if provider.issuer.trim_end_matches('/') != cfg.issuer.trim_end_matches('/') {
            return Err(format!(
                "the provider reports issuer {}, not {}",
                provider.issuer, cfg.issuer
            ));
        }
        let jwks: JwkSet = self.get_json(&provider.jwks_uri).await?;
        let d = Discovered {
            provider,
            jwks,
            fetched: Instant::now(),
        };
        *self.cache.write().await = Some(d.clone());
        Ok(d)
    }

    async fn get_json<T: serde::de::DeserializeOwned>(&self, url: &str) -> Result<T, String> {
        let resp = self
            .http
            .get(url)
            .send()
            .await
            .map_err(|e| format!("could not reach {url}: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("{url} answered HTTP {}", resp.status().as_u16()));
        }
        resp.json()
            .await
            .map_err(|e| format!("{url} returned invalid JSON: {e}"))
    }
}

/// State kept in a signed cookie between `start` and `callback`.
#[derive(Serialize, Deserialize)]
struct Pending {
    /// `state` parameter.
    s: String,
    nonce: String,
    /// PKCE code verifier.
    v: String,
    /// Where to go after signing in.
    next: String,
    /// Expiry, unix seconds.
    exp: i64,
}

fn now_secs() -> i64 {
    time::OffsetDateTime::now_utc().unix_timestamp()
}

fn redirect_uri(state: &AppState) -> String {
    format!(
        "{}/api/v1/auth/oidc/callback",
        state.config.server.public_url.trim_end_matches('/')
    )
}

/// Only same-site paths, so the login can't be turned into an open redirect.
fn safe_next(next: Option<&str>) -> String {
    match next {
        Some(n) if n.starts_with('/') && !n.starts_with("//") && !n.contains('\\') => n.to_string(),
        _ => "/".to_string(),
    }
}

fn pending_cookie(state: &AppState, value: String, max_age: i64) -> Cookie<'static> {
    Cookie::build((COOKIE, value))
        .path(COOKIE_PATH)
        .http_only(true)
        // Lax: the cookie must come back on the provider's top-level redirect.
        .same_site(SameSite::Lax)
        .secure(state.config.server.cookies_secure())
        .max_age(time::Duration::seconds(max_age))
        .build()
}

/// Back to the sign-in page with a message.
fn fail(jar: CookieJar, state: &AppState, message: &str) -> Response {
    tracing::warn!(error = %message, "OIDC sign-in failed");
    let encoded: String = url::form_urlencoded::byte_serialize(message.as_bytes()).collect();
    (
        jar.add(pending_cookie(state, String::new(), 0)),
        Redirect::to(&format!("/login?error={encoded}")),
    )
        .into_response()
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct StartQuery {
    /// Path to open after signing in.
    pub next: Option<String>,
}

/// Starts single sign-on: redirects to the identity provider.
#[utoipa::path(get, path = "/api/v1/auth/oidc/start", tag = "auth", params(StartQuery),
    responses((status = 303, description = "Redirect to the identity provider"), (status = 404, description = "OIDC is not configured")))]
pub async fn start(
    State(state): State<AppState>,
    jar: CookieJar,
    Query(q): Query<StartQuery>,
) -> Response {
    let cfg = &state.config.oidc;
    if !cfg.enabled() {
        return ApiError::not_found("single sign-on").into_response();
    }
    let d = match state.oidc.discovered(cfg, false).await {
        Ok(d) => d,
        Err(e) => return fail(jar, &state, &format!("single sign-on is unavailable: {e}")),
    };
    let pending = Pending {
        s: generate_token(""),
        nonce: generate_token(""),
        v: format!("{}{}", generate_token(""), generate_token("")),
        next: safe_next(q.next.as_deref()),
        exp: now_secs() + PENDING_SECS,
    };
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(pending.v.as_bytes()));
    let Ok(mut url) = url::Url::parse(&d.provider.authorization_endpoint) else {
        return fail(
            jar,
            &state,
            "the identity provider's authorization endpoint is invalid",
        );
    };
    url.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", &cfg.client_id)
        .append_pair("redirect_uri", &redirect_uri(&state))
        .append_pair("scope", "openid email profile")
        .append_pair("state", &pending.s)
        .append_pair("nonce", &pending.nonce)
        .append_pair("code_challenge", &challenge)
        .append_pair("code_challenge_method", "S256");
    let token = state
        .secrets
        .sign_token(&serde_json::to_vec(&pending).unwrap_or_default());
    (
        jar.add(pending_cookie(&state, token, PENDING_SECS)),
        Redirect::to(url.as_str()),
    )
        .into_response()
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct CallbackQuery {
    pub code: Option<String>,
    pub state: Option<String>,
    pub error: Option<String>,
    pub error_description: Option<String>,
}

#[derive(Deserialize)]
struct TokenResponse {
    id_token: Option<String>,
    access_token: Option<String>,
}

#[derive(Deserialize)]
struct Claims {
    sub: String,
    #[serde(default)]
    email: Option<String>,
    /// A boolean, or the string `"true"` for some providers.
    #[serde(default)]
    email_verified: Option<serde_json::Value>,
    #[serde(default)]
    nonce: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    preferred_username: Option<String>,
}

#[derive(Deserialize)]
struct UserInfo {
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    email_verified: Option<serde_json::Value>,
}

/// `false` only when the provider says the address is unverified.
fn not_unverified(v: Option<&serde_json::Value>) -> bool {
    match v {
        None => true,
        Some(serde_json::Value::Bool(b)) => *b,
        Some(serde_json::Value::String(s)) => s.eq_ignore_ascii_case("true"),
        Some(_) => false,
    }
}

/// The identity provider sends the browser back here.
#[utoipa::path(get, path = "/api/v1/auth/oidc/callback", tag = "auth", params(CallbackQuery),
    responses((status = 303, description = "Signed in; redirect into the app, or to /login with an error")))]
pub async fn callback(
    State(state): State<AppState>,
    jar: CookieJar,
    ip: ClientIp,
    headers: HeaderMap,
    Query(q): Query<CallbackQuery>,
) -> Response {
    let cfg = state.config.oidc.clone();
    if !cfg.enabled() {
        return ApiError::not_found("single sign-on").into_response();
    }
    if let Some(err) = q.error.as_deref() {
        let detail = q.error_description.as_deref().unwrap_or(err);
        return fail(
            jar,
            &state,
            &format!("the identity provider refused: {detail}"),
        );
    }
    let pending = jar
        .get(COOKIE)
        .and_then(|c| state.secrets.open_token(c.value()))
        .and_then(|p| serde_json::from_slice::<Pending>(&p).ok())
        .filter(|p| p.exp >= now_secs());
    let Some(pending) = pending else {
        return fail(
            jar,
            &state,
            "the sign-in took too long or was started elsewhere; try again",
        );
    };
    let state_ok = q.state.as_deref().is_some_and(|s| {
        s.len() == pending.s.len()
            && s.bytes()
                .zip(pending.s.bytes())
                .fold(0u8, |acc, (a, b)| acc | (a ^ b))
                == 0
    });
    let (true, Some(code)) = (state_ok, q.code.as_deref()) else {
        return fail(jar, &state, "invalid sign-in response; try again");
    };
    match finish(&state, &cfg, code, &pending).await {
        Ok(user_id) => {
            let session =
                match create_session(&state, user_id, &user_agent(&headers), &ip.to_string()).await
                {
                    Ok(s) => s,
                    Err(_) => return fail(jar, &state, "could not start a session; try again"),
                };
            (
                jar.add(pending_cookie(&state, String::new(), 0))
                    .add(session.cookie),
                Redirect::to(&pending.next),
            )
                .into_response()
        }
        Err(message) => fail(jar, &state, &message),
    }
}

/// Code exchange, token validation and user lookup. Returns the user to sign in.
async fn finish(
    state: &AppState,
    cfg: &OidcConfig,
    code: &str,
    pending: &Pending,
) -> Result<Uuid, String> {
    let d = state.oidc.discovered(cfg, false).await?;
    let redirect = redirect_uri(state);
    let mut form = vec![
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", redirect.as_str()),
        ("code_verifier", pending.v.as_str()),
        ("client_id", cfg.client_id.as_str()),
    ];
    let methods = &d.provider.token_endpoint_auth_methods_supported;
    // client_secret_basic is the default when the provider lists nothing.
    let basic = methods.is_empty() || methods.iter().any(|m| m == "client_secret_basic");
    let mut req = state.oidc.http.post(&d.provider.token_endpoint);
    if basic {
        req = req.basic_auth(&cfg.client_id, Some(&cfg.client_secret));
    } else {
        form.push(("client_secret", cfg.client_secret.as_str()));
    }
    let body = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(&form)
        .finish();
    let resp = req
        .header("content-type", "application/x-www-form-urlencoded")
        .body(body)
        .send()
        .await
        .map_err(|e| format!("could not reach the token endpoint: {e}"))?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(format!(
            "the identity provider refused the code (HTTP {}): {}",
            status.as_u16(),
            body.chars().take(200).collect::<String>()
        ));
    }
    let tokens: TokenResponse = resp
        .json()
        .await
        .map_err(|e| format!("invalid token response: {e}"))?;
    let id_token = tokens
        .id_token
        .ok_or("the identity provider sent no ID token")?;
    let claims = validate(state, cfg, &id_token, &pending.nonce).await?;

    let mut email = claims.email.clone();
    let mut verified = not_unverified(claims.email_verified.as_ref());
    if email.is_none()
        && let (Some(endpoint), Some(access)) =
            (&d.provider.userinfo_endpoint, &tokens.access_token)
    {
        let info: UserInfo = state
            .oidc
            .http
            .get(endpoint)
            .bearer_auth(access)
            .send()
            .await
            .map_err(|e| format!("could not reach the userinfo endpoint: {e}"))?
            .json()
            .await
            .map_err(|e| format!("invalid userinfo response: {e}"))?;
        email = info.email;
        verified = not_unverified(info.email_verified.as_ref());
    }
    let email = email
        .map(|e| e.trim().to_lowercase())
        .filter(|e| e.contains('@'))
        .ok_or("the identity provider didn't share an email address")?;
    if !verified {
        return Err(format!("{email} isn't verified at the identity provider"));
    }
    let name = claims
        .name
        .clone()
        .or(claims.preferred_username.clone())
        .unwrap_or_default();
    find_or_create(state, cfg, &claims.sub, &email, &name).await
}

async fn validate(
    state: &AppState,
    cfg: &OidcConfig,
    token: &str,
    nonce: &str,
) -> Result<Claims, String> {
    let header = decode_header(token).map_err(|e| format!("invalid ID token: {e}"))?;
    let symmetric = matches!(
        header.alg,
        Algorithm::HS256 | Algorithm::HS384 | Algorithm::HS512
    );
    let mut found = None;
    for refresh in [false, true] {
        let d = state.oidc.discovered(cfg, refresh).await?;
        let key = if symmetric {
            // HS* ID tokens are signed with the client secret.
            Some(DecodingKey::from_secret(cfg.client_secret.as_bytes()))
        } else {
            let jwk = match &header.kid {
                Some(kid) => d.jwks.find(kid),
                None => d.jwks.keys.first(),
            };
            jwk.and_then(|k| DecodingKey::from_jwk(k).ok())
        };
        if let Some(k) = key {
            found = Some((k, d.provider.issuer));
            break;
        }
    }
    let (key, issuer) = found.ok_or("the ID token is signed with an unknown key")?;
    let mut v = Validation::new(header.alg);
    v.set_issuer(&[issuer.as_str(), issuer.trim_end_matches('/')]);
    v.set_audience(&[cfg.client_id.as_str()]);
    v.leeway = 60;
    let data = decode::<Claims>(token, &key, &v).map_err(|e| format!("invalid ID token: {e}"))?;
    if data.claims.nonce.as_deref() != Some(nonce) {
        return Err("the ID token doesn't match this sign-in (nonce)".into());
    }
    Ok(data.claims)
}

#[derive(sqlx::FromRow)]
struct Existing {
    id: Uuid,
    disabled: bool,
    oidc_subject: Option<String>,
}

async fn find_or_create(
    state: &AppState,
    cfg: &OidcConfig,
    sub: &str,
    email: &str,
    name: &str,
) -> Result<Uuid, String> {
    let db = |e: sqlx::Error| format!("database error: {e}");
    let by_subject: Option<Existing> =
        sqlx::query_as("SELECT id, disabled, oidc_subject FROM users WHERE oidc_subject = $1")
            .bind(sub)
            .fetch_optional(&state.pool)
            .await
            .map_err(db)?;
    let user = match by_subject {
        Some(u) => u,
        None => {
            let by_email: Option<Existing> = sqlx::query_as(
                "SELECT id, disabled, oidc_subject FROM users WHERE lower(email) = $1",
            )
            .bind(email)
            .fetch_optional(&state.pool)
            .await
            .map_err(db)?;
            match by_email {
                Some(u) if u.oidc_subject.as_deref().is_some_and(|s| s != sub) => {
                    return Err(format!(
                        "{email} is already linked to another single sign-on account"
                    ));
                }
                Some(u) => {
                    sqlx::query("UPDATE users SET oidc_subject = $2 WHERE id = $1")
                        .bind(u.id)
                        .bind(sub)
                        .execute(&state.pool)
                        .await
                        .map_err(db)?;
                    u
                }
                None => {
                    let domain = email.rsplit('@').next().unwrap_or_default();
                    let allowed = cfg.auto_create_domains.iter().any(|d| {
                        d.trim()
                            .trim_start_matches('@')
                            .eq_ignore_ascii_case(domain)
                    });
                    if !allowed {
                        return Err(format!(
                            "there is no Trailscope account for {email}; ask an admin for an invite"
                        ));
                    }
                    let id = Uuid::now_v7();
                    sqlx::query(
                        "INSERT INTO users (id, email, name, password_hash, role, can_view_bodies, oidc_subject) \
                         VALUES ($1, $2, $3, NULL, 'viewer', FALSE, $4)",
                    )
                    .bind(id)
                    .bind(email)
                    .bind(name.chars().take(120).collect::<String>())
                    .bind(sub)
                    .execute(&state.pool)
                    .await
                    .map_err(db)?;
                    tracing::info!(%email, "created a viewer account from single sign-on");
                    Existing {
                        id,
                        disabled: false,
                        oidc_subject: Some(sub.to_string()),
                    }
                }
            }
        }
    };
    if user.disabled {
        return Err(format!("the account for {email} is disabled"));
    }
    Ok(user.id)
}
