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

use std::collections::BTreeMap;

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::abi;
use crate::error::{host_result, Error, HostError};

/// how a body travels in a JSON string
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BodyEncoding {
    /// the body is the string itself
    #[default]
    Utf8,
    /// the body is the standard base64 (with padding) of the bytes, for a body that is not UTF-8
    Base64,
}

/// the HTTP request a router handles
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    /// the method, upper case, e.g. `GET`
    #[serde(default)]
    pub method: String,
    /// the path below the plugin's route, starting with `/`, percent-encoded as sent: `/api/plugins/{name}/by-month`
    /// gives `/by-month`, and `/api/plugins/{name}` gives `/`
    #[serde(default)]
    pub path: String,
    /// the decoded query string: every key with all its values, in order
    #[serde(default)]
    pub query: BTreeMap<String, Vec<String>>,
    /// the headers, names lower case, the values of a repeated header joined with `, `. zhang never passes the
    /// credential headers `authorization`, `proxy-authorization` and `cookie`
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    /// the body, encoded as [`Request::body_encoding`] says; empty when there is none
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub body_encoding: BodyEncoding,
}

impl Request {
    /// the first value of the query parameter `key`
    pub fn query_param(&self, key: &str) -> Option<&str> {
        self.query.get(key).and_then(|values| values.first()).map(String::as_str)
    }

    /// the header `name`, in any case
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(&name.to_ascii_lowercase()).map(String::as_str)
    }

    /// the body as bytes
    pub fn body_bytes(&self) -> Result<Vec<u8>, Error> {
        match self.body_encoding {
            BodyEncoding::Utf8 => Ok(self.body.clone().into_bytes()),
            BodyEncoding::Base64 => Ok(BASE64.decode(&self.body)?),
        }
    }

    /// the body parsed as JSON
    pub fn json<T: DeserializeOwned>(&self) -> Result<T, Error> {
        Ok(serde_json::from_slice(&self.body_bytes()?)?)
    }
}

/// The HTTP response a router returns. zhang sets `content-length`, `transfer-encoding` and `connection` itself;
/// without a `content-type` header the response is `text/plain; charset=utf-8`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Response {
    /// the HTTP status
    pub status: u16,
    /// the headers, one value per name
    pub headers: BTreeMap<String, String>,
    /// the body, encoded as [`Response::body_encoding`] says
    pub body: String,
    pub body_encoding: BodyEncoding,
}

impl Default for Response {
    /// an empty `200 OK`
    fn default() -> Self {
        Response {
            status: 200,
            headers: BTreeMap::new(),
            body: String::new(),
            body_encoding: BodyEncoding::Utf8,
        }
    }
}

impl Response {
    fn with_content(content_type: &str, body: String, body_encoding: BodyEncoding) -> Response {
        Response {
            body,
            body_encoding,
            ..Response::default()
        }
        .with_header("content-type", content_type)
    }

    /// a `200 OK` plain text response
    pub fn text(body: impl Into<String>) -> Response {
        Response::with_content("text/plain; charset=utf-8", body.into(), BodyEncoding::Utf8)
    }

    /// a `200 OK` HTML page
    pub fn html(body: impl Into<String>) -> Response {
        Response::with_content("text/html; charset=utf-8", body.into(), BodyEncoding::Utf8)
    }

    /// a `200 OK` JSON response
    pub fn json<T: Serialize + ?Sized>(value: &T) -> Result<Response, Error> {
        Ok(Response::with_content("application/json", serde_json::to_string(value)?, BodyEncoding::Utf8))
    }

    /// a `200 OK` response with any bytes, e.g. an image
    pub fn bytes(content_type: &str, body: impl AsRef<[u8]>) -> Response {
        Response::with_content(content_type, BASE64.encode(body), BodyEncoding::Base64)
    }

    /// this response with the HTTP status `status`
    pub fn with_status(mut self, status: u16) -> Response {
        self.status = status;
        self
    }

    /// this response with the header `name` set to `value`; names are lower-cased
    pub fn with_header(mut self, name: &str, value: impl Into<String>) -> Response {
        self.headers.insert(name.to_ascii_lowercase(), value.into());
        self
    }
}

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

/// what [`ledger_info`] answers
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerInfo {
    /// the `title` option
    pub title: Option<String>,
    /// the `operating_currency` option
    pub operating_currency: String,
    /// the ledger's timezone, an IANA name
    pub timezone: String,
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
