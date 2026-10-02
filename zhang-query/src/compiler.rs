//! Type resolution and planning: turns the syntax tree into a typed [`Plan`].
//!
//! The query pipeline is: parse ([`crate::parser`]) → compile/bind (this module: resolve
//! names, check types, plan grouping and ordering) → optimise ([`crate::optimizer`]:
//! rule-based rewrites) → execute ([`crate::executor`]).

use std::collections::BTreeSet;
use std::fmt;

use regex::{Regex, RegexBuilder};

pub(crate) use crate::ast::ArithOp;
use crate::ast::{self, BinaryOp, Expr, ExprKind, InTarget, Literal, LogicalOp, Select, Targets, UnaryOp};
use crate::error::{LocatedError, Span};
use crate::functions::aggregates::{is_aggregate, resolve_aggregate};
use crate::functions::{resolve_scalar, AggregateFunction, ScalarFunction};
use crate::params::{ParamRef, ParamTypes};
use crate::period::{Period, PeriodDate};
use crate::table::{self, ColumnDef, Table, BALANCE_COLUMN, POSTINGS};
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

pub(crate) enum RegexPattern {
    /// a constant pattern, compiled once by the optimizer
    Compiled(Regex),
    /// a pattern computed per row (cached by the executor)
    Dynamic(Box<CExpr>),
}

/// One `op operand` step of an arithmetic chain.
pub(crate) struct ArithStep {
    pub op: ArithOp,
    pub operand: CExpr,
    /// from the start of the chain to the end of this operand, for error positions
    pub span: Span,
}

/// A typed, executable expression.
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
}

impl Running {
    pub fn name(&self) -> &'static str {
        match self {
            Running::Balance => BALANCE_COLUMN,
            Running::Units => "units",
            Running::Cost => "cost",
        }
    }

    /// The expression the total stands for.
    pub fn expression(&self) -> &'static str {
        match self {
            Running::Balance => BALANCE_COLUMN,
            Running::Units => "units(balance)",
            Running::Cost => "cost(balance)",
        }
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

/// How the running `balance` is materialized.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct RunningPlan {
    /// the main pass keeps the running state, because an expression it evaluates reads it
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

/// The execution strategy chosen for a plan. [`Execution::naive`] evaluates every expression
/// for every row and applies LIMIT to the sorted rows (stopping early only without ORDER BY
/// and DISTINCT), which is what the optimizer and projector decisions must agree with.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Execution {
    pub running: RunningPlan,
    pub limit: LimitMode,
    /// the linear functions of `balance` the optimizer turned into running sums
    pub rewrites: Vec<Running>,
}

impl Execution {
    pub fn naive(plan: &Plan) -> Execution {
        let running_balance = plan.table.is_postings() && plan.referenced_columns().contains(BALANCE_COLUMN);
        Execution {
            running: RunningPlan {
                eager: running_balance,
                totals: if running_balance { vec![Running::Balance] } else { vec![] },
                ..RunningPlan::default()
            },
            limit: if plan.group_keys.is_none() && plan.order.is_empty() && !plan.distinct {
                LimitMode::StopScan
            } else {
                LimitMode::AfterSort
            },
            rewrites: vec![],
        }
    }
}

/// An aggregate call extracted from a target.
pub(crate) struct AggregateCall {
    pub function: &'static AggregateFunction,
    /// `None` for `count(*)`
    pub arg: Option<CExpr>,
}

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
    pub limit: Option<u64>,
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
                order.push((idx, item.descending));
            }
        }

        let group_keys = if !self.aggregates.is_empty() || group_by.is_some() {
            // without GROUP BY, the non-aggregate targets form the key (beanquery's implicit grouping)
            let keys = group_by.unwrap_or_else(|| (0..visible).filter(|idx| !targets[*idx].is_aggregate).collect());
            for idx in &keys {
                let target = &targets[*idx];
                if matches!(target.ty, DataType::Set | DataType::Inventory) {
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
            limit: select.limit,
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
        })
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
        let ty = resolved.function.returns.resolve(&types);
        let arg = widen(compiled, &resolved.widen).into_iter().next();
        self.aggregates.push(AggregateCall {
            function: resolved.function,
            arg,
        });
        info.has_aggregate = true;
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
                        return err(format!("operator {} is not supported for ({}, {})", op.symbol(), left_ty, right_ty), span);
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
    if matches!(targets[rows].ty, DataType::Set | DataType::Inventory) {
        return err(
            format!(
                "cannot pivot by '{}': values of type {} cannot be pivoted",
                targets[rows].name, targets[rows].ty
            ),
            columns[0].span,
        );
    }
    Ok(Pivot { rows, columns: cols })
}

/// Scalar functions that read the row being evaluated (its metadata): in grouped queries
/// they must be grouped or used inside an aggregate, like columns.
const ROW_FUNCTIONS: &[&str] = &["meta", "entry_meta", "any_meta"];

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

/// The error of an attribute access `base.attribute` that names no column: an unknown
/// attribute of a structured column, an attribute of a column that has none (both at the
/// attribute), or an unknown `base` (at the base).
fn attribute_error(table: &'static Table, name: &str, span: Span, mode: Mode, info: &mut ExprInfo) -> Result<Typed, LocatedError> {
    let (base, attribute) = name.rsplit_once('.').expect("an attribute access");
    let attribute_span = Span::new(span.end - attribute.len(), span.end);
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
    column_ref(table, base, Span::new(span.start, span.start + base.len()), mode, info)
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

fn coerce_date_literal(expr: CExpr, ty: DataType, other: DataType, span: Span) -> Result<Typed, LocatedError> {
    if ty == DataType::Str && other == DataType::Date {
        if let CExpr::Const(Value::Str(text)) = &expr {
            return match chrono::NaiveDate::parse_from_str(text, "%Y-%m-%d") {
                Ok(date) => Ok((CExpr::Const(Value::Date(date)), DataType::Date)),
                Err(_) => err(format!("'{}' is not a valid date (expected YYYY-MM-DD)", text), span),
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
            Str if op == ArithOp::Add => Some(Str),
            _ => None,
        },
        (Int, Int) => Some(if op == ArithOp::Div { Decimal } else { Int }),
        (Int | Decimal, Int | Decimal) => Some(Decimal),
        (Str, Str) if op == ArithOp::Add => Some(Str),
        (Date, Int) if matches!(op, ArithOp::Add | ArithOp::Sub) => Some(Date),
        (Int, Date) if op == ArithOp::Add => Some(Date),
        (Date, Date) if op == ArithOp::Sub => Some(Int),
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
        }
    }

    /// Add the names of the columns this expression reads to `columns`.
    pub(crate) fn collect_columns(&self, columns: &mut BTreeSet<&'static str>) {
        match self {
            CExpr::Column(def) => {
                columns.insert(def.name);
            }
            CExpr::Running(_) => {
                columns.insert(BALANCE_COLUMN);
            }
            _ => {}
        }
        for child in self.children() {
            child.collect_columns(columns);
        }
    }
}

impl Plan {
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
        if let Some(limit) = self.limit {
            let how = match self.execution.limit {
                LimitMode::AfterSort => "",
                LimitMode::StopScan => " (stops the scan)",
                LimitMode::TopK => " (top-k while scanning)",
                LimitMode::FirstGroups => " (first groups only)",
            };
            writeln!(f, "limit: {}{}", limit, how)?;
        }
        if let Some(pivot) = &self.pivot {
            writeln!(f, "pivot by: {} (rows), {} (columns)", pivot.rows, pivot.columns)?;
        }
        for rewrite in &self.execution.rewrites {
            writeln!(f, "rewrite: {} -> running {}", rewrite.expression(), rewrite.name())?;
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
            writeln!(f, "balance: {}", how.join(", "))?;
        }
        Ok(())
    }
}
