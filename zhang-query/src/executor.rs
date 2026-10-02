//! Evaluation of a [`Plan`] over a [`Dataset`].

use std::cmp::Ordering;
use std::collections::HashSet;

use bigdecimal::{BigDecimal, Zero};
use chrono::{Duration, NaiveDate};
use indexmap::IndexMap;
use zhang_ast::amount::Amount;

use crate::compiler::{build_regex, AggregateCall, ArithOp, CExpr, CmpOp, Plan, RegexPattern};
use crate::decimal;
use crate::error::LocatedError;
use crate::functions::{AggregateKind, FunctionContext};
use crate::params::Params;
use crate::prices::PriceMap;
use crate::table::{Dataset, Row};
use crate::value::{Inventory, Value};

/// The evaluation environment of one expression evaluation.
#[derive(Clone, Copy)]
pub(crate) struct Env<'e, 'a> {
    pub data: &'e Dataset<'a>,
    /// the current row; `None` when evaluating finished aggregates
    pub row: Option<&'e Row<'a>>,
    /// finished aggregate values of the current group
    pub aggregates: &'e [Value],
    pub params: &'e Params,
}

impl FunctionContext for Env<'_, '_> {
    fn today(&self) -> NaiveDate {
        self.data.today
    }

    fn prices(&self) -> &PriceMap {
        self.data.prices()
    }

    fn entry_meta(&self, key: &str) -> Option<String> {
        self.row.and_then(|row| self.data.entry_meta(row, key))
    }

    fn posting_meta(&self, key: &str) -> Option<String> {
        self.row.and_then(|row| self.data.posting_meta(row, key))
    }
}

impl CExpr {
    pub(crate) fn eval(&self, env: &Env<'_, '_>) -> Result<Value, LocatedError> {
        Ok(match self {
            CExpr::Const(value) => value.clone(),
            CExpr::Column(def) => match env.row {
                Some(row) => (def.get)(env.data, row),
                None => return Err(LocatedError::eval(format!("column '{}' is not available here", def.name), None)),
            },
            CExpr::Param(param) => env.params.get(param).cloned().unwrap_or(Value::Null),
            CExpr::Scalar { function, args, span } => {
                let mut values = Vec::with_capacity(args.len());
                for arg in args {
                    let value = arg.eval(env)?;
                    if value.is_null() {
                        return Ok(Value::Null);
                    }
                    values.push(value);
                }
                (function.eval)(&values, env).map_err(|message| LocatedError::eval(format!("{}(): {}", function.name, message), Some(*span)))?
            }
            CExpr::Aggregate(idx) => env.aggregates.get(*idx).cloned().unwrap_or(Value::Null),
            CExpr::WidenInt(inner) => match inner.eval(env)? {
                Value::Int(it) => Value::Decimal(BigDecimal::from(it)),
                other => other,
            },
            CExpr::Neg(inner, span) => match inner.eval(env)? {
                Value::Null => Value::Null,
                Value::Int(it) => Value::Int(it.checked_neg().ok_or_else(|| LocatedError::eval("integer overflow", Some(*span)))?),
                Value::Decimal(it) => Value::Decimal(-it),
                Value::Amount(it) => Value::Amount(-it),
                Value::Position(it) => Value::Position(-it),
                Value::Inventory(it) => Value::Inventory(-it),
                other => return Err(LocatedError::eval(format!("cannot negate {}", other.data_type()), Some(*span))),
            },
            // three-valued logic: NOT NULL is NULL
            CExpr::Not(inner) => match inner.eval(env)? {
                Value::Bool(it) => Value::Bool(!it),
                _ => Value::Null,
            },
            CExpr::And(left, right) => {
                let left = left.eval(env)?;
                if matches!(left, Value::Bool(false)) {
                    return Ok(Value::Bool(false));
                }
                match (left, right.eval(env)?) {
                    (_, Value::Bool(false)) => Value::Bool(false),
                    (Value::Bool(true), Value::Bool(true)) => Value::Bool(true),
                    _ => Value::Null,
                }
            }
            CExpr::Or(left, right) => {
                let left = left.eval(env)?;
                if matches!(left, Value::Bool(true)) {
                    return Ok(Value::Bool(true));
                }
                match (left, right.eval(env)?) {
                    (_, Value::Bool(true)) => Value::Bool(true),
                    (Value::Bool(false), Value::Bool(false)) => Value::Bool(false),
                    _ => Value::Null,
                }
            }
            CExpr::Arith { op, left, right, span } => {
                arithmetic(*op, left.eval(env)?, right.eval(env)?).map_err(|message| LocatedError::eval(message, Some(*span)))?
            }
            CExpr::Compare { op, left, right } => {
                let (left, right) = (left.eval(env)?, right.eval(env)?);
                if left.is_null() || right.is_null() {
                    return Ok(Value::Null);
                }
                Value::Bool(match op {
                    CmpOp::Eq => left == right,
                    CmpOp::Ne => left != right,
                    CmpOp::Lt => left.sort_cmp(&right) == Ordering::Less,
                    CmpOp::Le => left.sort_cmp(&right) != Ordering::Greater,
                    CmpOp::Gt => left.sort_cmp(&right) == Ordering::Greater,
                    CmpOp::Ge => left.sort_cmp(&right) != Ordering::Less,
                })
            }
            CExpr::Regex {
                subject,
                pattern,
                negated,
                span,
            } => {
                let subject = subject.eval(env)?;
                let Value::Str(subject) = subject else {
                    return Ok(Value::Null);
                };
                let matched = match pattern {
                    RegexPattern::Static(regex) => regex.is_match(&subject),
                    RegexPattern::Dynamic { expr, case_insensitive } => {
                        let Value::Str(pattern) = expr.eval(env)? else {
                            return Ok(Value::Null);
                        };
                        build_regex(&pattern, *case_insensitive)
                            .map_err(|message| LocatedError::eval(message, Some(*span)))?
                            .is_match(&subject)
                    }
                };
                Value::Bool(matched != *negated)
            }
            CExpr::InSet { needle, set, negated } => {
                let (needle, set) = (needle.eval(env)?, set.eval(env)?);
                match (needle, set) {
                    (Value::Str(needle), Value::Set(set)) => Value::Bool(set.contains(&needle) != *negated),
                    _ => Value::Null,
                }
            }
            CExpr::InList { needle, items, negated } => {
                let needle = needle.eval(env)?;
                if needle.is_null() {
                    return Ok(Value::Null);
                }
                let mut saw_null = false;
                let mut found = false;
                for item in items {
                    let item = item.eval(env)?;
                    if item.is_null() {
                        saw_null = true;
                    } else if item == needle {
                        found = true;
                        break;
                    }
                }
                if found {
                    Value::Bool(!*negated)
                } else if saw_null {
                    Value::Null
                } else {
                    Value::Bool(*negated)
                }
            }
            CExpr::IsNull { expr, negated } => Value::Bool(expr.eval(env)?.is_null() != *negated),
        })
    }
}

fn arithmetic(op: ArithOp, left: Value, right: Value) -> Result<Value, String> {
    use Value::*;
    let overflow = || "integer overflow".to_owned();
    Ok(match (left, right) {
        (Null, _) | (_, Null) => Null,
        (Int(a), Int(b)) => match op {
            ArithOp::Add => Int(a.checked_add(b).ok_or_else(overflow)?),
            ArithOp::Sub => Int(a.checked_sub(b).ok_or_else(overflow)?),
            ArithOp::Mul => Int(a.checked_mul(b).ok_or_else(overflow)?),
            ArithOp::Div => decimal::div(&BigDecimal::from(a), &BigDecimal::from(b)).map(Decimal).unwrap_or(Null),
        },
        (a @ (Int(_) | Decimal(_)), b @ (Int(_) | Decimal(_))) => {
            let (a, b) = (a.as_decimal().expect("numeric"), b.as_decimal().expect("numeric"));
            match op {
                ArithOp::Add => Decimal(a + b),
                ArithOp::Sub => Decimal(a - b),
                ArithOp::Mul => Decimal(a * b),
                ArithOp::Div => decimal::div(&a, &b).map(Decimal).unwrap_or(Null),
            }
        }
        (Str(a), Str(b)) if op == ArithOp::Add => Str(a + &b),
        (Date(date), Int(days)) => {
            let delta = Duration::try_days(days).ok_or("date out of range")?;
            let shifted = match op {
                ArithOp::Add => date.checked_add_signed(delta),
                _ => date.checked_sub_signed(delta),
            };
            Date(shifted.ok_or("date out of range")?)
        }
        (Int(days), Date(date)) => Date(
            date.checked_add_signed(Duration::try_days(days).ok_or("date out of range")?)
                .ok_or("date out of range")?,
        ),
        (Date(a), Date(b)) => Int((a - b).num_days()),
        (Amount(amount), n @ (Int(_) | Decimal(_))) => {
            let n = n.as_decimal().expect("numeric");
            match op {
                ArithOp::Mul => Amount(amount_with(&amount, &amount.number * n)),
                _ => decimal::div(&amount.number, &n)
                    .map(|number| Amount(amount_with(&amount, number)))
                    .unwrap_or(Null),
            }
        }
        (n @ (Int(_) | Decimal(_)), Amount(amount)) => Amount(amount_with(&amount, n.as_decimal().expect("numeric") * &amount.number)),
        (Amount(a), Amount(b)) => {
            if a.commodity != b.commodity {
                return Err(format!("cannot combine amounts in different currencies ({} and {})", a.commodity, b.commodity));
            }
            let number = if op == ArithOp::Add { &a.number + &b.number } else { &a.number - &b.number };
            Amount(amount_with(&a, number))
        }
        (a, b) => return Err(format!("unsupported operands {} and {}", a.data_type(), b.data_type())),
    })
}

fn amount_with(template: &Amount, number: BigDecimal) -> Amount {
    Amount::new(number, template.commodity.clone())
}

/// The running state of one aggregate call within one group.
enum Accumulator {
    Count(i64),
    SumInt(i64),
    SumDecimal(BigDecimal),
    SumInventory(Inventory),
    Pick(Option<Value>),
}

impl Accumulator {
    fn new(call: &AggregateCall) -> Self {
        match call.function.kind {
            AggregateKind::CountRows | AggregateKind::Count => Accumulator::Count(0),
            AggregateKind::SumInt => Accumulator::SumInt(0),
            AggregateKind::SumDecimal => Accumulator::SumDecimal(BigDecimal::zero()),
            AggregateKind::SumInventory => Accumulator::SumInventory(Inventory::new()),
            AggregateKind::First | AggregateKind::Last | AggregateKind::Min | AggregateKind::Max => Accumulator::Pick(None),
        }
    }

    fn update(&mut self, call: &AggregateCall, env: &Env<'_, '_>) -> Result<(), LocatedError> {
        let value = match &call.arg {
            Some(arg) => arg.eval(env)?,
            None => Value::Bool(true),
        };
        if value.is_null() {
            return Ok(());
        }
        match (self, value) {
            (Accumulator::Count(count), _) => *count += 1,
            (Accumulator::SumInt(sum), Value::Int(it)) => {
                *sum = sum.checked_add(it).ok_or_else(|| LocatedError::eval("integer overflow in sum()", None))?;
            }
            (Accumulator::SumDecimal(sum), value) => {
                if let Some(it) = value.as_decimal() {
                    *sum += it;
                }
            }
            (Accumulator::SumInventory(inventory), Value::Amount(it)) => inventory.add_amount(&it),
            (Accumulator::SumInventory(inventory), Value::Position(it)) => inventory.add_position(&it),
            (Accumulator::SumInventory(inventory), Value::Inventory(it)) => inventory.add_inventory(&it),
            (Accumulator::Pick(current), value) => {
                let replace = match (call.function.kind, current.as_ref()) {
                    (_, None) => true,
                    (AggregateKind::Last, Some(_)) => true,
                    (AggregateKind::Min, Some(current)) => value.sort_cmp(current) == Ordering::Less,
                    (AggregateKind::Max, Some(current)) => value.sort_cmp(current) == Ordering::Greater,
                    _ => false,
                };
                if replace {
                    *current = Some(value);
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn finish(self) -> Value {
        match self {
            Accumulator::Count(it) => Value::Int(it),
            Accumulator::SumInt(it) => Value::Int(it),
            Accumulator::SumDecimal(it) => Value::Decimal(it),
            Accumulator::SumInventory(it) => Value::Inventory(it),
            Accumulator::Pick(it) => it.unwrap_or(Value::Null),
        }
    }
}

fn passes(filter: &Option<CExpr>, env: &Env<'_, '_>) -> Result<bool, LocatedError> {
    match filter {
        None => Ok(true),
        Some(filter) => Ok(matches!(filter.eval(env)?, Value::Bool(true))),
    }
}

/// Run the plan and return the visible columns of the result rows.
pub(crate) fn execute(plan: &Plan, data: &Dataset<'_>, params: &Params) -> Result<Vec<Vec<Value>>, LocatedError> {
    let base = Env {
        data,
        row: None,
        aggregates: &[],
        params,
    };
    let mut rows: Vec<Vec<Value>> = vec![];

    match &plan.group_keys {
        None => {
            // without sorting or de-duplication LIMIT can stop the scan early
            let early_limit = if plan.order.is_empty() && !plan.distinct { plan.limit } else { None };
            for row in &data.rows {
                if early_limit.is_some_and(|limit| rows.len() as u64 >= limit) {
                    break;
                }
                let env = Env { row: Some(row), ..base };
                if !passes(&plan.filter, &env)? {
                    continue;
                }
                rows.push(plan.targets.iter().map(|target| target.expr.eval(&env)).collect::<Result<Vec<_>, _>>()?);
            }
        }
        Some(keys) => {
            let mut groups: IndexMap<Vec<Value>, Vec<Accumulator>> = IndexMap::new();
            for row in &data.rows {
                let env = Env { row: Some(row), ..base };
                if !passes(&plan.filter, &env)? {
                    continue;
                }
                let key = keys.iter().map(|idx| plan.targets[*idx].expr.eval(&env)).collect::<Result<Vec<_>, _>>()?;
                let accumulators = groups.entry(key).or_insert_with(|| plan.aggregates.iter().map(Accumulator::new).collect());
                for (call, accumulator) in plan.aggregates.iter().zip(accumulators.iter_mut()) {
                    accumulator.update(call, &env)?;
                }
            }
            for (key, accumulators) in groups {
                let finished = accumulators.into_iter().map(Accumulator::finish).collect::<Vec<_>>();
                let env = Env { aggregates: &finished, ..base };
                let mut out = Vec::with_capacity(plan.targets.len());
                for (idx, target) in plan.targets.iter().enumerate() {
                    match keys.iter().position(|key_idx| *key_idx == idx) {
                        Some(pos) => out.push(key[pos].clone()),
                        None => out.push(target.expr.eval(&env)?),
                    }
                }
                rows.push(out);
            }
        }
    }

    if !plan.order.is_empty() {
        rows.sort_by(|a, b| {
            for (idx, descending) in &plan.order {
                let ordering = a[*idx].sort_cmp(&b[*idx]);
                let ordering = if *descending { ordering.reverse() } else { ordering };
                if ordering != Ordering::Equal {
                    return ordering;
                }
            }
            Ordering::Equal
        });
    }
    for row in rows.iter_mut() {
        row.truncate(plan.visible);
    }
    if plan.distinct {
        let mut seen = HashSet::with_capacity(rows.len());
        rows.retain(|row| seen.insert(row.clone()));
    }
    if let Some(limit) = plan.limit {
        rows.truncate(usize::try_from(limit).unwrap_or(usize::MAX));
    }
    Ok(rows)
}
