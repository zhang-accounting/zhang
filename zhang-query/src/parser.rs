//! The query parser, written with `nom`.
//!
//! Grammar (keywords are case-insensitive):
//!
//! ```text
//! query      := SELECT [DISTINCT] targets [FROM expr] [WHERE expr]
//!               [GROUP BY item, ...] [ORDER BY item [ASC|DESC], ...] [LIMIT int] [;]
//! targets    := '*' | expr [AS name], ...
//! expr       := or
//! or         := and (OR and)*
//! and        := not (AND not)*
//! not        := NOT not | comparison
//! comparison := sum [ op sum | [NOT] IN in_target | IS [NOT] NULL ]
//!               op := = | == | != | <> | < | <= | > | >= | ~ | !~ | ?~
//! in_target  := '(' expr, ... ')' | sum
//! sum        := term (('+' | '-') term)*
//! term       := unary (('*' | '/') unary)*
//! unary      := '-' unary | '+' unary | primary
//! primary    := '(' expr ')' | literal | $n | :name | name '(' ['*' | expr, ...] ')' | name
//! literal    := 'string' | "string" | 2024-01-31 | 12 | 12.50 | TRUE | FALSE | NULL
//! ```
//!
//! `--` starts a comment that runs to the end of the line.

use std::borrow::Cow;
use std::cell::Cell;
use std::str::FromStr;

use bigdecimal::BigDecimal;
use chrono::NaiveDate;
use nom::bytes::complete::{tag, tag_no_case, take_while, take_while1};
use nom::error::{ErrorKind, ParseError};
use nom::{Err as NomErr, IResult};

use crate::ast::{BinaryOp, Expr, ExprKind, InTarget, Literal, OrderItem, Select, Target, Targets, UnaryOp};
use crate::error::{QueryError, QueryErrorKind, Span};
use crate::params::ParamRef;

#[derive(Debug, Clone)]
pub(crate) struct PError<'a> {
    input: &'a str,
    message: Cow<'static, str>,
}

impl<'a> ParseError<&'a str> for PError<'a> {
    fn from_error_kind(input: &'a str, _kind: ErrorKind) -> Self {
        PError {
            input,
            message: Cow::Borrowed(""),
        }
    }

    fn append(_input: &'a str, _kind: ErrorKind, other: Self) -> Self {
        other
    }

    fn or(self, other: Self) -> Self {
        // keep the error that got furthest into the input
        if other.input.len() <= self.input.len() {
            other
        } else {
            self
        }
    }
}

type PResult<'a, T> = IResult<&'a str, T, PError<'a>>;

const RESERVED: &[&str] = &[
    "select", "distinct", "from", "where", "group", "by", "order", "asc", "desc", "limit", "as", "and", "or", "not", "in", "is", "null", "true", "false",
    "having", "pivot",
];

fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_'
}

fn is_ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// Skip whitespace and `--` comments.
fn skip_ws(mut i: &str) -> &str {
    loop {
        let trimmed = i.trim_start();
        if let Some(comment) = trimmed.strip_prefix("--") {
            i = comment.find('\n').map(|idx| &comment[idx..]).unwrap_or("");
        } else {
            return trimmed;
        }
    }
}

fn error<'a, T>(input: &'a str, message: impl Into<Cow<'static, str>>) -> PResult<'a, T> {
    Err(NomErr::Error(PError {
        input,
        message: message.into(),
    }))
}

fn failure<'a, T>(input: &'a str, message: impl Into<Cow<'static, str>>) -> PResult<'a, T> {
    Err(NomErr::Failure(PError {
        input,
        message: message.into(),
    }))
}

/// Turn a recoverable error into a hard failure: used once a construct is committed.
fn cut<T>(result: PResult<'_, T>) -> PResult<'_, T> {
    result.map_err(|err| match err {
        NomErr::Error(err) => NomErr::Failure(err),
        other => other,
    })
}

/// A short description of what is at `i`, for error messages.
fn found(i: &str) -> String {
    let i = skip_ws(i);
    if i.is_empty() {
        return "end of query".to_owned();
    }
    let token: String = if i.starts_with(is_ident_char) {
        i.chars().take_while(|c| is_ident_char(*c)).take(24).collect()
    } else {
        i.chars().take(1).collect()
    };
    format!("'{}'", token)
}

fn keyword<'a>(word: &'static str) -> impl Fn(&'a str) -> PResult<'a, &'a str> {
    move |i: &'a str| {
        let i = skip_ws(i);
        let (rest, matched) = tag_no_case(word)(i)?;
        if rest.starts_with(is_ident_char) {
            return error(i, format!("expected {}", word.to_uppercase()));
        }
        Ok((rest, matched))
    }
}

fn is_keyword(i: &str, word: &'static str) -> bool {
    keyword(word)(i).is_ok()
}

fn symbol<'a>(sym: &'static str) -> impl Fn(&'a str) -> PResult<'a, &'a str> {
    move |i: &'a str| {
        let i = skip_ws(i);
        tag(sym)(i).or_else(|_: NomErr<PError<'a>>| error(i, format!("expected '{}', found {}", sym, found(i))))
    }
}

fn raw_identifier(i: &str) -> PResult<'_, &str> {
    let i = skip_ws(i);
    if !i.starts_with(is_ident_start) {
        return error(i, format!("expected a name, found {}", found(i)));
    }
    take_while1(is_ident_char)(i)
}

/// The longest accepted query text, in bytes.
pub const MAX_QUERY_LENGTH: usize = 64 * 1024;

/// The deepest accepted nesting: both how deeply the parser may recurse (parentheses,
/// function arguments, `NOT`, unary minus) and the height of the resulting syntax tree, so
/// that parsing, compiling and evaluating stay well within a 2 MiB thread stack even in
/// debug builds. Chains of `AND`, `OR`, `+` and `*` are built as balanced trees, so long
/// lists such as `account = 'a' OR account = 'b' OR ...` stay shallow; a chain mixing `-`
/// or `/` counts one level per operator.
pub const MAX_DEPTH: usize = 64;

#[cold]
fn too_deep(i: &str) -> NomErr<PError<'_>> {
    NomErr::Failure(PError {
        input: skip_ws(i),
        message: Cow::Owned(format!("the query is nested too deeply (at most {} levels)", MAX_DEPTH)),
    })
}

pub(crate) struct Parser<'s> {
    src: &'s str,
    /// current recursion depth
    depth: Cell<usize>,
}

/// Parse a complete query.
pub(crate) fn parse(src: &str) -> Result<Select, QueryError> {
    if src.len() > MAX_QUERY_LENGTH {
        return Err(QueryError::new(
            QueryErrorKind::Parse,
            format!("the query is too long ({} bytes; at most {} are accepted)", src.len(), MAX_QUERY_LENGTH),
        ));
    }
    let parser = Parser { src, depth: Cell::new(0) };
    match parser.select(src) {
        Ok((_, select)) => Ok(select),
        Err(NomErr::Error(err)) | Err(NomErr::Failure(err)) => {
            let message = if err.message.is_empty() {
                format!("syntax error near {}", found(err.input))
            } else {
                err.message.into_owned()
            };
            Err(QueryError::at(QueryErrorKind::Parse, message, src, src.len() - err.input.len()))
        }
        Err(NomErr::Incomplete(_)) => Err(QueryError::new(QueryErrorKind::Parse, "incomplete query")),
    }
}

impl<'s> Parser<'s> {
    fn offset(&self, i: &str) -> usize {
        self.src.len() - i.len()
    }

    fn select(&self, i: &'s str) -> PResult<'s, Select> {
        let i = skip_ws(i);
        let i = match keyword("select")(i) {
            Ok((rest, _)) => rest,
            Err(_) => {
                for statement in ["balances", "journal", "print"] {
                    if is_keyword(i, statement) {
                        return failure(i, format!("{} statements are not supported yet; only SELECT is", statement.to_uppercase()));
                    }
                }
                return failure(i, format!("expected SELECT, found {}", found(i)));
            }
        };
        let (i, distinct) = match keyword("distinct")(i) {
            Ok((rest, _)) => (rest, true),
            Err(_) => (i, false),
        };
        let (i, targets) = cut(self.targets(i))?;

        let (i, from) = match keyword("from")(i) {
            Ok((rest, _)) => {
                for unsupported in ["open", "close", "clear"] {
                    if is_keyword(rest, unsupported) {
                        return failure(skip_ws(rest), "FROM OPEN/CLOSE/CLEAR is not supported yet");
                    }
                }
                let table = skip_ws(rest);
                if table.starts_with('#') || (is_keyword(table, "postings") && !skip_ws(&table["postings".len()..]).starts_with(['=', '!', '<', '>', '~'])) {
                    return failure(
                        table,
                        "selecting a table with FROM is not supported yet; the query always reads postings, and FROM <expression> filters them",
                    );
                }
                let (rest, expr) = cut(self.expr(rest))?;
                (rest, Some(expr))
            }
            Err(_) => (i, None),
        };
        let (i, where_clause) = match keyword("where")(i) {
            Ok((rest, _)) => {
                let (rest, expr) = cut(self.expr(rest))?;
                (rest, Some(expr))
            }
            Err(_) => (i, None),
        };
        let (i, group_by) = match keyword("group")(i) {
            Ok((rest, _)) => {
                let (rest, _) = cut(keyword("by")(rest))?;
                let (rest, items) = cut(self.expr_list(rest))?;
                (rest, Some(items))
            }
            Err(_) => (i, None),
        };
        if is_keyword(i, "having") {
            return failure(skip_ws(i), "HAVING is not supported yet");
        }
        let (i, order_by) = match keyword("order")(i) {
            Ok((rest, _)) => {
                let (rest, _) = cut(keyword("by")(rest))?;
                let (rest, items) = cut(self.order_items(rest))?;
                (rest, Some(items))
            }
            Err(_) => (i, None),
        };
        if is_keyword(i, "pivot") {
            return failure(skip_ws(i), "PIVOT BY is not supported yet");
        }
        let (i, limit) = match keyword("limit")(i) {
            Ok((rest, _)) => {
                let rest = skip_ws(rest);
                let (after, digits) = take_while::<_, _, PError>(|c: char| c.is_ascii_digit())(rest)?;
                if digits.is_empty() || after.starts_with(is_ident_char) {
                    return failure(rest, format!("expected a non-negative integer after LIMIT, found {}", found(rest)));
                }
                let limit = digits.parse::<u64>().map_err(|_| {
                    NomErr::Failure(PError {
                        input: rest,
                        message: Cow::Borrowed("LIMIT is too large"),
                    })
                })?;
                (after, Some(limit))
            }
            Err(_) => (i, None),
        };
        let i = match symbol(";")(i) {
            Ok((rest, _)) => rest,
            Err(_) => i,
        };
        let i = skip_ws(i);
        if !i.is_empty() {
            return failure(i, format!("unexpected {}", found(i)));
        }
        Ok((
            i,
            Select {
                distinct,
                targets,
                from,
                where_clause,
                group_by,
                order_by,
                limit,
            },
        ))
    }

    fn targets(&self, i: &'s str) -> PResult<'s, Targets> {
        if let Ok((rest, _)) = symbol("*")(i) {
            return Ok((rest, Targets::Wildcard));
        }
        let mut targets = vec![];
        let mut i = i;
        loop {
            let (rest, expr) = self.expr(i)?;
            let (rest, alias) = match keyword("as")(rest) {
                Ok((rest, _)) => {
                    let (rest, alias) = cut(raw_identifier(rest))?;
                    (rest, Some(alias.to_owned()))
                }
                Err(_) => (rest, None),
            };
            targets.push(Target { expr, alias });
            match symbol(",")(rest) {
                Ok((rest, _)) => i = rest,
                Err(_) => return Ok((rest, Targets::List(targets))),
            }
        }
    }

    fn expr_list(&self, i: &'s str) -> PResult<'s, Vec<Expr>> {
        let mut items = vec![];
        let mut i = i;
        loop {
            let (rest, expr) = self.expr(i)?;
            items.push(expr);
            match symbol(",")(rest) {
                Ok((rest, _)) => i = rest,
                Err(_) => return Ok((rest, items)),
            }
        }
    }

    fn order_items(&self, i: &'s str) -> PResult<'s, Vec<OrderItem>> {
        let mut items = vec![];
        let mut i = i;
        loop {
            let (rest, expr) = self.expr(i)?;
            let (rest, descending) = if let Ok((rest, _)) = keyword("desc")(rest) {
                (rest, true)
            } else if let Ok((rest, _)) = keyword("asc")(rest) {
                (rest, false)
            } else {
                (rest, false)
            };
            items.push(OrderItem { expr, descending });
            match symbol(",")(rest) {
                Ok((rest, _)) => i = rest,
                Err(_) => return Ok((rest, items)),
            }
        }
    }

    pub(crate) fn expr(&self, i: &'s str) -> PResult<'s, Expr> {
        self.enter(i)?;
        let result = self.or_expr(i);
        self.leave();
        result
    }

    /// Go one recursion level deeper, failing past [`MAX_DEPTH`]; pair with [`Parser::leave`].
    fn enter(&self, i: &'s str) -> Result<(), NomErr<PError<'s>>> {
        let depth = self.depth.get() + 1;
        if depth > MAX_DEPTH {
            return Err(too_deep(i));
        }
        self.depth.set(depth);
        Ok(())
    }

    fn leave(&self) {
        self.depth.set(self.depth.get() - 1);
    }

    /// Build a node, failing when the tree would grow higher than [`MAX_DEPTH`].
    fn node(&self, kind: ExprKind, span: Span) -> Result<Expr, NomErr<PError<'s>>> {
        let expr = Expr::new(kind, span);
        if expr.height > MAX_DEPTH {
            return Err(too_deep(&self.src[span.start.min(self.src.len())..]));
        }
        Ok(expr)
    }

    fn binary(&self, op: BinaryOp, left: Expr, right: Expr) -> Result<Expr, NomErr<PError<'s>>> {
        let span = Span::new(left.span.start, right.span.end);
        self.node(ExprKind::Binary(op, Box::new(left), Box::new(right)), span)
    }

    /// Combine the operands of an associative operator into a balanced tree, keeping their
    /// left-to-right order (so evaluation order and short-circuiting are unchanged).
    fn balanced(&self, op: BinaryOp, mut operands: Vec<Expr>) -> Result<Expr, NomErr<PError<'s>>> {
        while operands.len() > 1 {
            let mut next = Vec::with_capacity(operands.len() / 2 + 1);
            let mut iter = operands.into_iter();
            while let Some(left) = iter.next() {
                match iter.next() {
                    Some(right) => next.push(self.binary(op, left, right)?),
                    None => next.push(left),
                }
            }
            operands = next;
        }
        Ok(operands.pop().expect("a chain has at least one operand"))
    }

    /// A chain of binary operators of the same precedence: balanced when every operator is
    /// the associative `balance_op`, left-deep otherwise.
    fn chain(&self, first: Expr, rest: Vec<(BinaryOp, Expr)>, balance_op: BinaryOp) -> Result<Expr, NomErr<PError<'s>>> {
        if rest.iter().all(|(op, _)| *op == balance_op) {
            let operands = std::iter::once(first).chain(rest.into_iter().map(|(_, expr)| expr)).collect();
            return self.balanced(balance_op, operands);
        }
        let mut left = first;
        for (op, right) in rest {
            left = self.binary(op, left, right)?;
        }
        Ok(left)
    }

    fn or_expr(&self, i: &'s str) -> PResult<'s, Expr> {
        let (mut i, first) = self.and_expr(i)?;
        let mut rest = vec![];
        while let Ok((after, _)) = keyword("or")(i) {
            let (after, right) = cut(self.and_expr(after))?;
            rest.push((BinaryOp::Or, right));
            i = after;
        }
        Ok((i, self.chain(first, rest, BinaryOp::Or)?))
    }

    fn and_expr(&self, i: &'s str) -> PResult<'s, Expr> {
        let (mut i, first) = self.not_expr(i)?;
        let mut rest = vec![];
        while let Ok((after, _)) = keyword("and")(i) {
            let (after, right) = cut(self.not_expr(after))?;
            rest.push((BinaryOp::And, right));
            i = after;
        }
        Ok((i, self.chain(first, rest, BinaryOp::And)?))
    }

    fn not_expr(&self, i: &'s str) -> PResult<'s, Expr> {
        let start = self.offset(skip_ws(i));
        if let Ok((rest, _)) = keyword("not")(i) {
            self.enter(rest)?;
            let inner = cut(self.not_expr(rest));
            self.leave();
            let (rest, inner) = inner?;
            let span = Span::new(start, inner.span.end);
            return Ok((rest, self.node(ExprKind::Unary(UnaryOp::Not, Box::new(inner)), span)?));
        }
        self.comparison(i)
    }

    fn comparison(&self, i: &'s str) -> PResult<'s, Expr> {
        let (i, left) = self.sum(i)?;
        self.comparison_operator(i, left)
    }

    /// What may follow the left operand of a comparison (kept out of [`Parser::comparison`]
    /// so the frame on the recursion path stays small).
    fn comparison_operator(&self, i: &'s str, left: Expr) -> PResult<'s, Expr> {
        // IS [NOT] NULL
        if let Ok((rest, _)) = keyword("is")(i) {
            let (rest, negated) = match keyword("not")(rest) {
                Ok((rest, _)) => (rest, true),
                Err(_) => (rest, false),
            };
            let (rest, _) = cut(keyword("null")(rest)).map_err(|_| {
                NomErr::Failure(PError {
                    input: skip_ws(rest),
                    message: Cow::Owned(format!("expected NULL after IS, found {}", found(rest))),
                })
            })?;
            let span = Span::new(left.span.start, self.offset(rest));
            return Ok((rest, self.node(ExprKind::IsNull { expr: Box::new(left), negated }, span)?));
        }

        // [NOT] IN
        let (in_rest, negated) = match keyword("not")(i) {
            Ok((rest, _)) => match keyword("in")(rest) {
                Ok((rest, _)) => (Some(rest), true),
                Err(_) => return failure(skip_ws(rest), format!("expected IN after NOT, found {}", found(rest))),
            },
            Err(_) => match keyword("in")(i) {
                Ok((rest, _)) => (Some(rest), false),
                Err(_) => (None, false),
            },
        };
        if let Some(rest) = in_rest {
            let (rest, haystack) = cut(self.in_target(rest))?;
            let span = Span::new(left.span.start, self.offset(rest));
            let kind = ExprKind::In {
                needle: Box::new(left),
                haystack,
                negated,
            };
            return Ok((rest, self.node(kind, span)?));
        }

        // binary comparison operators; longer symbols first
        const OPERATORS: &[(&str, BinaryOp)] = &[
            ("<=", BinaryOp::Le),
            (">=", BinaryOp::Ge),
            ("!=", BinaryOp::Ne),
            ("<>", BinaryOp::Ne),
            ("==", BinaryOp::Eq),
            ("!~", BinaryOp::NotMatch),
            ("?~", BinaryOp::MatchCase),
            ("=", BinaryOp::Eq),
            ("<", BinaryOp::Lt),
            (">", BinaryOp::Gt),
            ("~", BinaryOp::Match),
        ];
        let trimmed = skip_ws(i);
        for (sym, op) in OPERATORS {
            if let Some(rest) = trimmed.strip_prefix(sym) {
                let (rest, right) = cut(self.sum(rest))?;
                return Ok((rest, self.binary(*op, left, right)?));
            }
        }
        Ok((i, left))
    }

    fn in_target(&self, i: &'s str) -> PResult<'s, InTarget> {
        if let Ok((rest, _)) = symbol("(")(i) {
            let (rest, items) = cut(self.expr_list(rest))?;
            let (rest, _) = cut(symbol(")")(rest))?;
            return Ok((rest, InTarget::List(items)));
        }
        let (rest, expr) = self.sum(i)?;
        Ok((rest, InTarget::Expr(Box::new(expr))))
    }

    fn sum(&self, i: &'s str) -> PResult<'s, Expr> {
        let (mut i, first) = self.term(i)?;
        let mut rest = vec![];
        loop {
            let trimmed = skip_ws(i);
            let op = if trimmed.starts_with('+') {
                BinaryOp::Add
            } else if trimmed.starts_with('-') && !trimmed.starts_with("--") {
                BinaryOp::Sub
            } else {
                return Ok((i, self.chain(first, rest, BinaryOp::Add)?));
            };
            let (after, right) = cut(self.term(&trimmed[1..]))?;
            rest.push((op, right));
            i = after;
        }
    }

    fn term(&self, i: &'s str) -> PResult<'s, Expr> {
        let (mut i, first) = self.unary(i)?;
        let mut rest = vec![];
        loop {
            let trimmed = skip_ws(i);
            let op = if trimmed.starts_with('*') {
                BinaryOp::Mul
            } else if trimmed.starts_with('/') {
                BinaryOp::Div
            } else {
                return Ok((i, self.chain(first, rest, BinaryOp::Mul)?));
            };
            let (after, right) = cut(self.unary(&trimmed[1..]))?;
            rest.push((op, right));
            i = after;
        }
    }

    fn unary(&self, i: &'s str) -> PResult<'s, Expr> {
        let trimmed = skip_ws(i);
        let start = self.offset(trimmed);
        if trimmed.starts_with('-') && !trimmed.starts_with("--") {
            let operand = &trimmed[1..];
            self.enter(operand)?;
            let inner = cut(self.unary(operand));
            self.leave();
            let (rest, inner) = inner?;
            let span = Span::new(start, inner.span.end);
            let kind = match inner.kind {
                ExprKind::Literal(Literal::Int(value)) if value != i64::MIN => ExprKind::Literal(Literal::Int(-value)),
                ExprKind::Literal(Literal::Decimal(value)) => ExprKind::Literal(Literal::Decimal(-value)),
                kind => ExprKind::Unary(
                    UnaryOp::Neg,
                    Box::new(Expr {
                        kind,
                        span: inner.span,
                        height: inner.height,
                    }),
                ),
            };
            return Ok((rest, self.node(kind, span)?));
        }
        if let Some(rest) = trimmed.strip_prefix('+') {
            self.enter(rest)?;
            let result = cut(self.unary(rest));
            self.leave();
            return result;
        }
        self.primary(trimmed)
    }

    /// A primary expression. Parentheses recurse through here, so this only dispatches and
    /// keeps its stack frame small; each form is parsed by its own function.
    fn primary(&self, i: &'s str) -> PResult<'s, Expr> {
        let i = skip_ws(i);
        let start = self.offset(i);
        match i.chars().next() {
            None => error(i, "expected an expression, found end of query"),
            Some('(') => self.parenthesized(i, start),
            Some(quote @ ('\'' | '"')) => self.string_literal(i, quote, start),
            Some('$' | ':') => self.parameter(i, start),
            Some(c) if c.is_ascii_digit() || (c == '.' && i[1..].starts_with(|c: char| c.is_ascii_digit())) => self.number_or_date(i, start),
            Some(c) if is_ident_start(c) => self.identifier(i, start),
            Some(_) => error(i, format!("expected an expression, found {}", found(i))),
        }
    }

    fn leaf(&self, rest: &'s str, kind: ExprKind, start: usize) -> PResult<'s, Expr> {
        Ok((rest, self.node(kind, Span::new(start, self.offset(rest)))?))
    }

    fn parenthesized(&self, i: &'s str, start: usize) -> PResult<'s, Expr> {
        let (rest, inner) = cut(self.expr(&i[1..]))?;
        let (rest, _) = cut(symbol(")")(rest))?;
        Ok((
            rest,
            Expr {
                span: Span::new(start, self.offset(rest)),
                ..inner
            },
        ))
    }

    fn string_literal(&self, i: &'s str, quote: char, start: usize) -> PResult<'s, Expr> {
        let body = &i[1..];
        match body.find(quote) {
            Some(end) => self.leaf(&body[end + 1..], ExprKind::Literal(Literal::Str(body[..end].to_owned())), start),
            None => failure(i, "unterminated string literal"),
        }
    }

    fn parameter(&self, i: &'s str, start: usize) -> PResult<'s, Expr> {
        let body = &i[1..];
        if i.starts_with('$') {
            let (rest, digits) = take_while::<_, _, PError>(|c: char| c.is_ascii_digit())(body)?;
            return match digits.parse::<usize>() {
                Ok(idx) if idx >= 1 => self.leaf(rest, ExprKind::Param(ParamRef::Positional(idx)), start),
                _ => failure(i, "expected a parameter number after '$', e.g. $1"),
            };
        }
        if !body.starts_with(is_ident_start) {
            return failure(i, "expected a parameter name after ':', e.g. :from");
        }
        let (rest, name) = take_while1::<_, _, PError>(is_ident_char)(body)?;
        self.leaf(rest, ExprKind::Param(ParamRef::Named(name.to_owned())), start)
    }

    /// A literal keyword, a column or a function call.
    fn identifier(&self, i: &'s str, start: usize) -> PResult<'s, Expr> {
        let (rest, name) = raw_identifier(i)?;
        let lower = name.to_ascii_lowercase();
        match lower.as_str() {
            "true" => return self.leaf(rest, ExprKind::Literal(Literal::Bool(true)), start),
            "false" => return self.leaf(rest, ExprKind::Literal(Literal::Bool(false)), start),
            "null" => return self.leaf(rest, ExprKind::Literal(Literal::Null), start),
            _ => {}
        }
        if let Some(call_rest) = skip_ws(rest).strip_prefix('(') {
            return self.call(call_rest, lower, start);
        }
        if RESERVED.contains(&lower.as_str()) {
            return error(i, format!("expected an expression, found keyword {}", name.to_uppercase()));
        }
        self.leaf(rest, ExprKind::Column(lower), start)
    }

    /// The arguments of a call, after the opening parenthesis.
    fn call(&self, i: &'s str, name: String, start: usize) -> PResult<'s, Expr> {
        if let Ok((after_star, _)) = symbol("*")(i) {
            if let Ok((after, _)) = symbol(")")(after_star) {
                return self.leaf(
                    after,
                    ExprKind::Call {
                        name,
                        args: vec![],
                        star: true,
                    },
                    start,
                );
            }
        }
        let (after, args) = if let Ok((after, _)) = symbol(")")(i) {
            (after, vec![])
        } else {
            let (after, args) = cut(self.expr_list(i))?;
            let (after, _) = cut(symbol(")")(after))?;
            (after, args)
        };
        self.leaf(after, ExprKind::Call { name, args, star: false }, start)
    }

    fn number_or_date(&self, i: &'s str, start: usize) -> PResult<'s, Expr> {
        let (rest, int_part) = take_while::<_, _, PError>(|c: char| c.is_ascii_digit())(i)?;
        // bare date literal: YYYY-MM-DD
        if let (4, Some(candidate)) = (int_part.len(), i.get(..10)) {
            let tail = &i[candidate.len()..];
            let shape_ok = candidate.is_ascii()
                && candidate.as_bytes()[4] == b'-'
                && candidate.as_bytes()[7] == b'-'
                && candidate[5..7].chars().all(|c| c.is_ascii_digit())
                && candidate[8..10].chars().all(|c| c.is_ascii_digit());
            if shape_ok && !tail.starts_with(is_ident_char) {
                return match NaiveDate::parse_from_str(candidate, "%Y-%m-%d") {
                    Ok(date) => Ok((tail, Expr::new(ExprKind::Literal(Literal::Date(date)), Span::new(start, start + 10)))),
                    Err(_) => failure(i, format!("invalid date literal {}", candidate)),
                };
            }
        }
        let (rest, fraction) = match rest.strip_prefix('.') {
            Some(after_dot) => {
                let (after, digits) = take_while::<_, _, PError>(|c: char| c.is_ascii_digit())(after_dot)?;
                (after, Some(digits))
            }
            None => (rest, None),
        };
        if rest.starts_with(is_ident_char) {
            return failure(i, format!("invalid number literal near {}", found(i)));
        }
        let text = &i[..i.len() - rest.len()];
        let span = Span::new(start, start + text.len());
        let literal = match fraction {
            None => match int_part.parse::<i64>() {
                Ok(value) => Literal::Int(value),
                Err(_) => Literal::Decimal(BigDecimal::from_str(text).map_err(|_| {
                    NomErr::Failure(PError {
                        input: i,
                        message: Cow::Borrowed("invalid number literal"),
                    })
                })?),
            },
            Some(_) => {
                let normalized = if text.starts_with('.') { format!("0{}", text) } else { text.to_owned() };
                Literal::Decimal(BigDecimal::from_str(normalized.trim_end_matches('.')).map_err(|_| {
                    NomErr::Failure(PError {
                        input: i,
                        message: Cow::Borrowed("invalid number literal"),
                    })
                })?)
            }
        };
        Ok((rest, Expr::new(ExprKind::Literal(literal), span)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_ok(src: &str) -> Select {
        parse(src).unwrap_or_else(|e| panic!("{}: {}", src, e))
    }

    fn where_kind(src: &str) -> ExprKind {
        parse_ok(src).where_clause.unwrap().kind
    }

    #[test]
    fn parses_full_select() {
        let select = parse_ok(
            "select distinct year, root(account, 2) as category, sum(position) from year = 2024 where account ~ '^Expenses' \
             group by 1, category order by 1 desc, category asc limit 10;",
        );
        assert!(select.distinct);
        let Targets::List(targets) = &select.targets else { panic!() };
        assert_eq!(targets.len(), 3);
        assert_eq!(targets[1].alias.as_deref(), Some("category"));
        assert!(select.from.is_some());
        assert_eq!(select.group_by.as_ref().unwrap().len(), 2);
        let order = select.order_by.unwrap();
        assert!(order[0].descending);
        assert!(!order[1].descending);
        assert_eq!(select.limit, Some(10));
    }

    #[test]
    fn keywords_are_case_insensitive_and_wildcard() {
        let select = parse_ok("SeLeCt * WhErE TRUE");
        assert_eq!(select.targets, Targets::Wildcard);
        assert_eq!(select.where_clause.unwrap().kind, ExprKind::Literal(Literal::Bool(true)));
    }

    #[test]
    fn literals() {
        let select = parse_ok("SELECT 'a', \"b\", 2024-01-31, 12, 12.50, .5, -3, TRUE, false, NULL, $1, :from");
        let Targets::List(targets) = select.targets else { panic!() };
        let kinds = targets.into_iter().map(|t| t.expr.kind).collect::<Vec<_>>();
        assert_eq!(kinds[0], ExprKind::Literal(Literal::Str("a".into())));
        assert_eq!(kinds[1], ExprKind::Literal(Literal::Str("b".into())));
        assert_eq!(kinds[2], ExprKind::Literal(Literal::Date(NaiveDate::from_ymd_opt(2024, 1, 31).unwrap())));
        assert_eq!(kinds[3], ExprKind::Literal(Literal::Int(12)));
        assert_eq!(kinds[4], ExprKind::Literal(Literal::Decimal(BigDecimal::from_str("12.50").unwrap())));
        assert_eq!(kinds[5], ExprKind::Literal(Literal::Decimal(BigDecimal::from_str("0.5").unwrap())));
        assert_eq!(kinds[6], ExprKind::Literal(Literal::Int(-3)));
        assert_eq!(kinds[7], ExprKind::Literal(Literal::Bool(true)));
        assert_eq!(kinds[8], ExprKind::Literal(Literal::Bool(false)));
        assert_eq!(kinds[9], ExprKind::Literal(Literal::Null));
        assert_eq!(kinds[10], ExprKind::Param(ParamRef::Positional(1)));
        assert_eq!(kinds[11], ExprKind::Param(ParamRef::Named("from".into())));
    }

    #[test]
    fn precedence() {
        // a OR b AND NOT c  ==  a OR (b AND (NOT c))
        let ExprKind::Binary(BinaryOp::Or, _, right) = where_kind("SELECT * WHERE a OR b AND NOT c") else {
            panic!()
        };
        let ExprKind::Binary(BinaryOp::And, _, not) = right.kind else { panic!() };
        assert!(matches!(not.kind, ExprKind::Unary(UnaryOp::Not, _)));

        // 1 + 2 * 3 = 7
        let ExprKind::Binary(BinaryOp::Eq, left, _) = where_kind("SELECT * WHERE 1 + 2 * 3 = 7") else {
            panic!()
        };
        let ExprKind::Binary(BinaryOp::Add, _, mul) = left.kind else { panic!() };
        assert!(matches!(mul.kind, ExprKind::Binary(BinaryOp::Mul, _, _)));
    }

    #[test]
    fn operators() {
        for (src, op) in [
            ("a ~ 'x'", BinaryOp::Match),
            ("a !~ 'x'", BinaryOp::NotMatch),
            ("a ?~ 'x'", BinaryOp::MatchCase),
            ("a != 1", BinaryOp::Ne),
            ("a <> 1", BinaryOp::Ne),
            ("a <= 1", BinaryOp::Le),
            ("a >= 1", BinaryOp::Ge),
            ("a < 1", BinaryOp::Lt),
            ("a > 1", BinaryOp::Gt),
            ("a == 1", BinaryOp::Eq),
        ] {
            let ExprKind::Binary(parsed, _, _) = where_kind(&format!("SELECT * WHERE {}", src)) else {
                panic!("{}", src)
            };
            assert_eq!(parsed, op, "{}", src);
        }
        assert!(matches!(
            where_kind("SELECT * WHERE 'x' IN tags"),
            ExprKind::In {
                haystack: InTarget::Expr(_),
                negated: false,
                ..
            }
        ));
        assert!(matches!(
            where_kind("SELECT * WHERE payee NOT IN ('a', 'b')"),
            ExprKind::In {
                haystack: InTarget::List(_),
                negated: true,
                ..
            }
        ));
        assert!(matches!(where_kind("SELECT * WHERE payee IS NOT NULL"), ExprKind::IsNull { negated: true, .. }));
        assert!(matches!(where_kind("SELECT * WHERE payee is null"), ExprKind::IsNull { negated: false, .. }));
    }

    #[test]
    fn count_star_and_calls() {
        let select = parse_ok("SELECT count(*), today(), COUNT( * )");
        let Targets::List(targets) = select.targets else { panic!() };
        assert!(matches!(&targets[0].expr.kind, ExprKind::Call { name, star: true, .. } if name == "count"));
        assert!(matches!(&targets[1].expr.kind, ExprKind::Call { name, args, star: false } if name == "today" && args.is_empty()));
        assert!(matches!(&targets[2].expr.kind, ExprKind::Call { star: true, .. }));
    }

    #[test]
    fn comments_and_spans() {
        let src = "SELECT -- the account\n  account, sum(position)";
        let select = parse_ok(src);
        let Targets::List(targets) = select.targets else { panic!() };
        let span = targets[1].expr.span;
        assert_eq!(&src[span.start..span.end], "sum(position)");
    }

    fn parse_err(src: &str) -> QueryError {
        parse(src).expect_err(src)
    }

    #[test]
    fn error_positions_are_one_based_characters() {
        let err = parse_err("SELECT * WHERE payee = '午餐' AND x ~");
        assert_eq!(err.kind, QueryErrorKind::Parse);
        assert_eq!((err.line, err.column), (Some(1), Some(36)), "{}", err);
        assert!(err.message.contains("end of query"), "{}", err.message);

        let err = parse_err("SELECT date,\n  payee\n  FROM WHERE");
        assert_eq!((err.line, err.column), (Some(3), Some(8)), "{}", err);

        let err = parse_err("SELECT 'unterminated");
        assert_eq!((err.line, err.column), (Some(1), Some(8)));
        assert_eq!(err.message, "unterminated string literal");

        let err = parse_err("SELECT account account");
        assert_eq!(err.column, Some(16));
        assert!(err.message.contains("unexpected 'account'"));

        let err = parse_err("BALANCES");
        assert!(err.message.contains("not supported"));
        let err = parse_err("SELECT * FROM OPEN ON 2024-01-01");
        assert!(err.message.contains("OPEN/CLOSE/CLEAR"));
        let err = parse_err("SELECT * WHERE date > 2024-13-01");
        assert!(err.message.contains("invalid date"));
        let err = parse_err("SELECT * LIMIT -1");
        assert!(err.message.contains("LIMIT"));
        let err = parse_err("SELECT sum(position");
        assert!(err.message.contains("')'"), "{}", err.message);
    }
}
