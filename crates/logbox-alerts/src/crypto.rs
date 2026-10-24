//! Encryption of stored secrets (XChaCha20-Poly1305) and HMAC-SHA256 signing
//! (mute links, webhook payloads), keyed from `secret_key`.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chacha20poly1305::aead::{Aead, Generate, KeyInit};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use hmac::{Hmac, Mac};
use logbox_core::Config;
use rand::distr::{Alphanumeric, SampleString};
use sha2::{Digest, Sha256};
use sqlx::PgPool;

const NONCE_LEN: usize = 24;

#[derive(Debug, thiserror::Error)]
pub enum CryptoError {
    #[error("stored secret could not be decrypted (was LOGBOX_SECRET_KEY changed?)")]
    Decrypt,
    #[error("encryption failed")]
    Encrypt,
    #[error(transparent)]
    Db(#[from] sqlx::Error),
}

/// Keys derived from the configured secret.
#[derive(Clone)]
pub struct Keys {
    enc: [u8; 32],
    mac: [u8; 32],
    /// The secret came from configuration rather than the database fallback.
    pub from_config: bool,
}

impl std::fmt::Debug for Keys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Keys")
            .field("from_config", &self.from_config)
            .finish_non_exhaustive()
    }
}

impl Keys {
    pub fn from_secret(secret: &str, from_config: bool) -> Self {
        let derive = |label: &str| -> [u8; 32] {
            let mut h = Sha256::new();
            h.update(label.as_bytes());
            h.update(secret.as_bytes());
            h.finalize().into()
        };
        Self {
            enc: derive("logbox-enc\0"),
            mac: derive("logbox-mac\0"),
            from_config,
        }
    }

    /// `secret_key` from configuration, or a random key generated once and
    /// kept in the `settings` table.
    pub async fn load(config: &Config, pool: &PgPool) -> Result<Self, CryptoError> {
        if !config.secret_key.is_empty() {
            return Ok(Self::from_secret(&config.secret_key, true));
        }
        let fresh = Alphanumeric.sample_string(&mut rand::rng(), 48);
        sqlx::query(
            "INSERT INTO settings (key, value) VALUES ('secret_key', to_jsonb($1::text)) \
             ON CONFLICT (key) DO NOTHING",
        )
        .bind(&fresh)
        .execute(pool)
        .await?;
        let stored: String =
            sqlx::query_scalar("SELECT value #>> '{}' FROM settings WHERE key = 'secret_key'")
                .fetch_one(pool)
                .await?;
        Ok(Self::from_secret(&stored, false))
    }

    /// `nonce || ciphertext`.
    pub fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>, CryptoError> {
        let cipher =
            XChaCha20Poly1305::new_from_slice(&self.enc).map_err(|_| CryptoError::Encrypt)?;
        let nonce = XNonce::generate();
        let ct = cipher
            .encrypt(&nonce, plaintext)
            .map_err(|_| CryptoError::Encrypt)?;
        let mut out = Vec::with_capacity(NONCE_LEN + ct.len());
        out.extend_from_slice(nonce.as_slice());
        out.extend_from_slice(&ct);
        Ok(out)
    }

    pub fn decrypt(&self, data: &[u8]) -> Result<Vec<u8>, CryptoError> {
        if data.len() < NONCE_LEN {
            return Err(CryptoError::Decrypt);
        }
        let (nonce, ct) = data.split_at(NONCE_LEN);
        let nonce = XNonce::try_from(nonce).map_err(|_| CryptoError::Decrypt)?;
        let cipher =
            XChaCha20Poly1305::new_from_slice(&self.enc).map_err(|_| CryptoError::Decrypt)?;
        cipher.decrypt(&nonce, ct).map_err(|_| CryptoError::Decrypt)
    }

    pub fn encrypt_json<T: serde::Serialize>(&self, value: &T) -> Result<Vec<u8>, CryptoError> {
        let bytes = serde_json::to_vec(value).map_err(|_| CryptoError::Encrypt)?;
        self.encrypt(&bytes)
    }

    pub fn decrypt_json<T: serde::de::DeserializeOwned>(
        &self,
        data: &[u8],
    ) -> Result<T, CryptoError> {
        serde_json::from_slice(&self.decrypt(data)?).map_err(|_| CryptoError::Decrypt)
    }

    pub fn sign(&self, data: &[u8]) -> Vec<u8> {
        hmac_sha256(&self.mac, data)
    }

    pub fn verify(&self, data: &[u8], tag: &[u8]) -> bool {
        let Ok(mut mac) = <Hmac<Sha256> as KeyInit>::new_from_slice(&self.mac) else {
            return false;
        };
        mac.update(data);
        mac.verify_slice(tag).is_ok()
    }

    /// `base64url(payload).base64url(hmac)`.
    pub fn sign_token(&self, payload: &[u8]) -> String {
        format!(
            "{}.{}",
            URL_SAFE_NO_PAD.encode(payload),
            URL_SAFE_NO_PAD.encode(self.sign(payload))
        )
    }

    /// The payload of a token made by [`Keys::sign_token`], if the signature is valid.
    pub fn open_token(&self, token: &str) -> Option<Vec<u8>> {
        let (p, t) = token.split_once('.')?;
        let payload = URL_SAFE_NO_PAD.decode(p).ok()?;
        let tag = URL_SAFE_NO_PAD.decode(t).ok()?;
        self.verify(&payload, &tag).then_some(payload)
    }
}

/// HMAC-SHA256 with an arbitrary key (webhook signatures use the channel's own secret).
pub fn hmac_sha256(key: &[u8], data: &[u8]) -> Vec<u8> {
    match <Hmac<Sha256> as KeyInit>::new_from_slice(key) {
        Ok(mut mac) => {
            mac.update(data);
            mac.finalize().into_bytes().to_vec()
        }
        // HMAC accepts keys of any length; this branch is unreachable in practice.
        Err(_) => Vec::new(),
    }
}
