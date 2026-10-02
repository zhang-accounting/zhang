//! `str`, `length` and `maxwidth`.

use crate::functions::FunctionContext;
use crate::value::{position_sort_cmp, Inventory, Value};

/// beancount's `Inventory.__str__`: the positions in position sort order (common currencies
/// first), comma separated, in parentheses.
pub(crate) fn inventory_to_string(inventory: &Inventory) -> String {
    let mut positions = inventory.positions().collect::<Vec<_>>();
    positions.sort_by(position_sort_cmp);
    format!("({})", positions.iter().map(ToString::to_string).collect::<Vec<_>>().join(", "))
}

/// beanquery `str`: `TRUE`/`FALSE` for booleans and beancount's text forms for amounts
/// (`10.00 USD`), positions (`10 AAPL {100 USD, 2024-01-01, "lot"}`) and inventories
/// (`(10.00 USD, 10 AAPL {100 USD, 2024-01-01})`). Numbers never use exponent notation
/// and sets are comma separated.
pub(super) fn str_(args: &[Value], _ctx: &dyn FunctionContext) -> Result<Value, String> {
    Ok(Value::Str(match &args[0] {
        Value::Inventory(inventory) => inventory_to_string(inventory),
        other => other.to_string(),
    }))
}

pub(super) fn length(args: &[Value], _ctx: &dyn FunctionContext) -> Result<Value, String> {
    let length = match &args[0] {
        Value::Str(string) => string.chars().count(),
        Value::Set(set) => set.len(),
        _ => return Err("length() expects a string or a set".to_owned()),
    };
    i64::try_from(length)
        .map(Value::Int)
        .map_err(|_| "length() overflows a 64-bit integer".to_owned())
}

/// What `maxwidth` appends to a shortened text.
const PLACEHOLDER: &str = " [...]";

/// beanquery `maxwidth(text, width)`, which is Python's `textwrap.shorten`: every run of
/// whitespace becomes one space and the ends are trimmed; a text still longer than `width`
/// characters keeps as many leading chunks as fit together with `" [...]"`. The width must
/// leave room for the placeholder (at least 5), as in Python.
pub(super) fn maxwidth(args: &[Value], _ctx: &dyn FunctionContext) -> Result<Value, String> {
    let text = args[0].as_str().ok_or("maxwidth() expects a string")?;
    let width = args[1].as_int().ok_or("maxwidth() expects an integer width")?;
    let min = PLACEHOLDER.trim_start().chars().count();
    match usize::try_from(width) {
        Ok(width) if width >= min => Ok(Value::Str(shorten(text, width))),
        _ => Err(format!(
            "the width must be at least {} to fit the placeholder '{}', got {}",
            min,
            PLACEHOLDER.trim_start(),
            width
        )),
    }
}

/// Python's `textwrap.shorten(text, width)` with its default options (`width` ≥ 5).
fn shorten(text: &str, width: usize) -> String {
    // `str.split()` also splits at the ASCII separators U+001C..U+001F
    let collapsed = text
        .split(|c: char| c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c))
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let chars = collapsed.chars().collect::<Vec<_>>();
    if chars.len() <= width {
        return collapsed;
    }
    // the longest run of whole chunks, ending in a word, that leaves room for the placeholder
    let budget = width.saturating_sub(PLACEHOLDER.chars().count());
    let mut kept = 0;
    for (start, end) in wrap_chunks(&chars) {
        if end > budget {
            break;
        }
        if chars[start] != ' ' {
            kept = end;
        }
    }
    if kept == 0 {
        return PLACEHOLDER.trim_start().to_owned();
    }
    let mut shortened = chars[..kept].iter().collect::<String>();
    shortened.push_str(PLACEHOLDER);
    shortened
}

/// The `(start, end)` character ranges `textwrap` breaks a collapsed text into: single
/// spaces, and words, where a word also breaks after a hyphen between letters
/// (`well-known` → `well-`, `known`; but not `e-mail`) and around an em-dash written as
/// `--` between words.
fn wrap_chunks(chars: &[char]) -> Vec<(usize, usize)> {
    let at = |idx: usize| chars.get(idx).copied().unwrap_or(' ');
    // `\w`, and textwrap's letters (`[^\d\W]`; only ASCII digits are taken for `\d`) and
    // word punctuation
    let word = |c: char| c == '_' || c.is_alphanumeric();
    let letter = |c: char| word(c) && !c.is_ascii_digit();
    let word_punct = |c: char| word(c) || "!\"'&.,?".contains(c);
    // a run of two or more hyphens at `idx` followed by a word character (an em-dash)
    let em_dash = |idx: usize| {
        let run = chars[idx.min(chars.len())..].iter().take_while(|c| **c == '-').count();
        run >= 2 && word(at(idx + run))
    };

    let mut chunks = vec![];
    let mut start = 0;
    while start < chars.len() {
        let end = if chars[start] == ' ' {
            start + 1
        } else if start > 0 && word_punct(chars[start - 1]) && em_dash(start) {
            start + chars[start..].iter().take_while(|c| **c == '-').count()
        } else {
            // the shortest word that ends at a breakable hyphen, before a space, at the end
            // of the text or before an em-dash
            let mut end = start + 1;
            loop {
                let hyphen_breaks = at(end) == '-'
                    && ((end >= 2 && letter(at(end - 2)) && letter(at(end - 1)))
                        || (end >= 3 && letter(at(end - 3)) && at(end - 2) == '-' && letter(at(end - 1))))
                    && letter(at(end + 1))
                    && (letter(at(end + 2)) || (at(end + 2) == '-' && letter(at(end + 3))));
                if hyphen_breaks {
                    break end + 1;
                }
                if end == chars.len() || chars[end] == ' ' || (word_punct(chars[end - 1]) && em_dash(end)) {
                    break end;
                }
                end += 1;
            }
        };
        chunks.push((start, end));
        start = end;
    }
    chunks
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::super::testing::*;
    use super::*;
    use crate::value::{Cost, Position};

    #[test]
    fn str_of_scalars() {
        assert_eq!(call("str", vec![Value::Int(-3)]), Value::from("-3"));
        assert_eq!(call("str", vec![Value::Decimal(d("1.50"))]), Value::from("1.50"));
        assert_eq!(call("str", vec![Value::Decimal(d("1E+3"))]), Value::from("1000"));
        assert_eq!(call("str", vec![Value::Bool(true)]), Value::from("TRUE"));
        assert_eq!(call("str", vec![Value::Bool(false)]), Value::from("FALSE"));
        assert_eq!(call("str", vec!["text".into()]), Value::from("text"));
        assert_eq!(call("str", vec![Value::Date(date("2024-01-05"))]), Value::from("2024-01-05"));
        let tags = ["tag2", "tag1"].iter().map(|it| it.to_string()).collect::<BTreeSet<_>>();
        assert_eq!(call("str", vec![Value::Set(tags)]), Value::from("tag1, tag2"));
        assert_eq!(call("str", vec![Value::Null]), Value::Null);
    }

    #[test]
    fn str_of_accounting_types() {
        assert_eq!(call("str", vec![Value::Amount(amount("-1000.00", "USD"))]), Value::from("-1000.00 USD"));
        let lot = Position::new(
            amount("10", "AAPL"),
            Some(Cost {
                number: d("100"),
                currency: "USD".to_owned(),
                date: Some(date("2024-01-05")),
                label: Some("lot1".to_owned()),
            }),
        );
        assert_eq!(
            call("str", vec![Value::Position(lot.clone())]),
            Value::from("10 AAPL {100 USD, 2024-01-05, \"lot1\"}")
        );
        assert_eq!(call("str", vec![Value::Position(position("5", "XYZ", None))]), Value::from("5 XYZ"));
        // common currencies first, then by currency length, then number
        let inventory = inventory(vec![lot, position("5", "XYZ", Some(("2", "EUR"))), position("-1011.00", "USD", None)]);
        assert_eq!(
            call("str", vec![Value::Inventory(inventory)]),
            Value::from("(-1011.00 USD, 5 XYZ {2 EUR, 2024-01-05}, 10 AAPL {100 USD, 2024-01-05, \"lot1\"})")
        );
        assert_eq!(call("str", vec![Value::Inventory(Inventory::new())]), Value::from("()"));
    }

    #[test]
    fn length_of_string_and_set() {
        assert_eq!(call("length", vec!["Assets".into()]), Value::Int(6));
        assert_eq!(call("length", vec!["".into()]), Value::Int(0));
        assert_eq!(call("length", vec!["资产:银行".into()]), Value::Int(5));
        let tags = ["a", "b"].iter().map(|it| it.to_string()).collect::<BTreeSet<_>>();
        assert_eq!(call("length", vec![Value::Set(tags)]), Value::Int(2));
        assert_eq!(call("length", vec![Value::Set(BTreeSet::new())]), Value::Int(0));
    }

    /// Expected values from Python's `textwrap.shorten`, which beanquery's `maxwidth` calls.
    #[test]
    fn maxwidth_shortens_like_textwrap() {
        for (text, width, expected) in [
            ("Paying the  rent", 12, "Paying [...]"),
            ("  Eating out ", 48, "Eating out"),
            ("tab\tand\nnewline\u{a0}nbsp", 80, "tab and newline nbsp"),
            ("Investing 40% of cash in VBMPX", 20, "Investing 40% [...]"),
            // hyphenated words break after a hyphen between letters, but not in `e-mail`
            ("well-known fact about everything", 20, "well-known [...]"),
            ("abc-def-ghi jkl", 12, "abc- [...]"),
            ("e-mail address here", 12, "e-mail [...]"),
            ("word1 word2-more", 14, "word1 [...]"),
            // an em-dash written `--` is a chunk of its own
            ("x--y zz", 7, "x--y zz"),
            ("x--y zz", 6, "[...]"),
            // nothing fits next to the placeholder
            ("aaaaaaaaaaaaaaaaaaaa bb", 10, "[...]"),
            ("toolong", 5, "[...]"),
            ("short", 5, "short"),
            ("", 5, ""),
        ] {
            assert_eq!(
                call("maxwidth", vec![text.into(), Value::Int(width)]),
                Value::from(expected),
                "{:?} {}",
                text,
                width
            );
        }
        assert_eq!(call("maxwidth", vec![Value::Null, Value::Int(10)]), Value::Null);
        // as in Python, the width must fit the placeholder
        let err = try_call_with(&crate::functions::TestContext::default(), "maxwidth", vec!["text".into(), Value::Int(4)]).unwrap_err();
        assert!(err.contains("at least 5"), "{}", err);
        assert!(try_call_with(&crate::functions::TestContext::default(), "maxwidth", vec!["text".into(), Value::Int(-1)]).is_err());
    }
}
