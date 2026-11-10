//! TOTP (RFC 6238) helpers for two-factor authentication.

use anyhow::{Result, anyhow};
use rand::RngCore;
use totp_rs::{Algorithm, Secret, TOTP};

pub const ISSUER: &str = "SynthPanel";

/// Generate a new base32 encoded shared secret.
pub fn generate_secret() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    base32::encode(base32::Alphabet::Rfc4648 { padding: false }, &bytes)
}

/// Build a TOTP instance for an account.
pub fn build(secret: &str, account: &str) -> Result<TOTP> {
    let bytes = Secret::Encoded(secret.to_string())
        .to_bytes()
        .map_err(|error| anyhow!("invalid TOTP secret: {error}"))?;
    TOTP::new(
        Algorithm::SHA1,
        6,
        1,
        30,
        bytes,
        Some(ISSUER.to_string()),
        account.to_string(),
    )
    .map_err(|error| anyhow!("cannot build TOTP: {error}"))
}

/// Verify a six digit code against the shared secret.
pub fn verify(secret: &str, code: &str, account: &str) -> bool {
    let code = code.trim().replace(' ', "");
    if code.len() != 6 || !code.chars().all(|c| c.is_ascii_digit()) {
        return false;
    }
    match build(secret, account) {
        Ok(totp) => totp.check_current(&code).unwrap_or(false),
        Err(_) => false,
    }
}

/// `otpauth://` provisioning URI for authenticator apps.
pub fn provisioning_uri(secret: &str, account: &str) -> Result<String> {
    Ok(build(secret, account)?.get_url())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_secret_is_base32() {
        let secret = generate_secret();
        assert!(
            secret
                .chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
        );
        assert!(secret.len() >= 32);
        assert!(build(&secret, "user@example.com").is_ok());
    }

    #[test]
    fn valid_code_is_accepted() {
        let secret = generate_secret();
        let totp = build(&secret, "user@example.com").unwrap();
        let code = totp.generate_current().unwrap();
        assert!(verify(&secret, &code, "user@example.com"));
    }

    #[test]
    fn invalid_codes_are_rejected() {
        let secret = generate_secret();
        assert!(!verify(&secret, "000000", "user@example.com"));
        assert!(!verify(&secret, "abcdef", "user@example.com"));
        assert!(!verify(&secret, "", "user@example.com"));
        assert!(!verify("not-base32!", "123456", "user@example.com"));
    }
}
