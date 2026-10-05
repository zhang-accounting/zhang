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
//! transaction (or directive) over a name it rejects, so for a beancount ledger a
//! NEW name is also checked against what beancount 3.2.3 accepts. A name the
//! ledger already has (an opened account, a commodity that is defined or used, a
//! tag, link or metadata key used anywhere) passes with zhang's rules only: writing
//! it again cannot make the file worse for beancount, and refusing it would stop a
//! zhang-only user from posting to their own `Assets:银行`. A metadata key that is
//! not a bare word is quoted by the exporter, which zhang reads back but beancount
//! does not support at all, so in a zhang ledger every key is fine.

use std::cell::OnceCell;
use std::collections::HashSet;
use std::str::FromStr;
use std::sync::RwLock;

use zhang_ast::amount::Amount;
use zhang_ast::{Account, PostingCost, SingleTotalPrice};
use zhang_core::data_type::text::parser::{
    is_valid_account_name, is_valid_commodity_name, is_valid_tag_or_link, is_valid_transaction_flag, read_posting_cost, read_posting_price,
};
use zhang_core::data_type::Dialect;
use zhang_core::ledger::Ledger;
use zhang_core::store::Store;

use crate::error::ServerError;
use crate::ServerResult;

/// The rules for the names written to a ledger, which depend on its format.
pub enum Rules<'a> {
    /// a zhang ledger: a name only has to read back
    Zhang,
    /// a beancount ledger: a new name must also be one beancount accepts
    Beancount(KnownNames<'a>),
}

impl<'a> Rules<'a> {
    /// The rules for `ledger`, by its format ([`Ledger::dialect`]).
    pub fn of(ledger: &'a Ledger) -> Rules<'a> {
        match ledger.dialect {
            Dialect::Beancount => Rules::Beancount(KnownNames::of(&ledger.store)),
            Dialect::Zhang => Rules::Zhang,
        }
    }

    /// Whether `name` must pass beancount's rules: it is written to a beancount
    /// ledger, and `known` does not say the ledger has it already.
    fn beancount_checks(&self, known: impl FnOnce(&Names) -> &HashSet<String>, name: &str) -> bool {
        match self {
            Rules::Zhang => false,
            Rules::Beancount(names) => !known(names.get()).contains(name),
        }
    }
}

/// The names a ledger's store already has, collected in one pass over the store
/// the first time a name fails beancount's rules, so a request with only names
/// beancount accepts never scans the store.
pub struct KnownNames<'a> {
    store: &'a RwLock<Store>,
    names: OnceCell<Names>,
}

impl<'a> KnownNames<'a> {
    pub fn of(store: &'a RwLock<Store>) -> Self {
        KnownNames { store, names: OnceCell::new() }
    }

    fn get(&self) -> &Names {
        self.names.get_or_init(|| Names::of(&self.store.read().expect("poison lock detect")))
    }
}

#[derive(Default)]
struct Names {
    accounts: HashSet<String>,
    commodities: HashSet<String>,
    tags: HashSet<String>,
    links: HashSet<String>,
    meta_keys: HashSet<String>,
}

impl Names {
    fn of(store: &Store) -> Names {
        let mut names = Names::default();
        names.accounts.extend(store.accounts.keys().cloned());
        names.commodities.extend(store.commodities.keys().cloned());
        for posting in &store.postings {
            let amounts = [posting.unit.as_ref(), posting.cost.as_ref(), Some(&posting.inferred_amount)];
            names.commodities.extend(amounts.into_iter().flatten().map(|amount| amount.commodity.clone()));
            names.meta_keys.extend(posting.metas.iter().map(|meta| meta.key.clone()));
        }
        for price in &store.prices {
            names.commodities.extend([price.commodity.clone(), price.target_commodity.clone()]);
        }
        names
            .commodities
            .extend(store.balance_assertions.iter().map(|assertion| assertion.amount.commodity.clone()));
        for transaction in store.transactions.values() {
            names.tags.extend(transaction.tags.iter().cloned());
            names.links.extend(transaction.links.iter().cloned());
        }
        names.meta_keys.extend(store.metas.iter().map(|meta| meta.key.clone()));
        names
    }
}

const BARE_WORD: &str = "it cannot be empty or contain a space, tab, line break, `\"`, `:`, `(`, `)` or `,`";

fn invalid(what: &str, value: &str, rule: &str) -> ServerError {
    ServerError::InvalidInput(format!("invalid {what} {value:?}: {rule}"))
}

/// Parse an account name the ledger reads back unchanged.
pub fn account(name: &str, rules: &Rules) -> ServerResult<Account> {
    if !is_valid_account_name(name) {
        return Err(invalid(
            "account",
            name,
            "it is `Assets`, `Liabilities`, `Equity`, `Income` or `Expenses` followed by `:`-separated components, and a component cannot be empty or contain a space, tab, line break, `\"`, `:`, `(`, `)` or `,`",
        ));
    }
    if !is_valid_beancount_account(name) && rules.beancount_checks(|names| &names.accounts, name) {
        return Err(invalid(
            "account",
            name,
            "beancount account components contain only letters, digits, `-` and non-ASCII characters, start with an uppercase letter `A`-`Z`, a digit or a non-ASCII character, and the first component must start with an uppercase letter or a digit",
        ));
    }
    Ok(Account::from_str(name)?)
}

/// Check that the commodity of `amount` reads back unchanged.
pub fn amount(amount: &Amount, rules: &Rules) -> ServerResult<()> {
    let commodity = &amount.commodity;
    if !is_valid_commodity_name(commodity) {
        return Err(invalid(
            "commodity",
            commodity,
            "it starts with an ASCII letter and contains only ASCII letters, digits, `.`, `_`, `-` and `'`",
        ));
    }
    if !is_valid_beancount_commodity(commodity) && rules.beancount_checks(|names| &names.commodities, commodity) {
        return Err(invalid(
            "commodity",
            commodity,
            "beancount commodities start with an uppercase letter `A`-`Z`, end with an uppercase letter or a digit, and contain only `A`-`Z`, digits, `'`, `.`, `_` and `-`",
        ));
    }
    Ok(())
}

/// Parse the cost of a posting given as text in the ledger's own syntax, which the ledger parser reads
/// (`read_posting_cost`): `{150 USD}`, `{{1500 USD}}`, `{}` or `{150 USD, 2024-01-15, "lot"}`, with
/// its commodity checked like a unit's. Spaces around it are ignored. Anything else is a 400.
pub fn cost(text: &str, rules: &Rules) -> ServerResult<PostingCost> {
    let cost = read_posting_cost(text.trim()).ok_or_else(|| {
        invalid(
            "cost",
            text,
            "it is written as in the ledger: `{150 USD}` per unit, `{{1500 USD}}` in total, `{}` for whatever lot there is, or `{150 USD, 2024-01-15, \"lot\"}` with the acquisition date and the label of the lot",
        )
    })?;
    if let Some(base) = &cost.base {
        amount(base, rules)?;
    }
    Ok(cost)
}

/// Parse the price of a posting given as text in the ledger's own syntax (`read_posting_price`):
/// `@ 6 USD` per unit or `@@ 60 USD` in total, with its commodity checked like a unit's. Spaces
/// around it are ignored. Anything else is a 400.
pub fn price(text: &str, rules: &Rules) -> ServerResult<SingleTotalPrice> {
    let price =
        read_posting_price(text.trim()).ok_or_else(|| invalid("price", text, "it is written as in the ledger: `@ 6 USD` per unit or `@@ 60 USD` in total"))?;
    let (SingleTotalPrice::Single(per_unit) | SingleTotalPrice::Total(per_unit)) = &price;
    amount(per_unit, rules)?;
    Ok(price)
}

pub fn tag(name: &str, rules: &Rules) -> ServerResult<()> {
    tag_or_link("tag", name, rules.beancount_checks(|names| &names.tags, name))
}

pub fn link(name: &str, rules: &Rules) -> ServerResult<()> {
    tag_or_link("link", name, rules.beancount_checks(|names| &names.links, name))
}

fn tag_or_link(what: &str, name: &str, beancount_checks: bool) -> ServerResult<()> {
    if !is_valid_tag_or_link(name) {
        return Err(invalid(what, name, BARE_WORD));
    }
    if beancount_checks && !is_valid_beancount_tag_or_link(name) {
        return Err(invalid(
            what,
            name,
            &format!("beancount {what}s contain only ASCII letters, digits, `-`, `_`, `/` and `.`"),
        ));
    }
    Ok(())
}

/// Check a metadata key. In a zhang ledger every key reads back, quoted when it is
/// not a bare word. Beancount has no quoted keys, so in a beancount ledger a new
/// key must be one beancount accepts as it is.
pub fn meta_key(key: &str, rules: &Rules) -> ServerResult<()> {
    if !is_valid_beancount_meta_key(key) && rules.beancount_checks(|names| &names.meta_keys, key) {
        return Err(invalid(
            "metadata key",
            key,
            "beancount metadata keys start with a lowercase letter `a`-`z`, are at least two characters long and contain only ASCII letters, digits, `-` and `_`",
        ));
    }
    Ok(())
}

pub fn flag(flag: &str) -> ServerResult<()> {
    // every flag zhang reads (`*`, `!`, `#`, `&`, `?`, `%`, `A`-`Z`) is a beancount flag too
    if is_valid_transaction_flag(flag) {
        Ok(())
    } else {
        Err(invalid("flag", flag, "it is `*`, `!`, `#`, `&`, `?`, `%` or an uppercase ASCII letter"))
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

    /// Beancount rules for a ledger with an empty store, so every name is new.
    fn with_empty_store(check: impl FnOnce(&Rules)) {
        let store = RwLock::new(Store::default());
        check(&Rules::Beancount(KnownNames::of(&store)));
    }

    #[tokio::test]
    async fn invalid_values_are_bad_requests_naming_the_value() {
        let response = tag("two words", &Rules::Zhang).unwrap_err().into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let message = body["message"].as_str().unwrap();
        assert!(message.starts_with("invalid tag \"two words\": "), "{message}");
    }

    #[test]
    fn values_the_parser_reads_back_are_accepted() {
        with_empty_store(|strict| {
            assert!(account("Assets:Bank:中文", &Rules::Zhang).is_ok());
            assert!(tag("trip-2024", &Rules::Zhang).is_ok());
            assert!(flag("!").is_ok());
            assert!(meta_key("receipt no", &Rules::Zhang).is_ok());
            assert!(account("Assets:My Bank", &Rules::Zhang).is_err());
            assert!(account("Assets:My Bank", strict).is_err());
            assert!(tag("two words", &Rules::Zhang).is_err());
            assert!(flag("a").is_err());
        });
    }

    // The expectations below were checked against beancount 3.2.3: each name was
    // loaded with `beancount.loader.load_string` in a minimal ledger, and `true`
    // means it loaded without errors and read back unchanged. zhang's parsers accept
    // every name here.

    #[test]
    fn beancount_meta_keys() {
        with_empty_store(|strict| {
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
                assert!(meta_key(key, &Rules::Zhang).is_ok(), "{key:?}");
                assert_eq!(meta_key(key, strict).is_ok(), beancount, "{key:?}");
            }
        });
    }

    #[test]
    fn beancount_tags_and_links() {
        with_empty_store(|strict| {
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
                assert!(tag(name, &Rules::Zhang).is_ok() && link(name, &Rules::Zhang).is_ok(), "{name:?}");
                assert_eq!(tag(name, strict).is_ok(), beancount, "{name:?}");
                assert_eq!(link(name, strict).is_ok(), beancount, "{name:?}");
            }
        });
    }

    #[test]
    fn beancount_accounts() {
        with_empty_store(|strict| {
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
                assert!(account(name, &Rules::Zhang).is_ok(), "{name:?}");
                assert_eq!(account(name, strict).is_ok(), beancount, "{name:?}");
            }
            // beancount 3.2.3 accepts a non-ASCII decimal digit here too; it is refused
            // to keep to the standard library
            assert!(!is_valid_beancount_account("Assets:١bc"));
        });
    }

    #[test]
    fn beancount_commodities() {
        with_empty_store(|strict| {
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
                assert!(amount(&commodity, &Rules::Zhang).is_ok(), "{name:?}");
                assert_eq!(amount(&commodity, strict).is_ok(), beancount, "{name:?}");
            }
        });
    }

    #[test]
    fn names_the_ledger_already_has_pass_with_zhang_rules() {
        let mut store = Store::default();
        store.metas.push(zhang_core::domains::schemas::MetaDomain {
            meta_type: "TransactionMeta".to_owned(),
            type_identifier: "id".to_owned(),
            key: "Receipt".to_owned(),
            value: "1".to_owned(),
        });
        let store = RwLock::new(store);
        let rules = Rules::Beancount(KnownNames::of(&store));
        assert!(meta_key("Receipt", &rules).is_ok(), "an existing key passes");
        assert!(meta_key("Other", &rules).is_err(), "a new key must be one beancount accepts");
        assert!(tag("two words", &rules).is_err(), "zhang's rules still apply");
    }

    #[test]
    fn a_commodity_only_a_balance_assertion_uses_is_known() {
        use std::str::FromStr;

        use bigdecimal::BigDecimal;
        use chrono::TimeZone;
        use zhang_core::store::BalanceAssertionDomain;

        let mut store = Store::default();
        let usd = Amount::new(BigDecimal::from(1), "Usd");
        store.balance_assertions.push(BalanceAssertionDomain {
            id: uuid::Uuid::nil(),
            sequence: 1,
            directive: 0,
            datetime: chrono_tz::Tz::UTC.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            account: Account::from_str("Assets:Cash").unwrap(),
            amount: usd.clone(),
            tolerance: None,
            balance: usd.clone(),
            passed: true,
            span: Default::default(),
        });
        let store = RwLock::new(store);
        let rules = Rules::Beancount(KnownNames::of(&store));
        assert!(amount(&usd, &rules).is_ok(), "a commodity of an assertion is one the ledger has");
        assert!(
            amount(&Amount::new(BigDecimal::from(1), "Eur"), &rules).is_err(),
            "a new one must be one beancount accepts"
        );
    }
}
