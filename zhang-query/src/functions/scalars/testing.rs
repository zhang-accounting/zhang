//! Test helpers for the scalar function library, plus registry-wide sanity checks.

use std::collections::HashMap;
use std::str::FromStr;

use bigdecimal::BigDecimal;
use chrono::NaiveDate;

use crate::functions::{resolve_scalar, FunctionContext, TestContext, SCALAR_FUNCTIONS};
use crate::prices::PriceMap;
use crate::value::{Cost, Inventory, Position, Value};
use crate::Amount;

/// Resolve `name` against the static types of `args` and evaluate it the way the evaluator
/// does: widen int → decimal where the overload asks for it, propagate NULL, and check the
/// result against the declared return type.
pub(crate) fn try_call_with(ctx: &dyn FunctionContext, name: &str, args: Vec<Value>) -> Result<Value, String> {
    let types = args.iter().map(Value::data_type).collect::<Vec<_>>();
    let resolved = resolve_scalar(name, &types)?;
    let args = args
        .into_iter()
        .zip(&resolved.widen)
        .map(|(arg, widen)| {
            if *widen {
                Value::Decimal(arg.as_decimal().expect("int argument"))
            } else {
                arg
            }
        })
        .collect::<Vec<_>>();
    if args.iter().any(Value::is_null) {
        return Ok(Value::Null);
    }
    let result = (resolved.function.eval)(&args, ctx)?;
    let declared = resolved.function.returns.resolve(&types);
    assert!(
        result.is_null() || result.data_type() == declared,
        "{} returned {} but declares {}",
        resolved.function.signature(),
        result.data_type(),
        declared
    );
    Ok(result)
}

pub(crate) fn call_with(ctx: &dyn FunctionContext, name: &str, args: Vec<Value>) -> Value {
    try_call_with(ctx, name, args).unwrap()
}

pub(crate) fn call(name: &str, args: Vec<Value>) -> Value {
    call_with(&TestContext::default(), name, args)
}

/// Evaluate and render the result with `str()` semantics, which keeps decimal scales visible.
pub(crate) fn call_str_with(ctx: &dyn FunctionContext, name: &str, args: Vec<Value>) -> String {
    match call_with(ctx, name, args) {
        Value::Inventory(inventory) => super::strings::inventory_to_string(&inventory),
        other => other.to_string(),
    }
}

pub(crate) fn call_str(name: &str, args: Vec<Value>) -> String {
    call_str_with(&TestContext::default(), name, args)
}

pub(crate) fn d(number: &str) -> BigDecimal {
    BigDecimal::from_str(number).unwrap()
}

pub(crate) fn date(date: &str) -> NaiveDate {
    NaiveDate::from_str(date).unwrap()
}

pub(crate) fn amount(number: &str, currency: &str) -> Amount {
    Amount::new(d(number), currency)
}

/// `n currency`, held at `{cost_number cost_currency}` when `cost` is given.
pub(crate) fn position(number: &str, currency: &str, cost: Option<(&str, &str)>) -> Position {
    Position::new(
        amount(number, currency),
        cost.map(|(number, currency)| Cost {
            number: d(number),
            currency: currency.to_owned(),
            date: Some(date("2024-01-05")),
            label: None,
        }),
    )
}

pub(crate) fn inventory(positions: Vec<Position>) -> Inventory {
    positions.into_iter().collect()
}

/// A context whose price map holds `(date, base, quote, rate)` points.
pub(crate) fn context_with_prices(points: &[(&str, &str, &str, &str)]) -> TestContext {
    let rates = points.iter().map(|(_, _, _, rate)| d(rate)).collect::<Vec<_>>();
    let prices = PriceMap::from_points(points.iter().zip(&rates).map(|((day, base, quote, _), rate)| (date(day), *base, *quote, rate)));
    TestContext {
        prices,
        ..TestContext::default()
    }
}

/// A context with distinct posting and entry metadata.
pub(crate) struct MetaContext {
    pub inner: TestContext,
    pub posting: HashMap<String, String>,
    pub entry: HashMap<String, String>,
}

impl FunctionContext for MetaContext {
    fn today(&self) -> NaiveDate {
        self.inner.today
    }
    fn prices(&self) -> &PriceMap {
        &self.inner.prices
    }
    fn entry_meta(&self, key: &str) -> Option<String> {
        self.entry.get(key).cloned()
    }
    fn posting_meta(&self, key: &str) -> Option<String> {
        self.posting.get(key).cloned()
    }
}

#[test]
fn every_function_is_documented_and_no_overload_is_shadowed() {
    for (idx, function) in SCALAR_FUNCTIONS.iter().enumerate() {
        assert!(function.description.ends_with('.'), "{} needs a one-sentence description", function.signature());
        assert_eq!(function.name, function.name.to_lowercase(), "{} must be lower case", function.signature());
        for earlier in &SCALAR_FUNCTIONS[..idx] {
            assert!(
                !(earlier.name == function.name && earlier.params == function.params),
                "{} is shadowed by an earlier overload",
                function.signature()
            );
        }
    }
}

#[test]
fn null_arguments_propagate() {
    assert_eq!(call("units", vec![Value::Null]), Value::Null);
    assert_eq!(call("convert", vec![Value::Amount(amount("1", "USD")), Value::Null]), Value::Null);
}
