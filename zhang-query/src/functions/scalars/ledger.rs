//! beanquery 0.2.0's functions over the ledger's `open`, `close` and `commodity` directives:
//! `open_date`, `close_date`, `open_meta` and `commodity_meta` (also `currency_meta`), and zhang's
//! `open_meta_values`, its alias `account_budgets`, and `account_status`.
//!
//! As in beancount, an account's directives are its earliest `open` and earliest `close`, and
//! a currency's is its last `commodity` directive. An unknown account or currency is NULL.
//! The metadata is the directive's own (beancount adds `filename` and `lineno`, which zhang
//! does not keep); values are text. zhang's functions read the account lifecycle instead: the
//! `open` in effect at a date and time, its latest `open` by then.

use std::collections::BTreeSet;

use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
use zhang_core::domains::schemas::AccountStatus;

use crate::table::{meta_pairs, Dataset};
use crate::value::Value;

fn name_arg<'a>(args: &'a [Value], function: &str) -> Result<&'a str, String> {
    args[0].as_str().ok_or_else(|| format!("{}() expects a name", function))
}

fn date_value(date: Option<NaiveDate>) -> Value {
    date.map_or(Value::Null, Value::Date)
}

/// `open_date(account)`: the date of the account's `open` directive.
pub(super) fn open_date(args: &[Value], data: &Dataset<'_>) -> Result<Value, String> {
    let account = name_arg(args, "open_date")?;
    Ok(date_value(
        data.account_directives(account).and_then(|it| it.open).map(|open| open.date.naive_date()),
    ))
}

/// `close_date(account)`: the date of the account's `close` directive.
pub(super) fn close_date(args: &[Value], data: &Dataset<'_>) -> Result<Value, String> {
    let account = name_arg(args, "close_date")?;
    Ok(date_value(
        data.account_directives(account).and_then(|it| it.close).map(|close| close.date.naive_date()),
    ))
}

/// `open_meta(account)`: the metadata of the account's `open` directive, and
/// `open_meta(account, key)`: one value of it.
pub(super) fn open_meta(args: &[Value], data: &Dataset<'_>) -> Result<Value, String> {
    let account = name_arg(args, "open_meta")?;
    let meta = data.account_directives(account).and_then(|it| it.open).map(|open| &open.meta);
    meta_result(meta, args.get(1), "open_meta")
}

/// The wall-clock time `args[at]` and the optional `args[at + 1]` give: the date, at the time of day written `HH:MM:SS`
/// or `HH:MM` as the `time` column holds it, or at its start.
fn datetime_args(args: &[Value], at: usize, function: &str) -> Result<NaiveDateTime, String> {
    let Value::Date(date) = args[at] else {
        return Err(format!("{}() expects a date", function));
    };
    let time = match args.get(at + 1) {
        None => NaiveTime::MIN,
        Some(time) => {
            let text = time.as_str().ok_or_else(|| format!("{}() expects a time", function))?;
            NaiveTime::parse_from_str(text, "%H:%M:%S")
                .or_else(|_| NaiveTime::parse_from_str(text, "%H:%M"))
                .map_err(|_| format!("{}() expects a time as HH:MM:SS or HH:MM, not '{}'", function, text))?
        }
    };
    Ok(date.and_time(time))
}

/// The values of the metadata of an `open` in effect, as a set: empty when no `open` of the account is in effect.
fn values_set(values: Option<&BTreeSet<String>>) -> Value {
    Value::Set(values.cloned().unwrap_or_default())
}

/// `open_meta_values(account, key, date)` and `open_meta_values(account, key, date, time)`: every value of the metadata
/// `key` of the account's `open` in effect at the start of the date, or at its time of day (a zhang extension), as a set
/// (each value once, sorted). That is
/// its latest `open` at or before then, by the ledger's account lifecycle, so a later `open` replaces the values and a
/// `close` keeps them. Empty before its first `open`, and when that `open` has no `key`.
pub(super) fn open_meta_values(args: &[Value], data: &Dataset<'_>) -> Result<Value, String> {
    let account = name_arg(args, "open_meta_values")?;
    let key = args[1].as_str().ok_or_else(|| "open_meta_values() expects a metadata key".to_owned())?;
    let at = datetime_args(args, 2, "open_meta_values")?;
    Ok(values_set(data.open_meta_values(account, key, at)))
}

/// `account_budgets(account, date)`: the budgets a posting of the account at the start of the date
/// counts in (a zhang extension), as a posting of that date without a time sees them. An alias of
/// `open_meta_values(account, 'budget', date)`: the ledger's one rule of budget membership,
/// [`Ledger::account_budgets`](zhang_core::ledger::Ledger::account_budgets), reads the metadata
/// `budget` of the `open` in effect.
pub(super) fn account_budgets(args: &[Value], data: &Dataset<'_>) -> Result<Value, String> {
    let account = name_arg(args, "account_budgets")?;
    let at = datetime_args(args, 1, "account_budgets")?;
    Ok(values_set(data.account_budgets(account, at)))
}

/// `account_status(account, date)` and `account_status(account, date, time)`: whether the account is `'open'` or
/// `'closed'` at the start of the date, or at its time of day (a zhang extension), by the ledger's account lifecycle,
/// the rule every directive is checked with; NULL when neither an `open` nor a `close` of it is in effect then.
pub(super) fn account_status(args: &[Value], data: &Dataset<'_>) -> Result<Value, String> {
    let account = name_arg(args, "account_status")?;
    let at = datetime_args(args, 1, "account_status")?;
    Ok(match data.account_status(account, at) {
        Some(AccountStatus::Open) => Value::Str("open".to_owned()),
        Some(AccountStatus::Close) => Value::Str("closed".to_owned()),
        None => Value::Null,
    })
}

/// `commodity_meta(currency)`: the metadata of the currency's `commodity` directive, and
/// `commodity_meta(currency, key)`: one value of it.
pub(super) fn commodity_meta(args: &[Value], data: &Dataset<'_>) -> Result<Value, String> {
    let currency = name_arg(args, "commodity_meta")?;
    let meta = data.commodity_directive(currency).map(|commodity| &commodity.meta);
    meta_result(meta, args.get(1), "commodity_meta")
}

/// The whole metadata, or the (first) value of `key`; NULL without the directive.
fn meta_result(meta: Option<&zhang_ast::Meta>, key: Option<&Value>, function: &str) -> Result<Value, String> {
    let Some(meta) = meta else {
        return Ok(Value::Null);
    };
    match key {
        None => Ok(Value::Metas(meta_pairs(Some(meta)))),
        Some(key) => {
            let key = key.as_str().ok_or_else(|| format!("{}() expects a metadata key", function))?;
            Ok(meta.get_one(key).map_or(Value::Null, |value| Value::Str(value.as_str().to_owned())))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::testing::*;
    use super::*;

    #[test]
    fn without_a_ledger_every_lookup_is_null() {
        assert_eq!(call("open_date", vec!["Assets:Bank".into()]), Value::Null);
        assert_eq!(call("close_date", vec!["Assets:Bank".into()]), Value::Null);
        assert_eq!(call("open_meta", vec!["Assets:Bank".into()]), Value::Null);
        assert_eq!(call("open_meta", vec!["Assets:Bank".into(), "x".into()]), Value::Null);
        assert_eq!(call("commodity_meta", vec!["USD".into(), "name".into()]), Value::Null);
        assert_eq!(call("currency_meta", vec!["USD".into()]), Value::Null);
        assert_eq!(call("open_date", vec![Value::Null]), Value::Null);
        let date = Value::Date(NaiveDate::from_ymd_opt(2024, 1, 5).unwrap());
        assert_eq!(call("account_status", vec!["Assets:Bank".into(), date.clone()]), Value::Null);
        assert_eq!(call("account_status", vec!["Assets:Bank".into(), date, "10:00".into()]), Value::Null);
    }

    /// `open_meta_values()` is the set of the values of the `open` in effect, empty without one; `account_budgets()` is
    /// it with the key `budget`. The fixture's accounts are opened on 1970-01-01, without metadata.
    #[test]
    fn open_meta_values_are_empty_without_an_open_in_effect_or_the_key() {
        let empty = Value::Set(Default::default());
        let date = |it: &str| Value::Date(it.parse().unwrap());
        let values = |args: Vec<Value>| call("open_meta_values", args);
        assert_eq!(values(vec!["Assets:Bank".into(), "budget".into(), date("2024-01-05")]), empty);
        assert_eq!(values(vec!["Assets:Row".into(), "budget".into(), date("2024-01-05")]), empty);
        assert_eq!(values(vec!["Assets:Row".into(), "x".into(), date("1969-12-31"), "23:59".into()]), empty);
        assert_eq!(values(vec![Value::Null, "budget".into(), date("2024-01-05")]), Value::Null);
        assert_eq!(values(vec!["Assets:Row".into(), Value::Null, date("2024-01-05")]), Value::Null);
        assert_eq!(values(vec!["Assets:Row".into(), "x".into(), date("2024-01-05"), Value::Null]), Value::Null);
        assert_eq!(call("account_budgets", vec!["Assets:Row".into(), date("2024-01-05")]), empty);
        let err = try_call_with(
            &TestContext::default(),
            "open_meta_values",
            vec!["Assets:Row".into(), "x".into(), date("2024-01-05"), "noon".into()],
        )
        .unwrap_err();
        assert_eq!(err, "open_meta_values() expects a time as HH:MM:SS or HH:MM, not 'noon'");
    }
}
