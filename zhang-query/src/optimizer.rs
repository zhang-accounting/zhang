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
//!    `x OR NULL` is not `x`.
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
//! Then [`plan_execution`] makes two decisions about the plan as a whole:
//!
//! 8. [`rewrite_linear_balance`]: `units(balance)` and `cost(balance)` are linear in the
//!    positions, so they become running sums of `units(position)` / `cost(position)` per
//!    currency ([`Running::Units`], [`Running::Cost`]), which cost O(currencies) per row
//!    instead of O(open lots). `value()` and `convert()` price by date and are not linear.
//! 9. [`limit_mode`]: how LIMIT cuts the work short. Without ORDER BY a scan stops once it has
//!    LIMIT rows (telling DISTINCT rows apart while scanning) and an aggregate query only
//!    aggregates its first LIMIT groups (unless HAVING may drop some of them); with ORDER BY
//!    (and no DISTINCT) the scan keeps the top LIMIT rows instead of sorting them all.
//!
//! The expression rules rewrite the HAVING condition like any other expression; one that
//! folds to TRUE is dropped, like a filter.

use std::cell::RefCell;
use std::sync::Arc;

use regex::Regex;

use crate::compiler::{build_regex, CExpr, ConstSet, LimitMode, Plan, RegexPattern, Running, StrTest, StrTestKind};
use crate::error::LocatedError;
use crate::executor::eval_constant;
use crate::functions::ParamType;
use crate::params::Params;
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

/// The decisions about the plan as a whole: [`rewrite_linear_balance`] and [`limit_mode`].
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
    rewrites.sort();
    rewrites.dedup();
    plan.execution.rewrites = rewrites;
    plan.execution.limit = limit_mode(plan);
}

/// `units(balance)` → [`Running::Units`] and `cost(balance)` → [`Running::Cost`], bottom-up,
/// recording each rewrite.
pub(crate) fn rewrite_linear_balance(expr: CExpr, rewrites: &mut Vec<Running>) -> CExpr {
    let expr = expr
        .map_children(&mut |child| Ok::<_, std::convert::Infallible>(rewrite_linear_balance(child, rewrites)))
        .unwrap_or_else(|never| match never {});
    match expr {
        CExpr::Scalar { function, args, span }
            if function.params == [ParamType::Exact(DataType::Inventory)] && matches!(args.as_slice(), [CExpr::Running(Running::Balance)]) =>
        {
            let total = match function.name {
                "units" => Running::Units,
                "cost" => Running::Cost,
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
/// three-valued logic (`NULL` operands are never dropped).
pub(crate) fn simplify_logic(expr: CExpr) -> CExpr {
    let is_bool = |expr: &CExpr, value: bool| matches!(expr, CExpr::Const(Value::Bool(it)) if *it == value);
    match expr {
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
        let mut strings = set.only_strings().unwrap().collect::<Vec<_>>();
        strings.sort();
        assert_eq!(strings, ["x", "y"]);
        // a list with a NULL or a number is not only strings
        let CExpr::InConst { set, .. } = prepare_membership(in_list(col("account"), vec![str_("a"), null()])) else {
            panic!()
        };
        assert!(set.only_strings().is_none());
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
            let found = expr.as_under(&params);
            assert_eq!(found.map(|(_, ancestor)| ancestor), expected, "{}", show(&expr));
            if let Some((subject, _)) = found {
                assert_eq!(show(subject), "account");
            }
        }
        assert!(scalar("under", vec![col("account"), param]).as_under(&Params::new()).is_none());
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
