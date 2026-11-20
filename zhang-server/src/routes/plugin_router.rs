//! `/api/plugins/{name}` and every path below it, for any HTTP method: requests routed to the
//! router plugin of that name. See [`zhang_core::plugin::router`] for the plugin side.
//!
//! The routes sit with the rest of the API, behind the same authentication. A request runs the
//! plugin on a blocking thread, under the ledger read lock. What the plugin answers is sent with only
//! the headers zhang allows and sandboxed (see [`zhang_core::plugin::http::SECURITY_HEADERS`]), and the
//! plugin sees only the request headers zhang forwards
//! ([`zhang_core::plugin::http::FORWARDED_REQUEST_HEADERS`]). A plugin granted `allowed_writes` (within the
//! operator's ceiling) may append directives through `zhang_write_preview` and `zhang_write_commit`
//! ([`crate::plugin_writes`]): the text is in the file before the plugin's call returns, and once the
//! plugin answered the request, the ledger is reloaded as after an API write, and one line is logged
//! for each commit.
//!
//! The routes are bounded by [`PluginRouteLimits`]: the body of a request, and how many requests a
//! plugin handles at once. When the plugin cannot answer, the response is a JSON `{"message": "..."}`
//! with:
//!
//! A plugin that fails after its commit landed (it ran past its timeout, say, while it built its answer) is
//! answered with the status of its failure all the same, since its own answer is lost, but the entries are recorded:
//! the response says so with the [`COMMITTED_HEADER`] header and a `committed` object next to the message, so a
//! client tells the user what was recorded and does not send it again.
//!
//! - 404 when no router plugin has that name
//! - 413 when the request body is larger than [`PluginRouteLimits::max_body`]
//! - 501 when the plugin does not export `router`
//! - 502 when it returns something that is not a valid response
//! - 503, with `Retry-After`, when the plugin already handles [`PluginRouteLimits::max_concurrency`]
//!   requests and either [`PluginRouteLimits::max_waiting`] more wait for their turn or the turn did not
//!   come within [`PluginRouteLimits::max_wait`]
//! - 504 when it runs longer than its timeout
//! - 500 when it traps, returns an error, runs out of memory or cannot be loaded
//!
//! The details go to the log.

use std::collections::{BTreeMap, HashMap};
use std::fmt::{Display, Formatter};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use axum::body::{Body, Bytes};
use axum::extract::{FromRef, State};
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use http_body_util::{BodyExt, LengthLimitError, Limited};
use log::{error, info};
use percent_encoding::{percent_decode_str, utf8_percent_encode, AsciiSet, CONTROLS};
use serde_json::Value;
use tokio::sync::{OwnedRwLockReadGuard, OwnedSemaphorePermit, Semaphore};
use zhang_core::ledger::Ledger;
use zhang_core::plugin::http::PluginRequest;
use zhang_core::plugin::log_safe;
use zhang_core::plugin::router::{QueryFailure, RawQueryParam, RouterError, RouterHost, WriteBatch, WriteCommitted, WritePreview};
use zhang_core::plugin_policy::{PluginPolicy, Size};
use zhang_query::params::bind_json;
use zhang_query::{Params, Query, QueryError};
use zhang_shared::plugin_abi::HostError;

use crate::error::error_response;
use crate::plugin_writes::{self, Commit, PluginWrites};
use crate::response::{QueryResultEntity, QuerySchemaEntity};
use crate::routes::query::{execute_options, max_result_values};
use crate::state::{SharedLedger, SharedPluginWrites, SharedReloadSender};
use crate::ReloadSender;

/// The header of a failed plugin response whose plugin committed a write before it failed: the generation the commit
/// moved the ledger to. The body has the commit as `committed` (`file`, `files`, `bytes`, `entries`, `generation`)
pub const COMMITTED_HEADER: &str = "zhang-plugin-committed";

/// every router plugin's route and the paths below it: `/api/plugins/{name}`, `/api/plugins/{name}/`
/// and `/api/plugins/{name}/{*path}`. The handler splits the path itself, see [`split_route`]
pub const ROUTE: &str = "/api/plugins/*route";

/// what every router plugin's route starts with
const ROUTE_PREFIX: &str = "/api/plugins/";

/// the characters escaped in a plugin name to make it one path segment: the URL path set, plus
/// `/` and `%`
const SEGMENT: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'<')
    .add(b'>')
    .add(b'`')
    .add(b'?')
    .add(b'{')
    .add(b'}')
    .add(b'/')
    .add(b'%');

/// the largest request body a router plugin receives by default, see [`PluginRouteLimits::max_body`]
pub const DEFAULT_MAX_BODY: usize = 4 * 1024 * 1024;

/// how many requests a router plugin handles at once by default, see [`PluginRouteLimits::max_concurrency`]
pub const DEFAULT_MAX_CONCURRENCY: usize = 4;

/// how many requests may wait for a busy router plugin by default, see [`PluginRouteLimits::max_waiting`]
pub const DEFAULT_MAX_WAITING: usize = 16;

/// how long a request waits for a busy router plugin by default, see [`PluginRouteLimits::max_wait`]
pub const DEFAULT_MAX_WAIT: Duration = Duration::from_secs(5);

/// the environment variable setting [`PluginRouteLimits::max_body`]
pub const MAX_BODY_ENV: &str = "ZHANG_PLUGIN_MAX_BODY";

/// the environment variable setting [`PluginRouteLimits::max_concurrency`]
pub const MAX_CONCURRENCY_ENV: &str = "ZHANG_PLUGIN_MAX_CONCURRENCY";

/// how long a client is told to wait before it tries again a plugin that is busy, in seconds
const RETRY_AFTER_SECONDS: &str = "1";

/// What the plugin routes take, set when the server starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PluginRouteLimits {
    /// the largest request body, in bytes, a router plugin receives; a larger one is answered 413 before the plugin
    /// runs. The plugin gets the body inside a JSON document, as text or as base64, so it holds it a few times over:
    /// this keeps a request well inside the plugin's memory limit
    pub max_body: usize,
    /// how many requests one router plugin handles at once. Every request holds a blocking thread, a plugin
    /// instance (up to its memory limit) and the ledger's read lock
    pub max_concurrency: usize,
    /// how many more requests of a router plugin that handles `max_concurrency` wait for their turn, so that an MCP
    /// client's parallel tool calls are not refused; one more is answered 503 with `Retry-After` at once. A waiting
    /// request holds no lock and no thread
    pub max_waiting: usize,
    /// how long a request waits for its turn before it is answered 503 with `Retry-After`
    pub max_wait: Duration,
}

impl Default for PluginRouteLimits {
    fn default() -> Self {
        PluginRouteLimits {
            max_body: DEFAULT_MAX_BODY,
            max_concurrency: DEFAULT_MAX_CONCURRENCY,
            max_waiting: DEFAULT_MAX_WAITING,
            max_wait: DEFAULT_MAX_WAIT,
        }
    }
}

impl Display for PluginRouteLimits {
    /// the limits as the server logs them when it starts
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "router plugins take a body of at most {} ({MAX_BODY_ENV}) and {} requests at once each ({MAX_CONCURRENCY_ENV}), with {} more waiting up to {:?}",
            Size(self.max_body as u64),
            self.max_concurrency,
            self.max_waiting,
            self.max_wait
        )
    }
}

/// The requests of one router plugin: those it handles, and those waiting for their turn.
struct Gate {
    /// [`PluginRouteLimits::max_concurrency`] permits, one per request the plugin handles
    permits: Arc<Semaphore>,
    /// how many requests wait for a permit
    waiting: AtomicUsize,
}

/// one request waiting at a [`Gate`]; it stops counting when it gets its turn, gives up, or its client goes away
struct Waiting(Arc<Gate>);

impl Drop for Waiting {
    fn drop(&mut self) {
        self.0.waiting.fetch_sub(1, Ordering::SeqCst);
    }
}

/// The state of the plugin routes: their limits, and the requests each router plugin handles now.
pub struct PluginRoutes {
    limits: PluginRouteLimits,
    /// a [`Gate`] per router plugin, by name. A name gets one on its first request that a router plugin serves, so
    /// the map holds the router plugins the server served at most
    gates: Mutex<HashMap<String, Arc<Gate>>>,
}

impl PluginRoutes {
    pub fn new(limits: PluginRouteLimits) -> Self {
        PluginRoutes {
            limits,
            gates: Mutex::new(HashMap::new()),
        }
    }

    /// A permit for one more request of the router plugin `name`, held until the plugin returns. When the plugin
    /// already handles as many as it may, the request waits for its turn, as one of at most
    /// [`PluginRouteLimits::max_waiting`] and for at most [`PluginRouteLimits::max_wait`]; `None` when it cannot
    async fn acquire(&self, name: &str) -> Option<OwnedSemaphorePermit> {
        let gate = self
            .gates
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(name.to_owned())
            .or_insert_with(|| {
                Arc::new(Gate {
                    permits: Arc::new(Semaphore::new(self.limits.max_concurrency)),
                    waiting: AtomicUsize::new(0),
                })
            })
            .clone();
        if let Ok(permit) = gate.permits.clone().try_acquire_owned() {
            return Some(permit);
        }
        let waiting = Waiting(gate.clone());
        if gate.waiting.fetch_add(1, Ordering::SeqCst) >= self.limits.max_waiting {
            return None;
        }
        let permit = tokio::time::timeout(self.limits.max_wait, gate.permits.clone().acquire_owned()).await;
        drop(waiting);
        permit.ok()?.ok()
    }
}

/// What the plugin routes read: the ledger, their own state (the limits and the requests each plugin handles now),
/// and what a commit of a plugin's writes needs, the reload sender (the generation, the reload after a commit) and
/// the plugin writes (the token key, the commit lock). The server's [`crate::state::AppState`] gives one; a test
/// serves the routes with one of its own
#[derive(Clone)]
pub struct PluginRouteState {
    pub ledger: SharedLedger,
    pub routes: Arc<PluginRoutes>,
    pub reload_sender: SharedReloadSender,
    pub writes: SharedPluginWrites,
}

impl PluginRouteState {
    /// the state of the plugin routes over `ledger`, with the default limits, `reload_sender` and fresh plugin writes
    /// (a random token key)
    pub fn new(ledger: SharedLedger, reload_sender: Arc<ReloadSender>) -> Self {
        PluginRouteState {
            ledger,
            routes: Arc::new(PluginRoutes::new(PluginRouteLimits::default())),
            reload_sender: SharedReloadSender(reload_sender),
            writes: SharedPluginWrites(Arc::new(PluginWrites::new())),
        }
    }
}

impl FromRef<PluginRouteState> for SharedLedger {
    fn from_ref(state: &PluginRouteState) -> Self {
        state.ledger.clone()
    }
}

impl FromRef<PluginRouteState> for SharedReloadSender {
    fn from_ref(state: &PluginRouteState) -> Self {
        state.reload_sender.clone()
    }
}

/// What the operator lets plugins use, set when the server starts: the policy every plugin of the served ledger runs
/// under, and the limits of the plugin routes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PluginSettings {
    pub policy: PluginPolicy,
    pub routes: PluginRouteLimits,
}

impl Display for PluginSettings {
    /// the settings as the server logs them when it starts
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}; {}", self.policy, self.routes)
    }
}

/// where the router plugin named `name` serves requests: `/api/plugins/{name}`
pub fn route_of(name: &str) -> String {
    format!("{ROUTE_PREFIX}{}", utf8_percent_encode(name, SEGMENT))
}

/// the plugin name and the path below its route in a request path: `/api/plugins/report/by-month`
/// gives `("report", "/by-month")`, and `/api/plugins/report` gives `("report", "/")`. The name is
/// percent-decoded, the path is kept as sent.
fn split_route(path: &str) -> Option<(String, String)> {
    let rest = path.strip_prefix(ROUTE_PREFIX)?;
    let (name, below) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
    let name = percent_decode_str(name).decode_utf8().ok()?.into_owned();
    let below = if below.is_empty() { "/" } else { below };
    Some((name, below.to_owned()))
}

/// the body of a request, read up to `max_body` bytes; `None` when it is larger
async fn read_body(body: Body, max_body: usize) -> Result<Option<Bytes>, Response> {
    match Limited::new(body, max_body).collect().await {
        Ok(collected) => Ok(Some(collected.to_bytes())),
        Err(e) if e.downcast_ref::<LengthLimitError>().is_some() => Ok(None),
        Err(e) => Err(error_response(StatusCode::BAD_REQUEST, format!("the request body cannot be read: {e}"))),
    }
}

/// Route a request to the router plugin its path names.
pub async fn route_to_plugin(State(state): State<PluginRouteState>, method: Method, uri: Uri, headers: HeaderMap, body: Body) -> Response {
    let Some((name, path)) = split_route(uri.path()) else {
        return error_response(StatusCode::NOT_FOUND, "this is not the route of a router plugin".to_owned());
    };
    let max_body = state.routes.limits.max_body;
    let too_large = || {
        error_response(
            StatusCode::PAYLOAD_TOO_LARGE,
            format!(
                "the request body is larger than the {} a router plugin takes ({MAX_BODY_ENV})",
                Size(max_body as u64)
            ),
        )
    };
    // a declared length over the limit is refused before anything is read
    let declared = headers.get(header::CONTENT_LENGTH).and_then(|it| it.to_str().ok()?.parse::<u64>().ok());
    if declared.is_some_and(|length| length > max_body as u64) {
        return too_large();
    }
    let body = match read_body(body, max_body).await {
        Ok(Some(body)) => body,
        Ok(None) => return too_large(),
        Err(response) => return response,
    };
    let query = form_urlencoded::parse(uri.query().unwrap_or_default().as_bytes()).into_owned();
    let headers = headers
        .iter()
        .map(|(name, value)| (name.as_str().to_owned(), String::from_utf8_lossy(value.as_bytes()).into_owned()));
    let request = PluginRequest::new(method.as_str(), path, query, headers, body.to_vec());
    dispatch(state, name, request).await
}

/// the [`RouterHost`] of one request: it owns the ledger read lock until the plugin returns, and what a write of
/// the plugin needs: the reload sender (the generation) and the plugin writes (the token key, the commit lock)
struct ServerRouterHost {
    ledger: OwnedRwLockReadGuard<Ledger>,
    max_result_values: u64,
    reload_sender: Arc<ReloadSender>,
    writes: Arc<PluginWrites>,
    /// what the call committed, for the route handler to act on once the plugin returned. Per call, shared with
    /// the handler only: the compiled plugin and its host functions keep no state of a call
    committed: Arc<Mutex<Option<Commit>>>,
}

/// a failed query as the plugin sees it
fn failure(error: QueryError) -> QueryFailure {
    QueryFailure {
        message: error.message,
        line: error.line,
        column: error.column,
    }
}

/// a problem without a position in the query text
fn unlocated(message: String) -> QueryFailure {
    QueryFailure {
        message,
        line: None,
        column: None,
    }
}

impl ServerRouterHost {
    /// run `query` with `params` bound, with the limits of `POST /api/query`, and encode the
    /// result as that endpoint does
    fn run(&self, query: &Query, params: &Params) -> Result<Value, QueryFailure> {
        let result = query
            .execute_with_options(&self.ledger, params, &execute_options(self.max_result_values))
            .map_err(failure)?;
        serde_json::to_value(QueryResultEntity::from(result)).map_err(|e| unlocated(format!("the result cannot be encoded: {e}")))
    }
}

impl RouterHost for ServerRouterHost {
    /// compiled and run like `POST /api/query`, with the same limits
    fn query(&self, bql: &str) -> Result<Value, QueryFailure> {
        self.run(&Query::compile(bql).map_err(failure)?, &Params::new())
    }

    /// the parameters bound as the built-in queries bind theirs (`builtin::json_params`), then
    /// compiled with their declared types and run like [`RouterHost::query`]
    fn query_with_params(&self, bql: &str, params: &BTreeMap<String, RawQueryParam>) -> Result<Value, QueryFailure> {
        let (types, values) = bind_json(params.iter().map(|(name, param)| (name.as_str(), param.ty.as_str(), &param.value))).map_err(unlocated)?;
        self.run(&Query::compile_with_params(bql, &types).map_err(failure)?, &values)
    }

    /// the schema as `GET /api/query/schema` answers it
    fn schema(&self) -> Result<Value, QueryFailure> {
        serde_json::to_value(QuerySchemaEntity::from(zhang_query::schema())).map_err(|e| unlocated(format!("the schema cannot be encoded: {e}")))
    }

    /// as `POST /api/transactions/preview` previews, see [`plugin_writes::preview`]
    fn write_preview(&self, plugin: &str, batch: &WriteBatch) -> Result<WritePreview, HostError> {
        plugin_writes::preview(&self.ledger, &self.writes, &self.reload_sender, plugin, &batch.directives)
    }

    /// one `append` of the ledger's data source, as `POST /api/transactions` writes, see [`plugin_writes::commit`];
    /// the handler marks the ledger stale and reloads it once the plugin returned
    fn write_commit(&self, plugin: &str, batch: &WriteBatch, token: &str) -> Result<WriteCommitted, HostError> {
        let commit = plugin_writes::commit(&self.ledger, &self.writes, &self.reload_sender, plugin, &batch.directives, token)?;
        let committed = commit.committed.clone();
        *self.committed.lock().unwrap_or_else(PoisonError::into_inner) = Some(commit);
        Ok(committed)
    }
}

async fn dispatch(state: PluginRouteState, name: String, request: PluginRequest) -> Response {
    // the read lock is held for the look-up only: a request waiting for its turn below must not hold it, since a
    // writer queued behind it would keep every other reader of the server waiting too
    if state.ledger.read().await.plugins.router(&name).is_none() {
        return error_response(StatusCode::NOT_FOUND, format!("no router plugin is named {name}"));
    }
    // a busy plugin makes a few requests wait a few seconds for their turn, and answers any more 503 at once
    let Some(permit) = state.routes.acquire(&name).await else {
        let mut response = error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            format!(
                "plugin {name} is busy: it already handles {} requests ({MAX_CONCURRENCY_ENV}); try again later",
                state.routes.limits.max_concurrency
            ),
        );
        response
            .headers_mut()
            .insert(header::RETRY_AFTER, HeaderValue::from_static(RETRY_AFTER_SECONDS));
        return response;
    };
    // the plugin runs under the read lock, so the ledger cannot reload while it reads it; a reload that removed the
    // plugin while the request waited answers 404 below
    let committed = Arc::new(Mutex::new(None));
    let host = Arc::new(ServerRouterHost {
        ledger: state.ledger.0.clone().read_owned().await,
        max_result_values: max_result_values(),
        reload_sender: state.reload_sender.0.clone(),
        writes: state.writes.0.clone(),
        committed: committed.clone(),
    });
    let summary = format!("{} {}", request.method, request.path);
    let plugin_name = name.clone();
    // instantiating and running a plugin is CPU-bound and may block, so it stays off the async workers. The permit
    // goes with the call: a client that goes away does not stop the plugin, so the plugin still counts until it returns
    let result = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let plugin = host.ledger.plugins.router(&plugin_name)?;
        Some(plugin.execute_as_router(&request, &host.ledger, host.clone()))
    })
    .await;
    let logged = log_safe(&name);
    // what the plugin committed is in the files, the write is pending on the reload sender and the reload asked
    // for (the commit did both under the read lock, so a write queued behind this call reloads first): one line
    // of log per commit
    let commit = committed.lock().unwrap_or_else(PoisonError::into_inner).take();
    if let Some(commit) = &commit {
        info!(
            "plugin {logged} committed {} directives ({} bytes) to {} generation {}",
            commit.committed.entries,
            commit.committed.bytes,
            commit.files.join(", "),
            commit.committed.generation
        );
    }
    let (status, message) = match result {
        Ok(Some(Ok(response))) => return response.map(Body::from).into_response(),
        Ok(None) => return error_response(StatusCode::NOT_FOUND, format!("no router plugin is named {name}")),
        Ok(Some(Err(e))) => {
            error!("router plugin {logged} could not answer {}: {e}", log_safe(&summary));
            match e {
                RouterError::NoRouterExport => (StatusCode::NOT_IMPLEMENTED, format!("plugin {name} does not export `router`")),
                RouterError::BadResponse(reason) => (StatusCode::BAD_GATEWAY, format!("plugin {name} returned an invalid response: {reason}")),
                RouterError::Timeout => (StatusCode::GATEWAY_TIMEOUT, format!("plugin {name} ran longer than its timeout")),
                RouterError::Failed(_) => (StatusCode::INTERNAL_SERVER_ERROR, format!("plugin {name} failed while handling the request")),
                RouterError::Load(_) => (StatusCode::INTERNAL_SERVER_ERROR, format!("plugin {name} cannot be loaded")),
            }
        }
        Err(e) => {
            error!("router plugin {logged} could not answer {}: the task failed: {e}", log_safe(&summary));
            (StatusCode::INTERNAL_SERVER_ERROR, format!("plugin {name} failed while handling the request"))
        }
    };
    match commit {
        Some(commit) => failed_after_commit(&name, status, message, &commit),
        None => error_response(status, message),
    }
}

/// The answer of a plugin that failed with `status` and `message` after `commit` landed: its failure, with what was
/// recorded, so the client does not send the entries again. The commit is whole (one write per file, the files of
/// a batch written one after the other, every one of them landed); a commit that failed partway was an `io` error
/// the plugin was given, naming what landed, and is not reported here
fn failed_after_commit(name: &str, status: StatusCode, message: String, commit: &Commit) -> Response {
    let committed = &commit.committed;
    error!(
        "router plugin {} failed after it committed {} directives to {} generation {}: the client is told they are recorded",
        log_safe(name),
        committed.entries,
        commit.files.join(", "),
        committed.generation
    );
    let body = serde_json::json!({
        "message": format!(
            "{message}, after it committed {} directives to {} (generation {}): they are recorded, do not send them again",
            committed.entries,
            commit.files.join(", "),
            committed.generation
        ),
        "committed": {
            "file": committed.file,
            "files": commit.files,
            "bytes": committed.bytes,
            "entries": committed.entries,
            "generation": committed.generation,
        },
    });
    let mut response = (status, axum::Json(body)).into_response();
    response
        .headers_mut()
        .insert(COMMITTED_HEADER, axum::http::HeaderValue::from(committed.generation));
    response
}

#[cfg(test)]
mod test {
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use axum::body::Body;
    use axum::http::{header, Request, StatusCode};
    use axum::response::Response;
    use axum::routing::{any, get};
    use axum::Router;
    use serde_json::{json, Value};
    use tokio::sync::RwLock;
    use tower::ServiceExt;
    use zhang_core::clock::Clock;
    use zhang_core::data_source::LocalFileSystemDataSource;
    use zhang_core::data_type::text::ZhangDataType;
    use zhang_core::data_type::DataType;
    use zhang_core::ledger::{Ledger, LedgerProcessContext};
    use zhang_core::plugin::router::{RawQueryParam, RouterHost};

    use super::{route_of, route_to_plugin, split_route, PluginRouteLimits, PluginRouteState, PluginRoutes, ServerRouterHost, ROUTE};
    use crate::plugin_writes::PluginWrites;
    use crate::request::BuiltinParamValue;
    use crate::response::QuerySchemaEntity;
    use crate::routes::plugin::plugin_list;
    use crate::state::SharedLedger;
    use crate::{builtin, ReloadSender};

    /// the WAT fixtures of zhang-core's plugin tests, see `zhang-core/tests/wasm_plugins.rs`
    const FIXTURES: [(&str, &str); 8] = [
        ("router.wat", include_str!("../../../zhang-core/tests/plugins/router.wat")),
        ("router_query.wat", include_str!("../../../zhang-core/tests/plugins/router_query.wat")),
        (
            "router_query_params.wat",
            include_str!("../../../zhang-core/tests/plugins/router_query_params.wat"),
        ),
        ("router_no_export.wat", include_str!("../../../zhang-core/tests/plugins/router_no_export.wat")),
        ("router_malformed.wat", include_str!("../../../zhang-core/tests/plugins/router_malformed.wat")),
        ("router_trap.wat", include_str!("../../../zhang-core/tests/plugins/router_trap.wat")),
        ("router_loop.wat", include_str!("../../../zhang-core/tests/plugins/router_loop.wat")),
        ("router_hostile.wat", include_str!("../../../zhang-core/tests/plugins/router_hostile.wat")),
    ];

    /// the `timeout` of the looping fixture
    const LOOP_TIMEOUT: Duration = Duration::from_millis(500);

    /// a Lua router plugin, served next to the WAT fixtures: it echoes the request and reads the ledger through
    /// `zhang.query`, `zhang.query_params` and `zhang.ledger_info`, and reports a failed query's message
    const LUA_ROUTER: &str = r#"
local M = { name = "lua-report", version = "0.3.0" }

function M.router(request)
  local rows = zhang.query("SELECT account, sum(position) AS balance GROUP BY account ORDER BY account")
  local bound = zhang.query_params("SELECT date, narration WHERE date >= :from AND narration = :who AND number > :min", {
    from = zhang.date("2024-01-01"), who = zhang.str("lunch"), min = zhang.decimal("5.5"),
  })
  local ok, failure = pcall(zhang.query, "SELECT nope")
  return {
    status = 201,
    headers = { ["content-language"] = "lua" },
    json = { request = request, rows = rows, bound = bound, failure = failure, info = zhang.ledger_info() },
  }
end

return M
"#;

    /// A scratch ledger directory under the system temp dir, removed on drop.
    struct ScratchDir(PathBuf);

    impl Drop for ScratchDir {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).ok();
        }
    }

    /// the plugin list and the plugin routes, as the server registers them, over a ledger declaring every fixture
    async fn app() -> (ScratchDir, Router) {
        app_with(PluginRouteLimits::default()).await
    }

    /// [`app`] with the plugin routes bounded by `limits`
    async fn app_with(limits: PluginRouteLimits) -> (ScratchDir, Router) {
        let dir = ScratchDir(std::env::temp_dir().join(format!("zhang-plugin-router-{}", uuid::Uuid::new_v4())));
        std::fs::create_dir_all(&dir.0).unwrap();
        let mut content = "option \"features.plugin\" \"true\"\noption \"title\" \"Home\"\noption \"timezone\" \"Asia/Shanghai\"\n".to_owned();
        for (fixture, wat) in FIXTURES {
            std::fs::write(dir.0.join(fixture), wat).unwrap();
            // a local ledger resolves a module against the working directory, so declare them by absolute path
            content.push_str(&format!("plugin \"{}\"\n", dir.0.join(fixture).display()));
            if fixture == "router_loop.wat" {
                content.push_str(&format!("  timeout: \"{}ms\"\n", LOOP_TIMEOUT.as_millis()));
            }
        }
        std::fs::write(dir.0.join("report.lua"), LUA_ROUTER).unwrap();
        content.push_str(&format!("plugin \"{}\"\n", dir.0.join("report.lua").display()));
        content.push_str(
            "1970-01-01 open Assets:Cash\n1970-01-01 open Expenses:Food\n\
             2024-01-02 * \"lunch\"\n  Assets:Cash -10 CNY\n  Expenses:Food 10 CNY\n",
        );
        std::fs::write(dir.0.join("main.zhang"), content).unwrap();
        let source = Arc::new(LocalFileSystemDataSource::new(ZhangDataType {}));
        let ledger = Ledger::load(dir.0.clone(), "main.zhang".to_owned(), source).unwrap_or_else(|error| panic!("ledger should load: {error}"));
        let state = PluginRouteState {
            routes: Arc::new(PluginRoutes::new(limits)),
            ..detached_state(ledger)
        };
        let router = Router::new()
            .route("/api/plugins", get(plugin_list))
            .route(ROUTE, any(route_to_plugin))
            .with_state(state);
        (dir, router)
    }

    /// the state of the plugin routes over `ledger`, with a reload channel nobody listens to
    fn detached_state(ledger: Ledger) -> PluginRouteState {
        let (sender, _receiver) = tokio::sync::mpsc::channel(8);
        PluginRouteState::new(SharedLedger(Arc::new(RwLock::new(ledger))), Arc::new(ReloadSender::new(sender)))
    }

    async fn send(app: &Router, request: Request<Body>) -> Response {
        app.clone().oneshot(request).await.unwrap()
    }

    async fn json_body(response: Response) -> Value {
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&body).unwrap_or_else(|e| panic!("not JSON ({e}): {}", String::from_utf8_lossy(&body)))
    }

    #[tokio::test]
    async fn get_and_post_round_trip_through_the_plugin() {
        let (_dir, app) = app().await;

        let response = send(
            &app,
            Request::get("/api/plugins/router-echo/report/by%20month?k=v1&k=v2&q=a+b%26c")
                .header("X-Custom", "yes")
                .header(header::ACCEPT, "text/html")
                .header(header::ORIGIN, "http://localhost:8000")
                .header("Mcp-Session-Id", "s1")
                .header(header::AUTHORIZATION, "Basic dXNlcjpwYXNz")
                .header(header::COOKIE, "zhang_session=secret")
                .header("Cf-Access-Jwt-Assertion", "eyJ.secret")
                .header("X-Auth-Request-Email", "owner@example.com")
                .header("Tailscale-User-Login", "owner@example.com")
                .header("X-Forwarded-For", "10.0.0.1")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::CREATED);
        assert!(response.headers().get("x-echo").is_none(), "a header zhang does not allow is dropped");
        assert_eq!(response.headers()[header::CONTENT_TYPE], "application/json");
        assert_eq!(
            response.headers()[header::CONTENT_SECURITY_POLICY],
            "sandbox allow-scripts allow-forms allow-popups allow-downloads"
        );
        assert_eq!(
            json_body(response).await,
            json!({
                "method": "GET",
                "path": "/report/by%20month",
                "query": {"k": ["v1", "v2"], "q": ["a b&c"]},
                // only the allowlisted headers reach the plugin: no credentials, no proxy identity
                "headers": {"accept": "text/html", "origin": "http://localhost:8000", "mcp-session-id": "s1"},
                "body": "",
                "body_encoding": "utf8",
            })
        );

        let response = send(
            &app,
            Request::post("/api/plugins/router-echo")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"note": "say \"hi\""}"#))
                .unwrap(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::CREATED);
        assert_eq!(
            json_body(response).await,
            json!({
                "method": "POST",
                "path": "/",
                "query": {},
                "headers": {"content-type": "application/json"},
                "body": r#"{"note": "say \"hi\""}"#,
                "body_encoding": "utf8",
            })
        );

        let response = send(&app, Request::put("/api/plugins/router-echo/").body(Body::from(vec![0xff, 0xfe])).unwrap()).await;
        let body = json_body(response).await;
        assert_eq!((&body["method"], &body["path"]), (&json!("PUT"), &json!("/")));
        assert_eq!((&body["body"], &body["body_encoding"]), (&json!("//4="), &json!("base64")));
    }

    #[tokio::test]
    async fn the_plugin_can_query_the_ledger() {
        let (_dir, app) = app().await;

        let response = send(&app, Request::get("/api/plugins/router-query/").body(Body::empty()).unwrap()).await;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            json_body(response).await,
            json!([
                {"Ok": {
                    "columns": [{"name": "account", "type": "str"}, {"name": "balance", "type": "inventory"}],
                    "rows": [
                        ["Assets:Cash", {"positions": [{"units": {"number": "-10", "currency": "CNY"}, "cost": null}]}],
                        ["Expenses:Food", {"positions": [{"units": {"number": "10", "currency": "CNY"}, "cost": null}]}],
                    ],
                }},
                {"Ok": {"title": "Home", "operating_currency": "CNY", "timezone": "Asia/Shanghai"}},
            ])
        );
    }

    /// what the `router-query-params` fixture answers for the path `/{case}`: the raw result of the
    /// host function it calls for that case (see the fixture for the cases)
    async fn query_params_case(app: &Router, case: char) -> Value {
        let response = send(
            app,
            Request::get(format!("/api/plugins/router-query-params/{case}")).body(Body::empty()).unwrap(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        json_body(response).await
    }

    #[tokio::test]
    async fn the_plugin_can_bind_query_parameters_and_read_the_schema() {
        let (_dir, app) = app().await;

        // `SELECT date, narration WHERE date >= :from AND narration = :who AND number > :min`, with
        // `:from` 2024-01-01, `:who` "lunch" and `:min` 5.5: the one lunch posting of 10 CNY
        assert_eq!(
            query_params_case(&app, 'a').await,
            json!({"Ok": {
                "columns": [{"name": "date", "type": "date"}, {"name": "narration", "type": "str"}],
                "rows": [["2024-01-02", "lunch"]],
            }})
        );
        // `narration = :who` with `:who` NULL matches nothing
        assert_eq!(
            query_params_case(&app, 'b').await,
            json!({"Ok": {"columns": [{"name": "date", "type": "date"}], "rows": []}})
        );
        // a parameter the query uses but the input does not bind fails where the query uses it
        assert_eq!(
            query_params_case(&app, 'e').await,
            json!({"Err": {"kind": "query", "message": "parameter :from is not bound", "line": 1, "column": 27}})
        );
        assert_eq!(
            query_params_case(&app, 's').await,
            json!({"Ok": serde_json::to_value(QuerySchemaEntity::from(zhang_query::schema())).unwrap()})
        );
    }

    #[tokio::test]
    async fn a_bad_plugin_query_parameter_fails_as_a_builtin_parameter_does() {
        let (_dir, app) = app().await;

        // `{"type": "date", "value": 20240101}`: a date given as a number
        let plugin = query_params_case(&app, 'd').await;
        let builtin = builtin::json_params(
            builtin::get("postings.between").unwrap(),
            [
                ("from".to_owned(), Some(BuiltinParamValue::Int(20240101))),
                ("to".to_owned(), Some(BuiltinParamValue::Text("2024-01-31".to_owned()))),
            ]
            .into_iter()
            .collect(),
        )
        .unwrap_err();
        assert_eq!(builtin.to_string(), "parameter :from must be a date string YYYY-MM-DD or null");
        assert_eq!(
            plugin,
            json!({"Err": {"kind": "query", "message": builtin.to_string(), "line": null, "column": null}})
        );
        assert_eq!(
            query_params_case(&app, 'c').await,
            json!({"Err": {
                "kind": "query",
                "message": "parameter :who has the type \"money\", which is not one of bool, int, decimal, str, date, set",
                "line": null,
                "column": null,
            }})
        );
    }

    #[tokio::test]
    async fn plugins_that_cannot_answer_get_a_json_error() {
        let (_dir, app) = app().await;

        for (path, status, message) in [
            ("/api/plugins/nobody/x", StatusCode::NOT_FOUND, "no router plugin is named nobody"),
            (
                "/api/plugins/router-no-export",
                StatusCode::NOT_IMPLEMENTED,
                "plugin router-no-export does not export `router`",
            ),
            (
                "/api/plugins/router-malformed/x",
                StatusCode::BAD_GATEWAY,
                "plugin router-malformed returned an invalid response: expected ident at line 1 column 2",
            ),
            (
                "/api/plugins/router-trap/x",
                StatusCode::INTERNAL_SERVER_ERROR,
                "plugin router-trap failed while handling the request",
            ),
        ] {
            let response = send(&app, Request::get(path).body(Body::empty()).unwrap()).await;
            assert_eq!(response.status(), status, "{path}");
            assert_eq!(response.headers()[header::CONTENT_TYPE], "application/json", "{path}");
            assert_eq!(json_body(response).await, json!({ "message": message }), "{path}");
        }

        // the server keeps serving after a plugin trapped
        let response = send(&app, Request::get("/api/plugins/router-echo").body(Body::empty()).unwrap()).await;
        assert_eq!(response.status(), StatusCode::CREATED);
        // and the plugin routes leave the plugin list alone
        let response = send(&app, Request::get("/api/plugins").body(Body::empty()).unwrap()).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(json_body(response).await["data"].as_array().unwrap().len(), FIXTURES.len() + 1);
    }

    #[tokio::test]
    async fn a_lua_router_is_served_and_listed_under_the_scripts_own_name() {
        let (_dir, app) = app().await;

        // the listing shows the name and version the script declares, with its route
        let response = send(&app, Request::get("/api/plugins").body(Body::empty()).unwrap()).await;
        let listed = json_body(response).await["data"]
            .as_array()
            .unwrap()
            .iter()
            .find(|plugin| plugin["name"] == "lua-report")
            .cloned()
            .expect("the Lua router is listed");
        assert_eq!(
            listed,
            json!({"name": "lua-report", "version": "0.3.0", "plugin_type": ["Router"], "capabilities": {"allowed_hosts": []}, "route": "/api/plugins/lua-report", "writes_requested": [], "writes_granted": []})
        );

        let response = send(
            &app,
            Request::get("/api/plugins/lua-report/report/by%20month?k=v1&k=v2&q=a+b%26c")
                .header("X-Custom", "yes")
                .header("Accept-Language", "de")
                .header(header::AUTHORIZATION, "Basic dXNlcjpwYXNz")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::CREATED);
        assert_eq!(response.headers()["content-language"], "lua");
        assert_eq!(response.headers()[header::CONTENT_TYPE], "application/json; charset=utf-8");
        let body = json_body(response).await;
        assert_eq!(
            body["request"],
            json!({
                "method": "GET",
                "path": "/report/by%20month",
                "query": {"k": ["v1", "v2"], "q": ["a b&c"]},
                // only the allowed headers reach the plugin: never the credentials, nor a custom one
                "headers": {"accept-language": "de"},
                "body": "",
                "body_encoding": "utf8",
            })
        );
        // the ledger is read through the same host as a wasm router, with the same limits
        assert_eq!(
            body["rows"],
            json!({
                "columns": [{"name": "account", "type": "str"}, {"name": "balance", "type": "inventory"}],
                "rows": [
                    ["Assets:Cash", {"positions": [{"units": {"number": "-10", "currency": "CNY"}, "cost": null}]}],
                    ["Expenses:Food", {"positions": [{"units": {"number": "10", "currency": "CNY"}, "cost": null}]}],
                ],
            })
        );
        assert_eq!(
            body["bound"],
            json!({
                "columns": [{"name": "date", "type": "date"}, {"name": "narration", "type": "str"}],
                "rows": [["2024-01-02", "lunch"]],
            })
        );
        assert!(
            body["failure"].as_str().unwrap().starts_with("query error at line 1, column 8: "),
            "{}",
            body["failure"]
        );
        assert_eq!(body["info"], json!({"title": "Home", "operating_currency": "CNY", "timezone": "Asia/Shanghai"}));
    }

    #[tokio::test]
    async fn a_looping_plugin_answers_504_at_its_timeout() {
        let (_dir, app) = app().await;
        let started = Instant::now();

        let response = send(&app, Request::get("/api/plugins/router-loop/x").body(Body::empty()).unwrap()).await;

        let elapsed = started.elapsed();
        assert_eq!(response.status(), StatusCode::GATEWAY_TIMEOUT);
        assert_eq!(json_body(response).await, json!({"message": "plugin router-loop ran longer than its timeout"}));
        assert!(
            elapsed >= LOOP_TIMEOUT && elapsed < LOOP_TIMEOUT * 20,
            "answered after {elapsed:?}, not at the {LOOP_TIMEOUT:?} timeout"
        );
    }

    #[tokio::test]
    async fn plugin_queries_fail_as_api_queries_do() {
        let content = "1970-01-01 open Assets:Cash\n1970-01-01 open Expenses:Food\n\
                       2024-01-02 * \"lunch\"\n  Assets:Cash -10 CNY\n  Expenses:Food 10 CNY\n";
        let ledger = Ledger::process(LedgerProcessContext {
            directives: ZhangDataType {}.transform(content.to_owned(), None).unwrap(),
            entry: (PathBuf::from("."), "main.zhang".to_owned()),
            dialect: zhang_core::data_type::Dialect::Zhang,
            visited_files: vec![],
            data_source: Arc::new(LocalFileSystemDataSource::new(ZhangDataType {})),
            clock: Clock::System,
        })
        .unwrap();
        let ledger = Arc::new(RwLock::new(ledger));
        let (sender, _receiver) = tokio::sync::mpsc::channel(8);
        let reload_sender = Arc::new(ReloadSender::new(sender));
        let writes = Arc::new(PluginWrites::new());
        let host = |max_result_values| ServerRouterHost {
            ledger: ledger.clone().try_read_owned().unwrap(),
            max_result_values,
            reload_sender: reload_sender.clone(),
            writes: writes.clone(),
            committed: Arc::new(std::sync::Mutex::new(None)),
        };

        let failure = host(100).query("SELECT nope").unwrap_err();
        assert_eq!((failure.line, failure.column), (Some(1), Some(8)));
        // two postings of two cells each are four values
        let failure = host(3).query("SELECT account, position").unwrap_err();
        assert!(failure.message.starts_with("the result is too large"), "{}", failure.message);
        assert_eq!((failure.line, failure.column), (None, None));
        assert_eq!(
            host(4).query("SELECT account ORDER BY account").unwrap(),
            json!({"columns": [{"name": "account", "type": "str"}], "rows": [["Assets:Cash"], ["Expenses:Food"]]})
        );

        // with parameters: bound by name with their declared types, under the same limits
        let params = |pairs: &[(&str, &str, Value)]| {
            pairs
                .iter()
                .map(|(name, ty, value)| {
                    (
                        name.to_string(),
                        RawQueryParam {
                            ty: ty.to_string(),
                            value: value.clone(),
                        },
                    )
                })
                .collect::<std::collections::BTreeMap<_, _>>()
        };
        assert_eq!(
            host(4)
                .query_with_params(
                    "SELECT account WHERE account ~ :re AND date >= :from ORDER BY account",
                    &params(&[("re", "str", json!("^Exp")), ("from", "date", json!("2024-01-01"))])
                )
                .unwrap(),
            json!({"columns": [{"name": "account", "type": "str"}], "rows": [["Expenses:Food"]]})
        );
        let failure = host(4)
            .query_with_params("SELECT account WHERE date >= :from", &params(&[("from", "date", json!("soon"))]))
            .unwrap_err();
        assert_eq!(failure.message, "parameter :from must be a date string YYYY-MM-DD or null");
        assert_eq!((failure.line, failure.column), (None, None));
        let failure = host(4).query_with_params("SELECT account WHERE date >= :from", &params(&[])).unwrap_err();
        assert_eq!(failure.message, "parameter :from is not bound");
        assert_eq!((failure.line, failure.column), (Some(1), Some(30)));
        let failure = host(3)
            .query_with_params(
                "SELECT account, position WHERE date >= :from",
                &params(&[("from", "date", json!("2024-01-01"))]),
            )
            .unwrap_err();
        assert!(failure.message.starts_with("the result is too large"), "{}", failure.message);
    }

    #[tokio::test]
    async fn a_hostile_response_is_sent_sandboxed_without_its_headers() {
        let (_dir, app) = app().await;

        let response = send(&app, Request::get("/api/plugins/router-hostile/").body(Body::empty()).unwrap()).await;

        assert_eq!(response.status(), StatusCode::OK);
        let headers = response
            .headers()
            .iter()
            .filter(|(name, _)| *name != header::CONTENT_LENGTH)
            .map(|(name, value)| format!("{name}: {}", value.to_str().unwrap()))
            .collect::<Vec<_>>();
        assert_eq!(
            headers,
            vec![
                "cache-control: private, no-store",
                "content-type: text/html; charset=utf-8",
                "content-security-policy: sandbox allow-scripts allow-forms allow-popups allow-downloads",
                "x-content-type-options: nosniff",
            ]
        );
    }

    #[tokio::test]
    async fn a_body_over_the_limit_is_refused_before_the_plugin_runs() {
        let (_dir, app) = app_with(PluginRouteLimits {
            max_body: 1024,
            ..PluginRouteLimits::default()
        })
        .await;

        // at the limit, the plugin answers
        let response = send(&app, Request::post("/api/plugins/router-echo").body(Body::from(vec![b'a'; 1024])).unwrap()).await;
        assert_eq!(response.status(), StatusCode::CREATED);

        // one byte more, declared or streamed, is 413
        let expected = json!({"message": "the request body is larger than the 1 KiB a router plugin takes (ZHANG_PLUGIN_MAX_BODY)"});
        let response = send(&app, Request::post("/api/plugins/router-echo").body(Body::from(vec![b'a'; 1025])).unwrap()).await;
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
        assert_eq!(json_body(response).await, expected);
        let chunks = futures_util::stream::iter([Ok::<_, std::io::Error>(vec![b'a'; 1000]), Ok(vec![b'a'; 25])]);
        let response = send(&app, Request::post("/api/plugins/router-echo").body(Body::from_stream(chunks)).unwrap()).await;
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
        assert_eq!(json_body(response).await, expected);
        let response = send(
            &app,
            Request::post("/api/plugins/router-echo")
                .header(header::CONTENT_LENGTH, "1000000000")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }

    /// send `path` of `app` in the background
    fn spawn_get(app: &Router, path: &'static str) -> tokio::task::JoinHandle<Response> {
        let app = app.clone();
        tokio::spawn(async move { send(&app, Request::get(path).body(Body::empty()).unwrap()).await })
    }

    #[tokio::test]
    async fn a_busy_plugin_queues_a_few_requests_and_answers_more_503_with_retry_after() {
        let (_dir, app) = app_with(PluginRouteLimits {
            max_concurrency: 1,
            max_waiting: 1,
            max_wait: LOOP_TIMEOUT * 4,
            ..PluginRouteLimits::default()
        })
        .await;

        // the looping plugin handles the first request until its timeout, and the second waits for its turn
        let first = spawn_get(&app, "/api/plugins/router-loop/a");
        tokio::time::sleep(LOOP_TIMEOUT / 5).await;
        let second = spawn_get(&app, "/api/plugins/router-loop/b");
        tokio::time::sleep(LOOP_TIMEOUT / 5).await;

        // a third finds the queue full
        let started = Instant::now();
        let busy = send(&app, Request::get("/api/plugins/router-loop/c").body(Body::empty()).unwrap()).await;
        assert_eq!(busy.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert!(started.elapsed() < LOOP_TIMEOUT, "refused at once, after {:?}", started.elapsed());
        assert_eq!(busy.headers()[header::RETRY_AFTER], "1");
        assert_eq!(
            json_body(busy).await,
            json!({"message": "plugin router-loop is busy: it already handles 1 requests (ZHANG_PLUGIN_MAX_CONCURRENCY); try again later"})
        );
        // another plugin is not held up
        let response = send(&app, Request::get("/api/plugins/router-echo").body(Body::empty()).unwrap()).await;
        assert_eq!(response.status(), StatusCode::CREATED);

        // the waiting request got its turn once the first was over, and ran to its own timeout
        assert_eq!(first.await.unwrap().status(), StatusCode::GATEWAY_TIMEOUT);
        assert_eq!(second.await.unwrap().status(), StatusCode::GATEWAY_TIMEOUT);
        let response = send(&app, Request::get("/api/plugins/router-loop/d").body(Body::empty()).unwrap()).await;
        assert_eq!(response.status(), StatusCode::GATEWAY_TIMEOUT);
    }

    #[tokio::test]
    async fn a_request_whose_turn_does_not_come_in_time_is_answered_503() {
        let wait = LOOP_TIMEOUT / 5;
        let (_dir, app) = app_with(PluginRouteLimits {
            max_concurrency: 1,
            max_wait: wait,
            ..PluginRouteLimits::default()
        })
        .await;
        let first = spawn_get(&app, "/api/plugins/router-loop/a");
        tokio::time::sleep(LOOP_TIMEOUT / 10).await;

        let started = Instant::now();
        let busy = send(&app, Request::get("/api/plugins/router-loop/b").body(Body::empty()).unwrap()).await;

        assert_eq!(busy.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(busy.headers()[header::RETRY_AFTER], "1");
        let waited = started.elapsed();
        assert!(waited >= wait && waited < LOOP_TIMEOUT, "answered after {waited:?}, not after waiting {wait:?}");
        assert_eq!(first.await.unwrap().status(), StatusCode::GATEWAY_TIMEOUT);
    }

    #[test]
    fn a_route_names_the_plugin_and_the_path_below_it() {
        assert_eq!(split_route("/api/plugins/report"), Some(("report".to_owned(), "/".to_owned())));
        assert_eq!(split_route("/api/plugins/report/"), Some(("report".to_owned(), "/".to_owned())));
        assert_eq!(
            split_route("/api/plugins/my%20report/a/b%2Fc"),
            Some(("my report".to_owned(), "/a/b%2Fc".to_owned()))
        );
        assert_eq!(split_route("/api/other"), None);

        assert_eq!(route_of("report"), "/api/plugins/report");
        assert_eq!(route_of("my report/2024%"), "/api/plugins/my%20report%2F2024%25");
        let name = "my report/2024%";
        assert_eq!(split_route(&route_of(name)).map(|(name, _)| name).as_deref(), Some(name));
    }
}
