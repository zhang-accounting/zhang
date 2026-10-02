//! Checks for request values the server writes into the ledger unquoted.
//!
//! Account names, commodities, tags, links and flags are written without quotes,
//! so a value the ledger parser would not read back, such as the account
//! `Assets:My Bank` or the tag `two words`, would make the whole ledger fail to
//! load. A metadata key that is not a bare word is quoted by the exporter, which
//! zhang reads back but beancount does not support, so such keys are rejected for
//! beancount ledgers only. These checks reject such values with a 400 before anything is
//! written. They run the parser's own grammar (see
//! `zhang_core::data_type::text::parser`), which the beancount parser shares.

use std::str::FromStr;

use zhang_ast::amount::Amount;
use zhang_ast::Account;
use zhang_core::data_type::text::parser::{is_valid_account_name, is_valid_commodity_name, is_valid_meta_key, is_valid_tag_or_link, is_valid_transaction_flag};

use crate::error::ServerError;
use crate::ServerResult;

const BARE_WORD: &str = "it cannot be empty or contain a space, tab, line break, `\"`, `:`, `(`, `)` or `,`";

fn invalid(what: &str, value: &str, rule: &str) -> ServerError {
    ServerError::InvalidInput(format!("invalid {what} {value:?}: {rule}"))
}

/// Parse an account name the ledger reads back unchanged.
pub fn account(name: &str) -> ServerResult<Account> {
    if !is_valid_account_name(name) {
        return Err(invalid(
            "account",
            name,
            "it is `Assets`, `Liabilities`, `Equity`, `Income` or `Expenses` followed by `:`-separated components, and a component cannot be empty or contain a space, tab, line break, `\"`, `:`, `(`, `)` or `,`",
        ));
    }
    Ok(Account::from_str(name)?)
}

/// Check that the commodity of `amount` reads back unchanged.
pub fn amount(amount: &Amount) -> ServerResult<()> {
    if is_valid_commodity_name(&amount.commodity) {
        Ok(())
    } else {
        Err(invalid(
            "commodity",
            &amount.commodity,
            "it starts with an ASCII letter and contains only ASCII letters, digits, `.`, `_`, `-` and `'`",
        ))
    }
}

pub fn tag(name: &str) -> ServerResult<()> {
    if is_valid_tag_or_link(name) {
        Ok(())
    } else {
        Err(invalid("tag", name, BARE_WORD))
    }
}

pub fn link(name: &str) -> ServerResult<()> {
    if is_valid_tag_or_link(name) {
        Ok(())
    } else {
        Err(invalid("link", name, BARE_WORD))
    }
}

/// Check a metadata key. In a zhang ledger every key reads back, quoted when it is
/// not a bare word. Beancount has no quoted keys (beancount 3.2.3 rejects the whole
/// transaction), so in a beancount ledger, which Fava may read too, a key that would
/// need quotes is rejected instead of losing the transaction there.
pub fn meta_key(key: &str, beancount: bool) -> ServerResult<()> {
    if !beancount || is_valid_meta_key(key) {
        Ok(())
    } else {
        Err(invalid(
            "metadata key",
            key,
            "beancount does not support it: in a beancount ledger a key cannot be empty, contain a space, tab, line break, `\"`, `:`, `(`, `)` or `,`, or start with `;`, `#`, `*` or `//`",
        ))
    }
}

pub fn flag(flag: &str) -> ServerResult<()> {
    if is_valid_transaction_flag(flag) {
        Ok(())
    } else {
        Err(invalid("flag", flag, "it is `*`, `!`, `#` or an uppercase ASCII letter"))
    }
}

#[cfg(test)]
mod test {
    use axum::http::StatusCode;
    use axum::response::IntoResponse;

    use super::{account, flag, meta_key, tag};

    #[tokio::test]
    async fn invalid_values_are_bad_requests_naming_the_value() {
        let response = tag("two words").unwrap_err().into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let message = body["message"].as_str().unwrap();
        assert!(message.starts_with("invalid tag \"two words\": "), "{message}");
    }

    #[test]
    fn values_the_parser_reads_back_are_accepted() {
        assert!(account("Assets:Bank:中文").is_ok());
        assert!(tag("trip-2024").is_ok());
        assert!(flag("!").is_ok());
        assert!(meta_key("receipt no", false).is_ok());
        assert!(meta_key("receipt-no", true).is_ok());
        assert!(meta_key("receipt no", true).is_err());
        assert!(account("Assets:My Bank").is_err());
        assert!(tag("two words").is_err());
        assert!(flag("a").is_err());
    }
}
