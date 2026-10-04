//! The rule-based optimizer: rewrites a compiled [`Plan`] before it is executed.
//!
//! It runs between compile/bind and execute. [`optimize`] merges the row filters and then
//! rewrites every expression of the plan bottom-up: once a node's children are optimized,
//! each rule below gets a chance to replace the node. Every rule is a small local rewrite
//! with its own tests:
//!
//! 1. [`flatten_associative`]: nested `AND`/`OR` become one n-ary node, and an arithmetic
//!    chain whose first operand is itself a chain is spliced into it, so parenthesised long
//!    lists stay flat for evaluation.
//! 2. [`eliminate_double_negation`]: `NOT NOT x` is `x` (also for NULL).
//! 3. [`simplify_logic`]: drops neutral operands and short-circuits absorbing ones under
//!    three-valued logic: `TRUE AND x` is `x`, `x AND FALSE` is `FALSE` (even when x is NULL),
//!    `x OR TRUE` is `TRUE`, `FALSE OR x` is `x`; `NULL` operands are kept, because
//!    `x OR NULL` is not `x`. A `CASE` loses the branches whose condition is a constant that
//!    is not TRUE, and ends at the first whose condition is the constant TRUE.
//! 4. [`fold_constants`]: a node whose operands are all constants is evaluated once at compile
//!    time, unless it depends on the execution (`today()`, prices, row metadata). The running
//!    `balance` is state of the execution and is never folded.
//! 5. [`precompile_regex`]: a match against a constant pattern compiles the regular
//!    expression once; invalid patterns are compile errors at the pattern.
//! 6. [`prepare_membership`]: `x IN (a, b, ...)` with constant items, and `x IN <set>` with a
//!    constant set, become one hash lookup ([`CExpr::InConst`]) instead of comparing every
//!    item, or copying the set, for every row.
//! 7. [`prepare_str_test`]: `icontains(x, 'needle')`, `any_icontains(set, 'needle')` and
//!    `under(account, 'Ancestor')` prepare their constant once ([`CExpr::StrTest`]): the
//!    needle is lower-cased once, and the subject is read in place.
//!
//! Parameters are constants of one execution: [`bind`] replaces every parameter by its bound
//! value when a query is executed and runs these rules again, so that a pattern or a set
//! passed as a parameter is compiled or hashed once per execution, like a literal.
//! [`optimize_naive`] skips rules 6 and 7 and binding, so tests can check that they never
//! change a result.
//!
//! Then [`plan_execution`] makes decisions about the plan as a whole:
//!
//! 8. [`rewrite_linear_balance`]: `units(balance)` and `cost(balance)` are linear in the
//!    positions, so they become running sums of `units(position)` / `cost(position)` per
//!    currency ([`Running::Units`], [`Running::Cost`]), which cost O(currencies) per row
//!    instead of O(open lots). `value()` and `convert()` price by date and are not linear. The
//!    same goes for `units(account_balance)` and `cost(account_balance)`, summed per account
//!    ([`Running::AccountUnits`], [`Running::AccountCost`]).
//! 9. [`limit_mode`]: how LIMIT cuts the work short. Without ORDER BY a scan stops once it has
//!    LIMIT rows (telling DISTINCT rows apart while scanning) and an aggregate query only
//!    aggregates its first LIMIT groups (unless HAVING may drop some of them), or, when the rows
//!    of each group come one after another, only the groups of its window, past OFFSET; with
//!    ORDER BY (and no DISTINCT) the scan keeps the top LIMIT rows instead of sorting them all.
//! 10. [`account_scope`]: when the filter of a `postings` query can only hold for the rows of
//!     some accounts (`account = :account`), the execution only builds their rows.
//!
//! The expression rules rewrite the HAVING condition like any other expression; one that
//! folds to TRUE is dropped, like a filter.

use std::cell::RefCell;
use std::sync::Arc;

use regex::Regex;

use crate::compiler::{
    build_regex, AccountScope, BoundValue, CExpr, CmpOp, ConstSet, DateBound, LimitMode, Plan, RegexPattern, Running, ScopeValue, ScopedAccount, StrTest,
    StrTestKind, UnderAncestor,
};
use crate::error::LocatedError;
use crate::executor::eval_constant;
use crate::functions::ParamType;
use crate::params::Params;
use crate::projector::infallible;
use crate::table::Rows;
use crate::value::{DataType, Value};

/// Scalar functions that depend on the execution (its date, the ledger's prices and
/// directives, the current row) and are therefore never folded, even with constant arguments.
const NOT_FOLDABLE: &[&str] = &[
    "today",
    "meta",
    "entry_meta",
    "any_meta",
    "meta_values",
    "entry_meta_values",
    "convert",
    "value",
    "getprice",
    "open_date",
    "close_date",
    "open_meta",
    "account_budgets",
    "commodity_meta",
    "currency_meta",
];

/// Which expression rules run.
#[derive(Clone, Copy)]
struct Rules<'p> {
    /// prepare constants for the executor: [`prepare_membership`] and [`prepare_str_test`]
    prepare: bool,
    /// an invalid constant pattern is a compile error; otherwise the match is left to fail
    /// when it is evaluated (for parameters, which are bound after compiling)
    strict_regex: bool,
    /// the patterns compiled so far, so a pattern used several times (a keyword matched
    /// against several columns) is compiled once
    patterns: Option<&'p Patterns>,
}

/// Regular expressions compiled while binding, by pattern and case sensitivity.
#[derive(Default)]
struct Patterns(RefCell<Vec<(String, bool, Regex)>>);

impl Patterns {
    fn get(&self, pattern: &str, case_insensitive: bool) -> Result<Regex, String> {
        if let Some((_, _, regex)) = self.0.borrow().iter().find(|(p, ci, _)| p == pattern && *ci == case_insensitive) {
            return Ok(regex.clone());
        }
        let regex = build_regex(pattern, case_insensitive)?;
        self.0.borrow_mut().push((pattern.to_owned(), case_insensitive, regex.clone()));
        Ok(regex)
    }
}

impl Rules<'_> {
    const COMPILE: Rules<'static> = Rules {
        prepare: true,
        strict_regex: true,
        patterns: None,
    };
    #[cfg(test)]
    const NAIVE: Rules<'static> = Rules {
        prepare: false,
        strict_regex: true,
        patterns: None,
    };
}

/// Optimize a compiled plan: rewrite its expressions, then [`plan_execution`].
pub(crate) fn optimize(plan: Plan) -> Result<Plan, LocatedError> {
    let mut plan = optimize_expressions(plan, Rules::COMPILE)?;
    plan_execution(&mut plan);
    Ok(plan)
}

/// Only the expression rules that change no work per row, leaving the naive execution the
/// compiler chose: the reference the decisions of [`plan_execution`], the prepared constants
/// and [`bind`] are tested against.
#[cfg(test)]
pub(crate) fn optimize_naive(plan: Plan) -> Result<Plan, LocatedError> {
    optimize_expressions(plan, Rules::NAIVE)
}

/// The plan of one execution: every parameter replaced by the value bound to it, and the
/// expression rules run again over the constants this makes (a regular expression, an `IN`
/// set or a needle given as a parameter is then compiled, hashed or lower-cased once instead
/// of once per row). The execution decisions of the plan are kept: binding only turns
/// parameters into constants, so they still hold.
pub(crate) fn bind(plan: &Plan, params: &Params) -> Plan {
    let mut plan = plan.clone();
    let patterns = Patterns::default();
    let rules = Rules {
        prepare: true,
        strict_regex: false,
        patterns: Some(&patterns),
    };
    let bind_expr = |expr: CExpr| optimize_with(substitute(expr, params), rules).expect("binding raises no compile error");
    plan.filter = plan.filter.take().map(bind_expr);
    if matches!(plan.filter, Some(CExpr::Const(Value::Bool(true)))) {
        plan.filter = None;
    }
    plan.having = plan.having.take().map(bind_expr);
    if matches!(plan.having, Some(CExpr::Const(Value::Bool(true)))) {
        plan.having = None;
    }
    for target in &mut plan.targets {
        let expr = std::mem::replace(&mut target.expr, CExpr::Const(Value::Null));
        target.expr = bind_expr(expr);
    }
    for aggregate in &mut plan.aggregates {
        if let Some(arg) = aggregate.arg.take() {
            aggregate.arg = Some(bind_expr(arg));
        }
    }
    plan
}

/// Replace every parameter by its bound value (NULL when unbound, as when evaluated).
fn substitute(expr: CExpr, params: &Params) -> CExpr {
    match expr {
        CExpr::Param(param) => CExpr::Const(params.get(&param).cloned().unwrap_or(Value::Null)),
        other => other
            .map_children(&mut |child| Ok::<_, std::convert::Infallible>(substitute(child, params)))
            .unwrap_or_else(|never| match never {}),
    }
}

/// The decisions about the plan as a whole: [`rewrite_linear_balance`], [`limit_mode`] and
/// [`account_scope`].
pub(crate) fn plan_execution(plan: &mut Plan) {
    let mut rewrites = vec![];
    for target in &mut plan.targets {
        let expr = std::mem::replace(&mut target.expr, CExpr::Const(Value::Null));
        target.expr = rewrite_linear_balance(expr, &mut rewrites);
    }
    for aggregate in &mut plan.aggregates {
        if let Some(arg) = aggregate.arg.take() {
            aggregate.arg = Some(rewrite_linear_balance(arg, &mut rewrites));
        }
    }
    // the filter may read the account balances
    if let Some(filter) = plan.filter.take() {
        plan.filter = Some(rewrite_linear_balance(filter, &mut rewrites));
    }
    rewrites.sort();
    rewrites.dedup();
    plan.execution.rewrites = rewrites;
    plan.execution.limit = limit_mode(plan);
    plan.execution.scope = account_scope(plan);
    plan.execution.until = date_bound(plan);
}

/// The last date the rows of a table that generates its rows (the months of `#budgets`) need
/// to reach: a conjunct of the filter that only holds for rows dated up to a date,
///
/// - `date <= x`, `date < x` or `date = x` (`x` a date constant or parameter, `today()` or
///   `yearmonth(today())`, on either side),
/// - `yearmonth(date) <= x`, `yearmonth(date) < x` or `yearmonth(date) = x`.
///
/// The rows after it fail the filter, so not generating them changes no result, as an account
/// scope does for the postings ([`account_scope`]), and with the same rule: a conjunct before
/// the bounding one that may fail keeps every row. Generating up to a date the query asks for,
/// rather than through the end of the ledger, keeps a query of this month working when a date
/// typo dates an entry centuries ahead.
pub(crate) fn date_bound(plan: &Plan) -> Option<DateBound> {
    if !matches!(plan.table.rows, Rows::Generated(_)) {
        return None;
    }
    let conjuncts = match plan.filter.as_ref()? {
        CExpr::And(operands) => operands.as_slice(),
        filter => std::slice::from_ref(filter),
    };
    for conjunct in conjuncts {
        if let Some(bound) = bounding_date(conjunct) {
            return Some(bound);
        }
        if !infallible(conjunct) {
            return None;
        }
    }
    None
}

/// The date bound of a filter conjunct (see [`date_bound`]).
fn bounding_date(expr: &CExpr) -> Option<DateBound> {
    let is_date = |expr: &CExpr| matches!(expr, CExpr::Column(column) if column.name == "date");
    let is_month = |expr: &CExpr| matches!(expr, CExpr::Scalar { function, args, .. } if function.name == "yearmonth" && matches!(args.as_slice(), [date] if is_date(date)));
    let is_today = |expr: &CExpr| matches!(expr, CExpr::Scalar { function, args, .. } if function.name == "today" && args.is_empty());
    let bound = |expr: &CExpr| match expr {
        CExpr::Const(value @ (Value::Date(_) | Value::Null)) => Some(BoundValue::Const(value.clone())),
        CExpr::Param(param) => Some(BoundValue::Param(param.clone())),
        // `yearmonth(today())` is no later than today, so today bounds it too
        CExpr::Scalar { function, args, .. } if function.name == "yearmonth" && matches!(args.as_slice(), [today] if is_today(today)) => {
            Some(BoundValue::Today)
        }
        expr if is_today(expr) => Some(BoundValue::Today),
        _ => None,
    };
    let CExpr::Compare { op, left, right } = expr else {
        return None;
    };
    // `subject op value`, with the subject on the left
    let (subject, op, value) = if is_date(left) || is_month(left) {
        (left, *op, right)
    } else if is_date(right) || is_month(right) {
        (right, op.flipped(), left)
    } else {
        return None;
    };
    let exclusive = match op {
        CmpOp::Eq | CmpOp::Le => false,
        CmpOp::Lt => true,
        _ => return None,
    };
    Some(DateBound {
        value: bound(value)?,
        exclusive,
        month: is_month(subject),
    })
}

/// The accounts a `postings` query's rows can be limited to: those named by a conjunct of the
/// filter (FROM, then WHERE) that only holds for the rows of some accounts:
///
/// - `account = x` (a string constant or parameter, on either side),
/// - `account IN (x, y, ...)` (also once its constant items are hashed) and `account IN :set`,
/// - `under(account, x)` (`x` and its sub-accounts),
/// - an `OR` of these.
///
/// The rows of other accounts fail the filter, so leaving them out changes no result: the
/// filter still runs on the rows of the scope, which keeps every row of its accounts, in ledger
/// order. The running `balance` adds up the same rows, and `account_balance` and the lots only
/// depend on the rows of their own account. Two cases keep every row:
///
/// - the period modifiers (`OPEN ON`, `CLOSE ON`, `CLEAR`), which move balances to other
///   accounts before the filter runs;
/// - a conjunct before the scoping one that may fail with an error: the full scan evaluates it
///   for the rows of other accounts too, and would stop with that error.
pub(crate) fn account_scope(plan: &Plan) -> Option<AccountScope> {
    if !plan.table.is_postings() || plan.period.is_some() {
        return None;
    }
    let conjuncts = match plan.filter.as_ref()? {
        CExpr::And(operands) => operands.as_slice(),
        filter => std::slice::from_ref(filter),
    };
    for conjunct in conjuncts {
        if let Some(accounts) = scoped_accounts(conjunct) {
            return Some(AccountScope { accounts });
        }
        if !infallible(conjunct) {
            return None;
        }
    }
    None
}

/// The accounts a filter conjunct can only hold for (see [`account_scope`]).
///
/// To scope by another predicate on the account, add an arm here and, when it names accounts
/// in a new way, a [`ScopedAccount`] variant.
fn scoped_accounts(expr: &CExpr) -> Option<Vec<ScopedAccount>> {
    let is_account = |expr: &CExpr| matches!(expr, CExpr::Column(column) if column.name == "account");
    // a value naming accounts: a string or NULL, or a string parameter
    let named = |expr: &CExpr| match expr {
        CExpr::Const(value @ (Value::Str(_) | Value::Null)) => Some(ScopeValue::Const(value.clone())),
        CExpr::Param(param) => Some(ScopeValue::Param(param.clone())),
        _ => None,
    };
    match expr {
        CExpr::Compare { op: CmpOp::Eq, left, right } => {
            let value = match (is_account(left), is_account(right)) {
                (true, false) => right,
                (false, true) => left,
                _ => return None,
            };
            Some(vec![ScopedAccount::Named(named(value)?)])
        }
        CExpr::InList { needle, items, negated: false } if is_account(needle) => items.iter().map(|item| named(item).map(ScopedAccount::Named)).collect(),
        CExpr::InSet { needle, set, negated: false } if is_account(needle) => match set.as_ref() {
            CExpr::Const(value @ (Value::Set(_) | Value::Null)) => Some(vec![ScopedAccount::Named(ScopeValue::Const(value.clone()))]),
            CExpr::Param(param) => Some(vec![ScopedAccount::Named(ScopeValue::Param(param.clone()))]),
            _ => None,
        },
        // `account IN (...)` with constant items, prepared into a hash lookup
        CExpr::InConst { needle, set, negated: false } if is_account(needle) => set
            .values()
            .into_iter()
            .map(|value| match value {
                Value::Str(_) | Value::Set(_) | Value::Null => Some(ScopedAccount::Named(ScopeValue::Const(value))),
                _ => None,
            })
            .collect(),
        // `under(account, ancestor)`, as a call or as its prepared string test
        expr if expr.as_under().is_some() => {
            let (account, ancestor) = expr.as_under()?;
            if !is_account(account) {
                return None;
            }
            Some(vec![ScopedAccount::Under(match ancestor {
                UnderAncestor::Const(name) => ScopeValue::Const(name.map_or(Value::Null, Value::from)),
                UnderAncestor::Param(param) => ScopeValue::Param(param.clone()),
            })])
        }
        CExpr::Or(operands) => {
            let mut accounts = vec![];
            for operand in operands {
                accounts.extend(scoped_accounts(operand)?);
            }
            Some(accounts)
        }
        _ => None,
    }
}

/// `units(balance)` → [`Running::Units`] and `cost(balance)` → [`Running::Cost`] (and the
/// same per account for `account_balance`), bottom-up, recording each rewrite.
pub(crate) fn rewrite_linear_balance(expr: CExpr, rewrites: &mut Vec<Running>) -> CExpr {
    let expr = expr
        .map_children(&mut |child| Ok::<_, std::convert::Infallible>(rewrite_linear_balance(child, rewrites)))
        .unwrap_or_else(|never| match never {});
    match expr {
        CExpr::Scalar { function, args, span }
            if function.params == [ParamType::Exact(DataType::Inventory)]
                && matches!(args.as_slice(), [CExpr::Running(Running::Balance | Running::AccountBalance)]) =>
        {
            let per_account = matches!(args.as_slice(), [CExpr::Running(Running::AccountBalance)]);
            let total = match (function.name, per_account) {
                ("units", false) => Running::Units,
                ("cost", false) => Running::Cost,
                ("units", true) => Running::AccountUnits,
                ("cost", true) => Running::AccountCost,
                _ => return CExpr::Scalar { function, args, span },
            };
            rewrites.push(total);
            CExpr::Running(total)
        }
        other => other,
    }
}

/// How LIMIT can cut the work short (see the module docs).
pub(crate) fn limit_mode(plan: &Plan) -> LimitMode {
    match (&plan.group_keys, plan.order.is_empty(), plan.distinct, plan.limit.is_some()) {
        (None, true, _, _) => LimitMode::StopScan,
        (None, false, false, true) => LimitMode::TopK,
        // HAVING may drop any group once it is finished, so every group is built
        (Some(_), true, false, true) if plan.having.is_none() => LimitMode::FirstGroups,
        _ => LimitMode::AfterSort,
    }
}

fn optimize_expressions(mut plan: Plan, rules: Rules<'_>) -> Result<Plan, LocatedError> {
    let optimize_expr = |expr: CExpr| optimize_with(expr, rules);
    plan.filter = merge_filters(std::mem::take(&mut plan.filters)).map(optimize_expr).transpose()?;
    if matches!(plan.filter, Some(CExpr::Const(Value::Bool(true)))) {
        // a filter that always holds is no filter
        plan.filter = None;
    }
    plan.having = plan.having.take().map(optimize_expr).transpose()?;
    if matches!(plan.having, Some(CExpr::Const(Value::Bool(true)))) {
        // nor is a HAVING that holds for every group
        plan.having = None;
    }
    for target in &mut plan.targets {
        let expr = std::mem::replace(&mut target.expr, CExpr::Const(Value::Null));
        target.expr = optimize_expr(expr)?;
    }
    for aggregate in &mut plan.aggregates {
        if let Some(arg) = aggregate.arg.take() {
            aggregate.arg = Some(optimize_expr(arg)?);
        }
    }
    Ok(plan)
}

/// Optimize one expression bottom-up. The recursion follows the tree, whose height the parser
/// bounds by [`crate::MAX_DEPTH`].
#[cfg(test)]
pub(crate) fn optimize_expr(expr: CExpr) -> Result<CExpr, LocatedError> {
    optimize_with(expr, Rules::COMPILE)
}

fn optimize_with(expr: CExpr, rules: Rules<'_>) -> Result<CExpr, LocatedError> {
    let expr = expr.map_children(&mut |child| optimize_with(child, rules))?;
    let expr = flatten_associative(expr);
    let expr = eliminate_double_negation(expr);
    let expr = simplify_logic(expr);
    let expr = fold_constants(expr);
    let expr = if rules.strict_regex {
        precompile_regex(expr)?
    } else {
        precompile_valid_regex(expr, rules.patterns.unwrap_or(&Patterns::default()))
    };
    if !rules.prepare {
        return Ok(expr);
    }
    Ok(prepare_str_test(prepare_membership(expr)))
}

/// The FROM expression and WHERE as one filter.
pub(crate) fn merge_filters(mut filters: Vec<CExpr>) -> Option<CExpr> {
    match filters.len() {
        0 => None,
        1 => filters.pop(),
        _ => Some(CExpr::And(filters)),
    }
}

/// Splice nested `AND`/`OR` into their parent, and an arithmetic chain that starts with a
/// chain into one chain (chains evaluate left to right, so `(a - b) * c` keeps its meaning).
pub(crate) fn flatten_associative(expr: CExpr) -> CExpr {
    match expr {
        CExpr::And(operands) => CExpr::And(splice(operands, |operand| match operand {
            CExpr::And(inner) => Ok(inner),
            other => Err(other),
        })),
        CExpr::Or(operands) => CExpr::Or(splice(operands, |operand| match operand {
            CExpr::Or(inner) => Ok(inner),
            other => Err(other),
        })),
        CExpr::Arith { first, rest } => match *first {
            CExpr::Arith {
                first: inner_first,
                rest: mut inner_rest,
            } => {
                inner_rest.extend(rest);
                CExpr::Arith {
                    first: inner_first,
                    rest: inner_rest,
                }
            }
            first => CExpr::Arith { first: Box::new(first), rest },
        },
        other => other,
    }
}

fn splice(operands: Vec<CExpr>, unwrap: impl Fn(CExpr) -> Result<Vec<CExpr>, CExpr>) -> Vec<CExpr> {
    let mut flat = Vec::with_capacity(operands.len());
    for operand in operands {
        match unwrap(operand) {
            Ok(inner) => flat.extend(inner),
            Err(operand) => flat.push(operand),
        }
    }
    flat
}

/// `NOT NOT x` is `x`; with three-valued logic this also holds for NULL.
pub(crate) fn eliminate_double_negation(expr: CExpr) -> CExpr {
    match expr {
        CExpr::Not(inner) => match *inner {
            CExpr::Not(x) => *x,
            inner => CExpr::Not(Box::new(inner)),
        },
        other => other,
    }
}

/// Drop neutral constant operands of `AND`/`OR` and short-circuit absorbing ones, following
/// three-valued logic (`NULL` operands are never dropped). Drop the branches of a `CASE` whose
/// condition is a constant that is not TRUE, and end it at the first one that is TRUE.
pub(crate) fn simplify_logic(expr: CExpr) -> CExpr {
    let is_bool = |expr: &CExpr, value: bool| matches!(expr, CExpr::Const(Value::Bool(it)) if *it == value);
    match expr {
        CExpr::Case { branches, otherwise } => {
            let mut kept = Vec::with_capacity(branches.len());
            for (condition, value) in branches {
                match condition {
                    CExpr::Const(Value::Bool(true)) if kept.is_empty() => return value,
                    CExpr::Const(Value::Bool(true)) => {
                        return CExpr::Case {
                            branches: kept,
                            otherwise: Box::new(value),
                        }
                    }
                    // FALSE or NULL: never chosen
                    CExpr::Const(_) => {}
                    condition => kept.push((condition, value)),
                }
            }
            if kept.is_empty() {
                *otherwise
            } else {
                CExpr::Case { branches: kept, otherwise }
            }
        }
        CExpr::And(operands) => {
            if operands.iter().any(|it| is_bool(it, false)) {
                return CExpr::Const(Value::Bool(false));
            }
            let mut operands = operands.into_iter().filter(|it| !is_bool(it, true)).collect::<Vec<_>>();
            match operands.len() {
                0 => CExpr::Const(Value::Bool(true)),
                1 => operands.pop().expect("one operand"),
                _ => CExpr::And(operands),
            }
        }
        CExpr::Or(operands) => {
            if operands.iter().any(|it| is_bool(it, true)) {
                return CExpr::Const(Value::Bool(true));
            }
            let mut operands = operands.into_iter().filter(|it| !is_bool(it, false)).collect::<Vec<_>>();
            match operands.len() {
                0 => CExpr::Const(Value::Bool(false)),
                1 => operands.pop().expect("one operand"),
                _ => CExpr::Or(operands),
            }
        }
        other => other,
    }
}

/// Evaluate a node whose operands are all constants once, at compile time.
pub(crate) fn fold_constants(expr: CExpr) -> CExpr {
    let foldable = match &expr {
        CExpr::Const(_) | CExpr::Column(_) | CExpr::Running(_) | CExpr::Param(_) | CExpr::Aggregate(_) | CExpr::Target(_) => false,
        CExpr::Scalar { function, .. } if NOT_FOLDABLE.contains(&function.name) => false,
        node => node.children().iter().all(|child| matches!(child, CExpr::Const(_))),
    };
    if !foldable {
        return expr;
    }
    // `eval_constant` also refuses anything that touched the execution context, and leaves
    // failing expressions to report their error when they run
    match eval_constant(&expr) {
        Some(value) => CExpr::Const(value),
        None => expr,
    }
}

/// [`precompile_regex`] for a pattern bound at execution: an invalid pattern is left to
/// report its error, at the match, when a row evaluates it (as before binding).
fn precompile_valid_regex(expr: CExpr, patterns: &Patterns) -> CExpr {
    match expr {
        CExpr::Regex {
            subject,
            pattern: RegexPattern::Dynamic(pattern),
            case_insensitive,
            negated,
            span,
            pattern_span,
        } => {
            let pattern = match *pattern {
                CExpr::Const(Value::Str(text)) => match patterns.get(&text, case_insensitive) {
                    Ok(regex) => RegexPattern::Compiled(regex),
                    Err(_) => RegexPattern::Dynamic(Box::new(CExpr::Const(Value::Str(text)))),
                },
                other => RegexPattern::Dynamic(Box::new(other)),
            };
            CExpr::Regex {
                subject,
                pattern,
                case_insensitive,
                negated,
                span,
                pattern_span,
            }
        }
        other => other,
    }
}

/// `x IN (constants)` and `x IN <constant set>` as one hash lookup ([`CExpr::InConst`]).
pub(crate) fn prepare_membership(expr: CExpr) -> CExpr {
    match expr {
        CExpr::InList { needle, items, negated } if items.iter().all(|item| matches!(item, CExpr::Const(_))) => {
            let items = items
                .into_iter()
                .map(|item| match item {
                    CExpr::Const(value) => value,
                    _ => unreachable!("every item is a constant"),
                })
                .collect();
            CExpr::InConst {
                needle,
                set: Arc::new(ConstSet::from_list(items)),
                negated,
            }
        }
        CExpr::InSet { needle, set, negated } => match *set {
            CExpr::Const(Value::Set(elements)) => CExpr::InConst {
                needle,
                set: Arc::new(ConstSet::from_set(&elements)),
                negated,
            },
            set => CExpr::InSet {
                needle,
                set: Box::new(set),
                negated,
            },
        },
        other => other,
    }
}

/// `icontains`, `any_icontains` and `under` with a constant string as their second argument,
/// prepared once ([`CExpr::StrTest`]).
pub(crate) fn prepare_str_test(expr: CExpr) -> CExpr {
    match expr {
        CExpr::Scalar { function, mut args, span }
            if matches!(function.name, "icontains" | "any_icontains" | "under") && matches!(args.as_slice(), [_, CExpr::Const(Value::Str(_))]) =>
        {
            let Some(CExpr::Const(Value::Str(argument))) = args.pop() else {
                unreachable!("a constant string argument")
            };
            let subject = args.pop().expect("two arguments");
            let kind = match function.name {
                "icontains" => StrTestKind::IContains(argument.to_lowercase()),
                "any_icontains" => StrTestKind::AnyIContains(argument.to_lowercase()),
                _ => StrTestKind::Under,
            };
            CExpr::StrTest {
                subject: Box::new(subject),
                test: Arc::new(StrTest { function, argument, kind }),
                span,
            }
        }
        other => other,
    }
}

/// Compile the regular expression of a match against a constant pattern once.
pub(crate) fn precompile_regex(expr: CExpr) -> Result<CExpr, LocatedError> {
    match expr {
        CExpr::Regex {
            subject,
            pattern: RegexPattern::Dynamic(pattern),
            case_insensitive,
            negated,
            span,
            pattern_span,
        } => {
            let pattern = match *pattern {
                CExpr::Const(Value::Str(text)) => {
                    RegexPattern::Compiled(build_regex(&text, case_insensitive).map_err(|message| LocatedError::compile(message, pattern_span))?)
                }
                other => RegexPattern::Dynamic(Box::new(other)),
            };
            Ok(CExpr::Regex {
                subject,
                pattern,
                case_insensitive,
                negated,
                span,
                pattern_span,
            })
        }
        other => Ok(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::{ArithOp, ArithStep, Running};
    use crate::error::{QueryErrorKind, Span};
    use crate::functions::SCALAR_FUNCTIONS;
    use crate::table::column;
    use crate::ParamTypes;

    fn col(name: &str) -> CExpr {
        CExpr::Column(column(name).unwrap())
    }
    fn bool_(value: bool) -> CExpr {
        CExpr::Const(Value::Bool(value))
    }
    fn null() -> CExpr {
        CExpr::Const(Value::Null)
    }
    fn str_(value: &str) -> CExpr {
        CExpr::Const(Value::from(value))
    }
    fn is_null(expr: CExpr) -> CExpr {
        CExpr::IsNull {
            expr: Box::new(expr),
            negated: false,
        }
    }
    fn chain(first: CExpr, rest: Vec<(ArithOp, CExpr)>) -> CExpr {
        CExpr::Arith {
            first: Box::new(first),
            rest: rest
                .into_iter()
                .map(|(op, operand)| ArithStep {
                    op,
                    operand,
                    span: Span::default(),
                })
                .collect(),
        }
    }
    fn scalar(name: &str, args: Vec<CExpr>) -> CExpr {
        CExpr::Scalar {
            function: SCALAR_FUNCTIONS.iter().find(|it| it.name == name && it.params.len() == args.len()).unwrap(),
            args,
            span: Span::default(),
        }
    }
    fn regex(subject: CExpr, pattern: CExpr) -> CExpr {
        CExpr::Regex {
            subject: Box::new(subject),
            pattern: RegexPattern::Dynamic(Box::new(pattern)),
            case_insensitive: true,
            negated: false,
            span: Span::default(),
            pattern_span: Span::new(3, 5),
        }
    }
    fn show(expr: &CExpr) -> String {
        expr.to_string()
    }

    #[test]
    fn flattens_nested_and_or_and_left_chains() {
        let nested = CExpr::And(vec![
            CExpr::And(vec![is_null(col("payee")), is_null(col("narration"))]),
            is_null(col("account")),
        ]);
        assert_eq!(
            show(&flatten_associative(nested)),
            "((payee IS NULL) AND (narration IS NULL) AND (account IS NULL))"
        );
        let nested = CExpr::Or(vec![is_null(col("payee")), CExpr::Or(vec![is_null(col("narration")), is_null(col("account"))])]);
        assert_eq!(
            show(&flatten_associative(nested)),
            "((payee IS NULL) OR (narration IS NULL) OR (account IS NULL))"
        );
        // an OR inside an AND stays
        let mixed = CExpr::And(vec![CExpr::Or(vec![bool_(true), null()]), null()]);
        assert_eq!(show(&flatten_associative(mixed)), "((TRUE OR NULL) AND NULL)");
        // (number - 1) * 2 is the chain number - 1 * 2 evaluated left to right
        let left = chain(
            chain(col("number"), vec![(ArithOp::Sub, CExpr::Const(Value::Int(1)))]),
            vec![(ArithOp::Mul, CExpr::Const(Value::Int(2)))],
        );
        assert_eq!(show(&flatten_associative(left)), "(number - 1 * 2)");
        // number - (1 - 2) cannot be spliced
        let right = chain(
            col("number"),
            vec![(
                ArithOp::Sub,
                chain(CExpr::Const(Value::Int(1)), vec![(ArithOp::Sub, CExpr::Const(Value::Int(2)))]),
            )],
        );
        assert_eq!(show(&flatten_associative(right)), "(number - (1 - 2))");
    }

    #[test]
    fn eliminates_double_negation() {
        let expr = CExpr::Not(Box::new(CExpr::Not(Box::new(is_null(col("payee"))))));
        assert_eq!(show(&eliminate_double_negation(expr)), "(payee IS NULL)");
        let single = CExpr::Not(Box::new(is_null(col("payee"))));
        assert_eq!(show(&eliminate_double_negation(single)), "NOT (payee IS NULL)");
    }

    #[test]
    fn simplifies_logic_with_three_valued_semantics() {
        let x = || is_null(col("payee"));
        assert_eq!(show(&simplify_logic(CExpr::And(vec![bool_(true), x()]))), "(payee IS NULL)");
        assert_eq!(show(&simplify_logic(CExpr::And(vec![x(), bool_(false)]))), "FALSE");
        // x AND FALSE is FALSE even when x is NULL
        assert_eq!(show(&simplify_logic(CExpr::And(vec![null(), bool_(false)]))), "FALSE");
        assert_eq!(show(&simplify_logic(CExpr::Or(vec![x(), bool_(true)]))), "TRUE");
        assert_eq!(show(&simplify_logic(CExpr::Or(vec![bool_(false), x()]))), "(payee IS NULL)");
        // NULL operands are kept: x OR NULL is not x
        assert_eq!(show(&simplify_logic(CExpr::Or(vec![x(), null()]))), "((payee IS NULL) OR NULL)");
        assert_eq!(show(&simplify_logic(CExpr::And(vec![x(), null()]))), "((payee IS NULL) AND NULL)");
        assert_eq!(show(&simplify_logic(CExpr::And(vec![bool_(true), bool_(true)]))), "TRUE");
        assert_eq!(show(&simplify_logic(CExpr::Or(vec![bool_(false), bool_(false)]))), "FALSE");
    }

    /// The date bound of a generated table's filter, as EXPLAIN shows it.
    #[test]
    fn bounds_generated_tables_by_the_dates_the_filter_keeps() {
        let bound = |sql: &str| {
            let types = ParamTypes::new().bind("month", DataType::Date);
            let query = crate::Query::compile_with_params(sql, &types).unwrap_or_else(|err| panic!("{sql}: {err}"));
            query.plan.execution.until.as_ref().map(|it| it.to_string())
        };
        let some = |text: &str| Some(text.to_owned());
        assert_eq!(bound("SELECT name FROM #budgets WHERE date <= 2024-06-01"), some("date <= 2024-06-01"));
        assert_eq!(bound("SELECT name FROM #budgets WHERE date < :month"), some("date < :month"));
        assert_eq!(bound("SELECT name FROM #budgets WHERE :month > date"), some("date < :month"));
        assert_eq!(bound("SELECT name FROM #budgets WHERE '2024-06-01' = date"), some("date <= 2024-06-01"));
        assert_eq!(
            bound("SELECT name FROM #budgets WHERE yearmonth(date) = :month"),
            some("yearmonth(date) <= :month")
        );
        assert_eq!(
            bound("SELECT name FROM #budgets WHERE name = 'a' AND date <= :month AND closed"),
            some("date <= :month")
        );
        assert_eq!(bound("SELECT name FROM #budgets WHERE date >= :month"), None);
        assert_eq!(bound("SELECT name FROM #budgets WHERE date <= today()"), some("date <= today()"));
        assert_eq!(bound("SELECT name FROM #budgets WHERE date = yearmonth(today())"), some("date <= today()"));
        assert_eq!(bound("SELECT name FROM #budgets WHERE yearmonth(today()) >= date"), some("date <= today()"));
        assert_eq!(bound("SELECT name FROM #budgets WHERE date <= date_add(today(), 1)"), None);
        assert_eq!(bound("SELECT name FROM #budget_events WHERE date <= :month"), None);
        let explain = crate::Query::compile("SELECT name FROM #budgets WHERE date <= 2024-06-01").unwrap().explain();
        assert!(explain.contains("generate: the rows up to date <= 2024-06-01\n"), "{explain}");
        // the last date of an execution, on 2024-09-09
        let day = |m: u32, d: u32| chrono::NaiveDate::from_ymd_opt(2024, m, d).unwrap();
        let resolve = |sql: &str, params: Params| {
            let types = ParamTypes::new().bind("month", DataType::Date);
            let query = crate::Query::compile_with_params(sql, &types).unwrap();
            query.plan.execution.until.as_ref().unwrap().resolve(&params, day(9, 9))
        };
        let month = |date| Params::new().bind("month", date);
        assert_eq!(resolve("SELECT name FROM #budgets WHERE date <= :month", month(day(6, 1))), day(6, 1));
        assert_eq!(resolve("SELECT name FROM #budgets WHERE date < :month", month(day(6, 1))), day(5, 31));
        assert_eq!(
            resolve("SELECT name FROM #budgets WHERE yearmonth(date) = :month", month(day(2, 1))),
            day(2, 29)
        );
        assert_eq!(
            resolve("SELECT name FROM #budgets WHERE yearmonth(date) < :month", month(day(3, 1))),
            day(2, 29)
        );
        assert_eq!(
            resolve("SELECT name FROM #budgets WHERE date <= :month", Params::new().bind("month", Value::Null)),
            chrono::NaiveDate::MIN
        );
        assert_eq!(resolve("SELECT name FROM #budgets WHERE date = yearmonth(today())", Params::new()), day(9, 9));
    }

    #[test]
    fn folds_pure_constant_subtrees_only() {
        let concat = chain(str_("^Ex"), vec![(ArithOp::Add, str_("penses"))]);
        assert_eq!(show(&fold_constants(concat)), "'^Expenses'");
        assert_eq!(show(&fold_constants(CExpr::Not(Box::new(bool_(true))))), "FALSE");
        assert_eq!(
            show(&fold_constants(scalar("root", vec![str_("Assets:Bank:Cash"), CExpr::Const(Value::Int(2))]))),
            "'Assets:Bank'"
        );
        // functions of the execution are never folded
        assert_eq!(show(&fold_constants(scalar("today", vec![]))), "today()");
        assert_eq!(show(&fold_constants(scalar("entry_meta", vec![str_("x")]))), "entry_meta('x')");
        // nodes reading a column are not constant
        assert_eq!(show(&fold_constants(is_null(col("payee")))), "(payee IS NULL)");
        // nor is the running balance, which changes from row to row
        assert_eq!(show(&fold_constants(CExpr::Running(Running::Balance))), "balance");
        assert_eq!(show(&fold_constants(is_null(CExpr::Running(Running::Balance)))), "(balance IS NULL)");
        assert_eq!(
            show(&optimize_expr(scalar("units", vec![CExpr::Running(Running::Balance)])).unwrap()),
            "units(balance)"
        );
        // failures are left for execution, which reports them with a position
        let overflow = chain(CExpr::Const(Value::Int(i64::MAX)), vec![(ArithOp::Add, CExpr::Const(Value::Int(1)))]);
        assert!(matches!(fold_constants(overflow), CExpr::Arith { .. }));
    }

    #[test]
    fn precompiles_constant_patterns() {
        let compiled = precompile_regex(regex(col("account"), str_("^Expenses"))).unwrap();
        assert_eq!(show(&compiled), "(account ~ /^Expenses/i)");
        let dynamic = precompile_regex(regex(col("account"), col("narration"))).unwrap();
        assert_eq!(show(&dynamic), "(account ~ regex_i(narration))");
        let err = precompile_regex(regex(col("account"), str_("("))).err().unwrap();
        assert_eq!(err.kind, QueryErrorKind::Compile);
        assert_eq!(err.span, Some(Span::new(3, 5)));
    }

    #[test]
    fn merges_from_and_where_into_one_filter() {
        assert!(merge_filters(vec![]).is_none());
        assert_eq!(show(&merge_filters(vec![is_null(col("payee"))]).unwrap()), "(payee IS NULL)");
        let merged = merge_filters(vec![
            is_null(col("payee")),
            CExpr::And(vec![is_null(col("narration")), is_null(col("account"))]),
        ])
        .unwrap();
        assert_eq!(
            show(&optimize_expr(merged).unwrap()),
            "((payee IS NULL) AND (narration IS NULL) AND (account IS NULL))"
        );
    }

    fn scope_of(sql: &str) -> Option<String> {
        let types = crate::ParamTypes::new().bind("a", DataType::Str).bind("set", DataType::Set);
        let query = crate::Query::compile_with_params(sql, &types).unwrap_or_else(|err| panic!("{sql}: {err}"));
        query.plan.execution.scope.map(|scope| scope.to_string())
    }

    #[test]
    fn scopes_postings_to_the_accounts_the_filter_names() {
        let some = |it: &str| Some(it.to_owned());
        assert_eq!(scope_of("SELECT date WHERE account = 'A'"), some("'A'"));
        assert_eq!(scope_of("SELECT date WHERE :a = account"), some(":a"));
        assert_eq!(scope_of("SELECT date WHERE account IN ('A', :a, NULL)"), some("'A', :a, NULL"));
        assert_eq!(scope_of("SELECT date WHERE account IN :set"), some(":set"));
        assert_eq!(
            scope_of("SELECT date WHERE account = 'A' OR (account IN :set OR account = :a)"),
            some("'A', :set, :a")
        );
        // a constant folded from an expression, and infallible conjuncts before it
        assert_eq!(scope_of("SELECT date WHERE account = 'Ex' + 'penses'"), some("'Expenses'"));
        assert_eq!(
            scope_of("SELECT date FROM year = 2024 WHERE number > 0 AND 'x' IN tags AND account = 'A' AND payee ~ narration"),
            some("'A'")
        );
        assert_eq!(scope_of("SELECT account, sum(position) WHERE account = :a GROUP BY account"), some(":a"));
        // the search functions never fail, so a keyword test before the account keeps the scope
        assert_eq!(
            scope_of("SELECT date WHERE icontains(payee, :a) AND any_icontains(tags, :a) AND under(account, :a)"),
            some("under :a")
        );
        assert_eq!(scope_of("SELECT date WHERE icontains(payee, 'x') AND account IN ('A', 'B')"), some("'A', 'B'"));

        // the filter holds for rows of any account
        for sql in [
            "SELECT date",
            "SELECT date WHERE account != 'A'",
            "SELECT date WHERE NOT account = 'A'",
            "SELECT date WHERE account NOT IN ('A')",
            "SELECT date WHERE account = payee",
            "SELECT date WHERE account IN ('A', payee)",
            "SELECT date WHERE account = 'A' OR year = 2024",
            "SELECT date WHERE account ~ '^A'",
            // a full scan evaluates a conjunct before the scoping one for every row, and stops at its error
            "SELECT date WHERE payee ~ narration AND account = 'A'",
            "SELECT date WHERE number / 0 > 1 AND account = 'A'",
            // the period modifiers move balances to other accounts before the filter runs
            "SELECT date FROM OPEN ON 2024-01-01 WHERE account = 'A'",
            "SELECT date FROM CLOSE CLEAR WHERE account = 'A'",
            // only the postings table is scoped
            "SELECT date FROM #balances WHERE account = 'A'",
        ] {
            assert_eq!(scope_of(sql), None, "{sql}");
        }
    }

    /// `under(account, ancestor)`: the scope keeps the ancestor and its sub-accounts, whether the
    /// ancestor is a literal (prepared into a string test when compiled), a parameter, or a
    /// parameter bound to a literal in the plan of the execution. Scoped and unscoped
    /// executions return the same rows and totals.
    #[test]
    fn scopes_postings_to_the_accounts_under_an_ancestor() {
        let source = zhang_core::data_source::LocalFileSystemDataSource::new(zhang_core::data_type::text::ZhangDataType {});
        let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../integration-tests/fava-demo-ledger");
        let ledger = zhang_core::ledger::Ledger::load_with_data_source(dir, "main.zhang".to_owned(), std::sync::Arc::new(source)).unwrap();
        let options = crate::ExecuteOptions {
            today: None,
            timeout: None,
            max_result_values: None,
            count_total: true,
        };
        let columns = "SELECT date, account, payee, balance, account_balance WHERE payee IS NOT NULL AND";
        let mut counts = vec![];
        for ancestor in ["Assets:US", "Assets:US:BofA", "Assets:U", "Nowhere"] {
            let forms = [
                (
                    format!("{columns} (under(account, '{ancestor}') OR account = 'Income:US:Hoogle:Salary')"),
                    format!("under '{ancestor}', 'Income:US:Hoogle:Salary'"),
                ),
                (
                    format!("{columns} (under(account, :root) OR account = 'Income:US:Hoogle:Salary') LIMIT :n"),
                    "under :root, 'Income:US:Hoogle:Salary'".to_owned(),
                ),
            ];
            let params = crate::Params::new().bind("root", ancestor).bind("n", 100_000i64);
            let mut results = vec![];
            for (sql, scope) in forms {
                let run = |scoped: bool| {
                    let mut query = crate::Query::compile_with_params(&sql, &params.types()).unwrap();
                    if scoped {
                        assert_eq!(query.plan.execution.scope.as_ref().map(|it| it.to_string()), Some(scope.clone()), "{sql}");
                        assert!(
                            query.explain().contains(&format!("scan: the rows of the accounts {scope}\n")),
                            "{}",
                            query.explain()
                        );
                    } else {
                        query.plan.execution.scope = None;
                    }
                    let result = query.execute_with_options(&ledger, &params, &options).unwrap();
                    format!("{:?} {:?}", result.rows, result.total)
                };
                let scoped = run(true);
                assert_eq!(scoped, run(false), "{sql}");
                results.push(scoped);
            }
            assert_eq!(results[0], results[1], "{ancestor}");
            counts.push(results[0].matches("Date(").count());
        }
        // 'Assets:U' is not an ancestor of 'Assets:US': like 'Nowhere', only the salary is left
        assert!(
            counts[0] > counts[1] && counts[1] > counts[2] && counts[2] == counts[3] && counts[3] > 0,
            "{counts:?}"
        );
    }

    fn in_list(needle: CExpr, items: Vec<CExpr>) -> CExpr {
        CExpr::InList {
            needle: Box::new(needle),
            items,
            negated: false,
        }
    }

    #[test]
    fn constant_membership_becomes_a_hash_lookup() {
        let prepared = prepare_membership(in_list(col("account"), vec![str_("a"), str_("b"), null()]));
        assert!(matches!(&prepared, CExpr::InConst { set, .. } if set.contains_str("b") && !set.contains_str("c") && set.has_null()));
        // EXPLAIN shows the list as written
        assert_eq!(show(&prepared), "(account IN ('a', 'b', NULL))");
        // an int item matches the equal decimal
        let numbers = prepare_membership(in_list(
            col("number"),
            vec![CExpr::Const(Value::Int(4)), CExpr::Const(Value::Decimal("8.95".parse().unwrap()))],
        ));
        let CExpr::InConst { set, .. } = &numbers else { panic!("{}", show(&numbers)) };
        assert!(set.contains(&Value::Decimal("4.00".parse().unwrap())) && !set.contains(&Value::Int(8)));
        // a list with a column is evaluated item by item
        let dynamic = prepare_membership(in_list(col("account"), vec![str_("a"), col("payee")]));
        assert!(matches!(dynamic, CExpr::InList { .. }));
        // a constant set: the elements of a bound parameter
        let set = CExpr::InSet {
            needle: Box::new(col("account")),
            set: Box::new(CExpr::Const(Value::Set(["x".to_owned(), "y".to_owned()].into_iter().collect()))),
            negated: true,
        };
        let prepared = prepare_membership(set);
        assert_eq!(show(&prepared), "(account NOT IN {'x', 'y'})");
        let CExpr::InConst { set, .. } = &prepared else { panic!() };
        assert_eq!(set.values(), vec![Value::Set(["x".to_owned(), "y".to_owned()].into_iter().collect())]);
        // a list keeps its items as written
        let CExpr::InConst { set, .. } = prepare_membership(in_list(col("account"), vec![str_("a"), null()])) else {
            panic!()
        };
        assert_eq!(set.values(), vec![Value::from("a"), Value::Null]);
    }

    #[test]
    fn string_tests_prepare_their_constant() {
        let prepared = prepare_str_test(scalar("icontains", vec![col("payee"), str_("CaFé")]));
        let CExpr::StrTest { test, .. } = &prepared else {
            panic!("{}", show(&prepared))
        };
        assert!(matches!(&test.kind, StrTestKind::IContains(needle) if needle == "café"));
        assert_eq!(show(&prepared), "icontains(payee, 'CaFé')");
        let under = prepare_str_test(scalar("under", vec![col("account"), str_("Assets")]));
        assert_eq!(show(&under), "under(account, 'Assets')");
        // a NULL or non-constant second argument stays a call
        assert!(matches!(
            prepare_str_test(scalar("icontains", vec![col("payee"), null()])),
            CExpr::Scalar { .. }
        ));
        assert!(matches!(
            prepare_str_test(scalar("under", vec![col("account"), col("payee")])),
            CExpr::Scalar { .. }
        ));
    }

    #[test]
    fn under_is_recognized_in_every_form() {
        let params = Params::new().bind("root", "Assets:US");
        let param = CExpr::Param(crate::params::ParamRef::Named("root".into()));
        for (expr, expected) in [
            (scalar("under", vec![col("account"), str_("Assets")]), Some("Assets")),
            (prepare_str_test(scalar("under", vec![col("account"), str_("Expenses")])), Some("Expenses")),
            (scalar("under", vec![col("account"), param.clone()]), Some("Assets:US")),
            (scalar("under", vec![col("account"), null()]), None),
            (scalar("icontains", vec![col("account"), str_("Assets")]), None),
        ] {
            let found = expr.as_under();
            assert_eq!(found.and_then(|(_, ancestor)| ancestor.bound(&params)), expected, "{}", show(&expr));
            if let Some((subject, _)) = found {
                assert_eq!(show(subject), "account");
            }
        }
        let unbound = scalar("under", vec![col("account"), param]);
        assert!(unbound.as_under().unwrap().1.bound(&Params::new()).is_none());
        assert!(scalar("under", vec![col("payee"), col("account")]).as_under().is_none());
    }

    #[test]
    fn binding_turns_parameters_into_prepared_constants() {
        let sql = "SELECT date WHERE payee ~ :pattern AND account IN :accounts AND icontains(narration, :needle) AND under(account, :root) AND payee !~ :bad";
        let types = crate::ParamTypes::new()
            .bind("pattern", DataType::Str)
            .bind("accounts", DataType::Set)
            .bind("needle", DataType::Str)
            .bind("root", DataType::Str)
            .bind("bad", DataType::Str);
        let query = crate::Query::compile_with_params(sql, &types).unwrap();
        let params = Params::new()
            .bind("pattern", "^Cafe")
            .bind("accounts", Value::Set(["Assets:Cash".to_owned()].into_iter().collect()))
            .bind("needle", "LUNCH")
            .bind("root", "Assets")
            .bind("bad", "(");
        let bound = bind(&query.plan, &params);
        assert_eq!(
            show(bound.filter.as_ref().unwrap()),
            "((payee ~ /^Cafe/i) AND (account IN {'Assets:Cash'}) AND icontains(narration, 'LUNCH') AND under(account, 'Assets') AND (payee !~ regex_i('(')))"
        );
        // the compiled plan keeps its parameters
        assert!(show(query.plan.filter.as_ref().unwrap()).contains("regex_i(:pattern)"));
        // a parameter that makes the filter always hold removes it
        let query =
            crate::Query::compile_with_params("SELECT date WHERE :all OR account = 'x'", &crate::ParamTypes::new().bind("all", DataType::Bool)).unwrap();
        assert!(bind(&query.plan, &Params::new().bind("all", true)).filter.is_none());
        assert_eq!(
            show(bind(&query.plan, &Params::new().bind("all", Value::Null)).filter.as_ref().unwrap()),
            "(NULL OR (account = 'x'))"
        );
        // a filter that folds to FALSE or NULL keeps every row out: it stays
        let query = crate::Query::compile_with_params("SELECT date WHERE :all", &crate::ParamTypes::new().bind("all", DataType::Bool)).unwrap();
        assert!(bind(&query.plan, &Params::new().bind("all", true)).filter.is_none());
        assert_eq!(show(bind(&query.plan, &Params::new().bind("all", false)).filter.as_ref().unwrap()), "FALSE");
        assert_eq!(
            show(bind(&query.plan, &Params::new().bind("all", Value::Null)).filter.as_ref().unwrap()),
            "NULL"
        );
        // and so does HAVING
        let query = crate::Query::compile_with_params(
            "SELECT account, count(*) GROUP BY account HAVING count(*) > 0 AND :all",
            &crate::ParamTypes::new().bind("all", DataType::Bool),
        )
        .unwrap();
        assert!(bind(&query.plan, &Params::new().bind("all", true)).having.is_some());
        assert_eq!(show(bind(&query.plan, &Params::new().bind("all", false)).having.as_ref().unwrap()), "FALSE");
        let query = crate::Query::compile_with_params(
            "SELECT account, count(*) GROUP BY account HAVING (count(*) > 0 OR :all) AND :n = 1",
            &crate::ParamTypes::new().bind("all", DataType::Bool).bind("n", DataType::Int),
        )
        .unwrap();
        let bound = |all: bool, n: Value| bind(&query.plan, &Params::new().bind("all", all).bind("n", n));
        assert!(bound(true, Value::Int(1)).having.is_none());
        assert_eq!(show(bound(true, Value::Null).having.as_ref().unwrap()), "NULL");
        assert_eq!(show(bound(true, Value::Int(2)).having.as_ref().unwrap()), "FALSE");
    }

    #[test]
    fn rules_compose_bottom_up() {
        // account ~ ('^Ex' + 'penses') AND NOT NOT TRUE  →  a precompiled regex
        let expr = CExpr::And(vec![
            regex(col("account"), chain(str_("^Ex"), vec![(ArithOp::Add, str_("penses"))])),
            CExpr::Not(Box::new(CExpr::Not(Box::new(bool_(true))))),
        ]);
        assert_eq!(show(&optimize_expr(expr).unwrap()), "(account ~ /^Expenses/i)");
    }
}
