//! Evaluation of a [`Plan`] over a [`Dataset`].

use std::cell::{Cell, RefCell};
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashSet};
use std::sync::OnceLock;
use std::time::{Duration as StdDuration, Instant};

use bigdecimal::{BigDecimal, Zero};
use chrono::{Duration, NaiveDate};
use indexmap::map::Entry;
use indexmap::IndexMap;
use regex::Regex;
use zhang_ast::amount::Amount;
use zhang_ast::Commodity;

use crate::compiler::{build_regex, AggregateCall, ArithOp, CExpr, CmpOp, ConstSet, LimitMode, Plan, RegexPattern, StrTest, StrTestKind, Window};
use crate::error::{LocatedError, QueryErrorKind, Span};
use crate::functions::{AccountDirectives, AggregateKind, FunctionContext, ScalarFunction};
use crate::params::Params;
use crate::prices::PriceMap;
use crate::projector::{borrowed_str, set_membership};
use crate::running::RunningState;
use crate::table::{Dataset, RowRef};
use crate::value::{calendar_value, Inventory, Position, Value};
use crate::{decimal, ColumnInfo};

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
    pub row: Option<RowRef<'e, 'a>>,
    /// finished aggregate values of the current group
    pub aggregates: &'e [Value],
    /// the targets of the current group's result row, which HAVING reads
    pub cells: &'e [Value],
    /// the running totals including the current row, when the plan reads them
    pub running: Option<&'e RunningState>,
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
        self.data.zip(self.row).and_then(|(data, row)| data.row_entry_meta(row, key))
    }

    fn posting_meta(&self, key: &str) -> Option<String> {
        self.impure.set(self.impure.get() || self.data.is_none());
        self.data.zip(self.row).and_then(|(data, row)| data.row_meta(row, key))
    }

    fn posting_meta_values(&self, key: &str) -> Vec<String> {
        self.impure.set(self.impure.get() || self.data.is_none());
        self.data.zip(self.row).map(|(data, row)| data.row_meta_values(row, key)).unwrap_or_default()
    }

    fn entry_meta_values(&self, key: &str) -> Vec<String> {
        self.impure.set(self.impure.get() || self.data.is_none());
        self.data
            .zip(self.row)
            .map(|(data, row)| data.row_entry_meta_values(row, key))
            .unwrap_or_default()
    }

    fn account_directives(&self, account: &str) -> Option<AccountDirectives<'_>> {
        self.impure.set(self.impure.get() || self.data.is_none());
        self.data?.account_directives(account)
    }

    fn commodity_directive(&self, currency: &str) -> Option<&Commodity> {
        self.impure.set(self.impure.get() || self.data.is_none());
        self.data?.commodity_directive(currency)
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
        cells: &[],
        running: None,
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
                    Ok(def.value(data, row))
                }
                _ => Err(LocatedError::eval(format!("column '{}' is not available here", def.name), None)),
            },
            CExpr::Running(total) => match env.running {
                Some(running) => Ok(Value::Inventory(running.value(*total, env.row))),
                None => {
                    // never constant: folding gives up on it
                    env.impure.set(true);
                    Err(LocatedError::eval(format!("{} is not available here", total.column()), None))
                }
            },
            CExpr::Param(param) => Ok(env.params.get(param).cloned().unwrap_or(Value::Null)),
            CExpr::Scalar { function, args, span } => eval_scalar(function, args, *span, env),
            CExpr::Aggregate(idx) => Ok(env.aggregates.get(*idx).cloned().unwrap_or(Value::Null)),
            CExpr::Target(idx) => match env.cells.get(*idx) {
                Some(value) => Ok(value.clone()),
                None => {
                    // never constant: folding gives up on it
                    env.impure.set(true);
                    Err(LocatedError::eval("a target is not available here", None))
                }
            },
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
            CExpr::InConst { needle, set, negated } => eval_in_const(needle, set, *negated, env),
            CExpr::StrTest { subject, test, .. } => eval_str_test(subject, test, env),
        }
    }
}

/// `x [NOT] IN <constants>` with the items hashed: a string needle is looked up in place.
fn eval_in_const(needle: &CExpr, set: &ConstSet, negated: bool, env: &Env<'_, '_>) -> Result<Value, LocatedError> {
    let found = match borrowed_str(needle, env) {
        Some(None) => return Ok(Value::Null),
        Some(Some(needle)) => set.contains_str(needle),
        None => match needle.eval(env)? {
            Value::Null => return Ok(Value::Null),
            needle => set.contains(&needle),
        },
    };
    Ok(if found {
        Value::Bool(!negated)
    } else if set.has_null() {
        Value::Null
    } else {
        Value::Bool(negated)
    })
}

/// A prepared `icontains`, `any_icontains` or `under` (see [`StrTestKind`]).
fn eval_str_test(subject: &CExpr, test: &StrTest, env: &Env<'_, '_>) -> Result<Value, LocatedError> {
    if let StrTestKind::AnyIContains(needle) = &test.kind {
        return Ok(match subject.eval(env)? {
            Value::Set(items) => Value::Bool(items.iter().any(|item| item.to_lowercase().contains(needle.as_str()))),
            _ => Value::Null,
        });
    }
    let matches = |text: &str| match &test.kind {
        StrTestKind::IContains(needle) => text.to_lowercase().contains(needle.as_str()),
        _ => crate::functions::is_under(text, &test.argument),
    };
    Ok(match borrowed_str(subject, env) {
        Some(text) => text.map_or(Value::Null, |text| Value::Bool(matches(text))),
        None => match subject.eval(env)? {
            Value::Str(text) => Value::Bool(matches(&text)),
            _ => Value::Null,
        },
    })
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
        // a date outside the calendar (years 1 to 9999) is NULL
        (Date(date), Int(days)) => {
            let shifted = Duration::try_days(days).and_then(|delta| match op {
                ArithOp::Add => date.checked_add_signed(delta),
                _ => date.checked_sub_signed(delta),
            });
            calendar_value(shifted)
        }
        (Int(days), Date(date)) => calendar_value(Duration::try_days(days).and_then(|delta| date.checked_add_signed(delta))),
        (Date(a), Date(b)) => Int((a - b).num_days()),
        (Date(date), Interval(interval)) => calendar_value(match op {
            ArithOp::Add => interval.add_to(date),
            _ => interval.subtract_from(date),
        }),
        (Interval(interval), Date(date)) => calendar_value(interval.add_to(date)),
        (Interval(a), Interval(b)) => Interval(
            match op {
                ArithOp::Add => a.checked_add(&b),
                _ => a.checked_sub(&b),
            }
            .ok_or("interval out of range")?,
        ),
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
    /// a deferred `first()` / `last()`: the ordinal (in the filtered rows) of the row whose
    /// value it picks, evaluated by the replay
    PickRow(Option<usize>),
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

    /// Remember the row a deferred `first()` / `last()` picks.
    fn pick_row(&mut self, call: &AggregateCall, ordinal: usize) {
        if let Accumulator::PickRow(picked) = self {
            if picked.is_none() || call.function.kind == AggregateKind::Last {
                *picked = Some(ordinal);
            }
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
            Accumulator::PickRow(_) => 1,
        }
    }

    fn finish(self) -> Value {
        match self {
            Accumulator::Count(it) => Value::Int(it),
            Accumulator::SumInt(it) => Value::Int(it),
            Accumulator::SumDecimal(it) => Value::Decimal(it),
            Accumulator::SumInventory(it) => Value::Inventory(it),
            Accumulator::Pick(it) => it.unwrap_or(Value::Null),
            Accumulator::PickRow(_) => unreachable!("the replay resolves deferred aggregates"),
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

    pub(crate) fn check(deadline: Option<&Deadline>, counter: usize) -> Result<(), LocatedError> {
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

    pub(crate) fn charge(&mut self, weight: u64) -> Result<(), LocatedError> {
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

    pub(crate) fn release(&mut self, weight: u64) {
        self.used = self.used.saturating_sub(weight);
    }

    /// Account for something that held `before` values and now holds `after`.
    pub(crate) fn change(&mut self, before: u64, after: u64) -> Result<(), LocatedError> {
        if after >= before {
            self.charge(after - before)
        } else {
            self.release(before - after);
            Ok(())
        }
    }
}

/// The size of a value in [`Budget`] values: one per value, plus one per position of a
/// position or inventory, per element of a set and per pair of metadata (`metas`), and per
/// 64 bytes of text (also the text of those elements and pairs), so that a budget bounds the
/// memory, and the encoded size, of a result.
pub(crate) fn weight(value: &Value) -> u64 {
    match value {
        Value::Str(text) => text_weight(text.len()),
        Value::Set(set) => 1 + set.iter().map(|item| text_weight(item.len())).sum::<u64>(),
        Value::Metas(pairs) => 1 + pairs.iter().map(|(key, value)| text_weight(key.len() + value.len())).sum::<u64>(),
        Value::Position(_) => 2,
        Value::Inventory(inventory) => inventory_weight(inventory),
        Value::Null | Value::Bool(_) | Value::Int(_) | Value::Decimal(_) | Value::Date(_) | Value::Amount(_) | Value::Interval(_) => 1,
    }
}

/// The size of a text of `len` bytes in [`Budget`] values (see [`weight`]); column names
/// built from the data are charged like text cells.
pub(crate) fn text_weight(len: usize) -> u64 {
    1 + len as u64 / 64
}

fn inventory_weight(inventory: &Inventory) -> u64 {
    1 + inventory.len() as u64
}

pub(crate) fn row_weight(row: &[Value]) -> u64 {
    row.iter().map(weight).sum()
}

/// One output row while it is being built: its cells, and for a plan that defers targets
/// the ordinal of its row in the filtered rows.
struct Built {
    cells: Vec<Value>,
    ordinal: usize,
    /// the index of the row in the dataset
    index: usize,
}

/// Rows ranked by the ORDER BY, ties broken by arrival, as a stable sort orders them.
struct Ranked<'p> {
    row: Built,
    arrival: usize,
    order: &'p [(usize, bool)],
}

fn order_cmp(order: &[(usize, bool)], a: &[Value], b: &[Value]) -> Ordering {
    for (idx, descending) in order {
        let ordering = a[*idx].sort_cmp(&b[*idx]);
        let ordering = if *descending { ordering.reverse() } else { ordering };
        if ordering != Ordering::Equal {
            return ordering;
        }
    }
    Ordering::Equal
}

impl Ord for Ranked<'_> {
    fn cmp(&self, other: &Self) -> Ordering {
        order_cmp(self.order, &self.row.cells, &other.row.cells).then(self.arrival.cmp(&other.arrival))
    }
}

impl PartialOrd for Ranked<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for Ranked<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Ranked<'_> {}

/// Where the main pass of a non-aggregate query puts its rows.
enum Collector<'p> {
    All(Vec<Built>),
    /// the first `k` rows of the ORDER BY, as a max-heap that drops its last row when full
    TopK {
        k: usize,
        heap: BinaryHeap<Ranked<'p>>,
        arrivals: usize,
    },
}

impl<'p> Collector<'p> {
    fn len(&self) -> usize {
        match self {
            Collector::All(rows) => rows.len(),
            Collector::TopK { heap, .. } => heap.len(),
        }
    }

    fn push(&mut self, row: Built, order: &'p [(usize, bool)], budget: &mut Budget) -> Result<(), LocatedError> {
        budget.charge(row_weight(&row.cells))?;
        match self {
            Collector::All(rows) => rows.push(row),
            Collector::TopK { k, heap, arrivals } => {
                heap.push(Ranked {
                    row,
                    arrival: *arrivals,
                    order,
                });
                *arrivals += 1;
                if heap.len() > *k {
                    if let Some(dropped) = heap.pop() {
                        budget.release(row_weight(&dropped.row.cells));
                    }
                }
            }
        }
        Ok(())
    }

    /// The rows, in ORDER BY order for a top-k.
    fn into_rows(self) -> Vec<Built> {
        match self {
            Collector::All(rows) => rows,
            Collector::TopK { heap, .. } => heap.into_sorted_vec().into_iter().map(|ranked| ranked.row).collect(),
        }
    }
}

/// The filtered rows of the main pass, in ledger order, kept for the replay.
struct Filtered {
    /// indexes into the dataset rows; only recorded when the plan replays
    rows: Option<Vec<usize>>,
    count: usize,
}

impl Filtered {
    /// Record a row that passed the filter; returns its ordinal.
    fn push(&mut self, idx: usize) -> usize {
        if let Some(rows) = &mut self.rows {
            rows.push(idx);
        }
        self.count += 1;
        self.count - 1
    }
}

/// Run the plan with its LIMIT and OFFSET and return the visible columns of the result rows,
/// without a [`Budget`].
#[cfg(test)]
pub(crate) fn execute(plan: &Plan, data: &Dataset<'_>, params: &Params, deadline: Option<Deadline>) -> Result<Vec<Vec<Value>>, LocatedError> {
    let run = Run {
        window: plan.window(params)?,
        count_total: false,
    };
    execute_within(plan, data, params, deadline, Budget::new(None), run).map(|output| output.rows)
}

/// What one execution asks of a plan besides its rows.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Run {
    /// LIMIT and OFFSET with their parameters bound ([`Plan::window`])
    pub window: Option<Window>,
    /// count the rows the query has before LIMIT and OFFSET ([`Output::total`])
    pub count_total: bool,
}

/// The rows of a result, and its columns when the data decides them (PIVOT BY).
pub(crate) struct Output {
    pub rows: Vec<Vec<Value>>,
    /// `None` when the columns are the plan's visible targets
    pub columns: Option<Vec<ColumnInfo>>,
    /// the number of rows before LIMIT and OFFSET (and before PIVOT BY), when
    /// [`Run::count_total`] asks for it
    pub total: Option<u64>,
}

/// The state shared by the passes of one execution.
struct Execution<'x, 'a> {
    plan: &'x Plan,
    data: &'x Dataset<'a>,
    base: Env<'x, 'a>,
    deadline: Option<&'x Deadline>,
}

impl<'x, 'a> Execution<'x, 'a> {
    /// Replay the filtered rows in ledger order up to the last of `needs` (ordinal, slot)
    /// pairs, sorted by ordinal, calling `visit` at each with the row and the running totals.
    fn replay(
        &self, filtered: &[usize], needs: &[(usize, usize)], mut visit: impl FnMut(usize, &Env<'_, 'a>) -> Result<(), LocatedError>,
    ) -> Result<(), LocatedError> {
        let Some(&(last, _)) = needs.last() else {
            return Ok(());
        };
        let mut running = RunningState::new(&self.plan.execution.running.totals);
        let mut next = 0;
        // the rows of the table added to the account balances so far: every row up to the
        // filtered one, whatever the filter
        let mut observed = 0;
        for (ordinal, idx) in filtered[..=last].iter().enumerate() {
            Deadline::check(self.deadline, ordinal)?;
            while running.observes() && observed <= *idx {
                Deadline::check(self.deadline, observed)?;
                if let RowRef::Posting(posting) = self.data.row(observed) {
                    running.observe(posting);
                }
                observed += 1;
            }
            let row = self.data.row(*idx);
            if let RowRef::Posting(posting) = row {
                running.add(posting);
            }
            let env = Env {
                row: Some(row),
                running: Some(&running),
                ..self.base
            };
            while next < needs.len() && needs[next].0 == ordinal {
                visit(needs[next].1, &env)?;
                next += 1;
            }
        }
        Ok(())
    }
}

/// Run the plan within the `budget` and return the visible columns of the result rows,
/// pivoted when the plan has a PIVOT BY.
///
/// The plan's [`crate::compiler::Execution`] says how: whether the main pass keeps the
/// running totals, which targets and aggregates over them wait for a replay of the filtered
/// rows (so that only the rows a query returns materialize a balance), and how LIMIT cuts
/// the scan short. LIMIT and OFFSET come from `run`: the execution keeps the first
/// OFFSET + LIMIT rows (or groups), then skips OFFSET of them.
///
/// With [`Run::count_total`] it also counts the rows before LIMIT and OFFSET without building
/// more of them than it would anyway: a scan that stops early goes on evaluating the filter
/// only (or, for DISTINCT, the targets that tell rows apart), and an aggregate query that
/// aggregates only its first groups goes on collecting the keys of the others.
pub(crate) fn execute_within(
    plan: &Plan, data: &Dataset<'_>, params: &Params, deadline: Option<Deadline>, mut budget: Budget, run: Run,
) -> Result<Output, LocatedError> {
    let regexes = RegexCache::default();
    let impure = Cell::new(false);
    let base = Env {
        data: Some(data),
        row: None,
        aggregates: &[],
        cells: &[],
        running: None,
        params,
        regexes: &regexes,
        impure: &impure,
    };
    let execution = Execution {
        plan,
        data,
        base,
        deadline: deadline.as_ref(),
    };
    let strategy = &plan.execution;
    let mut running = strategy.running.eager.then(|| RunningState::new(&strategy.running.totals));
    let mut filtered = Filtered {
        rows: strategy.running.replays().then(Vec::new),
        count: 0,
    };
    // the rows the execution keeps before skipping the offset
    let end = run.window.map(|window| usize::try_from(window.end()).unwrap_or(usize::MAX));

    // the number of rows before LIMIT and OFFSET when the strategy counts them on the way
    // (otherwise it is the number of rows left after DISTINCT), and whether LIMIT and OFFSET
    // were already applied
    let (mut rows, counted, windowed) = match &plan.group_keys {
        None => {
            let deferred = &strategy.running.deferred_targets;
            let late = &strategy.late_targets;
            let stop_at = end.filter(|_| strategy.limit == LimitMode::StopScan);
            let mut collector = match end {
                Some(k) if strategy.limit == LimitMode::TopK => Collector::TopK {
                    k,
                    heap: BinaryHeap::new(),
                    arrivals: 0,
                },
                _ => Collector::All(vec![]),
            };
            // DISTINCT without ORDER BY tells rows apart while scanning
            let mut seen = (plan.distinct && strategy.limit == LimitMode::StopScan).then(HashSet::new);
            // rows past the window of a scan that stops early, only counted
            let mut past_window = 0u64;
            for (counter, row) in data.iter().enumerate() {
                Deadline::check(execution.deadline, counter)?;
                let full = stop_at.is_some_and(|limit| collector.len() >= limit);
                if full && !run.count_total {
                    break;
                }
                if let (Some(running), RowRef::Posting(posting)) = (&mut running, row) {
                    running.observe(posting);
                }
                let env = Env {
                    row: Some(row),
                    running: running.as_ref(),
                    ..base
                };
                if !passes(&plan.filter, &env)? {
                    continue;
                }
                if full && seen.is_none() {
                    // every row that passes the filter is a result row
                    past_window += 1;
                    continue;
                }
                let ordinal = filtered.push(counter);
                if let (Some(running), RowRef::Posting(posting)) = (&mut running, row) {
                    running.add(posting);
                }
                let env = Env {
                    row: Some(row),
                    running: running.as_ref(),
                    ..base
                };
                let mut cells = Vec::with_capacity(plan.targets.len());
                for (idx, target) in plan.targets.iter().enumerate() {
                    cells.push(if deferred.contains(&idx) || late.contains(&idx) {
                        Value::Null
                    } else {
                        target.expr.eval(&env)?
                    });
                }
                if let Some(seen) = &mut seen {
                    let key = cells[..plan.visible].to_vec();
                    if full {
                        // past the window, DISTINCT rows are only told apart, to count them
                        if !seen.contains(&key) {
                            budget.charge(row_weight(&key))?;
                            seen.insert(key);
                        }
                        continue;
                    }
                    if !seen.insert(key) {
                        continue;
                    }
                }
                collector.push(
                    Built {
                        cells,
                        ordinal,
                        index: counter,
                    },
                    &plan.order,
                    &mut budget,
                )?;
            }
            let counted = match strategy.limit {
                LimitMode::StopScan => Some(match &seen {
                    Some(seen) => seen.len() as u64,
                    None => filtered.count as u64 + past_window,
                }),
                // every filtered row is ranked
                LimitMode::TopK => Some(filtered.count as u64),
                LimitMode::AfterSort | LimitMode::FirstGroups => None,
            };
            let ranked = matches!(collector, Collector::TopK { .. });
            let mut rows = collector.into_rows();
            if !plan.order.is_empty() && !ranked {
                rows.sort_by(|a, b| order_cmp(&plan.order, &a.cells, &b.cells));
            }
            if deferred.is_empty() && late.is_empty() {
                (rows.into_iter().map(|row| row.cells).collect::<Vec<_>>(), counted, false)
            } else {
                // the rows are chosen: LIMIT and OFFSET apply now (no DISTINCT defers), then
                // the late targets are built for the rows that are left, and the replay
                // evaluates their deferred targets
                let counted = Some(counted.unwrap_or(rows.len() as u64));
                if let Some(window) = run.window {
                    window.apply(&mut rows);
                }
                for (counter, row) in rows.iter_mut().enumerate() {
                    Deadline::check(execution.deadline, counter)?;
                    let env = Env {
                        row: Some(data.row(row.index)),
                        ..base
                    };
                    for idx in late {
                        let value = plan.targets[*idx].expr.eval(&env)?;
                        budget.change(weight(&row.cells[*idx]), weight(&value))?;
                        row.cells[*idx] = value;
                    }
                }
                if !deferred.is_empty() {
                    let mut needs = rows.iter().enumerate().map(|(slot, row)| (row.ordinal, slot)).collect::<Vec<_>>();
                    needs.sort_unstable();
                    let filtered_rows = filtered.rows.take().unwrap_or_default();
                    execution.replay(&filtered_rows, &needs, |slot, env| {
                        for idx in deferred {
                            let value = plan.targets[*idx].expr.eval(env)?;
                            budget.change(weight(&rows[slot].cells[*idx]), weight(&value))?;
                            rows[slot].cells[*idx] = value;
                        }
                        Ok(())
                    })?;
                }
                (rows.into_iter().map(|row| row.cells).collect::<Vec<_>>(), counted, true)
            }
        }
        Some(keys) => {
            let deferred = &strategy.running.deferred_aggregates;
            let first_groups = end.filter(|_| strategy.limit == LimitMode::FirstGroups);
            let mut groups: IndexMap<Vec<Value>, Vec<Accumulator>> = IndexMap::new();
            // the keys of the groups past the first ones, only collected to count them
            let mut later_groups: HashSet<Vec<Value>> = HashSet::new();
            for (counter, row) in data.iter().enumerate() {
                Deadline::check(execution.deadline, counter)?;
                if let (Some(running), RowRef::Posting(posting)) = (&mut running, row) {
                    running.observe(posting);
                }
                let env = Env {
                    row: Some(row),
                    running: running.as_ref(),
                    ..base
                };
                if !passes(&plan.filter, &env)? {
                    continue;
                }
                let ordinal = filtered.push(counter);
                if let (Some(running), RowRef::Posting(posting)) = (&mut running, row) {
                    running.add(posting);
                }
                let env = Env {
                    row: Some(row),
                    running: running.as_ref(),
                    ..base
                };
                let key = keys.iter().map(|idx| plan.targets[*idx].expr.eval(&env)).collect::<Result<Vec<_>, _>>()?;
                // LIMIT without ORDER BY keeps the first groups, so later ones are skipped
                let full = first_groups.is_some_and(|limit| groups.len() >= limit);
                let accumulators = match groups.entry(key) {
                    Entry::Occupied(entry) => entry.into_mut(),
                    Entry::Vacant(entry) if full => {
                        if run.count_total && !later_groups.contains(entry.key()) {
                            budget.charge(row_weight(entry.key()))?;
                            later_groups.insert(entry.into_key());
                        }
                        continue;
                    }
                    Entry::Vacant(entry) => {
                        let accumulators = plan
                            .aggregates
                            .iter()
                            .enumerate()
                            .map(|(idx, call)| {
                                if deferred.contains(&idx) {
                                    Accumulator::PickRow(None)
                                } else {
                                    Accumulator::new(call)
                                }
                            })
                            .collect::<Vec<_>>();
                        budget.charge(row_weight(entry.key()) + accumulators.iter().map(Accumulator::weight).sum::<u64>())?;
                        entry.insert(accumulators)
                    }
                };
                for (idx, (call, accumulator)) in plan.aggregates.iter().zip(accumulators.iter_mut()).enumerate() {
                    if deferred.contains(&idx) {
                        accumulator.pick_row(call, ordinal);
                        continue;
                    }
                    let before = accumulator.weight();
                    accumulator.update(call, &env)?;
                    budget.change(before, accumulator.weight())?;
                }
            }
            // only the first groups were built (no HAVING and no DISTINCT drop any)
            let counted = first_groups.map(|_| (groups.len() + later_groups.len()) as u64);
            drop(later_groups);
            if !deferred.is_empty() {
                // the replay evaluates every deferred first()/last() at the row it picks
                let mut needs = vec![];
                for (group, (_, accumulators)) in groups.iter().enumerate() {
                    for (idx, accumulator) in accumulators.iter().enumerate() {
                        if let Accumulator::PickRow(Some(ordinal)) = accumulator {
                            needs.push((*ordinal, group * plan.aggregates.len() + idx));
                        }
                    }
                }
                needs.sort_unstable();
                let filtered_rows = filtered.rows.take().unwrap_or_default();
                execution.replay(&filtered_rows, &needs, |slot, env| {
                    let (group, idx) = (slot / plan.aggregates.len(), slot % plan.aggregates.len());
                    let value = match &plan.aggregates[idx].arg {
                        Some(arg) => arg.eval(env)?,
                        None => Value::Null,
                    };
                    budget.change(1, weight(&value))?;
                    let (_, accumulators) = groups.get_index_mut(group).expect("a group");
                    accumulators[idx] = Accumulator::Pick((!value.is_null()).then_some(value));
                    Ok(())
                })?;
                // a group that never reached its deferred aggregate picks nothing
                for (_, accumulators) in groups.iter_mut() {
                    for accumulator in accumulators.iter_mut() {
                        if let Accumulator::PickRow(None) = accumulator {
                            *accumulator = Accumulator::Pick(None);
                        }
                    }
                }
            }
            let mut rows = Vec::with_capacity(groups.len());
            for (counter, (key, accumulators)) in groups.into_iter().enumerate() {
                Deadline::check(execution.deadline, counter)?;
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
                if let Some(having) = &plan.having {
                    // a group for which HAVING is not TRUE (FALSE or NULL) is dropped
                    if !matches!(having.eval(&Env { cells: &out, ..env })?, Value::Bool(true)) {
                        continue;
                    }
                }
                budget.charge(row_weight(&out))?;
                rows.push(out);
            }
            if !plan.order.is_empty() {
                rows.sort_by(|a, b| order_cmp(&plan.order, a, b));
            }
            (rows, counted, false)
        }
    };
    Deadline::check(execution.deadline, 0)?;

    for row in rows.iter_mut() {
        row.truncate(plan.visible);
    }
    if plan.distinct && strategy.limit != LimitMode::StopScan {
        let mut seen = HashSet::with_capacity(rows.len());
        rows.retain(|row| seen.insert(row.clone()));
    }
    let total = run.count_total.then(|| counted.unwrap_or(rows.len() as u64));
    if let (Some(window), false) = (run.window, windowed) {
        window.apply(&mut rows);
    }
    match plan.pivot {
        Some(spec) => {
            let (columns, rows) = crate::pivot::pivot(plan, spec, rows, &mut budget)?;
            Ok(Output {
                rows,
                columns: Some(columns),
                total,
            })
        }
        None => Ok(Output { rows, columns: None, total }),
    }
}
