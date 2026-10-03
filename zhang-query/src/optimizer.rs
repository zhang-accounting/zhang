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
//!
//! Then [`plan_execution`] makes two decisions about the plan as a whole:
//!
//! 6. [`rewrite_linear_balance`]: `units(balance)` and `cost(balance)` are linear in the
//!    positions, so they become running sums of `units(position)` / `cost(position)` per
//!    currency ([`Running::Units`], [`Running::Cost`]), which cost O(currencies) per row
//!    instead of O(open lots). `value()` and `convert()` price by date and are not linear.
//! 7. [`limit_mode`]: how LIMIT cuts the work short. Without ORDER BY a scan stops once it has
//!    LIMIT rows (telling DISTINCT rows apart while scanning) and an aggregate query only
//!    aggregates its first LIMIT groups (unless HAVING may drop some of them); with ORDER BY
//!    (and no DISTINCT) the scan keeps the top LIMIT rows instead of sorting them all.
//! 8. [`account_scope`]: when the filter of a `postings` query can only hold for the rows of
//!    some accounts (`account = :account`), the execution only builds their rows.
//!
//! The expression rules rewrite the HAVING condition like any other expression; one that
//! folds to TRUE is dropped, like a filter.

use crate::compiler::{build_regex, AccountScope, CExpr, CmpOp, LimitMode, Plan, RegexPattern, Running, ScopeValue, ScopedAccount};
use crate::error::LocatedError;
use crate::executor::eval_constant;
use crate::functions::ParamType;
use crate::projector::infallible;
use crate::value::{DataType, Value};

/// Scalar functions that depend on the execution (its date, the ledger's prices, the current
/// row) and are therefore never folded, even with constant arguments.
const NOT_FOLDABLE: &[&str] = &["today", "meta", "entry_meta", "any_meta", "convert", "value", "getprice"];

/// Optimize a compiled plan: rewrite its expressions, then [`plan_execution`].
pub(crate) fn optimize(plan: Plan) -> Result<Plan, LocatedError> {
    let mut plan = optimize_expressions(plan)?;
    plan_execution(&mut plan);
    Ok(plan)
}

/// Only the expression rules, leaving the naive execution the compiler chose: the reference
/// the decisions of [`plan_execution`] are tested against.
#[cfg(test)]
pub(crate) fn optimize_naive(plan: Plan) -> Result<Plan, LocatedError> {
    optimize_expressions(plan)
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
    rewrites.sort();
    rewrites.dedup();
    plan.execution.rewrites = rewrites;
    plan.execution.limit = limit_mode(plan);
    plan.execution.scope = account_scope(plan);
}

/// The accounts a `postings` query's rows can be limited to: those named by a conjunct of the
/// filter (FROM, then WHERE) that only holds for the rows of some accounts:
///
/// - `account = x` (a string constant or parameter, on either side),
/// - `account IN (x, y, ...)` and `account IN :set`,
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
        // `under(account, ancestor)`
        CExpr::Scalar { function, args, .. } if function.name == "under" => match args.as_slice() {
            [account, ancestor] if is_account(account) => Some(vec![ScopedAccount::Under(named(ancestor)?)]),
            _ => None,
        },
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

fn optimize_expressions(mut plan: Plan) -> Result<Plan, LocatedError> {
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
pub(crate) fn optimize_expr(expr: CExpr) -> Result<CExpr, LocatedError> {
    let expr = expr.map_children(&mut optimize_expr)?;
    let expr = flatten_associative(expr);
    let expr = eliminate_double_negation(expr);
    let expr = simplify_logic(expr);
    let expr = fold_constants(expr);
    precompile_regex(expr)
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

    /// `under(account, ancestor)`, which another change adds: the scope keeps the ancestor and
    /// its sub-accounts, read here with a stand-in for the function.
    #[test]
    fn scopes_postings_to_the_accounts_under_an_ancestor() {
        use crate::functions::{FunctionContext, ReturnType, ScalarFunction};

        fn under(args: &[Value], _: &dyn FunctionContext) -> Result<Value, String> {
            let (Value::Str(account), Value::Str(ancestor)) = (&args[0], &args[1]) else {
                return Err("strings".to_owned());
            };
            let rest = account.strip_prefix(ancestor.as_str());
            Ok(Value::Bool(rest.is_some_and(|rest| rest.is_empty() || rest.starts_with(':'))))
        }
        static UNDER: ScalarFunction = ScalarFunction {
            name: "under",
            params: &[ParamType::Exact(DataType::Str), ParamType::Exact(DataType::Str)],
            returns: ReturnType::Exact(DataType::Bool),
            description: "",
            eval: under,
        };

        let source = zhang_core::data_source::LocalFileSystemDataSource::new(zhang_core::data_type::text::ZhangDataType {});
        let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../integration-tests/fava-demo-ledger");
        let ledger = zhang_core::ledger::Ledger::load_with_data_source(dir, "main.zhang".to_owned(), std::sync::Arc::new(source)).unwrap();
        let options = crate::ExecuteOptions {
            today: None,
            timeout: None,
            max_result_values: None,
        };
        let mut counts = vec![];
        for ancestor in ["Assets:US", "Assets:US:BofA", "Assets:U", "Nowhere"] {
            let run = |scoped: bool| {
                let mut query = crate::Query::compile("SELECT date, account, payee, balance, account_balance WHERE account = ''").unwrap();
                let account = CExpr::Column(column("account").unwrap());
                let parent = CExpr::Const(Value::from(ancestor));
                query.plan.filter = Some(CExpr::And(vec![
                    CExpr::IsNull {
                        expr: Box::new(col("payee")),
                        negated: true,
                    },
                    CExpr::Or(vec![
                        CExpr::Scalar {
                            function: &UNDER,
                            args: vec![account, parent],
                            span: Span::default(),
                        },
                        CExpr::Compare {
                            op: CmpOp::Eq,
                            left: Box::new(col("account")),
                            right: Box::new(str_("Income:US:Hoogle:Salary")),
                        },
                    ]),
                ]));
                query.plan.execution.scope = account_scope(&query.plan).filter(|_| scoped);
                if scoped {
                    let scope = query.plan.execution.scope.as_ref().map(|it| it.to_string());
                    assert_eq!(scope, Some(format!("under '{ancestor}', 'Income:US:Hoogle:Salary'")));
                }
                let result = query.execute_with_options(&ledger, &crate::Params::new(), &options).unwrap();
                format!("{:?}", result.rows)
            };
            let scoped = run(true);
            assert_eq!(scoped, run(false), "{ancestor}");
            counts.push(scoped.matches("Date(").count());
        }
        // 'Assets:U' is not an ancestor of 'Assets:US': like 'Nowhere', only the salary is left
        assert!(
            counts[0] > counts[1] && counts[1] > counts[2] && counts[2] == counts[3] && counts[3] > 0,
            "{counts:?}"
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
