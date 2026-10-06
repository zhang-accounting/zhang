//! Test helpers for the scalar function library, plus registry-wide sanity checks.

use std::collections::HashMap;
use std::fmt::Write;
use std::str::FromStr;
use std::sync::{Arc, PoisonError};

use bigdecimal::BigDecimal;
use chrono::NaiveDate;
use zhang_core::data_source::LocalFileSystemDataSource;
use zhang_core::data_type::text::ZhangDataType;
use zhang_core::ledger::Ledger;

use crate::functions::{resolve_scalar, Eval, SCALAR_FUNCTIONS};
use crate::projector::Projection;
use crate::table::{Dataset, RowRef};
use crate::value::{Cost, Inventory, Position, Value};
use crate::Amount;

/// A ledger to unit-test functions with: "today", the `price` directives of `prices`, and one
/// transaction with the metadata `meta`, whose first posting is the row being evaluated.
pub(crate) struct TestContext {
    pub today: NaiveDate,
    /// `(date, base, quote, rate)` points, in ledger order
    pub prices: Vec<(String, String, String, String)>,
    pub meta: HashMap<String, String>,
}

impl Default for TestContext {
    fn default() -> Self {
        TestContext {
            today: NaiveDate::from_ymd_opt(2024, 6, 30).expect("valid date"),
            prices: vec![],
            meta: Default::default(),
        }
    }
}

/// A [`TestContext`] whose row has distinct posting and transaction metadata.
pub(crate) struct MetaContext {
    pub inner: TestContext,
    pub posting: HashMap<String, String>,
    pub entry: HashMap<String, String>,
}

/// The ledger a [`TestContext`] or a [`MetaContext`] describes.
pub(crate) struct Fixture<'c> {
    context: &'c TestContext,
    posting: Option<&'c HashMap<String, String>>,
    entry: &'c HashMap<String, String>,
}

impl<'c> From<&'c TestContext> for Fixture<'c> {
    fn from(context: &'c TestContext) -> Self {
        Fixture {
            context,
            posting: None,
            entry: &context.meta,
        }
    }
}

impl<'c> From<&'c MetaContext> for Fixture<'c> {
    fn from(context: &'c MetaContext) -> Self {
        Fixture {
            context: &context.inner,
            posting: Some(&context.posting),
            entry: &context.entry,
        }
    }
}

impl Fixture<'_> {
    fn ledger(&self) -> String {
        let meta = |text: &mut String, indent: &str, meta: &HashMap<String, String>| {
            let mut pairs = meta.iter().collect::<Vec<_>>();
            pairs.sort();
            for (key, value) in pairs {
                writeln!(text, "{}{}: \"{}\"", indent, key, value).unwrap();
            }
        };
        let mut text = String::new();
        for (date, base, quote, rate) in &self.context.prices {
            writeln!(text, "{} price {} {} {}", date, base, rate, quote).unwrap();
        }
        text.push_str("1970-01-01 open Assets:Row\n1970-01-01 open Equity:Row\n2000-01-01 * \"the row\"\n");
        meta(&mut text, "  ", self.entry);
        text.push_str("  Assets:Row 1 ROW\n");
        meta(&mut text, "    ", self.posting.unwrap_or(&HashMap::new()));
        text.push_str("  Equity:Row -1 ROW\n");
        text
    }

    /// Evaluate the implementation, with the execution of a query of the ledger at its row when it reads them.
    fn eval(&self, eval: Eval, args: &[Value]) -> Result<Value, String> {
        match eval {
            Eval::Args(eval) => eval(args),
            Eval::Execution(eval) => self.execute(|data, _| eval(args, data)),
            Eval::Row(eval) => self.execute(|data, row| eval(args, data, row)),
        }
    }

    fn execute<R>(&self, f: impl FnOnce(&Dataset<'_>, Option<RowRef<'_, '_>>) -> R) -> R {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("main.zhang"), self.ledger()).expect("write ledger");
        let source = LocalFileSystemDataSource::new(ZhangDataType {});
        let ledger = Ledger::load_with_data_source(dir.path().to_path_buf(), "main.zhang".to_owned(), Arc::new(source)).expect("cannot load ledger");
        let store = ledger.store.read().unwrap_or_else(PoisonError::into_inner);
        let data = Dataset::new(&ledger, &store, self.context.today, Projection::all());
        f(&data, data.rows.first().map(RowRef::Posting))
    }
}

/// Resolve `name` against the static types of `args` and evaluate it the way the evaluator
/// does: widen int → decimal where the overload asks for it, propagate NULL, and check the
/// result against the declared return type.
pub(crate) fn try_call_with<'c>(ctx: impl Into<Fixture<'c>>, name: &str, args: Vec<Value>) -> Result<Value, String> {
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
    let result = ctx.into().eval(resolved.function.eval, &args)?;
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

pub(crate) fn call_with<'c>(ctx: impl Into<Fixture<'c>>, name: &str, args: Vec<Value>) -> Value {
    try_call_with(ctx, name, args).unwrap()
}

pub(crate) fn call(name: &str, args: Vec<Value>) -> Value {
    call_with(&TestContext::default(), name, args)
}

/// Evaluate and render the result with `str()` semantics, which keeps decimal scales visible.
pub(crate) fn call_str_with<'c>(ctx: impl Into<Fixture<'c>>, name: &str, args: Vec<Value>) -> String {
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

/// A context whose ledger has the `price` directives of `(date, base, quote, rate)` points.
pub(crate) fn context_with_prices(points: &[(&str, &str, &str, &str)]) -> TestContext {
    TestContext {
        prices: points
            .iter()
            .map(|(day, base, quote, rate)| (day.to_string(), base.to_string(), quote.to_string(), rate.to_string()))
            .collect(),
        ..TestContext::default()
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
