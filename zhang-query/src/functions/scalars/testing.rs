//! Test helpers for the scalar function library, plus registry-wide sanity checks.

use std::collections::{BTreeSet, HashMap};
use std::fmt::Write;
use std::str::FromStr;
use std::sync::Arc;

use bigdecimal::BigDecimal;
use chrono::{NaiveDate, NaiveTime};
use zhang_core::data_source::LocalFileSystemDataSource;
use zhang_core::data_type::text::ZhangDataType;
use zhang_core::ledger::Ledger;

use crate::functions::{resolve_scalar, Eval, ParamType, SCALAR_FUNCTIONS};
use crate::projector::Projection;
use crate::table::{Dataset, RowRef};
use crate::value::{Cost, DataType, Inventory, Position, Value};
use crate::Amount;

/// A ledger to unit-test functions with: "today" and the time of day of "now", the `price` directives of `prices`,
/// and one transaction with the metadata `meta`, whose first posting is the row being evaluated.
pub(crate) struct TestContext {
    pub today: NaiveDate,
    pub time: NaiveTime,
    /// `(date, base, quote, rate)` points, in ledger order
    pub prices: Vec<(String, String, String, String)>,
    pub meta: HashMap<String, String>,
}

impl Default for TestContext {
    fn default() -> Self {
        TestContext {
            today: NaiveDate::from_ymd_opt(2024, 6, 30).expect("valid date"),
            time: NaiveTime::MIN,
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
        let data = Dataset::new(&ledger, self.context.today.and_time(self.context.time), Projection::all());
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

/// Values of a type a function could answer NULL or an error to: empty, zero, mismatched and unpriced ones.
fn samples(ty: DataType) -> Vec<Value> {
    match ty {
        DataType::Bool => vec![Value::Bool(true), Value::Bool(false)],
        DataType::Int => vec![Value::Int(0), Value::Int(-1), Value::Int(i64::MIN), Value::Int(i64::MAX)],
        DataType::Decimal => vec![Value::Decimal(d("0")), Value::Decimal(d("-1.50")), Value::Decimal(d("1E+30"))],
        DataType::Str => ["", "USD", "EUR", "Assets", "Assets:Bank", "assets:bank", "x y"]
            .into_iter()
            .map(Value::from)
            .collect(),
        DataType::Date => vec![Value::Date(date("2024-02-29")), Value::Date(date("1970-01-01"))],
        DataType::Set => vec![Value::Set(BTreeSet::new()), Value::Set(BTreeSet::from(["a".to_owned()]))],
        DataType::Amount => vec![
            Value::Amount(amount("0", "USD")),
            Value::Amount(amount("-2.50", "EUR")),
            Value::Amount(amount("10", "AAPL")),
        ],
        DataType::Position => vec![
            Value::Position(position("1", "USD", None)),
            Value::Position(position("10", "AAPL", Some(("100", "USD")))),
            Value::Position(position("-5", "XYZ", Some(("2", "EUR")))),
        ],
        DataType::Inventory => vec![
            Value::Inventory(inventory(vec![])),
            Value::Inventory(inventory(vec![
                position("10", "AAPL", Some(("100", "USD"))),
                position("-1011.00", "USD", None),
                position("3", "EUR", None),
            ])),
        ],
        DataType::Null | DataType::Interval | DataType::Metas => vec![],
    }
}

/// every way to pick one value per list, in order
fn combinations(lists: &[Vec<Value>]) -> Vec<Vec<Value>> {
    lists.iter().fold(vec![vec![]], |picked, list| {
        picked
            .iter()
            .flat_map(|args| list.iter().map(move |value| args.iter().cloned().chain([value.clone()]).collect()))
            .collect()
    })
}

/// A `#[total]` function answers a value, never NULL or an error, whatever its arguments: the projector trusts the
/// flag to defer `first()` / `last()` over a running total to one row, which must not be a NULL one. Every total
/// function is called with every combination of values of its parameter types that could make it answer NULL:
/// another currency, an empty inventory, an empty string, no price.
#[test]
fn every_total_function_returns_a_value() {
    let ctx = context_with_prices(&[("2024-01-01", "AAPL", "USD", "150")]);
    let any = [
        DataType::Bool,
        DataType::Int,
        DataType::Decimal,
        DataType::Str,
        DataType::Date,
        DataType::Set,
        DataType::Amount,
        DataType::Position,
        DataType::Inventory,
    ];
    for function in SCALAR_FUNCTIONS.iter().filter(|it| it.total) {
        let per_param = function
            .params
            .iter()
            .map(|param| match param {
                ParamType::Exact(ty) | ParamType::Variadic(ty) => samples(*ty),
                ParamType::Any => any.into_iter().flat_map(samples).collect(),
            })
            .collect::<Vec<_>>();
        for args in combinations(&per_param) {
            let types = args.iter().map(Value::data_type).collect::<Vec<_>>();
            let result = Fixture::from(&ctx).eval(function.eval, &args);
            match &result {
                Ok(value) if !value.is_null() => assert_eq!(
                    value.data_type(),
                    function.returns.resolve(&types),
                    "{} returned {:?} for {:?}",
                    function.signature(),
                    value,
                    args
                ),
                _ => panic!("{} is flagged total but answers {:?} to {:?}", function.signature(), result, args),
            }
        }
    }
}

#[test]
fn null_arguments_propagate() {
    assert_eq!(call("units", vec![Value::Null]), Value::Null);
    assert_eq!(call("convert", vec![Value::Amount(amount("1", "USD")), Value::Null]), Value::Null);
}
