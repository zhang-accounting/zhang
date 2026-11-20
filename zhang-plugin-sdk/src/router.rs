//! Router plugins: HTTP endpoints served by a plugin.
//!
//! A plugin with a `router` handler serves `/api/plugins/{name}` and every path below it, for any HTTP method,
//! behind zhang's own authentication. `{name}` is the plugin's name. The handler gets a [`Request`] and returns a
//! [`Response`] of any content type, so a router can answer JSON for a script or a whole HTML page.
//!
//! Every request runs in a fresh plugin instance: nothing survives between requests. The ledger is read-only:
//! [`query`] and [`query_with_params`] run BQL over it, [`schema`] lists what a query can use, [`ledger_info`]
//! describes the ledger, and no host function changes it, with one exception a plugin opts into and the ledger owner
//! grants: [`write_preview`] and [`write_commit`] append transactions and `open`s to the ledger's files (see
//! "Writes" below). All of them answer [`HostErrorKind::Unavailable`](crate::HostErrorKind::Unavailable) outside a
//! request, e.g. in a processor.
//!
//! # Writes
//!
//! A plugin that declares `writes` in [`plugin!`](crate::plugin) and whose `plugin` directive grants them with
//! `allowed_writes` appends directives in two steps: [`write_preview`] validates a batch exactly as
//! `POST /api/transactions/preview` would, shows the text the ledger writes, the file it goes to and what the ledger
//! would report, and mints a token when every entry is clean; [`write_commit`] appends the same batch with that
//! token, in one write, and the server reloads the ledger once the request is answered. A token is bound to the
//! ledger as it was previewed: after a reload, a commit or another change, or after 10 minutes, the commit is a
//! [`HostErrorKind::Conflict`](crate::HostErrorKind::Conflict) and the plugin previews again. Amounts are strings
//! ([`UnitWrite::Amount`]): the host refuses a JSON number anywhere in a batch, so no float ever reaches the ledger.
//!
//! **Security:** a router's pages are served from zhang's own address, so a script on such a page can call
//! zhang's whole API with the user's session. Never echo untrusted input into HTML unescaped. Never write what a
//! user typed into a query's text either: bind it with [`query_with_params`].

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
pub use zhang_shared::plugin_abi::{
    Encoding as BodyEncoding, LedgerInfo, MetaWrite, OpenWrite, PostingWrite, QueryParam as Param, Request, Response, TransactionWrite, UnitWrite, WriteBatch,
    WriteCommitted, WriteDirective, WriteKind, WritePreview, WritePreviewEntry,
};

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

/// The named parameters of a query, for [`query_with_params`]: `bind("from", Param::Date(Some("2024-01-01".into())))`
/// binds `:from`. A [`Param`] is typed, `Bool`, `Int`, `Decimal` (as text, e.g. `"12.50"`), `Str`, `Date` (as
/// `YYYY-MM-DD`) or `Set`, each `None` for NULL; the query is compiled against these types, so `:from` above
/// compares as a date. Positional parameters (`$1`) cannot be bound.
///
/// ```
/// use zhang_plugin_sdk::router::{Param, Params};
///
/// let params = Params::new()
///     .bind("from", Param::Date(Some("2024-01-01".into())))
///     .bind("re", Param::Str(None));
/// assert_eq!(params.len(), 2);
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct Params(BTreeMap<String, Param>);

impl Params {
    /// no parameters
    pub fn new() -> Params {
        Params::default()
    }

    /// these parameters with `:name` bound to `value`, replacing an earlier binding of the name
    pub fn bind(mut self, name: impl Into<String>, value: Param) -> Params {
        self.0.insert(name.into(), value);
        self
    }

    /// the parameter `:name`
    pub fn get(&self, name: &str) -> Option<&Param> {
        self.0.get(name)
    }

    /// how many parameters are bound
    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// the input of `zhang_query_params`, as the host reads it (`zhang_shared::plugin_abi::QueryWithParams`)
#[derive(Serialize)]
struct QueryInput<'a> {
    bql: &'a str,
    params: &'a Params,
}

/// Run a read-only BQL query with its named parameters bound, as [`query`] runs one: the query is compiled
/// against the parameters' types, and `:name` in its text is the value bound as `name`. Bind what a user typed
/// instead of writing it into the query. A parameter the query does not use is ignored. A parameter the query uses
/// but `params` does not bind, or a query that fails, is a [`HostErrorKind::Query`](crate::HostErrorKind::Query)
/// error with its line and column; a parameter that cannot be read as its type (`Date(Some("soon"))`) is one
/// without a position, e.g. `parameter :from must be a date string YYYY-MM-DD or null`.
///
/// ```no_run
/// use zhang_plugin_sdk::router::{query_with_params, Param, Params};
///
/// let since = "2024-01-01"; // what the user typed
/// let params = Params::new().bind("since", Param::Date(Some(since.to_owned())));
/// let result = query_with_params("SELECT account, sum(position) WHERE date >= :since GROUP BY account", &params)?;
/// # Ok::<(), zhang_plugin_sdk::HostError>(())
/// ```
pub fn query_with_params(bql: &str, params: &Params) -> Result<QueryResult, HostError> {
    let input =
        serde_json::to_vec(&QueryInput { bql, params }).map_err(|e| HostError::new(crate::HostErrorKind::Invalid, format!("cannot encode the query: {e}")))?;
    host_result(abi::QUERY_PARAMS, &abi::query_params(&input)?)
}

/// The tables, columns and functions a query can use, as `GET /api/query/schema` answers them:
/// `{"tables": [{"name", "description", "columns": [{"name", "type", "description"}]}], "columns": [...],
/// "functions": [{"name", "signature", "description", "aggregate"}], "keywords": [...]}`, where `columns` are
/// those of the default `postings` table. Kept as JSON, so a newer zhang can add to it.
pub fn schema() -> Result<serde_json::Value, HostError> {
    host_result(abi::QUERY_SCHEMA, &abi::query_schema()?)
}

/// the ledger's title, operating currency and timezone
pub fn ledger_info() -> Result<LedgerInfo, HostError> {
    host_result(abi::LEDGER_INFO, &abi::ledger_info()?)
}

/// the batch of `directives` as the host reads it, with `token` for a commit
fn batch(directives: &[WriteDirective], token: Option<&str>) -> Result<Vec<u8>, HostError> {
    let encode = |e: serde_json::Error| HostError::new(crate::HostErrorKind::Invalid, format!("cannot encode the batch: {e}"));
    let directives = directives.iter().map(serde_json::to_value).collect::<Result<Vec<_>, _>>().map_err(encode)?;
    serde_json::to_vec(&WriteBatch {
        directives,
        token: token.map(str::to_owned),
    })
    .map_err(encode)
}

/// Preview `directives` to append to the ledger: each one validated in order (an `open` earlier in the batch counts
/// for the transactions after it), its text as the ledger writes it, the file it goes to, and what the ledger would
/// report against it; `token` is set when every entry is clean. The plugin must declare `writes` in [`plugin!`] and
/// its directive must grant them, else [`HostErrorKind::Denied`](crate::HostErrorKind::Denied) (or
/// [`Unavailable`](crate::HostErrorKind::Unavailable) without the declaration). At most 50 directives, whose text is
/// at most 64 KiB.
///
/// ```no_run
/// use zhang_plugin_sdk::router::{self, OpenWrite, PostingWrite, TransactionWrite, UnitWrite, WriteDirective};
///
/// let batch = vec![
///     WriteDirective::Open(OpenWrite { date: "2026-10-13".into(), account: "Expenses:Coffee".into(), commodities: vec!["USD".into()], ..Default::default() }),
///     WriteDirective::Transaction(TransactionWrite {
///         datetime: "2026-10-13".into(),
///         payee: "Blue Bottle".into(),
///         narration: Some("coffee".into()),
///         postings: vec![
///             PostingWrite { account: "Expenses:Coffee".into(), unit: Some(UnitWrite::Amount { number: "4.50".into(), commodity: "USD".into() }), ..Default::default() },
///             PostingWrite { account: "Assets:Cash".into(), ..Default::default() },
///         ],
///         ..Default::default()
///     }),
/// ];
/// let preview = router::write_preview(&batch)?;
/// if let Some(token) = &preview.token {
///     let committed = router::write_commit(&batch, token)?;
///     assert_eq!(committed.entries, 2);
/// }
/// # Ok::<(), zhang_plugin_sdk::HostError>(())
/// ```
pub fn write_preview(directives: &[WriteDirective]) -> Result<WritePreview, HostError> {
    host_result(abi::WRITE_PREVIEW, &abi::write_preview(&batch(directives, None)?)?)
}

/// Append `directives`, previewed with [`write_preview`], with the `token` the preview answered: the text is in the
/// file when this returns, and the ledger reloads once the request is answered. A batch that differs from the
/// previewed one, a token of another ledger generation (a reload or a commit happened since), an expired token, or
/// a second commit before the ledger reloaded is a [`HostErrorKind::Conflict`](crate::HostErrorKind::Conflict):
/// preview again. A batch the ledger refuses is [`Invalid`](crate::HostErrorKind::Invalid), a file that cannot be
/// written [`Io`](crate::HostErrorKind::Io).
pub fn write_commit(directives: &[WriteDirective], token: &str) -> Result<WriteCommitted, HostError> {
    host_result(abi::WRITE_COMMIT, &abi::write_commit(&batch(directives, Some(token))?)?)
}

#[cfg(test)]
mod test {
    use serde_json::json;

    use super::{BodyEncoding, LedgerInfo, Param, Params, QueryInput, QueryResult, Request, Response};
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

    #[test]
    fn should_write_the_query_with_params_zhang_reads() {
        let params = Params::new()
            .bind("from", Param::Date(Some("2024-01-01".into())))
            .bind("re", Param::Str(None))
            .bind("min", Param::Decimal(Some("12.50".into())))
            .bind("n", Param::Int(Some(3)))
            .bind("on", Param::Bool(Some(true)))
            .bind("tags", Param::Set(Some(vec!["a".into()])))
            // a later binding of a name replaces the earlier one
            .bind("n", Param::Int(None));
        assert_eq!((params.len(), params.get("n")), (6, Some(&Param::Int(None))));
        assert_eq!(
            serde_json::to_value(QueryInput {
                bql: "SELECT date WHERE date >= :from",
                params: &params
            })
            .unwrap(),
            json!({
                "bql": "SELECT date WHERE date >= :from",
                "params": {
                    "from": {"type": "date", "value": "2024-01-01"},
                    "re": {"type": "str", "value": null},
                    "min": {"type": "decimal", "value": "12.50"},
                    "n": {"type": "int", "value": null},
                    "on": {"type": "bool", "value": true},
                    "tags": {"type": "set", "value": ["a"]},
                },
            })
        );
        assert!(Params::new().is_empty());
        assert_eq!(serde_json::to_value(Params::new()).unwrap(), json!({}));
        // natively the host functions are unavailable
        assert_eq!(super::query_with_params("SELECT 1", &params).unwrap_err().kind, HostErrorKind::Unavailable);
        assert_eq!(super::schema().unwrap_err().kind, HostErrorKind::Unavailable);
    }
}

#[cfg(test)]
mod write_test {
    use serde_json::json;

    use super::{batch, OpenWrite, PostingWrite, TransactionWrite, UnitWrite, WriteDirective, WritePreview};
    use crate::error::{host_result, HostErrorKind};

    #[test]
    fn should_write_the_batch_zhang_reads_with_amounts_as_strings() {
        let directives = vec![
            WriteDirective::Open(OpenWrite {
                date: "2026-10-13".into(),
                account: "Expenses:Coffee".into(),
                commodities: vec!["USD".into()],
                booking_method: Some("FIFO".into()),
                ..Default::default()
            }),
            WriteDirective::Transaction(TransactionWrite {
                datetime: "2026-10-13".into(),
                payee: "Blue Bottle".into(),
                narration: Some("coffee".into()),
                postings: vec![
                    PostingWrite {
                        account: "Expenses:Coffee".into(),
                        unit: Some(UnitWrite::Amount {
                            number: "4.50".into(),
                            commodity: "USD".into(),
                        }),
                        comment: Some("morning".into()),
                        ..Default::default()
                    },
                    PostingWrite {
                        account: "Assets:Cash".into(),
                        ..Default::default()
                    },
                ],
                tags: vec!["mcp".into()],
                ..Default::default()
            }),
        ];
        let encoded: serde_json::Value = serde_json::from_slice(&batch(&directives, Some("t0k")).unwrap()).unwrap();
        assert_eq!(
            encoded,
            json!({
                "directives": [
                    {"kind": "open", "date": "2026-10-13", "account": "Expenses:Coffee", "commodities": ["USD"], "booking_method": "FIFO", "metas": []},
                    {"kind": "transaction", "datetime": "2026-10-13", "payee": "Blue Bottle", "narration": "coffee",
                     "postings": [
                        {"account": "Expenses:Coffee", "unit": {"number": "4.50", "commodity": "USD"}, "comment": "morning"},
                        {"account": "Assets:Cash", "unit": null},
                     ],
                     "tags": ["mcp"], "links": [], "metas": []},
                ],
                "token": "t0k",
            })
        );
        let preview: serde_json::Value = serde_json::from_slice(&batch(&directives, None).unwrap()).unwrap();
        assert!(preview.get("token").is_none(), "a preview sends no token");
        // natively the host functions are unavailable
        assert_eq!(super::write_preview(&directives).unwrap_err().kind, HostErrorKind::Unavailable);
        assert_eq!(super::write_commit(&directives, "t0k").unwrap_err().kind, HostErrorKind::Unavailable);
    }

    #[test]
    fn should_read_a_preview_and_the_write_error_kinds() {
        let preview: WritePreview = host_result(
            "zhang_write_preview",
            br#"{"Ok": {"ok": false, "file": "data/2026/10.zhang", "generation": 17, "entries": [
                {"index": 0, "kind": "transaction", "file": "data/2026/10.zhang", "text": "2026-10-13 * \"x\"", "field_errors": [], "unbalanced": [],
                 "errors": [{"error_type": "AccountDoesNotExist", "metas": {"account_name": "Expenses:Coffee"}}]},
                {"index": 1, "kind": "open", "file": "data/2026/10.zhang", "text": null, "errors": [], "already_open": true}
            ], "token": null}}"#,
        )
        .unwrap();
        assert!(!preview.ok);
        assert_eq!((preview.generation, preview.token), (17, None));
        assert_eq!(preview.entries[0].errors[0]["error_type"], "AccountDoesNotExist");
        assert_eq!(preview.entries[1].already_open, Some(true));
        let conflict = host_result::<WritePreview>("zhang_write_commit", br#"{"Err": {"kind": "conflict", "message": "preview again"}}"#).unwrap_err();
        assert_eq!(conflict.kind, HostErrorKind::Conflict);
        assert_eq!(
            host_result::<WritePreview>("zhang_write_commit", br#"{"Err": {"kind": "io", "message": "disk full"}}"#)
                .unwrap_err()
                .kind,
            HostErrorKind::Io
        );
    }
}
