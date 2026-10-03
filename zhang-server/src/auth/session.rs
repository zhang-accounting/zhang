//! Signed session tokens: `<claims>.<signature>`, both base64url without padding, where the
//! signature is the HMAC-SHA256 of the encoded claims.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// What a session token carries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionClaims {
    /// who the session belongs to
    pub sub: String,
    /// expiry, in unix seconds
    pub exp: i64,
    /// for a password session: a fingerprint of the credential it was created with, so changing
    /// `ZHANG_AUTH` ends the sessions created with the old one
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pw: Option<String>,
    /// for a passkey session: the id of the passkey it was created with, so removing the passkey
    /// ends its sessions
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pk: Option<String>,
}

/// The HMAC key that signs the session tokens.
pub struct SessionKey(Vec<u8>);

impl SessionKey {
    /// `secret` is `ZHANG_SESSION_SECRET`; without it a random key is generated, so the sessions
    /// end when the server restarts.
    pub fn new(secret: Option<&str>) -> Self {
        match secret {
            Some(secret) => SessionKey(secret.as_bytes().to_vec()),
            None => SessionKey(rand::random::<[u8; 32]>().to_vec()),
        }
    }

    fn mac(&self) -> HmacSha256 {
        HmacSha256::new_from_slice(&self.0).expect("hmac accepts keys of any length")
    }

    fn tag(&self, data: &[u8]) -> Vec<u8> {
        let mut mac = self.mac();
        mac.update(data);
        mac.finalize().into_bytes().to_vec()
    }

    pub fn sign(&self, claims: &SessionClaims) -> String {
        let payload = URL_SAFE_NO_PAD.encode(serde_json::to_vec(claims).expect("session claims are serializable"));
        let signature = URL_SAFE_NO_PAD.encode(self.tag(payload.as_bytes()));
        format!("{payload}.{signature}")
    }

    /// The claims of `token` when its signature is valid and it has not expired at `now` (unix seconds).
    pub fn verify(&self, token: &str, now: i64) -> Option<SessionClaims> {
        let (payload, signature) = token.split_once('.')?;
        let signature = URL_SAFE_NO_PAD.decode(signature).ok()?;
        let mut mac = self.mac();
        mac.update(payload.as_bytes());
        mac.verify_slice(&signature).ok()?;
        let claims: SessionClaims = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(payload).ok()?).ok()?;
        (claims.exp > now).then_some(claims)
    }

    /// A short keyed digest of `data`, safe to embed in a token.
    pub fn fingerprint(&self, data: &[u8]) -> String {
        URL_SAFE_NO_PAD.encode(&self.tag(data)[..16])
    }

    /// Compares two secrets in constant time (with respect to their content and their lengths).
    pub fn secrets_equal(&self, given: &[u8], expected: &[u8]) -> bool {
        let expected = self.tag(expected);
        let mut mac = self.mac();
        mac.update(given);
        mac.verify_slice(&expected).is_ok()
    }
}

#[cfg(test)]
mod test {
    use super::{SessionClaims, SessionKey};

    fn claims(exp: i64) -> SessionClaims {
        SessionClaims {
            sub: "admin".to_owned(),
            exp,
            pw: Some("fingerprint".to_owned()),
            pk: None,
        }
    }

    #[test]
    fn signed_tokens_verify_until_they_expire() {
        let key = SessionKey::new(Some("secret"));
        let token = key.sign(&claims(100));
        assert_eq!(key.verify(&token, 99), Some(claims(100)));
        assert_eq!(key.verify(&token, 100), None);
    }

    #[test]
    fn tokens_signed_with_another_key_or_tampered_are_rejected() {
        let key = SessionKey::new(Some("secret"));
        let token = SessionKey::new(Some("another")).sign(&claims(100));
        assert_eq!(key.verify(&token, 0), None);

        let token = key.sign(&claims(100));
        let (_, signature) = token.split_once('.').unwrap();
        let forged = format!("{}.{}", key.sign(&claims(i64::MAX)).split_once('.').unwrap().0, signature);
        assert_eq!(key.verify(&forged, 0), None);
        assert_eq!(key.verify("garbage", 0), None);
        assert_eq!(key.verify("", 0), None);
    }

    #[test]
    fn random_keys_differ() {
        let token = SessionKey::new(None).sign(&claims(100));
        assert_eq!(SessionKey::new(None).verify(&token, 0), None);
    }

    #[test]
    fn secrets_are_compared_exactly() {
        let key = SessionKey::new(None);
        assert!(key.secrets_equal(b"letmein", b"letmein"));
        assert!(!key.secrets_equal(b"letmein", b"letmein!"));
        assert!(!key.secrets_equal(b"", b"letmein"));
    }
}
