//! Host functions zhang offers WASM plugins.
//!
//! They live in extism's `extism:host/user` namespace. A plugin opts into one by importing it: a plugin
//! that imports none of them never notices them, and a plugin that imports one fails to load on a
//! zhang that predates it, with an "unknown import" error.
//!
//! - `zhang_emit_error(payload: i64)`, no result: report a problem without failing the load.
//!   `payload` is the offset of a kernel memory block holding the JSON
//!   `{"message": "...", "span": {...}?, "metas": {"key": "value"}?}`, where `span` is the span of a
//!   directive the plugin received. The problem becomes a [`ErrorKind::PluginError`] in the ledger's
//!   error list, on that span or else on the plugin's directive, with the metas `plugin` (the plugin's
//!   name), `message` and the plugin's own `metas`. A payload that cannot be read is reported the same
//!   way, with a message saying it is invalid.
//!
//! Every plugin instance gets its own [`PluginHost`]. It keeps what the host functions collect while
//! the instance runs, until the stage running the plugin hands it to the pipeline.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use extism::{CurrentPlugin, Function, UserData, Val, EXTISM_USER_MODULE, PTR};
use log::warn;
use serde::Deserialize;
use zhang_ast::error::ErrorKind;
use zhang_ast::SpanInfo;

use crate::pipeline::{StageContext, StageError};

/// name of the host function a plugin reports a problem with
pub const EMIT_ERROR: &str = "zhang_emit_error";

/// meta of a [`ErrorKind::PluginError`] holding the name of the plugin that reported it
pub const PLUGIN_META: &str = "plugin";
/// meta of a [`ErrorKind::PluginError`] holding the problem the plugin described
pub const MESSAGE_META: &str = "message";

/// what a plugin passes to `zhang_emit_error`
#[derive(Deserialize)]
struct EmitErrorPayload {
    message: String,
    /// kept as a value, so a span that is not well-formed falls back to the directive's span
    /// instead of rejecting the whole report
    #[serde(default)]
    span: Option<serde_json::Value>,
    #[serde(default)]
    metas: Option<HashMap<String, String>>,
}

/// the state the host functions of one plugin instance share
struct HostState {
    /// the plugin's name, given as the `plugin` meta of the errors it reports
    plugin: String,
    /// the span of the plugin's directive, where an error without a usable span of its own is reported
    directive_span: SpanInfo,
    /// the errors the plugin reported, in call order
    errors: Vec<StageError>,
}

/// the host side of one plugin instance: the host functions to link into it and what they collected
pub struct PluginHost {
    state: Arc<Mutex<HostState>>,
}

impl PluginHost {
    pub fn new(plugin: impl Into<String>, directive_span: SpanInfo) -> Self {
        let state = HostState {
            plugin: plugin.into(),
            directive_span,
            errors: vec![],
        };
        Self {
            state: Arc::new(Mutex::new(state)),
        }
    }

    /// every host function zhang offers, bound to this host
    pub fn functions(&self) -> Vec<Function> {
        vec![Function::new(EMIT_ERROR, [PTR], [], UserData::Rust(self.state.clone()), emit_error).with_namespace(EXTISM_USER_MODULE)]
    }

    /// take the errors the plugin reported so far
    pub fn take_errors(&self) -> Vec<StageError> {
        std::mem::take(&mut self.state.lock().unwrap_or_else(PoisonError::into_inner).errors)
    }

    /// hand the errors the plugin reported so far to the pipeline
    pub fn forward_to(&self, ctx: &mut StageContext) {
        for error in self.take_errors() {
            ctx.emit_error(error.kind, error.span, error.metas);
        }
    }
}

/// `zhang_emit_error(payload)`. It never traps: a payload it cannot read is reported as an invalid payload
fn emit_error(plugin: &mut CurrentPlugin, inputs: &[Val], _outputs: &mut [Val], state: UserData<HostState>) -> Result<(), extism::Error> {
    let payload = inputs
        .first()
        .and_then(|offset| plugin.memory_from_val(offset))
        .and_then(|handle| plugin.memory_bytes(handle).ok())
        .map(<[u8]>::to_vec);
    let state = state.get()?;
    let mut state = state.lock().unwrap_or_else(PoisonError::into_inner);
    let error = plugin_error(&state.plugin, &state.directive_span, payload.as_deref());
    state.errors.push(error);
    Ok(())
}

/// the [`ErrorKind::PluginError`] a `zhang_emit_error` payload reports; `payload` is `None` when the plugin
/// passed no readable memory block.
///
/// - span: the payload's own `span` when it is a well-formed span, otherwise `directive_span`
/// - metas: the payload's `metas`, then `plugin` (the plugin's name) and `message`, which win over a
///   payload meta of the same name
///
/// A payload that is not valid JSON of that shape is reported on `directive_span` with a message saying so.
fn plugin_error(plugin: &str, directive_span: &SpanInfo, payload: Option<&[u8]>) -> StageError {
    let parsed = payload
        .ok_or_else(|| "it is not a memory block".to_owned())
        .and_then(|payload| serde_json::from_slice::<EmitErrorPayload>(payload).map_err(|e| e.to_string()));
    let (message, span, mut metas) = match parsed {
        Ok(payload) => {
            let span = payload
                .span
                .and_then(|span| well_formed_span(plugin, span))
                .unwrap_or_else(|| directive_span.clone());
            (payload.message, span, payload.metas.unwrap_or_default())
        }
        Err(e) => {
            warn!("plugin {plugin} called {EMIT_ERROR} with an invalid payload: {e}");
            (
                format!("the plugin called {EMIT_ERROR} with an invalid payload: {e}"),
                directive_span.clone(),
                HashMap::new(),
            )
        }
    };
    metas.insert(PLUGIN_META.to_owned(), plugin.to_owned());
    metas.insert(MESSAGE_META.to_owned(), message);
    StageError {
        kind: ErrorKind::PluginError,
        span,
        metas,
    }
}

/// the span a plugin gave, if it is well-formed
fn well_formed_span(plugin: &str, span: serde_json::Value) -> Option<SpanInfo> {
    match serde_json::from_value::<SpanInfo>(span) {
        Ok(span) if span.start <= span.end => Some(span),
        Ok(span) => {
            warn!(
                "plugin {plugin} reported an error on a span ending before it starts ({}..{}); using its directive's span",
                span.start, span.end
            );
            None
        }
        Err(e) => {
            warn!("plugin {plugin} reported an error on a span that is not well-formed ({e}); using its directive's span");
            None
        }
    }
}

#[cfg(test)]
mod test {
    use std::collections::HashMap;
    use std::path::PathBuf;

    use zhang_ast::error::ErrorKind;
    use zhang_ast::SpanInfo;

    use super::plugin_error;

    fn directive_span() -> SpanInfo {
        SpanInfo {
            start: 0,
            end: 25,
            content: "plugin \"validator.wasm\"\n".to_owned(),
            filename: Some(PathBuf::from("main.zhang")),
        }
    }

    fn metas(entries: &[(&str, &str)]) -> HashMap<String, String> {
        entries.iter().map(|(key, value)| (key.to_string(), value.to_string())).collect()
    }

    #[test]
    fn should_report_on_the_directive_span_with_the_plugin_and_message() {
        let error = plugin_error("validator", &directive_span(), Some(br#"{"message": "payee is missing"}"#));

        assert_eq!(error.kind, ErrorKind::PluginError);
        assert_eq!(error.span, directive_span());
        assert_eq!(error.metas, metas(&[("plugin", "validator"), ("message", "payee is missing")]));
    }

    #[test]
    fn should_keep_plugin_metas_but_let_the_host_keys_win() {
        let payload = br#"{"message": "payee is missing", "metas": {"rule": "R1", "plugin": "spoofed", "message": "spoofed"}}"#;

        let error = plugin_error("validator", &directive_span(), Some(payload));

        assert_eq!(error.metas, metas(&[("rule", "R1"), ("plugin", "validator"), ("message", "payee is missing")]));
    }

    #[test]
    fn should_use_a_well_formed_span_from_the_plugin() {
        let payload = br#"{"message": "m", "span": {"start": 40, "end": 90, "content": "2024-01-02 * \"lunch\"", "filename": "other.zhang"}, "metas": null}"#;

        let error = plugin_error("validator", &directive_span(), Some(payload));

        assert_eq!(
            error.span,
            SpanInfo {
                start: 40,
                end: 90,
                content: "2024-01-02 * \"lunch\"".to_owned(),
                filename: Some(PathBuf::from("other.zhang")),
            }
        );
    }

    #[test]
    fn should_fall_back_to_the_directive_span_for_a_span_that_is_not_well_formed() {
        for span in [
            r#"null"#,
            r#""line 3""#,
            r#"{"start": 3}"#,
            r#"{"start": -1, "end": 4, "content": ""}"#,
            r#"{"start": 9, "end": 4, "content": ""}"#,
        ] {
            let payload = format!(r#"{{"message": "m", "span": {span}}}"#);

            let error = plugin_error("validator", &directive_span(), Some(payload.as_bytes()));

            assert_eq!(error.span, directive_span(), "span {span}");
            assert_eq!(error.metas, metas(&[("plugin", "validator"), ("message", "m")]), "span {span}");
        }
    }

    #[test]
    fn should_report_an_invalid_payload_without_failing() {
        for payload in [
            &b"not json"[..],
            br#"{"metas": {"rule": "R1"}}"#,
            br#"{"message": 42}"#,
            br#"{"message": "m", "metas": {"count": 3}}"#,
            b"\xff\xfe",
        ] {
            let error = plugin_error("validator", &directive_span(), Some(payload));

            assert_eq!(error.kind, ErrorKind::PluginError);
            assert_eq!(error.span, directive_span());
            assert_eq!(error.metas.len(), 2, "only the host metas: {:?}", error.metas);
            assert_eq!(error.metas["plugin"], "validator");
            assert!(
                error.metas["message"].starts_with("the plugin called zhang_emit_error with an invalid payload: "),
                "{}",
                error.metas["message"]
            );
        }
    }

    #[test]
    fn should_report_a_missing_memory_block_as_an_invalid_payload() {
        let error = plugin_error("validator", &directive_span(), None);

        assert_eq!(error.span, directive_span());
        assert_eq!(
            error.metas["message"],
            "the plugin called zhang_emit_error with an invalid payload: it is not a memory block"
        );
    }
}
