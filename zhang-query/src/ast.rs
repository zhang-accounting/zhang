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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UnaryOp {
    Neg,
    Not,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
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
    And,
    Or,
}

impl BinaryOp {
    pub fn symbol(&self) -> &'static str {
        match self {
            BinaryOp::Add => "+",
            BinaryOp::Sub => "-",
            BinaryOp::Mul => "*",
            BinaryOp::Div => "/",
            BinaryOp::Eq => "=",
            BinaryOp::Ne => "!=",
            BinaryOp::Lt => "<",
            BinaryOp::Le => "<=",
            BinaryOp::Gt => ">",
            BinaryOp::Ge => ">=",
            BinaryOp::Match => "~",
            BinaryOp::NotMatch => "!~",
            BinaryOp::MatchCase => "?~",
            BinaryOp::And => "AND",
            BinaryOp::Or => "OR",
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
    Binary(BinaryOp, Box<Expr>, Box<Expr>),
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
            (ExprKind::Literal(a), ExprKind::Literal(b)) => a == b,
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

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Select {
    pub distinct: bool,
    pub targets: Targets,
    pub from: Option<Expr>,
    pub where_clause: Option<Expr>,
    pub group_by: Option<Vec<Expr>>,
    pub order_by: Option<Vec<OrderItem>>,
    pub limit: Option<u64>,
}
