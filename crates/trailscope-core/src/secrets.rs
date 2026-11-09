//! Tokens: ingest keys, API tokens, session tokens. Only SHA-256 hashes are
//! stored; plaintext is shown to the user once.

use rand::distr::{Alphanumeric, SampleString};
use sha2::{Digest, Sha256};

pub const INGEST_KEY_PREFIX: &str = "trs_ingest_";
pub const API_TOKEN_PREFIX: &str = "trs_api_";

/// A fresh random token: `prefix` + 40 base62 characters (~238 bits).
pub fn generate_token(prefix: &str) -> String {
    let mut s = String::with_capacity(prefix.len() + 40);
    s.push_str(prefix);
    Alphanumeric.append_string(&mut rand::rng(), &mut s, 40);
    s
}

/// Hex-encoded SHA-256 of a token, as stored in Postgres.
pub fn hash_token(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

/// The first characters of a token, safe to display (`trs_ingest_AbCd…`).
pub fn display_prefix(token: &str) -> String {
    let cut = token
        .char_indices()
        .nth(prefix_len(token) + 4)
        .map_or(token.len(), |(i, _)| i);
    format!("{}…", &token[..cut])
}

fn prefix_len(token: &str) -> usize {
    [INGEST_KEY_PREFIX, API_TOKEN_PREFIX]
        .iter()
        .find(|p| token.starts_with(**p))
        .map_or(0, |p| p.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_unique_and_prefixed() {
        let a = generate_token(INGEST_KEY_PREFIX);
        let b = generate_token(INGEST_KEY_PREFIX);
        assert_ne!(a, b);
        assert!(a.starts_with("trs_ingest_"));
        assert_eq!(a.len(), INGEST_KEY_PREFIX.len() + 40);
    }

    #[test]
    fn hashes_are_stable_hex() {
        assert_eq!(
            hash_token("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn display_prefix_keeps_four_secret_chars() {
        assert_eq!(display_prefix("trs_ingest_AbCdEfGh"), "trs_ingest_AbCd…");
        assert_eq!(display_prefix("trs_api_XyZ12345"), "trs_api_XyZ1…");
    }
}
