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
//! Add a row to [`scalars::SCALAR_FUNCTIONS`] (one row per overload) and an implementation:
//!
//! ```ignore
//! year(Date) -> Int = year, "The year of a date.";
//!
//! fn year(args: &[Value]) -> Result<Value, String> {
//!     let date = args[0].as_date().ok_or("year() expects a date")?;
//!     Ok(Value::Int(date.year() as i64))
//! }
//! ```
//!
//! A row starts with its flags when the function reads more than its arguments or is total:
//! - `#[execution]`: it also reads the execution, its date ("today") and the ledger's prices and
//!   `open`, `close` and `commodity` directives, and takes the execution's rows and lookups
//!   (`fn(&[Value], &Dataset) -> ...`). It is never folded into a constant.
//! - `#[row]`: it also reads the row being evaluated, its metadata, and takes the row as well
//!   (`fn(&[Value], &Dataset, Option<RowRef>) -> ...`, no row over finished aggregates). Like a
//!   column, it must be grouped or aggregated in a grouped query, and it is never folded.
//! - `#[total]`: it returns a value, never NULL or an error, for any arguments of its types, so
//!   the executor may evaluate it for fewer rows.
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

/// The parameters of a registry row, as [`ParamType`]s: a type, `any`, `type...` for a variadic
/// last parameter, or `*` (none) for `count(*)`.
macro_rules! params {
    ([] *) => { &[] };
    ([$($done:expr),*]) => { &[$($done),*] };
    ([$($done:expr),*] any $(, $($rest:tt)*)?) => { params!([$($done,)* ParamType::Any] $($($rest)*)?) };
    ([$($done:expr),*] $ty:ident ... $(, $($rest:tt)*)?) => { params!([$($done,)* ParamType::Variadic($ty)] $($($rest)*)?) };
    ([$($done:expr),*] $ty:ident $(, $($rest:tt)*)?) => { params!([$($done,)* ParamType::Exact($ty)] $($($rest)*)?) };
}

/// The return type of a registry row, as a [`ReturnType`]: a type, or `SameAsArg(i)`.
macro_rules! returns {
    (SameAsArg($idx:literal)) => {
        ReturnType::SameAsArg($idx)
    };
    ($ty:ident) => {
        ReturnType::Exact($ty)
    };
}

pub mod aggregates;
pub mod scalars;

use zhang_ast::{Close, Open};

pub use self::aggregates::{AggregateFunction, AggregateKind};
#[cfg(test)]
pub(crate) use self::scalars::TestContext;
pub use self::scalars::SCALAR_FUNCTIONS;
pub(crate) use self::scalars::{is_under, reads_the_row};
use crate::table::{Dataset, RowRef};
use crate::value::{DataType, Value};

/// A parameter of a function signature.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamType {
    /// exactly this type (`Int` arguments are accepted, and widened, for `Decimal`)
    Exact(DataType),
    /// any type
    Any,
    /// any number of arguments, also none, each of this type as for [`ParamType::Exact`]; only
    /// as the last parameter
    Variadic(DataType),
}

impl ParamType {
    /// Whether an argument of static type `arg` is accepted, and if so whether it needs to
    /// be widened from `int` to `decimal`.
    pub(crate) fn accepts(&self, arg: DataType) -> Option<bool> {
        match self {
            ParamType::Any => Some(false),
            ParamType::Exact(_) | ParamType::Variadic(_) if arg == DataType::Null => Some(false),
            ParamType::Exact(expected) | ParamType::Variadic(expected) if *expected == arg => Some(false),
            ParamType::Exact(DataType::Decimal) | ParamType::Variadic(DataType::Decimal) if arg == DataType::Int => Some(true),
            ParamType::Exact(_) | ParamType::Variadic(_) => None,
        }
    }

    /// The name of the type (of each argument, for [`ParamType::Variadic`]).
    pub fn name(&self) -> &'static str {
        match self {
            ParamType::Exact(ty) | ParamType::Variadic(ty) => ty.name(),
            ParamType::Any => "any",
        }
    }
}

/// The parameters of a signature, e.g. `str, int`, and `str, ...` for a variadic `str`.
pub(crate) fn params_signature(params: &[ParamType]) -> String {
    params
        .iter()
        .map(|param| match param {
            ParamType::Variadic(ty) => format!("{}, ...", ty.name()),
            other => other.name().to_owned(),
        })
        .collect::<Vec<_>>()
        .join(", ")
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

/// The `open` and `close` directives of an account, as [`Dataset::account_directives`] finds them.
#[derive(Debug, Clone, Copy, Default)]
pub struct AccountDirectives<'a> {
    pub open: Option<&'a Open>,
    pub close: Option<&'a Close>,
}

/// The implementation of a scalar function, which says what it reads besides its arguments; see the
/// module docs for the contract.
#[derive(Clone, Copy)]
pub(crate) enum Eval {
    /// only its arguments: a call whose arguments are constants is folded when the query compiles
    Args(fn(&[Value]) -> Outcome),
    /// the execution as well (`#[execution]`): its date and the ledger's prices and directives
    Execution(fn(&[Value], &Dataset<'_>) -> Outcome),
    /// the row being evaluated as well (`#[row]`), `None` over finished aggregates
    Row(fn(&[Value], &Dataset<'_>, Option<RowRef<'_, '_>>) -> Outcome),
}

/// What an implementation returns: the value, or the message of a runtime failure.
pub(crate) type Outcome = Result<Value, String>;

/// One overload of a scalar (row-level) function.
pub struct ScalarFunction {
    pub name: &'static str,
    pub params: &'static [ParamType],
    pub returns: ReturnType,
    pub description: &'static str,
    pub(crate) eval: Eval,
    /// it returns a value, never NULL or an error, for any arguments of its types (`#[total]`)
    pub(crate) total: bool,
}

impl ScalarFunction {
    /// e.g. `root(str, int) -> str`
    pub fn signature(&self) -> String {
        format!("{}({}) -> {}", self.name, params_signature(self.params), self.returns.describe(self.params))
    }

    /// Whether it reads nothing but its arguments, so that a call with constant arguments can be folded.
    pub(crate) fn reads_only_its_arguments(&self) -> bool {
        matches!(self.eval, Eval::Args(_))
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
        let fits = match params.split_last() {
            Some((ParamType::Variadic(_), fixed)) => args.len() >= fixed.len(),
            _ => params.len() == args.len(),
        };
        if !fits {
            continue;
        }
        // the arguments past the last parameter are those of a variadic one
        let widen = args
            .iter()
            .enumerate()
            .map(|(idx, arg)| params.get(idx).or(params.last()).and_then(|param| param.accepts(*arg)))
            .collect::<Option<Vec<bool>>>();
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
