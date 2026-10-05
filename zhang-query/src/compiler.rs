//! Type resolution and planning: turns the syntax tree into a typed [`Plan`].
//!
//! The query pipeline is: parse ([`crate::parser`]) → compile/bind (this module: resolve
//! names, check types, plan grouping and ordering) → optimise ([`crate::optimizer`]:
//! rule-based rewrites) → execute ([`crate::executor`]).

use std::collections::{BTreeSet, HashSet};
use std::fmt;
use std::sync::Arc;

use chrono::{Datelike, NaiveDate};
use regex::{Regex, RegexBuilder};

pub(crate) use crate::ast::ArithOp;
use crate::ast::{self, BinaryOp, Count, CountValue, Expr, ExprKind, InTarget, Literal, LogicalOp, Select, Targets, UnaryOp};
use crate::error::{LocatedError, Span};
use crate::functions::aggregates::{is_aggregate, resolve_aggregate};
use crate::functions::{resolve_scalar, AggregateFunction, AggregateKind, ScalarFunction};
use crate::params::{ParamRef, ParamTypes, Params};
use crate::period::{Period, PeriodDate};
use crate::table::{self, ColumnDef, Scope, Table, ACCOUNT_BALANCE_COLUMN, BALANCE_COLUMN, POSTINGS};
use crate::value::{DataType, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CmpOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

impl CmpOp {
    /// The operator with its operands swapped: `a < b` is `b > a`.
    pub(crate) fn flipped(self) -> CmpOp {
        match self {
            CmpOp::Lt => CmpOp::Gt,
            CmpOp::Le => CmpOp::Ge,
            CmpOp::Gt => CmpOp::Lt,
            CmpOp::Ge => CmpOp::Le,
            other => other,
        }
    }

    fn symbol(&self) -> &'static str {
        match self {
            CmpOp::Eq => "=",
            CmpOp::Ne => "!=",
            CmpOp::Lt => "<",
            CmpOp::Le => "<=",
            CmpOp::Gt => ">",
            CmpOp::Ge => ">=",
        }
    }
}

#[derive(Clone)]
pub(crate) enum RegexPattern {
    /// a constant pattern, compiled once by the optimizer
    Compiled(Regex),
    /// a pattern computed per row (cached by the executor)
    Dynamic(Box<CExpr>),
}

/// One `op operand` step of an arithmetic chain.
#[derive(Clone)]
pub(crate) struct ArithStep {
    pub op: ArithOp,
    pub operand: CExpr,
    /// from the start of the chain to the end of this operand, for error positions
    pub span: Span,
}

/// A typed, executable expression.
#[derive(Clone)]
pub(crate) enum CExpr {
    Const(Value),
    Column(&'static ColumnDef),
    /// the `balance` column (the running inventory of the rows produced so far, including
    /// the current one), or a linear function of it the optimizer turned into its own running
    /// sum. It is stateful, so it is never folded, and the executor provides it.
    Running(Running),
    Param(ParamRef),
    Scalar {
        function: &'static ScalarFunction,
        args: Vec<CExpr>,
        span: Span,
    },
    /// the finished value of the aggregate with this index
    Aggregate(usize),
    /// the value of the target with this index in the current result row: HAVING reads the
    /// GROUP BY keys and the aggregate targets of a finished group this way
    Target(usize),
    WidenInt(Box<CExpr>),
    Neg(Box<CExpr>, Span),
    Not(Box<CExpr>),
    /// n-ary, evaluated left to right with three-valued logic
    And(Vec<CExpr>),
    Or(Vec<CExpr>),
    /// `first op x op y ...` evaluated left to right
    Arith {
        first: Box<CExpr>,
        rest: Vec<ArithStep>,
    },
    Compare {
        op: CmpOp,
        left: Box<CExpr>,
        right: Box<CExpr>,
    },
    Regex {
        subject: Box<CExpr>,
        pattern: RegexPattern,
        case_insensitive: bool,
        negated: bool,
        span: Span,
        pattern_span: Span,
    },
    InSet {
        needle: Box<CExpr>,
        set: Box<CExpr>,
        negated: bool,
    },
    InList {
        needle: Box<CExpr>,
        items: Vec<CExpr>,
        negated: bool,
    },
    IsNull {
        expr: Box<CExpr>,
        negated: bool,
    },
    /// `CASE WHEN cond THEN value ... ELSE otherwise END`: the value of the first condition
    /// that is TRUE (a NULL condition is not), else `otherwise` (`NULL` when the query has no
    /// ELSE). Only the chosen value is evaluated.
    Case {
        branches: Vec<(CExpr, CExpr)>,
        otherwise: Box<CExpr>,
    },
    /// `x [NOT] IN (constants)` or `x [NOT] IN <constant set>` as a hash lookup, which the
    /// optimizer prepares from [`CExpr::InList`] / [`CExpr::InSet`] once their items are
    /// constants (literals, or parameters bound for one execution)
    InConst {
        needle: Box<CExpr>,
        set: Arc<ConstSet>,
        negated: bool,
    },
    /// a string test against a constant, prepared once by the optimizer from the call of
    /// `icontains`, `any_icontains` or `under` whose second argument is a constant string
    StrTest {
        subject: Box<CExpr>,
        test: Arc<StrTest>,
        span: Span,
    },
}

/// The constant items of a membership test, hashed: strings are looked up without copying
/// the needle, other values by their value (an int equals the decimal of the same number).
pub(crate) struct ConstSet {
    /// the items as written, for EXPLAIN; empty for the elements of a set
    items: Vec<Value>,
    /// whether the items are the elements of a set (`IN :tags`) rather than a list
    from_set: bool,
    strings: HashSet<String>,
    others: HashSet<Value>,
    /// a NULL item: a needle that matches no other item is then NULL, not FALSE
    has_null: bool,
}

impl ConstSet {
    /// The items of `x IN (a, b, ...)`.
    pub fn from_list(items: Vec<Value>) -> ConstSet {
        let mut set = ConstSet {
            items: vec![],
            from_set: false,
            strings: HashSet::new(),
            others: HashSet::new(),
            has_null: false,
        };
        for item in &items {
            match item {
                Value::Null => set.has_null = true,
                Value::Str(text) => {
                    set.strings.insert(text.clone());
                }
                other => {
                    set.others.insert(other.clone());
                }
            }
        }
        set.items = items;
        set
    }

    /// The elements of the set of `x IN <set>`.
    pub fn from_set(elements: &BTreeSet<String>) -> ConstSet {
        ConstSet {
            items: vec![],
            from_set: true,
            strings: elements.iter().cloned().collect(),
            others: HashSet::new(),
            has_null: false,
        }
    }

    pub fn contains_str(&self, needle: &str) -> bool {
        self.strings.contains(needle)
    }

    pub fn contains(&self, needle: &Value) -> bool {
        match needle {
            Value::Str(text) => self.strings.contains(text.as_str()),
            other => self.others.contains(other),
        }
    }

    pub fn has_null(&self) -> bool {
        self.has_null
    }

    /// The items as values: those of a list as written, or the elements of a set as one set.
    /// For scans restricted to the accounts of `account IN (...)` ([`crate::optimizer::account_scope`]).
    pub fn values(&self) -> Vec<Value> {
        if self.from_set {
            vec![Value::Set(self.strings.iter().cloned().collect())]
        } else {
            self.items.clone()
        }
    }
}

/// The ancestor of an `under(subject, ancestor)` call ([`CExpr::as_under`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UnderAncestor<'e> {
    /// a constant: the ancestor's name, or `None` for NULL, which no account is under
    Const(Option<&'e str>),
    /// a parameter, bound when the query executes
    Param(&'e ParamRef),
}

impl<'e> UnderAncestor<'e> {
    /// The ancestor's name with `params` bound; `None` when it is NULL.
    #[cfg(test)]
    pub fn bound(self, params: &'e Params) -> Option<&'e str> {
        match self {
            UnderAncestor::Const(name) => name,
            UnderAncestor::Param(param) => params.get(param).and_then(Value::as_str),
        }
    }
}

/// A string test of [`CExpr::StrTest`], with its constant prepared once.
pub(crate) struct StrTest {
    /// the function the test was prepared from (`icontains`, `any_icontains` or `under`)
    pub function: &'static ScalarFunction,
    /// the constant argument as written
    pub argument: String,
    pub kind: StrTestKind,
}

pub(crate) enum StrTestKind {
    /// `icontains(subject, needle)`: the needle lower-cased
    IContains(String),
    /// `any_icontains(set, needle)`: the needle lower-cased
    AnyIContains(String),
    /// `under(account, ancestor)`
    Under,
}

/// What a [`CExpr::Running`] total adds up, row by row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Running {
    /// the positions: the `balance` column
    Balance,
    /// `units(position)`, which sums to `units(balance)`
    Units,
    /// `cost(position)`, which sums to `cost(balance)`
    Cost,
    /// the positions of every row of the current row's account, whatever the filter: the
    /// `account_balance` column
    AccountBalance,
    /// `units(position)` per account, which sums to `units(account_balance)`
    AccountUnits,
    /// `cost(position)` per account, which sums to `cost(account_balance)`
    AccountCost,
}

impl Running {
    pub fn name(&self) -> &'static str {
        match self {
            Running::Balance => BALANCE_COLUMN,
            Running::Units => "units",
            Running::Cost => "cost",
            Running::AccountBalance => ACCOUNT_BALANCE_COLUMN,
            Running::AccountUnits => "account units",
            Running::AccountCost => "account cost",
        }
    }

    /// The expression the total stands for.
    pub fn expression(&self) -> &'static str {
        match self {
            Running::Balance => BALANCE_COLUMN,
            Running::Units => "units(balance)",
            Running::Cost => "cost(balance)",
            Running::AccountBalance => ACCOUNT_BALANCE_COLUMN,
            Running::AccountUnits => "units(account_balance)",
            Running::AccountCost => "cost(account_balance)",
        }
    }

    /// The column the total reads.
    pub fn column(&self) -> &'static str {
        match self {
            Running::Balance | Running::Units | Running::Cost => BALANCE_COLUMN,
            Running::AccountBalance | Running::AccountUnits | Running::AccountCost => ACCOUNT_BALANCE_COLUMN,
        }
    }

    /// Whether the total adds up the rows of every account separately, whatever the filter
    /// (`account_balance`), rather than the rows the filter selects (`balance`).
    pub fn per_account(&self) -> bool {
        self.column() == ACCOUNT_BALANCE_COLUMN
    }
}

/// How the executor applies LIMIT.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum LimitMode {
    /// on the finished rows, after ORDER BY and DISTINCT
    #[default]
    AfterSort,
    /// stop scanning once LIMIT rows are produced (no ORDER BY); DISTINCT rows are told
    /// apart while scanning
    StopScan,
    /// keep only the first LIMIT rows of the ORDER BY while scanning (no DISTINCT)
    TopK,
    /// aggregate only the first LIMIT groups (no ORDER BY, no DISTINCT)
    FirstGroups,
}

/// How the running totals (`balance`, `account_balance`) are materialized.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct RunningPlan {
    /// the main pass keeps the running state, because an expression it evaluates (the filter,
    /// for the account balances) reads it
    pub eager: bool,
    /// visible targets of a non-aggregate query that the main pass leaves empty: once ORDER
    /// BY and LIMIT have chosen the rows, one replay over the filtered rows in ledger order
    /// evaluates them for the chosen rows only
    pub deferred_targets: Vec<usize>,
    /// `first()` / `last()` aggregates that only remember which row they pick; the replay
    /// evaluates their argument at that row
    pub deferred_aggregates: Vec<usize>,
    /// the running totals the state keeps
    pub totals: Vec<Running>,
}

impl RunningPlan {
    /// Whether the plan reads a running total at all.
    pub fn used(&self) -> bool {
        !self.totals.is_empty()
    }

    pub fn replays(&self) -> bool {
        !self.deferred_targets.is_empty() || !self.deferred_aggregates.is_empty()
    }
}

/// The accounts whose rows of the `postings` table an execution reads: the plan's filter can
/// only hold for their rows (see [`crate::optimizer::account_scope`]), so the others are never
/// built. The filter still applies to the rows of these accounts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AccountScope {
    /// the union of these accounts
    pub accounts: Vec<ScopedAccount>,
}

/// Accounts of an [`AccountScope`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ScopedAccount {
    /// the accounts a value names (`account = x`, `account IN (x, ...)`, `account IN :set`)
    Named(ScopeValue),
    /// the accounts a value names, with their sub-accounts (`under(account, x)`)
    Under(ScopeValue),
}

/// The value that names the accounts of a [`ScopedAccount`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ScopeValue {
    /// a constant: a string, a set of strings, or NULL (no account)
    Const(Value),
    /// a string or set parameter, read when the query executes
    Param(ParamRef),
}

impl AccountScope {
    /// The rows of one execution, with the parameters bound.
    pub fn resolve(&self, params: &Params) -> Scope {
        let mut exact = BTreeSet::new();
        let mut subtrees = vec![];
        for account in &self.accounts {
            let (value, under) = match account {
                ScopedAccount::Named(value) => (value, false),
                ScopedAccount::Under(value) => (value, true),
            };
            let value = match value {
                ScopeValue::Const(value) => Some(value),
                ScopeValue::Param(param) => params.get(param),
            };
            match value {
                Some(Value::Str(name)) if under => subtrees.push(name.clone()),
                Some(Value::Str(name)) => {
                    exact.insert(name.clone());
                }
                Some(Value::Set(names)) if !under => exact.extend(names.iter().cloned()),
                // NULL names no account: the filter holds for no row
                _ => {}
            }
        }
        Scope::Accounts { exact, subtrees }
    }
}

/// `'Assets:Bank', :accounts, under 'Expenses'`
impl fmt::Display for AccountScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (idx, account) in self.accounts.iter().enumerate() {
            if idx > 0 {
                f.write_str(", ")?;
            }
            let (value, under) = match account {
                ScopedAccount::Named(value) => (value, ""),
                ScopedAccount::Under(value) => (value, "under "),
            };
            match value {
                ScopeValue::Const(value) => write!(f, "{}{}", under, CExpr::Const(value.clone()))?,
                ScopeValue::Param(param) => write!(f, "{}{}", under, param)?,
            }
        }
        Ok(())
    }
}

/// The execution strategy chosen for a plan. [`Execution::naive`] evaluates every expression
/// for every row and applies LIMIT to the sorted rows (stopping early only without ORDER BY
/// and DISTINCT), which is what the optimizer and projector decisions must agree with.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Execution {
    pub running: RunningPlan,
    pub limit: LimitMode,
    /// the linear functions of `balance` the optimizer turned into running sums
    pub rewrites: Vec<Running>,
    /// the accounts the rows are limited to; `None` reads every row
    pub scope: Option<AccountScope>,
    /// the targets of a top-k query ([`LimitMode::TopK`]) built only for the rows LIMIT and
    /// OFFSET keep: visible, no ORDER BY key, infallible and not reading a running total. The
    /// scan ranks every row by its keys alone.
    pub late_targets: Vec<usize>,
    /// the last date of the rows a table that generates its rows generates; `None` generates
    /// every row
    pub until: Option<DateBound>,
}

/// The last date the rows a filter keeps can have, from a conjunct of the filter such as
/// `date <= x` ([`crate::optimizer::date_bound`]). A table that generates its rows (the months
/// of `#budgets`) generates none after it; the filter still applies to the rows it generates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DateBound {
    /// a date constant, NULL, a date parameter, or today
    pub value: BoundValue,
    /// whether the value itself is excluded (`date < x`)
    pub exclusive: bool,
    /// whether the filter compares the month of the date (`yearmonth(date) = x`): every date
    /// of the last month passes
    pub month: bool,
}

/// The value of a [`DateBound`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BoundValue {
    /// a date or NULL
    Const(Value),
    /// a date parameter, read when the query executes
    Param(ParamRef),
    /// `today()`, or `yearmonth(today())`, which is no later: the date of the execution
    Today,
}

impl DateBound {
    /// The last date the rows of one execution can have, with the parameters bound and `today`
    /// the date of `today()`. A NULL bound holds for no row: nothing is generated.
    pub fn resolve(&self, params: &Params, today: NaiveDate) -> NaiveDate {
        let today = Value::Date(today);
        let value = match &self.value {
            BoundValue::Const(value) => Some(value),
            BoundValue::Param(param) => params.get(param),
            BoundValue::Today => Some(&today),
        };
        let Some(Value::Date(date)) = value else {
            return NaiveDate::MIN;
        };
        let date = if self.exclusive { date.pred_opt().unwrap_or(NaiveDate::MIN) } else { *date };
        if self.month {
            let next = date.with_day(1).and_then(|first| first.checked_add_months(chrono::Months::new(1)));
            next.and_then(|next| next.pred_opt()).unwrap_or(NaiveDate::MAX)
        } else {
            date
        }
    }
}

/// `date <= 2024-06-01`, `yearmonth(date) < :month`
impl fmt::Display for DateBound {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let subject = if self.month { "yearmonth(date)" } else { "date" };
        let op = if self.exclusive { "<" } else { "<=" };
        match &self.value {
            BoundValue::Const(value) => write!(f, "{} {} {}", subject, op, CExpr::Const(value.clone())),
            BoundValue::Param(param) => write!(f, "{} {} {}", subject, op, param),
            BoundValue::Today => write!(f, "{} {} today()", subject, op),
        }
    }
}

impl Execution {
    pub fn naive(plan: &Plan) -> Execution {
        let referenced = plan.referenced_columns();
        let totals = [Running::Balance, Running::AccountBalance]
            .into_iter()
            .filter(|total| plan.table.is_postings() && referenced.contains(total.column()))
            .collect::<Vec<_>>();
        Execution {
            running: RunningPlan {
                eager: !totals.is_empty(),
                totals,
                ..RunningPlan::default()
            },
            limit: if plan.group_keys.is_none() && plan.order.is_empty() && !plan.distinct {
                LimitMode::StopScan
            } else {
                LimitMode::AfterSort
            },
            rewrites: vec![],
            scope: None,
            late_targets: vec![],
            until: None,
        }
    }
}

/// An aggregate call extracted from a target.
#[derive(Clone)]
pub(crate) struct AggregateCall {
    pub function: &'static AggregateFunction,
    /// `None` for `count(*)`
    pub arg: Option<CExpr>,
}

#[derive(Clone)]
pub(crate) struct PlannedTarget {
    pub name: String,
    pub ty: DataType,
    pub expr: CExpr,
    pub is_aggregate: bool,
    pub span: Span,
}

/// `PIVOT BY`: the result is reshaped after LIMIT into one row per value of the `rows`
/// target and one column per value of the `columns` target (both visible target indexes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Pivot {
    pub rows: usize,
    pub columns: usize,
}

#[derive(Clone)]
pub(crate) struct Plan {
    /// the table the query reads (`FROM #name`; `postings` by default)
    pub table: &'static Table,
    /// visible targets first, then hidden ones added by GROUP BY / ORDER BY
    pub targets: Vec<PlannedTarget>,
    pub visible: usize,
    /// `OPEN` / `CLOSE` / `CLEAR` of the FROM clause: transforms the postings before the
    /// filters see them
    pub period: Option<Period>,
    /// the row filters as compiled: the FROM expression, then WHERE
    pub filters: Vec<CExpr>,
    /// the single row filter the executor applies, set by the optimizer from `filters`
    pub filter: Option<CExpr>,
    pub aggregates: Vec<AggregateCall>,
    /// `Some` for aggregate queries: the indexes of the targets forming the group key
    pub group_keys: Option<Vec<usize>>,
    /// `HAVING`: the finished groups for which it is not TRUE are dropped. It reads the
    /// group's targets ([`CExpr::Target`]) and aggregates, never a row.
    pub having: Option<CExpr>,
    /// (target index, descending)
    pub order: Vec<(usize, bool)>,
    pub distinct: bool,
    /// `LIMIT`: a literal or an integer parameter, resolved per execution ([`Plan::window`])
    pub limit: Option<Count>,
    /// `OFFSET`, only with a LIMIT
    pub offset: Option<Count>,
    /// `PIVOT BY`, applied to the rows LIMIT leaves
    pub pivot: Option<Pivot>,
    /// every parameter reference with its declared type, for checking bound values
    pub params: Vec<(ParamRef, DataType, Span)>,
    /// how the executor runs the plan, decided by the optimizer and the projector
    pub execution: Execution,
}

/// Where an expression is compiled.
#[derive(Clone, Copy)]
enum Mode {
    /// a per-row expression; aggregates are rejected naming the clause
    Row(&'static str),
    /// a target: aggregates are extracted into [`Plan::aggregates`]
    Target,
    /// the HAVING condition: like a target, but a subexpression equal to a target (a GROUP
    /// BY key or an aggregate target) reads its value ([`CExpr::Target`])
    Having,
}

/// What HAVING can read besides aggregates: the GROUP BY keys and aggregate targets of the
/// group.
struct HavingScope {
    /// (expression, target index, type, is an aggregate) of every key and aggregate target
    targets: Vec<(Expr, usize, DataType, bool)>,
    /// the names of the visible targets, for a hint when HAVING uses one
    names: Vec<String>,
}

#[derive(Default)]
struct ExprInfo {
    has_aggregate: bool,
    /// the first column referenced outside of an aggregate call
    bare_column: Option<(String, Span)>,
}

struct Compiler<'q> {
    src: &'q str,
    /// the table whose columns the query's names resolve to
    table: &'static Table,
    param_types: &'q ParamTypes,
    aggregates: Vec<AggregateCall>,
    /// the source of each aggregate call of `aggregates`, at the same index: a call that is
    /// written again (`last(balance)` in two targets) reuses the first one's accumulation
    aggregate_sources: Vec<(bool, Vec<Expr>)>,
    params: Vec<(ParamRef, DataType, Span)>,
    /// set while the HAVING condition is compiled
    having: Option<HavingScope>,
}

type Typed = (CExpr, DataType);

pub(crate) fn compile(src: &str, select: &Select, param_types: &ParamTypes) -> Result<Plan, LocatedError> {
    let table = match &select.table {
        None => &POSTINGS,
        Some(name) => match table::find(&name.name) {
            Some(table) => table,
            None => {
                let names = table::tables().iter().map(|table| format!("#{}", table.name)).collect::<Vec<_>>();
                let what = if name.bare {
                    format!("unknown column or table '{}'", name.name)
                } else {
                    format!("unknown table '#{}'", name.name)
                };
                return err(format!("{}; the tables are {}", what, names.join(", ")), name.span);
            }
        },
    };
    let mut compiler = Compiler {
        src,
        table,
        param_types,
        aggregates: vec![],
        aggregate_sources: vec![],
        params: vec![],
        having: None,
    };
    compiler.plan(select)
}

fn err<T>(message: impl Into<String>, span: Span) -> Result<T, LocatedError> {
    Err(LocatedError::compile(message, span))
}

impl Compiler<'_> {
    fn text(&self, span: Span) -> &str {
        self.src.get(span.start..span.end).unwrap_or("").trim()
    }

    /// The name of an unaliased target: its source text, or the rendering of the compiled
    /// expression when it has none (expressions synthesized by BALANCES and JOURNAL).
    fn name(&self, span: Span, compiled: &CExpr) -> String {
        match self.text(span) {
            "" => compiled.to_string(),
            text => text.to_owned(),
        }
    }

    fn plan(&mut self, select: &Select) -> Result<Plan, LocatedError> {
        // targets
        let target_exprs: Vec<(Expr, Option<String>)> = match &select.targets {
            Targets::Wildcard => self
                .table
                .wildcard
                .iter()
                .map(|name| (Expr::new(ExprKind::Column((*name).to_owned()), Span::default()), Some((*name).to_owned())))
                .collect(),
            Targets::List(targets) => targets.iter().map(|it| (it.expr.clone(), it.alias.clone())).collect(),
        };
        let mut targets = vec![];
        let mut target_asts = vec![];
        let mut bare_columns = vec![];
        for (expr, alias) in target_exprs {
            let (target, bare) = self.target(&expr, alias)?;
            targets.push(target);
            bare_columns.push(bare);
            target_asts.push(expr);
        }
        let visible = targets.len();

        // the period modifiers transform the postings; the FROM expression then filters the
        // transformed rows like WHERE (as in beanquery)
        let period = select.period.as_ref().map(|period| self.period(period)).transpose()?;

        // FROM and WHERE are both row filters; the optimizer merges them
        let mut filters = vec![];
        for (clause, expr) in [("FROM", &select.from), ("WHERE", &select.where_clause)] {
            if let Some(expr) = expr {
                let (compiled, ty) = self.expr(expr, Mode::Row(clause), &mut ExprInfo::default())?;
                if !matches!(ty, DataType::Bool | DataType::Null) {
                    return err(format!("{} expects a boolean expression, got {}", clause, ty), expr.span);
                }
                filters.push(compiled);
            }
        }

        // GROUP BY
        let group_by = match &select.group_by {
            Some(items) => {
                let mut keys = vec![];
                for item in items {
                    let idx = self.resolve_reference(item, "GROUP BY", &mut targets, &mut target_asts, &mut bare_columns, visible)?;
                    if targets[idx].is_aggregate {
                        return err(format!("cannot GROUP BY the aggregate '{}'", targets[idx].name), item.span);
                    }
                    if !keys.contains(&idx) {
                        keys.push(idx);
                    }
                }
                Some(keys)
            }
            None => None,
        };

        // ORDER BY
        let mut order = vec![];
        if let Some(items) = &select.order_by {
            for item in items {
                let idx = self.resolve_reference(&item.expr, "ORDER BY", &mut targets, &mut target_asts, &mut bare_columns, visible)?;
                if targets[idx].ty == DataType::Interval {
                    return err(format!("cannot order by '{}': intervals have no order", targets[idx].name), item.expr.span);
                }
                order.push((idx, item.descending));
            }
        }

        let group_keys = if !self.aggregates.is_empty() || group_by.is_some() {
            // without GROUP BY, the non-aggregate targets form the key (beanquery's implicit grouping)
            let keys = group_by.unwrap_or_else(|| (0..visible).filter(|idx| !targets[*idx].is_aggregate).collect());
            for idx in &keys {
                let target = &targets[*idx];
                if matches!(target.ty, DataType::Set | DataType::Inventory | DataType::Metas) {
                    return err(
                        format!("cannot group by '{}': values of type {} cannot be grouped", target.name, target.ty),
                        target.span,
                    );
                }
            }
            for (idx, target) in targets.iter().enumerate() {
                if target.is_aggregate {
                    if let Some((name, span)) = &bare_columns[idx] {
                        return err(
                            format!(
                                "column '{}' must be used inside an aggregate function, or be a separate target in GROUP BY",
                                name
                            ),
                            *span,
                        );
                    }
                } else if !keys.contains(&idx) && bare_columns[idx].is_some() {
                    // expressions without columns (constants, parameters) are the same for every row
                    return err(
                        format!(
                            "all non-aggregate targets must be covered by the GROUP BY clause of an aggregate query; '{}' is missing",
                            target.name
                        ),
                        target.span,
                    );
                }
            }
            Some(keys)
        } else {
            None
        };

        let having = match &select.having {
            Some(expr) => Some(self.having(expr, group_keys.as_deref(), &targets, &target_asts, visible)?),
            None => None,
        };
        let pivot = match &select.pivot_by {
            Some(columns) => Some(pivot(columns, &targets[..visible], group_keys.as_deref())?),
            None => None,
        };
        let limit = select.limit.as_ref().map(|count| self.count(count, "LIMIT")).transpose()?;
        let offset = select.offset.as_ref().map(|count| self.count(count, "OFFSET")).transpose()?;
        if let (
            Some(CountValue::Literal(limit)),
            Some(Count {
                value: CountValue::Literal(offset),
                span,
            }),
        ) = (limit.as_ref().map(|it| &it.value), &offset)
        {
            if limit.checked_add(*offset).is_none() {
                return err("OFFSET plus LIMIT is too large", *span);
            }
        }

        let mut plan = Plan {
            table: self.table,
            targets,
            visible,
            period,
            filters,
            filter: None,
            aggregates: std::mem::take(&mut self.aggregates),
            group_keys,
            having,
            order,
            distinct: select.distinct,
            limit,
            offset,
            pivot,
            params: std::mem::take(&mut self.params),
            execution: Execution::default(),
        };
        plan.execution = Execution::naive(&plan);
        Ok(plan)
    }

    /// Compile the HAVING condition of an aggregate query.
    ///
    /// As in beanquery it must use an aggregate function, and names are columns of the table,
    /// never target aliases. A column may only be read through a GROUP BY key: a
    /// subexpression equal to a target (a key, or an aggregate target, whose finished value
    /// is reused) reads that target of the group; any other column must be inside an
    /// aggregate function. (beanquery evaluates such a column on an arbitrary posting.)
    fn having(&mut self, expr: &Expr, keys: Option<&[usize]>, targets: &[PlannedTarget], target_asts: &[Expr], visible: usize) -> Result<CExpr, LocatedError> {
        let Some(keys) = keys else {
            return err("HAVING requires a GROUP BY clause", expr.span);
        };
        self.having = Some(HavingScope {
            targets: target_asts
                .iter()
                .zip(targets)
                .enumerate()
                .filter(|(idx, (_, target))| target.is_aggregate || keys.contains(idx))
                .map(|(idx, (ast, target))| (ast.clone(), idx, target.ty, target.is_aggregate))
                .collect(),
            names: targets[..visible].iter().map(|target| target.name.clone()).collect(),
        });
        let mut info = ExprInfo::default();
        let compiled = self.expr(expr, Mode::Having, &mut info);
        self.having = None;
        let (compiled, ty) = compiled?;
        if let Some((name, span)) = info.bare_column {
            return err(
                format!("column '{}' must be a GROUP BY key or be used inside an aggregate function in HAVING", name),
                span,
            );
        }
        if !info.has_aggregate {
            return err(
                "HAVING must use an aggregate function such as count(*) or sum(...); filter rows with WHERE",
                expr.span,
            );
        }
        if !matches!(ty, DataType::Bool | DataType::Null) {
            return err(format!("HAVING expects a boolean expression, got {}", ty), expr.span);
        }
        Ok(compiled)
    }

    /// In HAVING, an expression equal to a key or an aggregate target reads that target of the
    /// group; a target name that is not a column gets a hint. `None` compiles it as usual.
    fn having_reference(&self, expr: &Expr, info: &mut ExprInfo) -> Option<Result<Typed, LocatedError>> {
        let scope = self.having.as_ref()?;
        if let Some((_, idx, ty, is_aggregate)) = scope.targets.iter().find(|(ast, ..)| ast.same_as(expr)) {
            info.has_aggregate |= *is_aggregate;
            return Some(Ok((CExpr::Target(*idx), *ty)));
        }
        match &expr.kind {
            ExprKind::Column(name) if self.table.column(name).is_none() && scope.names.iter().any(|it| it.eq_ignore_ascii_case(name)) => Some(err(
                format!(
                    "unknown column '{}': HAVING cannot use target names; repeat the target's expression instead",
                    name
                ),
                expr.span,
            )),
            _ => None,
        }
    }

    /// The value of `LIMIT` / `OFFSET`: a literal, or a parameter declared as an integer.
    fn count(&mut self, count: &Count, clause: &str) -> Result<Count, LocatedError> {
        if let CountValue::Param(param) = &count.value {
            let (_, ty) = self.param(param, count.span)?;
            if ty != DataType::Int {
                return err(format!("{} expects an integer, but parameter {} is a {}", clause, param, ty), count.span);
            }
        }
        Ok(count.clone())
    }

    /// Compile `OPEN ON` / `CLOSE [ON]` / `CLEAR`.
    fn period(&mut self, period: &ast::Period) -> Result<Period, LocatedError> {
        let open = period.open.as_ref().map(|date| self.period_date(date, "OPEN ON")).transpose()?;
        let close = match &period.close {
            None => None,
            Some(None) => Some(None),
            Some(Some(date)) => Some(Some(self.period_date(date, "CLOSE ON")?)),
        };
        if let (Some(PeriodDate::Fixed(open)), Some(Some(PeriodDate::Fixed(close)))) = (&open, &close) {
            if close < open {
                let span = period.close.as_ref().and_then(Option::as_ref).map(|date| date.span).unwrap_or_default();
                return err(format!("the CLOSE date {} is before the OPEN date {}", close, open), span);
            }
        }
        Ok(Period {
            open,
            close,
            clear: period.clear,
        })
    }

    /// A date literal or a date parameter of the period modifiers.
    fn period_date(&mut self, date: &Expr, clause: &str) -> Result<PeriodDate, LocatedError> {
        match &date.kind {
            ExprKind::Literal(Literal::Date(date)) => Ok(PeriodDate::Fixed(*date)),
            ExprKind::Param(param) => match self.param(param, date.span)? {
                (_, DataType::Date) => Ok(PeriodDate::Param(param.clone(), date.span)),
                (_, ty) => err(format!("{} expects a date, but parameter {} is a {}", clause, param, ty), date.span),
            },
            _ => err(format!("{} expects a date literal or a date parameter", clause), date.span),
        }
    }

    fn target(&mut self, expr: &Expr, alias: Option<String>) -> Result<(PlannedTarget, Option<(String, Span)>), LocatedError> {
        let mut info = ExprInfo::default();
        let (compiled, ty) = self.expr(expr, Mode::Target, &mut info)?;
        let name = alias.unwrap_or_else(|| self.name(expr.span, &compiled));
        Ok((
            PlannedTarget {
                name,
                ty,
                expr: compiled,
                is_aggregate: info.has_aggregate,
                span: expr.span,
            },
            info.bare_column,
        ))
    }

    /// Resolve a GROUP BY / ORDER BY item to a target index: a 1-based target index, a
    /// target alias, an expression equal to a target, or else a new hidden target.
    fn resolve_reference(
        &mut self, item: &Expr, clause: &'static str, targets: &mut Vec<PlannedTarget>, target_asts: &mut Vec<Expr>,
        bare_columns: &mut Vec<Option<(String, Span)>>, visible: usize,
    ) -> Result<usize, LocatedError> {
        if let Some(index) = item.as_index() {
            if index < 1 || index as usize > visible {
                return err(
                    format!("{} index {} is out of range: the query has {} targets", clause, index, visible),
                    item.span,
                );
            }
            return Ok(index as usize - 1);
        }
        if let Some(name) = item.as_identifier() {
            if let Some(idx) = targets[..visible].iter().position(|target| target.name.eq_ignore_ascii_case(name)) {
                return Ok(idx);
            }
        }
        if let Some(idx) = target_asts.iter().position(|ast| ast.same_as(item)) {
            return Ok(idx);
        }
        let mode = if clause == "GROUP BY" { Mode::Row(clause) } else { Mode::Target };
        let mut info = ExprInfo::default();
        let (compiled, ty) = self.expr(item, mode, &mut info)?;
        targets.push(PlannedTarget {
            name: self.name(item.span, &compiled),
            ty,
            expr: compiled,
            is_aggregate: info.has_aggregate,
            span: item.span,
        });
        target_asts.push(item.clone());
        bare_columns.push(info.bare_column);
        Ok(targets.len() - 1)
    }

    /// Compile an expression. This function recurses once per tree level, so it only
    /// dispatches: the work of each node kind lives in separate functions to keep its stack
    /// frame small (the parser bounds the tree height by [`crate::MAX_DEPTH`]).
    fn expr(&mut self, expr: &Expr, mode: Mode, info: &mut ExprInfo) -> Result<Typed, LocatedError> {
        let span = expr.span;
        if let Mode::Having = mode {
            if let Some(reference) = self.having_reference(expr, info) {
                return reference;
            }
        }
        Ok(match &expr.kind {
            ExprKind::Literal(literal) => return Ok(literal_value(literal)),
            ExprKind::Param(param) => return self.param(param, span),
            ExprKind::Column(name) => return column_ref(self.table, name, span, mode, info),
            ExprKind::Call { name, args, star } => self.call(name, args, *star, span, mode, info)?,
            ExprKind::Unary(op, inner) => {
                let operand = self.expr(inner, mode, info)?;
                unary(*op, operand, inner.span, span)?
            }
            ExprKind::Binary(op, left, right) => {
                let left_typed = self.expr(left, mode, info)?;
                let right_typed = self.expr(right, mode, info)?;
                self.binary(*op, left_typed, left, right_typed, right, span)?
            }
            ExprKind::Logical(op, operands) => self.logical(*op, operands, mode, info)?,
            ExprKind::Arith(first, rest) => self.arith(first, rest, mode, info)?,
            ExprKind::In { needle, haystack, negated } => self.in_expr(needle, haystack, *negated, mode, info)?,
            ExprKind::IsNull { expr: inner, negated } => {
                let (compiled, _) = self.expr(inner, mode, info)?;
                (
                    CExpr::IsNull {
                        expr: Box::new(compiled),
                        negated: *negated,
                    },
                    DataType::Bool,
                )
            }
            ExprKind::Case { branches, otherwise } => self.case(branches, otherwise.as_deref(), mode, info)?,
        })
    }

    /// `CASE WHEN ... END`: every condition is a boolean, and the values have one type, NULL
    /// aside; integers are widened to decimals when other values are decimals.
    fn case(&mut self, branches: &[(Expr, Expr)], otherwise: Option<&Expr>, mode: Mode, info: &mut ExprInfo) -> Result<Typed, LocatedError> {
        let mut conditions = Vec::with_capacity(branches.len());
        let mut values = Vec::with_capacity(branches.len() + 1);
        for (when, then) in branches {
            let (condition, ty) = self.expr(when, mode, info)?;
            if !matches!(ty, DataType::Bool | DataType::Null) {
                return err(format!("a WHEN condition must be a boolean, got {}", ty), when.span);
            }
            conditions.push(condition);
            values.push((self.expr(then, mode, info)?, then.span));
        }
        let otherwise = match otherwise {
            Some(otherwise) => Some((self.expr(otherwise, mode, info)?, otherwise.span)),
            None => None,
        };
        // the type of the values: the first that is not NULL, or decimal when some are int
        // and the others decimal
        let mut ty = DataType::Null;
        for ((_, value_ty), value_span) in values.iter().chain(otherwise.iter()) {
            ty = match (ty, *value_ty) {
                (DataType::Null, other) | (other, DataType::Null) => other,
                (DataType::Int, DataType::Decimal) | (DataType::Decimal, DataType::Int) => DataType::Decimal,
                (a, b) if a == b => a,
                (a, b) => {
                    return err(
                        format!("the values of a CASE must have one type, but this one is {} and an earlier one {}", b, a),
                        *value_span,
                    )
                }
            };
        }
        let widen = |(value, value_ty): Typed| {
            if ty == DataType::Decimal && value_ty == DataType::Int {
                CExpr::WidenInt(Box::new(value))
            } else {
                value
            }
        };
        let branches = conditions
            .into_iter()
            .zip(values)
            .map(|(condition, (value, _))| (condition, widen(value)))
            .collect();
        let otherwise = otherwise.map_or(CExpr::Const(Value::Null), |(value, _)| widen(value));
        Ok((
            CExpr::Case {
                branches,
                otherwise: Box::new(otherwise),
            },
            ty,
        ))
    }

    fn logical(&mut self, op: LogicalOp, operands: &[Expr], mode: Mode, info: &mut ExprInfo) -> Result<Typed, LocatedError> {
        let mut compiled = Vec::with_capacity(operands.len());
        for operand in operands {
            let (c, ty) = self.expr(operand, mode, info)?;
            expect_bool(op.symbol(), ty, operand.span)?;
            compiled.push(c);
        }
        let expr = match op {
            LogicalOp::And => CExpr::And(compiled),
            LogicalOp::Or => CExpr::Or(compiled),
        };
        Ok((expr, DataType::Bool))
    }

    fn arith(&mut self, first: &Expr, rest: &[(ArithOp, Expr)], mode: Mode, info: &mut ExprInfo) -> Result<Typed, LocatedError> {
        let (first_c, mut ty) = self.expr(first, mode, info)?;
        let mut steps = Vec::with_capacity(rest.len());
        for (op, operand) in rest {
            let (operand_c, operand_ty) = self.expr(operand, mode, info)?;
            let span = Span::new(first.span.start, operand.span.end);
            ty = match arith_type(*op, ty, operand_ty) {
                Some(result) => result,
                None => return err(format!("operator {} is not supported for ({}, {})", op.symbol(), ty, operand_ty), span),
            };
            steps.push(ArithStep {
                op: *op,
                operand: operand_c,
                span,
            });
        }
        Ok((
            CExpr::Arith {
                first: Box::new(first_c),
                rest: steps,
            },
            ty,
        ))
    }

    fn param(&mut self, param: &ParamRef, span: Span) -> Result<Typed, LocatedError> {
        match self.param_types.get(param) {
            Some(ty) => {
                self.params.push((param.clone(), ty, span));
                Ok((CExpr::Param(param.clone()), ty))
            }
            None => err(format!("parameter {} is not bound", param), span),
        }
    }

    fn call(&mut self, name: &str, args: &[Expr], star: bool, span: Span, mode: Mode, info: &mut ExprInfo) -> Result<Typed, LocatedError> {
        if is_aggregate(name) {
            return self.aggregate(name, args, star, span, mode, info);
        }
        if star {
            return err(format!("{}(*) is not supported; only count(*) is", name), span);
        }
        if ROW_FUNCTIONS.contains(&name) && info.bare_column.is_none() {
            // metadata functions read the current row, like a column reference
            info.bare_column = Some((format!("{}(...)", name), span));
        }
        let mut compiled = Vec::with_capacity(args.len());
        let mut types = Vec::with_capacity(args.len());
        for arg in args {
            let (c, ty) = self.expr(arg, mode, info)?;
            compiled.push(c);
            types.push(ty);
        }
        scalar_call(name, compiled, &types, span)
    }

    fn in_expr(&mut self, needle: &Expr, haystack: &InTarget, negated: bool, mode: Mode, info: &mut ExprInfo) -> Result<Typed, LocatedError> {
        let (needle_c, needle_ty) = self.expr(needle, mode, info)?;
        let as_set = |set: CExpr, needle_c: CExpr| -> Result<Typed, LocatedError> {
            if !matches!(needle_ty, DataType::Str | DataType::Null) {
                return err(format!("IN over a set expects a string on the left, got {}", needle_ty), needle.span);
            }
            Ok((
                CExpr::InSet {
                    needle: Box::new(needle_c),
                    set: Box::new(set),
                    negated,
                },
                DataType::Bool,
            ))
        };
        match haystack {
            InTarget::Expr(set) => {
                let (set_c, set_ty) = self.expr(set, mode, info)?;
                if !matches!(set_ty, DataType::Set | DataType::Null) {
                    return err(format!("IN expects a set (such as tags) or a parenthesized list, got {}", set_ty), set.span);
                }
                as_set(set_c, needle_c)
            }
            InTarget::List(items) => {
                let mut compiled = vec![];
                for item in items {
                    let (item_c, item_ty) = self.expr(item, mode, info)?;
                    if items.len() == 1 && item_ty == DataType::Set {
                        return as_set(item_c, needle_c);
                    }
                    let (needle_ok, item_c) = coerce_comparable(needle_ty, item_c, item_ty, item.span)?;
                    if !needle_ok {
                        return err(format!("cannot compare {} with {} in IN list", needle_ty, item_ty), item.span);
                    }
                    compiled.push(item_c);
                }
                Ok((
                    CExpr::InList {
                        needle: Box::new(needle_c),
                        items: compiled,
                        negated,
                    },
                    DataType::Bool,
                ))
            }
        }
    }

    fn aggregate(&mut self, name: &str, args: &[Expr], star: bool, span: Span, mode: Mode, info: &mut ExprInfo) -> Result<Typed, LocatedError> {
        if let Mode::Row(clause) = mode {
            return err(format!("aggregate function {}() is not allowed in {}", name, clause), span);
        }
        let mut compiled = vec![];
        let mut types = vec![];
        for arg in args {
            // arguments are per-row expressions; nested aggregates are rejected
            let (c, ty) = self.expr(arg, Mode::Row("an aggregate argument"), &mut ExprInfo::default())?;
            compiled.push(c);
            types.push(ty);
        }
        let resolved = resolve_aggregate(name, star, &types).map_err(|message| LocatedError::compile(message, span))?;
        if matches!(resolved.function.kind, AggregateKind::Min | AggregateKind::Max) && types.first() == Some(&DataType::Interval) {
            return err(format!("{}() is not supported for intervals: intervals have no order", name), span);
        }
        let ty = resolved.function.returns.resolve(&types);
        info.has_aggregate = true;
        // the same function over the same arguments accumulates the same value: compute it once
        let same = self.aggregates.iter().zip(&self.aggregate_sources).position(|(call, (call_star, call_args))| {
            std::ptr::eq(call.function, resolved.function)
                && *call_star == star
                && call_args.len() == args.len()
                && call_args.iter().zip(args).all(|(a, b)| a.same_as(b))
        });
        if let Some(idx) = same {
            return Ok((CExpr::Aggregate(idx), ty));
        }
        let arg = widen(compiled, &resolved.widen).into_iter().next();
        self.aggregates.push(AggregateCall {
            function: resolved.function,
            arg,
        });
        self.aggregate_sources.push((star, args.to_vec()));
        Ok((CExpr::Aggregate(self.aggregates.len() - 1), ty))
    }

    /// Type-check a binary operator over already compiled operands; `left` and `right` are
    /// only used for error positions.
    fn binary(
        &mut self, op: BinaryOp, (left_c, left_ty): Typed, left: &Expr, (right_c, right_ty): Typed, right: &Expr, span: Span,
    ) -> Result<Typed, LocatedError> {
        match op {
            BinaryOp::Eq | BinaryOp::Ne | BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge => {
                let cmp = match op {
                    BinaryOp::Eq => CmpOp::Eq,
                    BinaryOp::Ne => CmpOp::Ne,
                    BinaryOp::Lt => CmpOp::Lt,
                    BinaryOp::Le => CmpOp::Le,
                    BinaryOp::Gt => CmpOp::Gt,
                    _ => CmpOp::Ge,
                };
                // a string literal compared with a date is read as a date
                let (left_c, left_ty) = coerce_date_literal(left_c, left_ty, right_ty, left.span)?;
                let (right_c, right_ty) = coerce_date_literal(right_c, right_ty, left_ty, right.span)?;
                let comparable =
                    left_ty == right_ty || left_ty == DataType::Null || right_ty == DataType::Null || (left_ty.is_numeric() && right_ty.is_numeric());
                if !comparable {
                    return err(format!("cannot compare {} with {}{}", left_ty, right_ty, compare_hint(left_ty, right_ty)), span);
                }
                if !matches!(cmp, CmpOp::Eq | CmpOp::Ne) {
                    let orderable = |ty: DataType| {
                        matches!(
                            ty,
                            DataType::Null | DataType::Bool | DataType::Int | DataType::Decimal | DataType::Str | DataType::Date
                        )
                    };
                    if !orderable(left_ty) || !orderable(right_ty) {
                        let hint = if left_ty == DataType::Interval || right_ty == DataType::Interval {
                            ": intervals have no order (1 month is neither more nor less than 30 days)"
                        } else {
                            ""
                        };
                        return err(
                            format!("operator {} is not supported for ({}, {}){}", op.symbol(), left_ty, right_ty, hint),
                            span,
                        );
                    }
                }
                Ok((
                    CExpr::Compare {
                        op: cmp,
                        left: Box::new(left_c),
                        right: Box::new(right_c),
                    },
                    DataType::Bool,
                ))
            }
            BinaryOp::Match | BinaryOp::NotMatch | BinaryOp::MatchCase => {
                for (ty, side) in [(left_ty, left), (right_ty, right)] {
                    if !matches!(ty, DataType::Str | DataType::Null) {
                        return err(format!("operator {} expects strings, got {}", op.symbol(), ty), side.span);
                    }
                }
                let case_insensitive = op != BinaryOp::MatchCase;
                // `x ~ pattern` and `x !~ pattern`, but beanquery's case-sensitive `pattern ?~ x`
                // takes the pattern on the left
                let (subject_c, pattern_c, pattern_span) = if op == BinaryOp::MatchCase {
                    (right_c, left_c, left.span)
                } else {
                    (left_c, right_c, right.span)
                };
                // the optimizer compiles constant patterns once
                Ok((
                    CExpr::Regex {
                        subject: Box::new(subject_c),
                        pattern: RegexPattern::Dynamic(Box::new(pattern_c)),
                        case_insensitive,
                        negated: op == BinaryOp::NotMatch,
                        span,
                        pattern_span,
                    },
                    DataType::Bool,
                ))
            }
        }
    }
}

/// Resolve the two columns of `PIVOT BY` (target names or 1-based indexes) to visible
/// targets, with beanquery's rules: the query is an aggregate query, the columns differ, and
/// the second one is a GROUP BY key (its values become the columns).
fn pivot(columns: &[Expr; 2], targets: &[PlannedTarget], group_keys: Option<&[usize]>) -> Result<Pivot, LocatedError> {
    let resolve = |item: &Expr| -> Result<usize, LocatedError> {
        if let Some(index) = item.as_index() {
            if index < 1 || index as usize > targets.len() {
                return err(
                    format!("PIVOT BY index {} is out of range: the query has {} targets", index, targets.len()),
                    item.span,
                );
            }
            return Ok(index as usize - 1);
        }
        let name = item.as_identifier().unwrap_or_default();
        targets
            .iter()
            .position(|target| target.name.eq_ignore_ascii_case(name))
            .map_or_else(|| err(format!("PIVOT BY column '{}' is not a target name", name), item.span), Ok)
    };
    let (rows, cols) = (resolve(&columns[0])?, resolve(&columns[1])?);
    let span = Span::new(columns[0].span.start, columns[1].span.end);
    let Some(keys) = group_keys else {
        return err("PIVOT BY needs an aggregate query: group the rows with GROUP BY", span);
    };
    if rows == cols {
        return err("the two PIVOT BY columns must be different targets", span);
    }
    if !keys.contains(&cols) {
        return err(
            format!("the second PIVOT BY column must be a GROUP BY key; '{}' is not", targets[cols].name),
            columns[1].span,
        );
    }
    if matches!(targets[rows].ty, DataType::Set | DataType::Inventory | DataType::Metas | DataType::Interval) {
        return err(
            format!(
                "cannot pivot by '{}': values of type {} cannot be pivoted",
                targets[rows].name, targets[rows].ty
            ),
            columns[0].span,
        );
    }
    // the pivoted columns are sorted by their value
    if targets[cols].ty == DataType::Interval {
        return err(format!("cannot pivot by '{}': intervals have no order", targets[cols].name), columns[1].span);
    }
    Ok(Pivot { rows, columns: cols })
}

/// Scalar functions that read the row being evaluated (its metadata): in grouped queries
/// they must be grouped or used inside an aggregate, like columns.
const ROW_FUNCTIONS: &[&str] = &["meta", "entry_meta", "any_meta", "meta_values", "entry_meta_values"];

fn literal_value(literal: &Literal) -> Typed {
    match literal {
        Literal::Null => (CExpr::Const(Value::Null), DataType::Null),
        Literal::Bool(it) => (CExpr::Const(Value::Bool(*it)), DataType::Bool),
        Literal::Int(it) => (CExpr::Const(Value::Int(*it)), DataType::Int),
        Literal::Decimal(it) => (CExpr::Const(Value::Decimal(it.clone())), DataType::Decimal),
        Literal::Str(it) => (CExpr::Const(Value::Str(it.clone())), DataType::Str),
        Literal::Date(it) => (CExpr::Const(Value::Date(*it)), DataType::Date),
    }
}

fn column_ref(table: &'static Table, name: &str, span: Span, mode: Mode, info: &mut ExprInfo) -> Result<Typed, LocatedError> {
    match table.column(name) {
        Some(def) => {
            if info.bare_column.is_none() {
                info.bare_column = Some((def.name.to_owned(), span));
            }
            if table.is_postings() && def.name == ACCOUNT_BALANCE_COLUMN {
                // a running total, which the filter may read: it does not depend on the filter
                return Ok((CExpr::Running(Running::AccountBalance), def.ty));
            }
            if !(table.is_postings() && def.name == BALANCE_COLUMN) {
                return Ok((CExpr::Column(def), def.ty));
            }
            if let Mode::Row(clause @ ("FROM" | "WHERE")) = mode {
                return err(
                    format!("balance cannot be used in {}: the running balance adds up the rows the filter selects", clause),
                    span,
                );
            }
            Ok((CExpr::Running(Running::Balance), def.ty))
        }
        None if name.contains('.') => attribute_error(table, name, span, mode, info),
        None => {
            let hint = if crate::functions::SCALAR_FUNCTIONS.iter().any(|it| it.name == name) || is_aggregate(name) {
                format!("; did you mean {}(...)?", name)
            } else {
                String::new()
            };
            if table.is_postings() {
                err(format!("unknown column '{}'{}", name, hint), span)
            } else {
                let columns = table.columns.iter().map(|column| column.name).collect::<Vec<_>>();
                err(
                    format!("unknown column '{}' in #{} (its columns are {}){}", name, table.name, columns.join(", "), hint),
                    span,
                )
            }
        }
    }
}

/// The error of a dotted name `base.attribute...` that names no column, for the longest prefix
/// that is a column: an unknown attribute of a structured column, or an attribute of a column
/// that has none, both reported at the attribute after that prefix. When no prefix is a
/// column, the first part is an unknown column. Iterative, so that no number of dots can
/// exhaust the stack (the parser also caps it, at [`crate::parser::MAX_NAME_PARTS`]).
fn attribute_error(table: &'static Table, name: &str, span: Span, mode: Mode, info: &mut ExprInfo) -> Result<Typed, LocatedError> {
    let dots = name.match_indices('.').map(|(idx, _)| idx).collect::<Vec<_>>();
    for (k, &dot) in dots.iter().enumerate().rev() {
        let base = &name[..dot];
        let end = dots.get(k + 1).copied().unwrap_or(name.len());
        let attribute = &name[dot + 1..end];
        let attribute_span = Span::new(span.start + dot + 1, span.start + end);
        let attributes = table.attributes(base);
        if !attributes.is_empty() {
            return err(
                format!("unknown attribute '{}' of {}; its attributes are {}", attribute, base, attributes.join(", ")),
                attribute_span,
            );
        }
        if let Some(column) = table.column(base) {
            return err(
                format!(
                    "{} is a {} and has no attributes, so {}.{} does not exist",
                    column.name, column.ty, column.name, attribute
                ),
                attribute_span,
            );
        }
    }
    let first = &name[..dots.first().copied().unwrap_or(name.len())];
    column_ref(table, first, Span::new(span.start, span.start + first.len()), mode, info)
}

fn scalar_call(name: &str, args: Vec<CExpr>, types: &[DataType], span: Span) -> Result<Typed, LocatedError> {
    let resolved = resolve_scalar(name, types).map_err(|message| LocatedError::compile(message, span))?;
    let ty = resolved.function.returns.resolve(types);
    Ok((
        CExpr::Scalar {
            function: resolved.function,
            args: widen(args, &resolved.widen),
            span,
        },
        ty,
    ))
}

fn unary(op: UnaryOp, (compiled, ty): Typed, operand_span: Span, span: Span) -> Result<Typed, LocatedError> {
    match op {
        UnaryOp::Neg => match ty {
            DataType::Null | DataType::Int | DataType::Decimal | DataType::Amount | DataType::Position | DataType::Inventory => {
                Ok((CExpr::Neg(Box::new(compiled), span), ty))
            }
            _ => err(format!("cannot negate a value of type {}", ty), span),
        },
        UnaryOp::Not => {
            expect_bool("NOT", ty, operand_span)?;
            Ok((CExpr::Not(Box::new(compiled)), DataType::Bool))
        }
    }
}

/// Upper bound for the compiled size of one regular expression (the `regex` default is
/// 10 MiB), so a single pattern cannot take much memory or compile time.
const REGEX_SIZE_LIMIT: usize = 1 << 20;

pub(crate) fn build_regex(pattern: &str, case_insensitive: bool) -> Result<Regex, String> {
    RegexBuilder::new(pattern)
        .case_insensitive(case_insensitive)
        .size_limit(REGEX_SIZE_LIMIT)
        .dfa_size_limit(REGEX_SIZE_LIMIT)
        .build()
        .map_err(|e| format!("invalid regular expression: {}", e))
}

fn widen(args: Vec<CExpr>, widen: &[bool]) -> Vec<CExpr> {
    args.into_iter()
        .zip(widen)
        .map(|(arg, widen)| if *widen { CExpr::WidenInt(Box::new(arg)) } else { arg })
        .collect()
}

fn expect_bool(op: &str, ty: DataType, span: Span) -> Result<(), LocatedError> {
    if matches!(ty, DataType::Bool | DataType::Null) {
        Ok(())
    } else {
        err(format!("{} expects a boolean operand, got {}", op, ty), span)
    }
}

/// A string literal compared with a date, read as a date by the one text-to-date rule
/// ([`crate::value::parse_date`], as `date(text)` reads it); a string it does not read is an error.
fn coerce_date_literal(expr: CExpr, ty: DataType, other: DataType, span: Span) -> Result<Typed, LocatedError> {
    if ty == DataType::Str && other == DataType::Date {
        if let CExpr::Const(Value::Str(text)) = &expr {
            return match crate::value::parse_date(text) {
                Some(date) => Ok((CExpr::Const(Value::Date(date)), DataType::Date)),
                None => err(format!("'{}' is not a valid date (expected YYYY-MM-DD, as date() reads it)", text), span),
            };
        }
    }
    Ok((expr, ty))
}

/// How to compare a value that carries a currency with a number, for the error message.
fn compare_hint(left: DataType, right: DataType) -> &'static str {
    let carrier = match (left, right) {
        (carrier, other) | (other, carrier) if other.is_numeric() && matches!(carrier, DataType::Amount | DataType::Position | DataType::Inventory) => carrier,
        _ => return "",
    };
    match carrier {
        DataType::Inventory => ": an inventory may hold several currencies; compare the number of one of them, e.g. number(only('USD', sum(position))) > 100",
        DataType::Position => ": compare its number, e.g. number(units(position)) > 100",
        _ => ": compare its number, e.g. number(price) > 100",
    }
}

/// Whether `needle` and an IN-list item are comparable, after coercing date literals.
fn coerce_comparable(needle: DataType, item: CExpr, item_ty: DataType, span: Span) -> Result<(bool, CExpr), LocatedError> {
    let (item, item_ty) = coerce_date_literal(item, item_ty, needle, span)?;
    let ok = needle == item_ty || needle == DataType::Null || item_ty == DataType::Null || (needle.is_numeric() && item_ty.is_numeric());
    Ok((ok, item))
}

/// The result type of an arithmetic operator, or `None` if unsupported.
pub(crate) fn arith_type(op: ArithOp, left: DataType, right: DataType) -> Option<DataType> {
    use DataType::*;
    match (left, right) {
        (Null, Null) => Some(Null),
        (Null, other) | (other, Null) => match other {
            Int | Decimal | Amount | Date => Some(if op == ArithOp::Div && other == Int { Decimal } else { other }),
            Interval if matches!(op, ArithOp::Add | ArithOp::Sub) => Some(Interval),
            Str if op == ArithOp::Add => Some(Str),
            _ => None,
        },
        (Int, Int) => Some(if op == ArithOp::Div { Decimal } else { Int }),
        (Int | Decimal, Int | Decimal) => Some(Decimal),
        (Str, Str) if op == ArithOp::Add => Some(Str),
        (Date, Int) if matches!(op, ArithOp::Add | ArithOp::Sub) => Some(Date),
        (Int, Date) if op == ArithOp::Add => Some(Date),
        (Date, Date) if op == ArithOp::Sub => Some(Int),
        // beanquery's date arithmetic with relativedelta intervals
        (Date, Interval) if matches!(op, ArithOp::Add | ArithOp::Sub) => Some(Date),
        (Interval, Date) if op == ArithOp::Add => Some(Date),
        (Interval, Interval) if matches!(op, ArithOp::Add | ArithOp::Sub) => Some(Interval),
        (Amount, Int | Decimal) if matches!(op, ArithOp::Mul | ArithOp::Div) => Some(Amount),
        (Int | Decimal, Amount) if op == ArithOp::Mul => Some(Amount),
        (Amount, Amount) if matches!(op, ArithOp::Add | ArithOp::Sub) => Some(Amount),
        _ => None,
    }
}

impl CExpr {
    /// Rebuild this node with every direct child passed through `f` (used by the optimizer's
    /// bottom-up rewrites).
    pub(crate) fn map_children<E>(self, f: &mut impl FnMut(CExpr) -> Result<CExpr, E>) -> Result<CExpr, E> {
        let boxed = |expr: Box<CExpr>, f: &mut dyn FnMut(CExpr) -> Result<CExpr, E>| f(*expr).map(Box::new);
        Ok(match self {
            leaf @ (CExpr::Const(_) | CExpr::Column(_) | CExpr::Running(_) | CExpr::Param(_) | CExpr::Aggregate(_) | CExpr::Target(_)) => leaf,
            CExpr::Scalar { function, args, span } => CExpr::Scalar {
                function,
                args: args.into_iter().map(&mut *f).collect::<Result<_, _>>()?,
                span,
            },
            CExpr::WidenInt(inner) => CExpr::WidenInt(boxed(inner, f)?),
            CExpr::Neg(inner, span) => CExpr::Neg(boxed(inner, f)?, span),
            CExpr::Not(inner) => CExpr::Not(boxed(inner, f)?),
            CExpr::And(operands) => CExpr::And(operands.into_iter().map(&mut *f).collect::<Result<_, _>>()?),
            CExpr::Or(operands) => CExpr::Or(operands.into_iter().map(&mut *f).collect::<Result<_, _>>()?),
            CExpr::Arith { first, rest } => {
                let first = boxed(first, f)?;
                let mut steps = Vec::with_capacity(rest.len());
                for step in rest {
                    steps.push(ArithStep {
                        op: step.op,
                        operand: f(step.operand)?,
                        span: step.span,
                    });
                }
                CExpr::Arith { first, rest: steps }
            }
            CExpr::Compare { op, left, right } => CExpr::Compare {
                op,
                left: boxed(left, f)?,
                right: boxed(right, f)?,
            },
            CExpr::Regex {
                subject,
                pattern,
                case_insensitive,
                negated,
                span,
                pattern_span,
            } => CExpr::Regex {
                subject: boxed(subject, f)?,
                pattern: match pattern {
                    RegexPattern::Dynamic(expr) => RegexPattern::Dynamic(boxed(expr, f)?),
                    compiled => compiled,
                },
                case_insensitive,
                negated,
                span,
                pattern_span,
            },
            CExpr::InSet { needle, set, negated } => CExpr::InSet {
                needle: boxed(needle, f)?,
                set: boxed(set, f)?,
                negated,
            },
            CExpr::InList { needle, items, negated } => CExpr::InList {
                needle: boxed(needle, f)?,
                items: items.into_iter().map(&mut *f).collect::<Result<_, _>>()?,
                negated,
            },
            CExpr::IsNull { expr, negated } => CExpr::IsNull {
                expr: boxed(expr, f)?,
                negated,
            },
            CExpr::Case { branches, otherwise } => CExpr::Case {
                branches: branches
                    .into_iter()
                    .map(|(condition, value)| Ok((f(condition)?, f(value)?)))
                    .collect::<Result<_, _>>()?,
                otherwise: boxed(otherwise, f)?,
            },
            CExpr::InConst { needle, set, negated } => CExpr::InConst {
                needle: boxed(needle, f)?,
                set,
                negated,
            },
            CExpr::StrTest { subject, test, span } => CExpr::StrTest {
                subject: boxed(subject, f)?,
                test,
                span,
            },
        })
    }

    /// The direct children of this node.
    pub(crate) fn children(&self) -> Vec<&CExpr> {
        match self {
            CExpr::Const(_) | CExpr::Column(_) | CExpr::Running(_) | CExpr::Param(_) | CExpr::Aggregate(_) | CExpr::Target(_) => vec![],
            CExpr::Scalar { args, .. } => args.iter().collect(),
            CExpr::WidenInt(inner) | CExpr::Neg(inner, _) | CExpr::Not(inner) => vec![inner],
            CExpr::And(operands) | CExpr::Or(operands) => operands.iter().collect(),
            CExpr::Arith { first, rest } => std::iter::once(first.as_ref()).chain(rest.iter().map(|step| &step.operand)).collect(),
            CExpr::Compare { left, right, .. } => vec![left, right],
            CExpr::Regex { subject, pattern, .. } => match pattern {
                RegexPattern::Dynamic(expr) => vec![subject, expr],
                RegexPattern::Compiled(_) => vec![subject],
            },
            CExpr::InSet { needle, set, .. } => vec![needle, set],
            CExpr::InList { needle, items, .. } => std::iter::once(needle.as_ref()).chain(items.iter()).collect(),
            CExpr::IsNull { expr, .. } => vec![expr],
            CExpr::Case { branches, otherwise } => branches
                .iter()
                .flat_map(|(condition, value)| [condition, value])
                .chain(std::iter::once(otherwise.as_ref()))
                .collect(),
            CExpr::InConst { needle, .. } => vec![needle],
            CExpr::StrTest { subject, .. } => vec![subject],
        }
    }

    /// `under(subject, ancestor)` with its ancestor, however the plan holds the call: a call
    /// with a constant or a parameter as the ancestor, or the test the optimizer prepared from
    /// a constant ancestor ([`CExpr::StrTest`]). For scans restricted to the accounts under an
    /// ancestor ([`crate::optimizer::account_scope`]).
    pub(crate) fn as_under(&self) -> Option<(&CExpr, UnderAncestor<'_>)> {
        match self {
            CExpr::StrTest { subject, test, .. } if matches!(test.kind, StrTestKind::Under) => Some((subject, UnderAncestor::Const(Some(&test.argument)))),
            CExpr::Scalar { function, args, .. } if function.name == "under" => match args.as_slice() {
                [subject, CExpr::Const(Value::Str(ancestor))] => Some((subject, UnderAncestor::Const(Some(ancestor)))),
                [subject, CExpr::Const(Value::Null)] => Some((subject, UnderAncestor::Const(None))),
                [subject, CExpr::Param(param)] => Some((subject, UnderAncestor::Param(param))),
                _ => None,
            },
            _ => None,
        }
    }

    /// Add the names of the columns this expression reads to `columns`.
    pub(crate) fn collect_columns(&self, columns: &mut BTreeSet<&'static str>) {
        match self {
            CExpr::Column(def) => {
                columns.insert(def.name);
            }
            CExpr::Running(total) => {
                columns.insert(total.column());
            }
            _ => {}
        }
        for child in self.children() {
            child.collect_columns(columns);
        }
    }
}

/// `LIMIT` and `OFFSET` of one execution, with their parameters bound.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Window {
    pub limit: u64,
    pub offset: u64,
}

impl Window {
    /// How many leading rows an execution keeps before it skips the offset: OFFSET plus
    /// LIMIT, which [`Plan::window`] checked does not overflow.
    pub fn end(&self) -> u64 {
        self.offset.saturating_add(self.limit)
    }

    /// The rows the window keeps of `rows`: skip `offset`, then keep at most `limit`.
    pub fn apply<T>(&self, rows: &mut Vec<T>) {
        let offset = usize::try_from(self.offset).unwrap_or(usize::MAX).min(rows.len());
        rows.drain(..offset);
        rows.truncate(usize::try_from(self.limit).unwrap_or(usize::MAX));
    }
}

/// The value of a `LIMIT` / `OFFSET` with its parameter bound: a negative or NULL value is
/// an error at the parameter, never a wrap-around.
fn resolve_count(count: &Count, clause: &str, params: &Params) -> Result<u64, LocatedError> {
    match &count.value {
        CountValue::Literal(value) => Ok(*value),
        CountValue::Param(param) => match params.get(param) {
            Some(Value::Int(value)) => u64::try_from(*value)
                .map_err(|_| LocatedError::compile(format!("{} must not be negative, but parameter {} is {}", clause, param, value), count.span)),
            Some(Value::Null) | None => err(format!("{} expects an integer, but parameter {} is NULL", clause, param), count.span),
            Some(other) => err(
                format!("{} expects an integer, but parameter {} is a {}", clause, param, other.data_type()),
                count.span,
            ),
        },
    }
}

fn count_text(count: &Count) -> String {
    match &count.value {
        CountValue::Literal(value) => value.to_string(),
        CountValue::Param(param) => param.to_string(),
    }
}

impl Plan {
    /// LIMIT and OFFSET with `params` bound; `None` without a LIMIT.
    pub(crate) fn window(&self, params: &Params) -> Result<Option<Window>, LocatedError> {
        let Some(limit) = &self.limit else {
            return Ok(None);
        };
        let limit_value = resolve_count(limit, "LIMIT", params)?;
        let offset_value = match &self.offset {
            Some(offset) => resolve_count(offset, "OFFSET", params)?,
            None => 0,
        };
        if limit_value.checked_add(offset_value).is_none() {
            return err("OFFSET plus LIMIT is too large", self.offset.as_ref().map_or(limit.span, |it| it.span));
        }
        Ok(Some(Window {
            limit: limit_value,
            offset: offset_value,
        }))
    }

    /// The columns of the plan's table read anywhere in the plan (targets, filter, aggregate
    /// arguments), for a projection stage that only computes what is used.
    pub(crate) fn referenced_columns(&self) -> BTreeSet<&'static str> {
        let mut columns = BTreeSet::new();
        for target in &self.targets {
            target.expr.collect_columns(&mut columns);
        }
        for filter in self.filters.iter().chain(self.filter.as_ref()).chain(self.having.as_ref()) {
            filter.collect_columns(&mut columns);
        }
        for aggregate in &self.aggregates {
            if let Some(arg) = &aggregate.arg {
                arg.collect_columns(&mut columns);
            }
        }
        columns
    }
}

/// A compact, readable rendering used by `EXPLAIN`-style plan dumps.
impl fmt::Display for CExpr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fn list(f: &mut fmt::Formatter<'_>, items: &[CExpr], separator: &str) -> fmt::Result {
            for (idx, item) in items.iter().enumerate() {
                if idx > 0 {
                    f.write_str(separator)?;
                }
                write!(f, "{}", item)?;
            }
            Ok(())
        }
        match self {
            CExpr::Const(Value::Str(it)) => write!(f, "'{}'", it),
            CExpr::Const(Value::Date(it)) => write!(f, "{}", it),
            CExpr::Const(value) => write!(f, "{}", value),
            CExpr::Column(def) => f.write_str(def.name),
            CExpr::Running(running) => f.write_str(running.expression()),
            CExpr::Param(param) => write!(f, "{}", param),
            CExpr::Scalar { function, args, .. } => {
                write!(f, "{}(", function.name)?;
                list(f, args, ", ")?;
                f.write_str(")")
            }
            CExpr::Aggregate(idx) => write!(f, "agg#{}", idx),
            CExpr::Target(idx) => write!(f, "target#{}", idx),
            CExpr::WidenInt(inner) => write!(f, "decimal({})", inner),
            CExpr::Neg(inner, _) => write!(f, "-{}", inner),
            CExpr::Not(inner) => write!(f, "NOT {}", inner),
            CExpr::And(operands) => {
                f.write_str("(")?;
                list(f, operands, " AND ")?;
                f.write_str(")")
            }
            CExpr::Or(operands) => {
                f.write_str("(")?;
                list(f, operands, " OR ")?;
                f.write_str(")")
            }
            CExpr::Arith { first, rest } => {
                write!(f, "({}", first)?;
                for step in rest {
                    write!(f, " {} {}", step.op.symbol(), step.operand)?;
                }
                f.write_str(")")
            }
            CExpr::Compare { op, left, right } => write!(f, "({} {} {})", left, op.symbol(), right),
            CExpr::Regex {
                subject,
                pattern,
                case_insensitive,
                negated,
                ..
            } => {
                let op = if *negated { "!~" } else { "~" };
                match pattern {
                    RegexPattern::Compiled(regex) => write!(f, "({} {} /{}/{})", subject, op, regex.as_str(), if *case_insensitive { "i" } else { "" }),
                    RegexPattern::Dynamic(expr) => write!(f, "({} {} regex{}({}))", subject, op, if *case_insensitive { "_i" } else { "" }, expr),
                }
            }
            CExpr::InSet { needle, set, negated } => write!(f, "({} {}IN {})", needle, if *negated { "NOT " } else { "" }, set),
            CExpr::InList { needle, items, negated } => {
                write!(f, "({} {}IN (", needle, if *negated { "NOT " } else { "" })?;
                list(f, items, ", ")?;
                f.write_str("))")
            }
            CExpr::IsNull { expr, negated } => write!(f, "({} IS {}NULL)", expr, if *negated { "NOT " } else { "" }),
            CExpr::Case { branches, otherwise } => {
                f.write_str("CASE")?;
                for (condition, value) in branches {
                    write!(f, " WHEN {} THEN {}", condition, value)?;
                }
                write!(f, " ELSE {} END", otherwise)
            }
            CExpr::InConst { needle, set, negated } => {
                write!(f, "({} {}IN ", needle, if *negated { "NOT " } else { "" })?;
                if set.from_set {
                    let mut elements = set.strings.iter().collect::<Vec<_>>();
                    elements.sort();
                    write!(f, "{{{}}})", elements.iter().map(|it| format!("'{}'", it)).collect::<Vec<_>>().join(", "))
                } else {
                    let items = set.items.iter().cloned().map(CExpr::Const).collect::<Vec<_>>();
                    f.write_str("(")?;
                    list(f, &items, ", ")?;
                    f.write_str("))")
                }
            }
            CExpr::StrTest { subject, test, .. } => write!(f, "{}({}, '{}')", test.function.name, subject, test.argument),
        }
    }
}

/// An `EXPLAIN`-style dump of the plan, one clause per line. The first line names the table,
/// unless the plan reads the default `postings` table.
impl fmt::Display for Plan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if !self.table.is_postings() {
            writeln!(f, "table: #{}", self.table.name)?;
        }
        for (idx, target) in self.targets.iter().enumerate() {
            let hidden = if idx >= self.visible { " (hidden)" } else { "" };
            writeln!(f, "target {}: {} = {} : {}{}", idx, target.name, target.expr, target.ty, hidden)?;
        }
        for (idx, aggregate) in self.aggregates.iter().enumerate() {
            match &aggregate.arg {
                Some(arg) => writeln!(f, "agg#{}: {}({})", idx, aggregate.function.name, arg)?,
                None => writeln!(f, "agg#{}: {}(*)", idx, aggregate.function.name)?,
            }
        }
        if let Some(period) = &self.period {
            writeln!(f, "period: {}", period)?;
        }
        match &self.filter {
            Some(filter) => writeln!(f, "filter: {}", filter)?,
            None => {
                for filter in &self.filters {
                    writeln!(f, "filter: {}", filter)?;
                }
            }
        }
        if let Some(keys) = &self.group_keys {
            writeln!(f, "group by: {:?}", keys)?;
        }
        if let Some(having) = &self.having {
            writeln!(f, "having: {}", having)?;
        }
        if !self.order.is_empty() {
            let order = self
                .order
                .iter()
                .map(|(idx, descending)| format!("{} {}", idx, if *descending { "DESC" } else { "ASC" }))
                .collect::<Vec<_>>();
            writeln!(f, "order by: {}", order.join(", "))?;
        }
        if self.distinct {
            writeln!(f, "distinct")?;
        }
        if let Some(limit) = &self.limit {
            let how = match self.execution.limit {
                LimitMode::AfterSort => "",
                LimitMode::StopScan => " (stops the scan)",
                LimitMode::TopK => " (top-k while scanning)",
                LimitMode::FirstGroups => " (first groups only)",
            };
            let offset = self.offset.as_ref().map(|offset| format!(" offset {}", count_text(offset))).unwrap_or_default();
            writeln!(f, "limit: {}{}{}", count_text(limit), offset, how)?;
            if !self.execution.late_targets.is_empty() {
                writeln!(f, "late targets: {:?} (built for the kept rows only)", self.execution.late_targets)?;
            }
        }
        if let Some(pivot) = &self.pivot {
            writeln!(f, "pivot by: {} (rows), {} (columns)", pivot.rows, pivot.columns)?;
        }
        for rewrite in &self.execution.rewrites {
            writeln!(f, "rewrite: {} -> running {}", rewrite.expression(), rewrite.name())?;
        }
        if let Some(until) = &self.execution.until {
            writeln!(f, "generate: the rows up to {}", until)?;
        }
        if let Some(scope) = &self.execution.scope {
            writeln!(f, "scan: the rows of the accounts {}", scope)?;
        }
        let running = &self.execution.running;
        if running.used() {
            let mut how = vec![];
            if running.eager {
                how.push("running while scanning".to_owned());
            }
            if !running.deferred_targets.is_empty() {
                how.push(format!("deferred targets {:?}", running.deferred_targets));
            }
            for idx in &running.deferred_aggregates {
                how.push(format!("deferred agg#{}", idx));
            }
            let mut columns = running.totals.iter().map(Running::column).collect::<Vec<_>>();
            columns.dedup();
            writeln!(f, "{}: {}", columns.join(", "), how.join(", "))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::ParamTypes;

    /// Compile `SELECT <name> FROM #accounts` with a dotted name the parser would reject, on a
    /// thread with a 2 MiB stack.
    fn compile_dotted(parts: usize) -> LocatedError {
        std::thread::Builder::new()
            .stack_size(2 << 20)
            .spawn(move || {
                let src = "SELECT a FROM #accounts";
                let mut select = crate::parser::parse(src).unwrap();
                let name = vec!["a"; parts].join(".");
                let Targets::List(targets) = &mut select.targets else { panic!() };
                targets[0].expr = Expr::new(ExprKind::Column(name.clone()), Span::new(7, 7 + name.len()));
                compile(src, &select, &ParamTypes::default()).err().expect("an unknown column")
            })
            .unwrap()
            .join()
            .unwrap()
    }

    #[test]
    fn dotted_names_resolve_without_recursion() {
        for parts in [2, 10_000, 32_000] {
            let error = compile_dotted(parts);
            assert_eq!(
                error.message.lines().next().unwrap().split(" in #").next(),
                Some("unknown column 'a'"),
                "{parts}"
            );
            assert_eq!(error.span, Some(Span::new(7, 8)), "{parts}");
        }
    }
}
