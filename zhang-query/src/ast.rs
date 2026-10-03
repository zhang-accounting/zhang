//! The syntax tree produced by the parser.

use bigdecimal::BigDecimal;
use chrono::NaiveDate;

use crate::error::Span;
use crate::params::ParamRef;

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Literal {
    Null,
    Bool(bool),
    Int(i64),
    Decimal(BigDecimal),
    Str(String),
    Date(NaiveDate),
}

impl Literal {
    /// The same literal: decimals keep their scale, since it is part of their value's text
    /// (`1.0` and `1.00` are equal numbers, but `number * 1.0` and `number * 1.00` print
    /// differently).
    fn same_as(&self, other: &Literal) -> bool {
        match (self, other) {
            (Literal::Decimal(a), Literal::Decimal(b)) => a.as_bigint_and_exponent() == b.as_bigint_and_exponent(),
            (a, b) => a == b,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UnaryOp {
    Neg,
    Not,
}

/// `AND` / `OR`, parsed into n-ary [`ExprKind::Logical`] nodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LogicalOp {
    And,
    Or,
}

impl LogicalOp {
    pub fn symbol(&self) -> &'static str {
        match self {
            LogicalOp::And => "AND",
            LogicalOp::Or => "OR",
        }
    }
}

/// Arithmetic operators, parsed into left-to-right [`ExprKind::Arith`] chains.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ArithOp {
    Add,
    Sub,
    Mul,
    Div,
}

impl ArithOp {
    pub fn symbol(&self) -> &'static str {
        match self {
            ArithOp::Add => "+",
            ArithOp::Sub => "-",
            ArithOp::Mul => "*",
            ArithOp::Div => "/",
        }
    }
}

/// Comparison and match operators; these never chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BinaryOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    /// `~`: case-insensitive regular-expression search
    Match,
    /// `!~`
    NotMatch,
    /// `?~`: case-sensitive regular-expression search with the pattern on the LEFT
    /// (`'^Assets' ?~ account`), as in beanquery
    MatchCase,
}

impl BinaryOp {
    pub fn symbol(&self) -> &'static str {
        match self {
            BinaryOp::Eq => "=",
            BinaryOp::Ne => "!=",
            BinaryOp::Lt => "<",
            BinaryOp::Le => "<=",
            BinaryOp::Gt => ">",
            BinaryOp::Ge => ">=",
            BinaryOp::Match => "~",
            BinaryOp::NotMatch => "!~",
            BinaryOp::MatchCase => "?~",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum InTarget {
    /// `x IN (a, b, c)`
    List(Vec<Expr>),
    /// `x IN tags`
    Expr(Box<Expr>),
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ExprKind {
    Literal(Literal),
    Param(ParamRef),
    /// a bare identifier, lower-cased
    Column(String),
    /// a function call; `star` for `count(*)`; the name is lower-cased
    Call {
        name: String,
        args: Vec<Expr>,
        star: bool,
    },
    Unary(UnaryOp, Box<Expr>),
    /// a comparison or a match: `a = b`, `a ~ b`
    Binary(BinaryOp, Box<Expr>, Box<Expr>),
    /// `a AND b AND ...` or `a OR b OR ...` with at least two operands; n-ary, so long
    /// chains such as lists of accounts never nest
    Logical(LogicalOp, Vec<Expr>),
    /// `first op1 x1 op2 x2 ...` with at least one operator, evaluated left to right (operators
    /// of one precedence level; higher-precedence chains are nested operands)
    Arith(Box<Expr>, Vec<(ArithOp, Expr)>),
    In {
        needle: Box<Expr>,
        haystack: InTarget,
        negated: bool,
    },
    IsNull {
        expr: Box<Expr>,
        negated: bool,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Expr {
    pub kind: ExprKind,
    pub span: Span,
    /// height of this subtree (a leaf is 1); bounded by the parser so that the recursive
    /// passes over the tree (compile, evaluate, drop) cannot overflow the stack
    pub height: usize,
}

impl ExprKind {
    /// The largest height among the direct children.
    fn children_height(&self) -> usize {
        let max = |exprs: &[Expr]| exprs.iter().map(|it| it.height).max().unwrap_or(0);
        match self {
            ExprKind::Literal(_) | ExprKind::Param(_) | ExprKind::Column(_) => 0,
            ExprKind::Call { args, .. } => max(args),
            ExprKind::Unary(_, inner) => inner.height,
            ExprKind::Binary(_, left, right) => left.height.max(right.height),
            ExprKind::Logical(_, operands) => max(operands),
            ExprKind::Arith(first, rest) => rest.iter().map(|(_, it)| it.height).max().unwrap_or(0).max(first.height),
            ExprKind::In { needle, haystack, .. } => needle.height.max(match haystack {
                InTarget::List(items) => max(items),
                InTarget::Expr(expr) => expr.height,
            }),
            ExprKind::IsNull { expr, .. } => expr.height,
        }
    }
}

impl Expr {
    pub fn new(kind: ExprKind, span: Span) -> Expr {
        let height = kind.children_height() + 1;
        Expr { kind, span, height }
    }

    /// Structural equality ignoring source positions, used to match `GROUP BY` / `ORDER BY`
    /// expressions against the targets.
    pub fn same_as(&self, other: &Expr) -> bool {
        match (&self.kind, &other.kind) {
            (ExprKind::Literal(a), ExprKind::Literal(b)) => a.same_as(b),
            (ExprKind::Param(a), ExprKind::Param(b)) => a == b,
            (ExprKind::Column(a), ExprKind::Column(b)) => a == b,
            (
                ExprKind::Call {
                    name: a,
                    args: a_args,
                    star: a_star,
                },
                ExprKind::Call {
                    name: b,
                    args: b_args,
                    star: b_star,
                },
            ) => a == b && a_star == b_star && same_list(a_args, b_args),
            (ExprKind::Unary(a_op, a), ExprKind::Unary(b_op, b)) => a_op == b_op && a.same_as(b),
            (ExprKind::Binary(a_op, a_l, a_r), ExprKind::Binary(b_op, b_l, b_r)) => a_op == b_op && a_l.same_as(b_l) && a_r.same_as(b_r),
            (ExprKind::Logical(a_op, a), ExprKind::Logical(b_op, b)) => a_op == b_op && same_list(a, b),
            (ExprKind::Arith(a_first, a_rest), ExprKind::Arith(b_first, b_rest)) => {
                a_first.same_as(b_first) && a_rest.len() == b_rest.len() && a_rest.iter().zip(b_rest).all(|((a_op, a), (b_op, b))| a_op == b_op && a.same_as(b))
            }
            (
                ExprKind::In {
                    needle: a,
                    haystack: a_h,
                    negated: a_n,
                },
                ExprKind::In {
                    needle: b,
                    haystack: b_h,
                    negated: b_n,
                },
            ) => {
                a_n == b_n
                    && a.same_as(b)
                    && match (a_h, b_h) {
                        (InTarget::List(a), InTarget::List(b)) => same_list(a, b),
                        (InTarget::Expr(a), InTarget::Expr(b)) => a.same_as(b),
                        _ => false,
                    }
            }
            (ExprKind::IsNull { expr: a, negated: a_n }, ExprKind::IsNull { expr: b, negated: b_n }) => a_n == b_n && a.same_as(b),
            _ => false,
        }
    }

    /// The integer literal, if this expression is one (used for `GROUP BY 1`).
    pub fn as_index(&self) -> Option<i64> {
        match &self.kind {
            ExprKind::Literal(Literal::Int(idx)) => Some(*idx),
            _ => None,
        }
    }

    /// The identifier, if this expression is a bare column name.
    pub fn as_identifier(&self) -> Option<&str> {
        match &self.kind {
            ExprKind::Column(name) => Some(name),
            _ => None,
        }
    }
}

fn same_list(a: &[Expr], b: &[Expr]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(a, b)| a.same_as(b))
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Target {
    pub expr: Expr,
    pub alias: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Targets {
    /// `SELECT *`
    Wildcard,
    List(Vec<Target>),
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct OrderItem {
    pub expr: Expr,
    pub descending: bool,
}

/// The accounting-period modifiers of a FROM clause: `[OPEN ON date] [CLOSE [ON date]] [CLEAR]`.
///
/// Each date is a date literal or a parameter.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Period {
    pub open: Option<Expr>,
    /// `Some(None)` for a bare `CLOSE`
    pub close: Option<Option<Expr>>,
    pub clear: bool,
}

/// The table of a `FROM #name` clause, as written (the compiler resolves it).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TableName {
    /// the name without `#`; table names are case-sensitive
    pub name: String,
    /// the whole `#name`
    pub span: Span,
    /// written without `#` (`FROM prices`)
    pub bare: bool,
}

/// A parsed `FROM` clause: a table, or a row filter and/or the period modifiers.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct FromClause {
    pub table: Option<TableName>,
    pub expr: Option<Expr>,
    pub period: Option<Period>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Select {
    pub distinct: bool,
    pub targets: Targets,
    /// `FROM #name`; `None` reads the postings
    pub table: Option<TableName>,
    pub from: Option<Expr>,
    /// `OPEN` / `CLOSE` / `CLEAR` of the FROM clause, applied to the postings before any filter
    pub period: Option<Period>,
    pub where_clause: Option<Expr>,
    pub group_by: Option<Vec<Expr>>,
    /// `HAVING expr`, which the grammar only accepts after a GROUP BY
    pub having: Option<Expr>,
    pub order_by: Option<Vec<OrderItem>>,
    /// `PIVOT BY a, b`: each a target name or a 1-based target index
    pub pivot_by: Option<[Expr; 2]>,
    pub limit: Option<Count>,
    /// `OFFSET n`, which the grammar only accepts after a LIMIT
    pub offset: Option<Count>,
}

/// The value of `LIMIT` or `OFFSET`: a non-negative integer literal, or a parameter bound to
/// an integer when the query is executed.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum CountValue {
    Literal(u64),
    Param(ParamRef),
}

/// `LIMIT` or `OFFSET` with the position of its value.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Count {
    pub value: CountValue,
    pub span: Span,
}

impl Count {
    #[cfg(test)]
    pub fn literal(&self) -> Option<u64> {
        match self.value {
            CountValue::Literal(value) => Some(value),
            CountValue::Param(_) => None,
        }
    }
}
