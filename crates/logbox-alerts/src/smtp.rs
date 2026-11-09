//! SMTP settings: stored in Postgres (password encrypted), overridden field
//! by field by configuration (`[smtp]` / `LOGBOX_SMTP_*`), which locks them.

use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use lettre::message::{Mailbox, MultiPart};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
use logbox_core::Config;
use logbox_core::config::SmtpConfig;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

use crate::crypto::{CryptoError, Keys};

pub use lettre::Address;

/// What the UI stores (`settings.smtp`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct StoredSmtp {
    pub host: String,
    pub port: Option<u16>,
    pub security: String,
    pub username: String,
    pub from: String,
    pub from_name: String,
    /// Base64 of the encrypted password.
    pub password_enc: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Security {
    Starttls,
    Tls,
    None,
}

impl Security {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "" | "starttls" => Some(Self::Starttls),
            "tls" | "ssl" | "implicit" => Some(Self::Tls),
            "none" | "plain" => Some(Self::None),
            _ => None,
        }
    }
}

/// The settings actually used to send.
#[derive(Clone)]
pub struct SmtpSettings {
    pub host: String,
    pub port: u16,
    pub security: Security,
    pub username: String,
    pub password: String,
    pub from: String,
    pub from_name: String,
}

impl std::fmt::Debug for SmtpSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SmtpSettings")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("security", &self.security)
            .field("username", &self.username)
            .field("from", &self.from)
            .finish_non_exhaustive()
    }
}

impl SmtpSettings {
    /// Host and sender are set, so emails can be sent.
    pub fn configured(&self) -> bool {
        !self.host.is_empty() && !self.from.is_empty()
    }
}

/// Fields set by configuration (env or `logbox.toml`); the UI shows them locked.
pub fn locked_fields(cfg: &SmtpConfig) -> Vec<&'static str> {
    let d = SmtpConfig::default();
    // An empty variable (a compose file passing `${LOGBOX_SMTP_HOST:-}`) doesn't lock the field.
    let env = |k: &str| logbox_core::config::env_is_set(&format!("SMTP_{k}"));
    let mut out = Vec::new();
    if env("HOST") || cfg.host != d.host {
        out.push("host");
    }
    if env("PORT") || cfg.port != d.port {
        out.push("port");
    }
    if env("SECURITY") || cfg.security != d.security {
        out.push("security");
    }
    if env("USERNAME") || cfg.username != d.username {
        out.push("username");
    }
    if env("PASSWORD") || cfg.password != d.password {
        out.push("password");
    }
    if env("FROM") || cfg.from != d.from {
        out.push("from");
    }
    if env("FROM_NAME") || cfg.from_name != d.from_name {
        out.push("from_name");
    }
    out
}

pub async fn load_stored(pool: &PgPool) -> Result<StoredSmtp, sqlx::Error> {
    let v: Option<serde_json::Value> =
        sqlx::query_scalar("SELECT value FROM settings WHERE key = 'smtp'")
            .fetch_optional(pool)
            .await?;
    Ok(v.and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default())
}

pub async fn save_stored(pool: &PgPool, s: &StoredSmtp) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO settings (key, value, updated_at) VALUES ('smtp', $1, now()) \
         ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, updated_at = now()",
    )
    .bind(sqlx::types::Json(s))
    .execute(pool)
    .await?;
    Ok(())
}

pub fn encrypt_password(keys: &Keys, password: &str) -> Result<String, CryptoError> {
    Ok(STANDARD.encode(keys.encrypt(password.as_bytes())?))
}

/// Configuration wins over stored values, field by field.
pub fn effective(
    config: &Config,
    stored: &StoredSmtp,
    keys: &Keys,
) -> Result<SmtpSettings, CryptoError> {
    let cfg = &config.smtp;
    let locked = locked_fields(cfg);
    let pick = |name: &str, from_cfg: &str, from_ui: &str| -> String {
        if locked.contains(&name) {
            from_cfg.to_string()
        } else {
            from_ui.trim().to_string()
        }
    };
    let password = if locked.contains(&"password") {
        cfg.password.clone()
    } else {
        match &stored.password_enc {
            Some(enc) => {
                let bytes = STANDARD.decode(enc).map_err(|_| CryptoError::Decrypt)?;
                String::from_utf8(keys.decrypt(&bytes)?).map_err(|_| CryptoError::Decrypt)?
            }
            None => String::new(),
        }
    };
    let security_text = pick("security", &cfg.security, &stored.security);
    Ok(SmtpSettings {
        host: pick("host", &cfg.host, &stored.host),
        port: if locked.contains(&"port") {
            cfg.port
        } else {
            stored.port.unwrap_or(cfg.port)
        },
        security: Security::parse(&security_text).unwrap_or(Security::Starttls),
        username: pick("username", &cfg.username, &stored.username),
        password,
        from: pick("from", &cfg.from, &stored.from),
        from_name: {
            let n = pick("from_name", &cfg.from_name, &stored.from_name);
            if n.is_empty() { "Logbox".into() } else { n }
        },
    })
}

pub async fn load(
    config: &Config,
    pool: &PgPool,
    keys: &Keys,
) -> Result<SmtpSettings, CryptoError> {
    let stored = load_stored(pool).await?;
    effective(config, &stored, keys)
}

/// An email ready to send.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Email {
    pub to: String,
    pub subject: String,
    pub text: String,
    pub html: String,
}

/// Sends one email. `Ok` carries the server's reply, `Err` the full error chain.
pub async fn send(settings: &SmtpSettings, email: &Email) -> Result<String, String> {
    if !settings.configured() {
        return Err("SMTP is not configured (host and from address are required)".into());
    }
    let from_addr = settings
        .from
        .parse()
        .map_err(|e| format!("invalid from address {:?}: {e}", settings.from))?;
    let to: Mailbox = email
        .to
        .parse()
        .map_err(|e| format!("invalid recipient {:?}: {e}", email.to))?;
    let message = Message::builder()
        .from(Mailbox::new(Some(settings.from_name.clone()), from_addr))
        .to(to)
        .subject(&email.subject)
        .multipart(MultiPart::alternative_plain_html(
            email.text.clone(),
            email.html.clone(),
        ))
        .map_err(|e| format!("could not build the message: {e}"))?;
    let builder = match settings.security {
        Security::Starttls => AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&settings.host),
        Security::Tls => AsyncSmtpTransport::<Tokio1Executor>::relay(&settings.host),
        Security::None => Ok(AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(
            &settings.host,
        )),
    }
    .map_err(|e| error_chain(&e))?;
    let mut builder = builder
        .port(settings.port)
        .timeout(Some(Duration::from_secs(20)));
    if !settings.username.is_empty() {
        builder = builder.credentials(Credentials::new(
            settings.username.clone(),
            settings.password.clone(),
        ));
    }
    let transport = builder.build();
    match transport.send(message).await {
        Ok(resp) => Ok(format!(
            "{} {}",
            resp.code(),
            resp.message().collect::<Vec<_>>().join(" ")
        )),
        Err(e) => Err(error_chain(&e)),
    }
}

/// `error: cause: cause`, so the UI can show exactly what the server said.
pub fn error_chain(e: &dyn std::error::Error) -> String {
    let mut out = e.to_string();
    let mut src = e.source();
    while let Some(s) = src {
        let msg = s.to_string();
        if !out.contains(&msg) {
            out.push_str(": ");
            out.push_str(&msg);
        }
        src = s.source();
    }
    out
}
