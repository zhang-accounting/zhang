//! Session based authentication of the web UI and the API.
//!
//! Two methods can be enabled side by side:
//! - **password**: `--auth user:pass` / `ZHANG_AUTH`, entered on the login page of the frontend
//!   and exchanged for a session; the `Authorization: Basic` header keeps working for scripts;
//! - **passkey**: `--passkey <secret>` / `ZHANG_PASSKEY`, WebAuthn credentials stored in
//!   [`PASSKEYS_PATH`] of the data source; the secret allows registering a passkey without a session.
//!
//! A login sets the [`SESSION_COOKIE`] cookie, an HMAC-SHA256 signed token carrying the subject
//! and the expiry. When authentication is enabled every `/api/*` route but `/api/auth/*` requires
//! a session (or the Basic header), answering `401 {"message": "unauthorized"}` otherwise. There is
//! no `WWW-Authenticate` header, so browsers show the login page instead of their credentials popup.
//!
//! The session token is accepted as `Authorization: Bearer <token>` too: the mobile app signs in
//! through the login page in the system browser and receives the token of that session through a
//! one-time code, see [`app_login`].

pub mod app_login;
pub mod limiter;
pub mod passkey;
pub mod session;

use std::net::SocketAddr;
use std::ops::Deref;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use axum::extract::{ConnectInfo, Request, State};
use axum::http::{header, HeaderMap, HeaderValue, Method};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;
use chrono::Utc;
use gotcha::oas::Responses;
use gotcha::{Responsible, Schematic};
use log::{error, info, warn};
use serde::Serialize;
use tokio::sync::RwLock;
use webauthn_rs::prelude::{AuthenticationResult, Passkey, Url};
use zhang_core::ledger::Ledger;
use zhang_core::{ZhangError, ZhangResult};

use self::app_login::{AppCodes, ReturnToError};
use self::limiter::FailureLimiter;
use self::passkey::{Ceremonies, Ceremony, CeremonyKind, RelyingParty};
pub use self::passkey::{PasskeyRecord, PASSKEYS_PATH, STATE_DIR};
use self::session::{SessionClaims, SessionKey};
use crate::error::ServerError;
use crate::response::{AppCodeEntity, AppTokenEntity, AuthMethodsEntity, AuthStatusEntity, PasskeyEntity, ResponseWrapper};
use crate::{ServeConfig, ServerResult};

/// The name of the session cookie.
pub const SESSION_COOKIE: &str = "zhang_session";

/// How long a session lasts.
pub const SESSION_TTL_SECONDS: i64 = 30 * 24 * 60 * 60;

/// The subject of passkey sessions when no password user is configured.
const PASSKEY_SUBJECT: &str = "owner";

/// The `ZHANG_AUTH` credential.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PasswordCredential {
    pub username: String,
    pub password: String,
}

impl PasswordCredential {
    /// Parses `user:pass`; the password may contain colons.
    pub fn parse(credential: &str) -> Self {
        let (username, password) = credential.split_once(':').unwrap_or((credential, ""));
        PasswordCredential {
            username: username.to_owned(),
            password: password.to_owned(),
        }
    }
}

/// The authentication settings of the server.
#[derive(Debug, Clone, Default)]
pub struct AuthConfig {
    /// enables the password method
    pub password: Option<PasswordCredential>,
    /// enables the passkey method; the secret that allows registering a passkey without a session
    pub passkey_secret: Option<String>,
    /// overrides the WebAuthn relying party id (by default the host of the request, or of `passkey_origin`)
    pub passkey_rp_id: Option<String>,
    /// overrides the origin the browser reports (by default derived from the request)
    pub passkey_origin: Option<String>,
    /// the key that signs the sessions; random (sessions end on restart) when absent
    pub session_secret: Option<String>,
    /// more schemes the app login handoff may return to (comma separated), next to `zhang-app`
    pub app_return_schemes: Option<String>,
}

impl AuthConfig {
    pub fn from_serve_config(opts: &ServeConfig) -> Self {
        fn non_empty(value: &Option<String>) -> Option<String> {
            value.as_ref().map(|it| it.trim().to_owned()).filter(|it| !it.is_empty())
        }
        AuthConfig {
            password: opts.auth_credential.as_deref().map(PasswordCredential::parse),
            passkey_secret: opts.passkey_secret.clone().filter(|it| !it.is_empty()),
            passkey_rp_id: non_empty(&opts.passkey_rp_id),
            passkey_origin: non_empty(&opts.passkey_origin),
            session_secret: opts.session_secret.clone().filter(|it| !it.is_empty()),
            app_return_schemes: non_empty(&opts.app_return_schemes),
        }
    }
}

/// Who a request is authenticated as.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Principal {
    pub user: String,
}

/// the answer to a request without a valid credential or session
fn unauthorized() -> ServerError {
    ServerError::Unauthorized("unauthorized".to_owned())
}

fn passkey_disabled() -> ServerError {
    ServerError::InvalidInput("passkey login is not enabled".to_owned())
}

fn expired_ceremony() -> ServerError {
    ServerError::InvalidInput("the passkey request is unknown or expired, please try again".to_owned())
}

fn invalid_app_code() -> ServerError {
    ServerError::Unauthorized("invalid or expired code".to_owned())
}

/// A JSON response that may also set (or clear) the session cookie.
pub struct WithSessionCookie<T: Serialize + Schematic> {
    cookie: Option<HeaderValue>,
    body: ResponseWrapper<T>,
}

impl<T: Serialize + Schematic> IntoResponse for WithSessionCookie<T> {
    fn into_response(self) -> Response {
        match self.cookie {
            Some(cookie) => ([(header::SET_COOKIE, cookie)], self.body).into_response(),
            None => self.body.into_response(),
        }
    }
}

impl<T: Serialize + Schematic> Responsible for WithSessionCookie<T> {
    fn response() -> Responses {
        <ResponseWrapper<T> as Responsible>::response()
    }
}

/// The scheme and host (with the port) a request was sent to, as seen by the browser: the
/// `X-Forwarded-Proto` / `X-Forwarded-Host` headers of a reverse proxy win over `Host`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct RequestSite {
    scheme: String,
    host: String,
}

/// The value of an `X-Forwarded-*` header as the reverse proxy in front of zhang set it: a proxy
/// appends what it saw, so it is the rightmost entry (the ones before it are whatever the client
/// sent). Several headers are read in order, as one list.
pub(crate) fn forwarded_value<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get_all(name)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .map(str::trim)
        .rfind(|value| !value.is_empty())
}

impl RequestSite {
    fn from_headers(headers: &HeaderMap) -> Option<Self> {
        let host = forwarded_value(headers, "x-forwarded-host").or_else(|| {
            headers
                .get(header::HOST)
                .and_then(|value| value.to_str().ok())
                .map(str::trim)
                .filter(|value| !value.is_empty())
        })?;
        Some(RequestSite {
            scheme: request_scheme(headers),
            host: host.to_owned(),
        })
    }
}

fn request_scheme(headers: &HeaderMap) -> String {
    forwarded_value(headers, "x-forwarded-proto")
        .map(str::to_ascii_lowercase)
        .unwrap_or_else(|| "http".to_owned())
}

/// The value of the session cookie: `Secure` when the browser talks https.
fn session_cookie(headers: &HeaderMap, token: Option<&str>) -> HeaderValue {
    let mut cookie = match token {
        Some(token) => format!("{SESSION_COOKIE}={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age={SESSION_TTL_SECONDS}"),
        None => format!("{SESSION_COOKIE}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0"),
    };
    if request_scheme(headers) == "https" {
        cookie.push_str("; Secure");
    }
    HeaderValue::from_str(&cookie).expect("session cookies are valid header values")
}

/// The values of the session cookie in the `Cookie` headers.
fn session_cookie_values(headers: &HeaderMap) -> impl Iterator<Item = &str> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .filter(|(name, _)| *name == SESSION_COOKIE)
        .map(|(_, value)| value.trim())
}

/// The token of an `Authorization: Bearer` header.
fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = value.trim().split_once(' ')?;
    let token = token.trim();
    (scheme.eq_ignore_ascii_case("bearer") && !token.is_empty()).then_some(token)
}

/// The session tokens a request carries: those of its session cookies, then its Bearer token.
fn session_tokens(headers: &HeaderMap) -> impl Iterator<Item = &str> {
    session_cookie_values(headers).chain(bearer_token(headers))
}

/// Whether a request has to be authenticated: the API, but not its auth endpoints, nor the CORS
/// preflights. The frontend (static assets and the SPA fallback) never is.
pub fn requires_authentication(method: &Method, path: &str) -> bool {
    path.starts_with("/api/") && !path.starts_with("/api/auth/") && method != Method::OPTIONS
}

/// The authentication state shared by the middleware and the auth endpoints.
pub struct AuthState {
    config: AuthConfig,
    key: SessionKey,
    ledger: Arc<RwLock<Ledger>>,
    passkeys: RwLock<Vec<PasskeyRecord>>,
    ceremonies: Mutex<Ceremonies>,
    failures: Mutex<FailureLimiter>,
    /// the schemes the app login handoff may return to
    app_return_schemes: Vec<String>,
    app_codes: Mutex<AppCodes>,
}

#[derive(Clone)]
pub struct SharedAuth(pub Arc<AuthState>);

impl Deref for SharedAuth {
    type Target = Arc<AuthState>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl AuthState {
    pub fn new(config: AuthConfig, ledger: Arc<RwLock<Ledger>>) -> Self {
        if let Some(origin) = &config.passkey_origin {
            if Url::parse(origin).is_err() {
                error!("ZHANG_PASSKEY_ORIGIN `{origin}` is not a valid URL, passkeys will not work");
            }
        }
        AuthState {
            key: SessionKey::new(config.session_secret.as_deref()),
            app_return_schemes: app_login::return_schemes(config.app_return_schemes.as_deref()),
            app_codes: Mutex::new(AppCodes::default()),
            config,
            ledger,
            passkeys: RwLock::new(vec![]),
            ceremonies: Mutex::new(Ceremonies::default()),
            failures: Mutex::new(FailureLimiter::default()),
        }
    }

    pub fn enabled(&self) -> bool {
        self.password_enabled() || self.passkey_enabled()
    }

    pub fn password_enabled(&self) -> bool {
        self.config.password.is_some()
    }

    pub fn passkey_enabled(&self) -> bool {
        self.config.passkey_secret.is_some()
    }

    pub fn has_session_secret(&self) -> bool {
        self.config.session_secret.is_some()
    }

    /// Reads the registered passkeys from the data source; a missing file holds none.
    pub async fn load_passkeys(&self) -> ZhangResult<usize> {
        if !self.passkey_enabled() {
            return Ok(0);
        }
        let content = {
            let ledger = self.ledger.read().await;
            ledger.data_source.get(PASSKEYS_PATH.to_owned())
        };
        let content = match content {
            Ok(content) => content,
            Err(e) if e.is_file_not_found() => vec![],
            Err(e) => return Err(e),
        };
        let records = passkey::parse_records(&content).map_err(|e| ZhangError::CustomError(format!("cannot read {PASSKEYS_PATH}: {e}")))?;
        let count = records.len();
        *self.passkeys.write().await = records;
        Ok(count)
    }

    async fn save_passkeys(&self, records: &[PasskeyRecord]) -> ServerResult<()> {
        let content = serde_json::to_vec_pretty(records).map_err(|e| ServerError::Internal(format!("cannot serialize the passkeys: {e}")))?;
        let ledger = self.ledger.read().await;
        ledger.data_source.save(&ledger, PASSKEYS_PATH.to_owned(), &content).map_err(|e| {
            error!("cannot save {PASSKEYS_PATH}: {e}");
            ServerError::Internal(format!("cannot save the passkeys: {e}"))
        })
    }

    fn password_fingerprint(&self, credential: &PasswordCredential) -> String {
        self.key
            .fingerprint(format!("password\0{}\0{}", credential.username, credential.password).as_bytes())
    }

    /// Whether `username` / `password` are the configured credential.
    pub fn verify_password(&self, username: &str, password: &str) -> bool {
        match &self.config.password {
            Some(credential) => {
                // compare both, without short-circuiting
                let username_matches = self.key.secrets_equal(username.as_bytes(), credential.username.as_bytes());
                let password_matches = self.key.secrets_equal(password.as_bytes(), credential.password.as_bytes());
                username_matches & password_matches
            }
            None => false,
        }
    }

    fn verify_passkey_secret(&self, secret: &str) -> bool {
        match &self.config.passkey_secret {
            Some(expected) => self.key.secrets_equal(secret.as_bytes(), expected.as_bytes()),
            None => false,
        }
    }

    fn passkey_subject(&self) -> String {
        match &self.config.password {
            Some(credential) => credential.username.clone(),
            None => PASSKEY_SUBJECT.to_owned(),
        }
    }

    fn issue_session(&self, claims: SessionClaims) -> String {
        self.key.sign(&claims)
    }

    fn password_session(&self) -> Option<(Principal, String)> {
        let credential = self.config.password.as_ref()?;
        let token = self.issue_session(SessionClaims {
            sub: credential.username.clone(),
            exp: Utc::now().timestamp() + SESSION_TTL_SECONDS,
            pw: Some(self.password_fingerprint(credential)),
            pk: None,
        });
        Some((
            Principal {
                user: credential.username.clone(),
            },
            token,
        ))
    }

    fn passkey_session(&self, passkey_id: &str) -> (Principal, String) {
        let subject = self.passkey_subject();
        let token = self.issue_session(SessionClaims {
            sub: subject.clone(),
            exp: Utc::now().timestamp() + SESSION_TTL_SECONDS,
            pw: None,
            pk: Some(passkey_id.to_owned()),
        });
        (Principal { user: subject }, token)
    }

    /// The claims of a session token that is signed, not expired, and still backed by the
    /// configuration: the same password credential, or a passkey that is still registered.
    async fn session_claims(&self, token: &str) -> Option<SessionClaims> {
        let claims = self.key.verify(token, Utc::now().timestamp())?;
        let valid = match (&claims.pw, &claims.pk) {
            (Some(fingerprint), None) => self
                .config
                .password
                .as_ref()
                .is_some_and(|credential| self.password_fingerprint(credential) == *fingerprint),
            (None, Some(passkey_id)) => self.passkey_enabled() && self.passkeys.read().await.iter().any(|record| record.id == *passkey_id),
            _ => false,
        };
        valid.then_some(claims)
    }

    /// The session (its principal and token) of the first valid session cookie or Bearer token.
    async fn session(&self, headers: &HeaderMap) -> Option<(Principal, String)> {
        for token in session_tokens(headers) {
            if let Some(claims) = self.session_claims(token).await {
                return Some((Principal { user: claims.sub }, token.to_owned()));
            }
        }
        None
    }

    /// The principal of an `Authorization: Basic` header, accepted when the password method is enabled.
    fn basic_principal(&self, headers: &HeaderMap) -> Option<Principal> {
        let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
        let (scheme, encoded) = value.trim().split_once(' ')?;
        if !scheme.eq_ignore_ascii_case("basic") {
            return None;
        }
        let decoded = String::from_utf8(BASE64_STANDARD.decode(encoded.trim()).ok()?).ok()?;
        let (username, password) = decoded.split_once(':')?;
        self.verify_password(username, password).then(|| Principal { user: username.to_owned() })
    }

    /// Who the request is authenticated as, by its session cookie, its Bearer token or its Basic header.
    pub async fn authenticate(&self, headers: &HeaderMap) -> Option<Principal> {
        match self.session(headers).await {
            Some((principal, _)) => Some(principal),
            None => self.basic_principal(headers),
        }
    }

    async fn require_principal(&self, headers: &HeaderMap) -> ServerResult<Principal> {
        self.authenticate(headers).await.ok_or_else(unauthorized)
    }

    fn ensure_passkey_enabled(&self) -> ServerResult<()> {
        if self.passkey_enabled() {
            Ok(())
        } else {
            Err(passkey_disabled())
        }
    }

    async fn ledger_title(&self) -> Option<String> {
        let ledger = self.ledger.read().await;
        ledger.options.option::<String>("title").ok().flatten()
    }

    /// The `GET /api/auth/status` body for `principal`.
    pub async fn status(&self, principal: Option<&Principal>) -> AuthStatusEntity {
        let title = self.ledger_title().await;
        let enabled = self.enabled();
        AuthStatusEntity {
            enabled,
            authenticated: !enabled || principal.is_some(),
            methods: AuthMethodsEntity {
                password: self.password_enabled(),
                passkey: self.passkey_enabled(),
            },
            passkey_registered: !self.passkeys.read().await.is_empty(),
            user: principal.filter(|_| enabled).map(|it| it.user.clone()),
            title,
            app_login: true,
            app_return_schemes: self.app_return_schemes.clone(),
        }
    }

    async fn status_with_cookie(&self, headers: &HeaderMap, session: Option<(Principal, String)>) -> WithSessionCookie<AuthStatusEntity> {
        let principal = session.as_ref().map(|(principal, _)| principal);
        let body = ResponseWrapper {
            data: self.status(principal).await,
        };
        WithSessionCookie {
            cookie: Some(session_cookie(headers, session.as_ref().map(|(_, token)| token.as_str()))),
            body,
        }
    }

    /// The status of `principal`, leaving its session (cookie) as it is.
    async fn status_keeping_session(&self, principal: &Principal) -> WithSessionCookie<AuthStatusEntity> {
        WithSessionCookie {
            cookie: None,
            body: ResponseWrapper {
                data: self.status(Some(principal)).await,
            },
        }
    }

    /// The relying party of a passkey ceremony started by a request.
    fn relying_party(&self, headers: &HeaderMap) -> ServerResult<RelyingParty> {
        let origin = match &self.config.passkey_origin {
            Some(origin) => Url::parse(origin).map_err(|_| ServerError::Internal(format!("ZHANG_PASSKEY_ORIGIN `{origin}` is not a valid URL")))?,
            None => {
                let site = RequestSite::from_headers(headers).ok_or_else(|| ServerError::InvalidInput("the request has no Host header".to_owned()))?;
                Url::parse(&format!("{}://{}", site.scheme, site.host))
                    .map_err(|_| ServerError::InvalidInput(format!("`{}://{}` is not a valid origin", site.scheme, site.host)))?
            }
        };
        let id = match &self.config.passkey_rp_id {
            Some(id) => id.clone(),
            None => origin
                .host_str()
                .map(|host| host.to_owned())
                .ok_or_else(|| ServerError::InvalidInput(format!("`{origin}` has no host")))?,
        };
        Ok(RelyingParty { id, origin })
    }

    fn start_ceremony(&self, kind: CeremonyKind, relying_party: RelyingParty) -> String {
        self.ceremonies.lock().expect("ceremonies lock is poisoned").start(kind, relying_party)
    }

    /// The client a sign-in attempt comes from, see [`limiter::client_key`].
    fn client(&self, headers: &HeaderMap, peer: Option<ConnectInfo<SocketAddr>>) -> String {
        limiter::client_key(headers, peer.map(|it| it.0))
    }

    fn failures(&self) -> std::sync::MutexGuard<'_, FailureLimiter> {
        self.failures.lock().expect("failures lock is poisoned")
    }

    /// Refuses a client (or everyone) after too many failed sign-in attempts.
    fn check_attempts(&self, client: &str) -> ServerResult<()> {
        match self.failures().blocked_for(client, Instant::now()) {
            Some(wait) => {
                warn!("refused a sign-in attempt from {client}: too many failed attempts");
                Err(ServerError::TooManyAttempts(wait))
            }
            None => Ok(()),
        }
    }

    fn record_failed_attempt(&self, client: &str) {
        self.failures().record_failure(client, Instant::now());
    }

    fn reset_failed_attempts(&self, client: &str) {
        self.failures().reset(client);
    }

    fn take_ceremony(&self, state_id: &str) -> ServerResult<Ceremony> {
        self.ceremonies
            .lock()
            .expect("ceremonies lock is poisoned")
            .take(state_id)
            .ok_or_else(expired_ceremony)
    }

    /// A one-time code for the app url `return_to`, bound to the session of the caller.
    async fn issue_app_code(&self, headers: &HeaderMap, return_to: &str) -> ServerResult<AppCodeEntity> {
        if !self.enabled() {
            return Err(ServerError::InvalidInput("authentication is not enabled".to_owned()));
        }
        let (principal, session) = self.session(headers).await.ok_or_else(unauthorized)?;
        let return_to = app_login::parse_return_to(return_to, &self.app_return_schemes).map_err(|e| {
            if let ReturnToError::SchemeNotAllowed(_) = e {
                warn!("refused an app login handoff of {} to `{return_to}`: {e}", principal.user);
            }
            ServerError::InvalidInput(e.to_string())
        })?;
        let code = self.app_codes.lock().expect("app codes lock is poisoned").issue(session, Instant::now());
        Ok(AppCodeEntity {
            redirect: app_login::redirect_with_code(return_to, &code),
            code,
        })
    }

    /// The session token a one-time code was issued for, when the code is valid and the session still is.
    async fn exchange_app_code(&self, code: &str) -> ServerResult<AppTokenEntity> {
        let session = self
            .app_codes
            .lock()
            .expect("app codes lock is poisoned")
            .take(code.trim(), Instant::now())
            .ok_or_else(invalid_app_code)?;
        let claims = self.session_claims(&session).await.ok_or_else(invalid_app_code)?;
        let expires_at = chrono::DateTime::from_timestamp(claims.exp, 0).ok_or_else(invalid_app_code)?;
        Ok(AppTokenEntity {
            token: session,
            expires_at,
            user: claims.sub,
        })
    }

    async fn passkey_entities(&self) -> Vec<PasskeyEntity> {
        self.passkeys
            .read()
            .await
            .iter()
            .map(|record| PasskeyEntity {
                id: record.id.clone(),
                name: record.name.clone(),
                created_at: record.created_at,
            })
            .collect()
    }

    async fn add_passkey(&self, name: Option<&str>, passkey: Passkey) -> ServerResult<PasskeyRecord> {
        let mut passkeys = self.passkeys.write().await;
        if passkeys.iter().any(|record| record.passkey.cred_id() == passkey.cred_id()) {
            return Err(ServerError::Conflict("this passkey is already registered".to_owned()));
        }
        let record = PasskeyRecord {
            id: webauthn_rs::prelude::Uuid::new_v4().to_string(),
            name: passkey::passkey_name(name, passkeys.len()),
            created_at: Utc::now(),
            passkey,
        };
        let mut updated = passkeys.clone();
        updated.push(record.clone());
        self.save_passkeys(&updated).await?;
        *passkeys = updated;
        info!("passkey `{}` is registered", record.name);
        Ok(record)
    }

    /// Records a passkey login (its counter and backup state); the id of the passkey that was used.
    async fn record_passkey_use(&self, result: &AuthenticationResult) -> ServerResult<String> {
        let mut passkeys = self.passkeys.write().await;
        let mut updated = passkeys.clone();
        let mut used = None;
        for record in updated.iter_mut() {
            if let Some(changed) = record.passkey.update_credential(result) {
                used = Some((record.id.clone(), changed));
                break;
            }
        }
        let Some((id, changed)) = used else {
            return Err(ServerError::Unauthorized("this passkey is not registered".to_owned()));
        };
        if changed {
            self.save_passkeys(&updated).await?;
            *passkeys = updated;
        }
        Ok(id)
    }

    async fn remove_passkey(&self, id: &str) -> ServerResult<()> {
        let mut passkeys = self.passkeys.write().await;
        if !passkeys.iter().any(|record| record.id == id) {
            return Err(ServerError::NoSuchPasskey);
        }
        if passkeys.len() == 1 && !self.password_enabled() {
            return Err(ServerError::Conflict(
                "cannot remove the last passkey while password login is disabled, it would lock you out".to_owned(),
            ));
        }
        let updated: Vec<PasskeyRecord> = passkeys.iter().filter(|record| record.id != id).cloned().collect();
        self.save_passkeys(&updated).await?;
        *passkeys = updated;
        Ok(())
    }
}

/// The middleware guarding the API when authentication is enabled.
pub async fn require_authentication(State(auth): State<SharedAuth>, request: Request, next: Next) -> Response {
    if !requires_authentication(request.method(), request.uri().path()) || auth.authenticate(request.headers()).await.is_some() {
        return next.run(request).await;
    }
    unauthorized().into_response()
}

/// The handlers of the `/api/auth/*` endpoints, all reachable without a session.
pub mod handlers {
    use std::net::SocketAddr;

    use axum::extract::{ConnectInfo, Path, State};
    use axum::http::HeaderMap;
    use axum::Json;
    use gotcha::api;
    use log::warn;
    use webauthn_rs::prelude::{PublicKeyCredential, RegisterPublicKeyCredential, Uuid};

    use super::passkey::CeremonyKind;
    use super::{expired_ceremony, unauthorized, SharedAuth, WithSessionCookie};
    use crate::error::ServerError;
    use crate::request::{
        AppCodeRequest, AppExchangeRequest, LoginRequest, PasskeyLoginFinishRequest, PasskeyRegisterFinishRequest, PasskeyRegisterStartRequest,
    };
    use crate::response::{AppCodeEntity, AppTokenEntity, AuthStatusEntity, PasskeyChallengeEntity, PasskeyEntity, ResponseWrapper};
    use crate::{ApiResult, ServerResult};

    fn webauthn_error(context: &str, error: impl std::fmt::Display) -> ServerError {
        ServerError::Internal(format!("{context}: {error}"))
    }

    /// Whether authentication is enabled, which methods, and who the caller is.
    #[api(group = "auth")]
    pub async fn get_auth_status(State(auth): State<SharedAuth>, #[api(skip)] headers: HeaderMap) -> ApiResult<AuthStatusEntity> {
        let principal = auth.authenticate(&headers).await;
        ResponseWrapper::json(auth.status(principal.as_ref()).await)
    }

    /// Exchanges the `ZHANG_AUTH` credential for a session cookie.
    #[api(group = "auth")]
    pub async fn auth_login(
        State(auth): State<SharedAuth>, #[api(skip)] headers: HeaderMap, #[api(skip)] peer: Option<ConnectInfo<SocketAddr>>, Json(payload): Json<LoginRequest>,
    ) -> ServerResult<WithSessionCookie<AuthStatusEntity>> {
        if !auth.password_enabled() {
            return Err(ServerError::InvalidInput("password login is not enabled".to_owned()));
        }
        let client = auth.client(&headers, peer);
        auth.check_attempts(&client)?;
        if !auth.verify_password(&payload.username, &payload.password) {
            warn!("failed password login for user {:?} from {client}", payload.username);
            auth.record_failed_attempt(&client);
            return Err(ServerError::Unauthorized("invalid username or password".to_owned()));
        }
        auth.reset_failed_attempts(&client);
        Ok(auth.status_with_cookie(&headers, auth.password_session()).await)
    }

    /// Ends the session by clearing its cookie.
    #[api(group = "auth")]
    pub async fn auth_logout(State(auth): State<SharedAuth>, #[api(skip)] headers: HeaderMap) -> WithSessionCookie<AuthStatusEntity> {
        auth.status_with_cookie(&headers, None).await
    }

    /// Starts registering a passkey; needs a session or the `ZHANG_PASSKEY` secret.
    #[api(group = "auth")]
    pub async fn passkey_register_start(
        State(auth): State<SharedAuth>, #[api(skip)] headers: HeaderMap, #[api(skip)] peer: Option<ConnectInfo<SocketAddr>>,
        Json(payload): Json<PasskeyRegisterStartRequest>,
    ) -> ApiResult<PasskeyChallengeEntity> {
        auth.ensure_passkey_enabled()?;
        if auth.authenticate(&headers).await.is_none() {
            let Some(secret) = payload.secret.as_deref() else {
                return Err(unauthorized());
            };
            let client = auth.client(&headers, peer);
            auth.check_attempts(&client)?;
            if !auth.verify_passkey_secret(secret) {
                warn!("invalid passkey registration secret from {client}");
                auth.record_failed_attempt(&client);
                return Err(ServerError::Unauthorized("invalid registration secret".to_owned()));
            }
        }
        let relying_party = auth.relying_party(&headers)?;
        let webauthn = relying_party.webauthn()?;
        let exclude: Vec<_> = auth.passkeys.read().await.iter().map(|record| record.passkey.cred_id().clone()).collect();
        let user = auth.ledger_title().await.unwrap_or_else(|| "zhang".to_owned());
        let (options, state) = webauthn
            .start_passkey_registration(Uuid::new_v4(), &user, &user, (!exclude.is_empty()).then_some(exclude))
            .map_err(|e| webauthn_error("cannot start the passkey registration", e))?;
        let mut options = serde_json::to_value(options).map_err(|e| webauthn_error("cannot serialize the passkey options", e))?;
        // ask for a discoverable credential, what browsers and platforms call a passkey (and what
        // makes Android offer its password manager)
        if let Some(selection) = options.pointer_mut("/publicKey/authenticatorSelection").and_then(|it| it.as_object_mut()) {
            selection.insert("residentKey".to_owned(), "required".into());
            selection.insert("requireResidentKey".to_owned(), true.into());
        }
        let state_id = auth.start_ceremony(CeremonyKind::Registration { state, name: payload.name }, relying_party);
        Ok(ResponseWrapper {
            data: PasskeyChallengeEntity { state_id, options },
        })
    }

    /// Finishes registering a passkey and stores it. A caller without a session (registering with
    /// the secret) is signed in with the new passkey; a signed-in caller keeps their session.
    #[api(group = "auth")]
    pub async fn passkey_register_finish(
        State(auth): State<SharedAuth>, #[api(skip)] headers: HeaderMap, #[api(skip)] peer: Option<ConnectInfo<SocketAddr>>,
        Json(payload): Json<PasskeyRegisterFinishRequest>,
    ) -> ServerResult<WithSessionCookie<AuthStatusEntity>> {
        auth.ensure_passkey_enabled()?;
        let principal = auth.authenticate(&headers).await;
        let ceremony = auth.take_ceremony(&payload.state_id)?;
        let CeremonyKind::Registration { state, name } = ceremony.kind else {
            return Err(expired_ceremony());
        };
        let credential: RegisterPublicKeyCredential =
            serde_json::from_value(payload.credential).map_err(|e| ServerError::InvalidInput(format!("invalid passkey credential: {e}")))?;
        let passkey = ceremony
            .relying_party
            .webauthn()?
            .finish_passkey_registration(&credential, &state)
            .map_err(|e| {
                warn!("passkey registration failed: {e}");
                ServerError::InvalidInput(format!("passkey registration failed: {e}"))
            })?;
        let record = auth.add_passkey(payload.name.as_deref().or(name.as_deref()), passkey).await?;
        auth.reset_failed_attempts(&auth.client(&headers, peer));
        match principal {
            // moving the session onto the new passkey would end it when that passkey is removed
            Some(principal) => Ok(auth.status_keeping_session(&principal).await),
            None => {
                let session = auth.passkey_session(&record.id);
                Ok(auth.status_with_cookie(&headers, Some(session)).await)
            }
        }
    }

    /// Starts a passkey login, allowing every registered passkey.
    #[api(group = "auth")]
    pub async fn passkey_login_start(State(auth): State<SharedAuth>, #[api(skip)] headers: HeaderMap) -> ApiResult<PasskeyChallengeEntity> {
        auth.ensure_passkey_enabled()?;
        let passkeys: Vec<_> = auth.passkeys.read().await.iter().map(|record| record.passkey.clone()).collect();
        if passkeys.is_empty() {
            return Err(ServerError::InvalidInput("no passkey is registered".to_owned()));
        }
        let relying_party = auth.relying_party(&headers)?;
        let (options, state) = relying_party
            .webauthn()?
            .start_passkey_authentication(&passkeys)
            .map_err(|e| webauthn_error("cannot start the passkey login", e))?;
        let options = serde_json::to_value(options).map_err(|e| webauthn_error("cannot serialize the passkey options", e))?;
        let state_id = auth.start_ceremony(CeremonyKind::Authentication { state }, relying_party);
        Ok(ResponseWrapper {
            data: PasskeyChallengeEntity { state_id, options },
        })
    }

    /// Finishes a passkey login and signs the caller in.
    #[api(group = "auth")]
    pub async fn passkey_login_finish(
        State(auth): State<SharedAuth>, #[api(skip)] headers: HeaderMap, #[api(skip)] peer: Option<ConnectInfo<SocketAddr>>,
        Json(payload): Json<PasskeyLoginFinishRequest>,
    ) -> ServerResult<WithSessionCookie<AuthStatusEntity>> {
        auth.ensure_passkey_enabled()?;
        let ceremony = auth.take_ceremony(&payload.state_id)?;
        let CeremonyKind::Authentication { state } = ceremony.kind else {
            return Err(expired_ceremony());
        };
        let credential: PublicKeyCredential =
            serde_json::from_value(payload.credential).map_err(|e| ServerError::InvalidInput(format!("invalid passkey credential: {e}")))?;
        let result = ceremony
            .relying_party
            .webauthn()?
            .finish_passkey_authentication(&credential, &state)
            .map_err(|e| {
                warn!("passkey login failed: {e}");
                ServerError::Unauthorized("passkey verification failed".to_owned())
            })?;
        let passkey_id = auth.record_passkey_use(&result).await?;
        auth.reset_failed_attempts(&auth.client(&headers, peer));
        let session = auth.passkey_session(&passkey_id);
        Ok(auth.status_with_cookie(&headers, Some(session)).await)
    }

    /// Issues a one-time code for the app login handoff, valid for 60 seconds, and the app url
    /// `return_to` with the code appended; needs a session (cookie or Bearer). The scheme of
    /// `return_to` must be `zhang-app` or one of `ZHANG_APP_RETURN_SCHEMES`.
    #[api(group = "auth")]
    pub async fn app_login_code(
        State(auth): State<SharedAuth>, #[api(skip)] headers: HeaderMap, Json(payload): Json<AppCodeRequest>,
    ) -> ApiResult<AppCodeEntity> {
        Ok(ResponseWrapper {
            data: auth.issue_app_code(&headers, &payload.return_to).await?,
        })
    }

    /// Exchanges a one-time code of the app login handoff for the session token, to send as
    /// `Authorization: Bearer`. A code can be exchanged once; failures count towards the sign-in limit.
    #[api(group = "auth")]
    pub async fn app_login_exchange(
        State(auth): State<SharedAuth>, #[api(skip)] headers: HeaderMap, #[api(skip)] peer: Option<ConnectInfo<SocketAddr>>,
        Json(payload): Json<AppExchangeRequest>,
    ) -> ApiResult<AppTokenEntity> {
        let client = auth.client(&headers, peer);
        auth.check_attempts(&client)?;
        match auth.exchange_app_code(&payload.code).await {
            Ok(token) => {
                auth.reset_failed_attempts(&client);
                Ok(ResponseWrapper { data: token })
            }
            Err(e) => {
                warn!("invalid app login code from {client}");
                auth.record_failed_attempt(&client);
                Err(e)
            }
        }
    }

    /// The registered passkeys; needs a session.
    #[api(group = "auth")]
    pub async fn get_passkeys(State(auth): State<SharedAuth>, #[api(skip)] headers: HeaderMap) -> ApiResult<Vec<PasskeyEntity>> {
        auth.require_principal(&headers).await?;
        auth.ensure_passkey_enabled()?;
        Ok(ResponseWrapper {
            data: auth.passkey_entities().await,
        })
    }

    /// Removes a passkey (and ends its sessions); needs a session. The last passkey stays while
    /// password login is disabled.
    #[api(group = "auth")]
    pub async fn delete_passkey(
        State(auth): State<SharedAuth>, #[api(skip)] headers: HeaderMap, Path((passkey_id,)): Path<(String,)>,
    ) -> ApiResult<Vec<PasskeyEntity>> {
        auth.require_principal(&headers).await?;
        auth.ensure_passkey_enabled()?;
        auth.remove_passkey(&passkey_id).await?;
        Ok(ResponseWrapper {
            data: auth.passkey_entities().await,
        })
    }
}

#[cfg(test)]
mod test {
    use axum::http::{HeaderMap, HeaderValue, Method};

    use super::{
        bearer_token, forwarded_value, requires_authentication, session_cookie, session_cookie_values, session_tokens, PasswordCredential, RequestSite,
    };

    fn headers(pairs: &[(&'static str, &'static str)]) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for (name, value) in pairs {
            headers.append(*name, HeaderValue::from_static(value));
        }
        headers
    }

    #[test]
    fn credentials_split_at_the_first_colon() {
        assert_eq!(
            PasswordCredential::parse("admin:pa:ss"),
            PasswordCredential {
                username: "admin".to_owned(),
                password: "pa:ss".to_owned()
            }
        );
        assert_eq!(PasswordCredential::parse("admin").password, "");
    }

    #[test]
    fn only_the_api_but_its_auth_endpoints_requires_authentication() {
        assert!(requires_authentication(&Method::GET, "/api/info"));
        assert!(requires_authentication(&Method::GET, "/api/sse"));
        assert!(requires_authentication(&Method::GET, "/api/authors"));
        assert!(!requires_authentication(&Method::GET, "/api/auth/status"));
        assert!(!requires_authentication(&Method::POST, "/api/auth/passkey/login/start"));
        assert!(!requires_authentication(&Method::OPTIONS, "/api/info"));
        assert!(!requires_authentication(&Method::GET, "/"));
        assert!(!requires_authentication(&Method::GET, "/assets/index.js"));
        assert!(!requires_authentication(&Method::GET, "/login"));
    }

    #[test]
    fn forwarded_values_are_the_rightmost_entry() {
        let value = |pairs| forwarded_value(&headers(pairs), "x-forwarded-for").map(str::to_owned);
        assert_eq!(value(&[("x-forwarded-for", "203.0.113.7")]).as_deref(), Some("203.0.113.7"));
        assert_eq!(value(&[("x-forwarded-for", "198.51.100.1, 203.0.113.7 ")]).as_deref(), Some("203.0.113.7"));
        // several headers are one list, in order
        assert_eq!(
            value(&[("x-forwarded-for", "198.51.100.1, 203.0.113.7"), ("x-forwarded-for", "10.0.0.1")]).as_deref(),
            Some("10.0.0.1")
        );
        assert_eq!(value(&[("x-forwarded-for", "203.0.113.7, ")]).as_deref(), Some("203.0.113.7"));
        assert_eq!(value(&[("x-forwarded-for", " , ")]), None);
        assert_eq!(value(&[]), None);
    }

    #[test]
    fn the_site_prefers_the_forwarded_headers() {
        let site = RequestSite::from_headers(&headers(&[("host", "localhost:8000")])).unwrap();
        assert_eq!((site.scheme.as_str(), site.host.as_str()), ("http", "localhost:8000"));

        let forwarded = headers(&[
            ("host", "10.0.0.1:8000"),
            ("x-forwarded-host", "spoofed.example.org, zhang.example.com"),
            ("x-forwarded-proto", "http, HTTPS"),
        ]);
        let site = RequestSite::from_headers(&forwarded).unwrap();
        assert_eq!((site.scheme.as_str(), site.host.as_str()), ("https", "zhang.example.com"));
        assert!(RequestSite::from_headers(&HeaderMap::new()).is_none());
    }

    #[test]
    fn session_cookies_are_secure_over_https() {
        let cookie = session_cookie(&headers(&[]), Some("token"));
        assert_eq!(cookie, "zhang_session=token; Path=/; HttpOnly; SameSite=Lax; Max-Age=2592000");
        let cookie = session_cookie(&headers(&[("x-forwarded-proto", "https")]), None);
        assert_eq!(cookie, "zhang_session=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0; Secure");
    }

    #[test]
    fn session_cookies_are_found_among_others() {
        let headers = headers(&[("cookie", "theme=dark; zhang_session=a"), ("cookie", "zhang_session=b")]);
        assert_eq!(session_cookie_values(&headers).collect::<Vec<_>>(), vec!["a", "b"]);
    }

    #[test]
    fn bearer_tokens_are_session_tokens_after_the_cookies() {
        assert_eq!(bearer_token(&headers(&[("authorization", "Bearer abc.def")])), Some("abc.def"));
        assert_eq!(bearer_token(&headers(&[("authorization", "bearer  abc.def ")])), Some("abc.def"));
        assert_eq!(bearer_token(&headers(&[("authorization", "Bearer ")])), None);
        assert_eq!(bearer_token(&headers(&[("authorization", "Basic YWRtaW46c2VjcmV0")])), None);
        assert_eq!(bearer_token(&headers(&[])), None);
        let both = headers(&[("cookie", "zhang_session=a"), ("authorization", "Bearer b")]);
        assert_eq!(session_tokens(&both).collect::<Vec<_>>(), vec!["a", "b"]);
    }
}
