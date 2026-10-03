use std::fmt::{Debug, Display, Formatter};

use serde::de::DeserializeOwned;
use serde::Deserialize;

/// Why a plugin export failed. Returning it from a handler fails the export: a failing `processor` or
/// `mapper` aborts the whole ledger load with this message, and a failing `router` answers the request
/// with status 500. To report a problem *in the ledger* and keep loading, use
/// [`errors::emit_error`](crate::errors::emit_error) instead.
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

/// A zhang host function answered with an error. Host functions report problems as values and never
/// trap, so a missing file or a bad query is an ordinary `Err` the plugin can handle.
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

/// The kind of a [`HostError`], as the host names it (`"not_found"` is [`HostErrorKind::NotFound`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum HostErrorKind {
    /// files: the path is not granted by `allowed_paths`, leads outside the grant or the ledger root, or the
    /// plugin is not running as a processor or mapper
    Denied,
    /// files: the path does not exist
    NotFound,
    /// files: the file is larger than 16 MiB, or the directory has more than 10 000 entries
    TooLarge,
    /// files: the ledger's source cannot do this, e.g. list a directory
    Unsupported,
    /// files: the request is not well-formed: an empty path, a directory to read, a file to list; the SDK
    /// also uses it for a file [`fs::read_to_string`](crate::fs::read_to_string) finds is not UTF-8
    Invalid,
    /// router host functions: the argument could not be read
    InvalidInput,
    /// router host functions outside a request; on a native (non-WASM) build, every host function
    Unavailable,
    /// `zhang_query`: the query failed to parse, compile or run; see [`HostError::line`] and
    /// [`HostError::column`]
    Query,
    /// a kind this SDK does not know, from a newer zhang, or an answer the SDK cannot read
    #[serde(other)]
    Other,
}

impl HostError {
    pub(crate) fn new(kind: HostErrorKind, message: impl Into<String>) -> HostError {
        HostError {
            kind,
            message: message.into(),
            line: None,
            column: None,
        }
    }

    /// what every host function answers on a native build, outside zhang
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub(crate) fn outside_zhang(function: &str) -> HostError {
        HostError::new(
            HostErrorKind::Unavailable,
            format!("{function} is only available inside zhang: this is a native build of the plugin"),
        )
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

/// what a host function returning a value answers: `{"Ok": value}` or `{"Err": {"kind", "message", ...}}`
#[derive(Deserialize)]
enum HostResult<T> {
    Ok(T),
    Err(HostError),
}

/// read the answer of the host function `function`
pub(crate) fn host_result<T: DeserializeOwned>(function: &str, answer: &[u8]) -> Result<T, HostError> {
    match serde_json::from_slice::<HostResult<T>>(answer) {
        Ok(HostResult::Ok(value)) => Ok(value),
        Ok(HostResult::Err(error)) => Err(error),
        Err(e) => Err(HostError::new(
            HostErrorKind::Other,
            format!("{function} answered something this SDK cannot read: {e}"),
        )),
    }
}

#[cfg(test)]
mod test {
    use serde_json::Value;

    use super::{host_result, Error, HostError, HostErrorKind};

    #[test]
    fn should_read_ok_and_err_answers() {
        assert_eq!(host_result::<u32>("f", br#"{"Ok": 7}"#), Ok(7));
        assert_eq!(
            host_result::<u32>("f", br#"{"Err": {"kind": "not_found", "message": "a.txt: no such file"}}"#),
            Err(HostError::new(HostErrorKind::NotFound, "a.txt: no such file"))
        );
        let query = host_result::<Value>("f", br#"{"Err": {"kind": "query", "message": "unknown column", "line": 1, "column": 8}}"#).unwrap_err();
        assert_eq!((query.kind, query.line, query.column), (HostErrorKind::Query, Some(1), Some(8)));
        assert_eq!(query.to_string(), "unknown column (line 1, column 8)");
    }

    #[test]
    fn should_tolerate_kinds_from_a_newer_host_and_reject_garbage() {
        let newer = host_result::<u32>("f", br#"{"Err": {"kind": "rate_limited", "message": "later"}}"#).unwrap_err();
        assert_eq!(newer.kind, HostErrorKind::Other);
        let garbage = host_result::<u32>("zhang_now", b"nope").unwrap_err();
        assert_eq!(garbage.kind, HostErrorKind::Other);
        assert!(
            garbage.message.starts_with("zhang_now answered something this SDK cannot read"),
            "{}",
            garbage.message
        );
    }

    #[test]
    fn should_keep_the_sources_of_a_converted_error() {
        let error: Error = serde_json::from_str::<u32>("\"x\"").map_err(Error::from).unwrap_err();
        assert_eq!(error.message(), "invalid type: string \"x\", expected u32 at line 1 column 3");
        assert_eq!(Error::msg("boom").to_string(), "boom");
    }
}
