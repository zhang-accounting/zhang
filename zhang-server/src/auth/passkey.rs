//! Passkey (WebAuthn) records and the ceremonies in progress.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use webauthn_rs::prelude::{Passkey, PasskeyAuthentication, PasskeyRegistration, Url, Uuid};
use webauthn_rs::{Webauthn, WebauthnBuilder};

use super::AuthError;

/// Where the registered passkeys are kept, relative to the ledger root of the data source.
pub const PASSKEYS_PATH: &str = ".zhang/passkeys.json";

/// How long a started registration or login can be finished.
const CEREMONY_TTL: Duration = Duration::from_secs(5 * 60);

/// Pending ceremonies are created without a session (by the login page), so their number is capped.
const MAX_PENDING_CEREMONIES: usize = 1024;

/// A registered passkey, as stored in [`PASSKEYS_PATH`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PasskeyRecord {
    pub id: String,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub passkey: Passkey,
}

/// The relying party a ceremony runs for: its id (a domain) and the origin the browser reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelyingParty {
    pub id: String,
    pub origin: Url,
}

impl RelyingParty {
    pub fn webauthn(&self) -> Result<Webauthn, AuthError> {
        WebauthnBuilder::new(&self.id, &self.origin)
            .and_then(|builder| builder.rp_name("Zhang").build())
            .map_err(|_| {
                AuthError::BadRequest(format!(
                    "passkeys cannot be used from {}: the relying party id `{}` must be a domain name (or localhost) that the origin is on; \
                     open zhang through its domain name, or set ZHANG_PASSKEY_RP_ID and ZHANG_PASSKEY_ORIGIN",
                    self.origin.as_str().trim_end_matches('/'),
                    self.id
                ))
            })
    }
}

pub enum CeremonyKind {
    Registration { state: PasskeyRegistration, name: Option<String> },
    Authentication { state: PasskeyAuthentication },
}

/// A started ceremony, finished with the relying party it was started with.
pub struct Ceremony {
    pub kind: CeremonyKind,
    pub relying_party: RelyingParty,
    started_at: Instant,
}

/// Ceremonies in progress, keyed by a random state id; each one can be finished once, within [`CEREMONY_TTL`].
#[derive(Default)]
pub struct Ceremonies {
    pending: HashMap<String, Ceremony>,
}

impl Ceremonies {
    pub fn start(&mut self, kind: CeremonyKind, relying_party: RelyingParty) -> String {
        self.purge_expired();
        if self.pending.len() >= MAX_PENDING_CEREMONIES {
            if let Some(oldest) = self.pending.iter().min_by_key(|(_, ceremony)| ceremony.started_at).map(|(id, _)| id.clone()) {
                self.pending.remove(&oldest);
            }
        }
        let state_id = Uuid::new_v4().simple().to_string();
        self.pending.insert(
            state_id.clone(),
            Ceremony {
                kind,
                relying_party,
                started_at: Instant::now(),
            },
        );
        state_id
    }

    pub fn take(&mut self, state_id: &str) -> Option<Ceremony> {
        self.purge_expired();
        self.pending.remove(state_id)
    }

    fn purge_expired(&mut self) {
        self.pending.retain(|_, ceremony| ceremony.started_at.elapsed() < CEREMONY_TTL);
    }
}

/// Parses the content of [`PASSKEYS_PATH`]; an empty file holds no passkeys.
pub fn parse_records(content: &[u8]) -> Result<Vec<PasskeyRecord>, serde_json::Error> {
    if content.iter().all(u8::is_ascii_whitespace) {
        return Ok(vec![]);
    }
    serde_json::from_slice(content)
}

/// The name of a new passkey: the given one, trimmed and capped, or `Passkey <n>`.
pub fn passkey_name(name: Option<&str>, existing: usize) -> String {
    match name.map(str::trim).filter(|it| !it.is_empty()) {
        Some(name) => name.chars().take(64).collect(),
        None => format!("Passkey {}", existing + 1),
    }
}

#[cfg(test)]
mod test {
    use webauthn_rs::prelude::Url;

    use super::{parse_records, passkey_name, RelyingParty};

    #[test]
    fn empty_files_hold_no_passkeys() {
        assert!(parse_records(b"").unwrap().is_empty());
        assert!(parse_records(b" \n").unwrap().is_empty());
        assert!(parse_records(b"[]").unwrap().is_empty());
        assert!(parse_records(b"{").is_err());
    }

    #[test]
    fn passkey_names_default_to_a_numbered_name() {
        assert_eq!(passkey_name(None, 0), "Passkey 1");
        assert_eq!(passkey_name(Some("  "), 1), "Passkey 2");
        assert_eq!(passkey_name(Some(" MacBook "), 1), "MacBook");
        assert_eq!(passkey_name(Some(&"x".repeat(100)), 0).len(), 64);
    }

    #[test]
    fn relying_parties_need_a_domain() {
        let localhost = RelyingParty {
            id: "localhost".to_owned(),
            origin: Url::parse("http://localhost:8000").unwrap(),
        };
        assert!(localhost.webauthn().is_ok());
        let ip = RelyingParty {
            id: "127.0.0.1".to_owned(),
            origin: Url::parse("http://127.0.0.1:8000").unwrap(),
        };
        assert!(ip.webauthn().is_err());
        let parent = RelyingParty {
            id: "example.com".to_owned(),
            origin: Url::parse("https://zhang.example.com").unwrap(),
        };
        assert!(parent.webauthn().is_ok());
    }
}
