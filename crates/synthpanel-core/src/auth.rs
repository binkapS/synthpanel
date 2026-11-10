//! Password hashing shared between the web process and the installer.

use anyhow::Result;
use argon2::Argon2;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};

pub fn hash_password(password: &str) -> Result<String> {
    let salt = SaltString::generate(&mut argon2::password_hash::rand_core::OsRng);
    let hash = Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|error| anyhow::anyhow!("failed to hash the password: {error}"))?;
    Ok(hash.to_string())
}

pub fn verify_password(password: &str, hash: &str) -> bool {
    let parsed = match PasswordHash::new(hash) {
        Ok(parsed) => parsed,
        Err(_) => return false,
    };
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok()
}
