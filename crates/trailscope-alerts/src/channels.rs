//! Notification channels: generic webhook (HMAC-SHA256 signed), Slack incoming
//! webhook, Telegram bot and email. Secrets are stored encrypted.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use crate::crypto::{Keys, hmac_sha256};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChannelKind {
    Webhook,
    Slack,
    Telegram,
    Email,
}

impl ChannelKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Webhook => "webhook",
            Self::Slack => "slack",
            Self::Telegram => "telegram",
            Self::Email => "email",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "webhook" => Some(Self::Webhook),
            "slack" => Some(Self::Slack),
            "telegram" => Some(Self::Telegram),
            "email" => Some(Self::Email),
            _ => None,
        }
    }
}

/// Non-secret settings, returned by the API.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ChannelConfig {
    /// Telegram chat ID.
    pub chat_id: String,
    /// Email: members who receive it.
    pub member_ids: Vec<Uuid>,
    /// Email: other addresses that receive it.
    pub emails: Vec<String>,
    /// Webhook and Slack: the URL's host, to tell channels apart without revealing the URL.
    pub url_host: String,
}

/// Encrypted at rest; never returned by the API.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ChannelSecret {
    pub url: String,
    pub signing_secret: String,
    pub bot_token: String,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ChannelRow {
    pub id: Uuid,
    pub project_id: Uuid,
    pub name: String,
    pub kind: String,
    pub config: sqlx::types::Json<ChannelConfig>,
    pub secret: Option<Vec<u8>>,
}

pub async fn load(pool: &PgPool, ids: &[Uuid]) -> Result<Vec<ChannelRow>, sqlx::Error> {
    sqlx::query_as(
        "SELECT id, project_id, name, kind, config, secret FROM alert_channels WHERE id = ANY($1)",
    )
    .bind(ids)
    .fetch_all(pool)
    .await
}

/// Email addresses for a list of members (active accounts only) plus plain addresses.
pub async fn recipients(
    pool: &PgPool,
    member_ids: &[Uuid],
    emails: &[String],
) -> Result<Vec<String>, sqlx::Error> {
    let mut out: Vec<String> = sqlx::query_scalar(
        "SELECT email FROM users WHERE id = ANY($1) AND NOT disabled ORDER BY email",
    )
    .bind(member_ids)
    .fetch_all(pool)
    .await?;
    for e in emails {
        let e = e.trim().to_ascii_lowercase();
        if !e.is_empty() && !out.iter().any(|x| x.eq_ignore_ascii_case(&e)) {
            out.push(e);
        }
    }
    Ok(out)
}

pub fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(concat!("Trailscope/", env!("CARGO_PKG_VERSION")))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

/// Sends a non-email message. `payload` is what the outbox stored for this channel kind.
pub async fn send(
    http: &reqwest::Client,
    keys: &Keys,
    ch: &ChannelRow,
    payload: &Value,
) -> Result<String, String> {
    let secret: ChannelSecret = match &ch.secret {
        Some(s) => keys.decrypt_json(s).map_err(|e| e.to_string())?,
        None => ChannelSecret::default(),
    };
    let kind =
        ChannelKind::parse(&ch.kind).ok_or_else(|| format!("unknown channel kind {}", ch.kind))?;
    let resp = match kind {
        ChannelKind::Webhook => {
            let body = serde_json::to_vec(payload.get("body").unwrap_or(payload))
                .map_err(|e| e.to_string())?;
            let ts = time::OffsetDateTime::now_utc().unix_timestamp().to_string();
            let mut signed = ts.clone().into_bytes();
            signed.push(b'.');
            signed.extend_from_slice(&body);
            let sig = hex::encode(hmac_sha256(secret.signing_secret.as_bytes(), &signed));
            http.post(&secret.url)
                .header("content-type", "application/json")
                .header("x-trailscope-timestamp", ts)
                .header("x-trailscope-signature", format!("sha256={sig}"))
                .body(body)
                .send()
                .await
        }
        ChannelKind::Slack => {
            let text = payload
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_default();
            http.post(&secret.url)
                .json(&json!({ "text": text }))
                .send()
                .await
        }
        ChannelKind::Telegram => {
            let text = payload
                .get("html")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let url = format!(
                "https://api.telegram.org/bot{}/sendMessage",
                secret.bot_token
            );
            http.post(url)
                .json(&json!({
                    "chat_id": ch.config.chat_id,
                    "text": text,
                    "parse_mode": "HTML",
                    "disable_web_page_preview": true,
                }))
                .send()
                .await
        }
        ChannelKind::Email => return Err("email channels are delivered per recipient".into()),
    }
    // Never echo the URL (it is the secret for Slack and webhooks).
    .map_err(|e| crate::smtp::error_chain(&e.without_url()))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    let snippet: String = text.chars().take(300).collect();
    if status.is_success() {
        Ok(format!("{} {}", status.as_u16(), snippet)
            .trim()
            .to_string())
    } else {
        Err(format!("HTTP {}: {}", status.as_u16(), snippet))
    }
}
