//! The HTTP harness of the zhang-server tests (feature `server`).
//!
//! Two levels, as the tests use them:
//!
//! - **handler level**, the default: a route handler is called as a function with the `State` of a loaded ledger
//!   ([`state`], [`states`]) and its response read as JSON ([`respond`], [`data`], [`body`], [`answer`]). Nothing of
//!   the router (middleware, extractors' rejections, paths) takes part.
//! - **router level**, for what only the router shows (authentication, path encoding, content types, error bodies
//!   of rejections): the app the server runs ([`app`], [`router`]) and a request sent through it ([`send`], [`call`],
//!   [`get`], [`post`], [`put`]), answered as a [`Reply`].
//!
//! The ledger comes from [`crate::ledger::Scratch`] (a directory the test owns) or from a fixture; the harness only
//! loads, wraps and reads. A response body is read whole, except an event stream, which is left unread.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

use axum::body::Body;
use axum::extract::State;
use axum::http::{header, HeaderMap, Method, Request, StatusCode};
use axum::response::IntoResponse;
use axum::Router;
use bytes::Bytes;
use gotcha::{GotchaApp, GotchaContext};
use serde_json::Value;
use tokio::sync::RwLock;
use tower::ServiceExt;
use zhang_core::data_source::{DataSource, LoadResult, LocalFileSystemDataSource};
use zhang_core::data_type::text::ZhangDataType;
use zhang_core::ledger::Ledger;
use zhang_core::ZhangResult;
use zhang_server::broadcast::Broadcaster;
pub use zhang_server::state::{SharedLedger, SharedReloadSender};
pub use zhang_server::{ReloadSender, ServeConfig, ServerApp};

use crate::fixtures::FixtureLedger;

// ---- handler level ----------------------------------------------------------------------------------------------

/// A loaded ledger as the handlers share it.
pub fn shared(ledger: Ledger) -> SharedLedger {
    SharedLedger(Arc::new(RwLock::new(ledger)))
}

/// The `State` a handler reading the ledger takes.
pub fn state(ledger: Ledger) -> State<SharedLedger> {
    State(shared(ledger))
}

/// The `State` a writing handler takes to ask for a reload: a sender nobody listens to, so a write that asks for a
/// reload does not fail for it (the ledger is reloaded by the test when it wants to see the files).
pub fn reload() -> State<SharedReloadSender> {
    let (sender, _receiver) = tokio::sync::mpsc::channel(1);
    State(SharedReloadSender(Arc::new(ReloadSender::new(sender))))
}

/// The two `State`s of a writing handler.
pub fn states(ledger: Ledger) -> (State<SharedLedger>, State<SharedReloadSender>) {
    (state(ledger), reload())
}

/// A fixture ledger as the handlers share it, loaded once per process (by name) and shared read-only between the
/// tests of the process; a test that writes to the ledger takes its own copy ([`crate::ledger::Scratch::copy_of`]).
/// Panics, naming the fixture, when it does not load.
pub fn fixture_shared(fixture: &FixtureLedger) -> SharedLedger {
    static SHARED: OnceLock<Mutex<HashMap<String, SharedLedger>>> = OnceLock::new();
    let mut loaded = SHARED.get_or_init(Default::default).lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    loaded
        .entry(fixture.name.clone())
        .or_insert_with(|| {
            shared(
                fixture
                    .load()
                    .unwrap_or_else(|error| panic!("{}: cannot load the ledger: {error}", fixture.name)),
            )
        })
        .clone()
}

/// A response read whole: its status, headers and body as JSON (`Null` when empty, a `String` when not JSON).
pub struct Answer {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Value,
}

impl Answer {
    /// the `X-Total-Count` header, as the paged account journal sets it
    pub fn total_count(&self) -> Option<u64> {
        self.header("X-Total-Count")
            .map(|it| it.parse().unwrap_or_else(|error| panic!("X-Total-Count {it:?}: {error}")))
    }

    /// a header's value as text
    pub fn header(&self, name: &str) -> Option<String> {
        self.headers.get(name).map(|it| it.to_str().expect("a text header").to_owned())
    }

    /// the content type, empty when there is none
    pub fn content_type(&self) -> String {
        self.header(header::CONTENT_TYPE.as_str()).unwrap_or_default()
    }

    /// `body["data"]`, the payload of a successful answer (`Null` when there is none)
    pub fn data(&self) -> Value {
        self.body.get("data").cloned().unwrap_or(Value::Null)
    }
}

/// A handler's response, read whole.
pub async fn answer(response: impl IntoResponse) -> Answer {
    let response = response.into_response();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = read_body(&headers, response.into_body()).await;
    Answer {
        status,
        headers,
        body: json_or_text(&bytes),
    }
}

/// The status and the JSON body of a handler's response (`Null` when empty, a `String` when not JSON).
pub async fn respond(response: impl IntoResponse) -> (StatusCode, Value) {
    let answer = answer(response).await;
    (answer.status, answer.body)
}

/// The status, the `X-Total-Count` header and the JSON body of a handler's response.
pub async fn respond_with_total(response: impl IntoResponse) -> (StatusCode, Option<u64>, Value) {
    let answer = answer(response).await;
    (answer.status, answer.total_count(), answer.body)
}

/// The JSON body of a response that must be a success; the status and the body are in the message when it is not.
pub async fn body(response: impl IntoResponse) -> Value {
    let answer = answer(response).await;
    assert!(answer.status.is_success(), "{}: {}", answer.status, answer.body);
    answer.body
}

/// `body["data"]` of a response that must be a success.
pub async fn data(response: impl IntoResponse) -> Value {
    let answer = answer(response).await;
    assert!(answer.status.is_success(), "{}: {}", answer.status, answer.body);
    answer.data()
}

// ---- router level -----------------------------------------------------------------------------------------------

/// The authentication settings of a server: none by default.
#[derive(Default, Clone, Debug)]
pub struct Settings {
    pub auth_credential: Option<String>,
    pub passkey_secret: Option<String>,
    pub passkey_rp_id: Option<String>,
    pub passkey_origin: Option<String>,
    pub session_secret: Option<String>,
    pub app_return_schemes: Option<String>,
}

/// The configuration of a server on the ledger `dir/entry` read through `data_source`, on an ephemeral port, with
/// the daily report off.
pub fn serve_config(dir: &Path, entry: &str, data_source: Arc<dyn DataSource>, settings: &Settings) -> ServeConfig {
    ServeConfig {
        path: dir.to_path_buf(),
        endpoint: entry.to_owned(),
        addr: "127.0.0.1".to_owned(),
        port: 0,
        no_report: true,
        data_source,
        auth_credential: settings.auth_credential.clone(),
        passkey_secret: settings.passkey_secret.clone(),
        passkey_rp_id: settings.passkey_rp_id.clone(),
        passkey_origin: settings.passkey_origin.clone(),
        session_secret: settings.session_secret.clone(),
        app_return_schemes: settings.app_return_schemes.clone(),
    }
}

/// The app the server runs for `config`, serving `ledger`, with a reload channel nobody listens to.
pub fn app(config: ServeConfig, ledger: Arc<RwLock<Ledger>>) -> ServerApp {
    let (sender, _receiver) = tokio::sync::mpsc::channel(8);
    zhang_server::create_server_app(config, ledger, Broadcaster::create(), Arc::new(ReloadSender::new(sender)))
}

/// The router of an app, built as the server builds it.
pub async fn router_of(app: ServerApp) -> Router {
    let config = app.config().await.expect("the app's config");
    let state = app.state(&config).await.unwrap_or_else(|error| panic!("the app's state should build: {error}"));
    app.build_router(GotchaContext { config, state }).await.expect("the router builds")
}

/// The router of a server on the ledger `dir/entry`, loaded through `data_source` (see [`RootedFileSystem`] and
/// [`crate::fixtures::data_source_for`]), and the ledger it serves. Panics when the ledger does not load.
pub async fn app_and_ledger(dir: &Path, entry: &str, data_source: Arc<dyn DataSource>, settings: &Settings) -> (ServerApp, Arc<RwLock<Ledger>>) {
    let ledger = Ledger::load(dir.to_path_buf(), entry.to_owned(), data_source.clone()).unwrap_or_else(|error| panic!("ledger should load: {error}"));
    let ledger = Arc::new(RwLock::new(ledger));
    (app(serve_config(dir, entry, data_source, settings), ledger.clone()), ledger)
}

/// The router of a server on the ledger `dir/entry`, loaded through `data_source`.
pub async fn router(dir: &Path, entry: &str, data_source: Arc<dyn DataSource>, settings: &Settings) -> Router {
    router_of(app_and_ledger(dir, entry, data_source, settings).await.0).await
}

/// A local file system data source resolving relative paths against the ledger root, like the opendal one the CLI
/// uses: a missing file reads as empty, saving creates the parent folders. The ledger is read with the zhang parser.
pub struct RootedFileSystem {
    root: std::path::PathBuf,
    inner: LocalFileSystemDataSource,
}

impl RootedFileSystem {
    pub fn new(root: &Path) -> RootedFileSystem {
        RootedFileSystem {
            root: root.to_path_buf(),
            inner: LocalFileSystemDataSource::new(ZhangDataType {}),
        }
    }

    fn resolve(&self, path: &str) -> std::path::PathBuf {
        let path = std::path::PathBuf::from(path);
        if path.is_absolute() {
            path
        } else {
            self.root.join(path)
        }
    }
}

impl DataSource for RootedFileSystem {
    fn get(&self, path: String) -> ZhangResult<Vec<u8>> {
        match std::fs::read(self.resolve(&path)) {
            Ok(content) => Ok(content),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(vec![]),
            Err(e) => Err(e.into()),
        }
    }

    fn load(&self, entry: String, endpoint: String) -> ZhangResult<LoadResult> {
        self.inner.load(entry, endpoint)
    }

    fn save(&self, _ledger: &Ledger, path: String, content: &[u8]) -> ZhangResult<()> {
        let path = self.resolve(&path);
        std::fs::create_dir_all(path.parent().expect("a file has a parent"))?;
        std::fs::write(path, content)?;
        Ok(())
    }
}

/// A response of the router, read whole: the status, the headers, the raw bytes and the body as JSON (`Null` when
/// empty, a `String` when not JSON).
pub struct Reply {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub bytes: Bytes,
    pub body: Value,
}

impl Reply {
    /// the body as JSON; panics, showing the text, when it is not JSON
    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.bytes).unwrap_or_else(|_| panic!("expect JSON, got {:?}", String::from_utf8_lossy(&self.bytes)))
    }

    /// the body as text
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.bytes).into_owned()
    }

    /// a header's value as text
    pub fn header(&self, name: &str) -> Option<String> {
        self.headers.get(name).map(|it| it.to_str().expect("a text header").to_owned())
    }

    /// the content type, empty when there is none
    pub fn content_type(&self) -> String {
        self.header(header::CONTENT_TYPE.as_str()).unwrap_or_default()
    }
}

/// A request sent through the router, as a client would send it.
pub async fn send(router: &Router, request: Request<Body>) -> Reply {
    let response = router.clone().oneshot(request).await.expect("the router answers");
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = read_body(&headers, response.into_body()).await;
    let body = json_or_text(&bytes);
    Reply { status, headers, bytes, body }
}

/// A request with `headers` and, when given, `body` sent as JSON.
pub fn json_request(method: Method, uri: &str, headers: &[(&str, &str)], body: Option<&Value>) -> Request<Body> {
    let mut request = Request::builder().method(method).uri(uri);
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    match body {
        Some(body) => request
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(serde_json::to_vec(body).expect("JSON serialises"))),
        None => request.body(Body::empty()),
    }
    .expect("a valid request")
}

/// `method uri` with `headers` and, when given, `body` as JSON, through the router.
pub async fn call(router: &Router, method: Method, uri: &str, headers: &[(&str, &str)], body: Option<&Value>) -> Reply {
    send(router, json_request(method, uri, headers, body)).await
}

/// `GET uri` through the router.
pub async fn get(router: &Router, uri: &str, headers: &[(&str, &str)]) -> Reply {
    call(router, Method::GET, uri, headers, None).await
}

/// `POST uri` with a JSON body through the router.
pub async fn post(router: &Router, uri: &str, headers: &[(&str, &str)], body: &Value) -> Reply {
    call(router, Method::POST, uri, headers, Some(body)).await
}

/// `PUT uri` with a JSON body through the router.
pub async fn put(router: &Router, uri: &str, headers: &[(&str, &str)], body: &Value) -> Reply {
    call(router, Method::PUT, uri, headers, Some(body)).await
}

// ---- shared -------------------------------------------------------------------------------------------------------

/// The bytes of a body, whole; an event stream, which never ends, is left unread.
async fn read_body(headers: &HeaderMap, body: Body) -> Bytes {
    let streaming = headers
        .get(header::CONTENT_TYPE)
        .is_some_and(|it| it.to_str().is_ok_and(|it| it.starts_with("text/event-stream")));
    if streaming {
        return Bytes::new();
    }
    axum::body::to_bytes(body, usize::MAX).await.expect("the body reads")
}

/// `Null` for no bytes, the JSON value they hold, or the text when they are not JSON.
fn json_or_text(bytes: &Bytes) -> Value {
    if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(bytes).unwrap_or_else(|_| Value::String(String::from_utf8_lossy(bytes).into_owned()))
    }
}
