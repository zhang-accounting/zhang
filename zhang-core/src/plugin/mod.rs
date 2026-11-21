//! The WASM plugin runtime.
//!
//! Plugins are written against the plugin ABI the host functions and config keys of these modules
//! define; the `zhang-plugin-sdk` crate wraps it for plugin authors. [`PluginType`] lives in
//! `zhang-ast`, so the host and the SDK share its serde representation.

use std::borrow::Cow;

pub use semver::Version;
pub use zhang_ast::PluginType;

pub mod analyzer;
pub mod bundled;
pub mod capabilities;
pub mod files;
pub mod host;
pub mod http;
pub mod lua;
pub mod net;
pub mod router;
pub mod runtime;
pub mod stage;
pub mod store;
pub mod write;

/// `text` from a plugin (its name, a message, an error) made safe to put into a log line: every control character
/// (a newline, a carriage return, an escape sequence) and every character that changes the direction or the line of
/// the text around it is written as an escape, `\n` or `\u{1b}`, so a plugin cannot end the log line it is in and
/// forge the lines after it. Anything else, a backslash too, is kept as it is.
pub fn log_safe(text: &str) -> Cow<'_, str> {
    fn unsafe_char(c: char) -> bool {
        c.is_control() || matches!(c, '\u{2028}' | '\u{2029}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
    }
    if !text.chars().any(unsafe_char) {
        return Cow::Borrowed(text);
    }
    let mut escaped = String::with_capacity(text.len() + 8);
    for c in text.chars() {
        match c {
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            c if unsafe_char(c) => escaped.extend(c.escape_unicode()),
            c => escaped.push(c),
        }
    }
    Cow::Owned(escaped)
}

#[cfg(test)]
mod test {
    use super::log_safe;

    #[test]
    fn should_escape_what_could_forge_a_log_line() {
        assert_eq!(log_safe("fx-rate"), "fx-rate");
        assert_eq!(log_safe("汇率 plugin \\n"), "汇率 plugin \\n", "a backslash and other text are kept");
        assert_eq!(
            log_safe("ok\n[2024-01-01T00:00:00Z ERROR zhang] forged"),
            "ok\\n[2024-01-01T00:00:00Z ERROR zhang] forged"
        );
        assert_eq!(log_safe("a\r\tb\u{1b}[31mred\u{7f}"), "a\\r\\tb\\u{1b}[31mred\\u{7f}");
        assert_eq!(log_safe("x\u{2028}y\u{202e}z\u{85}"), "x\\u{2028}y\\u{202e}z\\u{85}");
    }
}
