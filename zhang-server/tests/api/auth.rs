//! Session login and passkey authentication (#435), through the router the server runs.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{header, Method, Request, StatusCode};
use axum::Router;
use base64::engine::general_purpose::{STANDARD as BASE64_STANDARD, URL_SAFE_NO_PAD};
use base64::Engine as _;
use gotcha::{GotchaApp, GotchaContext};
use hmac::{Hmac, KeyInit, Mac};
use serde_json::{json, Value};
use sha2::Sha256;
use tokio::sync::RwLock;
use webauthn_authenticator_rs::softpasskey::SoftPasskey;
use webauthn_authenticator_rs::WebauthnAuthenticator;
use webauthn_rs::prelude::{CreationChallengeResponse, RequestChallengeResponse, Url};
use zhang_core::ledger::Ledger;
use zhang_server::ServerApp;
use zhang_testkit::http::{router_of, send, Reply, RootedFileSystem};
use zhang_testkit::ledger::Scratch;

const MAIN: &str = "option \"title\" \"Auth Test\"\n1970-01-01 commodity CNY\n1970-01-01 open Assets:Cash\n";
const HOST: &str = "localhost:8010";
const ORIGIN: &str = "http://localhost:8010";
const SESSION_SECRET: &str = "test-session-secret";

/// A scratch ledger directory with [`MAIN`], removed on drop.
fn scratch() -> Scratch {
    Scratch::zhang(MAIN)
}

/// A ledger serving zhang-core's echo router plugin, named `router-echo`: it answers every
/// request with status 201 and the request JSON it received.
fn with_router_plugin() -> Scratch {
    let dir = scratch();
    let module = dir.dir().join("router.wat");
    std::fs::write(&module, include_str!("../../../zhang-core/tests/plugins/router.wat")).unwrap();
    // a local ledger resolves a module against the working directory, so declare it by absolute path
    let main = format!("option \"features.plugin\" \"true\"\nplugin \"{}\"\n{MAIN}", module.display());
    std::fs::write(dir.main_file(), main).unwrap();
    dir
}

/// the passkeys the server stores in the ledger directory
trait Passkeys {
    fn passkeys_file(&self) -> PathBuf;
    fn stored_passkeys(&self) -> Vec<Value>;
}

impl Passkeys for Scratch {
    fn passkeys_file(&self) -> PathBuf {
        self.dir().join(".zhang/passkeys.json")
    }

    fn stored_passkeys(&self) -> Vec<Value> {
        serde_json::from_slice::<Vec<Value>>(&std::fs::read(self.passkeys_file()).unwrap()).unwrap()
    }
}

#[derive(Default, Clone)]
struct Settings {
    auth: Option<&'static str>,
    passkey: Option<&'static str>,
    rp_id: Option<&'static str>,
    origin: Option<&'static str>,
    session_secret: Option<&'static str>,
    app_return_schemes: Option<&'static str>,
}

impl Settings {
    fn password() -> Self {
        Settings {
            auth: Some("admin:secret"),
            session_secret: Some(SESSION_SECRET),
            ..Settings::default()
        }
    }

    fn passkey() -> Self {
        Settings {
            passkey: Some("letmein"),
            session_secret: Some(SESSION_SECRET),
            ..Settings::default()
        }
    }

    fn both() -> Self {
        Settings {
            auth: Some("admin:secret"),
            passkey: Some("letmein"),
            session_secret: Some(SESSION_SECRET),
            ..Settings::default()
        }
    }
}

async fn app(dir: &Path, settings: &Settings) -> ServerApp {
    app_and_ledger(dir, settings).await.0
}

/// The app, and the ledger it serves: the ledger of `dir` read through a source without a local root, as the
/// CLI's opendal one.
async fn app_and_ledger(dir: &Path, settings: &Settings) -> (ServerApp, Arc<RwLock<Ledger>>) {
    let settings = zhang_testkit::http::Settings {
        auth_credential: settings.auth.map(str::to_owned),
        passkey_secret: settings.passkey.map(str::to_owned),
        passkey_rp_id: settings.rp_id.map(str::to_owned),
        passkey_origin: settings.origin.map(str::to_owned),
        session_secret: settings.session_secret.map(str::to_owned),
        app_return_schemes: settings.app_return_schemes.map(str::to_owned),
    };
    zhang_testkit::http::app_and_ledger(dir, "main.zhang", Arc::new(RootedFileSystem::new(dir)), &settings).await
}

async fn server(dir: &Path, settings: &Settings) -> Router {
    router_of(app(dir, settings).await).await
}

/// the cookies a reply sets
trait Cookies {
    /// The `zhang_session=...` pair the reply sets.
    fn session_cookie(&self) -> String;
    fn set_cookie(&self) -> String;
}

impl Cookies for Reply {
    fn session_cookie(&self) -> String {
        let cookie = self.headers.get(header::SET_COOKIE).expect("a session cookie is set").to_str().unwrap();
        let pair = cookie.split(';').next().unwrap().to_owned();
        assert!(pair.starts_with("zhang_session="), "unexpected cookie {cookie}");
        pair
    }

    fn set_cookie(&self) -> String {
        self.headers.get(header::SET_COOKIE).expect("a cookie is set").to_str().unwrap().to_owned()
    }
}

/// `method uri` with the `Host` of the passkey origin, `headers` and `body` as JSON.
async fn call(router: &Router, method: Method, uri: &str, headers: &[(&str, &str)], body: Option<Value>) -> Reply {
    call_from(router, None, method, uri, headers, body).await
}

/// Sends a request from the `peer` address of the connection.
async fn call_from(router: &Router, peer: Option<&str>, method: Method, uri: &str, headers: &[(&str, &str)], body: Option<Value>) -> Reply {
    let mut request = Request::builder().method(method).uri(uri).header(header::HOST, HOST);
    if let Some(peer) = peer {
        request = request.extension(ConnectInfo(peer.parse::<SocketAddr>().unwrap()));
    }
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let request = match body {
        Some(body) => request
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap())),
        None => request.body(Body::empty()),
    }
    .unwrap();
    send(router, request).await
}

async fn get(router: &Router, uri: &str, headers: &[(&str, &str)]) -> Reply {
    call(router, Method::GET, uri, headers, None).await
}

async fn post(router: &Router, uri: &str, headers: &[(&str, &str)], body: Value) -> Reply {
    call(router, Method::POST, uri, headers, Some(body)).await
}

fn assert_unauthorized(reply: &Reply) {
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED, "{}", reply.body);
    assert_eq!(reply.body, json!({"message": "unauthorized"}));
    assert!(reply.headers.get(header::WWW_AUTHENTICATE).is_none(), "no browser popup");
}

async fn password_login(router: &Router) -> String {
    let reply = post(router, "/api/auth/login", &[], json!({"username": "admin", "password": "secret"})).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    reply.session_cookie()
}

fn soft_authenticator() -> WebauthnAuthenticator<SoftPasskey> {
    // the soft token cannot verify the user, it only claims to
    WebauthnAuthenticator::new(SoftPasskey::new(true))
}

fn origin() -> Url {
    Url::parse(ORIGIN).unwrap()
}

/// Registers a passkey of `authenticator`, the start request carrying `start` and `headers`.
async fn register_passkey(router: &Router, authenticator: &mut WebauthnAuthenticator<SoftPasskey>, headers: &[(&str, &str)], start: Value) -> Reply {
    let started = post(router, "/api/auth/passkey/register/start", headers, start).await;
    assert_eq!(started.status, StatusCode::OK, "{}", started.body);
    let mut options = started.body["data"]["options"].clone();
    assert_eq!(options["publicKey"]["rp"]["id"], "localhost");
    assert_eq!(options["publicKey"]["authenticatorSelection"]["residentKey"], "required");
    assert_eq!(options["publicKey"]["authenticatorSelection"]["userVerification"], "required");
    // the soft token cannot store discoverable credentials, so it is asked for a plain one
    options["publicKey"]["authenticatorSelection"]["requireResidentKey"] = false.into();
    let options: CreationChallengeResponse = serde_json::from_value(options).unwrap();
    let credential = authenticator.do_registration(origin(), options).expect("the soft passkey registers");
    post(
        router,
        "/api/auth/passkey/register/finish",
        headers,
        json!({"state_id": started.body["data"]["state_id"], "name": null, "credential": credential}),
    )
    .await
}

async fn passkey_login(router: &Router, authenticator: &mut WebauthnAuthenticator<SoftPasskey>) -> Reply {
    let started = post(router, "/api/auth/passkey/login/start", &[], json!({})).await;
    assert_eq!(started.status, StatusCode::OK, "{}", started.body);
    let options: RequestChallengeResponse = serde_json::from_value(started.body["data"]["options"].clone()).unwrap();
    let credential = authenticator.do_authentication(origin(), options).expect("the soft passkey signs");
    post(
        router,
        "/api/auth/passkey/login/finish",
        &[],
        json!({"state_id": started.body["data"]["state_id"], "credential": credential}),
    )
    .await
}

fn sign_token(claims: Value) -> String {
    fn tag(data: &[u8]) -> Vec<u8> {
        let mut mac = Hmac::<Sha256>::new_from_slice(SESSION_SECRET.as_bytes()).unwrap();
        mac.update(data);
        mac.finalize().into_bytes().to_vec()
    }
    let payload = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).unwrap());
    let signature = URL_SAFE_NO_PAD.encode(tag(payload.as_bytes()));
    format!("zhang_session={payload}.{signature}")
}

fn password_fingerprint() -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(SESSION_SECRET.as_bytes()).unwrap();
    mac.update(b"password\0admin\0secret");
    URL_SAFE_NO_PAD.encode(&mac.finalize().into_bytes()[..16])
}

#[tokio::test]
async fn without_credentials_everything_stays_open() {
    let dir = scratch();
    let router = server(dir.dir(), &Settings::default()).await;

    assert_eq!(get(&router, "/api/info", &[]).await.status, StatusCode::OK);
    let status = get(&router, "/api/auth/status", &[]).await;
    assert_eq!(status.status, StatusCode::OK);
    assert_eq!(
        status.body,
        json!({"data": {
            "enabled": false,
            "authenticated": true,
            "methods": {"password": false, "passkey": false},
            "passkey_registered": false,
            "user": null,
            "title": "Auth Test",
            "app_login": true,
            "app_return_schemes": ["zhang-app"]
        }})
    );
    assert!(!dir.passkeys_file().exists());
}

#[tokio::test]
async fn password_sessions_guard_the_api() {
    let dir = scratch();
    let router = server(dir.dir(), &Settings::password()).await;

    assert_unauthorized(&get(&router, "/api/info", &[]).await);
    assert_unauthorized(&get(&router, "/api/sse", &[]).await);
    let status = get(&router, "/api/auth/status", &[]).await;
    assert_eq!(
        status.body,
        json!({"data": {
            "enabled": true,
            "authenticated": false,
            "methods": {"password": true, "passkey": false},
            "passkey_registered": false,
            "user": null,
            "title": "Auth Test",
            "app_login": true,
            "app_return_schemes": ["zhang-app"]
        }})
    );

    for (username, password) in [("admin", "wrong"), ("root", "secret"), ("", "")] {
        let reply = post(&router, "/api/auth/login", &[], json!({"username": username, "password": password})).await;
        assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
        assert_eq!(reply.body, json!({"message": "invalid username or password"}));
        assert!(reply.headers.get(header::SET_COOKIE).is_none());
    }

    let login = post(&router, "/api/auth/login", &[], json!({"username": "admin", "password": "secret"})).await;
    assert_eq!(login.status, StatusCode::OK);
    let set_cookie = login.set_cookie();
    assert!(set_cookie.ends_with("; Path=/; HttpOnly; SameSite=Lax; Max-Age=2592000"), "{set_cookie}");
    assert_eq!(login.body["data"]["authenticated"], true);
    assert_eq!(login.body["data"]["user"], "admin");
    let cookie = login.session_cookie();

    let info = get(&router, "/api/info", &[("cookie", &cookie)]).await;
    assert_eq!(info.status, StatusCode::OK);
    assert_eq!(info.body["data"]["title"], "Auth Test");
    // the event stream is authenticated by the cookie as well (EventSource sends it)
    let sse = get(&router, "/api/sse", &[("cookie", &cookie)]).await;
    assert_eq!(sse.status, StatusCode::OK);
    let status = get(&router, "/api/auth/status", &[("cookie", &format!("theme=dark; {cookie}"))]).await;
    assert_eq!(status.body["data"]["authenticated"], true);
    assert_eq!(status.body["data"]["user"], "admin");

    let logout = post(&router, "/api/auth/logout", &[("cookie", &cookie)], json!({})).await;
    assert_eq!(logout.status, StatusCode::OK);
    assert_eq!(logout.set_cookie(), "zhang_session=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0");
    assert_eq!(logout.body["data"]["authenticated"], false);

    // without a passkey secret, the passkey endpoints are off
    let reply = post(&router, "/api/auth/passkey/register/start", &[], json!({"secret": "letmein"})).await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    assert_eq!(reply.body, json!({"message": "passkey login is not enabled"}));
    let reply = post(&router, "/api/auth/passkey/login/start", &[], json!({})).await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn sessions_are_secure_cookies_behind_an_https_proxy() {
    let dir = scratch();
    let router = server(dir.dir(), &Settings::password()).await;
    let login = post(
        &router,
        "/api/auth/login",
        &[("x-forwarded-proto", "https")],
        json!({"username": "admin", "password": "secret"}),
    )
    .await;
    assert!(login.set_cookie().ends_with("; Secure"), "{}", login.set_cookie());
}

#[tokio::test]
async fn the_basic_header_still_works_for_scripts() {
    let dir = scratch();
    let router = server(dir.dir(), &Settings::password()).await;

    let valid = format!("Basic {}", BASE64_STANDARD.encode("admin:secret"));
    assert_eq!(get(&router, "/api/info", &[("authorization", &valid)]).await.status, StatusCode::OK);
    let status = get(&router, "/api/auth/status", &[("authorization", &valid)]).await;
    assert_eq!(status.body["data"]["authenticated"], true);

    let invalid = format!("Basic {}", BASE64_STANDARD.encode("admin:wrong"));
    assert_unauthorized(&get(&router, "/api/info", &[("authorization", &invalid)]).await);
    assert_unauthorized(&get(&router, "/api/info", &[("authorization", "Bearer admin:secret")]).await);

    // the Basic header is the password method, so it is refused when only passkeys are enabled
    let dir = scratch();
    let router = server(dir.dir(), &Settings::passkey()).await;
    assert_unauthorized(&get(&router, "/api/info", &[("authorization", &valid)]).await);
}

#[tokio::test]
async fn tampered_expired_or_outdated_sessions_are_rejected() {
    let dir = scratch();
    let router = server(dir.dir(), &Settings::password()).await;
    let cookie = password_login(&router).await;

    // a different signature, and a different payload under the original signature
    let mut tampered = cookie.clone().into_bytes();
    let position = cookie.find('.').unwrap() + 5;
    tampered[position] = if tampered[position] == b'A' { b'B' } else { b'A' };
    assert_unauthorized(&get(&router, "/api/info", &[("cookie", &String::from_utf8(tampered).unwrap())]).await);
    let (_, signature) = cookie.split_once('.').unwrap();
    let (payload, _) = sign_token(json!({"sub": "root", "exp": chrono::Utc::now().timestamp() + 60, "pw": password_fingerprint()}))
        .split_once('.')
        .map(|(payload, signature)| (payload.to_owned(), signature.to_owned()))
        .unwrap();
    assert_unauthorized(&get(&router, "/api/info", &[("cookie", &format!("{payload}.{signature}"))]).await);
    assert_unauthorized(&get(&router, "/api/info", &[("cookie", "zhang_session=garbage")]).await);

    let now = chrono::Utc::now().timestamp();
    let forged = sign_token(json!({"sub": "admin", "exp": now + 60, "pw": password_fingerprint()}));
    assert_eq!(
        get(&router, "/api/info", &[("cookie", &forged)]).await.status,
        StatusCode::OK,
        "tokens are signed with the session secret"
    );
    let expired = sign_token(json!({"sub": "admin", "exp": now - 1, "pw": password_fingerprint()}));
    assert_unauthorized(&get(&router, "/api/info", &[("cookie", &expired)]).await);
    let unbound = sign_token(json!({"sub": "admin", "exp": now + 60}));
    assert_unauthorized(&get(&router, "/api/info", &[("cookie", &unbound)]).await);

    // sessions survive a restart with the same session secret ...
    let restarted = server(dir.dir(), &Settings::password()).await;
    assert_eq!(get(&restarted, "/api/info", &[("cookie", &cookie)]).await.status, StatusCode::OK);
    // ... but not a new password ...
    let rotated = server(
        dir.dir(),
        &Settings {
            auth: Some("admin:another"),
            ..Settings::password()
        },
    )
    .await;
    assert_unauthorized(&get(&rotated, "/api/info", &[("cookie", &cookie)]).await);
    // ... nor a restart without a session secret
    let random_secret = server(
        dir.dir(),
        &Settings {
            session_secret: None,
            ..Settings::password()
        },
    )
    .await;
    assert_unauthorized(&get(&random_secret, "/api/info", &[("cookie", &cookie)]).await);
}

#[tokio::test]
async fn passkey_registration_needs_the_secret_or_a_session() {
    let dir = scratch();
    let router = server(dir.dir(), &Settings::both()).await;

    let reply = post(&router, "/api/auth/passkey/register/start", &[], json!({"secret": null, "name": null})).await;
    assert_unauthorized(&reply);
    let reply = post(&router, "/api/auth/passkey/register/start", &[], json!({"secret": "wrong"})).await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    assert_eq!(reply.body, json!({"message": "invalid registration secret"}));

    let reply = post(&router, "/api/auth/passkey/register/start", &[], json!({"secret": "letmein", "name": "Laptop"})).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert!(reply.body["data"]["state_id"].as_str().is_some_and(|it| !it.is_empty()));
    assert!(reply.body["data"]["options"]["publicKey"]["challenge"].is_string());

    let cookie = password_login(&router).await;
    let reply = post(&router, "/api/auth/passkey/register/start", &[("cookie", &cookie)], json!({"secret": null})).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);

    // nothing is stored until a registration finishes
    assert!(!dir.passkeys_file().exists());
    let reply = post(&router, "/api/auth/passkey/login/start", &[], json!({})).await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    assert_eq!(reply.body, json!({"message": "no passkey is registered"}));
}

#[tokio::test]
async fn passkeys_register_log_in_and_persist() {
    let dir = scratch();
    let router = server(dir.dir(), &Settings::passkey()).await;
    let status = get(&router, "/api/auth/status", &[]).await;
    assert_eq!(status.body["data"]["methods"], json!({"password": false, "passkey": true}));
    assert_eq!(status.body["data"]["passkey_registered"], false);
    let reply = post(&router, "/api/auth/login", &[], json!({"username": "admin", "password": "secret"})).await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    assert_eq!(reply.body, json!({"message": "password login is not enabled"}));

    // the first passkey, registered with the secret, signs the caller in
    let mut phone = soft_authenticator();
    let registered = register_passkey(&router, &mut phone, &[], json!({"secret": "letmein", "name": "Phone"})).await;
    assert_eq!(registered.status, StatusCode::OK, "{}", registered.body);
    assert_eq!(registered.body["data"]["authenticated"], true);
    assert_eq!(registered.body["data"]["user"], "owner");
    assert_eq!(registered.body["data"]["passkey_registered"], true);
    let cookie = registered.session_cookie();
    assert_eq!(get(&router, "/api/info", &[("cookie", &cookie)]).await.status, StatusCode::OK);

    let stored = dir.stored_passkeys();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0]["name"], "Phone");
    let id = stored[0]["id"].as_str().unwrap().to_owned();
    chrono::DateTime::parse_from_rfc3339(stored[0]["created_at"].as_str().unwrap()).expect("created_at is RFC 3339");
    assert!(stored[0]["passkey"]["cred"]["cred_id"].is_string(), "{}", stored[0]);
    assert_eq!(stored[0]["passkey"]["cred"]["counter"], 0);

    let listed = get(&router, "/api/auth/passkeys", &[("cookie", &cookie)]).await;
    assert_eq!(listed.status, StatusCode::OK);
    assert_eq!(listed.body["data"], json!([{"id": id, "name": "Phone", "created_at": stored[0]["created_at"]}]));
    assert_unauthorized(&get(&router, "/api/auth/passkeys", &[]).await);

    // a passkey login, which records the signature counter
    let login = passkey_login(&router, &mut phone).await;
    assert_eq!(login.status, StatusCode::OK, "{}", login.body);
    let cookie = login.session_cookie();
    assert_eq!(get(&router, "/api/info", &[("cookie", &cookie)]).await.status, StatusCode::OK);
    assert_eq!(dir.stored_passkeys()[0]["passkey"]["cred"]["counter"], 1);

    // a passkey that is not registered is refused
    let started = post(&router, "/api/auth/passkey/login/start", &[], json!({})).await;
    let options: RequestChallengeResponse = serde_json::from_value(started.body["data"]["options"].clone()).unwrap();
    assert!(
        soft_authenticator().do_authentication(origin(), options).is_err(),
        "unknown credentials cannot sign"
    );

    // the passkeys are read back after a restart
    let restarted = server(dir.dir(), &Settings::passkey()).await;
    assert_eq!(get(&restarted, "/api/auth/status", &[]).await.body["data"]["passkey_registered"], true);
    assert_eq!(get(&restarted, "/api/info", &[("cookie", &cookie)]).await.status, StatusCode::OK);
    let login = passkey_login(&restarted, &mut phone).await;
    assert_eq!(login.status, StatusCode::OK, "{}", login.body);
    assert_eq!(dir.stored_passkeys()[0]["passkey"]["cred"]["counter"], 2);

    // more passkeys are added with a session, and the same one is not registered twice
    let mut laptop = soft_authenticator();
    let second = register_passkey(&restarted, &mut laptop, &[("cookie", &cookie)], json!({"name": "Laptop"})).await;
    assert_eq!(second.status, StatusCode::OK, "{}", second.body);
    let names: Vec<_> = dir.stored_passkeys().iter().map(|it| it["name"].as_str().unwrap().to_owned()).collect();
    assert_eq!(names, vec!["Phone", "Laptop"]);
    let started = post(&restarted, "/api/auth/passkey/register/start", &[("cookie", &cookie)], json!({})).await;
    assert_eq!(
        started.body["data"]["options"]["publicKey"]["excludeCredentials"].as_array().map(Vec::len),
        Some(2)
    );
}

#[tokio::test]
async fn passkey_ceremonies_are_single_use() {
    let dir = scratch();
    let router = server(dir.dir(), &Settings::passkey()).await;
    let mut authenticator = soft_authenticator();
    assert_eq!(
        register_passkey(&router, &mut authenticator, &[], json!({"secret": "letmein"})).await.status,
        StatusCode::OK
    );
    assert_eq!(dir.stored_passkeys()[0]["name"], "Passkey 1");

    let started = post(&router, "/api/auth/passkey/login/start", &[], json!({})).await;
    let state_id = started.body["data"]["state_id"].clone();
    let options: RequestChallengeResponse = serde_json::from_value(started.body["data"]["options"].clone()).unwrap();
    let credential = authenticator.do_authentication(origin(), options).unwrap();
    let body = json!({"state_id": state_id, "credential": credential});
    assert_eq!(post(&router, "/api/auth/passkey/login/finish", &[], body.clone()).await.status, StatusCode::OK);
    let replayed = post(&router, "/api/auth/passkey/login/finish", &[], body).await;
    assert_eq!(replayed.status, StatusCode::BAD_REQUEST);
    assert_eq!(replayed.body, json!({"message": "the passkey request is unknown or expired, please try again"}));

    // a registration state cannot finish a login
    let started = post(&router, "/api/auth/passkey/register/start", &[], json!({"secret": "letmein"})).await;
    let reply = post(
        &router,
        "/api/auth/passkey/login/finish",
        &[],
        json!({"state_id": started.body["data"]["state_id"], "credential": credential}),
    )
    .await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn removing_passkeys_ends_their_sessions_but_never_locks_out() {
    let dir = scratch();
    let router = server(dir.dir(), &Settings::passkey()).await;
    let mut phone = soft_authenticator();
    let phone_cookie = register_passkey(&router, &mut phone, &[], json!({"secret": "letmein", "name": "Phone"}))
        .await
        .session_cookie();
    let phone_id = dir.stored_passkeys()[0]["id"].as_str().unwrap().to_owned();

    let reply = call(&router, Method::DELETE, &format!("/api/auth/passkeys/{phone_id}"), &[], None).await;
    assert_unauthorized(&reply);
    let reply = call(
        &router,
        Method::DELETE,
        &format!("/api/auth/passkeys/{phone_id}"),
        &[("cookie", &phone_cookie)],
        None,
    )
    .await;
    assert_eq!(reply.status, StatusCode::CONFLICT);
    assert_eq!(
        reply.body,
        json!({"message": "cannot remove the last passkey while password login is disabled, it would lock you out"})
    );
    let reply = call(&router, Method::DELETE, "/api/auth/passkeys/unknown", &[("cookie", &phone_cookie)], None).await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);

    let mut laptop = soft_authenticator();
    let added = register_passkey(&router, &mut laptop, &[("cookie", &phone_cookie)], json!({"name": "Laptop"})).await;
    assert_eq!(added.status, StatusCode::OK, "{}", added.body);
    let laptop_cookie = passkey_login(&router, &mut laptop).await.session_cookie();
    // a phone login started before the removal
    let started = post(&router, "/api/auth/passkey/login/start", &[], json!({})).await;
    let options: RequestChallengeResponse = serde_json::from_value(started.body["data"]["options"].clone()).unwrap();
    let pending_login = json!({"state_id": started.body["data"]["state_id"], "credential": phone.do_authentication(origin(), options).unwrap()});
    let reply = call(
        &router,
        Method::DELETE,
        &format!("/api/auth/passkeys/{phone_id}"),
        &[("cookie", &laptop_cookie)],
        None,
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(reply.body["data"].as_array().unwrap().len(), 1);
    assert_eq!(reply.body["data"][0]["name"], "Laptop");
    assert_eq!(dir.stored_passkeys().len(), 1);
    // the sessions of the removed passkey end, and it cannot log in anymore
    assert_unauthorized(&get(&router, "/api/info", &[("cookie", &phone_cookie)]).await);
    assert_eq!(get(&router, "/api/info", &[("cookie", &laptop_cookie)]).await.status, StatusCode::OK);
    let reply = post(&router, "/api/auth/passkey/login/finish", &[], pending_login).await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    assert_eq!(reply.body, json!({"message": "this passkey is not registered"}));
    let started = post(&router, "/api/auth/passkey/login/start", &[], json!({})).await;
    let options: RequestChallengeResponse = serde_json::from_value(started.body["data"]["options"].clone()).unwrap();
    assert!(phone.do_authentication(origin(), options).is_err());

    // with password login enabled, the last passkey can go
    let both = server(dir.dir(), &Settings::both()).await;
    let cookie = password_login(&both).await;
    let laptop_id = dir.stored_passkeys()[0]["id"].as_str().unwrap().to_owned();
    let reply = call(&both, Method::DELETE, &format!("/api/auth/passkeys/{laptop_id}"), &[("cookie", &cookie)], None).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(reply.body["data"], json!([]));
    assert_eq!(dir.stored_passkeys(), Vec::<Value>::new());
}

#[tokio::test]
async fn the_relying_party_follows_the_proxy_or_the_overrides() {
    let dir = scratch();
    let router = server(dir.dir(), &Settings::passkey()).await;
    let start = json!({"secret": "letmein"});

    let reply = post(
        &router,
        "/api/auth/passkey/register/start",
        &[("x-forwarded-host", "zhang.example.com"), ("x-forwarded-proto", "https")],
        start.clone(),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(reply.body["data"]["options"]["publicKey"]["rp"]["id"], "zhang.example.com");

    // passkeys need a domain name
    let reply = post(
        &router,
        "/api/auth/passkey/register/start",
        &[("x-forwarded-host", "127.0.0.1:8010")],
        start.clone(),
    )
    .await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    assert!(reply.body["message"].as_str().unwrap().contains("ZHANG_PASSKEY_RP_ID"), "{}", reply.body);

    let overridden = server(
        dir.dir(),
        &Settings {
            origin: Some("https://zhang.example.com"),
            ..Settings::passkey()
        },
    )
    .await;
    let reply = post(&overridden, "/api/auth/passkey/register/start", &[], start.clone()).await;
    assert_eq!(reply.body["data"]["options"]["publicKey"]["rp"]["id"], "zhang.example.com");

    let parent = server(
        dir.dir(),
        &Settings {
            origin: Some("https://zhang.example.com"),
            rp_id: Some("example.com"),
            ..Settings::passkey()
        },
    )
    .await;
    let reply = post(&parent, "/api/auth/passkey/register/start", &[], start.clone()).await;
    assert_eq!(reply.body["data"]["options"]["publicKey"]["rp"]["id"], "example.com");

    // a registration started for one origin cannot be finished from another
    let mut authenticator = soft_authenticator();
    let started = post(&router, "/api/auth/passkey/register/start", &[], start).await;
    let mut options = started.body["data"]["options"].clone();
    options["publicKey"]["authenticatorSelection"]["requireResidentKey"] = false.into();
    let options: CreationChallengeResponse = serde_json::from_value(options).unwrap();
    let credential = authenticator.do_registration(Url::parse("http://localhost:9999").unwrap(), options).unwrap();
    let reply = post(
        &router,
        "/api/auth/passkey/register/finish",
        &[],
        json!({"state_id": started.body["data"]["state_id"], "credential": credential}),
    )
    .await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{}", reply.body);
    assert!(!dir.passkeys_file().exists());
}

#[tokio::test]
async fn the_frontend_stays_reachable() {
    let dir = scratch();
    let router = server(dir.dir(), &Settings::both()).await;
    for uri in ["/", "/login", "/accounts", "/assets/index.js"] {
        let reply = get(&router, uri, &[]).await;
        assert_ne!(reply.status, StatusCode::UNAUTHORIZED, "{uri} is served");
    }
    // CORS preflights carry no credentials
    let reply = call(
        &router,
        Method::OPTIONS,
        "/api/info",
        &[("origin", "http://localhost:5173"), ("access-control-request-method", "GET")],
        None,
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
}

#[tokio::test]
async fn an_unreadable_passkey_file_stops_the_start() {
    let dir = scratch();
    std::fs::create_dir_all(dir.dir().join(".zhang")).unwrap();
    std::fs::write(dir.passkeys_file(), "{ not json").unwrap();
    let app = app(dir.dir(), &Settings::passkey()).await;
    let config = app.config().await.unwrap();
    let error = app.state(&config).await.err().expect("the server refuses to start");
    assert!(error.to_string().contains(".zhang/passkeys.json"), "{error}");

    // the file is only read when passkeys are enabled
    let router = server(dir.dir(), &Settings::password()).await;
    assert_eq!(get(&router, "/api/auth/status", &[]).await.status, StatusCode::OK);
}

#[tokio::test]
async fn the_auth_endpoints_are_documented() {
    let dir = scratch();
    let router = server(dir.dir(), &Settings::both()).await;
    let spec = get(&router, "/openapi.json", &[]).await;
    assert_eq!(spec.status, StatusCode::OK);
    let paths: Vec<_> = spec.body["paths"]
        .as_object()
        .unwrap()
        .keys()
        .filter(|it| it.starts_with("/api/auth"))
        .cloned()
        .collect();
    assert_eq!(
        paths,
        vec![
            "/api/auth/app/code",
            "/api/auth/app/exchange",
            "/api/auth/login",
            "/api/auth/logout",
            "/api/auth/passkey/login/finish",
            "/api/auth/passkey/login/start",
            "/api/auth/passkey/register/finish",
            "/api/auth/passkey/register/start",
            "/api/auth/passkeys",
            "/api/auth/passkeys/{passkey_id}",
            "/api/auth/status",
        ]
    );
    assert!(spec.body["paths"]["/api/auth/passkeys/{passkey_id}"]["delete"].is_object());
}

fn assert_too_many_attempts(reply: &Reply) {
    assert_eq!(reply.status, StatusCode::TOO_MANY_REQUESTS, "{}", reply.body);
    assert_eq!(reply.body, json!({"message": "too many attempts, try again in 15 minutes"}));
    let retry_after: u64 = reply
        .headers
        .get(header::RETRY_AFTER)
        .expect("Retry-After is set")
        .to_str()
        .unwrap()
        .parse()
        .unwrap();
    assert!((14 * 60..=15 * 60).contains(&retry_after), "Retry-After: {retry_after}");
}

async fn login_from(router: &Router, client: &str, password: &str) -> Reply {
    post(
        router,
        "/api/auth/login",
        &[("x-forwarded-for", client)],
        json!({"username": "admin", "password": password}),
    )
    .await
}

#[tokio::test]
async fn failed_logins_are_rate_limited_per_client() {
    let dir = scratch();
    let router = server(dir.dir(), &Settings::both()).await;
    let attacker = "203.0.113.7";

    for _ in 0..5 {
        assert_eq!(login_from(&router, attacker, "guess").await.status, StatusCode::UNAUTHORIZED);
    }
    // the 6th attempt is refused before the password is checked, even the right one
    assert_too_many_attempts(&login_from(&router, attacker, "guess").await);
    assert_too_many_attempts(&login_from(&router, attacker, "secret").await);
    // the registration secret shares the count
    let reply = post(
        &router,
        "/api/auth/passkey/register/start",
        &[("x-forwarded-for", attacker)],
        json!({"secret": "letmein"}),
    )
    .await;
    assert_too_many_attempts(&reply);

    // another client is not affected
    assert_eq!(login_from(&router, "203.0.113.8", "guess").await.status, StatusCode::UNAUTHORIZED);
    let reply = login_from(&router, "203.0.113.8", "secret").await;
    assert_eq!(reply.status, StatusCode::OK);
    // and a session keeps working from the refused address
    let cookie = reply.session_cookie();
    let info = get(&router, "/api/info", &[("x-forwarded-for", attacker), ("cookie", &cookie)]).await;
    assert_eq!(info.status, StatusCode::OK);
    let reply = post(
        &router,
        "/api/auth/passkey/register/start",
        &[("x-forwarded-for", attacker), ("cookie", &cookie)],
        json!({"secret": null}),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
}

#[tokio::test]
async fn a_successful_login_resets_the_client_count() {
    let dir = scratch();
    let router = server(dir.dir(), &Settings::password()).await;
    let client = "198.51.100.20";

    for _ in 0..4 {
        assert_eq!(login_from(&router, client, "guess").await.status, StatusCode::UNAUTHORIZED);
    }
    assert_eq!(login_from(&router, client, "secret").await.status, StatusCode::OK);
    for _ in 0..5 {
        assert_eq!(login_from(&router, client, "guess").await.status, StatusCode::UNAUTHORIZED);
    }
    assert_too_many_attempts(&login_from(&router, client, "guess").await);
}

#[tokio::test]
async fn failed_registration_secrets_are_rate_limited() {
    let dir = scratch();
    let router = server(dir.dir(), &Settings::passkey()).await;
    let start = |secret: &'static str| json!({"secret": secret, "name": null});

    for _ in 0..5 {
        let reply = post(&router, "/api/auth/passkey/register/start", &[("x-forwarded-for", "192.0.2.1")], start("guess")).await;
        assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    }
    let reply = post(
        &router,
        "/api/auth/passkey/register/start",
        &[("x-forwarded-for", "192.0.2.1")],
        start("letmein"),
    )
    .await;
    assert_too_many_attempts(&reply);
    let reply = post(
        &router,
        "/api/auth/passkey/register/start",
        &[("x-forwarded-for", "192.0.2.2")],
        start("letmein"),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    // requests without a secret are refused without counting
    for _ in 0..10 {
        let reply = post(
            &router,
            "/api/auth/passkey/register/start",
            &[("x-forwarded-for", "192.0.2.3")],
            json!({"secret": null}),
        )
        .await;
        assert_unauthorized(&reply);
    }
    let reply = post(
        &router,
        "/api/auth/passkey/register/start",
        &[("x-forwarded-for", "192.0.2.3")],
        start("letmein"),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
}

#[tokio::test]
async fn without_a_forwarded_address_the_peer_is_the_client() {
    let dir = scratch();
    let router = server(dir.dir(), &Settings::password()).await;
    let body = json!({"username": "admin", "password": "guess"});

    for _ in 0..5 {
        let reply = call_from(&router, Some("198.51.100.1:50000"), Method::POST, "/api/auth/login", &[], Some(body.clone())).await;
        assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    }
    // another port of the same address is the same client
    let reply = call_from(&router, Some("198.51.100.1:50001"), Method::POST, "/api/auth/login", &[], Some(body.clone())).await;
    assert_too_many_attempts(&reply);
    let reply = call_from(&router, Some("198.51.100.2:50000"), Method::POST, "/api/auth/login", &[], Some(body.clone())).await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    // behind a proxy, the address it appended to X-Forwarded-For wins over the peer (the proxy)
    let reply = call_from(
        &router,
        Some("198.51.100.1:50002"),
        Method::POST,
        "/api/auth/login",
        &[("x-forwarded-for", "192.0.2.50")],
        Some(json!({"username": "admin", "password": "secret"})),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
}

#[tokio::test]
async fn too_many_failures_overall_refuse_everyone() {
    let dir = scratch();
    let router = server(dir.dir(), &Settings::password()).await;
    for client in 0..10 {
        let client = format!("192.0.2.{}", client + 100);
        for _ in 0..5 {
            assert_eq!(login_from(&router, &client, "guess").await.status, StatusCode::UNAUTHORIZED);
        }
    }
    assert_too_many_attempts(&login_from(&router, "192.0.2.200", "secret").await);
}

async fn login_through(router: &Router, peer: &str, forwarded_for: &str, password: &str) -> Reply {
    call_from(
        router,
        Some(peer),
        Method::POST,
        "/api/auth/login",
        &[("x-forwarded-for", forwarded_for)],
        Some(json!({"username": "admin", "password": password})),
    )
    .await
}

#[tokio::test]
async fn behind_a_proxy_the_address_it_appended_is_the_client() {
    let dir = scratch();
    let router = server(dir.dir(), &Settings::both()).await;
    let proxy = "10.0.0.2:41000";

    // the client rotates what it sends, the proxy appends its real address
    for spoofed in 0..5 {
        let forwarded_for = format!("192.0.2.{spoofed}, 203.0.113.7");
        assert_eq!(login_through(&router, proxy, &forwarded_for, "guess").await.status, StatusCode::UNAUTHORIZED);
    }
    assert_too_many_attempts(&login_through(&router, proxy, "192.0.2.99, 203.0.113.7", "secret").await);
    assert_too_many_attempts(&login_through(&router, proxy, "203.0.113.7", "secret").await);
    // a client claiming the blocked address is judged by its real one
    assert_eq!(login_through(&router, proxy, "203.0.113.7, 203.0.113.8", "secret").await.status, StatusCode::OK);

    // the host and the scheme are the rightmost entries as well
    let reply = post(
        &router,
        "/api/auth/passkey/register/start",
        &[
            ("x-forwarded-host", "spoofed.example.org, zhang.example.com"),
            ("x-forwarded-proto", "http, https"),
        ],
        json!({"secret": "letmein"}),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(reply.body["data"]["options"]["publicKey"]["rp"]["id"], "zhang.example.com");
    let login = post(
        &router,
        "/api/auth/login",
        &[("x-forwarded-for", "203.0.113.9"), ("x-forwarded-proto", "https, http")],
        json!({"username": "admin", "password": "secret"}),
    )
    .await;
    assert!(!login.set_cookie().contains("Secure"), "{}", login.set_cookie());
}

#[tokio::test]
async fn adding_a_passkey_keeps_the_current_session() {
    let dir = scratch();
    let router = server(dir.dir(), &Settings::both()).await;

    // signed in with the password, a passkey is added, then removed
    let password_cookie = password_login(&router).await;
    let mut laptop = soft_authenticator();
    let added = register_passkey(&router, &mut laptop, &[("cookie", &password_cookie)], json!({"name": "Laptop"})).await;
    assert_eq!(added.status, StatusCode::OK, "{}", added.body);
    assert!(added.headers.get(header::SET_COOKIE).is_none(), "the session is kept");
    assert_eq!(added.body["data"]["authenticated"], true);
    assert_eq!(added.body["data"]["user"], "admin");
    assert_eq!(added.body["data"]["passkey_registered"], true);
    let laptop_id = dir.stored_passkeys()[0]["id"].as_str().unwrap().to_owned();
    let removed = call(
        &router,
        Method::DELETE,
        &format!("/api/auth/passkeys/{laptop_id}"),
        &[("cookie", &password_cookie)],
        None,
    )
    .await;
    assert_eq!(removed.status, StatusCode::OK, "{}", removed.body);
    assert_eq!(get(&router, "/api/info", &[("cookie", &password_cookie)]).await.status, StatusCode::OK);
    assert_eq!(
        get(&router, "/api/auth/status", &[("cookie", &password_cookie)]).await.body["data"]["authenticated"],
        true
    );

    // signed in with passkey A, passkey B is added, then removed
    let dir = scratch();
    let router = server(dir.dir(), &Settings::passkey()).await;
    let mut phone = soft_authenticator();
    let first = register_passkey(&router, &mut phone, &[], json!({"secret": "letmein", "name": "Phone"})).await;
    let phone_cookie = first.session_cookie();
    let mut laptop = soft_authenticator();
    let added = register_passkey(&router, &mut laptop, &[("cookie", &phone_cookie)], json!({"name": "Laptop"})).await;
    assert_eq!(added.status, StatusCode::OK, "{}", added.body);
    assert!(added.headers.get(header::SET_COOKIE).is_none(), "the session is kept");
    let laptop_id = dir.stored_passkeys()[1]["id"].as_str().unwrap().to_owned();
    let removed = call(
        &router,
        Method::DELETE,
        &format!("/api/auth/passkeys/{laptop_id}"),
        &[("cookie", &phone_cookie)],
        None,
    )
    .await;
    assert_eq!(removed.status, StatusCode::OK, "{}", removed.body);
    assert_eq!(get(&router, "/api/info", &[("cookie", &phone_cookie)]).await.status, StatusCode::OK);

    // a registration with the secret, without a session, still signs in with the new passkey
    assert!(first.set_cookie().contains("Max-Age=2592000"));
}

#[tokio::test]
async fn the_passkeys_file_is_not_part_of_the_ledger() {
    let dir = scratch();
    let (app, ledger) = app_and_ledger(dir.dir(), &Settings::passkey()).await;
    let config = app.config().await.unwrap();
    let state = app.state(&config).await.unwrap();
    let router = app.build_router(GotchaContext { config, state }).await.unwrap();

    let mut authenticator = soft_authenticator();
    let registered = register_passkey(&router, &mut authenticator, &[], json!({"secret": "letmein"})).await;
    assert_eq!(registered.status, StatusCode::OK, "{}", registered.body);
    assert_eq!(passkey_login(&router, &mut authenticator).await.status, StatusCode::OK);
    assert!(dir.passkeys_file().exists());

    // neither a file of the ledger nor an input of its load, before and after a reload
    for reload in [false, true] {
        let mut ledger = ledger.write().await;
        if reload {
            ledger.reload().unwrap();
        }
        assert!(
            !ledger.visited_files.iter().any(|path| path.components().any(|it| it.as_os_str() == ".zhang")),
            "{:?}",
            ledger.visited_files
        );
        assert!(ledger.extra_inputs.is_empty(), "{:?}", ledger.extra_inputs);
    }
}

#[tokio::test]
async fn router_plugins_need_a_session_and_never_see_it() {
    let dir = with_router_plugin();
    let router = server(dir.dir(), &Settings::password()).await;

    // refused like the rest of the API, whatever the method, and so is the plugin list
    assert_unauthorized(&get(&router, "/api/plugins/router-echo/x", &[]).await);
    assert_unauthorized(&post(&router, "/api/plugins/router-echo/x", &[], json!({"a": 1})).await);
    assert_unauthorized(&call(&router, Method::PUT, "/api/plugins/router-echo", &[], None).await);
    assert_unauthorized(&call(&router, Method::DELETE, "/api/plugins/router-echo/", &[], None).await);
    assert_unauthorized(&get(&router, "/api/plugins/router-echo/x", &[("cookie", "zhang_session=forged")]).await);
    assert_unauthorized(&get(&router, "/api/plugins", &[]).await);
    // OPTIONS skips the session check for CORS preflights, but the CORS layer answers every OPTIONS
    // request itself: it never reaches a plugin
    for headers in [&[][..], &[("origin", "http://localhost:5173"), ("access-control-request-method", "POST")][..]] {
        let reply = call(&router, Method::OPTIONS, "/api/plugins/router-echo/x", headers, None).await;
        assert_eq!(reply.status, StatusCode::OK);
        assert!(reply.headers.get("x-echo").is_none(), "the plugin ran without a session");
        assert_eq!(reply.body, Value::Null);
    }

    let cookie = password_login(&router).await;
    let reply = get(&router, "/api/plugins/router-echo/x?k=v", &[("cookie", &format!("theme=dark; {cookie}"))]).await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    assert_eq!(reply.headers["x-echo"], "router");
    assert_eq!((&reply.body["method"], &reply.body["path"]), (&json!("GET"), &json!("/x")));
    assert_eq!(reply.body["query"], json!({"k": ["v"]}));
    // neither the session cookie nor any other cookie reaches the plugin
    assert_eq!(reply.body["headers"], json!({"host": HOST}));

    let reply = post(&router, "/api/plugins/router-echo/x", &[("cookie", &cookie)], json!({"a": 1})).await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    assert_eq!((&reply.body["method"], &reply.body["body"]), (&json!("POST"), &json!("{\"a\":1}")));
    assert_eq!(reply.body["headers"], json!({"content-type": "application/json", "host": HOST}));

    // scripts sending the Basic header reach the plugin too, which never sees the credential
    let basic = format!("Basic {}", BASE64_STANDARD.encode("admin:secret"));
    let reply = call(&router, Method::PUT, "/api/plugins/router-echo", &[("authorization", &basic)], None).await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    assert_eq!((&reply.body["method"], &reply.body["path"]), (&json!("PUT"), &json!("/")));
    assert_eq!(reply.body["headers"], json!({"host": HOST}));

    let list = get(&router, "/api/plugins", &[("cookie", &cookie)]).await;
    assert_eq!(list.status, StatusCode::OK);
    assert_eq!(list.body["data"][0]["route"], "/api/plugins/router-echo");
}

/// The token of a `zhang_session=<token>` cookie pair, as an `Authorization: Bearer` header value.
fn bearer(cookie: &str) -> String {
    format!("Bearer {}", cookie.strip_prefix("zhang_session=").unwrap())
}

#[tokio::test]
async fn session_tokens_work_as_bearer_tokens() {
    let dir = scratch();
    let router = server(dir.dir(), &Settings::both()).await;
    let cookie = password_login(&router).await;

    let info = get(&router, "/api/info", &[("authorization", &bearer(&cookie))]).await;
    assert_eq!(info.status, StatusCode::OK, "{}", info.body);
    let status = get(&router, "/api/auth/status", &[("authorization", &bearer(&cookie))]).await;
    assert_eq!(status.body["data"]["authenticated"], true);
    assert_eq!(status.body["data"]["user"], "admin");

    // a passkey session too
    let mut phone = soft_authenticator();
    let registered = register_passkey(&router, &mut phone, &[("cookie", &cookie)], json!({"name": "Phone"})).await;
    assert_eq!(registered.status, StatusCode::OK, "{}", registered.body);
    let passkey_cookie = passkey_login(&router, &mut phone).await.session_cookie();
    assert_eq!(
        get(&router, "/api/info", &[("authorization", &bearer(&passkey_cookie))]).await.status,
        StatusCode::OK
    );

    // the same checks as the cookie: signed, not expired, still backed by the configuration
    assert_unauthorized(&get(&router, "/api/info", &[("authorization", "Bearer garbage")]).await);
    let expired = sign_token(json!({"sub": "admin", "exp": chrono::Utc::now().timestamp() - 1, "pw": password_fingerprint()}));
    assert_unauthorized(&get(&router, "/api/info", &[("authorization", &bearer(&expired))]).await);
    let rotated = server(
        dir.dir(),
        &Settings {
            auth: Some("admin:another"),
            ..Settings::both()
        },
    )
    .await;
    assert_unauthorized(&get(&rotated, "/api/info", &[("authorization", &bearer(&cookie))]).await);
    // an invalid bearer token does not hide a valid cookie
    let reply = get(&router, "/api/info", &[("authorization", "Bearer garbage"), ("cookie", &cookie)]).await;
    assert_eq!(reply.status, StatusCode::OK);
}

async fn app_code(router: &Router, headers: &[(&str, &str)], return_to: &str) -> Reply {
    post(router, "/api/auth/app/code", headers, json!({ "return_to": return_to })).await
}

async fn app_exchange(router: &Router, client: &str, code: &str) -> Reply {
    post(router, "/api/auth/app/exchange", &[("x-forwarded-for", client)], json!({ "code": code })).await
}

#[tokio::test]
async fn the_app_login_handoff_exchanges_a_one_time_code_for_the_session_token() {
    let dir = scratch();
    let router = server(dir.dir(), &Settings::password()).await;
    let status = get(&router, "/api/auth/status", &[]).await;
    assert_eq!(status.body["data"]["app_login"], true);
    assert_eq!(status.body["data"]["app_return_schemes"], json!(["zhang-app"]));

    // a code needs a session
    assert_unauthorized(&app_code(&router, &[], "zhang-app://auth/callback").await);
    let basic = format!("Basic {}", BASE64_STANDARD.encode("admin:secret"));
    assert_unauthorized(&app_code(&router, &[("authorization", &basic)], "zhang-app://auth/callback").await);

    let cookie = password_login(&router).await;
    let issued = app_code(&router, &[("cookie", &cookie)], "zhang-app://auth/callback?x=1").await;
    assert_eq!(issued.status, StatusCode::OK, "{}", issued.body);
    let code = issued.body["data"]["code"].as_str().unwrap().to_owned();
    assert_eq!(URL_SAFE_NO_PAD.decode(&code).unwrap().len(), 32);
    assert_eq!(issued.body["data"]["redirect"], format!("zhang-app://auth/callback?x=1&code={code}"));
    assert!(issued.headers.get(header::SET_COOKIE).is_none(), "the browser session stays as it is");

    let exchanged = app_exchange(&router, "198.51.100.30", &code).await;
    assert_eq!(exchanged.status, StatusCode::OK, "{}", exchanged.body);
    let token = exchanged.body["data"]["token"].as_str().unwrap().to_owned();
    assert_eq!(format!("zhang_session={token}"), cookie, "the token is the session of the browser");
    assert_eq!(exchanged.body["data"]["user"], "admin");
    let expires_at = chrono::DateTime::parse_from_rfc3339(exchanged.body["data"]["expires_at"].as_str().unwrap()).unwrap();
    let ttl = expires_at.timestamp() - chrono::Utc::now().timestamp();
    assert!((30 * 24 * 60 * 60 - 60..=30 * 24 * 60 * 60).contains(&ttl), "expires in {ttl} s");
    let info = get(&router, "/api/info", &[("authorization", &format!("Bearer {token}"))]).await;
    assert_eq!(info.status, StatusCode::OK);

    // a code is used once
    let again = app_exchange(&router, "198.51.100.30", &code).await;
    assert_eq!(again.status, StatusCode::UNAUTHORIZED);
    assert_eq!(again.body, json!({"message": "invalid or expired code"}));
    assert_eq!(app_exchange(&router, "198.51.100.30", "unknown").await.status, StatusCode::UNAUTHORIZED);

    // the app can ask for a code with its Bearer token too
    let issued = app_code(&router, &[("authorization", &format!("Bearer {token}"))], "zhang-app://auth/callback").await;
    assert_eq!(issued.status, StatusCode::OK, "{}", issued.body);
    assert_eq!(
        issued.body["data"]["redirect"],
        format!("zhang-app://auth/callback?code={}", issued.body["data"]["code"].as_str().unwrap())
    );
}

#[tokio::test]
async fn the_app_login_handoff_only_returns_to_allowed_schemes() {
    let dir = scratch();
    let router = server(dir.dir(), &Settings::password()).await;
    let cookie = password_login(&router).await;
    for (return_to, message) in [
        (
            "https://evil.example.com/callback",
            "the scheme `https` of return_to is not allowed, allow it with ZHANG_APP_RETURN_SCHEMES",
        ),
        (
            "my-app://callback",
            "the scheme `my-app` of return_to is not allowed, allow it with ZHANG_APP_RETURN_SCHEMES",
        ),
        ("/settings", "return_to is not a valid url"),
        ("zhang-app://auth/callback?code=planted", "return_to must not carry a `code` parameter"),
    ] {
        let reply = app_code(&router, &[("cookie", &cookie)], return_to).await;
        assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{return_to}: {}", reply.body);
        assert_eq!(reply.body, json!({ "message": message }), "{return_to}");
    }

    let router = server(
        dir.dir(),
        &Settings {
            app_return_schemes: Some("My-App, javascript"),
            ..Settings::password()
        },
    )
    .await;
    let status = get(&router, "/api/auth/status", &[]).await;
    assert_eq!(status.body["data"]["app_return_schemes"], json!(["zhang-app", "my-app"]));
    let reply = app_code(&router, &[("cookie", &cookie)], "my-app://callback").await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    let reply = app_code(&router, &[("cookie", &cookie)], "javascript:alert(1)").await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);

    // without authentication there is no session to hand off
    let open = server(dir.dir(), &Settings::default()).await;
    let reply = app_code(&open, &[], "zhang-app://auth/callback").await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    assert_eq!(reply.body, json!({"message": "authentication is not enabled"}));
}

#[tokio::test]
async fn passkey_sessions_are_handed_off_until_they_end() {
    let dir = scratch();
    let router = server(dir.dir(), &Settings::both()).await;
    let cookie = password_login(&router).await;
    let mut phone = soft_authenticator();
    register_passkey(&router, &mut phone, &[("cookie", &cookie)], json!({"name": "Phone"})).await;
    let phone_cookie = passkey_login(&router, &mut phone).await.session_cookie();
    let new_code = || async {
        app_code(&router, &[("cookie", &phone_cookie)], "zhang-app://auth/callback").await.body["data"]["code"]
            .as_str()
            .unwrap()
            .to_owned()
    };

    let exchanged = app_exchange(&router, "198.51.100.31", &new_code().await).await;
    assert_eq!(exchanged.status, StatusCode::OK, "{}", exchanged.body);
    assert_eq!(exchanged.body["data"]["user"], "admin");
    let token = exchanged.body["data"]["token"].as_str().unwrap().to_owned();
    assert_eq!(
        get(&router, "/api/info", &[("authorization", &format!("Bearer {token}"))]).await.status,
        StatusCode::OK
    );
    let code = new_code().await;

    // the passkey is removed before the app exchanges the code
    let phone_id = dir.stored_passkeys()[0]["id"].as_str().unwrap().to_owned();
    let reply = call(&router, Method::DELETE, &format!("/api/auth/passkeys/{phone_id}"), &[("cookie", &cookie)], None).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    let reply = app_exchange(&router, "198.51.100.31", &code).await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    assert_eq!(reply.body, json!({"message": "invalid or expired code"}));
    // and so does the token the app received
    assert_unauthorized(&get(&router, "/api/info", &[("authorization", &format!("Bearer {token}"))]).await);
}

#[tokio::test]
async fn failed_app_login_exchanges_are_rate_limited_with_the_logins() {
    let dir = scratch();
    let router = server(dir.dir(), &Settings::password()).await;
    let cookie = password_login(&router).await;
    let code = app_code(&router, &[("cookie", &cookie)], "zhang-app://auth/callback").await.body["data"]["code"]
        .as_str()
        .unwrap()
        .to_owned();
    let attacker = "203.0.113.9";

    for _ in 0..5 {
        assert_eq!(app_exchange(&router, attacker, "guess").await.status, StatusCode::UNAUTHORIZED);
    }
    // refused before the code is looked at, even a valid one, which stays valid
    assert_too_many_attempts(&app_exchange(&router, attacker, &code).await);
    assert_too_many_attempts(&login_from(&router, attacker, "secret").await);
    assert_eq!(app_exchange(&router, "203.0.113.10", &code).await.status, StatusCode::OK);
}
