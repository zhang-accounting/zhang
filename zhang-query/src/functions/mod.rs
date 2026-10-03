//! Function registries.
//!
//! Every function is described by a declarative entry: its name, parameter types,
//! return type, a one-line description and an implementation. Overloads are simply
//! several entries with the same name; the compiler picks the first entry whose
//! parameters accept the argument types. The same entries generate the
//! `GET /api/query/schema` function list, so there is no second list to maintain.
//!
//! # Adding a scalar function
//!
//! Add an entry to [`scalars::SCALAR_FUNCTIONS`] (one entry per overload) and an
//! implementation with the [`ScalarImpl`] signature:
//!
//! ```ignore
//! ScalarFunction {
//!     name: "year",
//!     params: &[ParamType::Exact(DataType::Date)],
//!     returns: ReturnType::Exact(DataType::Int),
//!     description: "The year of a date.",
//!     eval: year,
//! },
//!
//! fn year(args: &[Value], _ctx: &dyn FunctionContext) -> Result<Value, String> {
//!     let date = args[0].as_date().ok_or("year() expects a date")?;
//!     Ok(Value::Int(date.year() as i64))
//! }
//! ```
//!
//! Contract for implementations:
//! - Arguments arrive already type-checked against `params`: an `Exact(Decimal)` parameter
//!   receives `Value::Decimal` (integer arguments are widened by the compiler), an
//!   `Exact(T)` parameter receives `T`, and `Any` receives any non-null value.
//! - Functions are NULL-propagating: if any argument is `NULL` the evaluator returns `NULL`
//!   without calling `eval`.
//! - The returned value must have the declared return type (or be `Value::Null`).
//! - Return `Err(message)` for runtime failures; the evaluator attaches the source position
//!   of the call.
//! - Use the [`FunctionContext`] for anything outside the arguments: today's date, the
//!   price map, the `open`, `close` and `commodity` directives of the ledger, and metadata
//!   of the row being evaluated.

pub mod aggregates;
pub mod scalars;

use chrono::NaiveDate;
use zhang_ast::{Close, Commodity, Open};

pub use self::aggregates::{AggregateFunction, AggregateKind};
pub(crate) use self::scalars::is_under;
pub use self::scalars::SCALAR_FUNCTIONS;
use crate::prices::PriceMap;
use crate::value::{DataType, Value};

/// A parameter of a function signature.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamType {
    /// exactly this type (`Int` arguments are accepted, and widened, for `Decimal`)
    Exact(DataType),
    /// any type
    Any,
}

impl ParamType {
    /// Whether an argument of static type `arg` is accepted, and if so whether it needs to
    /// be widened from `int` to `decimal`.
    pub(crate) fn accepts(&self, arg: DataType) -> Option<bool> {
        match self {
            ParamType::Any => Some(false),
            ParamType::Exact(_) if arg == DataType::Null => Some(false),
            ParamType::Exact(expected) if *expected == arg => Some(false),
            ParamType::Exact(DataType::Decimal) if arg == DataType::Int => Some(true),
            ParamType::Exact(_) => None,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            ParamType::Exact(ty) => ty.name(),
            ParamType::Any => "any",
        }
    }
}

/// The return type of a function signature.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReturnType {
    Exact(DataType),
    /// the static type of the argument at this index (e.g. `first(x)` returns the type of `x`)
    SameAsArg(usize),
}

impl ReturnType {
    pub(crate) fn resolve(&self, args: &[DataType]) -> DataType {
        match self {
            ReturnType::Exact(ty) => *ty,
            ReturnType::SameAsArg(idx) => args.get(*idx).copied().unwrap_or(DataType::Null),
        }
    }

    fn describe(&self, params: &[ParamType]) -> &'static str {
        match self {
            ReturnType::Exact(ty) => ty.name(),
            ReturnType::SameAsArg(idx) => params.get(*idx).map(|it| it.name()).unwrap_or("any"),
        }
    }
}

/// What a scalar function can see besides its arguments.
pub trait FunctionContext {
    /// "Today" for `today()`: the current date in the ledger's timezone, unless the caller
    /// pinned it (see [`crate::Query::execute_at`]).
    fn today(&self) -> NaiveDate;

    /// The ledger's price map (built lazily on first use).
    fn prices(&self) -> &PriceMap;

    /// Metadata `key` of the transaction of the row being evaluated, as a string.
    /// `None` when absent or when there is no current row.
    fn entry_meta(&self, key: &str) -> Option<String>;

    /// Metadata `key` of the posting being evaluated.
    fn posting_meta(&self, key: &str) -> Option<String>;

    /// Every value of the posting metadata `key` of the row being evaluated, in written order
    /// (a repeated key has several). Defaults to [`FunctionContext::posting_meta`].
    fn posting_meta_values(&self, key: &str) -> Vec<String> {
        self.posting_meta(key).into_iter().collect()
    }

    /// Every value of the transaction metadata `key` of the row being evaluated, in written
    /// order. Defaults to [`FunctionContext::entry_meta`].
    fn entry_meta_values(&self, key: &str) -> Vec<String> {
        self.entry_meta(key).into_iter().collect()
    }

    /// The `open` and `close` directives of an account (the earliest of each, as beancount
    /// keeps them); `None` when the account has neither.
    fn account_directives(&self, _account: &str) -> Option<AccountDirectives<'_>> {
        None
    }

    /// The `commodity` directive of a currency (the last one, as beancount keeps it).
    fn commodity_directive(&self, _currency: &str) -> Option<&Commodity> {
        None
    }
}

/// The `open` and `close` directives of an account, as [`FunctionContext::account_directives`]
/// finds them.
#[derive(Debug, Clone, Copy, Default)]
pub struct AccountDirectives<'a> {
    pub open: Option<&'a Open>,
    pub close: Option<&'a Close>,
}

/// Implementation of a scalar function, see the module docs for the contract.
pub type ScalarImpl = fn(&[Value], &dyn FunctionContext) -> Result<Value, String>;

/// One overload of a scalar (row-level) function.
pub struct ScalarFunction {
    pub name: &'static str,
    pub params: &'static [ParamType],
    pub returns: ReturnType,
    pub description: &'static str,
    pub eval: ScalarImpl,
}

impl ScalarFunction {
    /// e.g. `root(str, int) -> str`
    pub fn signature(&self) -> String {
        format!(
            "{}({}) -> {}",
            self.name,
            self.params.iter().map(ParamType::name).collect::<Vec<_>>().join(", "),
            self.returns.describe(self.params)
        )
    }
}

impl std::fmt::Debug for ScalarFunction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.signature())
    }
}

/// A resolved call: the chosen overload and, per argument, whether to widen int → decimal.
pub(crate) struct Resolved<T: 'static> {
    pub function: &'static T,
    pub widen: Vec<bool>,
}

/// Find the first scalar overload named `name` (case-insensitive) accepting `args`.
pub(crate) fn resolve_scalar(name: &str, args: &[DataType]) -> Result<Resolved<ScalarFunction>, String> {
    let candidates = SCALAR_FUNCTIONS.iter().filter(|it| it.name.eq_ignore_ascii_case(name)).collect::<Vec<_>>();
    resolve(name, args, &candidates, |it| it.params, |it| it.signature())
}

pub(crate) fn resolve<T: 'static>(
    name: &str, args: &[DataType], candidates: &[&'static T], params_of: impl Fn(&T) -> &'static [ParamType], signature_of: impl Fn(&T) -> String,
) -> Result<Resolved<T>, String> {
    if candidates.is_empty() {
        return Err(format!("unknown function '{}'", name));
    }
    for candidate in candidates {
        let params = params_of(candidate);
        if params.len() != args.len() {
            continue;
        }
        let widen = params.iter().zip(args).map(|(param, arg)| param.accepts(*arg)).collect::<Option<Vec<bool>>>();
        if let Some(widen) = widen {
            return Ok(Resolved { function: candidate, widen });
        }
    }
    Err(format!(
        "no overload of {}({}); available: {}",
        name.to_lowercase(),
        args.iter().map(DataType::name).collect::<Vec<_>>().join(", "),
        candidates.iter().map(|it| signature_of(it)).collect::<Vec<_>>().join(", ")
    ))
}

/// A [`FunctionContext`] for unit-testing functions without a ledger.
#[cfg(test)]
pub(crate) struct TestContext {
    pub today: NaiveDate,
    pub prices: PriceMap,
    pub meta: std::collections::HashMap<String, String>,
}

#[cfg(test)]
impl Default for TestContext {
    fn default() -> Self {
        TestContext {
            today: NaiveDate::from_ymd_opt(2024, 6, 30).expect("valid date"),
            prices: PriceMap::default(),
            meta: Default::default(),
        }
    }
}

#[cfg(test)]
impl FunctionContext for TestContext {
    fn today(&self) -> NaiveDate {
        self.today
    }
    fn prices(&self) -> &PriceMap {
        &self.prices
    }
    fn entry_meta(&self, key: &str) -> Option<String> {
        self.meta.get(key).cloned()
    }
    fn posting_meta(&self, _key: &str) -> Option<String> {
        None
    }
}
