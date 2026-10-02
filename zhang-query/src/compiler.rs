//! Type resolution and planning: turns the syntax tree into a typed [`Plan`].

use regex::{Regex, RegexBuilder};

use crate::ast::{BinaryOp, Expr, ExprKind, InTarget, Literal, Select, Targets, UnaryOp};
use crate::error::{LocatedError, Span};
use crate::functions::aggregates::{is_aggregate, resolve_aggregate};
use crate::functions::{resolve_scalar, AggregateFunction, ScalarFunction};
use crate::params::{ParamRef, ParamTypes};
use crate::table::{column, ColumnDef, WILDCARD_COLUMNS};
use crate::value::{DataType, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ArithOp {
    Add,
    Sub,
    Mul,
    Div,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CmpOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

pub(crate) enum RegexPattern {
    /// a literal pattern, compiled once
    Static(Regex),
    /// a pattern computed per row
    Dynamic { expr: Box<CExpr>, case_insensitive: bool },
}

/// A typed, executable expression.
pub(crate) enum CExpr {
    Const(Value),
    Column(&'static ColumnDef),
    Param(ParamRef),
    Scalar {
        function: &'static ScalarFunction,
        args: Vec<CExpr>,
        span: Span,
    },
    /// the finished value of the aggregate with this index
    Aggregate(usize),
    WidenInt(Box<CExpr>),
    Neg(Box<CExpr>, Span),
    Not(Box<CExpr>),
    And(Box<CExpr>, Box<CExpr>),
    Or(Box<CExpr>, Box<CExpr>),
    Arith {
        op: ArithOp,
        left: Box<CExpr>,
        right: Box<CExpr>,
        span: Span,
    },
    Compare {
        op: CmpOp,
        left: Box<CExpr>,
        right: Box<CExpr>,
    },
    Regex {
        subject: Box<CExpr>,
        pattern: RegexPattern,
        negated: bool,
        span: Span,
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

pub(crate) struct Plan {
    /// visible targets first, then hidden ones added by GROUP BY / ORDER BY
    pub targets: Vec<PlannedTarget>,
    pub visible: usize,
    pub filter: Option<CExpr>,
    pub aggregates: Vec<AggregateCall>,
    /// `Some` for aggregate queries: the indexes of the targets forming the group key
    pub group_keys: Option<Vec<usize>>,
    /// (target index, descending)
    pub order: Vec<(usize, bool)>,
    pub distinct: bool,
    pub limit: Option<u64>,
    /// every parameter reference with its declared type, for checking bound values
    pub params: Vec<(ParamRef, DataType, Span)>,
}

/// Where an expression is compiled.
#[derive(Clone, Copy)]
enum Mode {
    /// a per-row expression; aggregates are rejected naming the clause
    Row(&'static str),
    /// a target: aggregates are extracted into [`Plan::aggregates`]
    Target,
}

#[derive(Default)]
struct ExprInfo {
    has_aggregate: bool,
    /// the first column referenced outside of an aggregate call
    bare_column: Option<(String, Span)>,
}

struct Compiler<'q> {
    src: &'q str,
    param_types: &'q ParamTypes,
    aggregates: Vec<AggregateCall>,
    params: Vec<(ParamRef, DataType, Span)>,
}

type Typed = (CExpr, DataType);

pub(crate) fn compile(src: &str, select: &Select, param_types: &ParamTypes) -> Result<Plan, LocatedError> {
    let mut compiler = Compiler {
        src,
        param_types,
        aggregates: vec![],
        params: vec![],
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

    fn plan(&mut self, select: &Select) -> Result<Plan, LocatedError> {
        // targets
        let target_exprs: Vec<(Expr, Option<String>)> = match &select.targets {
            Targets::Wildcard => WILDCARD_COLUMNS
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

        // FROM and WHERE are both row filters
        let mut filter = None;
        for (clause, expr) in [("FROM", &select.from), ("WHERE", &select.where_clause)] {
            if let Some(expr) = expr {
                let (compiled, ty) = self.expr(expr, Mode::Row(clause), &mut ExprInfo::default())?;
                if !matches!(ty, DataType::Bool | DataType::Null) {
                    return err(format!("{} expects a boolean expression, got {}", clause, ty), expr.span);
                }
                filter = Some(match filter {
                    None => compiled,
                    Some(previous) => CExpr::And(Box::new(previous), Box::new(compiled)),
                });
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

        Ok(Plan {
            targets,
            visible,
            filter,
            aggregates: std::mem::take(&mut self.aggregates),
            group_keys,
            order,
            distinct: select.distinct,
            limit: select.limit,
            params: std::mem::take(&mut self.params),
        })
    }

    fn target(&mut self, expr: &Expr, alias: Option<String>) -> Result<(PlannedTarget, Option<(String, Span)>), LocatedError> {
        let mut info = ExprInfo::default();
        let (compiled, ty) = self.expr(expr, Mode::Target, &mut info)?;
        let name = alias.unwrap_or_else(|| self.text(expr.span).to_owned());
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
            name: self.text(item.span).to_owned(),
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
        let typed = match &expr.kind {
            ExprKind::Literal(literal) => return Ok(literal_value(literal)),
            ExprKind::Param(param) => return self.param(param, span),
            ExprKind::Column(name) => return column_ref(name, span, info),
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
        };
        Ok(fold_constants(typed))
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
            BinaryOp::And | BinaryOp::Or => {
                expect_bool(op.symbol(), left_ty, left.span)?;
                expect_bool(op.symbol(), right_ty, right.span)?;
                let (l, r) = (Box::new(left_c), Box::new(right_c));
                Ok((if op == BinaryOp::And { CExpr::And(l, r) } else { CExpr::Or(l, r) }, DataType::Bool))
            }
            BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div => {
                let arith = match op {
                    BinaryOp::Add => ArithOp::Add,
                    BinaryOp::Sub => ArithOp::Sub,
                    BinaryOp::Mul => ArithOp::Mul,
                    _ => ArithOp::Div,
                };
                let Some(ty) = arith_type(arith, left_ty, right_ty) else {
                    return err(format!("operator {} is not supported for ({}, {})", op.symbol(), left_ty, right_ty), span);
                };
                Ok((
                    CExpr::Arith {
                        op: arith,
                        left: Box::new(left_c),
                        right: Box::new(right_c),
                        span,
                    },
                    ty,
                ))
            }
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
                    return err(format!("cannot compare {} with {}", left_ty, right_ty), span);
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
                let pattern = match pattern_c {
                    CExpr::Const(Value::Str(pattern)) => {
                        RegexPattern::Static(build_regex(&pattern, case_insensitive).map_err(|message| LocatedError::compile(message, pattern_span))?)
                    }
                    other => RegexPattern::Dynamic {
                        expr: Box::new(other),
                        case_insensitive,
                    },
                };
                Ok((
                    CExpr::Regex {
                        subject: Box::new(subject_c),
                        pattern,
                        negated: op == BinaryOp::NotMatch,
                        span,
                    },
                    DataType::Bool,
                ))
            }
        }
    }
}

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

fn column_ref(name: &str, span: Span, info: &mut ExprInfo) -> Result<Typed, LocatedError> {
    match column(name) {
        Some(def) => {
            if info.bare_column.is_none() {
                info.bare_column = Some((def.name.to_owned(), span));
            }
            Ok((CExpr::Column(def), def.ty))
        }
        None => {
            let hint = if crate::functions::SCALAR_FUNCTIONS.iter().any(|it| it.name == name) || is_aggregate(name) {
                format!("; did you mean {}(...)?", name)
            } else {
                String::new()
            };
            err(format!("unknown column '{}'{}", name, hint), span)
        }
    }
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

/// Evaluate a node whose operands are all constants once, at compile time, unless it reads
/// anything from the execution (today, prices, the row): `account ~ ('^Ex' + 'penses')`
/// then compiles its regular expression only once.
fn fold_constants((expr, ty): Typed) -> Typed {
    if !expr.is_foldable() {
        return (expr, ty);
    }
    match crate::executor::eval_constant(&expr) {
        Some(value) => (CExpr::Const(value), ty),
        None => (expr, ty),
    }
}

impl CExpr {
    /// Whether this node computes something from constant operands only.
    fn is_foldable(&self) -> bool {
        let constant = |expr: &CExpr| matches!(expr, CExpr::Const(_));
        match self {
            CExpr::Const(_) | CExpr::Column(_) | CExpr::Param(_) | CExpr::Aggregate(_) => false,
            CExpr::Scalar { args, .. } => args.iter().all(constant),
            CExpr::WidenInt(inner) | CExpr::Neg(inner, _) | CExpr::Not(inner) => constant(inner),
            CExpr::And(left, right) | CExpr::Or(left, right) => constant(left) && constant(right),
            CExpr::Arith { left, right, .. } | CExpr::Compare { left, right, .. } => constant(left) && constant(right),
            CExpr::Regex { subject, pattern, .. } => {
                constant(subject)
                    && match pattern {
                        RegexPattern::Static(_) => true,
                        RegexPattern::Dynamic { expr, .. } => constant(expr),
                    }
            }
            CExpr::InSet { needle, set, .. } => constant(needle) && constant(set),
            CExpr::InList { needle, items, .. } => constant(needle) && items.iter().all(constant),
            CExpr::IsNull { expr, .. } => constant(expr),
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
