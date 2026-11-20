//! The login handoff of the mobile app: the app opens the login page in the system browser with
//! `?return_to=<app url>`; once signed in, the page asks for a one-time code
//! (`POST /api/auth/app/code`) and navigates to the app url carrying it, and the app exchanges the
//! code for the session token (`POST /api/auth/app/exchange`), sent as `Authorization: Bearer`.
//!
//! A code is random, can be exchanged once, within [`APP_CODE_TTL`], and is kept in memory (codes
//! are lost when the server restarts). The app url must use one of the allowed schemes, so the
//! handoff cannot redirect the browser to a web site.
//!
//! The app proves it is the one that started the sign-in with PKCE (RFC 7636): it opens the login
//! page with `code_challenge` (the base64url SHA-256 of a random `code_verifier` it keeps) and
//! `code_challenge_method=S256`, the page passes them on to `POST /api/auth/app/code`, and the
//! exchange then needs the `code_verifier`. Another app that registered the same url scheme and
//! intercepts the redirect gets the code but not the verifier. The app's `state` is echoed in the
//! redirect next to the code, so the app can tell the redirect answers its own sign-in.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use log::warn;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use webauthn_rs::prelude::Url;

/// The scheme the zhang app registers for its login callback, always allowed.
pub const DEFAULT_APP_RETURN_SCHEME: &str = "zhang-app";

/// How long a code can be exchanged.
pub const APP_CODE_TTL: Duration = Duration::from_secs(60);

/// Codes are issued to signed-in callers only, but their number is capped all the same.
const MAX_PENDING_CODES: usize = 1024;

/// The query parameter of the app url that carries the code.
const CODE_PARAMETER: &str = "code";

/// The query parameter of the app url that echoes the `state` of the app.
const STATE_PARAMETER: &str = "state";

/// The longest `state` echoed to the app.
pub const MAX_STATE_LENGTH: usize = 512;

/// The only code challenge method: `plain` would put the verifier itself in the login url.
pub const S256: &str = "S256";

/// Schemes of the browser itself: they would run or read something in the browser, or load a web
/// site (an open redirect, handing the code to whoever runs it) instead of opening an app.
const REFUSED_SCHEMES: [&str; 11] = ["javascript", "data", "vbscript", "file", "blob", "about", "http", "https", "ws", "wss", "ftp"];

/// The allowed schemes of the app url: [`DEFAULT_APP_RETURN_SCHEME`] and those of
/// `ZHANG_APP_RETURN_SCHEMES` (comma separated), lower-cased, without duplicates. Values that are
/// no URL scheme, or a scheme of the browser itself (web, websocket and ftp ones included), are
/// ignored with a warning: the server still starts, without them.
pub fn return_schemes(extra: Option<&str>) -> Vec<String> {
    let mut schemes = vec![DEFAULT_APP_RETURN_SCHEME.to_owned()];
    for scheme in extra.unwrap_or_default().split(',').map(str::trim).filter(|it| !it.is_empty()) {
        let scheme = scheme.trim_end_matches("://").trim_end_matches(':').to_ascii_lowercase();
        let valid = scheme.starts_with(|c: char| c.is_ascii_alphabetic()) && scheme.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
        if !valid {
            warn!("ZHANG_APP_RETURN_SCHEMES: `{scheme}` is not a url scheme, it is ignored");
            continue;
        }
        if REFUSED_SCHEMES.contains(&scheme.as_str()) {
            warn!("ZHANG_APP_RETURN_SCHEMES: `{scheme}` is a scheme of the browser, not of an app, it is ignored");
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

/// The PKCE challenge of a code (RFC 7636, `S256`): the SHA-256 of the app's code verifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CodeChallenge([u8; 32]);

impl CodeChallenge {
    /// Whether `verifier` is a code verifier (43 to 128 characters of `A-Z a-z 0-9 - . _ ~`) whose
    /// SHA-256 is this challenge, compared in constant time.
    pub fn is_met_by(&self, verifier: &str) -> bool {
        let well_formed = (43..=128).contains(&verifier.len()) && verifier.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'.' | b'_' | b'~'));
        well_formed && bool::from(Sha256::digest(verifier.as_bytes()).as_slice().ct_eq(&self.0))
    }
}

/// Why the PKCE challenge or the `state` of a handoff is refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HandoffParameterError {
    /// any method but `S256`; without a method RFC 7636 means `plain`
    MethodNotSupported,
    MethodWithoutChallenge,
    InvalidChallenge,
    StateTooLong,
    /// `state` is given and `return_to` carries one already: the app would get two
    ReturnToHasState,
}

impl std::fmt::Display for HandoffParameterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HandoffParameterError::MethodNotSupported => write!(f, "code_challenge_method must be `{S256}`"),
            HandoffParameterError::MethodWithoutChallenge => write!(f, "code_challenge_method needs a code_challenge"),
            HandoffParameterError::InvalidChallenge => write!(f, "code_challenge must be the base64url SHA-256 of the code verifier, without padding"),
            HandoffParameterError::StateTooLong => write!(f, "state must not be longer than {MAX_STATE_LENGTH} bytes"),
            HandoffParameterError::ReturnToHasState => write!(f, "return_to must not carry a `{STATE_PARAMETER}` parameter when `state` is given"),
        }
    }
}

/// The PKCE challenge of `code_challenge` and `code_challenge_method`, `None` without either.
pub fn parse_code_challenge(challenge: Option<&str>, method: Option<&str>) -> Result<Option<CodeChallenge>, HandoffParameterError> {
    let Some(challenge) = challenge else {
        return match method {
            Some(_) => Err(HandoffParameterError::MethodWithoutChallenge),
            None => Ok(None),
        };
    };
    if method != Some(S256) {
        return Err(HandoffParameterError::MethodNotSupported);
    }
    let digest = URL_SAFE_NO_PAD.decode(challenge).map_err(|_| HandoffParameterError::InvalidChallenge)?;
    let digest = <[u8; 32]>::try_from(digest).map_err(|_| HandoffParameterError::InvalidChallenge)?;
    Ok(Some(CodeChallenge(digest)))
}

/// The `state` to echo to `return_to`.
pub fn check_state<'a>(state: Option<&'a str>, return_to: &Url) -> Result<Option<&'a str>, HandoffParameterError> {
    let Some(state) = state else { return Ok(None) };
    if state.len() > MAX_STATE_LENGTH {
        return Err(HandoffParameterError::StateTooLong);
    }
    if return_to.query_pairs().any(|(name, _)| name == STATE_PARAMETER) {
        return Err(HandoffParameterError::ReturnToHasState);
    }
    Ok(Some(state))
}

/// `return_to` with `code=<code>`, then `state=<state>` when given, appended to its query (after
/// the parameters it has).
pub fn redirect_with_code(mut return_to: Url, code: &str, state: Option<&str>) -> String {
    {
        let mut query = return_to.query_pairs_mut();
        query.append_pair(CODE_PARAMETER, code);
        if let Some(state) = state {
            query.append_pair(STATE_PARAMETER, state);
        }
    }
    return_to.to_string()
}

struct PendingCode {
    session: String,
    issued_at: Instant,
    challenge: Option<CodeChallenge>,
}

/// The codes issued and not exchanged yet, each bound to the session token of the caller it was
/// issued to, and to the PKCE challenge of the app when it sent one; each one can be exchanged
/// once, within [`APP_CODE_TTL`].
#[derive(Default)]
pub struct AppCodes {
    pending: HashMap<String, PendingCode>,
}

impl AppCodes {
    /// A new code for `session` and `challenge`: 32 random bytes, base64url without padding.
    pub fn issue(&mut self, session: String, challenge: Option<CodeChallenge>, now: Instant) -> String {
        self.purge_expired(now);
        if self.pending.len() >= MAX_PENDING_CODES {
            if let Some(oldest) = self.pending.iter().min_by_key(|(_, code)| code.issued_at).map(|(code, _)| code.clone()) {
                self.pending.remove(&oldest);
            }
        }
        let code = URL_SAFE_NO_PAD.encode(rand::random::<[u8; 32]>());
        self.pending.insert(
            code.clone(),
            PendingCode {
                session,
                issued_at: now,
                challenge,
            },
        );
        code
    }

    /// The session token `code` was issued for, when it is pending, not expired at `now`, and
    /// `verifier` meets its challenge: a code issued with a challenge needs the verifier, and one
    /// issued without refuses a verifier (a PKCE downgrade, RFC 9700 section 2.1.1). The code is
    /// used up either way, so a wrong verifier cannot be retried, and the answer does not tell a
    /// wrong verifier from an unknown code.
    pub fn take(&mut self, code: &str, verifier: Option<&str>, now: Instant) -> Option<String> {
        self.purge_expired(now);
        let pending = self.pending.remove(code)?;
        let verified = match (&pending.challenge, verifier) {
            (None, None) => true,
            (Some(challenge), Some(verifier)) => challenge.is_met_by(verifier),
            (Some(_), None) | (None, Some(_)) => false,
        };
        verified.then_some(pending.session)
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
    use sha2::{Digest, Sha256};

    use super::{
        check_state, parse_code_challenge, parse_return_to, redirect_with_code, return_schemes, AppCodes, CodeChallenge, HandoffParameterError, ReturnToError,
        APP_CODE_TTL, MAX_PENDING_CODES, MAX_STATE_LENGTH,
    };

    /// The example of RFC 7636, appendix B.
    const VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
    const CHALLENGE: &str = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";

    fn challenge() -> CodeChallenge {
        parse_code_challenge(Some(CHALLENGE), Some("S256")).unwrap().unwrap()
    }

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
        // web, websocket and ftp schemes would hand the code to a web site
        assert_eq!(
            return_schemes(Some("http, HTTPS://, ws, wss:, ftp, my-app, about, blob, file")),
            vec!["zhang-app", "my-app"]
        );
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
        let redirect = |url: &str| redirect_with_code(parse_return_to(url, &schemes).unwrap(), "abc-_1", None);
        assert_eq!(redirect("zhang-app://auth/callback"), "zhang-app://auth/callback?code=abc-_1");
        assert_eq!(redirect("zhang-app://auth/callback?x=1"), "zhang-app://auth/callback?x=1&code=abc-_1");
        assert_eq!(redirect("zhang-app://auth/callback?"), "zhang-app://auth/callback?code=abc-_1");
        assert_eq!(redirect("zhang-app://auth/callback?x=1#frag"), "zhang-app://auth/callback?x=1&code=abc-_1#frag");
    }

    #[test]
    fn the_state_is_echoed_after_the_code() {
        let schemes = return_schemes(None);
        let return_to = parse_return_to("zhang-app://auth/callback?x=1", &schemes).unwrap();
        assert_eq!(
            redirect_with_code(return_to.clone(), "abc", Some("s-1")),
            "zhang-app://auth/callback?x=1&code=abc&state=s-1"
        );
        // encoded like any query value
        assert_eq!(
            redirect_with_code(return_to, "abc", Some("a b&c=d")),
            "zhang-app://auth/callback?x=1&code=abc&state=a+b%26c%3Dd"
        );
    }

    #[test]
    fn the_state_is_short_and_not_planted_in_return_to() {
        let schemes = return_schemes(None);
        let plain = parse_return_to("zhang-app://auth/callback", &schemes).unwrap();
        let with_state = parse_return_to("zhang-app://auth/callback?state=planted", &schemes).unwrap();
        assert_eq!(check_state(None, &plain), Ok(None));
        assert_eq!(check_state(Some("abc"), &plain), Ok(Some("abc")));
        let longest = "s".repeat(MAX_STATE_LENGTH);
        assert_eq!(check_state(Some(&longest), &plain), Ok(Some(longest.as_str())));
        assert_eq!(check_state(Some(&format!("{longest}s")), &plain), Err(HandoffParameterError::StateTooLong));
        assert_eq!(check_state(Some("abc"), &with_state), Err(HandoffParameterError::ReturnToHasState));
        // a return_to of its own state is kept as it is when the app sends none
        assert_eq!(check_state(None, &with_state), Ok(None));
    }

    #[test]
    fn only_s256_challenges_are_accepted() {
        assert_eq!(parse_code_challenge(None, None), Ok(None));
        assert!(parse_code_challenge(Some(CHALLENGE), Some("S256")).unwrap().is_some());
        for (challenge, method, error) in [
            (Some(CHALLENGE), Some("plain"), HandoffParameterError::MethodNotSupported),
            // without a method RFC 7636 means plain
            (Some(CHALLENGE), None, HandoffParameterError::MethodNotSupported),
            (Some(CHALLENGE), Some("s256"), HandoffParameterError::MethodNotSupported),
            (Some(CHALLENGE), Some("S512"), HandoffParameterError::MethodNotSupported),
            (None, Some("S256"), HandoffParameterError::MethodWithoutChallenge),
            // not 32 bytes
            (
                Some("E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-c"),
                Some("S256"),
                HandoffParameterError::InvalidChallenge,
            ),
            (Some("abc"), Some("S256"), HandoffParameterError::InvalidChallenge),
            (
                Some("E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM="),
                Some("S256"),
                HandoffParameterError::InvalidChallenge,
            ),
            (
                Some("E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw+cM"),
                Some("S256"),
                HandoffParameterError::InvalidChallenge,
            ),
            (
                Some("E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw"),
                Some("S256"),
                HandoffParameterError::InvalidChallenge,
            ),
        ] {
            assert_eq!(parse_code_challenge(challenge, method), Err(error), "{challenge:?} {method:?}");
        }
    }

    #[test]
    fn a_challenge_is_met_by_its_verifier_only() {
        let challenge = challenge();
        assert!(challenge.is_met_by(VERIFIER));
        assert!(!challenge.is_met_by("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXK"));
        assert!(!challenge.is_met_by(CHALLENGE));
        assert!(!challenge.is_met_by(""));

        // 43 to 128 characters of the unreserved set
        let verifier = |v: &str| {
            parse_code_challenge(Some(&URL_SAFE_NO_PAD.encode(Sha256::digest(v.as_bytes()))), Some("S256"))
                .unwrap()
                .unwrap()
        };
        for good in ["a".repeat(43), "~._-".repeat(32), "Az09".repeat(20)] {
            assert!(verifier(&good).is_met_by(&good), "{good}");
        }
        for bad in [
            "a".repeat(42),
            "a".repeat(129),
            format!("{}+", "a".repeat(42)),
            format!("{} ", "a".repeat(43)),
            format!("{}é", "a".repeat(42)),
        ] {
            assert!(!verifier(&bad).is_met_by(&bad), "{bad}");
        }
    }

    #[test]
    fn codes_are_random_and_can_be_exchanged_once() {
        let now = Instant::now();
        let mut codes = AppCodes::default();
        let code = codes.issue("token-a".to_owned(), None, now);
        let other = codes.issue("token-b".to_owned(), None, now);
        assert_ne!(code, other);
        assert_eq!(URL_SAFE_NO_PAD.decode(&code).unwrap().len(), 32);
        assert_eq!(codes.take("unknown", None, now), None);
        assert_eq!(codes.take(&code, None, now).as_deref(), Some("token-a"));
        assert_eq!(codes.take(&code, None, now), None, "a code is used once");
        assert_eq!(codes.take(&other, None, now).as_deref(), Some("token-b"));
    }

    #[test]
    fn a_code_of_a_challenge_needs_its_verifier_and_is_used_up_by_a_wrong_one() {
        let now = Instant::now();
        let mut codes = AppCodes::default();
        let code = codes.issue("token".to_owned(), Some(challenge()), now);
        assert_eq!(codes.take(&code, Some(VERIFIER), now).as_deref(), Some("token"));
        assert_eq!(codes.take(&code, Some(VERIFIER), now), None, "a code is used once");

        for wrong in [None, Some(""), Some("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXK"), Some(CHALLENGE)] {
            let code = codes.issue("token".to_owned(), Some(challenge()), now);
            assert_eq!(codes.take(&code, wrong, now), None, "{wrong:?}");
            assert_eq!(codes.take(&code, Some(VERIFIER), now), None, "{wrong:?} used the code up");
        }
    }

    #[test]
    fn a_code_without_a_challenge_refuses_a_verifier() {
        let now = Instant::now();
        let mut codes = AppCodes::default();
        let code = codes.issue("token".to_owned(), None, now);
        assert_eq!(codes.take(&code, Some(VERIFIER), now), None);
        assert_eq!(codes.take(&code, None, now), None, "and is used up");
    }

    #[test]
    fn codes_expire() {
        let now = Instant::now();
        let mut codes = AppCodes::default();
        let code = codes.issue("token".to_owned(), Some(challenge()), now);
        assert_eq!(codes.take(&code, Some(VERIFIER), now + APP_CODE_TTL), None);

        let code = codes.issue("token".to_owned(), Some(challenge()), now);
        assert_eq!(
            codes.take(&code, Some(VERIFIER), now + APP_CODE_TTL - Duration::from_millis(1)).as_deref(),
            Some("token")
        );
    }

    #[test]
    fn the_oldest_code_makes_room_for_a_new_one() {
        let start = Instant::now();
        let mut codes = AppCodes::default();
        let oldest = codes.issue("oldest".to_owned(), None, start);
        for _ in 1..MAX_PENDING_CODES {
            codes.issue("token".to_owned(), None, start + Duration::from_millis(1));
        }
        let newest = codes.issue("newest".to_owned(), None, start + Duration::from_millis(2));
        assert_eq!(codes.pending.len(), MAX_PENDING_CODES);
        assert_eq!(codes.take(&oldest, None, start + Duration::from_millis(2)), None);
        assert_eq!(codes.take(&newest, None, start + Duration::from_millis(2)).as_deref(), Some("newest"));
    }
}
