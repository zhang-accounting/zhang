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
use ParamType::{Any, Exact};

pub static AGGREGATE_FUNCTIONS: &[AggregateFunction] = &[
    AggregateFunction {
        name: "count",
        params: &[],
        star: true,
        returns: ReturnType::Exact(Int),
        description: "Number of rows.",
        kind: AggregateKind::CountRows,
    },
    AggregateFunction {
        name: "count",
        params: &[Any],
        star: false,
        returns: ReturnType::Exact(Int),
        description: "Number of non-NULL values.",
        kind: AggregateKind::Count,
    },
    AggregateFunction {
        name: "sum",
        params: &[Exact(Int)],
        star: false,
        returns: ReturnType::Exact(Int),
        description: "Sum of integers.",
        kind: AggregateKind::SumInt,
    },
    AggregateFunction {
        name: "sum",
        params: &[Exact(Decimal)],
        star: false,
        returns: ReturnType::Exact(Decimal),
        description: "Sum of decimals.",
        kind: AggregateKind::SumDecimal,
    },
    AggregateFunction {
        name: "sum",
        params: &[Exact(Amount)],
        star: false,
        returns: ReturnType::Exact(Inventory),
        description: "Sum of amounts into an inventory (one position per currency).",
        kind: AggregateKind::SumInventory,
    },
    AggregateFunction {
        name: "sum",
        params: &[Exact(Position)],
        star: false,
        returns: ReturnType::Exact(Inventory),
        description: "Sum of positions into an inventory, keeping lots (cost) apart.",
        kind: AggregateKind::SumInventory,
    },
    AggregateFunction {
        name: "sum",
        params: &[Exact(Inventory)],
        star: false,
        returns: ReturnType::Exact(Inventory),
        description: "Sum of inventories.",
        kind: AggregateKind::SumInventory,
    },
    AggregateFunction {
        name: "first",
        params: &[Any],
        star: false,
        returns: ReturnType::SameAsArg(0),
        description: "The first non-NULL value of the group, in row order.",
        kind: AggregateKind::First,
    },
    AggregateFunction {
        name: "last",
        params: &[Any],
        star: false,
        returns: ReturnType::SameAsArg(0),
        description: "The last non-NULL value of the group, in row order.",
        kind: AggregateKind::Last,
    },
    AggregateFunction {
        name: "min",
        params: &[Any],
        star: false,
        returns: ReturnType::SameAsArg(0),
        description: "The smallest non-NULL value of the group.",
        kind: AggregateKind::Min,
    },
    AggregateFunction {
        name: "max",
        params: &[Any],
        star: false,
        returns: ReturnType::SameAsArg(0),
        description: "The largest non-NULL value of the group.",
        kind: AggregateKind::Max,
    },
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
