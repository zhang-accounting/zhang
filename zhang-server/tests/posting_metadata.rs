//! Posting-level metadata through the server, written against the API contract:
//!
//! - every posting of a journal response has `metas: [{key, value}]`, like the transaction;
//! - create and update requests take an optional `metas: [{key, value}]` per posting, which
//!   is written under its posting and reads back after a reload;
//! - a beancount ledger rejects a new posting metadata key beancount cannot read with a 400
//!   and writes nothing, a zhang ledger takes it;
//! - the query route reads posting metadata with `meta()`, like beanquery.
//!
//! Requests are built from JSON, as the frontend sends them, and responses are read as JSON.

use std::path::PathBuf;

use axum::extract::Path as UrlPath;
use axum::http::StatusCode;
use axum::Json;
use serde_json::{json, Value};
use zhang_core::ledger::Ledger;
use zhang_server::request::{CreateTransactionRequest, JournalRequest, QueryRequest};
use zhang_server::routes::query::run_query;
use zhang_server::routes::transaction::{create_new_transaction, get_journals, update_single_transaction};
use zhang_server::routes::Query as UrlQuery;
use zhang_testkit::http::{respond, states};

#[derive(Clone, Copy, Debug)]
enum Format {
    Zhang,
    Beancount,
}

/// A ledger directory, removed on drop. The API appends January 2024 transactions to `data/2024/01.zhang`
/// (`01.bean` in a beancount ledger), which the main file includes; the local file system data source only
/// appends to an existing file, so it is created empty.
struct Scratch {
    dir: zhang_testkit::ledger::Scratch,
    format: Format,
}

const OPENS: &str = "1970-01-01 commodity CNY\n1970-01-01 open Assets:Cash\n1970-01-01 open Expenses:Food\n1970-01-01 open Expenses:Tips\n";

impl Scratch {
    fn new(format: Format, extra: &str) -> Scratch {
        let (main, data) = (Scratch::main_name_of(format), Scratch::data_name_of(format));
        let content = format!("option \"operating_currency\" \"CNY\"\ninclude \"{data}\"\n{OPENS}{extra}");
        Scratch {
            dir: zhang_testkit::ledger::Scratch::with_files(main, &[(main, &content), (data, "")]),
            format,
        }
    }

    /// A ledger whose main file is exactly `content`.
    fn with_main(format: Format, content: &str) -> Scratch {
        Scratch {
            dir: zhang_testkit::ledger::Scratch::with_main(Scratch::main_name_of(format), content),
            format,
        }
    }

    fn main_name_of(format: Format) -> &'static str {
        match format {
            Format::Zhang => "main.zhang",
            Format::Beancount => "main.bean",
        }
    }

    fn data_name_of(format: Format) -> &'static str {
        match format {
            Format::Zhang => "data/2024/01.zhang",
            Format::Beancount => "data/2024/01.bean",
        }
    }

    fn main_file(&self) -> PathBuf {
        self.dir.main_file()
    }

    fn data_file(&self) -> PathBuf {
        self.dir.dir().join(Scratch::data_name_of(self.format))
    }

    fn written(&self) -> String {
        std::fs::read_to_string(self.data_file()).unwrap()
    }

    async fn load(&self) -> Ledger {
        let ledger = self
            .dir
            .ledger()
            .unwrap_or_else(|err| panic!("the {:?} ledger should load: {err}", self.format));
        let errors: Vec<String> = ledger.errors.iter().map(|it| format!("{:?}", it.error_type)).collect();
        assert!(errors.is_empty(), "the {:?} ledger has errors: {errors:?}", self.format);
        ledger
    }

    async fn create(&self, request: CreateTransactionRequest) -> (StatusCode, Value) {
        let (ledger, reload) = states(self.load().await);
        respond(create_new_transaction(ledger, reload, Json(request)).await).await
    }

    async fn update(&self, id: &str, request: CreateTransactionRequest) -> (StatusCode, Value) {
        let (ledger, reload) = states(self.load().await);
        respond(update_single_transaction(ledger, reload, UrlPath((id.to_owned(),)), Json(request)).await).await
    }

    /// The records of `GET /api/journals`.
    async fn journal(&self) -> Vec<Value> {
        let (ledger, _) = states(self.load().await);
        let request = JournalRequest {
            page: None,
            size: Some(1000),
            keyword: None,
            tags: None,
            links: None,
        };
        let (status, body) = respond(get_journals(ledger, UrlQuery(request)).await).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body["data"]["records"].as_array().expect("journal records").clone()
    }

    /// The only transaction of the journal.
    async fn transaction(&self) -> Value {
        let records: Vec<Value> = self.journal().await.into_iter().filter(|it| it["type"] == "Transaction").collect();
        assert_eq!(records.len(), 1, "{records:?}");
        records.into_iter().next().unwrap()
    }
}

/// A create (or update) request for `2024-01-15 "Cafe" "lunch"` with the transaction
/// metadata `note: txn` and the given postings, as JSON.
fn request(postings: Value) -> CreateTransactionRequest {
    serde_json::from_value(json!({
        "datetime": "2024-01-15T12:00:00Z",
        "payee": "Cafe",
        "flag": null,
        "narration": "lunch",
        "postings": postings,
        "metas": [{"key": "note", "value": "txn"}],
        "tags": [],
        "links": [],
    }))
    .expect("a valid create request")
}

fn unit(number: &str) -> Value {
    json!({"number": number, "commodity": "CNY"})
}

fn metas(pairs: &[(&str, &str)]) -> Value {
    Value::Array(pairs.iter().map(|(key, value)| json!({"key": key, "value": value})).collect())
}

type Pairs = Vec<(String, String)>;

fn owned(pairs: &[(&str, &str)]) -> Pairs {
    let mut pairs: Pairs = pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    pairs.sort();
    pairs
}

/// The `metas` of a transaction or posting object of a response, sorted.
fn metas_of(object: &Value) -> Pairs {
    let metas = object["metas"].as_array().unwrap_or_else(|| panic!("expected a `metas` array in {object}"));
    let mut pairs: Pairs = metas
        .iter()
        .map(|meta| (meta["key"].as_str().unwrap().to_owned(), meta["value"].as_str().unwrap().to_owned()))
        .collect();
    pairs.sort();
    pairs
}

/// The postings of a journal transaction as (account, metas).
fn postings_of(record: &Value) -> Vec<(String, Pairs)> {
    record["postings"]
        .as_array()
        .expect("postings")
        .iter()
        .map(|posting| (posting["account"].as_str().unwrap().to_owned(), metas_of(posting)))
        .collect()
}

fn expected_postings(postings: &[(&str, &[(&str, &str)])]) -> Vec<(String, Pairs)> {
    postings.iter().map(|(account, pairs)| (account.to_string(), owned(pairs))).collect()
}

/// The written transactions of `text`: per transaction, its metadata lines (2 spaces, before
/// the first posting) and its postings, each with the metadata lines under it (4 spaces).
/// Fails on any other layout.
#[derive(Debug, PartialEq)]
struct Written {
    meta: Vec<String>,
    postings: Vec<(String, Vec<String>)>,
}

fn written_transactions(text: &str) -> Vec<Written> {
    let mut written: Vec<Written> = vec![];
    for line in text.lines() {
        let indent = line.len() - line.trim_start_matches(' ').len();
        let content = line.trim_start().to_owned();
        if line.starts_with("2024-") {
            written.push(Written {
                meta: vec![],
                postings: vec![],
            });
        } else if line.trim().is_empty() || indent == 0 {
            continue;
        } else {
            let txn = written
                .last_mut()
                .unwrap_or_else(|| panic!("an indented line outside a transaction: {line:?}\n{text}"));
            let is_posting = ["Assets:", "Liabilities:", "Equity:", "Income:", "Expenses:"]
                .iter()
                .any(|it| content.starts_with(it));
            match (indent, is_posting) {
                (2, true) => txn.postings.push((content, vec![])),
                (2, false) => {
                    assert!(txn.postings.is_empty(), "transaction metadata {line:?} after a posting in:\n{text}");
                    txn.meta.push(content);
                }
                (4, false) => {
                    let posting = txn
                        .postings
                        .last_mut()
                        .unwrap_or_else(|| panic!("posting metadata {line:?} before any posting in:\n{text}"));
                    posting.1.push(content);
                }
                _ => panic!("unexpected line {line:?} in:\n{text}"),
            }
        }
    }
    for txn in &mut written {
        txn.meta.sort();
        for posting in &mut txn.postings {
            posting.1.sort();
        }
    }
    written
}

fn written(meta: &[&str], postings: &[(&str, &[&str])]) -> Written {
    let mut meta: Vec<String> = meta.iter().map(|it| it.to_string()).collect();
    meta.sort();
    Written {
        meta,
        postings: postings
            .iter()
            .map(|(posting, metas)| {
                let mut metas: Vec<String> = metas.iter().map(|it| it.to_string()).collect();
                metas.sort();
                (posting.to_string(), metas)
            })
            .collect(),
    }
}

fn lunch_postings() -> Value {
    json!([
        {"account": "Assets:Cash", "unit": unit("-5"), "metas": metas(&[("receipt", "r-1"), ("document", "a \"quoted\" \\ value")])},
        {"account": "Expenses:Food", "unit": unit("4"), "metas": metas(&[("category", "food")])},
        {"account": "Expenses:Tips", "unit": null, "metas": metas(&[("memo", "cash tip")])},
    ])
}

#[tokio::test]
async fn created_posting_metadata_is_written_under_its_posting_and_reads_back() {
    let scratch = Scratch::new(Format::Zhang, "");
    let (status, body) = scratch.create(request(lunch_postings())).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let text = scratch.written();
    assert_eq!(
        written_transactions(&text),
        vec![written(
            &["note: \"txn\""],
            &[
                ("Assets:Cash -5 CNY", &["receipt: \"r-1\"", "document: \"a \\\"quoted\\\" \\\\ value\""]),
                ("Expenses:Food 4 CNY", &["category: \"food\""]),
                ("Expenses:Tips", &["memo: \"cash tip\""]),
            ],
        )],
        "{text}"
    );

    let txn = scratch.transaction().await;
    assert_eq!(metas_of(&txn), owned(&[("note", "txn")]), "{txn}");
    assert_eq!(
        postings_of(&txn),
        expected_postings(&[
            ("Assets:Cash", &[("document", "a \"quoted\" \\ value"), ("receipt", "r-1")]),
            ("Expenses:Food", &[("category", "food")]),
            ("Expenses:Tips", &[("memo", "cash tip")]),
        ]),
        "{txn}"
    );
}

#[tokio::test]
async fn a_zhang_ledger_takes_posting_metadata_keys_beancount_cannot_read() {
    let scratch = Scratch::new(Format::Zhang, "");
    let postings = json!([
        {"account": "Assets:Cash", "unit": unit("-5"), "metas": metas(&[("Receipt", "1"), ("receipt no", "2"), ("x", "3")])},
        {"account": "Expenses:Food", "unit": unit("5")},
    ]);
    let (status, body) = scratch.create(request(postings)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let text = scratch.written();
    assert_eq!(
        written_transactions(&text),
        vec![written(
            &["note: \"txn\""],
            &[
                ("Assets:Cash -5 CNY", &["Receipt: \"1\"", "\"receipt no\": \"2\"", "x: \"3\""]),
                ("Expenses:Food 5 CNY", &[]),
            ],
        )],
        "{text}"
    );
    let txn = scratch.transaction().await;
    assert_eq!(
        postings_of(&txn),
        expected_postings(&[("Assets:Cash", &[("Receipt", "1"), ("receipt no", "2"), ("x", "3")]), ("Expenses:Food", &[]),]),
        "{txn}"
    );
}

#[tokio::test]
async fn a_create_request_without_posting_metas_still_works() {
    let scratch = Scratch::new(Format::Zhang, "");
    let postings = json!([
        {"account": "Assets:Cash", "unit": unit("-5")},
        {"account": "Expenses:Food", "unit": unit("5")},
    ]);
    let (status, body) = scratch.create(request(postings)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let text = scratch.written();
    assert_eq!(
        written_transactions(&text),
        vec![written(&["note: \"txn\""], &[("Assets:Cash -5 CNY", &[]), ("Expenses:Food 5 CNY", &[])])],
        "{text}"
    );
    let txn = scratch.transaction().await;
    assert_eq!(metas_of(&txn), owned(&[("note", "txn")]), "{txn}");
    assert_eq!(postings_of(&txn), expected_postings(&[("Assets:Cash", &[]), ("Expenses:Food", &[])]), "{txn}");
}

#[tokio::test]
async fn updating_a_transaction_rewrites_its_posting_metadata() {
    let scratch = Scratch::new(Format::Zhang, "");
    let (status, body) = scratch.create(request(lunch_postings())).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let id = scratch.transaction().await["id"].as_str().unwrap().to_owned();

    // the client sends back what it got, with one posting's metadata changed, one's
    // removed and one's added to
    let postings = json!([
        {"account": "Assets:Cash", "unit": unit("-5"), "metas": metas(&[("receipt", "r-2")])},
        {"account": "Expenses:Food", "unit": unit("4"), "metas": []},
        {"account": "Expenses:Tips", "unit": null, "metas": metas(&[("memo", "cash tip"), ("category", "tip")])},
    ]);
    let (status, body) = scratch.update(&id, request(postings)).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let text = scratch.written();
    assert_eq!(
        written_transactions(&text),
        vec![written(
            &["note: \"txn\""],
            &[
                ("Assets:Cash -5 CNY", &["receipt: \"r-2\""]),
                ("Expenses:Food 4 CNY", &[]),
                ("Expenses:Tips", &["category: \"tip\"", "memo: \"cash tip\""]),
            ],
        )],
        "{text}"
    );
    let txn = scratch.transaction().await;
    assert_eq!(metas_of(&txn), owned(&[("note", "txn")]), "{txn}");
    assert_eq!(
        postings_of(&txn),
        expected_postings(&[
            ("Assets:Cash", &[("receipt", "r-2")]),
            ("Expenses:Food", &[]),
            ("Expenses:Tips", &[("category", "tip"), ("memo", "cash tip")]),
        ]),
        "{txn}"
    );

    // an update without posting metadata clears it
    let postings = json!([
        {"account": "Assets:Cash", "unit": unit("-5")},
        {"account": "Expenses:Food", "unit": unit("5")},
    ]);
    let (status, body) = scratch.update(&id, request(postings)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let txn = scratch.transaction().await;
    assert_eq!(postings_of(&txn), expected_postings(&[("Assets:Cash", &[]), ("Expenses:Food", &[])]), "{txn}");
}

#[tokio::test]
async fn every_journal_posting_has_its_own_metas() {
    let scratch = Scratch::new(
        Format::Zhang,
        "\n2024-01-02 * \"Shop\" \"lunch\"\n  note: \"t\"\n  Assets:Cash -10 CNY\n    receipt: \"r-1\"\n  Expenses:Food 10 CNY\n  legacy: \"after\"\n\n\
         2024-01-03 balance Assets:Cash -10 CNY\n",
    );
    let records = scratch.journal().await;
    assert_eq!(records.len(), 2, "{records:?}");
    for record in &records {
        for posting in record["postings"].as_array().expect("postings") {
            assert!(posting["metas"].is_array(), "every posting has `metas`: {record}");
        }
    }
    let txn = scratch.transaction().await;
    assert_eq!(metas_of(&txn), owned(&[("legacy", "after"), ("note", "t")]), "{txn}");
    assert_eq!(
        postings_of(&txn),
        expected_postings(&[("Assets:Cash", &[("receipt", "r-1")]), ("Expenses:Food", &[])]),
        "{txn}"
    );
}

#[tokio::test]
async fn a_beancount_ledger_rejects_a_new_posting_metadata_key_beancount_cannot_read() {
    // beancount 3.2.3 keys: `[a-z][a-zA-Z0-9\-_]+`
    for key in ["Receipt", "receipt no", "x"] {
        let scratch = Scratch::new(Format::Beancount, "");
        let main = std::fs::read_to_string(scratch.main_file()).unwrap();
        let postings = json!([
            {"account": "Assets:Cash", "unit": unit("-5"), "metas": metas(&[(key, "1")])},
            {"account": "Expenses:Food", "unit": unit("5")},
        ]);
        let (status, body) = scratch.create(request(postings)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "posting metadata key {key:?}: {body}");
        let message = body["message"].as_str().unwrap_or_default();
        assert!(message.contains(key), "the message names {key:?}: {body}");
        assert_eq!(scratch.written(), "", "nothing is written for {key:?}");
        assert_eq!(std::fs::read_to_string(scratch.main_file()).unwrap(), main, "nothing is written for {key:?}");
    }

    // an update is checked the same way
    let scratch = Scratch::new(Format::Beancount, "");
    let (status, body) = scratch
        .create(request(json!([
            {"account": "Assets:Cash", "unit": unit("-5")},
            {"account": "Expenses:Food", "unit": unit("5")},
        ])))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let before = scratch.written();
    let id = scratch.transaction().await["id"].as_str().unwrap().to_owned();
    let (status, body) = scratch
        .update(
            &id,
            request(json!([
                {"account": "Assets:Cash", "unit": unit("-5"), "metas": metas(&[("Receipt", "1")])},
                {"account": "Expenses:Food", "unit": unit("5")},
            ])),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(scratch.written(), before);
}

#[tokio::test]
async fn a_beancount_ledger_writes_posting_metadata_under_its_posting() {
    let scratch = Scratch::new(Format::Beancount, "");
    let postings = json!([
        {"account": "Assets:Cash", "unit": unit("-5"), "metas": metas(&[("receipt", "r-1")])},
        {"account": "Expenses:Food", "unit": unit("5"), "metas": metas(&[("category", "food"), ("note", "posting")])},
    ]);
    let (status, body) = scratch.create(request(postings)).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // the beancount exporter also writes the time of day as transaction metadata
    let text = scratch.written();
    let mut transactions = written_transactions(&text);
    assert_eq!(transactions.len(), 1, "{text}");
    let txn = transactions.remove(0);
    assert!(txn.meta.contains(&"note: \"txn\"".to_owned()), "{text}");
    assert!(txn.meta.iter().all(|it| it.starts_with("note:") || it.starts_with("time:")), "{text}");
    assert_eq!(
        txn.postings,
        written(
            &[],
            &[
                ("Assets:Cash -5 CNY", &["receipt: \"r-1\""]),
                ("Expenses:Food 5 CNY", &["category: \"food\"", "note: \"posting\""]),
            ]
        )
        .postings,
        "{text}"
    );

    let txn = scratch.transaction().await;
    assert_eq!(metas_of(&txn), owned(&[("note", "txn")]), "{txn}");
    assert_eq!(
        postings_of(&txn),
        expected_postings(&[
            ("Assets:Cash", &[("receipt", "r-1")]),
            ("Expenses:Food", &[("category", "food"), ("note", "posting")]),
        ]),
        "{txn}"
    );
}

/// A beancount ledger with posting metadata at the postings' indentation, deeper, after an
/// indented comment, on a posting without an amount, plus `pushmeta`.
const BEAN_LEDGER: &str = r#"1970-01-01 commodity USD
1970-01-01 open Assets:Cash
1970-01-01 open Expenses:Food
1970-01-01 open Expenses:Drink

pushmeta source: "import"
2024-01-02 * "Cafe" "lunch"
  category: "meal"
  Assets:Cash -10 USD
  receipt: "r-1"
  ; checked by hand
  Expenses:Food 7 USD
  category: "food"
  Expenses:Drink 3 USD
    trip: "rome"
popmeta source:

2024-01-03 * "Shop" "snack"
    trip: "home"
  Assets:Cash -2 USD
  Expenses:Food
  receipt: "r-2"
"#;

#[tokio::test]
async fn a_beancount_ledger_file_attaches_metadata_like_beancount() {
    let scratch = Scratch::with_main(Format::Beancount, BEAN_LEDGER);
    let records = scratch.journal().await;
    let find = |narration: &str| {
        records
            .iter()
            .find(|it| it["narration"] == narration)
            .unwrap_or_else(|| panic!("no {narration} in {records:?}"))
            .clone()
    };
    // produced by beancount 3.2.3
    let lunch = find("lunch");
    assert_eq!(metas_of(&lunch), owned(&[("category", "meal"), ("source", "import")]), "{lunch}");
    assert_eq!(
        postings_of(&lunch),
        expected_postings(&[
            ("Assets:Cash", &[("receipt", "r-1")]),
            ("Expenses:Food", &[("category", "food")]),
            ("Expenses:Drink", &[("trip", "rome")]),
        ]),
        "{lunch}"
    );
    // produced by beancount 3.2.3
    let snack = find("snack");
    assert_eq!(metas_of(&snack), owned(&[("trip", "home")]), "{snack}");
    assert_eq!(
        postings_of(&snack),
        expected_postings(&[("Assets:Cash", &[]), ("Expenses:Food", &[("receipt", "r-2")])]),
        "{snack}"
    );
}

/// The rows of `POST /api/query`, with `NULL` for a null cell.
async fn query(scratch: &Scratch, sql: &str) -> Vec<Vec<String>> {
    let (ledger, _) = states(scratch.load().await);
    let (status, body) = respond(
        run_query(
            ledger,
            Json(QueryRequest {
                query: sql.to_owned(),
                count_total: None,
            }),
        )
        .await,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{sql}: {body}");
    body["data"]["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            row.as_array()
                .unwrap()
                .iter()
                .map(|cell| match cell {
                    Value::Null => "NULL".to_owned(),
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                })
                .collect()
        })
        .collect()
}

/// One row per line, the cells separated by `|`.
fn table(rows: &str) -> Vec<Vec<String>> {
    rows.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.split('|').map(|cell| cell.trim().to_owned()).collect())
        .collect()
}

#[tokio::test]
async fn the_query_route_reads_posting_metadata_like_beanquery() {
    let scratch = Scratch::with_main(Format::Beancount, BEAN_LEDGER);
    // produced by beanquery 0.2.0 on the same ledger
    let sql = "SELECT date, account, meta('category'), entry_meta('category'), any_meta('category'), \
               meta('receipt'), entry_meta('receipt'), any_meta('receipt'), meta('trip'), entry_meta('trip'), any_meta('trip'), \
               meta('source'), entry_meta('source'), any_meta('source') ORDER BY date, account";
    assert_eq!(
        query(&scratch, sql).await,
        table(
            r#"
            2024-01-02 | Assets:Cash    | NULL | meal | meal | r-1  | NULL | r-1  | NULL | NULL | NULL | NULL | import | import
            2024-01-02 | Expenses:Drink | NULL | meal | meal | NULL | NULL | NULL | rome | NULL | rome | NULL | import | import
            2024-01-02 | Expenses:Food  | food | meal | food | NULL | NULL | NULL | NULL | NULL | NULL | NULL | import | import
            2024-01-03 | Assets:Cash    | NULL | NULL | NULL | NULL | NULL | NULL | NULL | home | home | NULL | NULL   | NULL
            2024-01-03 | Expenses:Food  | NULL | NULL | NULL | r-2  | NULL | r-2  | NULL | home | home | NULL | NULL   | NULL
            "#
        ),
        "{sql}"
    );
    // produced by beanquery 0.2.0 on the same ledger
    let sql = "SELECT account, any_meta('category') WHERE entry_meta('category') = 'meal' ORDER BY account";
    assert_eq!(
        query(&scratch, sql).await,
        table("Assets:Cash | meal\nExpenses:Drink | meal\nExpenses:Food | food"),
        "{sql}"
    );
    // produced by beanquery 0.2.0 on the same ledger
    let sql = "SELECT date, account WHERE meta('category') = 'food'";
    assert_eq!(query(&scratch, sql).await, table("2024-01-02 | Expenses:Food"), "{sql}");
}
