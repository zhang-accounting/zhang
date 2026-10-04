//! Quoting and unquoting of the strings in ledger files.
//!
//! [`quote_as`] / [`escape_with_quote`] (writing) and [`quoted_string`] (reading)
//! are inverses: for every string `s` and every [`QuoteStyle`],
//! `quoted_string(&quote_as(s, style))` returns `s` and consumes the whole input.
//! The zhang and the beancount text parsers both read quoted strings with
//! [`quoted_string`], so the two data types follow the same rules.
//!
//! # Writing
//!
//! Inside the quotes only `"` and `\` are escaped, as `\"` and `\\`, plus some
//! control characters (Unicode category `Cc`), depending on the [`QuoteStyle`]:
//!
//! - [`QuoteStyle::Zhang`] writes `\n`, `\t`, `\r`, and every other control
//!   character as `\uXXXX` with four hex digits. So are the characters that are
//!   invisible or reorder the text around them in an editor: the bidi controls
//!   (U+061C, U+200E, U+200F, U+202A–U+202E, U+2066–U+2069), the line and
//!   paragraph separators U+2028 and U+2029, which some editors also drop, the
//!   zero-width space U+200B and the byte order mark U+FEFF.
//! - [`QuoteStyle::Beancount`] writes only the escapes Python beancount decodes,
//!   `\n`, `\t`, `\r`, `\b` and `\f`, and every other control character as it is,
//!   because beancount has no unicode escape and would read `\u0007` as `u0007`.
//!
//! Every other character, such as `$`, `` ` ``, a no-break space, an emoji (with
//! its zero-width joiners) or CJK text, is written as it is. Neither style writes the legacy escapes `\$`,
//! `` \` `` or `\u{..}`.
//!
//! # Reading
//!
//! - `\"`, `\\`, `\/`, `\b`, `\f`, `\n`, `\r`, `\t` and `\uXXXX` decode as in
//!   JSON. A UTF-16 surrogate pair written as two `\uXXXX` escapes decodes to one
//!   character.
//! - The escapes written by older zhang versions decode too: `\$` to `$`, `` \` ``
//!   to `` ` ``, `\a`, `\v` and `\e` to BEL, VT and ESC, and `\u{H}` with one to
//!   six hex digits to that character.
//! - Any other escape `\X` is kept **verbatim**, as the two characters `\X`, so the
//!   regular expression `'\d+'` reads as `\d+`. Python beancount drops the
//!   backslash instead (`d+`); the two only differ for text that beancount would
//!   already change.
//! - A malformed `\u` escape (`\uZZZZ`, `\u{}`, more than six hex digits, a value
//!   above `10FFFF` or a lone surrogate) is a parse error reported at the
//!   backslash, see [`invalid_escape_at`].
//! - A backslash always escapes the character after it, so `\"` never ends the
//!   string: a string cannot end with a single backslash, write `\\` instead. A
//!   string such as `"abc\"` with no later quote is unterminated, which is a parse
//!   error as well.

use std::borrow::Cow;
use std::fmt::Write;

use nom::bytes::complete::take_while1;
use nom::character::complete::char;
use nom::error::{Error as NomError, ErrorKind};
use nom::multi::fold_many0;
use nom::sequence::delimited;
use nom::{Err as NomErr, IResult};
use zhang_ast::{SpanInfo, ZhangString};

pub trait StringExt {
    fn to_quote(&self) -> ZhangString;

    fn replace_by_span(&mut self, span: &SpanInfo, content: &str);
}

impl StringExt for String {
    fn to_quote(&self) -> ZhangString {
        ZhangString::QuoteString(self.to_owned())
    }

    fn replace_by_span(&mut self, span: &SpanInfo, content: &str) {
        self.replace_range(span.start..span.end, "");
        self.insert_str(span.start, content);
    }
}

/// How [`quote_as`] writes control characters; see the [module docs](self).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum QuoteStyle {
    /// zhang's text format: `\n`, `\t`, `\r`, and `\uXXXX` for other controls and
    /// for invisible or text-reordering characters, see the [module docs](self).
    #[default]
    Zhang,
    /// beancount's text format: only the escapes Python beancount decodes
    /// (`\n`, `\t`, `\r`, `\b`, `\f`), other controls are written raw.
    Beancount,
}

/// Write `s` as a double-quoted ledger string in zhang's [`QuoteStyle`]. See the
/// [module docs](self) for the rules; [`quoted_string`] reads the result back to `s`.
pub fn escape_with_quote(s: &str) -> Cow<'_, str> {
    quote_as(s, QuoteStyle::Zhang).into()
}

/// Write `s` as a double-quoted ledger string in the given style. See the
/// [module docs](self) for the rules; [`quoted_string`] reads the result back to `s`.
pub fn quote_as(s: &str, style: QuoteStyle) -> String {
    let mut output = String::with_capacity(s.len() + 2);
    output.push('"');

    for c in s.chars() {
        match (c, style) {
            ('"', _) => output.push_str("\\\""),
            ('\\', _) => output.push_str("\\\\"),
            ('\n', _) => output.push_str("\\n"),
            ('\t', _) => output.push_str("\\t"),
            ('\r', _) => output.push_str("\\r"),
            ('\u{08}', QuoteStyle::Beancount) => output.push_str("\\b"),
            ('\u{0c}', QuoteStyle::Beancount) => output.push_str("\\f"),
            // all of these are in the BMP, so four hex digits are enough
            (c, QuoteStyle::Zhang) if c.is_control() || is_hidden_format_char(c) => {
                write!(output, "\\u{:04x}", c as u32).expect("writing to a String cannot fail");
            }
            (c, _) => output.push(c),
        }
    }

    output.push('"');
    output
}

/// Characters [`QuoteStyle::Zhang`] writes as `\uXXXX` although they are not
/// controls: they are invisible, or reorder the text around them in an editor
/// (the "Trojan Source" bidi controls), or are dropped by some editors.
fn is_hidden_format_char(c: char) -> bool {
    matches!(
        c,
        '\u{061c}' | '\u{200b}' | '\u{200e}' | '\u{200f}' | '\u{2028}' | '\u{2029}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{feff}'
    )
}

/// The nom error kind of a malformed escape sequence. It is raised as
/// [`nom::Err::Failure`], so it is never backtracked over.
const INVALID_ESCAPE: ErrorKind = ErrorKind::EscapedTransform;

/// If `err` is the malformed escape error raised by [`quoted_string`], the input
/// starting at the backslash of that escape. Parsers use it to report the error at
/// the escape instead of at the start of the directive.
pub fn invalid_escape_at<'a>(err: &NomErr<NomError<&'a str>>) -> Option<&'a str> {
    match err {
        NomErr::Failure(error) if error.code == INVALID_ESCAPE => Some(error.input),
        _ => None,
    }
}

/// One piece of the body of a quoted string, after decoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StringFragment<'a> {
    /// A run of characters that are not escaped, kept as they are.
    Literal(&'a str),
    /// The character an escape sequence decodes to.
    Escaped(char),
    /// An unknown escape `\X`, kept verbatim as the two characters `\` and `X`.
    Verbatim(char),
}

impl StringFragment<'_> {
    fn push_to(self, output: &mut String) {
        match self {
            StringFragment::Literal(literal) => output.push_str(literal),
            StringFragment::Escaped(c) => output.push(c),
            StringFragment::Verbatim(c) => {
                output.push('\\');
                output.push(c);
            }
        }
    }
}

/// Parse one [`StringFragment`] of a quoted string body: a run of literal
/// characters or a single escape sequence.
///
/// Fails with a recoverable error on the closing `"`, at the end of the input and
/// on a backslash at the end of the input. A malformed `\u` escape fails with
/// [`nom::Err::Failure`] at the backslash.
pub fn string_fragment(i: &str) -> IResult<&str, StringFragment<'_>> {
    if i.starts_with('\\') {
        escape_sequence(i)
    } else {
        let (rest, literal) = take_while1(|c: char| c != '"' && c != '\\')(i)?;
        Ok((rest, StringFragment::Literal(literal)))
    }
}

/// Parse a double-quoted string and decode its escape sequences. See the
/// [module docs](self) for the rules.
pub fn quoted_string(i: &str) -> IResult<&str, String> {
    delimited(
        char('"'),
        fold_many0(string_fragment, String::new, |mut output, fragment| {
            fragment.push_to(&mut output);
            output
        }),
        char('"'),
    )(i)
}

/// Parse an escape sequence; `i` starts with the backslash.
fn escape_sequence(i: &str) -> IResult<&str, StringFragment<'_>> {
    let (after_backslash, _) = char('\\')(i)?;
    let mut chars = after_backslash.chars();
    let Some(escaped) = chars.next() else {
        // a backslash at the end of the input: the string is unterminated
        return Err(NomErr::Error(NomError::new(i, ErrorKind::Char)));
    };
    let rest = chars.as_str();
    let decoded = match escaped {
        '"' => '"',
        '\\' => '\\',
        '/' => '/',
        'b' => '\u{08}',
        'f' => '\u{0c}',
        'n' => '\n',
        'r' => '\r',
        't' => '\t',
        'u' => return unicode_escape(i, rest),
        // written by older zhang versions
        '$' => '$',
        '`' => '`',
        'a' => '\u{07}',
        'v' => '\u{0b}',
        'e' => '\u{1b}',
        // unknown escapes are kept verbatim, see the module docs
        other => return Ok((rest, StringFragment::Verbatim(other))),
    };
    Ok((rest, StringFragment::Escaped(decoded)))
}

/// Decode `\uXXXX`, a surrogate pair `\uXXXX\uXXXX` or the legacy `\u{H}` with one
/// to six hex digits. `escape` starts at the backslash, `rest` follows the `u`.
fn unicode_escape<'a>(escape: &'a str, rest: &'a str) -> IResult<&'a str, StringFragment<'a>> {
    let invalid = || NomErr::Failure(NomError::new(escape, INVALID_ESCAPE));

    if let Some(braced) = rest.strip_prefix('{') {
        let digits_len = braced.find(|c: char| !c.is_ascii_hexdigit()).unwrap_or(braced.len());
        let digits = &braced[..digits_len];
        let rest = braced[digits_len..].strip_prefix('}').ok_or_else(invalid)?;
        if digits.is_empty() || digits.len() > 6 {
            return Err(invalid());
        }
        let value = u32::from_str_radix(digits, 16).map_err(|_| invalid())?;
        let decoded = char::from_u32(value).ok_or_else(invalid)?;
        return Ok((rest, StringFragment::Escaped(decoded)));
    }

    let (rest, value) = four_hex_digits(rest).ok_or_else(invalid)?;
    if let Some(decoded) = char::from_u32(value) {
        return Ok((rest, StringFragment::Escaped(decoded)));
    }
    // a high surrogate must be followed by a `\uXXXX` low surrogate
    if (0xD800..0xDC00).contains(&value) {
        if let Some((rest, low)) = rest.strip_prefix("\\u").and_then(four_hex_digits) {
            if (0xDC00..0xE000).contains(&low) {
                let combined = 0x10000 + ((value - 0xD800) << 10) + (low - 0xDC00);
                let decoded = char::from_u32(combined).ok_or_else(invalid)?;
                return Ok((rest, StringFragment::Escaped(decoded)));
            }
        }
    }
    Err(invalid())
}

/// Read exactly four hex digits from the start of `i`.
fn four_hex_digits(i: &str) -> Option<(&str, u32)> {
    let digits = i.get(..4)?;
    if !digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let value = u32::from_str_radix(digits, 16).ok()?;
    Some((&i[4..], value))
}

#[cfg(test)]
pub(crate) mod test {
    use nom::error::ErrorKind;
    use nom::Err as NomErr;
    use zhang_ast::SpanInfo;

    use super::{escape_with_quote, invalid_escape_at, quote_as, quoted_string, string_fragment, QuoteStyle, StringExt, StringFragment};

    /// A small xorshift PRNG, so the property tests need no extra dependency and
    /// are reproducible.
    pub(crate) struct XorShift(u64);

    impl XorShift {
        pub(crate) fn new(seed: u64) -> Self {
            Self(seed.max(1))
        }

        pub(crate) fn next(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            self.0 = x;
            x
        }

        pub(crate) fn below(&mut self, bound: usize) -> usize {
            (self.next() % bound as u64) as usize
        }
    }

    /// A random string drawn mostly from the characters that need care: quotes,
    /// backslashes, legacy escape characters, controls, separators, emoji and CJK.
    pub(crate) fn random_string(rng: &mut XorShift) -> String {
        const INTERESTING: &[char] = &[
            '"', '\\', '$', '`', '\'', '/', 'u', '{', '}', 'd', 'n', ' ', '\n', '\r', '\t', '\u{0}', '\u{07}', '\u{08}', '\u{0b}', '\u{0c}', '\u{1b}',
            '\u{7f}', '\u{85}', '\u{9f}', '\u{a0}', '\u{ad}', '\u{200b}', '\u{200d}', '\u{2028}', '\u{2029}', '\u{3000}', '\u{feff}', '😀', '👨', '你', '好',
            '账', 'é', 'a', 'Z', '0', ';', '#', ':',
        ];
        let len = rng.below(24);
        (0..len)
            .map(|_| {
                if rng.below(5) == 0 {
                    // any scalar value, including unassigned and private-use ones
                    loop {
                        if let Some(c) = char::from_u32((rng.next() % 0x11_0000) as u32) {
                            break c;
                        }
                    }
                } else {
                    INTERESTING[rng.below(INTERESTING.len())]
                }
            })
            .collect()
    }

    fn unquote(input: &str) -> String {
        let (rest, value) = quoted_string(input).unwrap_or_else(|err| panic!("{input:?} should parse: {err:?}"));
        assert_eq!(rest, "", "{input:?} should be consumed entirely");
        value
    }

    #[test]
    fn test_escape_with_quote() {
        assert_eq!(r#""a""#, escape_with_quote("a"));
        assert_eq!(r#""a\"""#, escape_with_quote("a\""));
        // `$` and the backtick are written raw; older versions wrote `\$` and `` \` ``
        assert_eq!(r#""a$""#, escape_with_quote("a$"));
        assert_eq!(r#""a\\""#, escape_with_quote("a\\"));
        assert_eq!(r#""a ""#, escape_with_quote("a "));
        assert_eq!(r#""`""#, escape_with_quote("`"));
        // controls are written as `\uXXXX`; older versions wrote `\a\b\v\f\e`
        assert_eq!(r#""\u0007\u0008\u000b\u000c\u001b""#, escape_with_quote("\u{07}\u{08}\u{0b}\u{0c}\u{1b}"));
        assert_eq!(r#""\\d+""#, escape_with_quote("\\d+"));
    }

    #[test]
    fn escape_with_quote_writes_dollar_and_backtick_raw() {
        assert_eq!(r#""coffee $5""#, escape_with_quote("coffee $5"));
        assert_eq!(r#""`code`""#, escape_with_quote("`code`"));
    }

    #[test]
    fn escape_with_quote_writes_controls_as_short_escapes() {
        assert_eq!(r#""a\nb\tc\rd""#, escape_with_quote("a\nb\tc\rd"));
        assert_eq!(
            r#""\u0000\u0007\u0008\u000b\u000c\u001b\u007f\u0085\u009f""#,
            escape_with_quote("\u{0}\u{07}\u{08}\u{0b}\u{0c}\u{1b}\u{7f}\u{85}\u{9f}")
        );
    }

    #[test]
    fn escape_with_quote_writes_separators_and_symbols_raw() {
        for raw in ["\u{a0}", "\u{3000}", "\u{200c}", "\u{200d}", "\u{ad}", "😀", "👨‍👩‍👧", "你好", "é"] {
            assert_eq!(format!("\"{raw}\""), escape_with_quote(raw), "{raw:?} should be written raw");
            assert_eq!(format!("\"{raw}\""), quote_as(raw, QuoteStyle::Beancount), "{raw:?} should be written raw");
        }
    }

    #[test]
    fn zhang_style_escapes_invisible_and_bidi_characters() {
        let hidden = [
            ('\u{061c}', r"\u061c"),
            ('\u{200b}', r"\u200b"),
            ('\u{200e}', r"\u200e"),
            ('\u{200f}', r"\u200f"),
            ('\u{2028}', r"\u2028"),
            ('\u{2029}', r"\u2029"),
            ('\u{202a}', r"\u202a"),
            ('\u{202b}', r"\u202b"),
            ('\u{202c}', r"\u202c"),
            ('\u{202d}', r"\u202d"),
            ('\u{202e}', r"\u202e"),
            ('\u{2066}', r"\u2066"),
            ('\u{2067}', r"\u2067"),
            ('\u{2068}', r"\u2068"),
            ('\u{2069}', r"\u2069"),
            ('\u{feff}', r"\ufeff"),
        ];
        for (c, escaped) in hidden {
            let s = format!("a{c}b");
            assert_eq!(escape_with_quote(&s), format!("\"a{escaped}b\""), "{c:?}");
            // beancount has no unicode escape, so the beancount style writes them raw
            assert_eq!(quote_as(&s, QuoteStyle::Beancount), format!("\"{s}\""), "{c:?}");
            assert_eq!(unquote(&escape_with_quote(&s)), s);
        }
        // a right-to-left override cannot hide the closing quote from a reader
        assert_eq!(escape_with_quote("\u{202e}cba"), r#""\u202ecba""#);
    }

    #[test]
    fn beancount_style_writes_only_the_escapes_beancount_decodes() {
        assert_eq!(r#""a\"b\\c$`""#, quote_as("a\"b\\c$`", QuoteStyle::Beancount));
        assert_eq!(r#""\n\t\r\b\f""#, quote_as("\n\t\r\u{08}\u{0c}", QuoteStyle::Beancount));
        let raw_controls = "\u{0}\u{07}\u{0b}\u{1b}\u{7f}\u{85}\u{9f}";
        assert_eq!(format!("\"{raw_controls}\""), quote_as(raw_controls, QuoteStyle::Beancount));
    }

    #[test]
    fn quote_styles_never_write_unicode_or_legacy_escapes() {
        let mut rng = XorShift::new(0x5eed_0001);
        for _ in 0..2000 {
            let s = random_string(&mut rng);
            let zhang = escape_with_quote(&s);
            assert!(!zhang.contains("\\u{"), "{s:?} was written as {zhang}");
            let beancount = quote_as(&s, QuoteStyle::Beancount);
            // pair each backslash with the next character, the way the parser reads escapes
            let mut chars = beancount.chars();
            while let Some(c) = chars.next() {
                if c == '\\' {
                    let escaped = chars.next();
                    assert!(
                        matches!(escaped, Some('"' | '\\' | 'n' | 't' | 'r' | 'b' | 'f')),
                        "{s:?} was written as {beancount} with the escape \\{escaped:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn string_fragment_reads_literals_up_to_a_quote_or_backslash() {
        assert_eq!(string_fragment("ab c\"x"), Ok(("\"x", StringFragment::Literal("ab c"))));
        assert_eq!(string_fragment("你好\\n"), Ok(("\\n", StringFragment::Literal("你好"))));
        assert_eq!(string_fragment("a\nb\""), Ok(("\"", StringFragment::Literal("a\nb"))));
        assert!(matches!(string_fragment("\"x"), Err(NomErr::Error(_))));
        assert!(matches!(string_fragment(""), Err(NomErr::Error(_))));
    }

    #[test]
    fn string_fragment_decodes_known_escapes() {
        let cases = [
            ("\\\"", '"'),
            ("\\\\", '\\'),
            ("\\/", '/'),
            ("\\b", '\u{08}'),
            ("\\f", '\u{0c}'),
            ("\\n", '\n'),
            ("\\r", '\r'),
            ("\\t", '\t'),
            ("\\u0041", 'A'),
            ("\\u00a0", '\u{a0}'),
            ("\\u2028", '\u{2028}'),
            ("\\uD83D\\uDE00", '😀'),
        ];
        for (input, expected) in cases {
            assert_eq!(string_fragment(input), Ok(("", StringFragment::Escaped(expected))), "{input:?}");
        }
    }

    #[test]
    fn string_fragment_decodes_legacy_escapes() {
        let cases = [
            ("\\$", '$'),
            ("\\`", '`'),
            ("\\a", '\u{07}'),
            ("\\v", '\u{0b}'),
            ("\\e", '\u{1b}'),
            ("\\u{a0}", '\u{a0}'),
            ("\\u{A0}", '\u{a0}'),
            ("\\u{0}", '\u{0}'),
            ("\\u{2028}", '\u{2028}'),
            ("\\u{1F600}", '😀'),
            ("\\u{01f600}", '😀'),
            ("\\u{10FFFF}", '\u{10ffff}'),
        ];
        for (input, expected) in cases {
            assert_eq!(string_fragment(input), Ok(("", StringFragment::Escaped(expected))), "{input:?}");
        }
    }

    #[test]
    fn string_fragment_keeps_unknown_escapes_verbatim() {
        for c in ['d', 'w', 's', '.', '(', '[', '\'', ' ', 'x', 'é', '你', '\n'] {
            let input = format!("\\{c}rest");
            assert_eq!(string_fragment(&input), Ok(("rest", StringFragment::Verbatim(c))), "{input:?}");
        }
    }

    #[test]
    fn string_fragment_rejects_malformed_unicode_escapes() {
        for input in [
            "\\uZZZZ",
            "\\u12",
            "\\u",
            "\\u{}",
            "\\u{110000}",
            "\\u{D800}",
            "\\u{DFFF}",
            "\\u{1234567}",
            "\\u{12",
            "\\u{xyz}",
            "\\uD800",
            "\\uDC00",
            "\\uD800\\u0041",
            "\\uD800x",
        ] {
            let err = string_fragment(input).expect_err(input);
            assert_eq!(invalid_escape_at(&err), Some(input), "{input:?} should fail at its backslash");
        }
    }

    #[test]
    fn string_fragment_treats_a_trailing_backslash_as_unterminated() {
        assert!(matches!(string_fragment("\\"), Err(NomErr::Error(error)) if error.code == ErrorKind::Char));
    }

    #[test]
    fn quoted_string_decodes_mixed_content() {
        assert_eq!(unquote(r#""""#), "");
        assert_eq!(unquote(r#""coffee $5""#), "coffee $5");
        assert_eq!(unquote(r#""coffee \$5""#), "coffee $5");
        assert_eq!(unquote(r#""SELECT\u{a0}account""#), "SELECT\u{a0}account");
        assert_eq!(unquote(r#""narration ~ '\d+'""#), "narration ~ '\\d+'");
        assert_eq!(unquote(r#""narration ~ '\\d+'""#), "narration ~ '\\d+'");
        assert_eq!(unquote("\"two\nlines\""), "two\nlines");
        assert_eq!(unquote(r#""say \"hi\"""#), "say \"hi\"");
        assert_eq!(unquote(r#""ends with \\""#), "ends with \\");
    }

    #[test]
    fn quoted_string_leaves_the_rest_of_the_input() {
        assert_eq!(quoted_string(r#""a\"b" "c""#), Ok((r#" "c""#, "a\"b".to_string())));
    }

    #[test]
    fn quoted_string_errors_instead_of_panicking() {
        // unterminated strings, including a lone backslash before the closing quote, are recoverable errors
        for input in ["\"abc", "\"abc\\", "\"abc\\\"", "\"", "abc", ""] {
            let err = quoted_string(input).expect_err(input);
            assert!(matches!(err, NomErr::Error(_)), "{input:?}: {err:?}");
        }
        // malformed unicode escapes are failures positioned at the backslash
        for (input, at) in [
            ("\"a\\u{110000}b\"", "\\u{110000}b\""),
            ("\"a\\u{D800}b\"", "\\u{D800}b\""),
            ("\"a\\uZZZZb\"", "\\uZZZZb\""),
        ] {
            let err = quoted_string(input).expect_err(input);
            assert_eq!(invalid_escape_at(&err), Some(at), "{input:?}");
        }
    }

    #[test]
    fn export_then_parse_is_an_identity() {
        let mut rng = XorShift::new(0x5eed_0442);
        for _ in 0..20_000 {
            let s = random_string(&mut rng);
            assert_eq!(unquote(&escape_with_quote(&s)), s);
            assert_eq!(unquote(&quote_as(&s, QuoteStyle::Beancount)), s);
        }
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
        assert_eq!(origin, "hnewoworld");
    }
}
