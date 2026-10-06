//! Aggregate functions. Their accumulators live in the evaluator ([`crate::executor`]);
//! this module only declares the signatures.

use super::{resolve, ParamType, Resolved, ReturnType};
use crate::value::DataType;

/// How an aggregate accumulates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AggregateKind {
    /// `count(*)`: number of rows
    CountRows,
    /// `count(x)`: number of non-NULL values
    Count,
    SumInt,
    SumDecimal,
    /// sum of amounts/positions/inventories into an inventory
    SumInventory,
    /// first non-NULL value in row order
    First,
    /// last non-NULL value in row order
    Last,
    Min,
    Max,
}

/// One overload of an aggregate function.
#[derive(Debug)]
pub struct AggregateFunction {
    pub name: &'static str,
    /// parameter types; empty for `count(*)`
    pub params: &'static [ParamType],
    pub star: bool,
    pub returns: ReturnType,
    pub description: &'static str,
    pub kind: AggregateKind,
}

impl AggregateFunction {
    pub fn signature(&self) -> String {
        let params = if self.star { "*".to_owned() } else { super::params_signature(self.params) };
        let returns = match self.returns {
            ReturnType::Exact(ty) => ty.name(),
            ReturnType::SameAsArg(idx) => self.params.get(idx).map(ParamType::name).unwrap_or("any"),
        };
        format!("{}({}) -> {}", self.name, params, returns)
    }
}

use DataType::*;

/// The registry rows: `name(param) -> returns = kind, "description";`, one per overload, `name(*)` for `count(*)`.
macro_rules! aggregates {
    ($($name:ident($($param:tt)*) -> $returns:ident $(($arg:literal))? = $kind:ident, $description:literal;)*) => {
        &[$(AggregateFunction {
            name: stringify!($name),
            params: params!([] $($param)*),
            star: star!($($param)*),
            returns: returns!($returns $(($arg))?),
            description: $description,
            kind: AggregateKind::$kind,
        }),*]
    };
}

/// Whether a row's parameters are `*`.
macro_rules! star {
    (*) => {
        true
    };
    ($($param:tt)*) => {
        false
    };
}

/// The aggregate function registry: one row per overload.
pub static AGGREGATE_FUNCTIONS: &[AggregateFunction] = aggregates![
    count(*) -> Int = CountRows, "Number of rows.";
    count(any) -> Int = Count, "Number of non-NULL values.";
    sum(Int) -> Int = SumInt, "Sum of integers.";
    sum(Decimal) -> Decimal = SumDecimal, "Sum of decimals.";
    sum(Amount) -> Inventory = SumInventory, "Sum of amounts into an inventory (one position per currency).";
    sum(Position) -> Inventory = SumInventory, "Sum of positions into an inventory, keeping lots (cost) apart.";
    sum(Inventory) -> Inventory = SumInventory, "Sum of inventories.";
    first(any) -> SameAsArg(0) = First, "The first non-NULL value of the group, in row order.";
    last(any) -> SameAsArg(0) = Last, "The last non-NULL value of the group, in row order.";
    min(any) -> SameAsArg(0) = Min, "The smallest non-NULL value of the group.";
    max(any) -> SameAsArg(0) = Max, "The largest non-NULL value of the group.";
];

pub(crate) fn is_aggregate(name: &str) -> bool {
    AGGREGATE_FUNCTIONS.iter().any(|it| it.name.eq_ignore_ascii_case(name))
}

/// Resolve `name(args)`, or `name(*)` when `star` is set.
pub(crate) fn resolve_aggregate(name: &str, star: bool, args: &[DataType]) -> Result<Resolved<AggregateFunction>, String> {
    let candidates = AGGREGATE_FUNCTIONS
        .iter()
        .filter(|it| it.name.eq_ignore_ascii_case(name) && it.star == star)
        .collect::<Vec<_>>();
    if star && candidates.is_empty() {
        return Err(format!("{}(*) is not supported; only count(*) is", name.to_lowercase()));
    }
    resolve(name, args, &candidates, |it| it.params, |it| it.signature())
}
