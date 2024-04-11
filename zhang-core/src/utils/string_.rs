use std::borrow::Cow;

use unicode_categories::UnicodeCategories;
use zhang_ast::{SpanInfo, ZhangString};

pub trait StringExt {
    fn to_quote(&self) -> ZhangString;
    fn to_unquote(&self) -> ZhangString;
    fn into_quote(self) -> ZhangString;
    fn into_unquote(self) -> ZhangString;

    fn replace_by_span(&mut self, span: &SpanInfo, content: &str);
}

impl StringExt for String {
    fn to_quote(&self) -> ZhangString {
        ZhangString::QuoteString(self.to_owned())
    }

    fn to_unquote(&self) -> ZhangString {
        ZhangString::UnquoteString(self.to_owned())
    }

    fn into_quote(self) -> ZhangString {
        ZhangString::QuoteString(self)
    }

    fn into_unquote(self) -> ZhangString {
        ZhangString::UnquoteString(self)
    }

    fn replace_by_span(&mut self, span: &SpanInfo, content: &str) {
        self.replace_range(span.start..span.end, "");
        self.insert_str(span.start, content);
    }
}

pub fn escape_with_quote(s: &str) -> Cow<str> {
    let mut output = String::with_capacity(s.len());
    output.push('"');

    for c in s.chars() {
        if c == '"' {
            output += "\\\"";
        } else if c == '\\' {
            output += "\\\\";
        } else if c == ' ' {
            // avoid 'escape_unicode' for ' ' even though it's a separator
            output.push(c);
        } else if c == '$' {
            output += "\\$";
        } else if c == '`' {
            output += "\\`";
        } else if c.is_other() || c.is_separator() {
            output += &escape_character(c);
        } else {
            output.push(c);
        }
    }

    output.push('"');
    output.into()
}

fn escape_character(c: char) -> String {
    match c {
        '\u{07}' => "\\a".to_string(),
        '\u{08}' => "\\b".to_string(),
        '\u{0b}' => "\\v".to_string(),
        '\u{0c}' => "\\f".to_string(),
        '\u{1b}' => "\\e".to_string(),
        c => {
            // escape_default does the right thing for \t, \r, \n, and unicode
            c.escape_default().to_string()
        }
    }
}

#[cfg(test)]
mod test {
    use zhang_ast::SpanInfo;

    use crate::utils::string_::{escape_with_quote, StringExt};

    #[test]
    fn test_escapse_with_quote() {
        assert_eq!(r#""a""#, escape_with_quote("a"));
        assert_eq!(r#""a\"""#, escape_with_quote("a\""));
        assert_eq!(r#""a\$""#, escape_with_quote("a$"));
        assert_eq!(r#""a\\""#, escape_with_quote("a\\"));
        assert_eq!(r#""a ""#, escape_with_quote("a "));
        assert_eq!(r#""\`""#, escape_with_quote("`"));
        assert_eq!(r#""\a\b\v\f\e""#, escape_with_quote("\u{07}\u{08}\u{0b}\u{0c}\u{1b}"));
    }

    #[test]
    fn test_replace_by_span() {
        let info = SpanInfo {
            start: 1,
            end: 4,
            content: "".to_string(),
            filename: None,
        };

        let mut origin = "helloworld".to_string();
        origin.replace_by_span(&info, "new");
        assert_eq!(origin, "hnew0world");
    }
}
