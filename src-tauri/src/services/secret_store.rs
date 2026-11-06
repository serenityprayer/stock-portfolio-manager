//! Local secret protection for `ai_config.api_key` (P4).
//!
//! Threat model: a single-user desktop app whose SQLite file may be copied,
//! backed up or synced. We bind the key to the local OS account (home dir +
//! user name) and use AES-256-GCM, so the database alone is not enough to
//! recover the credential.
//!
//! Honest limitation: a process already running as the same OS user can derive
//! the same key. Protecting against that requires an OS keychain / DPAPI
//! prompt, which would break cross-platform builds and add an interactive
//! step. This is deliberate — see the design doc's P4 deviation notes.
//!
//! Storage format: `enc:v1:<base64(12-byte nonce || ciphertext+tag)>`.
//! Values without the prefix are treated as legacy plaintext, so existing
//! installations keep working and are encrypted on their next save.

use aes_gcm::aead::{Aead, AeadCore, KeyInit, OsRng};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use sha2::{Digest, Sha256};

const PREFIX: &str = "enc:v1:";
const SALT: &[u8] = b"stock-portfolio-manager/ai_config/v1";

fn account_key() -> [u8; 32] {
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_default();
    let user = std::env::var("USERNAME")
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_default();
    let mut hasher = Sha256::new();
    hasher.update(SALT);
    hasher.update(home.as_bytes());
    hasher.update(b"|");
    hasher.update(user.as_bytes());
    let digest = hasher.finalize();
    let mut key = [0u8; 32];
    key.copy_from_slice(&digest);
    key
}

pub fn encrypt(plain: &str) -> String {
    if plain.is_empty() {
        return String::new();
    }
    let cipher = match Aes256Gcm::new_from_slice(&account_key()) {
        Ok(c) => c,
        Err(_) => return plain.to_string(),
    };
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let ciphertext = match cipher.encrypt(&nonce, plain.as_bytes()) {
        Ok(c) => c,
        Err(_) => return plain.to_string(),
    };
    let mut blob = nonce.to_vec();
    blob.extend_from_slice(&ciphertext);
    format!("{PREFIX}{}", STANDARD.encode(blob))
}

/// Legacy plaintext passes through untouched. Corrupt/tampered ciphertext
/// returns an empty string rather than a garbage key that would fail at call
/// time with an opaque 401.
pub fn decrypt(stored: &str) -> String {
    let Some(encoded) = stored.strip_prefix(PREFIX) else {
        return stored.to_string();
    };
    let blob = match STANDARD.decode(encoded) {
        Ok(b) => b,
        Err(_) => return String::new(),
    };
    if blob.len() < 13 {
        return String::new();
    }
    let cipher = match Aes256Gcm::new_from_slice(&account_key()) {
        Ok(c) => c,
        Err(_) => return String::new(),
    };
    let nonce = Nonce::from_slice(&blob[..12]);
    match cipher.decrypt(nonce, &blob[12..]) {
        Ok(bytes) => String::from_utf8(bytes).unwrap_or_default(),
        Err(_) => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let secret = "sk-test-1234567890";
        let stored = encrypt(secret);
        assert!(stored.starts_with(PREFIX));
        assert_ne!(stored, secret);
        assert_eq!(decrypt(&stored), secret);
    }

    #[test]
    fn legacy_plaintext_still_reads() {
        assert_eq!(decrypt("sk-legacy-key"), "sk-legacy-key");
        assert_eq!(decrypt(""), "");
    }

    #[test]
    fn tampered_ciphertext_is_rejected() {
        let stored = encrypt("sk-test");
        let mut chars: Vec<char> = stored.chars().collect();
        let last = chars.len() - 1;
        chars[last] = if chars[last] == 'A' { 'B' } else { 'A' };
        let tampered: String = chars.into_iter().collect();
        assert_eq!(decrypt(&tampered), "");
    }

    #[test]
    fn each_encryption_uses_a_fresh_nonce() {
        assert_ne!(encrypt("same"), encrypt("same"));
        assert_eq!(decrypt(&encrypt("same")), "same");
    }
}
