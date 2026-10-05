//! beanquery 0.2.0's functions over the ledger's `open`, `close` and `commodity` directives:
//! `open_date`, `close_date`, `open_meta` and `commodity_meta` (also `currency_meta`), and zhang's
//! `account_budgets` and `account_status`.
//!
//! As in beancount, an account's directives are its earliest `open` and earliest `close`, and
//! a currency's is its last `commodity` directive. An unknown account or currency is NULL.
//! The metadata is the directive's own (beancount adds `filename` and `lineno`, which zhang
//! does not keep); values are text.

use chrono::{NaiveDate, NaiveTime};
use zhang_core::domains::schemas::AccountStatus;

use crate::functions::FunctionContext;
use crate::table::meta_pairs;
use crate::value::Value;

fn name_arg<'a>(args: &'a [Value], function: &str) -> Result<&'a str, String> {
    args[0].as_str().ok_or_else(|| format!("{}() expects a name", function))
}

fn date_value(date: Option<NaiveDate>) -> Value {
    date.map_or(Value::Null, Value::Date)
}

/// `open_date(account)`: the date of the account's `open` directive.
pub(super) fn open_date(args: &[Value], ctx: &dyn FunctionContext) -> Result<Value, String> {
    let account = name_arg(args, "open_date")?;
    Ok(date_value(
        ctx.account_directives(account).and_then(|it| it.open).map(|open| open.date.naive_date()),
    ))
}

/// `close_date(account)`: the date of the account's `close` directive.
pub(super) fn close_date(args: &[Value], ctx: &dyn FunctionContext) -> Result<Value, String> {
    let account = name_arg(args, "close_date")?;
    Ok(date_value(
        ctx.account_directives(account).and_then(|it| it.close).map(|close| close.date.naive_date()),
    ))
}

/// `open_meta(account)`: the metadata of the account's `open` directive, and
/// `open_meta(account, key)`: one value of it.
pub(super) fn open_meta(args: &[Value], ctx: &dyn FunctionContext) -> Result<Value, String> {
    let account = name_arg(args, "open_meta")?;
    let meta = ctx.account_directives(account).and_then(|it| it.open).map(|open| &open.meta);
    meta_result(meta, args.get(1), "open_meta")
}

/// `account_budgets(account, date)`: the budgets the account counts in at the date (a zhang
/// extension): those of its latest `open` on or before the date.
pub(super) fn account_budgets(args: &[Value], ctx: &dyn FunctionContext) -> Result<Value, String> {
    let account = name_arg(args, "account_budgets")?;
    let Value::Date(date) = args[1] else {
        return Err("account_budgets() expects a date".to_owned());
    };
    Ok(Value::Set(ctx.account_budgets(account, date).unwrap_or_default()))
}

/// `account_status(account, date)` and `account_status(account, date, time)`: whether the account is `'open'` or
/// `'closed'` at the start of the date, or at its time of day (a zhang extension), by the ledger's account lifecycle,
/// the rule every directive is checked with; NULL when neither an `open` nor a `close` of it is in effect then.
pub(super) fn account_status(args: &[Value], ctx: &dyn FunctionContext) -> Result<Value, String> {
    let account = name_arg(args, "account_status")?;
    let Value::Date(date) = args[1] else {
        return Err("account_status() expects a date".to_owned());
    };
    let time = match args.get(2) {
        None => NaiveTime::MIN,
        Some(time) => {
            let text = time.as_str().ok_or_else(|| "account_status() expects a time".to_owned())?;
            NaiveTime::parse_from_str(text, "%H:%M:%S")
                .or_else(|_| NaiveTime::parse_from_str(text, "%H:%M"))
                .map_err(|_| format!("account_status() expects a time as HH:MM:SS or HH:MM, not '{}'", text))?
        }
    };
    Ok(match ctx.account_status(account, date.and_time(time)) {
        Some(AccountStatus::Open) => Value::Str("open".to_owned()),
        Some(AccountStatus::Close) => Value::Str("closed".to_owned()),
        None => Value::Null,
    })
}

/// `commodity_meta(currency)`: the metadata of the currency's `commodity` directive, and
/// `commodity_meta(currency, key)`: one value of it.
pub(super) fn commodity_meta(args: &[Value], ctx: &dyn FunctionContext) -> Result<Value, String> {
    let currency = name_arg(args, "commodity_meta")?;
    let meta = ctx.commodity_directive(currency).map(|commodity| &commodity.meta);
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
}
