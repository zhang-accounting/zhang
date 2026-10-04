//! Reporting problems in the ledger without failing the load.
//!
//! A reported problem becomes a `PluginError` in the ledger's error list, the list the web UI and
//! `GET /api/errors` show, with the metas `plugin` (the plugin's name) and `message`, plus the plugin's own metas.
//! It points at the directive whose span the plugin passes, or at the plugin's `plugin` directive. The ledger
//! still loads; this is how validator plugins work.
//!
//! While a router handles a request there is no error list: zhang only logs what the plugin reports. Tell the
//! caller about a problem in the response instead.
//!
//! To abort the whole load instead, return an [`Error`](crate::Error) from the handler.

use std::collections::BTreeMap;

use serde::Serialize;
use zhang_ast::SpanInfo;

use crate::abi;

/// what `zhang_emit_error` receives
#[derive(Serialize)]
struct Payload<'a> {
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    span: Option<&'a SpanInfo>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    metas: BTreeMap<String, String>,
}

/// report `message` on the plugin's `plugin` directive
pub fn emit_error(message: impl Into<String>) {
    abi::emit_error(&payload(None, message.into(), BTreeMap::new()));
}

/// Report `message` on the directive whose span is `span` (the [`Spanned::span`](zhang_ast::Spanned) of a
/// directive the plugin received), with `metas` as extra error metas. The metas `plugin` and `message` are
/// zhang's own and win over a meta of the same name.
///
/// ```
/// use zhang_plugin_sdk::errors::emit_error_at;
/// use zhang_plugin_sdk::SpanInfo;
///
/// # let span = SpanInfo::default();
/// emit_error_at(&span, "payee is missing", [("rule", "require-payee")]);
/// ```
pub fn emit_error_at<K: Into<String>, V: Into<String>>(span: &SpanInfo, message: impl Into<String>, metas: impl IntoIterator<Item = (K, V)>) {
    let metas = metas.into_iter().map(|(key, value)| (key.into(), value.into())).collect();
    abi::emit_error(&payload(Some(span), message.into(), metas));
}

fn payload(span: Option<&SpanInfo>, message: String, metas: BTreeMap<String, String>) -> Vec<u8> {
    serde_json::to_vec(&Payload { message, span, metas }).expect("strings and spans always serialize to JSON")
}

#[cfg(test)]
mod test {
    use std::collections::BTreeMap;

    use serde_json::json;
    use zhang_ast::SpanInfo;

    use super::payload;

    #[test]
    fn should_send_the_payload_the_host_reads() {
        let bare: serde_json::Value = serde_json::from_slice(&payload(None, "boom".to_owned(), BTreeMap::new())).unwrap();
        assert_eq!(bare, json!({"message": "boom"}));

        let span = SpanInfo {
            start: 3,
            end: 9,
            content: "x".to_owned(),
            filename: Some("main.zhang".into()),
            ..SpanInfo::default()
        };
        let metas = BTreeMap::from([("rule".to_owned(), "payee".to_owned())]);
        let full: serde_json::Value = serde_json::from_slice(&payload(Some(&span), "no payee".to_owned(), metas)).unwrap();
        assert_eq!(
            full,
            json!({"message": "no payee", "span": {"start": 3, "end": 9, "content": "x", "filename": "main.zhang"}, "metas": {"rule": "payee"}})
        );
    }
}
