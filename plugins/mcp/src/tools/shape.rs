//! The shape of a tool's result: `structuredContent`, its `outputSchema`, and the two forms of `CallToolResult`.
//!
//! A tool that reads rows answers `{columns, rows, row_count, truncated}` with the cell encoding of
//! `POST /api/query`; a failure of the query or the host is a tool execution error: `isError: true` with the
//! message as text (`query error at line L, column C: …`, or `<kind>: <message>`) and **no** `structuredContent` (a client validating `structuredContent` against the tool's
//! `outputSchema` would otherwise reject the error itself).

use serde_json::{json, Value};

use crate::host::{HostError, HostErrorKind, QueryResult};

/// a tool's failure, reported as `isError: true`
#[derive(Debug, Clone, PartialEq)]
pub struct ToolError {
    pub message: String,
}

impl From<HostError> for ToolError {
    fn from(error: HostError) -> ToolError {
        let message = match (error.kind, error.line, error.column) {
            (HostErrorKind::Query, Some(line), Some(column)) => format!("query error at line {line}, column {column}: {}", error.message),
            (HostErrorKind::Query, _, _) => format!("query error: {}", error.message),
            (kind, _, _) => format!("{}: {}", kind_name(kind), error.message),
        };
        ToolError { message }
    }
}

fn kind_name(kind: HostErrorKind) -> &'static str {
    match kind {
        HostErrorKind::Denied => "denied",
        HostErrorKind::NotFound => "not_found",
        HostErrorKind::TooLarge => "too_large",
        HostErrorKind::Unsupported => "unsupported",
        HostErrorKind::Invalid => "invalid",
        HostErrorKind::InvalidInput => "invalid_input",
        HostErrorKind::Unavailable => "unavailable",
        HostErrorKind::Query => "query",
        _ => "other",
    }
}

/// `structuredContent` of a rows tool: the result, at most `limit` rows
pub fn rows(result: QueryResult, limit: usize) -> Value {
    let truncated = result.rows.len() > limit;
    let rows: Vec<Vec<Value>> = result.rows.into_iter().take(limit).collect();
    json!({
        "columns": result.columns,
        "rows": rows,
        "row_count": rows.len(),
        "truncated": truncated,
    })
}

/// the `outputSchema` of a rows tool without an `offset` argument
pub fn rows_schema() -> Value {
    rows_schema_with("true when more rows exist than were returned: narrow the query, or raise `limit`")
}

/// the `outputSchema` of a rows tool that pages with `offset`
pub fn paged_rows_schema() -> Value {
    rows_schema_with("true when more rows exist than were returned: page on with `offset`, or raise `limit`")
}

fn rows_schema_with(truncated: &str) -> Value {
    json!({
        "type": "object",
        "description": "A query result: the columns in order, then one array per row with a cell per column. A cell is \
                        null, a boolean, an integer, a string (decimals, dates and times are strings), a list of \
                        strings, an amount {number, currency}, a position {units, cost}, an inventory {positions} or \
                        a list of {key, value} metadata.",
        "properties": {
            "columns": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {"name": {"type": "string"}, "type": {"type": "string"}},
                    "required": ["name", "type"],
                },
            },
            "rows": {"type": "array", "items": {"type": "array"}},
            "row_count": {"type": "integer", "description": "the number of rows returned"},
            "truncated": {"type": "boolean", "description": truncated},
        },
        "required": ["columns", "rows", "row_count", "truncated"],
    })
}

/// the `outputSchema` of `ledger_info`
pub fn ledger_info_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "title": {"type": ["string", "null"], "description": "the ledger's title option"},
            "operating_currency": {"type": "string"},
            "timezone": {"type": "string", "description": "an IANA timezone name"},
            "now": {"type": "string", "description": "the current time by the ledger's clock, RFC 3339"},
            "today": {"type": "string", "description": "today by the ledger's clock, YYYY-MM-DD"},
        },
        "required": ["title", "operating_currency", "timezone", "now", "today"],
    })
}

/// the `outputSchema` of `schema`
pub fn schema_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "columns": {"type": "array", "description": "the columns of the default table, postings"},
            "functions": {"type": "array", "description": "every function with its signature; aggregate ones are marked"},
            "tables": {"type": "array", "description": "every table a query can read with FROM #name, with its columns"},
            "keywords": {"type": "array", "items": {"type": "string"}},
        },
        "required": ["columns", "functions", "tables", "keywords"],
    })
}

/// a successful `CallToolResult`: the JSON as text, and as `structuredContent`
pub fn success(structured: Value) -> Value {
    json!({
        "content": [{"type": "text", "text": structured.to_string()}],
        "structuredContent": structured,
    })
}

/// a failed `CallToolResult`
pub fn failure(error: &ToolError) -> Value {
    json!({
        "content": [{"type": "text", "text": error.message}],
        "isError": true,
    })
}

#[cfg(test)]
mod test {
    use serde_json::json;

    use super::{failure, rows, success, ToolError};
    use crate::host::{Column, HostError, HostErrorKind, QueryResult};

    fn result(n: usize) -> QueryResult {
        QueryResult {
            columns: vec![Column {
                name: "n".to_owned(),
                ty: "int".to_owned(),
            }],
            rows: (0..n).map(|i| vec![json!(i)]).collect(),
        }
    }

    #[test]
    fn truncates_rows_at_the_limit() {
        assert_eq!(
            rows(result(3), 2),
            json!({"columns": [{"name": "n", "type": "int"}], "rows": [[0], [1]], "row_count": 2, "truncated": true})
        );
        assert_eq!(rows(result(2), 2)["truncated"], false);
        assert_eq!(
            rows(result(0), 2),
            json!({"columns": [{"name": "n", "type": "int"}], "rows": [], "row_count": 0, "truncated": false})
        );
    }

    #[test]
    fn success_carries_the_json_as_text_too() {
        let call = success(json!({"a": 1}));
        assert_eq!(call["content"], json!([{"type": "text", "text": "{\"a\":1}"}]));
        assert_eq!(call["structuredContent"], json!({"a": 1}));
        assert!(call.get("isError").is_none());
    }

    #[test]
    fn errors_are_text_only() {
        let query = HostError {
            kind: HostErrorKind::Query,
            message: "unknown column `nope`".to_owned(),
            line: Some(3),
            column: Some(8),
        };
        assert_eq!(ToolError::from(query).message, "query error at line 3, column 8: unknown column `nope`");
        let limit = HostError::new(HostErrorKind::Query, "the result is too large");
        assert_eq!(ToolError::from(limit).message, "query error: the result is too large");
        let host = HostError::new(HostErrorKind::InvalidInput, "bad parameter");
        assert_eq!(ToolError::from(host).message, "invalid_input: bad parameter");
        let unavailable = HostError::new(HostErrorKind::Unavailable, "outside a request");
        assert_eq!(ToolError::from(unavailable).message, "unavailable: outside a request");
        let call = failure(&ToolError { message: "boom".to_owned() });
        assert_eq!(call, json!({"content": [{"type": "text", "text": "boom"}], "isError": true}));
    }
}
