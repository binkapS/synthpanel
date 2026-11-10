//! Password hashing, token generation and hashing.

use anyhow::Result;
use argon2::Argon2;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use rand::RngCore;
use sha2::{Digest, Sha256};

/// Hash a password with Argon2id using a random salt.
pub fn hash_password(password: &str) -> Result<String> {
    let salt = SaltString::generate(&mut argon2::password_hash::rand_core::OsRng);
    let hash = Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|error| anyhow::anyhow!("failed to hash password: {error}"))?;
    Ok(hash.to_string())
}

/// Verify a password against a stored PHC hash. Returns false for malformed
/// hashes instead of erroring, so callers cannot distinguish the two.
pub fn verify_password(password: &str, hash: &str) -> bool {
    let parsed = match PasswordHash::new(hash) {
        Ok(parsed) => parsed,
        Err(_) => return false,
    };
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok()
}

/// Generate a cryptographically random hex token.
pub fn generate_token(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    rand::thread_rng().fill_bytes(&mut buf);
    hex::encode(buf)
}

/// Hash a session token with SHA-256 so raw tokens are never stored.
pub fn hash_token(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    hex::encode(hasher.finalize())
}

/// Generate a CSRF token.
pub fn generate_csrf() -> String {
    generate_token(24)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_roundtrip() {
        let hash = hash_password("correct horse battery staple").unwrap();
        assert!(hash.starts_with("$argon2"));
        assert!(verify_password("correct horse battery staple", &hash));
        assert!(!verify_password("wrong", &hash));
        assert!(!verify_password("anything", "not-a-hash"));
    }

    #[test]
    fn tokens_are_unique_and_hashable() {
        let a = generate_token(16);
        let b = generate_token(16);
        assert_ne!(a, b);
        assert_eq!(a.len(), 32);
        assert_eq!(hash_token(&a).len(), 64);
        assert_ne!(hash_token(&a), hash_token(&b));
    }
}
