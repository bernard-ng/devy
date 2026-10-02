use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;

use super::SourceError;
use crate::config::Secret;

/// Shared-secret header check (Telegram's `X-Telegram-Bot-Api-Secret-Token`), in constant time.
pub fn verify_token(secret: &Secret, provided: Option<&str>) -> Result<(), SourceError> {
    let provided = provided.ok_or(SourceError::Unauthorized)?;
    if provided.as_bytes().ct_eq(secret.expose().as_bytes()).into() {
        Ok(())
    } else {
        Err(SourceError::Unauthorized)
    }
}

/// GitHub's `X-Hub-Signature-256: sha256=<hex hmac of the raw body>`.
pub fn verify_github_signature(
    secret: &Secret,
    header: Option<&str>,
    body: &[u8],
) -> Result<(), SourceError> {
    let signature = header
        .and_then(|h| h.strip_prefix("sha256="))
        .and_then(|hex| hex::decode(hex).ok())
        .ok_or(SourceError::Unauthorized)?;

    let mut mac =
        Hmac::<Sha256>::new_from_slice(secret.expose().as_bytes()).expect("HMAC accepts keys of any length");
    mac.update(body);
    mac.verify_slice(&signature)
        .map_err(|_| SourceError::Unauthorized)
}

/// Computes the header value GitHub would send; used by tests.
#[cfg(test)]
pub fn sign_github(secret: &Secret, body: &[u8]) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.expose().as_bytes()).unwrap();
    mac.update(body);
    format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_must_match_exactly() {
        let secret = Secret::new("s3cret");
        assert!(verify_token(&secret, Some("s3cret")).is_ok());
        assert!(verify_token(&secret, Some("s3cre")).is_err());
        assert!(verify_token(&secret, None).is_err());
    }

    #[test]
    fn github_signature_roundtrip() {
        let secret = Secret::new("s3cret");
        let header = sign_github(&secret, b"{}");
        assert!(verify_github_signature(&secret, Some(&header), b"{}").is_ok());
        assert!(verify_github_signature(&secret, Some(&header), b"{ }").is_err());
        assert!(verify_github_signature(&secret, Some("sha256=zz"), b"{}").is_err());
        assert!(verify_github_signature(&secret, Some("deadbeef"), b"{}").is_err());
        assert!(verify_github_signature(&secret, None, b"{}").is_err());
    }
}
