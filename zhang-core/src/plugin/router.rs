//! Router plugins: HTTP endpoints a WASM plugin defines.
//!
//! A plugin declaring the `Router` type serves `/api/plugins/{name}` and every path below it, for
//! any HTTP method, behind the same authentication as the rest of zhang's API. `{name}` is what the
//! plugin's `name` export returns, the name `/api/plugins` lists. When two router plugins share a
//! name, the first one declared serves it.
//!
//! # ABI
//!
//! - The plugin exports `router`. Its input is a [`PluginRequest`] as JSON, its output a
//!   [`PluginResponse`] as JSON. The response may have any content type, so a router can answer
//!   JSON for a script or a whole HTML page.
//! - It may import these host functions from `extism:host/user`. Each one returns the offset of a
//!   kernel memory block holding JSON, either `{"Ok": value}` or
//!   `{"Err": {"kind": "...", "message": "..."}}`. They report problems as values and never trap.
//!   - `zhang_query(bql: i64) -> i64` runs a read-only BQL query, given as the offset of a block
//!     holding its text, over the ledger being served. The value is
//!     `{"columns": [{"name", "type"}], "rows": [[cell]]}`, the `data` that `POST /api/query`
//!     answers, with the same time and result size limits. A query that fails gives the kind
//!     `query` with `message`, `line` and `column`.
//!   - `zhang_ledger_info() -> i64` gives
//!     `{"title": "..." or null, "operating_currency": "CNY", "timezone": "Asia/Shanghai"}`.
//!
//!   Both are only useful while `router` runs. They are linked into every other instance of a
//!   plugin too (registration, `processor`, `mapper`), so a plugin importing them still loads and
//!   runs as its other types; there they give the kind `unavailable`. A call whose argument is not
//!   a readable UTF-8 block gives the kind `invalid_input`.
//! - `zhang_emit_error` ([`crate::plugin::host`]) is linked into a router call too, so a plugin
//!   importing it can serve requests. A request has no error list to report into, so what the
//!   plugin reports is logged as a warning and otherwise dropped; the call still never traps. A
//!   router tells its caller about a problem in its response instead.
//!
//! # Isolation
//!
//! Every request runs in a new instance of the plugin, so no state survives between requests. The
//! instance gets the same config, `allowed_hosts` and timeout as the plugin's processor: a call
//! running longer than the timeout is stopped and answered as [`RouterError::Timeout`]. The caller
//! holds the ledger's read lock for the whole call, so the ledger cannot reload under the plugin,
//! and no host function can change the ledger.

use std::fmt::{Display, Formatter};
use std::sync::{Arc, PoisonError};

use extism::{CurrentPlugin, Function, Plugin as WasmPlugin, UserData, Val, EXTISM_USER_MODULE, PTR};
use log::{debug, warn};
use serde::Serialize;
use serde_json::{json, Value};

use crate::clock::LoadClock;
use crate::ledger::Ledger;
use crate::plugin::host::{read_input_str, MESSAGE_META};
use crate::plugin::http::{PluginRequest, PluginResponse};
use crate::plugin::store::{PluginStore, RegisteredPlugin};

/// the export handling a router plugin's requests
pub const ROUTER_EXPORT: &str = "router";
/// the host function running a BQL query over the ledger being served
pub const QUERY_FUNCTION: &str = "zhang_query";
/// the host function describing the ledger being served
pub const LEDGER_INFO_FUNCTION: &str = "zhang_ledger_info";

/// the most bytes `zhang_query` will read for a plugin's BQL text. A query is human-written SQL-like
/// text, so one mebibyte is far beyond any real query while capping what a single call can read.
const QUERY_MAX_LEN: usize = 1024 * 1024;

/// a query that failed to parse, compile or run
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryFailure {
    pub message: String,
    /// where in the query text the problem is, when it has a position
    pub line: Option<usize>,
    pub column: Option<usize>,
}

/// what the server lends a router call, read-only.
///
/// zhang-core cannot run queries itself (the query engine depends on it), so the server implements
/// this and owns the ledger read lock for as long as the call holds the host.
pub trait RouterHost: Send + Sync {
    /// run a BQL query over the ledger being served, with the limits of `POST /api/query`.
    /// The value is the `data` of that endpoint: `{"columns": [...], "rows": [...]}`
    fn query(&self, bql: &str) -> Result<Value, QueryFailure>;
}

/// why a router plugin could not answer a request
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouterError {
    /// the plugin does not export `router`
    NoRouterExport,
    /// the plugin could not be instantiated
    Load(String),
    /// the `router` call trapped, or the plugin returned an error
    Failed(String),
    /// the `router` call ran longer than the plugin's timeout and was stopped
    Timeout,
    /// the plugin returned something that is not a valid [`PluginResponse`]
    BadResponse(String),
}

impl Display for RouterError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            RouterError::NoRouterExport => write!(f, "the plugin does not export `{ROUTER_EXPORT}`"),
            RouterError::Load(e) => write!(f, "the plugin cannot be loaded: {e}"),
            RouterError::Failed(e) => write!(f, "the plugin failed: {e}"),
            RouterError::Timeout => write!(f, "the plugin ran longer than its timeout"),
            RouterError::BadResponse(e) => write!(f, "the plugin returned an invalid response: {e}"),
        }
    }
}

impl std::error::Error for RouterError {}

/// an error a host function returns as a value, as `{"kind": "...", "message": "...", ...}`
#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum HostError {
    Query {
        message: String,
        line: Option<usize>,
        column: Option<usize>,
    },
    InvalidInput {
        message: String,
    },
    Unavailable {
        message: String,
    },
}

/// what the host functions of one instance can reach; nothing outside a router call
#[derive(Default)]
struct RouterCall {
    host: Option<Arc<dyn RouterHost>>,
    ledger_info: Option<Value>,
}

/// the router host functions bound to `call`
fn host_functions(call: RouterCall) -> Vec<Function> {
    let call = UserData::new(call);
    vec![
        Function::new(QUERY_FUNCTION, [PTR], [PTR], call.clone(), zhang_query).with_namespace(EXTISM_USER_MODULE),
        Function::new(LEDGER_INFO_FUNCTION, [], [PTR], call, zhang_ledger_info).with_namespace(EXTISM_USER_MODULE),
    ]
}

/// the router host functions for an instance that is not handling a request: they answer
/// `{"Err": {"kind": "unavailable"}}`. Linked into every other instance, so a plugin importing them
/// still loads.
pub fn unavailable_host_functions() -> Vec<Function> {
    host_functions(RouterCall::default())
}

fn unavailable(function: &str) -> HostError {
    HostError::Unavailable {
        message: format!("{function} is only available while the plugin handles a request as a router"),
    }
}

/// write `result` into a new memory block as `{"Ok": ...}` or `{"Err": ...}` and return its offset
fn answer(plugin: &mut CurrentPlugin, outputs: &mut [Val], result: Result<Value, HostError>) -> Result<(), extism::Error> {
    let handle = plugin.memory_new(serde_json::to_string(&result)?)?;
    outputs[0] = plugin.memory_to_val(handle);
    Ok(())
}

/// `zhang_query(bql) -> result`
fn zhang_query(plugin: &mut CurrentPlugin, inputs: &[Val], outputs: &mut [Val], call: UserData<RouterCall>) -> Result<(), extism::Error> {
    // clone the host out, so the lock is not held while the query runs
    let host = call.get()?.lock().unwrap_or_else(PoisonError::into_inner).host.clone();
    // read the query text through the bounds-checked helper, so a forged block header cannot make
    // this read out of bounds (a crash) or larger than the limit; a bad input becomes an
    // `invalid_input` value, never a trap
    let bql = match inputs.first() {
        None => Err("the query is not a memory block".to_owned()),
        Some(offset) => read_input_str(plugin, offset, QUERY_MAX_LEN).map_err(|e| format!("the query {e}")),
    };
    let result = query_answer(host.as_deref(), bql.as_deref().map_err(String::as_str));
    answer(plugin, outputs, result)
}

/// what `zhang_query` answers, given the host of the router call (none outside one) and the query
/// text the plugin passed, or why it could not be read
fn query_answer(host: Option<&dyn RouterHost>, bql: Result<&str, &str>) -> Result<Value, HostError> {
    let host = host.ok_or_else(|| unavailable(QUERY_FUNCTION))?;
    let bql = bql.map_err(|message| HostError::InvalidInput { message: message.to_owned() })?;
    host.query(bql).map_err(|failure| HostError::Query {
        message: failure.message,
        line: failure.line,
        column: failure.column,
    })
}

/// `zhang_ledger_info() -> info`
fn zhang_ledger_info(plugin: &mut CurrentPlugin, _inputs: &[Val], outputs: &mut [Val], call: UserData<RouterCall>) -> Result<(), extism::Error> {
    let info = call.get()?.lock().unwrap_or_else(PoisonError::into_inner).ledger_info.clone();
    answer(plugin, outputs, info.ok_or_else(|| unavailable(LEDGER_INFO_FUNCTION)))
}

/// what `zhang_ledger_info` answers for `ledger`
fn ledger_info(ledger: &Ledger) -> Value {
    let title = ledger.operations().option::<String>("title").ok().flatten();
    json!({
        "title": title,
        "operating_currency": ledger.options.operating_currency,
        "timezone": ledger.options.timezone.name(),
    })
}

impl PluginStore {
    /// the router plugin serving `/api/plugins/{name}`: the first declared router plugin of that name
    pub fn router(&self, name: &str) -> Option<&RegisteredPlugin> {
        self.routers.iter().find(|plugin| plugin.name == name)
    }

    /// register a router plugin. One sharing the name of an earlier router plugin never serves a request
    pub(crate) fn add_router(&mut self, plugin: RegisteredPlugin) {
        if self.router(&plugin.name).is_some() {
            warn!(
                "two router plugins are named {}: the first one declared serves /api/plugins/{}, the later one never does",
                plugin.name, plugin.name
            );
        }
        self.routers.push(plugin);
    }
}

impl RegisteredPlugin {
    /// handle `request` in a new instance of the plugin, with the host functions reading `ledger`
    /// through `host`. The caller keeps `ledger` read-locked until this returns.
    ///
    /// The response is validated and ready to send (see [`PluginResponse::into_http`]).
    pub fn execute_as_router(&self, request: &PluginRequest, ledger: &Ledger, host: Arc<dyn RouterHost>) -> Result<http::Response<Vec<u8>>, RouterError> {
        debug!("plugin {} {} handles {} {}", self.name, self.version, request.method, request.path);
        let options = ledger.operations().options().map_err(|e| RouterError::Load(e.to_string()))?;
        let call = RouterCall {
            host: Some(host),
            ledger_info: Some(ledger_info(ledger)),
        };
        // `zhang_emit_error` too, so a plugin importing it can serve requests. `zhang_now` reads the
        // ledger's clock afresh for this request
        let plugin_host = self.routing_host(LoadClock::new(ledger.clock()), ledger.options.timezone);
        let functions = plugin_host.functions().into_iter().chain(host_functions(call));
        // the same manifest as a processor gets: config, allowed hosts and timeout
        let mut plugin = WasmPlugin::new(self.manifest(&options), functions, true).map_err(|e| RouterError::Load(format!("{e:#}")))?;
        if !plugin.function_exists(ROUTER_EXPORT) {
            return Err(RouterError::NoRouterExport);
        }
        let input = serde_json::to_vec(request).map_err(|e| RouterError::Failed(format!("cannot encode the request: {e}")))?;
        let output = plugin.call::<&[u8], &[u8]>(ROUTER_EXPORT, &input).map(<[u8]>::to_vec);
        // a request has no error list to report into
        for error in plugin_host.take_errors() {
            let message = error.metas.get(MESSAGE_META).map(String::as_str).unwrap_or_default();
            warn!(
                "router plugin {} reported a problem while handling {} {}: {message}",
                self.name, request.method, request.path
            );
        }
        let output = match output {
            Ok(output) => output,
            // extism reports a call it stopped at the manifest's timeout as exactly "timeout"
            Err(e) if e.to_string() == "timeout" => return Err(RouterError::Timeout),
            Err(e) => return Err(RouterError::Failed(format!("{e:?}"))),
        };
        PluginResponse::from_json(&output)
            .and_then(PluginResponse::into_http)
            .map_err(RouterError::BadResponse)
    }
}

#[cfg(test)]
mod test {
    use serde_json::{json, Value};

    use super::{query_answer, QueryFailure, RouterHost};

    /// answers `SELECT 1` and fails every other query
    struct OneQuery;

    impl RouterHost for OneQuery {
        fn query(&self, bql: &str) -> Result<Value, QueryFailure> {
            match bql {
                "SELECT 1" => Ok(json!({"columns": [], "rows": []})),
                _ => Err(QueryFailure {
                    message: "unknown column".to_owned(),
                    line: Some(1),
                    column: Some(8),
                }),
            }
        }
    }

    fn encoded(host: Option<&dyn RouterHost>, bql: Result<&str, &str>) -> Value {
        serde_json::to_value(query_answer(host, bql)).unwrap()
    }

    #[test]
    fn should_answer_query_results_and_failures_as_values() {
        assert_eq!(encoded(Some(&OneQuery), Ok("SELECT 1")), json!({"Ok": {"columns": [], "rows": []}}));
        assert_eq!(
            encoded(Some(&OneQuery), Ok("SELECT nope")),
            json!({"Err": {"kind": "query", "message": "unknown column", "line": 1, "column": 8}})
        );
        assert_eq!(
            encoded(Some(&OneQuery), Err("the query is not UTF-8 text")),
            json!({"Err": {"kind": "invalid_input", "message": "the query is not UTF-8 text"}})
        );
    }

    #[test]
    fn should_answer_unavailable_outside_a_router_call() {
        assert_eq!(
            encoded(None, Ok("SELECT 1")),
            json!({"Err": {"kind": "unavailable", "message": "zhang_query is only available while the plugin handles a request as a router"}})
        );
    }
}
