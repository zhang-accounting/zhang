//! Query errors with 1-based, character-based source positions.

use std::fmt;

/// A byte range in the query text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    pub fn new(start: usize, end: usize) -> Self {
        Span { start, end }
    }
}

/// Which stage produced a [`QueryError`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryErrorKind {
    /// the query text is not valid syntax
    Parse,
    /// unknown column/function, type mismatch, invalid grouping, unbound parameter, ...
    Compile,
    /// a runtime failure while evaluating rows (e.g. integer overflow)
    Eval,
}

/// An error produced while parsing, compiling or executing a query.
///
/// `line` and `column` are 1-based and count characters (Unicode scalar values), not
/// bytes; they are `None` when the error has no source position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryError {
    pub kind: QueryErrorKind,
    pub message: String,
    pub line: Option<usize>,
    pub column: Option<usize>,
}

impl QueryError {
    /// An error without a source position.
    pub fn new(kind: QueryErrorKind, message: impl Into<String>) -> Self {
        QueryError {
            kind,
            message: message.into(),
            line: None,
            column: None,
        }
    }

    /// An error located at byte offset `offset` of `source`.
    pub fn at(kind: QueryErrorKind, message: impl Into<String>, source: &str, offset: usize) -> Self {
        let (line, column) = line_column(source, offset);
        QueryError {
            kind,
            message: message.into(),
            line: Some(line),
            column: Some(column),
        }
    }
}

impl fmt::Display for QueryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (self.line, self.column) {
            (Some(line), Some(column)) => write!(f, "{} (line {}, column {})", self.message, line, column),
            _ => f.write_str(&self.message),
        }
    }
}

impl std::error::Error for QueryError {}

/// Convert a byte offset into a 1-based (line, column) pair, counting characters.
pub(crate) fn line_column(source: &str, offset: usize) -> (usize, usize) {
    let mut offset = offset.min(source.len());
    while !source.is_char_boundary(offset) {
        offset -= 1;
    }
    let before = &source[..offset];
    let line = before.matches('\n').count() + 1;
    let line_start = before.rfind('\n').map(|it| it + 1).unwrap_or(0);
    let column = before[line_start..].chars().count() + 1;
    (line, column)
}

/// An error raised while compiling or evaluating, located by a [`Span`] that is resolved
/// against the query text once it reaches the public API.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LocatedError {
    pub kind: QueryErrorKind,
    pub message: String,
    pub span: Option<Span>,
}

impl LocatedError {
    pub fn compile(message: impl Into<String>, span: Span) -> Self {
        LocatedError {
            kind: QueryErrorKind::Compile,
            message: message.into(),
            span: Some(span),
        }
    }
    pub fn eval(message: impl Into<String>, span: Option<Span>) -> Self {
        LocatedError {
            kind: QueryErrorKind::Eval,
            message: message.into(),
            span,
        }
    }

    pub fn resolve(self, source: &str) -> QueryError {
        match self.span {
            Some(span) => QueryError::at(self.kind, self.message, source, span.start),
            None => QueryError::new(self.kind, self.message),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn columns_count_characters_not_bytes() {
        let source = "SELECT '午餐'\n  , x";
        assert_eq!(line_column(source, 0), (1, 1));
        let x = source.find('x').unwrap();
        assert_eq!(line_column(source, x), (2, 5));
        let after_literal = source.find('\n').unwrap();
        assert_eq!(line_column(source, after_literal), (1, 12));
    }
}
