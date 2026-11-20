//! The write capability: `zhang_write_preview` and `zhang_write_commit`, the host functions a router plugin appends
//! directives to the ledger with. Off unless two switches are on:
//!
//! 1. the plugin **declares** the kinds it writes, `{"writes": ["transaction", "open"]}`, in its optional
//!    `capabilities` export ([`DeclaredCapabilities`]), read while it registers;
//! 2. the ledger owner **grants** them on the `plugin` directive, `allowed_writes: "transaction"`, one value per kind
//!    ([`crate::plugin::capabilities`]).
//!
//! A plugin may write the kinds in both. Each function takes the offset of a kernel memory block holding a
//! [`WriteBatch`] as JSON and answers `{"Ok": ...}` or `{"Err": {"kind": "...", "message": "..."}}`, never trapping:
//!
//! - `zhang_write_preview(input: i64) -> i64` answers a [`WritePreview`]: every directive's text as the ledger's format
//!   writes it, the file it goes to, what the ledger would report against it, and a `token` when every entry is clean.
//! - `zhang_write_commit(input: i64) -> i64` takes the same batch with the preview's `token` and answers a
//!   [`WriteCommitted`] once the text is in the file.
//!
//! The error kinds: `unavailable` outside a router call, or when the plugin declared no writes; `denied` when the
//! directive grants nothing, or not the kind of a directive in the batch; `invalid_input` when the block cannot be
//! read or is not the JSON of a batch; `invalid` when the batch breaks a bound ([`MAX_DIRECTIVES`], [`MAX_TEXT_LEN`]),
//! has an entry without a known `kind`, holds a JSON number anywhere (amounts are strings, so a float never changes a
//! balance), or fails the ledger's validation; `conflict` when the token is not for this ledger generation or this
//! batch, expired, the ledger is reloading, or a commit already happened in this generation; `io` when the file
//! could not be written.
//!
//! The checks that need no ledger run here, so a forged, oversized or ungranted input never reaches the server; the
//! ledger work (one validation, one exporter, one file policy, the API's) is the server's [`RouterHost`]
//! implementation.

use std::sync::{Arc, PoisonError};

use extism::{CurrentPlugin, UserData, Val};
use serde_json::Value;
pub use zhang_shared::plugin_abi::import::{WRITE_COMMIT as WRITE_COMMIT_FUNCTION, WRITE_PREVIEW as WRITE_PREVIEW_FUNCTION};
pub use zhang_shared::plugin_abi::{DeclaredCapabilities, WriteBatch, WriteCommitted, WriteKind, WritePreview, WritePreviewEntry};
use zhang_shared::plugin_abi::{HostError, HostErrorKind};

use crate::plugin::host::{read_input_str, HostState};
use crate::plugin::router::{answer, unavailable, RouterHost};

/// the most bytes a write function reads for its JSON input, the cap of every router host function
pub const MAX_INPUT_LEN: usize = 1024 * 1024;

/// the most directives one batch may hold
pub const MAX_DIRECTIVES: usize = 50;

/// the most bytes the exported text of one batch may hold, every directive counted
pub const MAX_TEXT_LEN: usize = 64 * 1024;

/// how long a preview token is valid, in seconds
pub const TOKEN_TTL_SECS: i64 = 10 * 60;

/// What a plugin may write while it handles a request as a router: the kinds it declared in its `capabilities` export,
/// and the kinds its `plugin` directive grants with `allowed_writes`. It may write those in both.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WriteAccess {
    pub declared: Vec<WriteKind>,
    pub granted: Vec<WriteKind>,
}

impl WriteAccess {
    /// the kinds the plugin may write: declared and granted
    pub fn allowed(&self) -> Vec<WriteKind> {
        self.declared.iter().copied().filter(|kind| self.granted.contains(kind)).collect()
    }

    /// why the plugin may not write at all, before any input is read: `unavailable` without a declaration (the
    /// plugin is not written to write), `denied` without a grant (the owner did not allow it)
    fn check_any(&self) -> Result<(), HostError> {
        if self.declared.is_empty() {
            return Err(HostError::new(
                HostErrorKind::Unavailable,
                "plugin did not declare writes: its `capabilities` export lists no `writes`",
            ));
        }
        if self.allowed().is_empty() {
            return Err(HostError::new(
                HostErrorKind::Denied,
                format!(
                    "plugin is not granted the writes it declares ({}): add `allowed_writes: \"{}\"` to its plugin directive",
                    names(&self.declared),
                    self.declared[0]
                ),
            ));
        }
        Ok(())
    }

    /// why the plugin may not write a directive of `kind`
    fn check_kind(&self, kind: WriteKind) -> Result<(), HostError> {
        if !self.declared.contains(&kind) {
            return Err(HostError::new(
                HostErrorKind::Denied,
                format!(
                    "plugin did not declare {kind} writes: its `capabilities` export lists {}",
                    names(&self.declared)
                ),
            ));
        }
        if !self.granted.contains(&kind) {
            return Err(HostError::new(
                HostErrorKind::Denied,
                format!("plugin is not granted {kind} writes: add `allowed_writes: \"{kind}\"` to its plugin directive"),
            ));
        }
        Ok(())
    }
}

/// the kinds, comma-separated, for a message
fn names(kinds: &[WriteKind]) -> String {
    kinds.iter().map(|kind| kind.to_string()).collect::<Vec<_>>().join(", ")
}

/// the kind of each directive of `batch`, in order, after the checks that need no ledger: at least one and at most
/// [`MAX_DIRECTIVES`] directives, each a JSON object with a known `kind`, and no JSON number anywhere
pub fn batch_kinds(batch: &WriteBatch) -> Result<Vec<WriteKind>, HostError> {
    let invalid = |message: String| HostError::new(HostErrorKind::Invalid, message);
    if batch.directives.is_empty() {
        return Err(invalid("the batch holds no directive".to_owned()));
    }
    if batch.directives.len() > MAX_DIRECTIVES {
        return Err(invalid(format!(
            "the batch holds {} directives, over the limit of {MAX_DIRECTIVES}",
            batch.directives.len()
        )));
    }
    let mut kinds = Vec::with_capacity(batch.directives.len());
    for (index, directive) in batch.directives.iter().enumerate() {
        let Some(object) = directive.as_object() else {
            return Err(invalid(format!("directives[{index}] is not a JSON object")));
        };
        let kind = match object.get("kind") {
            Some(Value::String(kind)) => WriteKind::parse(kind).ok_or_else(|| {
                invalid(format!(
                    "directives[{index}] has the kind {kind:?}, which is not one of {}",
                    names(&WriteKind::ALL)
                ))
            })?,
            Some(other) => return Err(invalid(format!("directives[{index}].kind is {other}, not a string"))),
            None => return Err(invalid(format!("directives[{index}] has no `kind`"))),
        };
        if let Some((path, number)) = first_number(directive, &format!("directives[{index}]")) {
            return Err(invalid(format!(
                "{path} is the number {number}: write amounts as strings, such as \"{number}\", so nothing is rounded"
            )));
        }
        kinds.push(kind);
    }
    Ok(kinds)
}

/// the first JSON number in `value`, with its path from `path`, in document order
fn first_number(value: &Value, path: &str) -> Option<(String, String)> {
    match value {
        Value::Number(number) => Some((path.to_owned(), number.to_string())),
        Value::Array(items) => items
            .iter()
            .enumerate()
            .find_map(|(index, item)| first_number(item, &format!("{path}[{index}]"))),
        Value::Object(fields) => fields.iter().find_map(|(key, item)| first_number(item, &format!("{path}.{key}"))),
        _ => None,
    }
}

/// what the write functions of one call reach: the server's host, the plugin's name, and what it may write
struct WriteCall {
    host: Arc<dyn RouterHost>,
    plugin: String,
    access: WriteAccess,
}

/// the write call of the current router call, or none outside one. The state is cloned out, so the lock is not
/// held while the ledger works
fn write_call(state: &UserData<HostState>) -> Result<Option<WriteCall>, extism::Error> {
    let state = state.get()?;
    let state = state.lock().unwrap_or_else(PoisonError::into_inner);
    Ok(state.router.as_ref().map(|(host, _)| WriteCall {
        host: host.clone(),
        plugin: state.plugin.clone(),
        access: state.writes.clone().unwrap_or_default(),
    }))
}

/// `zhang_write_preview(input) -> result`
pub(super) fn zhang_write_preview(plugin: &mut CurrentPlugin, inputs: &[Val], outputs: &mut [Val], state: UserData<HostState>) -> Result<(), extism::Error> {
    let call = write_call(&state)?;
    let input = read_write_input(plugin, inputs);
    let result = preview_answer(call.as_ref(), input.as_deref().map_err(String::as_str));
    answer(plugin, outputs, result)
}

/// `zhang_write_commit(input) -> result`
pub(super) fn zhang_write_commit(plugin: &mut CurrentPlugin, inputs: &[Val], outputs: &mut [Val], state: UserData<HostState>) -> Result<(), extism::Error> {
    let call = write_call(&state)?;
    let input = read_write_input(plugin, inputs);
    let result = commit_answer(call.as_ref(), input.as_deref().map_err(String::as_str));
    answer(plugin, outputs, result)
}

/// the JSON input the plugin passed, read through the bounds-checked helper so a forged block answers
/// `invalid_input` and never traps, or why it could not be read
fn read_write_input(plugin: &mut CurrentPlugin, inputs: &[Val]) -> Result<String, String> {
    match inputs.first() {
        None => Err("the input is not a memory block".to_owned()),
        Some(offset) => read_input_str(plugin, offset, MAX_INPUT_LEN).map_err(|e| format!("the input {e}")),
    }
}

/// the batch of `input`, once the call may write at all: the checks run in the order a plugin author needs them,
/// phase, declaration and grant before the input is looked at, so a plugin that may not write learns nothing from the
/// input it passes
fn checked_batch<'a>(function: &str, call: Option<&'a WriteCall>, input: Result<&str, &str>) -> Result<(&'a WriteCall, WriteBatch, Vec<WriteKind>), HostError> {
    let call = call.ok_or_else(|| unavailable(function))?;
    call.access.check_any()?;
    let input = input.map_err(|message| HostError::new(HostErrorKind::InvalidInput, message))?;
    let not_an_object = |reason: String| {
        HostError::new(
            HostErrorKind::InvalidInput,
            format!("the input is not a JSON object {{\"directives\": [{{\"kind\": \"...\", ...}}], \"token\": ...}}: {reason}"),
        )
    };
    let value: Value = serde_json::from_str(input).map_err(|e| not_an_object(e.to_string()))?;
    if !value.is_object() {
        return Err(not_an_object("it is not an object".to_owned()));
    }
    let batch: WriteBatch = serde_json::from_value(value).map_err(|e| not_an_object(e.to_string()))?;
    let kinds = batch_kinds(&batch)?;
    for kind in &kinds {
        call.access.check_kind(*kind)?;
    }
    Ok((call, batch, kinds))
}

/// what `zhang_write_preview` answers, given the call (none outside a router call) and the JSON input the plugin
/// passed, or why it could not be read
fn preview_answer(call: Option<&WriteCall>, input: Result<&str, &str>) -> Result<WritePreview, HostError> {
    let (call, batch, _) = checked_batch(WRITE_PREVIEW_FUNCTION, call, input)?;
    call.host.write_preview(&call.plugin, &batch)
}

/// what `zhang_write_commit` answers, as [`preview_answer`]; a batch without a token is `invalid`
fn commit_answer(call: Option<&WriteCall>, input: Result<&str, &str>) -> Result<WriteCommitted, HostError> {
    let (call, batch, _) = checked_batch(WRITE_COMMIT_FUNCTION, call, input)?;
    let token = batch.token.as_deref().filter(|token| !token.is_empty()).ok_or_else(|| {
        HostError::new(
            HostErrorKind::Invalid,
            "the batch has no token: preview it first, and commit it with the preview's token",
        )
    })?;
    call.host.write_commit(&call.plugin, &batch, token)
}

/// the kinds a plugin's `capabilities` export declares, by name; a kind this host does not know is left out with
/// a warning, so a plugin written for a newer zhang still loads
pub fn declared_writes(plugin: &str, declared: &DeclaredCapabilities) -> Vec<WriteKind> {
    let mut kinds = vec![];
    for name in &declared.writes {
        match WriteKind::parse(name) {
            Some(kind) if !kinds.contains(&kind) => kinds.push(kind),
            Some(_) => {}
            None => log::warn!("plugin {plugin} declares the write kind {name:?}, which this version of zhang does not know; ignoring it"),
        }
    }
    kinds
}

#[cfg(test)]
mod test {
    use std::sync::Arc;

    use serde_json::{json, Value};
    use zhang_shared::plugin_abi::{HostError, HostErrorKind, WriteBatch, WriteCommitted, WritePreview};

    use super::{batch_kinds, commit_answer, declared_writes, preview_answer, DeclaredCapabilities, WriteAccess, WriteCall, WriteKind};
    use crate::plugin::router::{QueryFailure, RouterHost};

    /// a host that echoes the batch it is asked to preview or commit
    struct Echo;

    impl RouterHost for Echo {
        fn query(&self, _bql: &str) -> Result<Value, QueryFailure> {
            unreachable!()
        }

        fn query_with_params(
            &self, _bql: &str, _params: &std::collections::BTreeMap<String, zhang_shared::plugin_abi::RawQueryParam>,
        ) -> Result<Value, QueryFailure> {
            unreachable!()
        }

        fn schema(&self) -> Result<Value, QueryFailure> {
            unreachable!()
        }

        fn write_preview(&self, plugin: &str, batch: &WriteBatch) -> Result<WritePreview, HostError> {
            Ok(WritePreview {
                ok: true,
                file: format!("{plugin}:{}", batch.directives.len()),
                generation: 7,
                entries: vec![],
                token: Some("t".to_owned()),
            })
        }

        fn write_commit(&self, plugin: &str, batch: &WriteBatch, token: &str) -> Result<WriteCommitted, HostError> {
            Ok(WriteCommitted {
                file: format!("{plugin}:{token}"),
                bytes: 0,
                entries: batch.directives.len(),
                generation: 8,
            })
        }
    }

    fn call(declared: &[WriteKind], granted: &[WriteKind]) -> WriteCall {
        WriteCall {
            host: Arc::new(Echo),
            plugin: "writer".to_owned(),
            access: WriteAccess {
                declared: declared.to_vec(),
                granted: granted.to_vec(),
            },
        }
    }

    const BOTH: [WriteKind; 2] = [WriteKind::Transaction, WriteKind::Open];

    fn batch(directives: Value) -> String {
        json!({"directives": directives}).to_string()
    }

    fn preview(call: Option<&WriteCall>, input: &str) -> Value {
        serde_json::to_value(preview_answer(call, Ok(input))).unwrap()
    }

    fn error(answer: Value) -> (String, String) {
        (
            answer["Err"]["kind"].as_str().unwrap().to_owned(),
            answer["Err"]["message"].as_str().unwrap().to_owned(),
        )
    }

    #[test]
    fn should_preview_and_commit_a_batch_the_plugin_may_write() {
        let both = call(&BOTH, &BOTH);
        let input = batch(json!([{"kind": "transaction", "payee": "x"}, {"kind": "open", "account": "Assets:Cash"}]));
        assert_eq!(
            preview(Some(&both), &input),
            json!({"Ok": {"ok": true, "file": "writer:2", "generation": 7, "entries": [], "token": "t"}})
        );
        let input = json!({"directives": [{"kind": "open", "account": "Assets:Cash"}], "token": "abc"}).to_string();
        assert_eq!(
            serde_json::to_value(commit_answer(Some(&both), Ok(&input))).unwrap(),
            json!({"Ok": {"file": "writer:abc", "bytes": 0, "entries": 1, "generation": 8}})
        );
        // a plugin granted and declaring only transactions writes one
        let transactions = call(&[WriteKind::Transaction], &[WriteKind::Transaction]);
        assert_eq!(preview(Some(&transactions), &batch(json!([{"kind": "transaction"}])))["Ok"]["file"], "writer:1");
    }

    #[test]
    fn should_answer_unavailable_outside_a_router_call_and_without_a_declaration() {
        let input = batch(json!([{"kind": "open"}]));
        let (kind, message) = error(preview(None, &input));
        assert_eq!(kind, "unavailable");
        assert!(message.starts_with("zhang_write_preview is only available while"), "{message}");
        let (kind, _) = error(serde_json::to_value(commit_answer(None, Ok(&input))).unwrap());
        assert_eq!(kind, "unavailable");
        // declared nothing, even when granted: the plugin is not written to write
        let (kind, message) = error(preview(Some(&call(&[], &BOTH)), &input));
        assert_eq!(kind, "unavailable");
        assert_eq!(message, "plugin did not declare writes: its `capabilities` export lists no `writes`");
    }

    #[test]
    fn should_deny_a_plugin_without_a_grant_before_reading_its_input() {
        // declared but not granted at all: denied even for an unreadable input
        let (kind, message) = error(serde_json::to_value(preview_answer(Some(&call(&BOTH, &[])), Err("the input is not a memory block"))).unwrap());
        assert_eq!(kind, "denied");
        assert_eq!(
            message,
            "plugin is not granted the writes it declares (transaction, open): add `allowed_writes: \"transaction\"` to its plugin directive"
        );
        // granted a kind the batch does not use
        let (kind, message) = error(preview(
            Some(&call(&BOTH, &[WriteKind::Open])),
            &batch(json!([{"kind": "open"}, {"kind": "transaction"}])),
        ));
        assert_eq!(kind, "denied");
        assert_eq!(
            message,
            "plugin is not granted transaction writes: add `allowed_writes: \"transaction\"` to its plugin directive"
        );
        // granted a kind the plugin did not declare
        let (kind, message) = error(preview(Some(&call(&[WriteKind::Open], &BOTH)), &batch(json!([{"kind": "transaction"}]))));
        assert_eq!(kind, "denied");
        assert_eq!(message, "plugin did not declare transaction writes: its `capabilities` export lists open");
    }

    #[test]
    fn should_answer_invalid_input_for_an_input_that_cannot_be_read() {
        let both = call(&BOTH, &BOTH);
        let (kind, message) = error(serde_json::to_value(preview_answer(Some(&both), Err("the input is 2000000 bytes, over the 1048576 byte limit"))).unwrap());
        assert_eq!(
            (kind.as_str(), message.as_str()),
            ("invalid_input", "the input is 2000000 bytes, over the 1048576 byte limit")
        );
        for input in ["[]", "SELECT date", r#"{"directives": "open"}"#] {
            let (kind, message) = error(preview(Some(&both), input));
            assert_eq!(kind, "invalid_input", "{input}");
            assert!(message.starts_with("the input is not a JSON object {\"directives\": ["), "{message}");
        }
    }

    #[test]
    fn should_refuse_a_batch_that_breaks_a_bound_or_names_no_kind() {
        let both = call(&BOTH, &BOTH);
        let invalid = |directives: Value| {
            let (kind, message) = error(preview(Some(&both), &batch(directives)));
            assert_eq!(kind, "invalid", "{message}");
            message
        };
        assert_eq!(invalid(json!([])), "the batch holds no directive");
        assert_eq!(
            invalid(Value::Array(vec![json!({"kind": "open"}); 51])),
            "the batch holds 51 directives, over the limit of 50"
        );
        assert_eq!(invalid(json!([{"kind": "open"}, "open"])), "directives[1] is not a JSON object");
        assert_eq!(invalid(json!([{"account": "x"}])), "directives[0] has no `kind`");
        assert_eq!(invalid(json!([{"kind": 1}])), "directives[0].kind is 1, not a string");
        assert_eq!(
            invalid(json!([{"kind": "balance"}])),
            "directives[0] has the kind \"balance\", which is not one of transaction, open"
        );
        assert_eq!(
            invalid(json!([{"kind": "transaction", "postings": [{"account": "a"}, {"account": "b", "unit": {"number": 4.5, "commodity": "USD"}}]}])),
            "directives[0].postings[1].unit.number is the number 4.5: write amounts as strings, such as \"4.5\", so nothing is rounded"
        );
        assert_eq!(
            invalid(json!([{"kind": "open", "metas": [{"key": "n", "value": 3}]}])),
            "directives[0].metas[0].value is the number 3: write amounts as strings, such as \"3\", so nothing is rounded"
        );
        // exactly the limit passes
        assert!(batch_kinds(&WriteBatch {
            directives: vec![json!({"kind": "open"}); 50],
            token: None
        })
        .is_ok());
    }

    #[test]
    fn should_require_a_token_to_commit() {
        let both = call(&BOTH, &BOTH);
        for input in [
            batch(json!([{"kind": "open"}])),
            json!({"directives": [{"kind": "open"}], "token": ""}).to_string(),
        ] {
            let (kind, message) = error(serde_json::to_value(commit_answer(Some(&both), Ok(&input))).unwrap());
            assert_eq!(kind, "invalid", "{input}");
            assert_eq!(message, "the batch has no token: preview it first, and commit it with the preview's token");
        }
    }

    #[test]
    fn should_read_the_declared_kinds_and_ignore_unknown_ones() {
        let declared: DeclaredCapabilities = serde_json::from_value(json!({"writes": ["open", "balance", "transaction", "open"]})).unwrap();
        assert_eq!(declared_writes("p", &declared), vec![WriteKind::Open, WriteKind::Transaction]);
        assert_eq!(declared_writes("p", &DeclaredCapabilities::default()), vec![]);
        let access = WriteAccess {
            declared: vec![WriteKind::Open, WriteKind::Transaction],
            granted: vec![WriteKind::Transaction],
        };
        assert_eq!(access.allowed(), vec![WriteKind::Transaction]);
    }

    #[test]
    fn should_write_the_error_kinds_the_abi_names() {
        assert_eq!(serde_json::to_value(HostError::new(HostErrorKind::Conflict, "x")).unwrap()["kind"], "conflict");
        assert_eq!(serde_json::to_value(HostError::new(HostErrorKind::Io, "x")).unwrap()["kind"], "io");
    }
}
