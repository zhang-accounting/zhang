//! Router plugins: HTTP endpoints a WASM plugin defines.
//!
//! A plugin declaring the `Router` type serves `/api/plugins/{name}` and every path below it, for
//! any HTTP method, behind the same authentication as the rest of zhang's API. `{name}` is what the
//! plugin's `name` export returns, the name `/api/plugins` lists. When two router plugins share a
//! name, the first one declared serves it, and the later one is reported on its directive, so the
//! conflict is visible where the owner reads the ledger's errors.
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
//!   - `zhang_query_params(input: i64) -> i64` runs a query with named parameters bound, so a
//!     plugin binds what a user typed instead of writing it into the BQL. The block holds a
//!     [`QueryWithParams`] as JSON, `{"bql": "... :from ...", "params": {"from": {"type": "date",
//!     "value": "2024-01-01"}}}`, with the same size limit as a query's text; the value is what
//!     `zhang_query` gives. A parameter whose type is not one of `bool`, `int`, `decimal`,
//!     `str`, `date` and `set`, or whose value is not of the shape that type takes (see
//!     [`RawQueryParam`]), is a `query` error naming the parameter, without a position; a
//!     parameter the query uses but the input does not bind is a `query` error at its position.
//!   - `zhang_query_schema() -> i64` gives the tables, columns and functions a query can use,
//!     the `data` that `GET /api/query/schema` answers.
//!   - `zhang_query_builtin(input: i64) -> i64` runs one of the server's named built-in queries,
//!     the BQL behind the figures the app shows: the block holds a [`BuiltinQueryCall`] as JSON,
//!     `{"name": "accounts.list", "params": {"date": "2026-10-13", "time": "09:30:00"}}`, and the
//!     value is what `zhang_query` gives. An unknown name, or a parameter that is missing, unknown
//!     or not of its type, is a `query` error without a position.
//!   - `zhang_write_preview(input: i64) -> i64` and `zhang_write_commit(input: i64) -> i64` append
//!     directives to the ledger, for a plugin that declares writes and whose directive grants them
//!     with `allowed_writes`; see [`crate::plugin::write`]. They are the one way a router changes the
//!     ledger's files; the ledger in memory stays as it is until the server reloads it after the call.
//!
//!   All of them are only useful while `router` runs. They are linked into every other instance
//!   of a plugin too (registration, `processor`, `mapper`), so a plugin importing them still
//!   loads and runs as its other types; there they give the kind `unavailable`. A call whose
//!   argument is not a readable UTF-8 block, or for `zhang_query_params` and `zhang_query_builtin`
//!   not the JSON described,
//!   gives the kind `invalid_input`.
//! - `zhang_emit_error` ([`crate::plugin::host`]) is linked into a router call too, so a plugin
//!   importing it can serve requests. A request has no error list to report into, so what the
//!   plugin reports is logged as a warning and otherwise dropped; the call still never traps. A
//!   router tells its caller about a problem in its response instead.
//!
//! # Isolation
//!
//! Every request runs in a new instance of the plugin, so no state survives between requests; only
//! the compiled code is shared, through the plugin's [`crate::plugin::runtime::PluginRuntime`]. The
//! instance gets the same config, `allowed_hosts`, memory limit and timeout as the plugin's processor,
//! all within the operator's [`PluginPolicy`](crate::plugin_policy::PluginPolicy): a call running
//! longer than the timeout is stopped and answered as [`RouterError::Timeout`]. The caller holds the
//! ledger's read lock for the whole call, so the ledger cannot reload under the plugin, and no host
//! function can change the ledger. That lock is why the policy's
//! [`max_timeout`](crate::plugin_policy::PluginPolicy::max_timeout) defaults to 30 seconds: while a
//! writer waits for the lock, no new reader gets it, so a long call would stall every request of the
//! server.
//!
//! What the plugin answers is sent only as [`PluginResponse::into_http`] makes it: the headers it may
//! set, and the sandbox zhang adds to every response.

use std::collections::BTreeMap;
use std::fmt::{Display, Formatter};
use std::sync::{Arc, PoisonError};
use std::time::Duration;

use extism::{CurrentPlugin, UserData, Val};
use log::{debug, warn};
use serde::Serialize;
use serde_json::value::{to_raw_value, RawValue};
use serde_json::Value;
use zhang_shared::plugin_abi::analysis::MAX_CONVERSIONS;
pub use zhang_shared::plugin_abi::analysis::{Conversion, ConvertedAmount, CONVERT as CONVERT_FUNCTION};
pub use zhang_shared::plugin_abi::export::ROUTER as ROUTER_EXPORT;
pub use zhang_shared::plugin_abi::import::{
    LEDGER_INFO as LEDGER_INFO_FUNCTION, QUERY as QUERY_FUNCTION, QUERY_BUILTIN as QUERY_BUILTIN_FUNCTION, QUERY_PARAMS as QUERY_PARAMS_FUNCTION,
    QUERY_SCHEMA as QUERY_SCHEMA_FUNCTION,
};
pub use zhang_shared::plugin_abi::{BuiltinQueryCall, QueryWithParams, RawQueryParam, WriteBatch, WriteCommitted, WritePreview};
use zhang_shared::plugin_abi::{HostError, HostErrorKind, LedgerInfo};

use crate::clock::LoadClock;
use crate::ledger::Ledger;
use crate::plugin::host::{read_input_str, HostState, MESSAGE_META};
use crate::plugin::http::{PluginRequest, PluginResponse};
use crate::plugin::log_safe;
use crate::plugin::store::{PluginStore, RegisteredPlugin};
use crate::plugin::write::WriteAccess;
use crate::plugin_policy::{Size, MAX_MEMORY_ENV};

/// the most bytes `zhang_query` will read for a plugin's BQL text, and `zhang_query_params` for its
/// whole JSON input. A query is human-written SQL-like text, so one mebibyte is far beyond any real
/// query while capping what a single call can read.
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

    /// run a BQL query with the named parameters `params` bound, as [`RouterHost::query`] runs one.
    /// The host reads each parameter's type by name and its value as JSON
    /// (`zhang_query::params::bind_json`), and reports a parameter it cannot read as a failure
    /// without a position, e.g. `parameter :from must be a date string YYYY-MM-DD or null`
    fn query_with_params(&self, bql: &str, params: &BTreeMap<String, RawQueryParam>) -> Result<Value, QueryFailure>;

    /// the tables, columns and functions a query can use: the `data` of `GET /api/query/schema`
    fn schema(&self) -> Result<Value, QueryFailure>;

    /// [`RouterHost::query`] written as JSON, what `zhang_query` answers with: a host that can write its answer
    /// straight to text, without a tree of JSON values first, says so here. The plugin reads the same JSON values
    fn query_json(&self, bql: &str) -> Result<Box<RawValue>, QueryFailure> {
        self.query(bql).map(|value| to_raw_value(&value).expect("a JSON value is written"))
    }

    /// [`RouterHost::query_with_params`] written as JSON, as [`RouterHost::query_json`] writes [`RouterHost::query`]
    fn query_with_params_json(&self, bql: &str, params: &BTreeMap<String, RawQueryParam>) -> Result<Box<RawValue>, QueryFailure> {
        self.query_with_params(bql, params)
            .map(|value| to_raw_value(&value).expect("a JSON value is written"))
    }

    /// Run the built-in query `name` with `params` bound, as `POST /api/query/builtins/{name}` runs it and with the
    /// limits of [`RouterHost::query`]; the value is the same shape. An unknown name, or a parameter that is missing,
    /// unknown or not of its type, is a failure without a position. A host without built-in queries says so
    fn query_builtin(&self, name: &str, params: &BTreeMap<String, Value>) -> Result<Value, QueryFailure> {
        let _ = params;
        Err(QueryFailure {
            message: format!("this host runs no built-in queries, so not {name}"),
            line: None,
            column: None,
        })
    }

    /// Convert `conversions` with the prices of the ledger, as the query engine converts (`convert(position, currency,
    /// date)`): for each, the amount in its target currency, or `None` when no price converts it. A host without
    /// prices says so
    fn convert(&self, conversions: &[Conversion]) -> Result<Vec<Option<ConvertedAmount>>, HostError> {
        let _ = conversions;
        Err(HostError::new(HostErrorKind::Unavailable, "this host converts no amounts"))
    }

    /// Preview `batch`, which `plugin` wants to append, as `POST /api/transactions/preview` previews a transaction:
    /// validate each directive in order, the earlier ones of the batch counted as written, export its text, choose
    /// its file by the ledger's policy, and mint a token for [`RouterHost::write_commit`] when every entry is clean.
    /// The host already checked the phase, the declaration, the grant, the kinds and the bounds that need no ledger
    /// ([`crate::plugin::write`]); the ledger's own validation, the exported text's size and the error kinds `invalid`
    /// and `conflict` are this method's. A host that cannot write answers `unavailable`
    fn write_preview(&self, plugin: &str, batch: &WriteBatch) -> Result<WritePreview, HostError> {
        let _ = (plugin, batch);
        Err(HostError::new(HostErrorKind::Unavailable, "this host cannot write to the ledger"))
    }

    /// Append `batch`, previewed as `token` says, in one `append` of the ledger's data source, once the token is
    /// verified against the ledger's generation and the batch's text: the text is in the file when this returns
    /// `Ok`. The caller then marks the ledger stale and asks for a reload, as an API write does
    fn write_commit(&self, plugin: &str, batch: &WriteBatch, token: &str) -> Result<WriteCommitted, HostError> {
        let _ = (plugin, batch, token);
        Err(HostError::new(HostErrorKind::Unavailable, "this host cannot write to the ledger"))
    }
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

/// what a router host function answers outside a router call, `{"Err": {"kind": "unavailable"}}`
pub(super) fn unavailable(function: &str) -> HostError {
    HostError::new(
        HostErrorKind::Unavailable,
        format!("{function} is only available while the plugin handles a request as a router"),
    )
}

/// write `result` into a new memory block as `{"Ok": ...}` or `{"Err": ...}` and return its offset
pub(super) fn answer<T: Serialize>(plugin: &mut CurrentPlugin, outputs: &mut [Val], result: Result<T, HostError>) -> Result<(), extism::Error> {
    let handle = plugin.memory_new(serde_json::to_string(&result)?)?;
    outputs[0] = plugin.memory_to_val(handle);
    Ok(())
}

/// `zhang_query(bql) -> result`
pub(super) fn zhang_query(plugin: &mut CurrentPlugin, inputs: &[Val], outputs: &mut [Val], state: UserData<HostState>) -> Result<(), extism::Error> {
    let host = router_host(&state)?;
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
fn query_answer(host: Option<&dyn RouterHost>, bql: Result<&str, &str>) -> Result<Box<RawValue>, HostError> {
    let host = host.ok_or_else(|| unavailable(QUERY_FUNCTION))?;
    let bql = bql.map_err(|message| HostError::new(HostErrorKind::InvalidInput, message))?;
    host.query_json(bql).map_err(query_error)
}

/// a failed query as the plugin sees it: the kind `query`, with the position when it has one
fn query_error(failure: QueryFailure) -> HostError {
    HostError {
        kind: HostErrorKind::Query,
        message: failure.message,
        line: failure.line,
        column: failure.column,
    }
}

/// the host of the current router call, or none outside one. The host is cloned out, so the
/// lock is not held while the query runs
fn router_host(state: &UserData<HostState>) -> Result<Option<Arc<dyn RouterHost>>, extism::Error> {
    Ok(state
        .get()?
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .router
        .as_ref()
        .map(|(host, _)| host.clone()))
}

/// `zhang_query_params(input) -> result`
pub(super) fn zhang_query_params(plugin: &mut CurrentPlugin, inputs: &[Val], outputs: &mut [Val], state: UserData<HostState>) -> Result<(), extism::Error> {
    let host = router_host(&state)?;
    // the whole JSON input is read under the same cap as a query's text, through the same
    // bounds-checked helper, so a forged block answers `invalid_input` and never traps
    let input = match inputs.first() {
        None => Err("the input is not a memory block".to_owned()),
        Some(offset) => read_input_str(plugin, offset, QUERY_MAX_LEN).map_err(|e| format!("the input {e}")),
    };
    let result = query_params_answer(host.as_deref(), input.as_deref().map_err(String::as_str));
    answer(plugin, outputs, result)
}

/// what `zhang_query_params` answers, given the host of the router call (none outside one) and
/// the JSON input the plugin passed, or why it could not be read
fn query_params_answer(host: Option<&dyn RouterHost>, input: Result<&str, &str>) -> Result<Box<RawValue>, HostError> {
    let host = host.ok_or_else(|| unavailable(QUERY_PARAMS_FUNCTION))?;
    let input = input.map_err(|message| HostError::new(HostErrorKind::InvalidInput, message))?;
    let query: QueryWithParams = serde_json::from_str(input).map_err(|e| {
        HostError::new(
            HostErrorKind::InvalidInput,
            format!("the input is not a JSON object {{\"bql\": \"...\", \"params\": {{\"name\": {{\"type\": \"...\", \"value\": ...}}}}}}: {e}"),
        )
    })?;
    host.query_with_params_json(&query.bql, &query.params).map_err(query_error)
}

/// `zhang_query_builtin(input) -> result`
pub(super) fn zhang_query_builtin(plugin: &mut CurrentPlugin, inputs: &[Val], outputs: &mut [Val], state: UserData<HostState>) -> Result<(), extism::Error> {
    let host = router_host(&state)?;
    // read under the cap of a query's input, through the bounds-checked helper, as zhang_query_params reads its own
    let input = match inputs.first() {
        None => Err("the input is not a memory block".to_owned()),
        Some(offset) => read_input_str(plugin, offset, QUERY_MAX_LEN).map_err(|e| format!("the input {e}")),
    };
    let result = query_builtin_answer(host.as_deref(), input.as_deref().map_err(String::as_str));
    answer(plugin, outputs, result)
}

/// what `zhang_query_builtin` answers, given the host of the router call (none outside one) and the JSON input the
/// plugin passed, or why it could not be read
fn query_builtin_answer(host: Option<&dyn RouterHost>, input: Result<&str, &str>) -> Result<Value, HostError> {
    let host = host.ok_or_else(|| unavailable(QUERY_BUILTIN_FUNCTION))?;
    let input = input.map_err(|message| HostError::new(HostErrorKind::InvalidInput, message))?;
    let call: BuiltinQueryCall = serde_json::from_str(input).map_err(|e| {
        HostError::new(
            HostErrorKind::InvalidInput,
            format!("the input is not a JSON object {{\"name\": \"...\", \"params\": {{\"name\": value}}}}: {e}"),
        )
    })?;
    host.query_builtin(&call.name, &call.params).map_err(query_error)
}

/// `zhang_query_schema() -> schema`
pub(super) fn zhang_query_schema(plugin: &mut CurrentPlugin, _inputs: &[Val], outputs: &mut [Val], state: UserData<HostState>) -> Result<(), extism::Error> {
    let host = router_host(&state)?;
    answer(plugin, outputs, schema_answer(host.as_deref()))
}

/// what `zhang_query_schema` answers, given the host of the router call (none outside one)
fn schema_answer(host: Option<&dyn RouterHost>) -> Result<Value, HostError> {
    let host = host.ok_or_else(|| unavailable(QUERY_SCHEMA_FUNCTION))?;
    host.schema().map_err(query_error)
}

/// the most bytes `zhang_convert` reads for its input, a list of at most [`MAX_CONVERSIONS`] conversions
const CONVERT_MAX_LEN: usize = 2 * 1024 * 1024;

/// `zhang_convert(input) -> amounts`
pub(super) fn zhang_convert(plugin: &mut CurrentPlugin, inputs: &[Val], outputs: &mut [Val], state: UserData<HostState>) -> Result<(), extism::Error> {
    let host = router_host(&state)?;
    let input = match inputs.first() {
        None => Err("the input is not a memory block".to_owned()),
        Some(offset) => read_input_str(plugin, offset, CONVERT_MAX_LEN).map_err(|e| format!("the input {e}")),
    };
    let result = convert_answer(host.as_deref(), input.as_deref().map_err(String::as_str));
    answer(plugin, outputs, result)
}

/// what `zhang_convert` answers, given the host of the call (none outside a router call or an analysis) and the JSON
/// input the plugin passed, or why it could not be read
fn convert_answer(host: Option<&dyn RouterHost>, input: Result<&str, &str>) -> Result<Vec<Option<ConvertedAmount>>, HostError> {
    let host = host.ok_or_else(|| {
        HostError::new(
            HostErrorKind::Unavailable,
            format!("{CONVERT_FUNCTION} is only available while the plugin handles a request or analyses the ledger"),
        )
    })?;
    let input = input.map_err(|message| HostError::new(HostErrorKind::InvalidInput, message))?;
    let conversions: Vec<Conversion> = serde_json::from_str(input).map_err(|e| {
        HostError::new(
            HostErrorKind::InvalidInput,
            format!("the input is not a JSON list of {{\"amount\": {{\"number\", \"currency\"}}, \"target\", \"date\", \"via\"?}}: {e}"),
        )
    })?;
    if conversions.len() > MAX_CONVERSIONS {
        return Err(HostError::new(
            HostErrorKind::InvalidInput,
            format!("the input holds {} conversions, more than the {MAX_CONVERSIONS} of one call", conversions.len()),
        ));
    }
    host.convert(&conversions)
}

/// `zhang_ledger_info() -> info`
pub(super) fn zhang_ledger_info(plugin: &mut CurrentPlugin, _inputs: &[Val], outputs: &mut [Val], state: UserData<HostState>) -> Result<(), extism::Error> {
    let info = state
        .get()?
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .router
        .as_ref()
        .map(|(_, info)| info.clone());
    answer(plugin, outputs, info.ok_or_else(|| unavailable(LEDGER_INFO_FUNCTION)))
}

/// what `zhang_ledger_info` answers for `ledger`
pub(crate) fn ledger_info(ledger: &Ledger) -> LedgerInfo {
    LedgerInfo {
        title: ledger.options.option::<String>("title").ok().flatten(),
        operating_currency: ledger.options.operating_currency.clone(),
        timezone: ledger.options.timezone.name().to_owned(),
    }
}

impl PluginStore {
    /// the router plugin serving `/api/plugins/{name}`
    pub fn router(&self, name: &str) -> Option<&RegisteredPlugin> {
        self.routers.iter().find(|plugin| plugin.name == name)
    }

    /// Register a router plugin. When a router plugin of its name was registered before it, that one keeps
    /// serving `/api/plugins/{name}` and this one never does: the problem is answered, naming both modules, for the
    /// ledger to report on this plugin's directive, so which code answers a route (with which grants) is never a
    /// silent matter of the order of the directives. A module declared again to run its processor or mapper a
    /// second time (`stages`), as a module declared twice runs twice, is no conflict: the first declaration's router
    /// serves, as before
    pub(crate) fn add_router(&mut self, plugin: RegisteredPlugin, stages: bool) -> Option<String> {
        let Some(first) = self.router(&plugin.name) else {
            self.routers.push(plugin);
            return None;
        };
        if stages && first.module() == plugin.module() {
            let name = log_safe(&plugin.name);
            debug!("router plugin {name} is declared again to run as a stage: the first declaration serves /api/plugins/{name}");
            return None;
        }
        Some(format!(
            "{} is a router named {}, as {} declared before it, which serves /api/plugins/{}: this one serves nothing. \
             Two router plugins cannot share a name; remove one of the directives",
            plugin.module(),
            plugin.name,
            first.module(),
            plugin.name
        ))
    }
}

impl RegisteredPlugin {
    /// what the plugin may write while it routes: the kinds it declared and the kinds its directive grants
    pub fn write_access(&self) -> WriteAccess {
        WriteAccess {
            declared: self.declared_writes().to_vec(),
            granted: self.capabilities().allowed_writes.clone(),
            operator_allows: self.policy().allows_writes(),
        }
    }

    /// handle `request` in a new instance of the plugin, with the host functions reading `ledger`
    /// through `host`. The caller keeps `ledger` read-locked until this returns.
    ///
    /// The call runs for at most [`RegisteredPlugin::router_timeout`]. The response is validated and ready to send
    /// (see [`PluginResponse::into_http`]). What the plugin wrote into an error is escaped ([`log_safe`]).
    pub fn execute_as_router(&self, request: &PluginRequest, ledger: &Ledger, host: Arc<dyn RouterHost>) -> Result<http::Response<Vec<u8>>, RouterError> {
        debug!(
            "plugin {} {} handles {} {}",
            log_safe(&self.name),
            log_safe(&self.version),
            request.method,
            log_safe(&request.path)
        );
        let options = ledger.options.all();
        // `zhang_emit_error` too, so a plugin importing it can serve requests. `zhang_now` reads the
        // ledger's clock afresh for this request
        let plugin_host = self
            .routing_host(LoadClock::new(ledger.clock()), ledger.options.timezone)
            .with_router(host, ledger_info(ledger))
            .with_writes(self.write_access());
        // the same manifest as a processor gets: config, allowed hosts and memory limit, with the router's timeout
        let mut plugin = self
            .runtime
            .instantiate(self.manifest_with_timeout(&options, self.router_timeout()), plugin_host.clone())
            .map_err(|e| RouterError::Load(log_safe(&format!("{e:#}")).into_owned()))?;
        if !plugin.function_exists(ROUTER_EXPORT) {
            return Err(RouterError::NoRouterExport);
        }
        let input = serde_json::to_vec(request).map_err(|e| RouterError::Failed(log_safe(&format!("cannot encode the request: {e}")).into_owned()))?;
        let output = plugin.call::<&[u8], &[u8]>(ROUTER_EXPORT, &input).map(<[u8]>::to_vec);
        // a request has no error list to report into
        for error in plugin_host.take_errors() {
            let message = error.metas.get(MESSAGE_META).map(String::as_str).unwrap_or_default();
            warn!(
                "router plugin {} reported a problem while handling {} {}: {}",
                log_safe(&self.name),
                request.method,
                log_safe(&request.path),
                log_safe(message)
            );
        }
        let output = match output {
            Ok(output) => output,
            Err(e) => {
                return Err(match e.to_string().as_str() {
                    // extism reports a call it stopped at the manifest's timeout as exactly "timeout"
                    "timeout" => RouterError::Timeout,
                    // and one whose memory would grow past the manifest's limit as exactly "oom"
                    "oom" => RouterError::Failed(format!(
                        "it ran out of memory: it needed more than the {} the server allows a plugin ({MAX_MEMORY_ENV})",
                        Size(self.policy().max_memory)
                    )),
                    _ => RouterError::Failed(log_safe(&format!("{e:#}")).into_owned()),
                });
            }
        };
        PluginResponse::from_json(&output)
            .and_then(PluginResponse::into_http)
            .map_err(|reason| RouterError::BadResponse(log_safe(&reason).into_owned()))
    }

    /// how long one router call of the plugin may run: its `timeout`, which the load already bounded by the
    /// policy's [`max_timeout`](crate::plugin_policy::PluginPolicy::max_timeout)
    pub fn router_timeout(&self) -> Duration {
        self.capabilities().timeout.min(self.policy().max_timeout)
    }
}

#[cfg(test)]
mod test {
    use std::collections::BTreeMap;

    use serde_json::{json, Value};

    use super::{convert_answer, query_answer, query_builtin_answer, query_params_answer, schema_answer, QueryFailure, RawQueryParam, RouterHost};

    /// answers `SELECT 1` and fails every other query; with parameters, echoes what it was given
    /// unless a parameter is named `bad`
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

        fn query_with_params(&self, bql: &str, params: &BTreeMap<String, RawQueryParam>) -> Result<Value, QueryFailure> {
            if params.contains_key("bad") {
                return Err(QueryFailure {
                    message: "parameter :bad must be a date string YYYY-MM-DD or null".to_owned(),
                    line: None,
                    column: None,
                });
            }
            Ok(json!({"bql": bql, "params": params}))
        }

        fn schema(&self) -> Result<Value, QueryFailure> {
            Ok(json!({"tables": [{"name": "postings"}]}))
        }

        fn query_builtin(&self, name: &str, params: &BTreeMap<String, Value>) -> Result<Value, QueryFailure> {
            match name {
                "accounts.list" => Ok(json!({"name": name, "params": params})),
                _ => Err(QueryFailure {
                    message: format!("there is no built-in query named {name}"),
                    line: None,
                    column: None,
                }),
            }
        }
    }

    /// answers `SELECT 1` only, and has no built-in queries
    struct NoBuiltins;

    impl RouterHost for NoBuiltins {
        fn query(&self, _bql: &str) -> Result<Value, QueryFailure> {
            Ok(json!({"columns": [], "rows": []}))
        }

        fn query_with_params(&self, _bql: &str, _params: &BTreeMap<String, RawQueryParam>) -> Result<Value, QueryFailure> {
            Ok(json!({"columns": [], "rows": []}))
        }

        fn schema(&self) -> Result<Value, QueryFailure> {
            Ok(json!({}))
        }
    }

    #[test]
    fn should_hand_conversions_to_the_host_and_answer_failures_as_values() {
        let convert = |host: Option<&dyn RouterHost>, input: Result<&str, &str>| serde_json::to_value(convert_answer(host, input)).unwrap();
        let one = r#"[{"amount": {"number": "1.50", "currency": "USD"}, "target": "CNY", "date": "2024-01-01"}]"#;
        assert_eq!(convert(None, Ok(one))["Err"]["kind"], json!("unavailable"));
        // a host without prices says so
        assert_eq!(convert(Some(&NoBuiltins), Ok(one))["Err"]["message"], json!("this host converts no amounts"));
        assert_eq!(convert(Some(&NoBuiltins), Ok("{}"))["Err"]["kind"], json!("invalid_input"));
        assert_eq!(
            convert(Some(&NoBuiltins), Err("the input is not a memory block"))["Err"]["kind"],
            json!("invalid_input")
        );
        let many = format!("[{}]", vec![&one[1..one.len() - 1]; super::MAX_CONVERSIONS + 1].join(","));
        let error = convert(Some(&NoBuiltins), Ok(&many));
        assert_eq!(error["Err"]["kind"], json!("invalid_input"));
        assert!(
            error["Err"]["message"].as_str().unwrap().ends_with("more than the 10000 of one call"),
            "{error}"
        );
    }

    #[test]
    fn should_hand_a_builtin_call_to_the_host_and_answer_failures_as_values() {
        let builtin = |host: Option<&dyn RouterHost>, input: Result<&str, &str>| serde_json::to_value(query_builtin_answer(host, input)).unwrap();
        assert_eq!(
            builtin(
                Some(&OneQuery),
                Ok(r#"{"name": "accounts.list", "params": {"date": "2026-10-13", "time": null}}"#)
            ),
            json!({"Ok": {"name": "accounts.list", "params": {"date": "2026-10-13", "time": null}}})
        );
        assert_eq!(
            builtin(Some(&OneQuery), Ok(r#"{"name": "nope"}"#)),
            json!({"Err": {"kind": "query", "message": "there is no built-in query named nope", "line": null, "column": null}})
        );
        let not_json = builtin(Some(&OneQuery), Ok(r#"{"params": {}}"#));
        assert_eq!(not_json["Err"]["kind"], json!("invalid_input"), "{not_json}");
        assert!(
            not_json["Err"]["message"]
                .as_str()
                .unwrap()
                .starts_with("the input is not a JSON object {\"name\": "),
            "{not_json}"
        );
        assert_eq!(
            builtin(None, Ok(r#"{"name": "accounts.list"}"#)),
            json!({"Err": {"kind": "unavailable", "message": "zhang_query_builtin is only available while the plugin handles a request as a router"}})
        );
        assert_eq!(
            builtin(Some(&NoBuiltins), Ok(r#"{"name": "accounts.list"}"#))["Err"]["message"],
            json!("this host runs no built-in queries, so not accounts.list")
        );
    }

    fn encoded(host: Option<&dyn RouterHost>, bql: Result<&str, &str>) -> Value {
        serde_json::to_value(query_answer(host, bql)).unwrap()
    }

    fn encoded_with_params(host: Option<&dyn RouterHost>, input: Result<&str, &str>) -> Value {
        serde_json::to_value(query_params_answer(host, input)).unwrap()
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
    fn should_hand_the_parsed_parameters_to_the_host_and_answer_failures_as_values() {
        let input = r#"{"bql": "SELECT date WHERE date >= :from", "params": {"from": {"type": "date", "value": "2024-01-01"}, "re": {"type": "str"}}}"#;
        assert_eq!(
            encoded_with_params(Some(&OneQuery), Ok(input)),
            json!({"Ok": {
                "bql": "SELECT date WHERE date >= :from",
                "params": {"from": {"type": "date", "value": "2024-01-01"}, "re": {"type": "str", "value": null}},
            }})
        );
        assert_eq!(
            encoded_with_params(
                Some(&OneQuery),
                Ok(r#"{"bql": "SELECT date", "params": {"bad": {"type": "date", "value": 1}}}"#)
            ),
            json!({"Err": {"kind": "query", "message": "parameter :bad must be a date string YYYY-MM-DD or null", "line": null, "column": null}})
        );
        // an input that is not the JSON described could not be read, like a block that is not UTF-8
        let not_json = encoded_with_params(Some(&OneQuery), Ok("SELECT date"));
        assert_eq!(not_json["Err"]["kind"], json!("invalid_input"));
        assert!(
            not_json["Err"]["message"]
                .as_str()
                .unwrap()
                .starts_with("the input is not a JSON object {\"bql\": \"...\", \"params\": "),
            "{not_json}"
        );
        assert_eq!(
            encoded_with_params(Some(&OneQuery), Ok(r#"{"params": {}}"#))["Err"]["kind"],
            json!("invalid_input")
        );
        assert_eq!(
            encoded_with_params(Some(&OneQuery), Err("the input is 2000000 bytes, over the 1048576 byte limit")),
            json!({"Err": {"kind": "invalid_input", "message": "the input is 2000000 bytes, over the 1048576 byte limit"}})
        );
        assert_eq!(
            serde_json::to_value(schema_answer(Some(&OneQuery))).unwrap(),
            json!({"Ok": {"tables": [{"name": "postings"}]}})
        );
    }

    #[test]
    fn should_answer_unavailable_outside_a_router_call() {
        assert_eq!(
            encoded(None, Ok("SELECT 1")),
            json!({"Err": {"kind": "unavailable", "message": "zhang_query is only available while the plugin handles a request as a router"}})
        );
        assert_eq!(
            encoded_with_params(None, Ok(r#"{"bql": "SELECT 1", "params": {}}"#)),
            json!({"Err": {"kind": "unavailable", "message": "zhang_query_params is only available while the plugin handles a request as a router"}})
        );
        assert_eq!(
            serde_json::to_value(schema_answer(None)).unwrap(),
            json!({"Err": {"kind": "unavailable", "message": "zhang_query_schema is only available while the plugin handles a request as a router"}})
        );
    }
}
