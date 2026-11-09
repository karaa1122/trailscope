//! Alerts: rules, channels, the built-in "Email on 5xx" rule (with its
//! recipients), SMTP settings, send history, mutes and signed mute links.

use axum::Json;
use axum::extract::{Form, Path, Query, State};
use axum::http::StatusCode;
use axum::response::Html;
use rand::distr::{Alphanumeric, SampleString};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use trailscope_alerts::channels::{self, ChannelConfig, ChannelKind, ChannelRow, ChannelSecret};
use trailscope_alerts::outbox::NewNotification;
use trailscope_alerts::render::{self, ErrorEmail, Reason};
use trailscope_alerts::rules::{RuleKind, RuleQuery, RuleRow};
use trailscope_alerts::smtp::{self, Email, Security, StoredSmtp};
use trailscope_alerts::{email5xx, mute};
use trailscope_core::rows::ErrorSignal;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::auth::{Auth, Role, Scope};
use crate::error::{ApiError, ApiResult};
use crate::{AppState, now_ms};

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct ProjectParam {
    pub project: Uuid,
}

fn rfc3339(t: Option<time::OffsetDateTime>) -> Option<String> {
    t.and_then(|t| {
        t.format(&time::format_description::well_known::Rfc3339)
            .ok()
    })
}

// ---- rules ---------------------------------------------------------------------------

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RuleOut {
    pub id: Uuid,
    pub project: Uuid,
    pub name: String,
    pub kind: String,
    #[schema(value_type = Object)]
    pub query: Value,
    pub environment: String,
    pub threshold: f64,
    pub window_minutes: i32,
    pub cooldown_minutes: i32,
    pub channel_ids: Vec<Uuid>,
    pub enabled: bool,
    /// `ok` or `firing`.
    pub state: String,
    pub last_value: Option<f64>,
    pub last_error: Option<String>,
    pub last_evaluated_at: Option<String>,
    pub state_changed_at: Option<String>,
    pub last_notified_at: Option<String>,
}

#[derive(sqlx::FromRow)]
struct RuleDb {
    id: Uuid,
    project_id: Uuid,
    name: String,
    kind: String,
    query: sqlx::types::Json<Value>,
    environment: String,
    threshold: f64,
    window_minutes: i32,
    cooldown_minutes: i32,
    channel_ids: Vec<Uuid>,
    enabled: bool,
    state: String,
    last_value: Option<f64>,
    last_error: Option<String>,
    last_evaluated_at: Option<time::OffsetDateTime>,
    state_changed_at: Option<time::OffsetDateTime>,
    last_notified_at: Option<time::OffsetDateTime>,
}

const RULE_COLUMNS: &str = "id, project_id, name, kind, query, environment, threshold, window_minutes, \
    cooldown_minutes, channel_ids, enabled, state, last_value, last_error, last_evaluated_at, state_changed_at, last_notified_at";

impl From<RuleDb> for RuleOut {
    fn from(r: RuleDb) -> Self {
        Self {
            id: r.id,
            project: r.project_id,
            name: r.name,
            kind: r.kind,
            query: r.query.0,
            environment: r.environment,
            threshold: r.threshold,
            window_minutes: r.window_minutes,
            cooldown_minutes: r.cooldown_minutes,
            channel_ids: r.channel_ids,
            enabled: r.enabled,
            state: r.state,
            last_value: r.last_value,
            last_error: r.last_error,
            last_evaluated_at: rfc3339(r.last_evaluated_at),
            state_changed_at: rfc3339(r.state_changed_at),
            last_notified_at: rfc3339(r.last_notified_at),
        }
    }
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RuleInput {
    pub project: Uuid,
    pub name: String,
    /// `count`, `error_rate`, `p95`, `new_error` or `spiking_error`.
    pub kind: String,
    /// `{"table": "requests", "input": {"mode": "sql", "text": "…"}}`
    #[schema(value_type = Object)]
    pub query: RuleQuery,
    #[serde(default)]
    pub environment: String,
    #[serde(default)]
    pub threshold: f64,
    pub window_minutes: i32,
    pub cooldown_minutes: Option<i32>,
    #[serde(default)]
    pub channel_ids: Vec<Uuid>,
    #[serde(default = "yes")]
    pub enabled: bool,
}

fn yes() -> bool {
    true
}

async fn validate_rule(state: &AppState, r: &RuleInput) -> ApiResult<(RuleKind, i32)> {
    let name = r.name.trim();
    if name.is_empty() || name.chars().count() > 120 {
        return Err(ApiError::bad_request(
            "give the rule a name (up to 120 characters)",
        ));
    }
    let kind =
        RuleKind::parse(&r.kind).ok_or_else(|| ApiError::bad_request("unknown rule kind"))?;
    if kind.uses_query() {
        r.query
            .expr()
            .map_err(|m| ApiError::bad_request(format!("filter: {m}")))?;
        if r.query.table() == trailscope_query::Table::Events && kind != RuleKind::Count {
            return Err(ApiError::bad_request(
                "error rate and p95 rules work on requests",
            ));
        }
    }
    if !r.threshold.is_finite() || r.threshold < 0.0 {
        return Err(ApiError::bad_request("the threshold must be a number ≥ 0"));
    }
    if !(1..=1440).contains(&r.window_minutes) {
        return Err(ApiError::bad_request(
            "the window must be 1 to 1440 minutes",
        ));
    }
    let cooldown = r
        .cooldown_minutes
        .unwrap_or((state.config.alerts.default_cooldown_secs / 60) as i32);
    if !(0..=10_080).contains(&cooldown) {
        return Err(ApiError::bad_request(
            "the cooldown must be 0 to 10080 minutes",
        ));
    }
    let owned: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM alert_channels WHERE project_id = $1 AND id = ANY($2)",
    )
    .bind(r.project)
    .bind(&r.channel_ids)
    .fetch_one(&state.pool)
    .await?;
    if owned as usize != r.channel_ids.len() {
        return Err(ApiError::bad_request("unknown channel for this project"));
    }
    Ok((kind, cooldown))
}

#[utoipa::path(get, path = "/api/v1/alerts/rules", tag = "alerts", params(ProjectParam), responses((status = 200, body = Vec<RuleOut>)))]
pub async fn list_rules(
    State(state): State<AppState>,
    auth: Auth,
    Query(q): Query<ProjectParam>,
) -> ApiResult<Json<Vec<RuleOut>>> {
    auth.require(Role::Viewer, Scope::Read)?;
    auth.check_project(q.project)?;
    let rows: Vec<RuleDb> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {RULE_COLUMNS} FROM alert_rules WHERE project_id = $1 ORDER BY name"
    )))
    .bind(q.project)
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(rows.into_iter().map(RuleOut::from).collect()))
}

#[utoipa::path(post, path = "/api/v1/alerts/rules", tag = "alerts", request_body = RuleInput, responses((status = 200, body = RuleOut)))]
pub async fn create_rule(
    State(state): State<AppState>,
    auth: Auth,
    Json(r): Json<RuleInput>,
) -> ApiResult<Json<RuleOut>> {
    auth.require(Role::Member, Scope::Write)?;
    auth.check_project(r.project)?;
    state.project_key(r.project).await?;
    let (kind, cooldown) = validate_rule(&state, &r).await?;
    let row: RuleDb = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "INSERT INTO alert_rules (id, project_id, name, kind, query, environment, threshold, window_minutes, \
           cooldown_minutes, channel_ids, enabled, created_by) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12) RETURNING {RULE_COLUMNS}"
    )))
    .bind(Uuid::now_v7())
    .bind(r.project)
    .bind(r.name.trim())
    .bind(kind.as_str())
    .bind(sqlx::types::Json(&r.query))
    .bind(r.environment.trim())
    .bind(r.threshold)
    .bind(r.window_minutes)
    .bind(cooldown)
    .bind(&r.channel_ids)
    .bind(r.enabled)
    .bind(auth.user.id)
    .fetch_one(&state.pool)
    .await?;
    Ok(Json(row.into()))
}

#[utoipa::path(put, path = "/api/v1/alerts/rules/{id}", tag = "alerts", request_body = RuleInput, responses((status = 200, body = RuleOut)))]
pub async fn update_rule(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
    Json(r): Json<RuleInput>,
) -> ApiResult<Json<RuleOut>> {
    auth.require(Role::Member, Scope::Write)?;
    auth.check_project(r.project)?;
    let (kind, cooldown) = validate_rule(&state, &r).await?;
    // A changed definition starts from `ok` again.
    let row: Option<RuleDb> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "UPDATE alert_rules SET name = $3, kind = $4, query = $5, environment = $6, threshold = $7, window_minutes = $8, \
           cooldown_minutes = $9, channel_ids = $10, enabled = $11, updated_at = now(), \
           state = CASE WHEN kind = $4 AND query = $5 AND threshold = $7 AND window_minutes = $8 AND environment = $6 THEN state ELSE 'ok' END \
         WHERE id = $1 AND project_id = $2 RETURNING {RULE_COLUMNS}"
    )))
    .bind(id)
    .bind(r.project)
    .bind(r.name.trim())
    .bind(kind.as_str())
    .bind(sqlx::types::Json(&r.query))
    .bind(r.environment.trim())
    .bind(r.threshold)
    .bind(r.window_minutes)
    .bind(cooldown)
    .bind(&r.channel_ids)
    .bind(r.enabled)
    .fetch_optional(&state.pool)
    .await?;
    row.map(|r| Json(r.into()))
        .ok_or_else(|| ApiError::not_found("rule"))
}

#[utoipa::path(delete, path = "/api/v1/alerts/rules/{id}", tag = "alerts", params(ProjectParam), responses((status = 204)))]
pub async fn delete_rule(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
    Query(q): Query<ProjectParam>,
) -> ApiResult<StatusCode> {
    auth.require(Role::Member, Scope::Write)?;
    auth.check_project(q.project)?;
    sqlx::query("DELETE FROM alert_rules WHERE id = $1 AND project_id = $2")
        .bind(id)
        .bind(q.project)
        .execute(&state.pool)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PreviewOut {
    pub value: f64,
    pub firing: bool,
}

/// The rule's value right now, for the editor.
#[utoipa::path(post, path = "/api/v1/alerts/rules/preview", tag = "alerts", request_body = RuleInput, responses((status = 200, body = PreviewOut)))]
pub async fn preview_rule(
    State(state): State<AppState>,
    auth: Auth,
    Json(r): Json<RuleInput>,
) -> ApiResult<Json<PreviewOut>> {
    auth.require(Role::Viewer, Scope::Read)?;
    auth.check_project(r.project)?;
    let (kind, cooldown) = validate_rule(
        &state,
        &RuleInput {
            channel_ids: Vec::new(),
            ..r_clone(&r)
        },
    )
    .await?;
    let row = RuleRow {
        id: Uuid::nil(),
        project_id: r.project,
        name: r.name.clone(),
        kind: kind.as_str().into(),
        query: sqlx::types::Json(r.query.clone()),
        environment: r.environment.trim().to_string(),
        threshold: r.threshold,
        window_minutes: r.window_minutes,
        cooldown_minutes: cooldown,
        channel_ids: Vec::new(),
        state: "ok".into(),
        state_changed_at: None,
        last_notified_at: None,
    };
    let value = trailscope_alerts::rules::evaluate(&state.ch, &row)
        .await
        .map_err(ApiError::bad_request)?;
    Ok(Json(PreviewOut {
        value,
        firing: value > r.threshold,
    }))
}

fn r_clone(r: &RuleInput) -> RuleInput {
    RuleInput {
        project: r.project,
        name: r.name.clone(),
        kind: r.kind.clone(),
        query: r.query.clone(),
        environment: r.environment.clone(),
        threshold: r.threshold,
        window_minutes: r.window_minutes,
        cooldown_minutes: r.cooldown_minutes,
        channel_ids: r.channel_ids.clone(),
        enabled: r.enabled,
    }
}

// ---- channels ------------------------------------------------------------------------

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ChannelOut {
    pub id: Uuid,
    pub name: String,
    pub kind: String,
    #[schema(value_type = Object)]
    pub config: ChannelConfig,
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ChannelInput {
    pub project: Uuid,
    pub name: String,
    /// `webhook`, `slack`, `telegram` or `email`.
    pub kind: String,
    /// Webhook or Slack incoming-webhook URL.
    #[serde(default)]
    pub url: String,
    /// Telegram bot token.
    #[serde(default)]
    pub bot_token: String,
    /// Telegram chat ID.
    #[serde(default)]
    pub chat_id: String,
    /// Email recipients: members…
    #[serde(default)]
    pub member_ids: Vec<Uuid>,
    /// …and other addresses.
    #[serde(default)]
    pub emails: Vec<String>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ChannelCreated {
    pub channel: ChannelOut,
    /// Webhooks only: the HMAC-SHA256 signing secret, shown once.
    pub signing_secret: Option<String>,
}

fn channel_out(r: ChannelRow) -> ChannelOut {
    ChannelOut {
        id: r.id,
        name: r.name,
        kind: r.kind,
        config: r.config.0,
    }
}

fn http_url(raw: &str, what: &str) -> ApiResult<url::Url> {
    let u = url::Url::parse(raw.trim())
        .map_err(|_| ApiError::bad_request(format!("enter the full {what} URL")))?;
    if !matches!(u.scheme(), "http" | "https") || u.host_str().is_none() {
        return Err(ApiError::bad_request(format!(
            "the {what} URL must be http or https"
        )));
    }
    Ok(u)
}

/// Lower-cased, de-duplicated addresses; rejects anything that isn't an email address.
pub fn clean_emails(list: &[String]) -> ApiResult<Vec<String>> {
    let mut out: Vec<String> = Vec::new();
    for e in list {
        let e = e.trim().to_ascii_lowercase();
        if e.is_empty() {
            continue;
        }
        if e.parse::<lettre_address::Address>().is_err() {
            return Err(ApiError::bad_request(format!(
                "“{e}” is not an email address"
            )));
        }
        if !out.contains(&e) {
            out.push(e);
        }
    }
    if out.len() > 100 {
        return Err(ApiError::bad_request("at most 100 addresses"));
    }
    Ok(out)
}

/// Re-export so this module can parse addresses without naming lettre directly.
mod lettre_address {
    pub use trailscope_alerts::smtp::Address;
}

async fn check_members(state: &AppState, ids: &[Uuid]) -> ApiResult<()> {
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM users WHERE id = ANY($1)")
        .bind(ids)
        .fetch_one(&state.pool)
        .await?;
    if n as usize != ids.len() {
        return Err(ApiError::bad_request("unknown member"));
    }
    Ok(())
}

#[utoipa::path(get, path = "/api/v1/alerts/channels", tag = "alerts", params(ProjectParam), responses((status = 200, body = Vec<ChannelOut>)))]
pub async fn list_channels(
    State(state): State<AppState>,
    auth: Auth,
    Query(q): Query<ProjectParam>,
) -> ApiResult<Json<Vec<ChannelOut>>> {
    auth.require(Role::Viewer, Scope::Read)?;
    auth.check_project(q.project)?;
    let rows: Vec<ChannelRow> = sqlx::query_as(
        "SELECT id, project_id, name, kind, config, secret FROM alert_channels WHERE project_id = $1 ORDER BY name",
    )
    .bind(q.project)
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(rows.into_iter().map(channel_out).collect()))
}

#[utoipa::path(post, path = "/api/v1/alerts/channels", tag = "alerts", request_body = ChannelInput, responses((status = 200, body = ChannelCreated)))]
pub async fn create_channel(
    State(state): State<AppState>,
    auth: Auth,
    Json(c): Json<ChannelInput>,
) -> ApiResult<Json<ChannelCreated>> {
    auth.require(Role::Admin, Scope::Admin)?;
    auth.check_project(c.project)?;
    state.project_key(c.project).await?;
    let name = c.name.trim();
    if name.is_empty() || name.chars().count() > 100 {
        return Err(ApiError::bad_request("give the channel a name"));
    }
    let kind =
        ChannelKind::parse(&c.kind).ok_or_else(|| ApiError::bad_request("unknown channel kind"))?;
    let mut config = ChannelConfig::default();
    let mut secret = ChannelSecret::default();
    let mut signing = None;
    match kind {
        ChannelKind::Webhook | ChannelKind::Slack => {
            let u = http_url(
                &c.url,
                if kind == ChannelKind::Slack {
                    "Slack webhook"
                } else {
                    "webhook"
                },
            )?;
            config.url_host = u.host_str().unwrap_or_default().to_string();
            secret.url = u.to_string();
            if kind == ChannelKind::Webhook {
                let s = format!("whsec_{}", Alphanumeric.sample_string(&mut rand::rng(), 32));
                secret.signing_secret = s.clone();
                signing = Some(s);
            }
        }
        ChannelKind::Telegram => {
            if c.bot_token.trim().is_empty() || c.chat_id.trim().is_empty() {
                return Err(ApiError::bad_request(
                    "Telegram needs a bot token and a chat ID",
                ));
            }
            secret.bot_token = c.bot_token.trim().to_string();
            config.chat_id = c.chat_id.trim().to_string();
        }
        ChannelKind::Email => {
            check_members(&state, &c.member_ids).await?;
            config.member_ids = c.member_ids.clone();
            config.emails = clean_emails(&c.emails)?;
            if config.member_ids.is_empty() && config.emails.is_empty() {
                return Err(ApiError::bad_request("list at least one recipient"));
            }
        }
    }
    let enc = if kind == ChannelKind::Email {
        None
    } else {
        Some(
            state
                .secrets
                .encrypt_json(&secret)
                .map_err(ApiError::internal)?,
        )
    };
    let row: ChannelRow = sqlx::query_as(
        "INSERT INTO alert_channels (id, project_id, name, kind, config, secret) VALUES ($1, $2, $3, $4, $5, $6) \
         RETURNING id, project_id, name, kind, config, secret",
    )
    .bind(Uuid::now_v7())
    .bind(c.project)
    .bind(name)
    .bind(kind.as_str())
    .bind(sqlx::types::Json(&config))
    .bind(enc)
    .fetch_one(&state.pool)
    .await?;
    Ok(Json(ChannelCreated {
        channel: channel_out(row),
        signing_secret: signing,
    }))
}

#[utoipa::path(delete, path = "/api/v1/alerts/channels/{id}", tag = "alerts", params(ProjectParam), responses((status = 204)))]
pub async fn delete_channel(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
    Query(q): Query<ProjectParam>,
) -> ApiResult<StatusCode> {
    auth.require(Role::Admin, Scope::Admin)?;
    auth.check_project(q.project)?;
    let mut tx = state.pool.begin().await?;
    sqlx::query(
        "UPDATE alert_rules SET channel_ids = array_remove(channel_ids, $1) WHERE project_id = $2",
    )
    .bind(id)
    .bind(q.project)
    .execute(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM alert_channels WHERE id = $1 AND project_id = $2")
        .bind(id)
        .bind(q.project)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SendResult {
    pub ok: bool,
    /// What the server answered, on success.
    pub response: Option<String>,
    /// The exact error, on failure.
    pub error: Option<String>,
    /// Who it was sent to.
    pub recipients: Vec<String>,
}

#[utoipa::path(post, path = "/api/v1/alerts/channels/{id}/test", tag = "alerts", params(ProjectParam), responses((status = 200, body = SendResult)))]
pub async fn test_channel(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
    Query(q): Query<ProjectParam>,
) -> ApiResult<Json<SendResult>> {
    auth.require(Role::Admin, Scope::Admin)?;
    auth.check_project(q.project)?;
    let ch = channels::load(&state.pool, &[id])
        .await?
        .into_iter()
        .find(|c| c.project_id == q.project)
        .ok_or_else(|| ApiError::not_found("channel"))?;
    let kind =
        ChannelKind::parse(&ch.kind).ok_or_else(|| ApiError::internal("unknown channel kind"))?;
    let subject = "[Trailscope] Test notification".to_string();
    let text = format!(
        "This is a test from Trailscope for the channel “{}”. If you can read it, the channel works.",
        ch.name
    );
    if kind == ChannelKind::Email {
        let to =
            channels::recipients(&state.pool, &ch.config.member_ids, &ch.config.emails).await?;
        let html = format!("<p>{}</p>", render::esc(&text));
        return send_emails(
            &state,
            Some(q.project),
            Some(ch.id),
            "test",
            &to,
            &subject,
            &text,
            &html,
        )
        .await
        .map(Json);
    }
    let payload = json!({
        "text": text,
        "html": format!("<b>Trailscope</b>\n{}", render::esc(&text)),
        "body": { "type": "test", "message": text },
    });
    let result = channels::send(&channels::http_client(), &state.secrets, &ch, &payload).await;
    let n = NewNotification {
        project_id: Some(q.project),
        source: "test",
        rule_id: None,
        rule_name: String::new(),
        channel_id: Some(ch.id),
        channel_kind: kind,
        target: ch.name.clone(),
        reason: "test",
        fingerprint: String::new(),
        subject,
        payload,
    };
    state.outbox.record(n, &result).await?;
    Ok(Json(match result {
        Ok(r) => SendResult {
            ok: true,
            response: Some(r),
            error: None,
            recipients: vec![ch.name],
        },
        Err(e) => SendResult {
            ok: false,
            response: None,
            error: Some(e),
            recipients: vec![ch.name],
        },
    }))
}

/// Sends an email synchronously to each recipient and records every send.
#[allow(clippy::too_many_arguments)]
async fn send_emails(
    state: &AppState,
    project: Option<Uuid>,
    channel: Option<Uuid>,
    source: &'static str,
    to: &[String],
    subject: &str,
    text: &str,
    html: &str,
) -> ApiResult<SendResult> {
    if to.is_empty() {
        return Err(ApiError::bad_request(
            "no recipients: list members or addresses first",
        ));
    }
    let settings = smtp::load(&state.config, &state.pool, &state.secrets)
        .await
        .map_err(ApiError::internal)?;
    let mut first_err = None;
    let mut last_ok = None;
    for addr in to {
        let email = Email {
            to: addr.clone(),
            subject: subject.to_string(),
            text: text.to_string(),
            html: html.to_string(),
        };
        let result = smtp::send(&settings, &email).await;
        match &result {
            Ok(r) => last_ok = Some(r.clone()),
            Err(e) => {
                first_err.get_or_insert_with(|| format!("{addr}: {e}"));
            }
        }
        let n = NewNotification {
            project_id: project,
            source,
            rule_id: None,
            rule_name: if source == "test_5xx" {
                "Email on 5xx".into()
            } else {
                String::new()
            },
            channel_id: channel,
            channel_kind: ChannelKind::Email,
            target: addr.clone(),
            reason: "test",
            fingerprint: String::new(),
            subject: subject.to_string(),
            payload: serde_json::to_value(&email).unwrap_or_default(),
        };
        state.outbox.record(n, &result).await?;
    }
    Ok(SendResult {
        ok: first_err.is_none(),
        response: last_ok,
        error: first_err,
        recipients: to.to_vec(),
    })
}

// ---- Email on 5xx --------------------------------------------------------------------

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Recipient {
    pub email: String,
    pub name: String,
    /// Set for project members.
    pub member_id: Option<Uuid>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct EmailRuleOut {
    pub enabled: bool,
    pub filter: String,
    pub member_ids: Vec<Uuid>,
    pub emails: Vec<String>,
    /// Everyone who receives these emails (active members and addresses).
    pub recipients: Vec<Recipient>,
    pub smtp_configured: bool,
    /// Enabled, SMTP configured and at least one recipient.
    pub active: bool,
    pub throttle_minutes: u32,
    pub hourly_cap: u32,
    pub regression_hours: u32,
}

#[derive(sqlx::FromRow)]
struct EmailRuleDb {
    enabled: bool,
    filter: String,
    member_ids: Vec<Uuid>,
    emails: Vec<String>,
}

async fn email_rule_out(state: &AppState, project: Uuid) -> ApiResult<EmailRuleOut> {
    sqlx::query("INSERT INTO email_rules (project_id) VALUES ($1) ON CONFLICT DO NOTHING")
        .bind(project)
        .execute(&state.pool)
        .await?;
    let r: EmailRuleDb = sqlx::query_as(
        "SELECT enabled, filter, member_ids, emails FROM email_rules WHERE project_id = $1",
    )
    .bind(project)
    .fetch_one(&state.pool)
    .await?;
    let members: Vec<(Uuid, String, String)> = sqlx::query_as(
        "SELECT id, email, name FROM users WHERE id = ANY($1) AND NOT disabled ORDER BY name, email",
    )
    .bind(&r.member_ids)
    .fetch_all(&state.pool)
    .await?;
    let mut recipients: Vec<Recipient> = members
        .into_iter()
        .map(|(id, email, name)| Recipient {
            email,
            name,
            member_id: Some(id),
        })
        .collect();
    for e in &r.emails {
        if !recipients.iter().any(|x| x.email.eq_ignore_ascii_case(e)) {
            recipients.push(Recipient {
                email: e.clone(),
                name: String::new(),
                member_id: None,
            });
        }
    }
    let smtp_configured = smtp::load(&state.config, &state.pool, &state.secrets)
        .await
        .map(|s| s.configured())
        .unwrap_or(false);
    let a = &state.config.alerts;
    Ok(EmailRuleOut {
        active: r.enabled && smtp_configured && !recipients.is_empty(),
        enabled: r.enabled,
        filter: r.filter,
        member_ids: r.member_ids,
        emails: r.emails,
        recipients,
        smtp_configured,
        throttle_minutes: a.email_throttle_minutes,
        hourly_cap: a.email_hourly_cap,
        regression_hours: a.regression_hours,
    })
}

#[utoipa::path(get, path = "/api/v1/alerts/email-rule", tag = "alerts", params(ProjectParam), responses((status = 200, body = EmailRuleOut)))]
pub async fn get_email_rule(
    State(state): State<AppState>,
    auth: Auth,
    Query(q): Query<ProjectParam>,
) -> ApiResult<Json<EmailRuleOut>> {
    auth.require(Role::Viewer, Scope::Read)?;
    auth.check_project(q.project)?;
    state.project_key(q.project).await?;
    Ok(Json(email_rule_out(&state, q.project).await?))
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct EmailRuleInput {
    pub project: Uuid,
    pub enabled: bool,
    pub filter: String,
    #[serde(default)]
    pub member_ids: Vec<Uuid>,
    #[serde(default)]
    pub emails: Vec<String>,
}

#[utoipa::path(put, path = "/api/v1/alerts/email-rule", tag = "alerts", request_body = EmailRuleInput, responses((status = 200, body = EmailRuleOut)))]
pub async fn put_email_rule(
    State(state): State<AppState>,
    auth: Auth,
    Json(r): Json<EmailRuleInput>,
) -> ApiResult<Json<EmailRuleOut>> {
    auth.require(Role::Admin, Scope::Admin)?;
    auth.check_project(r.project)?;
    state.project_key(r.project).await?;
    let filter = r.filter.trim();
    email5xx::parse_filter(filter).map_err(|m| ApiError::bad_request(format!("filter: {m}")))?;
    let mut members = r.member_ids.clone();
    members.sort();
    members.dedup();
    check_members(&state, &members).await?;
    let emails = clean_emails(&r.emails)?;
    sqlx::query(
        "INSERT INTO email_rules (project_id, enabled, filter, member_ids, emails, updated_at) VALUES ($1, $2, $3, $4, $5, now()) \
         ON CONFLICT (project_id) DO UPDATE SET enabled = $2, filter = $3, member_ids = $4, emails = $5, updated_at = now()",
    )
    .bind(r.project)
    .bind(r.enabled)
    .bind(filter)
    .bind(&members)
    .bind(&emails)
    .execute(&state.pool)
    .await?;
    Ok(Json(email_rule_out(&state, r.project).await?))
}

/// Sends a sample 5xx email to every recipient, so they can check it arrives.
#[utoipa::path(post, path = "/api/v1/alerts/email-rule/test", tag = "alerts", params(ProjectParam), responses((status = 200, body = SendResult)))]
pub async fn test_email_rule(
    State(state): State<AppState>,
    auth: Auth,
    Query(q): Query<ProjectParam>,
) -> ApiResult<Json<SendResult>> {
    auth.require(Role::Admin, Scope::Admin)?;
    auth.check_project(q.project)?;
    let rule = email_rule_out(&state, q.project).await?;
    let to: Vec<String> = rule.recipients.iter().map(|r| r.email.clone()).collect();
    let project_name: String = sqlx::query_scalar("SELECT name FROM projects WHERE id = $1")
        .bind(q.project)
        .fetch_one(&state.pool)
        .await?;
    let sig = ErrorSignal {
        project_id: q.project.to_string(),
        environment: "production".into(),
        service: "example-svc".into(),
        release: "v1.2.3".into(),
        timestamp: now_ms(),
        request_id: "req_test".into(),
        method: "POST".into(),
        route: "/items/:id/archive".into(),
        status: 500,
        duration_ms: 1318.0,
        user_id: "u_test".into(),
        error_type: "ExampleError".into(),
        error_message: "this is a test email from Trailscope".into(),
        error_stack:
            "ExampleError: this is a test email from Trailscope\n    at archive (src/items.ts:42:11)"
                .into(),
        error_fingerprint: "0".into(),
        ..ErrorSignal::default()
    };
    let rendered = render::error_email(&ErrorEmail {
        sig: &sig,
        project_name: &project_name,
        reason: &Reason::First,
        totals: (1, 1),
        public_url: &state.config.server.public_url,
        mute_links: Vec::new(),
    });
    let subject = format!("{} (test)", rendered.subject);
    send_emails(
        &state,
        Some(q.project),
        None,
        "test_5xx",
        &to,
        &subject,
        &rendered.text,
        &rendered.html,
    )
    .await
    .map(Json)
}

// ---- SMTP settings -------------------------------------------------------------------

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SmtpOut {
    pub host: String,
    pub port: u16,
    /// `starttls`, `tls` or `none`.
    pub security: String,
    pub username: String,
    pub from: String,
    pub from_name: String,
    /// Whether a password is set (it is never returned).
    pub password_set: bool,
    /// Fields set by configuration (env or file); they can't be changed here.
    pub locked: Vec<String>,
    pub configured: bool,
    /// `TRAILSCOPE_SECRET_KEY` is set (otherwise a key stored in Postgres encrypts the password).
    pub secret_key_configured: bool,
}

async fn smtp_out(state: &AppState) -> ApiResult<SmtpOut> {
    let stored = smtp::load_stored(&state.pool).await?;
    let eff =
        smtp::effective(&state.config, &stored, &state.secrets).map_err(ApiError::internal)?;
    let locked = smtp::locked_fields(&state.config.smtp);
    Ok(SmtpOut {
        configured: eff.configured(),
        host: eff.host,
        port: eff.port,
        security: match eff.security {
            Security::Starttls => "starttls",
            Security::Tls => "tls",
            Security::None => "none",
        }
        .into(),
        username: eff.username,
        from: eff.from,
        from_name: eff.from_name,
        password_set: !eff.password.is_empty(),
        locked: locked.into_iter().map(str::to_string).collect(),
        secret_key_configured: state.secrets.from_config,
    })
}

#[utoipa::path(get, path = "/api/v1/settings/smtp", tag = "alerts", responses((status = 200, body = SmtpOut)))]
pub async fn get_smtp(State(state): State<AppState>, auth: Auth) -> ApiResult<Json<SmtpOut>> {
    auth.require(Role::Admin, Scope::Admin)?;
    Ok(Json(smtp_out(&state).await?))
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SmtpInput {
    pub host: String,
    pub port: u16,
    pub security: String,
    #[serde(default)]
    pub username: String,
    pub from: String,
    #[serde(default)]
    pub from_name: String,
    /// Absent or null keeps the stored password; an empty string clears it.
    pub password: Option<String>,
}

#[utoipa::path(put, path = "/api/v1/settings/smtp", tag = "alerts", request_body = SmtpInput, responses((status = 200, body = SmtpOut)))]
pub async fn put_smtp(
    State(state): State<AppState>,
    auth: Auth,
    Json(s): Json<SmtpInput>,
) -> ApiResult<Json<SmtpOut>> {
    auth.require(Role::Admin, Scope::Admin)?;
    if Security::parse(&s.security).is_none() {
        return Err(ApiError::bad_request(
            "security must be starttls, tls or none",
        ));
    }
    let from = s.from.trim();
    if !from.is_empty() && from.parse::<lettre_address::Address>().is_err() {
        return Err(ApiError::bad_request(
            "the from address is not a valid email address",
        ));
    }
    if s.host.trim().contains(['/', ' ', ':']) {
        return Err(ApiError::bad_request(
            "enter the host name only, e.g. smtp.example.com",
        ));
    }
    let mut stored = smtp::load_stored(&state.pool).await?;
    stored = StoredSmtp {
        host: s.host.trim().to_string(),
        port: Some(s.port),
        security: s.security.trim().to_ascii_lowercase(),
        username: s.username.trim().to_string(),
        from: from.to_string(),
        from_name: s.from_name.trim().to_string(),
        password_enc: stored.password_enc,
    };
    match s.password.as_deref() {
        None => {}
        Some("") => stored.password_enc = None,
        Some(p) => {
            stored.password_enc =
                Some(smtp::encrypt_password(&state.secrets, p).map_err(ApiError::internal)?)
        }
    }
    smtp::save_stored(&state.pool, &stored).await?;
    Ok(Json(smtp_out(&state).await?))
}

#[derive(Deserialize, ToSchema)]
pub struct SmtpTestInput {
    pub to: String,
}

#[utoipa::path(post, path = "/api/v1/settings/smtp/test", tag = "alerts", request_body = SmtpTestInput, responses((status = 200, body = SendResult)))]
pub async fn test_smtp(
    State(state): State<AppState>,
    auth: Auth,
    Json(t): Json<SmtpTestInput>,
) -> ApiResult<Json<SendResult>> {
    auth.require(Role::Admin, Scope::Admin)?;
    let to = clean_emails(std::slice::from_ref(&t.to))?;
    let text = "This is a test email from Trailscope. Your SMTP settings work.";
    let html = format!("<p style=\"font-family:sans-serif\">{text}</p>");
    send_emails(
        &state,
        None,
        None,
        "test",
        &to,
        "[Trailscope] Test email",
        text,
        &html,
    )
    .await
    .map(Json)
}

// ---- history and mutes ---------------------------------------------------------------

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct HistoryQuery {
    pub project: Uuid,
    /// Up to 500 (default 100).
    pub limit: Option<i64>,
    /// `pending`, `sent` or `failed`.
    pub status: Option<String>,
}

#[derive(Serialize, ToSchema, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct HistoryRow {
    pub id: Uuid,
    #[serde(with = "time::serde::rfc3339")]
    #[schema(value_type = String)]
    pub created_at: time::OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    #[schema(value_type = Option<String>)]
    pub sent_at: Option<time::OffsetDateTime>,
    pub source: String,
    pub rule_name: String,
    pub channel_kind: String,
    pub target: String,
    pub reason: String,
    pub fingerprint: String,
    pub subject: String,
    pub status: String,
    pub attempts: i32,
    pub last_error: Option<String>,
    pub response: Option<String>,
}

#[utoipa::path(get, path = "/api/v1/alerts/history", tag = "alerts", params(HistoryQuery), responses((status = 200, body = Vec<HistoryRow>)))]
pub async fn history(
    State(state): State<AppState>,
    auth: Auth,
    Query(q): Query<HistoryQuery>,
) -> ApiResult<Json<Vec<HistoryRow>>> {
    auth.require(Role::Viewer, Scope::Read)?;
    auth.check_project(q.project)?;
    let rows = sqlx::query_as(
        "SELECT id, created_at, sent_at, source, rule_name, channel_kind, target, reason, fingerprint, subject, status, \
           attempts, last_error, response FROM notifications \
         WHERE project_id = $1 AND ($2::text IS NULL OR status = $2) ORDER BY created_at DESC LIMIT $3",
    )
    .bind(q.project)
    .bind(q.status.as_deref().filter(|s| !s.is_empty()))
    .bind(q.limit.unwrap_or(100).clamp(1, 500))
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(rows))
}

#[derive(Serialize, ToSchema, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct MuteOut {
    pub fingerprint: String,
    /// `null` when muted forever.
    #[serde(with = "time::serde::rfc3339::option")]
    #[schema(value_type = Option<String>)]
    pub muted_until: Option<time::OffsetDateTime>,
    pub forever: bool,
    #[serde(with = "time::serde::rfc3339")]
    #[schema(value_type = String)]
    pub last_seen_at: time::OffsetDateTime,
}

#[utoipa::path(get, path = "/api/v1/alerts/mutes", tag = "alerts", params(ProjectParam), responses((status = 200, body = Vec<MuteOut>)))]
pub async fn list_mutes(
    State(state): State<AppState>,
    auth: Auth,
    Query(q): Query<ProjectParam>,
) -> ApiResult<Json<Vec<MuteOut>>> {
    auth.require(Role::Viewer, Scope::Read)?;
    auth.check_project(q.project)?;
    let rows = sqlx::query_as(
        "SELECT fingerprint, CASE WHEN muted_until = 'infinity' THEN NULL ELSE muted_until END AS muted_until, \
           muted_until = 'infinity' AS forever, last_seen_at \
         FROM email_groups WHERE project_id = $1 AND muted_until > now() ORDER BY last_seen_at DESC",
    )
    .bind(q.project)
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(rows))
}

#[utoipa::path(delete, path = "/api/v1/alerts/mutes/{fingerprint}", tag = "alerts", params(ProjectParam), responses((status = 204)))]
pub async fn delete_mute(
    State(state): State<AppState>,
    auth: Auth,
    Path(fingerprint): Path<String>,
    Query(q): Query<ProjectParam>,
) -> ApiResult<StatusCode> {
    auth.require(Role::Member, Scope::Write)?;
    auth.check_project(q.project)?;
    mute::unmute(&state.pool, q.project, &fingerprint).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize, utoipa::IntoParams, ToSchema)]
pub struct MuteParam {
    pub t: String,
}

fn page(title: &str, body: &str) -> Html<String> {
    Html(format!(
        r#"<!doctype html><html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>{t}</title>
<style>body{{margin:0;background:#f6f7f9;font-family:-apple-system,'Segoe UI',Roboto,Helvetica,Arial,sans-serif;color:#11151b}}
main{{max-width:480px;margin:12vh auto;padding:28px 24px;background:#fff;border:1px solid #e1e4ea;border-radius:12px}}
h1{{font-size:20px;margin:0 0 12px}}p{{line-height:1.5}}button{{padding:12px 18px;border:0;border-radius:8px;background:#11151b;color:#fff;font-size:15px;font-weight:600;cursor:pointer}}
code{{font-size:13px;background:#f1f3f6;padding:2px 6px;border-radius:4px}}</style></head><body><main>{b}</main></body></html>"#,
        t = render::esc(title),
        b = body
    ))
}

/// The page behind a mute link: asks for confirmation (link scanners only GET).
#[utoipa::path(get, path = "/api/v1/alerts/mute", tag = "alerts", params(MuteParam), responses((status = 200, description = "HTML page")))]
pub async fn mute_page(State(state): State<AppState>, Query(q): Query<MuteParam>) -> Html<String> {
    match mute::open(&state.secrets, &q.t, now_ms() / 1000) {
        Some(t) => page(
            "Mute this error",
            &format!(
                r#"<h1>Mute this error {}?</h1><p>Trailscope will stop emailing about error group <code>{}</code> {}. Errors are still recorded.</p>
<form method="post" action="/api/v1/alerts/mute"><input type="hidden" name="t" value="{}"><button type="submit">Mute {}</button></form>"#,
                t.label(),
                render::esc(&t.f),
                t.label(),
                render::esc(&q.t),
                t.label()
            ),
        ),
        None => page(
            "Link expired",
            "<h1>This link has expired or is not valid</h1><p>Mute links work for a limited time. Open the error in Trailscope to manage notifications.</p>",
        ),
    }
}

#[utoipa::path(post, path = "/api/v1/alerts/mute", tag = "alerts", request_body(content = MuteParam, content_type = "application/x-www-form-urlencoded"),
    responses((status = 200, description = "HTML page")))]
pub async fn mute_apply(
    State(state): State<AppState>,
    Form(f): Form<MuteParam>,
) -> ApiResult<Html<String>> {
    let Some(t) = mute::open(&state.secrets, &f.t, now_ms() / 1000) else {
        return Ok(page(
            "Link expired",
            "<h1>This link has expired or is not valid</h1><p>Open the error in Trailscope to manage notifications.</p>",
        ));
    };
    let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM projects WHERE id = $1)")
        .bind(t.p)
        .fetch_one(&state.pool)
        .await?;
    if !exists {
        return Err(ApiError::not_found("project"));
    }
    mute::apply(&state.pool, &t).await?;
    Ok(page(
        "Muted",
        &format!(
            "<h1>Muted {}</h1><p>You won't get emails about error group <code>{}</code> {}. You can unmute it under Alerts in Trailscope.</p>",
            t.label(),
            render::esc(&t.f),
            t.label()
        ),
    ))
}
