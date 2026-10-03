//! The query parser, written with `nom`.
//!
//! Grammar (keywords are case-insensitive):
//!
//! ```text
//! query      := (select | balances | journal) [;]
//! select     := SELECT [DISTINCT] targets [from] [where]
//!               [GROUP BY item, ... [HAVING expr]] [ORDER BY item [ASC|DESC], ...]
//!               [PIVOT BY column, column] [LIMIT count [OFFSET count]]
//! count      := int | $n | :name                 (a parameter is bound to an int)
//! column     := name | int                       (a target name or a 1-based target index)
//! balances   := BALANCES [AT name] [from] [where]
//! journal    := JOURNAL ['regex' | $n | :name] [AT name] [from]
//! from       := FROM table | FROM [expr] [OPEN ON date] [CLOSE [ON date]] [CLEAR]   (at least one part)
//! table      := #name | name        (a bare name only when it names a table; SELECT only)
//! date       := 2024-01-31 | $n | :name
//! where      := WHERE expr
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
//! primary    := '(' expr ')' | literal | $n | :name | name '(' ['*' | expr, ...] ')' | name ('.' name)*
//! literal    := 'string' | "string" | 2024-01-31 | 12 | 12.50 | TRUE | FALSE | NULL
//! ```
//!
//! `--` starts a comment that runs to the end of the line.
//!
//! `BALANCES` and `JOURNAL` are shorthands that [`crate::statements`] desugars into a
//! [`Select`], so every later stage only ever sees SELECT.

use std::borrow::Cow;
use std::cell::Cell;
use std::str::FromStr;

use bigdecimal::BigDecimal;
use chrono::NaiveDate;
use nom::bytes::complete::{tag, tag_no_case, take_while, take_while1};
use nom::error::{ErrorKind, ParseError};
use nom::{Err as NomErr, IResult};

use crate::ast::{
    ArithOp, BinaryOp, Count, CountValue, Expr, ExprKind, FromClause, InTarget, Literal, LogicalOp, OrderItem, Period, Select, TableName, Target, Targets,
    UnaryOp,
};
use crate::error::{QueryError, QueryErrorKind, Span};
use crate::params::ParamRef;
use crate::statements::{self, AtFunction};

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
    "having", "pivot", "offset",
];

/// The clauses that may follow `FROM #table`.
const FOLLOWS_TABLE: &[&str] = &["where", "group", "order", "having", "pivot", "limit"];

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

/// The most parts a dotted name may have (`open.date` has two). Attribute access only needs
/// two today; the cap keeps names short whatever the query length.
pub const MAX_NAME_PARTS: usize = 8;

/// The longest accepted query text, in bytes.
pub const MAX_QUERY_LENGTH: usize = 64 * 1024;

/// The deepest accepted nesting, a defence in depth against stack exhaustion.
///
/// It bounds both how deeply the parser recurses (parentheses, function arguments, `IN`
/// lists, `NOT`, unary minus) and the height of the syntax tree it builds. Recursive descent
/// recurses once per nested parenthesis *before* any later stage could rewrite the tree, so
/// 10,000 `(` would exhaust the stack in the parser itself; this guard stops it first.
/// Parentheses only group and never create nodes, and chains of `AND`, `OR` and arithmetic
/// operators are single n-ary nodes, so long lists such as
/// `account = 'a' OR account = 'b' OR ...` stay flat whatever their length.
///
/// The value keeps parsing, compiling, optimising and evaluating the deepest accepted query
/// well within a 2 MiB thread stack even in debug builds, where each parser level costs
/// roughly 13 KiB of stack (measured: about 150 levels fit).
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
    match parser.statement(src) {
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

    /// A whole query: one statement, an optional `;`, then the end of the input.
    fn statement(&self, i: &'s str) -> PResult<'s, Select> {
        let i = skip_ws(i);
        let start = self.offset(i);
        let keyword_span = |rest: &str| Span::new(start, self.offset(rest));
        let (i, select) = if let Ok((rest, _)) = keyword("select")(i) {
            self.select(rest)?
        } else if let Ok((rest, _)) = keyword("balances")(i) {
            self.balances(rest, keyword_span(rest))?
        } else if let Ok((rest, _)) = keyword("journal")(i) {
            self.journal(rest, keyword_span(rest))?
        } else if is_keyword(i, "print") {
            return failure(i, "PRINT statements are not supported yet; only SELECT, BALANCES and JOURNAL are");
        } else {
            return failure(i, format!("expected SELECT, BALANCES or JOURNAL, found {}", found(i)));
        };
        let i = match symbol(";")(i) {
            Ok((rest, _)) => rest,
            Err(_) => i,
        };
        let i = skip_ws(i);
        if !i.is_empty() {
            return failure(i, format!("unexpected {}", found(i)));
        }
        Ok((i, select))
    }

    /// A SELECT, after the keyword.
    fn select(&self, i: &'s str) -> PResult<'s, Select> {
        let (i, distinct) = match keyword("distinct")(i) {
            Ok((rest, _)) => (rest, true),
            Err(_) => (i, false),
        };
        let (i, targets) = cut(self.targets(i))?;

        let (i, FromClause { table, expr: from, period }) = self.parse_from(i)?;
        let (i, where_clause) = self.parse_where(i)?;
        let (i, group_by) = match keyword("group")(i) {
            Ok((rest, _)) => {
                let (rest, _) = cut(keyword("by")(rest))?;
                let (rest, items) = cut(self.expr_list(rest))?;
                (rest, Some(items))
            }
            Err(_) => (i, None),
        };
        // as in beanquery, HAVING is part of the GROUP BY clause
        let (i, having) = match keyword("having")(i) {
            Ok(_) if group_by.is_none() => {
                return failure(skip_ws(i), "HAVING requires a GROUP BY clause; filter rows with WHERE");
            }
            Ok((rest, _)) => {
                let (rest, expr) = cut(self.expr(rest))?;
                if is_keyword(rest, "having") {
                    return failure(skip_ws(rest), "HAVING may appear only once; combine the conditions with AND");
                }
                (rest, Some(expr))
            }
            Err(_) => (i, None),
        };
        let (i, order_by) = match keyword("order")(i) {
            Ok((rest, _)) => {
                let (rest, _) = cut(keyword("by")(rest))?;
                let (rest, items) = cut(self.order_items(rest))?;
                (rest, Some(items))
            }
            Err(_) => (i, None),
        };
        if is_keyword(i, "having") {
            return failure(skip_ws(i), "HAVING must follow GROUP BY, before ORDER BY");
        }
        let (i, pivot_by) = match keyword("pivot")(i) {
            Ok((rest, _)) => {
                let (rest, _) = keyword("by")(rest).or_else(|_| failure(skip_ws(rest), format!("expected BY after PIVOT, found {}", found(rest))))?;
                let (rest, columns) = self.pivot_columns(rest)?;
                (rest, Some(columns))
            }
            Err(_) => (i, None),
        };
        let (i, limit) = match keyword("limit")(i) {
            Ok((rest, _)) => {
                let (rest, limit) = self.count(rest, "LIMIT")?;
                (rest, Some(limit))
            }
            Err(_) => (i, None),
        };
        let (i, offset) = match keyword("offset")(i) {
            Ok(_) if limit.is_none() => {
                return failure(skip_ws(i), "OFFSET must follow LIMIT, e.g. LIMIT 10 OFFSET 20");
            }
            Ok((rest, _)) => {
                let (rest, offset) = self.count(rest, "OFFSET")?;
                (rest, Some(offset))
            }
            Err(_) => (i, None),
        };
        if is_keyword(i, "pivot") {
            return failure(skip_ws(i), "PIVOT BY must come before LIMIT");
        }
        Ok((
            i,
            Select {
                distinct,
                targets,
                table,
                from,
                period,
                where_clause,
                group_by,
                having,
                order_by,
                pivot_by,
                limit,
                offset,
            },
        ))
    }

    /// The value of `LIMIT` or `OFFSET`, after the keyword: a non-negative integer or a
    /// parameter (which the compiler requires to be an integer).
    fn count(&self, i: &'s str, clause: &'static str) -> PResult<'s, Count> {
        let i = skip_ws(i);
        let start = self.offset(i);
        if i.starts_with(['$', ':']) {
            let (rest, param) = self.parameter(i, start)?;
            let ExprKind::Param(param) = param.kind else {
                unreachable!("parameter() parses a parameter")
            };
            let span = Span::new(start, self.offset(rest));
            return Ok((
                rest,
                Count {
                    value: CountValue::Param(param),
                    span,
                },
            ));
        }
        let (after, digits) = take_while::<_, _, PError>(|c: char| c.is_ascii_digit())(i)?;
        if digits.is_empty() || after.starts_with(is_ident_char) || after.starts_with('.') {
            return failure(
                i,
                format!("expected a non-negative integer or a parameter after {}, found {}", clause, found(i)),
            );
        }
        let value = digits.parse::<u64>().map_err(|_| {
            NomErr::Failure(PError {
                input: i,
                message: Cow::Owned(format!("{} is too large", clause)),
            })
        })?;
        Ok((
            after,
            Count {
                value: CountValue::Literal(value),
                span: Span::new(start, self.offset(after)),
            },
        ))
    }

    /// The two columns of `PIVOT BY`, after the keywords: each a target name or a 1-based
    /// target index, as in beanquery (expressions are not accepted).
    fn pivot_columns(&self, i: &'s str) -> PResult<'s, [Expr; 2]> {
        let (i, first) = self.pivot_column(i)?;
        let i = match symbol(",")(i) {
            Ok((rest, _)) => rest,
            Err(_) => return failure(skip_ws(i), format!("PIVOT BY takes two columns separated by a comma, found {}", found(i))),
        };
        let (i, second) = self.pivot_column(i)?;
        if symbol(",")(i).is_ok() {
            return failure(skip_ws(i), "PIVOT BY takes exactly two columns");
        }
        Ok((i, [first, second]))
    }

    fn pivot_column(&self, i: &'s str) -> PResult<'s, Expr> {
        let i = skip_ws(i);
        let start = self.offset(i);
        let expected = || failure(i, format!("expected a target name or number after PIVOT BY, found {}", found(i)));
        if i.starts_with(|c: char| c.is_ascii_digit()) {
            let (rest, digits) = take_while::<_, _, PError>(|c: char| c.is_ascii_digit())(i)?;
            if rest.starts_with(is_ident_char) || rest.starts_with('.') {
                return expected();
            }
            let index = digits.parse::<i64>().map_err(|_| {
                NomErr::Failure(PError {
                    input: i,
                    message: Cow::Borrowed("the PIVOT BY index is too large"),
                })
            })?;
            return Ok((rest, Expr::new(ExprKind::Literal(Literal::Int(index)), Span::new(start, self.offset(rest)))));
        }
        let Ok((rest, name)) = raw_identifier(i) else {
            return expected();
        };
        let name = name.to_ascii_lowercase();
        if RESERVED.contains(&name.as_str()) || skip_ws(rest).starts_with('(') {
            return expected();
        }
        Ok((rest, Expr::new(ExprKind::Column(name), Span::new(start, self.offset(rest)))))
    }

    /// `[FROM #name | FROM [expr] [OPEN ON date] [CLOSE [ON date]] [CLEAR]]`, shared by
    /// SELECT, BALANCES and JOURNAL. A FROM clause has at least one part. The modifiers come in
    /// this order, each at most once, as in beanquery; `open`, `close` and `clear` are not
    /// columns, so they cannot start the expression.
    ///
    /// A table stands alone, as in beanquery: neither a filter expression nor the period
    /// modifiers may follow it (WHERE filters its rows). The compiler resolves its name.
    fn parse_from(&self, i: &'s str) -> PResult<'s, FromClause> {
        const MODIFIERS: [&str; 3] = ["open", "close", "clear"];
        let Ok((i, _)) = keyword("from")(i) else {
            return Ok((
                i,
                FromClause {
                    table: None,
                    expr: None,
                    period: None,
                },
            ));
        };
        if let Some((rest, table)) = self.table_name(i)? {
            let next = skip_ws(rest);
            if MODIFIERS.iter().any(|modifier| is_keyword(rest, modifier)) {
                return failure(
                    next,
                    format!(
                        "OPEN ON, CLOSE and CLEAR cannot be combined with a table (#{}); they summarize the postings, so leave the table out: FROM OPEN ON ...",
                        table.name
                    ),
                );
            }
            if !next.is_empty() && !next.starts_with(';') && !FOLLOWS_TABLE.iter().any(|word| is_keyword(rest, word)) {
                return failure(
                    next,
                    format!("unexpected {} after the table #{}; filter its rows with WHERE", found(next), table.name),
                );
            }
            return Ok((
                rest,
                FromClause {
                    table: Some(table),
                    expr: None,
                    period: None,
                },
            ));
        }
        let (i, expr) = if MODIFIERS.iter().any(|modifier| is_keyword(i, modifier)) {
            (i, None)
        } else {
            let (rest, expr) = cut(self.expr(i))?;
            (rest, Some(expr))
        };

        let (i, open) = match keyword("open")(i) {
            Ok((rest, _)) => {
                let (rest, _) = keyword("on")(rest).or_else(|_| failure(skip_ws(rest), format!("expected ON after OPEN, found {}", found(rest))))?;
                let (rest, date) = self.period_date(rest, "OPEN ON")?;
                (rest, Some(date))
            }
            Err(_) => (i, None),
        };
        let (i, close) = match keyword("close")(i) {
            Ok((rest, _)) => match keyword("on")(rest) {
                Ok((rest, _)) => {
                    let (rest, date) = self.period_date(rest, "CLOSE ON")?;
                    (rest, Some(Some(date)))
                }
                Err(_) => (rest, Some(None)),
            },
            Err(_) => (i, None),
        };
        let (i, clear) = match keyword("clear")(i) {
            Ok((rest, _)) => (rest, true),
            Err(_) => (i, false),
        };
        if MODIFIERS.iter().any(|modifier| is_keyword(i, modifier)) {
            return failure(
                skip_ws(i),
                format!("unexpected {}: OPEN ON, CLOSE [ON] and CLEAR may each appear once, in this order", found(i)),
            );
        }
        let period = (open.is_some() || close.is_some() || clear).then_some(Period { open, close, clear });
        Ok((i, FromClause { table: None, expr, period }))
    }

    /// The table of a FROM clause, if it names one: `#name`, or, as in beanquery, a bare name
    /// that is not a column of the postings table and stands alone (followed by what may
    /// follow a table). So `FROM prices` reads the prices table, while `FROM payee ~ 'x'` and
    /// `FROM year = 2016` keep parsing as filters.
    fn table_name(&self, i: &'s str) -> Result<Option<(&'s str, TableName)>, NomErr<PError<'s>>> {
        const MODIFIERS: [&str; 3] = ["open", "close", "clear"];
        let i = skip_ws(i);
        let start = self.offset(i);
        if let Some(name) = i.strip_prefix('#') {
            let Ok((rest, name)) = take_while1::<_, _, PError>(is_ident_char)(name) else {
                return Err(NomErr::Failure(PError {
                    input: name,
                    message: Cow::Owned(format!("expected a table name after '#', e.g. #prices, found {}", found(name))),
                }));
            };
            let span = Span::new(start, self.offset(rest));
            let name = name.to_owned();
            return Ok(Some((rest, TableName { name, span, bare: false })));
        }
        let Ok((rest, name)) = raw_identifier(i) else {
            return Ok(None);
        };
        let lower = name.to_ascii_lowercase();
        let next = skip_ws(rest);
        let alone = next.is_empty() || next.starts_with(';') || FOLLOWS_TABLE.iter().chain(&MODIFIERS).any(|word| is_keyword(rest, word));
        let keyword = RESERVED.contains(&lower.as_str()) || MODIFIERS.contains(&lower.as_str());
        if !alone || keyword || crate::table::POSTINGS.column(name).is_some() {
            return Ok(None);
        }
        let span = Span::new(start, self.offset(rest));
        let name = name.to_owned();
        Ok(Some((rest, TableName { name, span, bare: true })))
    }

    /// The date of `OPEN ON` / `CLOSE ON`: a date literal or a parameter.
    fn period_date(&self, i: &'s str, clause: &str) -> PResult<'s, Expr> {
        let i = skip_ws(i);
        let start = self.offset(i);
        let expected = || format!("expected a date after {} (e.g. 2024-01-01, or a parameter), found {}", clause, found(i));
        let (rest, date) = match i.chars().next() {
            Some('$' | ':') => self.parameter(i, start)?,
            Some(c) if c.is_ascii_digit() => self.number_or_date(i, start)?,
            _ => return failure(i, expected()),
        };
        match date.kind {
            ExprKind::Literal(Literal::Date(_)) | ExprKind::Param(_) => Ok((rest, date)),
            _ => failure(i, expected()),
        }
    }

    /// `[WHERE expr]`, shared by SELECT and BALANCES.
    fn parse_where(&self, i: &'s str) -> PResult<'s, Option<Expr>> {
        let Ok((rest, _)) = keyword("where")(i) else {
            return Ok((i, None));
        };
        let (rest, expr) = cut(self.expr(rest))?;
        Ok((rest, Some(expr)))
    }

    /// `BALANCES [AT name] [from] [WHERE expr]`, after the keyword.
    fn balances(&self, i: &'s str, keyword: Span) -> PResult<'s, Select> {
        let (i, at) = self.at_clause(i)?;
        let (i, FromClause { expr: from, period, .. }) = self.statement_from(i, "BALANCES")?;
        let (i, where_clause) = self.parse_where(i)?;
        // the FROM clause is passed through as SELECT parses it
        Ok((
            i,
            Select {
                from,
                period,
                ..statements::balances(keyword, at, where_clause)
            },
        ))
    }

    /// `JOURNAL ['regex' | $n | :name] [AT name] [from]`, after the keyword. As in
    /// beanquery there is no WHERE clause; FROM filters the postings.
    fn journal(&self, i: &'s str, keyword: Span) -> PResult<'s, Select> {
        let trimmed = skip_ws(i);
        let start = self.offset(trimmed);
        let (i, account) = match trimmed.chars().next() {
            Some(quote @ ('\'' | '"')) => {
                let (rest, pattern) = self.string_literal(trimmed, quote, start)?;
                (rest, Some(pattern))
            }
            Some('$' | ':') => {
                let (rest, pattern) = self.parameter(trimmed, start)?;
                (rest, Some(pattern))
            }
            _ => (i, None),
        };
        let (i, at) = self.at_clause(i)?;
        let (i, FromClause { expr: from, period, .. }) = self.statement_from(i, "JOURNAL")?;
        if is_keyword(i, "where") {
            return failure(skip_ws(i), "JOURNAL has no WHERE clause; filter the postings with FROM <expression>");
        }
        Ok((
            i,
            Select {
                from,
                period,
                ..statements::journal(keyword, account, at)
            },
        ))
    }

    /// The FROM clause of BALANCES and JOURNAL, which always read the postings: `#name` is
    /// rejected (beanquery does not accept a table there either).
    fn statement_from(&self, i: &'s str, statement: &str) -> PResult<'s, FromClause> {
        let rejected = |at: &'s str| failure(at, format!("{} always reads the postings; FROM cannot name a table", statement));
        if let Ok((rest, _)) = keyword("from")(i) {
            let table = skip_ws(rest);
            if table.starts_with('#') {
                return rejected(table);
            }
        }
        let (rest, from) = self.parse_from(i)?;
        match &from.table {
            Some(table) => rejected(&self.src[table.span.start..]),
            None => Ok((rest, from)),
        }
    }

    /// `[AT name]`: the function applied to the positions and balances of BALANCES and
    /// JOURNAL, e.g. `AT cost`.
    fn at_clause(&self, i: &'s str) -> PResult<'s, Option<AtFunction>> {
        let Ok((rest, _)) = keyword("at")(i) else {
            return Ok((i, None));
        };
        let rest = skip_ws(rest);
        let expected = || format!("expected a function name after AT, e.g. AT cost, found {}", found(rest));
        let Ok((after, name)) = raw_identifier(rest) else {
            return failure(rest, expected());
        };
        let name = name.to_ascii_lowercase();
        if RESERVED.contains(&name.as_str()) {
            return failure(rest, expected());
        }
        let start = self.offset(rest);
        Ok((
            after,
            Some(AtFunction {
                name,
                span: Span::new(start, self.offset(after)),
            }),
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

    /// `a AND b AND ...` / `a OR b OR ...` as one n-ary node.
    fn logical(&self, op: LogicalOp, operands: Vec<Expr>) -> Result<Expr, NomErr<PError<'s>>> {
        if operands.len() == 1 {
            return Ok(operands.into_iter().next().expect("one operand"));
        }
        let span = Span::new(operands[0].span.start, operands[operands.len() - 1].span.end);
        self.node(ExprKind::Logical(op, operands), span)
    }

    /// `first op x op y ...` as one left-to-right chain node.
    fn arith(&self, first: Expr, rest: Vec<(ArithOp, Expr)>) -> Result<Expr, NomErr<PError<'s>>> {
        let Some((_, last)) = rest.last() else {
            return Ok(first);
        };
        let span = Span::new(first.span.start, last.span.end);
        self.node(ExprKind::Arith(Box::new(first), rest), span)
    }

    fn or_expr(&self, i: &'s str) -> PResult<'s, Expr> {
        let (mut i, first) = self.and_expr(i)?;
        let mut operands = vec![first];
        while let Ok((after, _)) = keyword("or")(i) {
            let (after, right) = cut(self.and_expr(after))?;
            operands.push(right);
            i = after;
        }
        Ok((i, self.logical(LogicalOp::Or, operands)?))
    }

    fn and_expr(&self, i: &'s str) -> PResult<'s, Expr> {
        let (mut i, first) = self.not_expr(i)?;
        let mut operands = vec![first];
        while let Ok((after, _)) = keyword("and")(i) {
            let (after, right) = cut(self.not_expr(after))?;
            operands.push(right);
            i = after;
        }
        Ok((i, self.logical(LogicalOp::And, operands)?))
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
                ArithOp::Add
            } else if trimmed.starts_with('-') && !trimmed.starts_with("--") {
                ArithOp::Sub
            } else {
                return Ok((i, self.arith(first, rest)?));
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
                ArithOp::Mul
            } else if trimmed.starts_with('/') {
                ArithOp::Div
            } else {
                return Ok((i, self.arith(first, rest)?));
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
        // attribute access on a structured column: `open.date` is the column `open.date`
        let (mut rest, mut lower, mut parts) = (rest, lower, 1);
        while let Some(attribute) = rest.strip_prefix('.').filter(|it| it.starts_with(is_ident_start)) {
            if parts == MAX_NAME_PARTS {
                return failure(rest, format!("a name has at most {} parts separated by '.', such as open.date", MAX_NAME_PARTS));
            }
            let (after, attribute) = take_while1::<_, _, PError>(is_ident_char)(attribute)?;
            lower.push('.');
            lower.push_str(&attribute.to_ascii_lowercase());
            rest = after;
            parts += 1;
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
        assert_eq!(select.limit.as_ref().and_then(Count::literal), Some(10));
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
        let ExprKind::Logical(LogicalOp::Or, operands) = where_kind("SELECT * WHERE a OR b AND NOT c") else {
            panic!()
        };
        let ExprKind::Logical(LogicalOp::And, and_operands) = &operands[1].kind else {
            panic!()
        };
        assert!(matches!(and_operands[1].kind, ExprKind::Unary(UnaryOp::Not, _)));

        // 1 + 2 * 3 = 7
        let ExprKind::Binary(BinaryOp::Eq, left, _) = where_kind("SELECT * WHERE 1 + 2 * 3 = 7") else {
            panic!()
        };
        let ExprKind::Arith(_, rest) = left.kind else { panic!() };
        assert_eq!(rest[0].0, ArithOp::Add);
        assert!(matches!(&rest[0].1.kind, ExprKind::Arith(_, mul) if mul[0].0 == ArithOp::Mul));
    }

    #[test]
    fn chains_are_flat_and_parentheses_create_no_nodes() {
        // a + b - c is one chain evaluated left to right
        let ExprKind::Binary(_, left, _) = where_kind("SELECT * WHERE 1 + 2 - 3 + 4 = 4") else {
            panic!()
        };
        let ExprKind::Arith(_, rest) = &left.kind else { panic!() };
        assert_eq!(
            rest.iter().map(|(op, _)| *op).collect::<Vec<_>>(),
            vec![ArithOp::Add, ArithOp::Sub, ArithOp::Add]
        );
        // a thousand ORs are one node of height 2
        let sql = format!("SELECT * WHERE {}TRUE", "FALSE OR ".repeat(1000));
        let filter = parse_ok(&sql).where_clause.unwrap();
        assert!(matches!(&filter.kind, ExprKind::Logical(LogicalOp::Or, operands) if operands.len() == 1001));
        assert_eq!(filter.height, 2);
        // parentheses only group
        let nested = parse_ok("SELECT * WHERE ((((TRUE))))").where_clause.unwrap();
        assert_eq!((nested.kind, nested.height), (ExprKind::Literal(Literal::Bool(true)), 1));
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

    #[test]
    fn attribute_access_is_a_dotted_column_name() {
        let select = parse_ok("SELECT Open.Date, close.meta.x, account FROM #accounts WHERE open.date > 2020-01-01");
        let Targets::List(targets) = &select.targets else { panic!() };
        assert_eq!(targets[0].expr.kind, ExprKind::Column("open.date".into()));
        assert_eq!(targets[0].expr.span, Span::new(7, 16));
        assert_eq!(targets[1].expr.kind, ExprKind::Column("close.meta.x".into()));
        assert_eq!(targets[2].expr.kind, ExprKind::Column("account".into()));
        // a number after the dot, or a space, is not an attribute
        assert!(parse("SELECT open.1").is_err());
        assert!(parse("SELECT open .date").is_err());
        assert!(parse("SELECT open.").is_err());
        // at most MAX_NAME_PARTS parts, the error at the dot of the first extra one
        let name = ["a"; MAX_NAME_PARTS].join(".");
        assert!(matches!(parse_ok(&format!("SELECT {name}")).targets, Targets::List(_)));
        let err = parse_err(&format!("SELECT {name}.b"));
        assert_eq!((err.kind, err.column), (QueryErrorKind::Parse, Some(8 + name.len())), "{}", err);
        assert!(err.message.contains("at most 8 parts"), "{}", err.message);
    }

    #[test]
    fn from_clause_tables() {
        let table = |src: &str| parse_ok(src).table.map(|table| (table.name, table.bare, table.span));
        assert_eq!(table("SELECT * FROM #prices"), Some(("prices".into(), false, Span::new(14, 21))));
        assert_eq!(table("SELECT * FROM #Prices WHERE TRUE"), Some(("Prices".into(), false, Span::new(14, 21))));
        assert_eq!(
            table("select date from #entries where type = 'open' order by date limit 3;"),
            Some(("entries".into(), false, Span::new(17, 25)))
        );
        // a bare name that is not a postings column stands for a table, as in beanquery
        assert_eq!(table("SELECT * FROM prices"), Some(("prices".into(), true, Span::new(14, 20))));
        assert_eq!(table("SELECT * FROM nosuch GROUP BY 1"), Some(("nosuch".into(), true, Span::new(14, 20))));
        // ... but a postings column, a keyword or anything followed by an expression is a filter
        for src in [
            "SELECT * FROM payee",
            "SELECT * FROM prices = 1",
            "SELECT * FROM year = 2016",
            "SELECT * FROM TRUE",
            "SELECT * FROM CLEAR",
        ] {
            let select = parse_ok(src);
            assert!(select.table.is_none(), "{}", src);
        }
        assert!(parse_ok("SELECT * FROM OPEN ON 2016-01-01").table.is_none());

        let err = parse_err("SELECT * FROM #");
        assert_eq!(err.column, Some(16), "{}", err);
        assert!(err.message.contains("expected a table name after '#'"), "{}", err.message);
        let err = parse_err("SELECT * FROM # prices");
        assert_eq!(err.column, Some(16), "{}", err);
        // a table is a complete FROM clause
        for (src, column) in [
            ("SELECT count(*) FROM #postings OPEN ON 2016-01-01", 32),
            ("SELECT count(*) FROM #entries CLOSE ON 2017-01-01 CLEAR", 31),
            ("SELECT count(*) FROM prices CLEAR", 29),
        ] {
            let err = parse_err(src);
            assert_eq!(err.kind, QueryErrorKind::Parse, "{}", src);
            assert_eq!(err.column, Some(column), "{}: {}", src, err);
            assert!(err.message.contains("cannot be combined with a table"), "{}", err.message);
        }
        let err = parse_err("SELECT * FROM #prices currency = 'USD'");
        assert_eq!(err.column, Some(23), "{}", err);
        assert!(err.message.contains("filter its rows with WHERE"), "{}", err.message);
        // BALANCES and JOURNAL always read the postings
        for (src, column) in [
            ("BALANCES FROM #prices", 15),
            ("JOURNAL 'Assets' FROM #postings", 23),
            ("BALANCES FROM prices WHERE TRUE", 15),
        ] {
            let err = parse_err(src);
            assert_eq!(err.column, Some(column), "{}: {}", src, err);
            assert!(err.message.contains("always reads the postings"), "{}", err.message);
        }
    }

    #[test]
    fn from_clause_period_modifiers() {
        let date = |y, m, d| ExprKind::Literal(Literal::Date(NaiveDate::from_ymd_opt(y, m, d).unwrap()));

        let select = parse_ok("SELECT * FROM OPEN ON 2016-01-01 CLOSE ON 2017-01-01 CLEAR WHERE TRUE");
        assert!(select.from.is_none());
        assert!(select.where_clause.is_some());
        let period = select.period.unwrap();
        assert_eq!(period.open.unwrap().kind, date(2016, 1, 1));
        assert_eq!(period.close.unwrap().unwrap().kind, date(2017, 1, 1));
        assert!(period.clear);

        // with an expression, lower case, a bare CLOSE and parameters
        let select = parse_ok("select * from year = 2016 open on :from close clear");
        assert!(matches!(select.from.unwrap().kind, ExprKind::Binary(BinaryOp::Eq, ..)));
        let period = select.period.unwrap();
        assert_eq!(period.open.unwrap().kind, ExprKind::Param(ParamRef::Named("from".into())));
        assert_eq!(period.close, Some(None));
        assert!(period.clear);

        let period = parse_ok("SELECT * FROM account ~ 'Assets' CLOSE ON $1").period.unwrap();
        assert_eq!((period.open, period.clear), (None, false));
        assert_eq!(period.close.unwrap().unwrap().kind, ExprKind::Param(ParamRef::Positional(1)));

        let select = parse_ok("SELECT * FROM CLEAR");
        assert_eq!(
            select.period,
            Some(Period {
                open: None,
                close: None,
                clear: true
            })
        );

        // no modifiers: no period
        assert!(parse_ok("SELECT * FROM year = 2016").period.is_none());

        // BALANCES and JOURNAL share the FROM clause
        let balances = parse_ok("BALANCES AT cost FROM year = 2016 OPEN ON 2016-01-01 CLOSE CLEAR WHERE TRUE");
        assert!(balances.from.is_some() && balances.where_clause.is_some());
        assert_eq!(balances.period.unwrap().close, Some(None));
        let journal = parse_ok("JOURNAL 'Assets' FROM CLOSE ON 2017-01-01");
        assert!(journal.from.is_none());
        assert_eq!(journal.period.unwrap().close.unwrap().unwrap().kind, date(2017, 1, 1));
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

        let err = parse_err("PRINT");
        assert!(err.message.contains("not supported"));
        let err = parse_err("SELECT * FROM CLEAR OPEN ON 2024-01-01");
        assert_eq!(err.column, Some(21), "{}", err);
        assert!(err.message.contains("in this order"), "{}", err.message);
        let err = parse_err("SELECT * FROM OPEN 2024-01-01");
        assert!(err.message.contains("expected ON after OPEN"), "{}", err.message);
        let err = parse_err("SELECT * FROM OPEN ON '2024-01-01'");
        assert_eq!(err.column, Some(23), "{}", err);
        assert!(err.message.contains("expected a date after OPEN ON"), "{}", err.message);
        let err = parse_err("SELECT * FROM CLOSE ON 2024");
        assert!(err.message.contains("expected a date after CLOSE ON"), "{}", err.message);
        let err = parse_err("SELECT * FROM postings OPEN ON 2024-01-01");
        assert!(err.message.contains("cannot be combined with a table (#postings)"), "{}", err.message);
        let err = parse_err("SELECT * WHERE date > 2024-13-01");
        assert!(err.message.contains("invalid date"));
        let err = parse_err("SELECT * LIMIT -1");
        assert!(err.message.contains("LIMIT"));
        let err = parse_err("SELECT sum(position");
        assert!(err.message.contains("')'"), "{}", err.message);
    }

    /// The target names (aliases) of a parsed statement.
    fn target_names(select: &Select) -> Vec<String> {
        let Targets::List(targets) = &select.targets else { panic!() };
        targets.iter().map(|it| it.alias.clone().unwrap_or_default()).collect()
    }

    #[test]
    fn parses_balances() {
        let select = parse_ok("balances at Cost from year = 2016 where account ~ 'Assets';");
        assert_eq!(target_names(&select), ["account", "sum(cost(position))"]);
        assert!(select.from.is_some() && select.where_clause.is_some());
        assert_eq!(select.group_by.as_ref().map(Vec::len), Some(2));
        assert_eq!(select.order_by.as_ref().map(Vec::len), Some(1));
        // the AT function call spans the function name
        let src = "BALANCES AT units";
        let Targets::List(targets) = parse_ok(src).targets else { panic!() };
        let ExprKind::Call { name, args, .. } = &targets[1].expr.kind else { panic!() };
        assert_eq!(name, "sum");
        assert!(matches!(&args[0].kind, ExprKind::Call { name, .. } if name == "units"));
        assert_eq!(&src[args[0].span.start..args[0].span.end], "units");

        let plain = parse_ok("BALANCES");
        assert_eq!(target_names(&plain), ["account", "sum(position)"]);
        assert!(plain.from.is_none() && plain.where_clause.is_none());
    }

    #[test]
    fn parses_journal() {
        let select = parse_ok("JOURNAL 'Assets:Bank' AT value FROM year = 2016");
        assert_eq!(
            target_names(&select),
            [
                "date",
                "flag",
                "maxwidth(payee, 48)",
                "maxwidth(narration, 80)",
                "account",
                "value(position)",
                "value(balance)"
            ]
        );
        assert!(select.from.is_some());
        let ExprKind::Binary(BinaryOp::Match, account, pattern) = select.where_clause.unwrap().kind else {
            panic!()
        };
        assert_eq!(account.kind, ExprKind::Column("account".into()));
        assert_eq!(pattern.kind, ExprKind::Literal(Literal::Str("Assets:Bank".into())));

        let select = parse_ok("journal \"Cash\"");
        assert_eq!(target_names(&select)[5..], ["position", "balance"]);
        assert!(select.from.is_none() && select.group_by.is_none() && select.order_by.is_none());
        let select = parse_ok("JOURNAL :account");
        let ExprKind::Binary(_, _, pattern) = select.where_clause.unwrap().kind else {
            panic!()
        };
        assert_eq!(pattern.kind, ExprKind::Param(ParamRef::Named("account".into())));
        // no account: no filter
        assert!(parse_ok("JOURNAL FROM year = 2016").where_clause.is_none());
    }

    #[test]
    fn parses_having_and_pivot_by_in_grammar_order() {
        let select = parse_ok(
            "select year, account, sum(number) as total group by 1, 2 having sum(number) > 10 and count(*) > 1 \
             order by 3 desc pivot by Account, 1 limit 5",
        );
        assert!(matches!(select.having.as_ref().unwrap().kind, ExprKind::Logical(LogicalOp::And, _)));
        let [first, second] = select.pivot_by.as_ref().unwrap();
        // names are lower-cased like columns; indexes are integer literals
        assert_eq!(first.as_identifier(), Some("account"));
        assert_eq!(second.as_index(), Some(1));
        assert_eq!(select.limit.as_ref().and_then(Count::literal), Some(5));
        assert!(parse_ok("SELECT a GROUP BY a").having.is_none() && parse_ok("SELECT a").pivot_by.is_none());
    }

    #[test]
    fn limit_and_offset_take_integers_or_parameters() {
        let select = parse_ok("SELECT a LIMIT 10 OFFSET 0");
        assert_eq!(select.limit.as_ref().and_then(Count::literal), Some(10));
        assert_eq!(select.offset.as_ref().and_then(Count::literal), Some(0));
        let src = "select a limit :size offset $2;";
        let select = parse_ok(src);
        let (limit, offset) = (select.limit.unwrap(), select.offset.unwrap());
        assert_eq!(limit.value, CountValue::Param(ParamRef::Named("size".into())));
        assert_eq!(&src[limit.span.start..limit.span.end], ":size");
        assert_eq!(offset.value, CountValue::Param(ParamRef::Positional(2)));
        assert_eq!(&src[offset.span.start..offset.span.end], "$2");
        assert!(parse_ok("SELECT a LIMIT 1").offset.is_none());
        // OFFSET is a keyword
        assert!(parse_err("SELECT offset").message.contains("keyword OFFSET"));
        assert!(parse_err("SELECT a OFFSET 1 LIMIT 1").message.contains("OFFSET must follow LIMIT"));
        assert!(parse_err("SELECT a LIMIT 1 OFFSET 1 PIVOT BY a, b")
            .message
            .contains("PIVOT BY must come before LIMIT"));
    }

    #[test]
    fn having_and_pivot_by_syntax_errors() {
        for (src, column, message) in [
            ("SELECT count(*) HAVING count(*) > 1", 17, "HAVING requires a GROUP BY clause"),
            ("SELECT a GROUP BY a ORDER BY a HAVING count(*) > 1", 32, "HAVING must follow GROUP BY"),
            ("SELECT a GROUP BY a HAVING count(*) > 1 HAVING count(*) < 9", 41, "HAVING may appear only once"),
            ("SELECT a GROUP BY a HAVING", 27, "end of query"),
            ("SELECT a, b GROUP BY a, b PIVOT a, b", 33, "expected BY after PIVOT, found 'a'"),
            ("SELECT a, b GROUP BY a, b PIVOT BY a", 37, "two columns separated by a comma"),
            ("SELECT a, b GROUP BY a, b PIVOT BY a, b, c", 40, "exactly two columns"),
            ("SELECT a, b GROUP BY a, b PIVOT BY a + 1, b", 38, "separated by a comma, found '+'"),
            ("SELECT a, b GROUP BY a, b PIVOT BY root(a, 2), b", 36, "expected a target name or number"),
            ("SELECT a, b GROUP BY a, b PIVOT BY 'a', b", 36, "expected a target name or number"),
            ("SELECT a, b GROUP BY a, b PIVOT BY a, -1", 39, "expected a target name or number"),
            ("SELECT a, b GROUP BY a, b PIVOT BY 1.5, 2", 36, "expected a target name or number"),
            ("SELECT a, b GROUP BY a, b PIVOT BY a, limit", 39, "expected a target name or number"),
            ("SELECT a, b GROUP BY a, b LIMIT 1 PIVOT BY a, b", 35, "PIVOT BY must come before LIMIT"),
            ("SELECT a, b GROUP BY a, b PIVOT BY a, b ORDER BY a", 41, "unexpected 'ORDER'"),
        ] {
            let err = parse_err(src);
            assert_eq!((err.line, err.column), (Some(1), Some(column)), "{}: {}", src, err);
            assert!(err.message.contains(message), "{}: {}", src, err.message);
        }
    }

    #[test]
    fn statement_syntax_errors() {
        for (src, column, message) in [
            ("BALANCES AT", 12, "expected a function name after AT"),
            ("BALANCES AT 1", 13, "expected a function name after AT"),
            ("JOURNAL AT FROM year = 2016", 12, "expected a function name after AT"),
            ("JOURNAL 'x' WHERE TRUE", 13, "JOURNAL has no WHERE clause"),
            ("JOURNAL 'x' 'y'", 13, "unexpected"),
            ("BALANCES WHERE", 15, "end of query"),
            ("BALANCES GROUP BY account", 10, "unexpected 'GROUP'"),
            ("BALANCES FROM CLEAR CLOSE", 21, "in this order"),
            ("JOURNAL 'x' FROM OPEN 2016-01-01", 23, "expected ON after OPEN"),
            ("JOURNAL 'unterminated", 9, "unterminated string literal"),
            ("SELECTS *", 1, "expected SELECT, BALANCES or JOURNAL, found 'SELECTS'"),
        ] {
            let err = parse_err(src);
            assert_eq!((err.line, err.column), (Some(1), Some(column)), "{}: {}", src, err);
            assert!(err.message.contains(message), "{}: {}", src, err.message);
        }
    }
}
