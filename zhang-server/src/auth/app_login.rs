//! The login handoff of the mobile app: the app opens the login page in the system browser with
//! `?return_to=<app url>`; once signed in, the page asks for a one-time code
//! (`POST /api/auth/app/code`) and navigates to the app url carrying it, and the app exchanges the
//! code for the session token (`POST /api/auth/app/exchange`), sent as `Authorization: Bearer`.
//!
//! A code is random, can be exchanged once, within [`APP_CODE_TTL`], and is kept in memory (codes
//! are lost when the server restarts). The app url must use one of the allowed schemes, so the
//! handoff cannot redirect the browser to a web site.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use log::warn;
use webauthn_rs::prelude::Url;

/// The scheme the zhang app registers for its login callback, always allowed.
pub const DEFAULT_APP_RETURN_SCHEME: &str = "zhang-app";

/// How long a code can be exchanged.
pub const APP_CODE_TTL: Duration = Duration::from_secs(60);

/// Codes are issued to signed-in callers only, but their number is capped all the same.
const MAX_PENDING_CODES: usize = 1024;

/// The query parameter of the app url that carries the code.
const CODE_PARAMETER: &str = "code";

/// Schemes that would run or read something in the browser instead of opening an app.
const REFUSED_SCHEMES: [&str; 6] = ["javascript", "data", "vbscript", "file", "blob", "about"];

/// The allowed schemes of the app url: [`DEFAULT_APP_RETURN_SCHEME`] and those of
/// `ZHANG_APP_RETURN_SCHEMES` (comma separated), lower-cased, without duplicates. Values that are
/// no URL scheme, or a scheme of the browser itself, are ignored with a warning.
pub fn return_schemes(extra: Option<&str>) -> Vec<String> {
    let mut schemes = vec![DEFAULT_APP_RETURN_SCHEME.to_owned()];
    for scheme in extra.unwrap_or_default().split(',').map(str::trim).filter(|it| !it.is_empty()) {
        let scheme = scheme.trim_end_matches("://").trim_end_matches(':').to_ascii_lowercase();
        let valid = scheme.starts_with(|c: char| c.is_ascii_alphabetic()) && scheme.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
        if !valid || REFUSED_SCHEMES.contains(&scheme.as_str()) {
            warn!("ZHANG_APP_RETURN_SCHEMES: `{scheme}` is not an app url scheme, it is ignored");
            continue;
        }
        if !schemes.contains(&scheme) {
            schemes.push(scheme);
        }
    }
    schemes
}

/// Why an app url is refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReturnToError {
    Invalid,
    SchemeNotAllowed(String),
    HasCode,
}

impl std::fmt::Display for ReturnToError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReturnToError::Invalid => write!(f, "return_to is not a valid url"),
            ReturnToError::SchemeNotAllowed(scheme) => write!(f, "the scheme `{scheme}` of return_to is not allowed, allow it with ZHANG_APP_RETURN_SCHEMES"),
            ReturnToError::HasCode => write!(f, "return_to must not carry a `{CODE_PARAMETER}` parameter"),
        }
    }
}

/// The app url `return_to`, checked against the allowed `schemes`.
pub fn parse_return_to(return_to: &str, schemes: &[String]) -> Result<Url, ReturnToError> {
    let url = Url::parse(return_to.trim()).map_err(|_| ReturnToError::Invalid)?;
    // the url crate lower-cases the scheme
    if !schemes.iter().any(|scheme| scheme == url.scheme()) {
        return Err(ReturnToError::SchemeNotAllowed(url.scheme().to_owned()));
    }
    if url.query_pairs().any(|(name, _)| name == CODE_PARAMETER) {
        return Err(ReturnToError::HasCode);
    }
    Ok(url)
}

/// `return_to` with `code=<code>` appended to its query (after the parameters it has).
pub fn redirect_with_code(mut return_to: Url, code: &str) -> String {
    return_to.query_pairs_mut().append_pair(CODE_PARAMETER, code);
    return_to.to_string()
}

struct PendingCode {
    session: String,
    issued_at: Instant,
}

/// The codes issued and not exchanged yet, each bound to the session token of the caller it was
/// issued to; each one can be exchanged once, within [`APP_CODE_TTL`].
#[derive(Default)]
pub struct AppCodes {
    pending: HashMap<String, PendingCode>,
}

impl AppCodes {
    /// A new code for `session`: 32 random bytes, base64url without padding.
    pub fn issue(&mut self, session: String, now: Instant) -> String {
        self.purge_expired(now);
        if self.pending.len() >= MAX_PENDING_CODES {
            if let Some(oldest) = self.pending.iter().min_by_key(|(_, code)| code.issued_at).map(|(code, _)| code.clone()) {
                self.pending.remove(&oldest);
            }
        }
        let code = URL_SAFE_NO_PAD.encode(rand::random::<[u8; 32]>());
        self.pending.insert(code.clone(), PendingCode { session, issued_at: now });
        code
    }

    /// The session token `code` was issued for, when it is pending and not expired at `now`; the
    /// code is used up either way.
    pub fn take(&mut self, code: &str, now: Instant) -> Option<String> {
        self.purge_expired(now);
        self.pending.remove(code).map(|code| code.session)
    }

    fn purge_expired(&mut self, now: Instant) {
        self.pending.retain(|_, code| now.saturating_duration_since(code.issued_at) < APP_CODE_TTL);
    }
}

#[cfg(test)]
mod test {
    use std::time::{Duration, Instant};

    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine as _;

    use super::{parse_return_to, redirect_with_code, return_schemes, AppCodes, ReturnToError, APP_CODE_TTL, MAX_PENDING_CODES};

    #[test]
    fn the_app_scheme_is_always_allowed_and_more_can_be_added() {
        assert_eq!(return_schemes(None), vec!["zhang-app"]);
        assert_eq!(return_schemes(Some("")), vec!["zhang-app"]);
        assert_eq!(
            return_schemes(Some(" My-App , zhang-app,com.example.zhang://, other: ")),
            vec!["zhang-app", "my-app", "com.example.zhang", "other"]
        );
        // no scheme, or one of the browser itself
        assert_eq!(return_schemes(Some("javascript, DATA, 1abc, a b, ftp/x")), vec!["zhang-app"]);
    }

    #[test]
    fn only_urls_of_an_allowed_scheme_are_returned_to() {
        let schemes = return_schemes(Some("my-app"));
        assert!(parse_return_to("zhang-app://auth/callback", &schemes).is_ok());
        assert!(parse_return_to("ZHANG-APP://auth/callback", &schemes).is_ok());
        assert!(parse_return_to("my-app:/callback", &schemes).is_ok());
        assert_eq!(
            parse_return_to("https://evil.example.com/?x=1", &schemes),
            Err(ReturnToError::SchemeNotAllowed("https".to_owned()))
        );
        assert_eq!(
            parse_return_to("javascript:alert(1)", &schemes),
            Err(ReturnToError::SchemeNotAllowed("javascript".to_owned()))
        );
        assert_eq!(parse_return_to("/relative/path", &schemes), Err(ReturnToError::Invalid));
        assert_eq!(parse_return_to("", &schemes), Err(ReturnToError::Invalid));
        assert_eq!(parse_return_to("zhang-app://auth/callback?code=planted", &schemes), Err(ReturnToError::HasCode));
    }

    #[test]
    fn the_code_is_appended_to_the_query() {
        let schemes = return_schemes(None);
        let redirect = |url: &str| redirect_with_code(parse_return_to(url, &schemes).unwrap(), "abc-_1");
        assert_eq!(redirect("zhang-app://auth/callback"), "zhang-app://auth/callback?code=abc-_1");
        assert_eq!(redirect("zhang-app://auth/callback?x=1"), "zhang-app://auth/callback?x=1&code=abc-_1");
        assert_eq!(redirect("zhang-app://auth/callback?"), "zhang-app://auth/callback?code=abc-_1");
        assert_eq!(redirect("zhang-app://auth/callback?x=1#frag"), "zhang-app://auth/callback?x=1&code=abc-_1#frag");
    }

    #[test]
    fn codes_are_random_and_can_be_exchanged_once() {
        let now = Instant::now();
        let mut codes = AppCodes::default();
        let code = codes.issue("token-a".to_owned(), now);
        let other = codes.issue("token-b".to_owned(), now);
        assert_ne!(code, other);
        assert_eq!(URL_SAFE_NO_PAD.decode(&code).unwrap().len(), 32);
        assert_eq!(codes.take("unknown", now), None);
        assert_eq!(codes.take(&code, now).as_deref(), Some("token-a"));
        assert_eq!(codes.take(&code, now), None, "a code is used once");
        assert_eq!(codes.take(&other, now).as_deref(), Some("token-b"));
    }

    #[test]
    fn codes_expire() {
        let now = Instant::now();
        let mut codes = AppCodes::default();
        let code = codes.issue("token".to_owned(), now);
        assert_eq!(codes.take(&code, now + APP_CODE_TTL), None);

        let code = codes.issue("token".to_owned(), now);
        assert_eq!(codes.take(&code, now + APP_CODE_TTL - Duration::from_millis(1)).as_deref(), Some("token"));
    }

    #[test]
    fn the_oldest_code_makes_room_for_a_new_one() {
        let start = Instant::now();
        let mut codes = AppCodes::default();
        let oldest = codes.issue("oldest".to_owned(), start);
        for _ in 1..MAX_PENDING_CODES {
            codes.issue("token".to_owned(), start + Duration::from_millis(1));
        }
        let newest = codes.issue("newest".to_owned(), start + Duration::from_millis(2));
        assert_eq!(codes.pending.len(), MAX_PENDING_CODES);
        assert_eq!(codes.take(&oldest, start + Duration::from_millis(2)), None);
        assert_eq!(codes.take(&newest, start + Duration::from_millis(2)).as_deref(), Some("newest"));
    }
}
