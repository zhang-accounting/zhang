//! Checks for request values the server writes into the ledger unquoted.
//!
//! Account names, commodities, tags, links and flags are written without quotes,
//! so a value the ledger parser would not read back, such as the account
//! `Assets:My Bank` or the tag `two words`, would make the whole ledger fail to
//! load. These checks reject such values with a 400 before anything is written.
//! They run the parser's own grammar (see `zhang_core::data_type::text::parser`),
//! which the beancount parser shares.
//!
//! A beancount ledger is often shared with Fava, which reads it with beancount
//! itself. Beancount accepts fewer names than zhang's parsers, and drops a whole
//! transaction (or directive) over a name it rejects, so for a beancount ledger the
//! names are also checked against what beancount 3.2.3 accepts. A metadata key
//! that is not a bare word is quoted by the exporter, which zhang reads back but
//! beancount does not support at all, so in a zhang ledger every key is fine.

use std::str::FromStr;

use zhang_ast::amount::Amount;
use zhang_ast::Account;
use zhang_core::data_type::is_beancount_endpoint;
use zhang_core::data_type::text::parser::{is_valid_account_name, is_valid_commodity_name, is_valid_tag_or_link, is_valid_transaction_flag};
use zhang_core::ledger::Ledger;

use crate::error::ServerError;
use crate::ServerResult;

/// The format of the ledger a name is written to, which decides the rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Zhang,
    Beancount,
}

impl Format {
    /// The format of `ledger`, from its main file's extension, as when loading it.
    pub fn of(ledger: &Ledger) -> Format {
        if is_beancount_endpoint(&ledger.entry.1) {
            Format::Beancount
        } else {
            Format::Zhang
        }
    }
}

const BARE_WORD: &str = "it cannot be empty or contain a space, tab, line break, `\"`, `:`, `(`, `)` or `,`";

fn invalid(what: &str, value: &str, rule: &str) -> ServerError {
    ServerError::InvalidInput(format!("invalid {what} {value:?}: {rule}"))
}

/// Parse an account name the ledger reads back unchanged.
pub fn account(name: &str, format: Format) -> ServerResult<Account> {
    if !is_valid_account_name(name) {
        return Err(invalid(
            "account",
            name,
            "it is `Assets`, `Liabilities`, `Equity`, `Income` or `Expenses` followed by `:`-separated components, and a component cannot be empty or contain a space, tab, line break, `\"`, `:`, `(`, `)` or `,`",
        ));
    }
    if format == Format::Beancount && !is_valid_beancount_account(name) {
        return Err(invalid(
            "account",
            name,
            "beancount account components contain only letters, digits, `-` and non-ASCII characters, start with an uppercase letter `A`-`Z`, a digit or a non-ASCII character, and the first component must start with an uppercase letter or a digit",
        ));
    }
    Ok(Account::from_str(name)?)
}

/// Check that the commodity of `amount` reads back unchanged.
pub fn amount(amount: &Amount, format: Format) -> ServerResult<()> {
    let commodity = &amount.commodity;
    if !is_valid_commodity_name(commodity) {
        return Err(invalid(
            "commodity",
            commodity,
            "it starts with an ASCII letter and contains only ASCII letters, digits, `.`, `_`, `-` and `'`",
        ));
    }
    if format == Format::Beancount && !is_valid_beancount_commodity(commodity) {
        return Err(invalid(
            "commodity",
            commodity,
            "beancount commodities start with an uppercase letter `A`-`Z`, end with an uppercase letter or a digit, and contain only `A`-`Z`, digits, `'`, `.`, `_` and `-`",
        ));
    }
    Ok(())
}

pub fn tag(name: &str, format: Format) -> ServerResult<()> {
    tag_or_link("tag", name, format)
}

pub fn link(name: &str, format: Format) -> ServerResult<()> {
    tag_or_link("link", name, format)
}

fn tag_or_link(what: &str, name: &str, format: Format) -> ServerResult<()> {
    if !is_valid_tag_or_link(name) {
        return Err(invalid(what, name, BARE_WORD));
    }
    if format == Format::Beancount && !is_valid_beancount_tag_or_link(name) {
        return Err(invalid(
            what,
            name,
            &format!("beancount {what}s contain only ASCII letters, digits, `-`, `_`, `/` and `.`"),
        ));
    }
    Ok(())
}

/// Check a metadata key. In a zhang ledger every key reads back, quoted when it is
/// not a bare word. Beancount has no quoted keys, so in a beancount ledger a key
/// must be one beancount accepts as it is.
pub fn meta_key(key: &str, format: Format) -> ServerResult<()> {
    if format == Format::Beancount && !is_valid_beancount_meta_key(key) {
        return Err(invalid(
            "metadata key",
            key,
            "beancount metadata keys start with a lowercase letter `a`-`z`, are at least two characters long and contain only ASCII letters, digits, `-` and `_`",
        ));
    }
    Ok(())
}

pub fn flag(flag: &str) -> ServerResult<()> {
    // every flag zhang reads (`*`, `!`, `#`, `A`-`Z`) is a beancount flag too
    if is_valid_transaction_flag(flag) {
        Ok(())
    } else {
        Err(invalid("flag", flag, "it is `*`, `!`, `#` or an uppercase ASCII letter"))
    }
}

// ---------------------------------------------------------------------------
// what beancount 3.2.3 accepts
// ---------------------------------------------------------------------------
//
// From beancount's lexer (`beancount/parser/lexer.l`) and its account check
// (`beancount/parser/grammar.py`), and checked against beancount 3.2.3 itself, see
// the tests. Each rule only accepts names zhang's parsers accept too.

/// `KEY = [a-z][a-zA-Z0-9\-_]+`, followed directly by the `:`.
fn is_valid_beancount_meta_key(key: &str) -> bool {
    let mut chars = key.chars();
    matches!(chars.next(), Some('a'..='z')) && !chars.as_str().is_empty() && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
}

/// `TAG = #[A-Za-z0-9\-_/.]+`, and the same for `LINK` after `^`.
fn is_valid_beancount_tag_or_link(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '/' | '.'))
}

/// `CURRENCY = [A-Z][A-Z0-9\'\.\_\-]*[A-Z0-9]`, or a single `[A-Z]`. (The
/// `/`-prefixed futures form is not a name zhang reads.)
fn is_valid_beancount_commodity(name: &str) -> bool {
    match name.as_bytes() {
        [first] => first.is_ascii_uppercase(),
        [first, middle @ .., last] => {
            first.is_ascii_uppercase()
                && middle
                    .iter()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || matches!(c, b'\'' | b'.' | b'_' | b'-'))
                && (last.is_ascii_uppercase() || last.is_ascii_digit())
        }
        [] => false,
    }
}

/// The lexer reads `ACCOUNTTYPE(:ACCOUNTNAME)+`, where a component is
/// `([A-Z0-9]|non-ASCII)([A-Za-z0-9\-]|non-ASCII)*`. The parser then matches only
/// the start of the name against `ACCOUNT_RE`, whose components start with a
/// Unicode uppercase letter or decimal digit, so the first component must also
/// start with one (`Assets:银行` is rejected, `Assets:Bank:银行` is accepted).
fn is_valid_beancount_account(name: &str) -> bool {
    let mut components = name.split(':').skip(1).peekable();
    let first_starts_upper_or_digit = components
        .peek()
        .and_then(|first| first.chars().next())
        .is_some_and(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || is_non_ascii_uppercase_letter(c));
    first_starts_upper_or_digit
        && components.all(|component| {
            let mut chars = component.chars();
            chars.next().is_some_and(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || !c.is_ascii())
                && chars.all(|c| c.is_ascii_alphanumeric() || c == '-' || !c.is_ascii())
        })
}

/// A non-ASCII uppercase letter (category `Lu`): the `Uppercase` property Rust checks,
/// without its `Other_Uppercase` part, such as `Ⓐ` and the roman numeral `Ⅰ`, which
/// beancount rejects. Non-ASCII decimal digits, which beancount also allows here,
/// are refused to keep to the standard library; that only refuses a few names
/// beancount would take.
fn is_non_ascii_uppercase_letter(c: char) -> bool {
    !c.is_ascii()
        && c.is_uppercase()
        && !matches!(
            c,
            '\u{2160}'..='\u{216f}' | '\u{24b6}'..='\u{24cf}' | '\u{1f130}'..='\u{1f149}' | '\u{1f150}'..='\u{1f169}' | '\u{1f170}'..='\u{1f189}'
        )
}

#[cfg(test)]
mod test {
    use axum::http::StatusCode;
    use axum::response::IntoResponse;

    use super::*;

    #[tokio::test]
    async fn invalid_values_are_bad_requests_naming_the_value() {
        let response = tag("two words", Format::Zhang).unwrap_err().into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let message = body["message"].as_str().unwrap();
        assert!(message.starts_with("invalid tag \"two words\": "), "{message}");
    }

    #[test]
    fn values_the_parser_reads_back_are_accepted() {
        assert!(account("Assets:Bank:中文", Format::Zhang).is_ok());
        assert!(tag("trip-2024", Format::Zhang).is_ok());
        assert!(flag("!").is_ok());
        assert!(meta_key("receipt no", Format::Zhang).is_ok());
        assert!(account("Assets:My Bank", Format::Zhang).is_err());
        assert!(account("Assets:My Bank", Format::Beancount).is_err());
        assert!(tag("two words", Format::Zhang).is_err());
        assert!(flag("a").is_err());
    }

    // The expectations below were checked against beancount 3.2.3: each name was
    // loaded with `beancount.loader.load_string` in a minimal ledger, and `true`
    // means it loaded without errors and read back unchanged. zhang's parsers accept
    // every name here.

    #[test]
    fn beancount_meta_keys() {
        let cases = [
            ("receipt", true),
            ("receipt-no", true),
            ("receipt_no", true),
            ("aB", true),
            ("a1", true),
            ("ab", true),
            ("a", false),
            ("Receipt", false),
            ("1a", false),
            ("_a", false),
            ("-a", false),
            ("a.b", false),
            ("a/b", false),
            ("a;b", false),
            ("a#b", false),
            ("é", false),
            ("aé", false),
            ("中文", false),
        ];
        for (key, beancount) in cases {
            assert_eq!(is_valid_beancount_meta_key(key), beancount, "{key:?}");
            assert!(meta_key(key, Format::Zhang).is_ok(), "{key:?}");
            assert_eq!(meta_key(key, Format::Beancount).is_ok(), beancount, "{key:?}");
        }
    }

    #[test]
    fn beancount_tags_and_links() {
        let cases = [
            ("trip", true),
            ("Trip", true),
            ("trip-2024", true),
            ("a_b", true),
            ("a/b", true),
            ("a.b", true),
            ("-", true),
            ("1", true),
            ("INV", true),
            ("旅行", false),
            ("aé", false),
            ("a#b", false),
            ("a^b", false),
            ("a;b", false),
            ("a!", false),
            ("a+b", false),
            ("a@b", false),
        ];
        for (name, beancount) in cases {
            assert_eq!(is_valid_beancount_tag_or_link(name), beancount, "{name:?}");
            assert!(tag(name, Format::Zhang).is_ok() && link(name, Format::Zhang).is_ok(), "{name:?}");
            assert_eq!(tag(name, Format::Beancount).is_ok(), beancount, "{name:?}");
            assert_eq!(link(name, Format::Beancount).is_ok(), beancount, "{name:?}");
        }
    }

    #[test]
    fn beancount_accounts() {
        let cases = [
            ("Assets:Bank", true),
            ("Assets:B", true),
            ("Assets:1Bank", true),
            ("Assets:Bank-1", true),
            ("Assets:Ébank", true),
            ("Assets:Банк", true),
            ("Assets:Bank:银行", true),
            ("Assets:B😀", true),
            ("Expenses:Food:Lunch", true),
            ("Assets:bank", false),
            ("Assets:银行", false),
            ("Assets:éBank", false),
            ("Assets:Bank_1", false),
            ("Assets:Bank.1", false),
            ("Assets:Bank'1", false),
            ("Assets:-Bank", false),
            ("Assets:Bank:x", false),
            ("Assets:Bank:x1", false),
            ("Assets:Bank:-x", false),
            ("Assets:A;B", false),
            ("Assets:A#B", false),
            ("Assets:A/B", false),
            ("Assets:Ⓐbc", false),
            ("Assets:Ⅰbc", false),
        ];
        for (name, beancount) in cases {
            assert_eq!(is_valid_beancount_account(name), beancount, "{name:?}");
            assert!(account(name, Format::Zhang).is_ok(), "{name:?}");
            assert_eq!(account(name, Format::Beancount).is_ok(), beancount, "{name:?}");
        }
        // beancount 3.2.3 accepts a non-ASCII decimal digit here too; it is refused
        // to keep to the standard library
        assert!(!is_valid_beancount_account("Assets:١bc"));
    }

    #[test]
    fn beancount_commodities() {
        let cases = [
            ("USD", true),
            ("V", true),
            ("A1", true),
            ("NT.TO", true),
            ("TLT_040921C144", true),
            ("A'B", true),
            ("A-B", true),
            ("A.B", true),
            ("usd", false),
            ("Usd", false),
            ("v", false),
            ("A_", false),
            ("A-", false),
            ("A'", false),
            ("A.", false),
            ("Ab", false),
        ];
        for (name, beancount) in cases {
            let commodity = Amount::new(1.into(), name);
            assert_eq!(is_valid_beancount_commodity(name), beancount, "{name:?}");
            assert!(amount(&commodity, Format::Zhang).is_ok(), "{name:?}");
            assert_eq!(amount(&commodity, Format::Beancount).is_ok(), beancount, "{name:?}");
        }
    }
}
