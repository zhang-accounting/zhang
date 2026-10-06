//! Router plugins: HTTP endpoints served by a plugin.
//!
//! A plugin with a `router` handler serves `/api/plugins/{name}` and every path below it, for any HTTP method,
//! behind zhang's own authentication. `{name}` is the plugin's name. The handler gets a [`Request`] and returns a
//! [`Response`] of any content type, so a router can answer JSON for a script or a whole HTML page.
//!
//! Every request runs in a fresh plugin instance: nothing survives between requests. The ledger is read-only:
//! [`query`] runs BQL over it and [`ledger_info`] describes it, and no host function changes it. Both answer
//! [`HostErrorKind::Unavailable`](crate::HostErrorKind::Unavailable) outside a request, e.g. in a processor.
//!
//! **Security:** a router's pages are served from zhang's own address, so a script on such a page can call
//! zhang's whole API with the user's session. Never echo untrusted input into HTML unescaped.

use serde::{Deserialize, Serialize};
pub use zhang_shared::plugin_abi::{Encoding as BodyEncoding, LedgerInfo, Request, Response};

use crate::abi;
use crate::error::{host_result, HostError};

/// the result of a query: what `POST /api/query` answers in `data`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QueryResult {
    pub columns: Vec<Column>,
    /// one row per result row, a cell per column; a cell is a string, number, date, amount or inventory as the
    /// query API encodes it
    pub rows: Vec<Vec<serde_json::Value>>,
}

/// a column of a [`QueryResult`]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Column {
    pub name: String,
    /// the column's type as the query API names it, e.g. `str`, `number` or `inventory`
    #[serde(rename = "type")]
    pub ty: String,
}

impl QueryResult {
    /// the position of the column `name`
    pub fn column(&self, name: &str) -> Option<usize> {
        self.columns.iter().position(|column| column.name == name)
    }
}

/// Run a read-only BQL query (see the query language page of zhang's user guide) over the ledger, with the time
/// and result size limits of `POST /api/query`. A query that fails is a
/// [`HostErrorKind::Query`](crate::HostErrorKind::Query) error with its line and column.
pub fn query(bql: &str) -> Result<QueryResult, HostError> {
    host_result(abi::QUERY, &abi::query(bql)?)
}

/// the ledger's title, operating currency and timezone
pub fn ledger_info() -> Result<LedgerInfo, HostError> {
    host_result(abi::LEDGER_INFO, &abi::ledger_info()?)
}

#[cfg(test)]
mod test {
    use serde_json::json;

    use super::{BodyEncoding, LedgerInfo, QueryResult, Request, Response};
    use crate::error::{host_result, HostErrorKind};

    #[test]
    fn should_read_the_request_zhang_sends() {
        let request: Request = serde_json::from_value(json!({
            "method": "POST",
            "path": "/by-month",
            "query": {"year": ["2024", "2025"]},
            "headers": {"content-type": "application/json"},
            "body": "{\"a\": 1}",
            "body_encoding": "utf8",
            "a field from a newer zhang": true,
        }))
        .unwrap();
        assert_eq!(request.query_param("year"), Some("2024"));
        assert_eq!(request.header("Content-Type"), Some("application/json"));
        assert_eq!(request.json::<serde_json::Value>().unwrap(), json!({"a": 1}));

        let binary = Request {
            body: "/wBB".to_owned(),
            body_encoding: BodyEncoding::Base64,
            ..Request::default()
        };
        assert_eq!(binary.body_bytes().unwrap(), vec![0xff, 0x00, 0x41]);
    }

    #[test]
    fn should_build_the_response_zhang_reads() {
        assert_eq!(
            serde_json::to_value(Response::html("<h1>hi</h1>").with_status(201).with_header("X-Report", "monthly")).unwrap(),
            json!({
                "status": 201,
                "headers": {"content-type": "text/html; charset=utf-8", "x-report": "monthly"},
                "body": "<h1>hi</h1>",
                "body_encoding": "utf8",
            })
        );
        let image = Response::bytes("image/png", [0xff, 0x00, 0x41]);
        assert_eq!((image.body.as_str(), image.body_encoding), ("/wBB", BodyEncoding::Base64));
        assert_eq!(Response::json(&json!([1])).unwrap().headers["content-type"], "application/json");
        assert_eq!(Response::default().status, 200);
    }

    #[test]
    fn should_read_query_results_and_ledger_info() {
        let result: QueryResult = host_result(
            "zhang_query",
            br#"{"Ok": {"columns": [{"name": "account", "type": "str"}], "rows": [["Assets:Cash"]]}}"#,
        )
        .unwrap();
        assert_eq!((result.column("account"), result.column("nope")), (Some(0), None));
        assert_eq!(result.columns[0].ty, "str");
        let info: LedgerInfo = host_result(
            "zhang_ledger_info",
            br#"{"Ok": {"title": null, "operating_currency": "CNY", "timezone": "Asia/Shanghai"}}"#,
        )
        .unwrap();
        assert_eq!((info.title, info.operating_currency.as_str()), (None, "CNY"));
        assert_eq!(super::query("SELECT 1").unwrap_err().kind, HostErrorKind::Unavailable);
    }
}
