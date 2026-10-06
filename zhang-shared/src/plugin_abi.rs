//! Plugin ABI v1 as it crosses the WASM boundary, defined once for the host (zhang-core) and the plugin SDK: the
//! names of the exports and host functions, the reserved config keys, and the JSON shapes the two sides exchange.
//!
//! Every shape here is part of the ABI. A field may be added, and a reader ignores fields it does not know; a field
//! is never removed, renamed or made required.

use std::collections::BTreeMap;
use std::fmt::{Debug, Display, Formatter};

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use serde::ser::SerializeStruct;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// The plugin ABI version, which the host passes to every plugin as the config [`config::ABI`]. Additions a plugin
/// can ignore (a new config key, a new host function) keep the version.
pub const ABI_VERSION: u32 = 1;

/// the functions a plugin exports
pub mod export {
    /// the plugin's name, a JSON string
    pub const NAME: &str = "name";
    /// the plugin's version, a JSON string
    pub const VERSION: &str = "version";
    /// the plugin types it implements, a JSON array of `Processor`, `Mapper` and `Router`
    pub const SUPPORTED_TYPE: &str = "supported_type";
    /// turns the whole directive stream into a new one
    pub const PROCESSOR: &str = "processor";
    /// turns each directive into zero or more directives
    pub const MAPPER: &str = "mapper";
    /// answers an HTTP [`Request`](super::Request) with a [`Response`](super::Response)
    pub const ROUTER: &str = "router";
}

/// the host functions zhang offers in extism's `extism:host/user` namespace
pub mod import {
    /// the host function a plugin reports a problem in the ledger with
    pub const EMIT_ERROR: &str = "zhang_emit_error";

    /// The host function a plugin reads the current time with.
    ///
    /// `zhang_now() -> i64` returns the offset of a kernel memory block holding the JSON
    ///
    /// ```json
    /// {"Ok": {"now": "2024-03-16T00:30:00+08:00", "today": "2024-03-16", "timezone": "Asia/Shanghai"}}
    /// ```
    ///
    /// with a [`Now`](super::Now) inside `Ok`. The host reads its clock once per load, on the first call from any
    /// plugin, so every call of a load returns the same value and all plugins agree on "today". A call from a
    /// processor or mapper makes the ledger depend on the date, so a server reloads it when the date changes; a
    /// call while the plugin registers (`name`, `version`, `supported_type`) does not. `zhang_now` has no error
    /// today, but a plugin should still handle `Err`.
    ///
    /// A plugin targeting WASI can also read the host's real clock, and OS entropy, through WASI: extism links them
    /// in, and a host function cannot intercept them. Reproducible plugins read the time with `zhang_now` only, and
    /// derive randomness from the [`config::SEED`](super::config::SEED) config.
    pub const NOW: &str = "zhang_now";

    /// the host function a plugin reads a granted file with
    pub const READ_FILE: &str = "zhang_read_file";
    /// the host function a plugin lists a granted directory with
    pub const LIST_DIR: &str = "zhang_list_dir";
    /// the host function a router plugin runs a BQL query over the ledger being served with
    pub const QUERY: &str = "zhang_query";
    /// the host function a router plugin reads the ledger's title, currency and timezone with
    pub const LEDGER_INFO: &str = "zhang_ledger_info";
}

/// the config keys the host sets for every plugin; a `plugin` directive's meta cannot override them
pub mod config {
    /// prefix of the config keys reserved for values the host sets
    pub const RESERVED_PREFIX: &str = "zhang.";
    /// config key holding the version of the plugin ABI the host speaks, [`ABI_VERSION`](super::ABI_VERSION) as a
    /// decimal string
    pub const ABI: &str = "zhang.abi";
    /// config key holding the plugin's `plugin` directive as written, a [`PluginDirective`](super::PluginDirective)
    /// as JSON
    pub const PLUGIN: &str = "zhang.plugin";
    /// config key holding the plugin's seed, a decimal `u64`
    pub const SEED: &str = "zhang.seed";
}

/// The plugin's `plugin` directive as written, from the config key [`config::PLUGIN`]. It carries what the flat
/// config cannot: the positional arguments, and every value of a repeated meta key. The directive
///
/// ```zhang
/// plugin "fx-rate.wasm" "USD" "strict"
///   allowed_hosts: "api.frankfurter.dev"
///   tag: "first"
///   tag: "second"
/// ```
///
/// reaches the plugin as (compact JSON, wrapped here for reading)
///
/// ```json
/// {"module":"fx-rate.wasm","args":["USD","strict"],
///  "meta":{"allowed_hosts":["api.frankfurter.dev"],"tag":["first","second"]}}
/// ```
///
/// Every value is a string, exactly as written; parsing amounts, dates or numbers is up to the plugin.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginDirective {
    /// the module as written in the directive
    #[serde(default)]
    pub module: String,
    /// the positional values after the module, in order; empty when there are none. Beancount's
    /// `plugin "module" "config"` passes its config string here
    #[serde(default)]
    pub args: Vec<String>,
    /// every meta key with all its values in source order, the keys sorted. Unlike the flat config it keeps every
    /// value of a repeated key, and it includes the capability keys such as `allowed_hosts` (a grant is not secret,
    /// and a plugin can inspect what it was granted) as well as keys starting with [`config::RESERVED_PREFIX`]
    #[serde(default)]
    pub meta: BTreeMap<String, Vec<String>>,
}

/// how bytes travel in a JSON string
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Encoding {
    /// the bytes are the string itself, which is valid UTF-8
    #[default]
    Utf8,
    /// the string is the standard base64 (with padding) of the bytes
    Base64,
}

impl Encoding {
    /// the bytes `text` stands for in this encoding
    pub fn decode(self, text: &str) -> Result<Vec<u8>, base64::DecodeError> {
        match self {
            Encoding::Utf8 => Ok(text.as_bytes().to_vec()),
            Encoding::Base64 => BASE64.decode(text),
        }
    }
}

/// what every host function that returns a value answers: `{"Ok": value}` or `{"Err": {"kind": "...", ...}}`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum HostResult<T> {
    Ok(T),
    Err(HostError),
}

impl<T> From<Result<T, HostError>> for HostResult<T> {
    fn from(result: Result<T, HostError>) -> Self {
        match result {
            Ok(value) => HostResult::Ok(value),
            Err(error) => HostResult::Err(error),
        }
    }
}

/// A zhang host function answered with an error. Host functions report problems as values and never trap, so a
/// missing file or a bad query is an ordinary `Err` the plugin can handle.
///
/// On the wire it is `{"kind": "...", "message": "..."}`; a [`HostErrorKind::Query`] error also has `line` and
/// `column`, `null` when the problem has no position.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct HostError {
    pub kind: HostErrorKind,
    /// what went wrong, for people
    #[serde(default)]
    pub message: String,
    /// for [`HostErrorKind::Query`]: the line of the query text the problem is on, when it has one
    #[serde(default)]
    pub line: Option<usize>,
    /// for [`HostErrorKind::Query`]: the column of the query text the problem is at, when it has one
    #[serde(default)]
    pub column: Option<usize>,
}

impl HostError {
    /// an error of the kind `kind`, without a position
    pub fn new(kind: HostErrorKind, message: impl Into<String>) -> HostError {
        HostError {
            kind,
            message: message.into(),
            line: None,
            column: None,
        }
    }
}

impl Serialize for HostError {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let with_position = self.kind == HostErrorKind::Query;
        let mut error = serializer.serialize_struct("HostError", if with_position { 4 } else { 2 })?;
        error.serialize_field("kind", &self.kind)?;
        error.serialize_field("message", &self.message)?;
        if with_position {
            error.serialize_field("line", &self.line)?;
            error.serialize_field("column", &self.column)?;
        }
        error.end()
    }
}

impl Display for HostError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match (self.line, self.column) {
            (Some(line), Some(column)) => write!(f, "{} (line {line}, column {column})", self.message),
            _ => f.write_str(&self.message),
        }
    }
}

impl std::error::Error for HostError {}

/// The kind of a [`HostError`], as the host names it (`"not_found"` is [`HostErrorKind::NotFound`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum HostErrorKind {
    /// files: the path is not granted by `allowed_paths`, leads outside the grant or the ledger root, or the
    /// operating system refused it; or the plugin is not running as a processor or mapper
    Denied,
    /// files: the path does not exist, or the ledger's source failed to read it
    NotFound,
    /// files: the file is larger than 16 MiB, or the directory has more than 10 000 entries
    TooLarge,
    /// files: the ledger's source cannot do this, e.g. list a directory
    Unsupported,
    /// files: the request is not well-formed: an unreadable path, a directory to read, a file to list. The SDK's
    /// `fs::read_to_string` also uses it for a file that is not UTF-8
    Invalid,
    /// router host functions: the argument could not be read
    InvalidInput,
    /// router host functions outside a request; on a native (non-WASM) build of a plugin, every host function
    Unavailable,
    /// `zhang_query`: the query failed to parse, compile or run; see [`HostError::line`] and [`HostError::column`]
    Query,
    /// a kind this side does not know, from a newer zhang, or an answer it cannot read
    #[serde(other)]
    Other,
}

/// what `zhang_now` answers inside `Ok`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Now {
    /// the current time of the load as RFC 3339, with the offset of the ledger timezone
    pub now: String,
    /// the date of `now` in the ledger timezone, `YYYY-MM-DD`
    pub today: String,
    /// the ledger timezone, an IANA name
    pub timezone: String,
}

/// what `zhang_read_file` answers inside `Ok`: `{"content": "...", "encoding": "utf8" | "base64"}`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileContent {
    pub content: String,
    #[serde(default)]
    pub encoding: Encoding,
}

impl FileContent {
    /// the content of a file with the bytes `bytes`: UTF-8 text as it is, unless escaping it in JSON would more than
    /// double it (mostly control characters), anything else as base64
    pub fn from_bytes(bytes: Vec<u8>) -> FileContent {
        let bytes = match String::from_utf8(bytes) {
            Ok(content) if json_escaped_len(&content) <= 2 * content.len() => {
                return FileContent {
                    content,
                    encoding: Encoding::Utf8,
                }
            }
            Ok(content) => content.into_bytes(),
            Err(e) => e.into_bytes(),
        };
        FileContent {
            content: BASE64.encode(bytes),
            encoding: Encoding::Base64,
        }
    }

    /// the file's bytes; an error when the content claims to be base64 but is not
    pub fn into_bytes(self) -> Result<Vec<u8>, base64::DecodeError> {
        match self.encoding {
            Encoding::Utf8 => Ok(self.content.into_bytes()),
            Encoding::Base64 => BASE64.decode(self.content),
        }
    }
}

/// the length of `text` escaped as a JSON string, without the quotes: a control character takes up to 6 bytes
pub fn json_escaped_len(text: &str) -> usize {
    text.bytes()
        .map(|byte| match byte {
            b'"' | b'\\' | b'\n' | b'\r' | b'\t' | 0x08 | 0x0c => 2,
            0x00..=0x1f => 6,
            _ => 1,
        })
        .sum()
}

/// what `zhang_list_dir` answers inside `Ok`: `{"entries": [...]}`, sorted by name
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirListing {
    pub entries: Vec<DirEntry>,
}

/// an entry of a listed directory: `{"name": "...", "kind": "file" | "dir"}`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirEntry {
    /// the entry's name, without its directory
    pub name: String,
    pub kind: EntryKind,
}

/// what a listed entry is
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum EntryKind {
    File,
    Dir,
    /// a kind this side does not know, from a newer zhang
    #[serde(other)]
    Other,
}

/// what `zhang_ledger_info` answers inside `Ok`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerInfo {
    /// the `title` option
    pub title: Option<String>,
    /// the `operating_currency` option
    pub operating_currency: String,
    /// the ledger's timezone, an IANA name
    pub timezone: String,
}

/// Request headers carrying the user's credentials for zhang itself, or for a proxy in front of it. A plugin sits
/// behind zhang's authentication and never needs them, and one allowed to reach the network could leak them, so they
/// are never passed on.
pub const WITHHELD_REQUEST_HEADERS: [&str; 3] = ["authorization", "proxy-authorization", "cookie"];

/// the HTTP request a router plugin's `router` export receives, as JSON:
///
/// ```json
/// {
///   "method": "GET",
///   "path": "/sub/path",
///   "query": {"k": ["v1", "v2"]},
///   "headers": {"accept": "text/html"},
///   "body": "",
///   "body_encoding": "utf8"
/// }
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    /// the method, upper case, e.g. `GET`
    #[serde(default)]
    pub method: String,
    /// the path below the plugin's route, starting with `/`, percent-encoded as sent: `/api/plugins/{name}/by-month`
    /// gives `/by-month`, and `/api/plugins/{name}` gives `/`
    #[serde(default)]
    pub path: String,
    /// the decoded query string: every key with all its values, in order
    #[serde(default)]
    pub query: BTreeMap<String, Vec<String>>,
    /// the headers, names lower case, the values of a repeated header joined with `, `. zhang never passes the
    /// credential headers [`WITHHELD_REQUEST_HEADERS`]
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    /// the body, encoded as [`Request::body_encoding`] says; empty when there is none
    #[serde(default)]
    pub body: String,
    /// `utf8` when the body is valid UTF-8, else `base64`
    #[serde(default)]
    pub body_encoding: Encoding,
}

impl Request {
    /// a request: the query pairs grouped by key, the header names lower-cased, the credential headers left out, and
    /// the body carried as UTF-8 when it is valid UTF-8 and as base64 otherwise
    pub fn new(
        method: impl Into<String>, path: impl Into<String>, query: impl IntoIterator<Item = (String, String)>,
        headers: impl IntoIterator<Item = (String, String)>, body: Vec<u8>,
    ) -> Request {
        let mut grouped_query: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (key, value) in query {
            grouped_query.entry(key).or_default().push(value);
        }
        let mut grouped_headers: BTreeMap<String, String> = BTreeMap::new();
        for (name, value) in headers {
            let name = name.to_ascii_lowercase();
            if WITHHELD_REQUEST_HEADERS.contains(&name.as_str()) {
                continue;
            }
            grouped_headers
                .entry(name)
                .and_modify(|joined| {
                    joined.push_str(", ");
                    joined.push_str(&value);
                })
                .or_insert(value);
        }
        let (body, body_encoding) = match String::from_utf8(body) {
            Ok(text) => (text, Encoding::Utf8),
            Err(e) => (BASE64.encode(e.into_bytes()), Encoding::Base64),
        };
        Request {
            method: method.into(),
            path: path.into(),
            query: grouped_query,
            headers: grouped_headers,
            body,
            body_encoding,
        }
    }

    /// the first value of the query parameter `key`
    pub fn query_param(&self, key: &str) -> Option<&str> {
        self.query.get(key).and_then(|values| values.first()).map(String::as_str)
    }

    /// the header `name`, in any case
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(&name.to_ascii_lowercase()).map(String::as_str)
    }

    /// the body as bytes
    pub fn body_bytes(&self) -> Result<Vec<u8>, Error> {
        Ok(self.body_encoding.decode(&self.body)?)
    }

    /// the body parsed as JSON
    pub fn json<T: serde::de::DeserializeOwned>(&self) -> Result<T, Error> {
        Ok(serde_json::from_slice(&self.body_bytes()?)?)
    }
}

/// The HTTP response a router plugin's `router` export returns, as JSON. Every field is optional, and `null` counts
/// as absent:
///
/// ```json
/// {
///   "status": 200,
///   "headers": {"content-type": "text/html; charset=utf-8"},
///   "body": "<h1>Report</h1>",
///   "body_encoding": "utf8"
/// }
/// ```
///
/// Any content type works, so a router can answer JSON for a script or a whole HTML page. zhang sets
/// `content-length`, `transfer-encoding` and `connection` itself; without a `content-type` header the response is
/// `text/plain; charset=utf-8`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Response {
    /// the HTTP status, 200 when absent
    #[serde(default = "default_status", deserialize_with = "null_as_status")]
    pub status: u16,
    /// the headers, one value per name
    #[serde(default, deserialize_with = "null_as_default")]
    pub headers: BTreeMap<String, String>,
    /// the body, encoded as [`Response::body_encoding`] says; empty when absent
    #[serde(default, deserialize_with = "null_as_default")]
    pub body: String,
    /// `utf8` (when absent) or `base64`, for a binary body such as an image
    #[serde(default, deserialize_with = "null_as_default")]
    pub body_encoding: Encoding,
}

impl Default for Response {
    /// an empty `200 OK`
    fn default() -> Self {
        Response {
            status: default_status(),
            headers: BTreeMap::new(),
            body: String::new(),
            body_encoding: Encoding::Utf8,
        }
    }
}

fn default_status() -> u16 {
    200
}

fn null_as_status<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u16, D::Error> {
    Ok(Option::<u16>::deserialize(deserializer)?.unwrap_or_else(default_status))
}

fn null_as_default<'de, D: Deserializer<'de>, T: Deserialize<'de> + Default>(deserializer: D) -> Result<T, D::Error> {
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

impl Response {
    fn with_content(content_type: &str, body: String, body_encoding: Encoding) -> Response {
        Response {
            body,
            body_encoding,
            ..Response::default()
        }
        .with_header("content-type", content_type)
    }

    /// a `200 OK` plain text response
    pub fn text(body: impl Into<String>) -> Response {
        Response::with_content("text/plain; charset=utf-8", body.into(), Encoding::Utf8)
    }

    /// a `200 OK` HTML page
    pub fn html(body: impl Into<String>) -> Response {
        Response::with_content("text/html; charset=utf-8", body.into(), Encoding::Utf8)
    }

    /// a `200 OK` JSON response
    pub fn json<T: Serialize + ?Sized>(value: &T) -> Result<Response, Error> {
        Ok(Response::with_content("application/json", serde_json::to_string(value)?, Encoding::Utf8))
    }

    /// a `200 OK` response with any bytes, e.g. an image
    pub fn bytes(content_type: &str, body: impl AsRef<[u8]>) -> Response {
        Response::with_content(content_type, BASE64.encode(body), Encoding::Base64)
    }

    /// this response with the HTTP status `status`
    pub fn with_status(mut self, status: u16) -> Response {
        self.status = status;
        self
    }

    /// this response with the header `name` set to `value`; names are lower-cased
    pub fn with_header(mut self, name: &str, value: impl Into<String>) -> Response {
        self.headers.insert(name.to_ascii_lowercase(), value.into());
        self
    }

    /// read the JSON a router returned; an error says why it is not a response. Unlike plain serde, this accepts
    /// only a JSON object
    pub fn from_json(output: &[u8]) -> Result<Response, String> {
        let value: serde_json::Value = serde_json::from_slice(output).map_err(|e| e.to_string())?;
        if !value.is_object() {
            return Err("the response is not a JSON object".to_owned());
        }
        serde_json::from_value(value).map_err(|e| e.to_string())
    }

    /// the body as bytes; an error when it claims to be base64 but is not
    pub fn body_bytes(&self) -> Result<Vec<u8>, base64::DecodeError> {
        self.body_encoding.decode(&self.body)
    }
}

/// the content type of a router response that does not set one
#[cfg(feature = "http")]
pub const DEFAULT_CONTENT_TYPE: &str = "text/plain; charset=utf-8";

#[cfg(feature = "http")]
impl Response {
    /// the HTTP response to send: [`DEFAULT_CONTENT_TYPE`] when the plugin sets no content type, and without the
    /// framing headers the host sets itself. An error says why the plugin's response cannot be sent: a status out
    /// of range, a header that is not valid HTTP, or a body that is not the base64 it claims to be.
    pub fn into_http(self) -> Result<http::Response<Vec<u8>>, String> {
        use http::header::{CONNECTION, CONTENT_LENGTH, CONTENT_TYPE, TRANSFER_ENCODING};
        use http::{HeaderName, HeaderValue, StatusCode};

        /// the response headers describing how the body is framed
        const FRAMING_HEADERS: [HeaderName; 3] = [CONTENT_LENGTH, TRANSFER_ENCODING, CONNECTION];

        let status = StatusCode::from_u16(self.status).map_err(|_| format!("{} is not an HTTP status", self.status))?;
        let body = self.body_bytes().map_err(|e| format!("the body is not valid base64: {e}"))?;
        let mut response = http::Response::new(body);
        *response.status_mut() = status;
        for (name, value) in &self.headers {
            let name = HeaderName::from_bytes(name.as_bytes()).map_err(|_| format!("{name:?} is not a valid header name"))?;
            if FRAMING_HEADERS.contains(&name) {
                continue;
            }
            let value = HeaderValue::from_str(value).map_err(|_| format!("the value of the header {name} is not a valid header value"))?;
            response.headers_mut().insert(name, value);
        }
        response
            .headers_mut()
            .entry(CONTENT_TYPE)
            .or_insert(HeaderValue::from_static(DEFAULT_CONTENT_TYPE));
        Ok(response)
    }
}

/// Why a plugin export failed. Returning it from a handler fails the export: a failing `processor` or `mapper` aborts
/// the whole ledger load with this message, and a failing `router` answers the request with status 500. To report a
/// problem *in the ledger* and keep loading, a plugin calls `zhang_emit_error` instead (the SDK's
/// `errors::emit_error`).
///
/// Any error type converts into it with `?`, keeping its message and the messages of its sources.
pub struct Error {
    message: String,
}

impl Error {
    /// an error with this message
    pub fn msg(message: impl Display) -> Error {
        Error { message: message.to_string() }
    }

    /// what went wrong
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl Display for Error {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl Debug for Error {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        Debug::fmt(&self.message, f)
    }
}

impl<E: std::error::Error> From<E> for Error {
    fn from(error: E) -> Error {
        let mut message = error.to_string();
        let mut source = error.source();
        while let Some(cause) = source {
            message.push_str(": ");
            message.push_str(&cause.to_string());
            source = cause.source();
        }
        Error { message }
    }
}
