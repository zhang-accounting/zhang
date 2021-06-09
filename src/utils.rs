use std::borrow::Cow;
use unicode_categories::UnicodeCategories;

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
    use crate::utils::escape_with_quote;

    #[test]
    fn test_escapse_with_quote() {
        assert_eq!(r#""a""#, escape_with_quote("a"));
        assert_eq!(r#""a\"""#, escape_with_quote("a\""));
        assert_eq!(r#""a\$""#, escape_with_quote("a$"));
        assert_eq!(r#""a\\""#, escape_with_quote("a\\"));
        assert_eq!(r#""a ""#, escape_with_quote("a "));
        assert_eq!(r#""\`""#, escape_with_quote("`"));
        assert_eq!(
            r#""\a\b\v\f\e""#,
            escape_with_quote("\u{07}\u{08}\u{0b}\u{0c}\u{1b}")
        );
    }
}
