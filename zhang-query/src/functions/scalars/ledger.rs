//! beanquery 0.2.0's functions over the ledger's `open`, `close` and `commodity` directives:
//! `open_date`, `close_date`, `open_meta` and `commodity_meta` (also `currency_meta`), and zhang's
//! `open_meta_values`, its alias `account_budgets`, `open_meta_has`, and `account_status`.
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
        Some(time) => time_of_day(time.as_str().ok_or_else(|| format!("{}() expects a time", function))?, function)?,
    };
    Ok(date.and_time(time))
}

/// The time of day `text` writes as `HH:MM:SS` or `HH:MM`, as the `time` column holds it, for the time argument of
/// `function`; the message of its failure otherwise. The two forms the `time` column and queries write, with two digits
/// each, are read by hand, since a query may read a time on every row; any other text is chrono's to read
/// (`%H:%M:%S`, then `%H:%M`), so that every text reads as it always has.
pub(crate) fn time_of_day(text: &str, function: &str) -> Result<NaiveTime, String> {
    let two_digits = |at: usize| match text.as_bytes()[at..at + 2] {
        [tens @ b'0'..=b'9', ones @ b'0'..=b'9'] => Some(u32::from(tens - b'0') * 10 + u32::from(ones - b'0')),
        _ => None,
    };
    let written = match text.as_bytes() {
        [_, _, b':', _, _, b':', _, _] => two_digits(0).zip(two_digits(3)).zip(two_digits(6)),
        [_, _, b':', _, _] => two_digits(0).zip(two_digits(3)).map(|it| (it, 0)),
        _ => None,
    };
    if let Some(time) = written.and_then(|((hour, minute), second)| NaiveTime::from_hms_opt(hour, minute, second)) {
        return Ok(time);
    }
    NaiveTime::parse_from_str(text, "%H:%M:%S")
        .or_else(|_| NaiveTime::parse_from_str(text, "%H:%M"))
        .map_err(|_| format!("{}() expects a time as HH:MM:SS or HH:MM, not '{}'", function, text))
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

/// `open_meta_has(account, key, value, date)` and `open_meta_has(account, key, value, date, time)` (a zhang extension):
/// whether `value` is among the values of the metadata `key` of the account's `open` in effect then, which is
/// `value IN open_meta_values(account, key, date[, time])` without the set. A query prepares the call so that a row
/// reads its arguments in place ([`crate::compiler::CExpr::OpenMetaHas`]); this is the call as written.
pub(super) fn open_meta_has(args: &[Value], data: &Dataset<'_>) -> Result<Value, String> {
    let account = name_arg(args, "open_meta_has")?;
    let key = args[1].as_str().ok_or_else(|| "open_meta_has() expects a metadata key".to_owned())?;
    let value = args[2].as_str().ok_or_else(|| "open_meta_has() expects a metadata value".to_owned())?;
    let at = datetime_args(args, 3, "open_meta_has")?;
    Ok(Value::Bool(open_meta_has_at(data, account, key, value, at)))
}

/// Whether `value` is among the values of the metadata `key` of the `open` of `account` in effect at `at`: what
/// `open_meta_has()` answers, read from the ledger's account lifecycle without copying the values.
pub(crate) fn open_meta_has_at(data: &Dataset<'_>, account: &str, key: &str, value: &str, at: NaiveDateTime) -> bool {
    data.open_meta_values(account, key, at).is_some_and(|values| values.contains(value))
}

/// `account_budgets(account, date)`, a legacy name kept for the budget system (RFC #698 §9.3): the values of the
/// metadata `budget` of the account's `open` in effect at the start of the date, as a posting of that date without a
/// time sees them. An alias of `open_meta_values(account, 'budget', date)`; which of them a posting counts toward is
/// the plugin `zhang:budget`'s to say.
pub(super) fn account_budgets(args: &[Value], data: &Dataset<'_>) -> Result<Value, String> {
    let account = name_arg(args, "account_budgets")?;
    let at = datetime_args(args, 1, "account_budgets")?;
    Ok(values_set(data.open_meta_values(account, "budget", at)))
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

    /// `open_meta_has()` is FALSE where `open_meta_values()` is empty, NULL with a NULL argument, and fails on a time
    /// it does not read as `open_meta_values()` does.
    #[test]
    fn open_meta_has_is_false_without_an_open_in_effect_or_the_key() {
        let date = |it: &str| Value::Date(it.parse().unwrap());
        let has = |args: Vec<Value>| call("open_meta_has", args);
        assert_eq!(
            has(vec!["Assets:Row".into(), "budget".into(), "x".into(), date("2024-01-05")]),
            Value::Bool(false)
        );
        assert_eq!(
            has(vec!["Nowhere".into(), "budget".into(), "x".into(), date("2024-01-05"), "10:00".into()]),
            Value::Bool(false)
        );
        assert_eq!(has(vec!["Assets:Row".into(), "budget".into(), Value::Null, date("2024-01-05")]), Value::Null);
        assert_eq!(
            has(vec!["Assets:Row".into(), "budget".into(), "x".into(), date("2024-01-05"), Value::Null]),
            Value::Null
        );
        let err = try_call_with(
            &TestContext::default(),
            "open_meta_has",
            vec!["Assets:Row".into(), "budget".into(), "x".into(), date("2024-01-05"), "24:00".into()],
        )
        .unwrap_err();
        assert_eq!(err, "open_meta_has() expects a time as HH:MM:SS or HH:MM, not '24:00'");
    }

    /// The two forms read by hand are read as chrono reads them, and every other text is chrono's.
    #[test]
    fn a_time_of_day_reads_as_chrono_reads_it() {
        let chrono = |text: &str| {
            NaiveTime::parse_from_str(text, "%H:%M:%S")
                .or_else(|_| NaiveTime::parse_from_str(text, "%H:%M"))
                .ok()
        };
        let texts = "00:00:00|00:00|23:59:59|23:59|12:30|07:05:09|9:05|09:5|9:05:01| 09:05|09:05 |09: 05|24:00|23:60|23:59:60|\
                     12:30:61|1a:00|12:3x|12:30:|12:30:0|12:30:00:00|12-30||noon|١٢:٣٠";
        for text in texts.split('|') {
            assert_eq!(time_of_day(text, "f").ok(), chrono(text), "{text:?}");
        }
        for hour in 0..24 {
            for minute in [0, 7, 59] {
                for text in [format!("{hour:02}:{minute:02}"), format!("{hour:02}:{minute:02}:{:02}", minute % 60)] {
                    assert_eq!(time_of_day(&text, "f").ok(), chrono(&text), "{text:?}");
                }
            }
        }
    }
}
