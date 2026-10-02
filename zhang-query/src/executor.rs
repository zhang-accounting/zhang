//! Evaluation of a [`Plan`] over a [`Dataset`].

use std::cell::{Cell, RefCell};
use std::cmp::Ordering;
use std::collections::HashSet;
use std::sync::OnceLock;
use std::time::{Duration as StdDuration, Instant};

use bigdecimal::{BigDecimal, Zero};
use chrono::{Duration, NaiveDate};
use indexmap::map::Entry;
use indexmap::IndexMap;
use regex::Regex;
use zhang_ast::amount::Amount;

use crate::compiler::{build_regex, AggregateCall, ArithOp, CExpr, CmpOp, Plan, RegexPattern};
use crate::decimal;
use crate::error::{LocatedError, QueryErrorKind, Span};
use crate::functions::{AggregateKind, FunctionContext, ScalarFunction};
use crate::params::Params;
use crate::prices::PriceMap;
use crate::projector::{borrowed_str, set_membership};
use crate::table::{self, Dataset, Row};
use crate::value::{Inventory, Position, Value};

/// How many rows are scanned between two deadline checks.
const DEADLINE_CHECK_INTERVAL: usize = 256;

/// How many dynamic (per-row) regular expressions are kept compiled.
const REGEX_CACHE_SIZE: usize = 32;

/// Compiled regular expressions for patterns only known per row (e.g. `x ~ :pattern` or
/// `x ~ narration`), most recently used first.
#[derive(Default)]
pub(crate) struct RegexCache {
    entries: RefCell<Vec<(String, bool, Regex)>>,
}

impl RegexCache {
    fn get(&self, pattern: &str, case_insensitive: bool) -> Result<Regex, String> {
        let mut entries = self.entries.borrow_mut();
        if let Some(idx) = entries.iter().position(|(p, ci, _)| p == pattern && *ci == case_insensitive) {
            let entry = entries.remove(idx);
            let regex = entry.2.clone();
            entries.insert(0, entry);
            return Ok(regex);
        }
        let regex = build_regex(pattern, case_insensitive)?;
        entries.insert(0, (pattern.to_owned(), case_insensitive, regex.clone()));
        entries.truncate(REGEX_CACHE_SIZE);
        Ok(regex)
    }
}

/// The evaluation environment of one expression evaluation.
#[derive(Clone, Copy)]
pub(crate) struct Env<'e, 'a> {
    /// the rows and lookups of this execution; `None` while folding constants at compile time
    pub data: Option<&'e Dataset<'a>>,
    /// the current row; `None` when evaluating finished aggregates
    pub row: Option<&'e Row<'a>>,
    /// finished aggregate values of the current group
    pub aggregates: &'e [Value],
    /// the running balance including the current row, when the plan reads it
    pub balance: Option<&'e Inventory>,
    pub params: &'e Params,
    pub regexes: &'e RegexCache,
    /// set when an expression reads the execution context; folding then gives up
    pub impure: &'e Cell<bool>,
}

fn empty_prices() -> &'static PriceMap {
    static EMPTY: OnceLock<PriceMap> = OnceLock::new();
    EMPTY.get_or_init(PriceMap::default)
}

impl FunctionContext for Env<'_, '_> {
    fn today(&self) -> NaiveDate {
        match self.data {
            Some(data) => data.today,
            None => {
                self.impure.set(true);
                NaiveDate::default()
            }
        }
    }

    fn prices(&self) -> &PriceMap {
        match self.data {
            Some(data) => data.prices(),
            None => {
                self.impure.set(true);
                empty_prices()
            }
        }
    }

    fn entry_meta(&self, key: &str) -> Option<String> {
        self.impure.set(self.impure.get() || self.data.is_none());
        self.data.zip(self.row).and_then(|(data, row)| data.entry_meta(row, key))
    }

    fn posting_meta(&self, key: &str) -> Option<String> {
        self.impure.set(self.impure.get() || self.data.is_none());
        self.data.zip(self.row).and_then(|(data, row)| data.posting_meta(row, key))
    }
}

/// Evaluate a constant expression at compile time; `None` when it reads the execution
/// context (today, prices, metadata) or fails (the error is then raised when it runs).
pub(crate) fn eval_constant(expr: &CExpr) -> Option<Value> {
    let params = Params::new();
    let regexes = RegexCache::default();
    let impure = Cell::new(false);
    let env = Env {
        data: None,
        row: None,
        aggregates: &[],
        balance: None,
        params: &params,
        regexes: &regexes,
        impure: &impure,
    };
    let value = expr.eval(&env).ok()?;
    if impure.get() {
        None
    } else {
        Some(value)
    }
}

impl CExpr {
    /// Evaluate. This recurses once per tree level, so it only dispatches and keeps its stack
    /// frame small; the work of each node kind lives in separate functions.
    pub(crate) fn eval(&self, env: &Env<'_, '_>) -> Result<Value, LocatedError> {
        match self {
            CExpr::Const(value) => Ok(value.clone()),
            CExpr::Column(def) => match (env.data, env.row) {
                (Some(data), Some(row)) => {
                    debug_assert!(data.projection.contains(def), "column '{}' is not projected", def.name);
                    Ok((def.get)(data, row))
                }
                _ => Err(LocatedError::eval(format!("column '{}' is not available here", def.name), None)),
            },
            CExpr::RunningBalance => match env.balance {
                Some(balance) => Ok(Value::Inventory(balance.clone())),
                None => {
                    // never constant: folding gives up on it
                    env.impure.set(true);
                    Err(LocatedError::eval("balance is not available here", None))
                }
            },
            CExpr::Param(param) => Ok(env.params.get(param).cloned().unwrap_or(Value::Null)),
            CExpr::Scalar { function, args, span } => eval_scalar(function, args, *span, env),
            CExpr::Aggregate(idx) => Ok(env.aggregates.get(*idx).cloned().unwrap_or(Value::Null)),
            CExpr::WidenInt(inner) => Ok(widen_int(inner.eval(env)?)),
            CExpr::Neg(inner, span) => negate(inner.eval(env)?, *span),
            CExpr::Not(inner) => Ok(not(inner.eval(env)?)),
            CExpr::And(operands) => eval_logical(operands, false, env),
            CExpr::Or(operands) => eval_logical(operands, true, env),
            CExpr::Arith { first, rest } => {
                let mut value = first.eval(env)?;
                for step in rest {
                    let operand = step.operand.eval(env)?;
                    value = arithmetic(step.op, value, operand).map_err(|message| LocatedError::eval(message, Some(step.span)))?;
                }
                Ok(value)
            }
            CExpr::Compare { op, left, right } => {
                if let (Some(left), Some(right)) = (borrowed_str(left, env), borrowed_str(right, env)) {
                    return Ok(compare_str(*op, left, right));
                }
                let left = left.eval(env)?;
                let right = right.eval(env)?;
                Ok(compare(*op, &left, &right))
            }
            CExpr::Regex {
                subject,
                pattern,
                case_insensitive,
                negated,
                span,
                ..
            } => match borrowed_str(subject, env) {
                Some(subject) => eval_regex(subject, pattern, *case_insensitive, *negated, *span, env),
                None => match subject.eval(env)? {
                    Value::Str(subject) => eval_regex(Some(&subject), pattern, *case_insensitive, *negated, *span, env),
                    _ => Ok(Value::Null),
                },
            },
            CExpr::InSet { needle, set, negated } => match set_membership(set, env) {
                Some(contains) => {
                    let found = match borrowed_str(needle, env) {
                        Some(needle) => needle.map(&contains),
                        None => match needle.eval(env)? {
                            Value::Str(needle) => Some(contains(&needle)),
                            _ => None,
                        },
                    };
                    Ok(found.map_or(Value::Null, |found| Value::Bool(found != *negated)))
                }
                None => {
                    let needle = needle.eval(env)?;
                    let set = set.eval(env)?;
                    Ok(in_set(needle, set, *negated))
                }
            },
            CExpr::InList { needle, items, negated } => {
                let needle = needle.eval(env)?;
                eval_in_list(needle, items, *negated, env)
            }
            CExpr::IsNull { expr, negated } => Ok(Value::Bool(expr.eval(env)?.is_null() != *negated)),
        }
    }
}

fn eval_scalar(function: &ScalarFunction, args: &[CExpr], span: Span, env: &Env<'_, '_>) -> Result<Value, LocatedError> {
    let mut values = Vec::with_capacity(args.len());
    for arg in args {
        let value = arg.eval(env)?;
        if value.is_null() {
            return Ok(Value::Null);
        }
        values.push(value);
    }
    (function.eval)(&values, env).map_err(|message| LocatedError::eval(format!("{}(): {}", function.name, message), Some(span)))
}

fn widen_int(value: Value) -> Value {
    match value {
        Value::Int(it) => Value::Decimal(BigDecimal::from(it)),
        other => other,
    }
}

fn negate(value: Value, span: Span) -> Result<Value, LocatedError> {
    Ok(match value {
        Value::Null => Value::Null,
        Value::Int(it) => Value::Int(it.checked_neg().ok_or_else(|| LocatedError::eval("integer overflow", Some(span)))?),
        Value::Decimal(it) => Value::Decimal(-it),
        Value::Amount(it) => Value::Amount(-it),
        Value::Position(it) => Value::Position(-it),
        Value::Inventory(it) => Value::Inventory(-it),
        other => return Err(LocatedError::eval(format!("cannot negate {}", other.data_type()), Some(span))),
    })
}

/// three-valued logic: NOT NULL is NULL
fn not(value: Value) -> Value {
    match value {
        Value::Bool(it) => Value::Bool(!it),
        _ => Value::Null,
    }
}

/// n-ary `AND` (`absorbing` FALSE) or `OR` (`absorbing` TRUE) with three-valued logic,
/// evaluated left to right and stopping at the first absorbing operand.
fn eval_logical(operands: &[CExpr], absorbing: bool, env: &Env<'_, '_>) -> Result<Value, LocatedError> {
    let mut saw_null = false;
    for operand in operands {
        match operand.eval(env)? {
            Value::Bool(value) if value == absorbing => return Ok(Value::Bool(absorbing)),
            Value::Bool(_) => {}
            _ => saw_null = true,
        }
    }
    Ok(if saw_null { Value::Null } else { Value::Bool(!absorbing) })
}

fn compare(op: CmpOp, left: &Value, right: &Value) -> Value {
    if left.is_null() || right.is_null() {
        return Value::Null;
    }
    Value::Bool(match op {
        CmpOp::Eq => left == right,
        CmpOp::Ne => left != right,
        CmpOp::Lt => left.sort_cmp(right) == Ordering::Less,
        CmpOp::Le => left.sort_cmp(right) != Ordering::Greater,
        CmpOp::Gt => left.sort_cmp(right) == Ordering::Greater,
        CmpOp::Ge => left.sort_cmp(right) != Ordering::Less,
    })
}

/// `compare` for two strings read in place (see [`borrowed_str`]); `None` is NULL.
fn compare_str(op: CmpOp, left: Option<&str>, right: Option<&str>) -> Value {
    let (Some(left), Some(right)) = (left, right) else {
        return Value::Null;
    };
    Value::Bool(match op {
        CmpOp::Eq => left == right,
        CmpOp::Ne => left != right,
        CmpOp::Lt => left < right,
        CmpOp::Le => left <= right,
        CmpOp::Gt => left > right,
        CmpOp::Ge => left >= right,
    })
}

/// Match a string subject (`None` is NULL) against the pattern.
fn eval_regex(
    subject: Option<&str>, pattern: &RegexPattern, case_insensitive: bool, negated: bool, span: Span, env: &Env<'_, '_>,
) -> Result<Value, LocatedError> {
    let Some(subject) = subject else {
        return Ok(Value::Null);
    };
    let matched = match pattern {
        RegexPattern::Compiled(regex) => regex.is_match(subject),
        RegexPattern::Dynamic(expr) => {
            let Value::Str(pattern) = expr.eval(env)? else {
                return Ok(Value::Null);
            };
            env.regexes
                .get(&pattern, case_insensitive)
                .map_err(|message| LocatedError::eval(message, Some(span)))?
                .is_match(subject)
        }
    };
    Ok(Value::Bool(matched != negated))
}

fn in_set(needle: Value, set: Value, negated: bool) -> Value {
    match (needle, set) {
        (Value::Str(needle), Value::Set(set)) => Value::Bool(set.contains(&needle) != negated),
        _ => Value::Null,
    }
}

fn eval_in_list(needle: Value, items: &[CExpr], negated: bool, env: &Env<'_, '_>) -> Result<Value, LocatedError> {
    if needle.is_null() {
        return Ok(Value::Null);
    }
    let mut saw_null = false;
    for item in items {
        let item = item.eval(env)?;
        if item.is_null() {
            saw_null = true;
        } else if item == needle {
            return Ok(Value::Bool(!negated));
        }
    }
    Ok(if saw_null { Value::Null } else { Value::Bool(negated) })
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
                ArithOp::Mul => Decimal(decimal::mul(&a, &b)),
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
                ArithOp::Mul => Amount(amount_with(&amount, decimal::mul(&amount.number, &n))),
                _ => decimal::div(&amount.number, &n)
                    .map(|number| Amount(amount_with(&amount, number)))
                    .unwrap_or(Null),
            }
        }
        (n @ (Int(_) | Decimal(_)), Amount(amount)) => Amount(amount_with(&amount, decimal::mul(&n.as_decimal().expect("numeric"), &amount.number))),
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
            (Accumulator::SumInventory(inventory), Value::Amount(it)) => inventory.add_owned_position(Position::new(it, None)),
            (Accumulator::SumInventory(inventory), Value::Position(it)) => inventory.add_owned_position(it),
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

    /// What the accumulator holds, in [`Budget`] values.
    fn weight(&self) -> u64 {
        match self {
            Accumulator::Count(_) | Accumulator::SumInt(_) | Accumulator::SumDecimal(_) => 1,
            Accumulator::SumInventory(inventory) => inventory_weight(inventory),
            Accumulator::Pick(value) => value.as_ref().map_or(1, weight),
        }
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

/// A wall-clock limit for one execution, checked every [`DEADLINE_CHECK_INTERVAL`] rows.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Deadline {
    at: Instant,
    limit: StdDuration,
}

impl Deadline {
    pub fn after(limit: StdDuration) -> Self {
        Deadline {
            at: Instant::now() + limit,
            limit,
        }
    }

    fn check(deadline: Option<&Deadline>, counter: usize) -> Result<(), LocatedError> {
        match deadline {
            Some(deadline) if counter.is_multiple_of(DEADLINE_CHECK_INTERVAL) && Instant::now() >= deadline.at => Err(LocatedError {
                kind: QueryErrorKind::Timeout,
                message: format!("the query was stopped because it ran longer than the {:?} time limit", deadline.limit),
                span: None,
            }),
            _ => Ok(()),
        }
    }
}

/// How much of its result one execution may hold, counted in values (see [`weight`]):
/// produced rows, including the hidden ORDER BY / GROUP BY targets, the keys and
/// accumulators of the groups while they are being built, and the finished rows.
///
/// A result grows with rows × columns, but a cell can be as large as an inventory, and the
/// running `balance` holds every open lot: `JOURNAL` over a ledger with thousands of open
/// lots would build rows × lots positions. The budget stops such an execution early, with a
/// [`QueryErrorKind::TooLarge`] error, before it holds that much memory.
pub(crate) struct Budget {
    limit: Option<u64>,
    used: u64,
}

impl Budget {
    pub fn new(limit: Option<u64>) -> Self {
        Budget { limit, used: 0 }
    }

    fn charge(&mut self, weight: u64) -> Result<(), LocatedError> {
        self.used = self.used.saturating_add(weight);
        match self.limit {
            Some(limit) if self.used > limit => Err(LocatedError {
                kind: QueryErrorKind::TooLarge,
                message: format!(
                    "the result is too large: it would hold more than {} values (cells, plus the positions of inventories); \
                     narrow the query with FROM or WHERE, or add a LIMIT",
                    limit
                ),
                span: None,
            }),
            _ => Ok(()),
        }
    }

    fn release(&mut self, weight: u64) {
        self.used = self.used.saturating_sub(weight);
    }

    /// Account for something that held `before` values and now holds `after`.
    fn change(&mut self, before: u64, after: u64) -> Result<(), LocatedError> {
        if after >= before {
            self.charge(after - before)
        } else {
            self.release(before - after);
            Ok(())
        }
    }
}

/// The size of a value in [`Budget`] values: one per value, plus one per position of a
/// position or inventory, per element of a set and per 64 bytes of text, so that a budget
/// bounds the memory, and the encoded size, of a result.
pub(crate) fn weight(value: &Value) -> u64 {
    match value {
        Value::Str(text) => 1 + text.len() as u64 / 64,
        Value::Set(set) => 1 + set.len() as u64,
        Value::Position(_) => 2,
        Value::Inventory(inventory) => inventory_weight(inventory),
        _ => 1,
    }
}

fn inventory_weight(inventory: &Inventory) -> u64 {
    1 + inventory.len() as u64
}

fn row_weight(row: &[Value]) -> u64 {
    row.iter().map(weight).sum()
}

/// The running `balance`: the positions of the rows that passed the filter so far, in row
/// order. Like beanquery's, it is accumulated before rows are grouped, sorted, de-duplicated
/// or limited, so ORDER BY reorders rows without changing their balances.
struct RunningBalance {
    /// `None` when the plan does not read `balance`
    inventory: Option<Inventory>,
}

impl RunningBalance {
    fn new(plan: &Plan) -> Self {
        RunningBalance {
            inventory: plan.running_balance.then(Inventory::new),
        }
    }

    /// Add a row that passed the filter; returns the balance including it.
    fn add(&mut self, row: &Row<'_>) -> Option<&Inventory> {
        let inventory = self.inventory.as_mut()?;
        inventory.add_owned_position(table::position(row));
        Some(inventory)
    }
}

/// Run the plan and return the visible columns of the result rows, without a [`Budget`].
#[cfg(test)]
pub(crate) fn execute(plan: &Plan, data: &Dataset<'_>, params: &Params, deadline: Option<Deadline>) -> Result<Vec<Vec<Value>>, LocatedError> {
    execute_within(plan, data, params, deadline, Budget::new(None))
}

/// Run the plan within the `budget` and return the visible columns of the result rows.
pub(crate) fn execute_within(
    plan: &Plan, data: &Dataset<'_>, params: &Params, deadline: Option<Deadline>, mut budget: Budget,
) -> Result<Vec<Vec<Value>>, LocatedError> {
    let regexes = RegexCache::default();
    let impure = Cell::new(false);
    let mut running = RunningBalance::new(plan);
    let base = Env {
        data: Some(data),
        row: None,
        aggregates: &[],
        balance: None,
        params,
        regexes: &regexes,
        impure: &impure,
    };
    let deadline = deadline.as_ref();
    let mut rows: Vec<Vec<Value>> = vec![];

    match &plan.group_keys {
        None => {
            // without sorting or de-duplication LIMIT can stop the scan early
            let early_limit = if plan.order.is_empty() && !plan.distinct { plan.limit } else { None };
            for (counter, row) in data.rows.iter().enumerate() {
                Deadline::check(deadline, counter)?;
                if early_limit.is_some_and(|limit| rows.len() as u64 >= limit) {
                    break;
                }
                let env = Env { row: Some(row), ..base };
                if !passes(&plan.filter, &env)? {
                    continue;
                }
                let env = Env {
                    balance: running.add(row),
                    ..env
                };
                let values = plan.targets.iter().map(|target| target.expr.eval(&env)).collect::<Result<Vec<_>, _>>()?;
                budget.charge(row_weight(&values))?;
                rows.push(values);
            }
        }
        Some(keys) => {
            let mut groups: IndexMap<Vec<Value>, Vec<Accumulator>> = IndexMap::new();
            for (counter, row) in data.rows.iter().enumerate() {
                Deadline::check(deadline, counter)?;
                let env = Env { row: Some(row), ..base };
                if !passes(&plan.filter, &env)? {
                    continue;
                }
                let env = Env {
                    balance: running.add(row),
                    ..env
                };
                let key = keys.iter().map(|idx| plan.targets[*idx].expr.eval(&env)).collect::<Result<Vec<_>, _>>()?;
                let accumulators = match groups.entry(key) {
                    Entry::Occupied(entry) => entry.into_mut(),
                    Entry::Vacant(entry) => {
                        let accumulators = plan.aggregates.iter().map(Accumulator::new).collect::<Vec<_>>();
                        budget.charge(row_weight(entry.key()) + accumulators.iter().map(Accumulator::weight).sum::<u64>())?;
                        entry.insert(accumulators)
                    }
                };
                for (call, accumulator) in plan.aggregates.iter().zip(accumulators.iter_mut()) {
                    let before = accumulator.weight();
                    accumulator.update(call, &env)?;
                    budget.change(before, accumulator.weight())?;
                }
            }
            for (counter, (key, accumulators)) in groups.into_iter().enumerate() {
                Deadline::check(deadline, counter)?;
                // the group becomes a row
                budget.release(row_weight(&key) + accumulators.iter().map(Accumulator::weight).sum::<u64>());
                let finished = accumulators.into_iter().map(Accumulator::finish).collect::<Vec<_>>();
                let env = Env { aggregates: &finished, ..base };
                let mut out = Vec::with_capacity(plan.targets.len());
                for (idx, target) in plan.targets.iter().enumerate() {
                    match keys.iter().position(|key_idx| *key_idx == idx) {
                        Some(pos) => out.push(key[pos].clone()),
                        None => out.push(target.expr.eval(&env)?),
                    }
                }
                budget.charge(row_weight(&out))?;
                rows.push(out);
            }
        }
    }
    Deadline::check(deadline, 0)?;

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
