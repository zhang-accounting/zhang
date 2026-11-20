//! What a client reads of a transaction in the journal, and sends back in an update, is what is written: the flag of
//! each posting, and the metadata in its order. An edit that sends back what it read leaves these as they were.

use axum::extract::{Path as UrlPath, State};
use axum::http::StatusCode;
use axum::Json;
use serde_json::{json, Value};
use zhang_server::request::JournalRequest;
use zhang_server::routes::transaction::{create_new_transaction, get_journals, preview_transaction_update, update_single_transaction};
use zhang_server::routes::{IfMatch, Query as UrlQuery};
use zhang_server::state::SharedLedger;
use zhang_testkit::http::{reload, respond, shared};
use zhang_testkit::ledger::Scratch;

const OPENS: &str = "option \"operating_currency\" \"CNY\"\n1970-01-01 commodity CNY\n1970-01-01 open Assets:Cash\n1970-01-01 open Expenses:Food\n\n";

/// A ledger in a scratch directory, served without a reload of the test's own.
struct Served {
    scratch: Scratch,
    ledger: SharedLedger,
    main: &'static str,
}

impl Served {
    fn new(main: &'static str, content: &str) -> Served {
        let scratch = Scratch::with_main(main, content);
        let ledger = shared(scratch.ledger().unwrap_or_else(|error| panic!("{main} loads: {error}")));
        Served { scratch, ledger, main }
    }

    /// the only transaction of the journal
    async fn transaction(&self) -> Value {
        let request = JournalRequest {
            page: None,
            size: Some(1000),
            keyword: None,
            tags: None,
            links: None,
        };
        let (status, body) = respond(get_journals(State(self.ledger.clone()), UrlQuery(request)).await).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let records = body["data"]["records"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|it| it["type"] == "Transaction")
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(records.len(), 1, "{}: {body}", self.main);
        records[0].clone()
    }

    async fn create(&self, request: Value) -> (StatusCode, Value) {
        let request = serde_json::from_value(request).expect("a create request");
        respond(create_new_transaction(State(self.ledger.clone()), reload(), Json(request)).await).await
    }

    /// an update of the only transaction
    async fn update(&self, request: Value) -> (StatusCode, Value) {
        let id = self.transaction().await["id"].as_str().unwrap().to_owned();
        let request = serde_json::from_value(request).expect("an update request");
        respond(update_single_transaction(State(self.ledger.clone()), reload(), UrlPath((id,)), IfMatch(None), Json(request)).await).await
    }

    /// the preview of an update of the only transaction
    async fn preview(&self, request: Value) -> Value {
        let id = self.transaction().await["id"].as_str().unwrap().to_owned();
        let request = serde_json::from_value(request).expect("an update request");
        let (status, body) =
            respond(preview_transaction_update(State(self.ledger.clone()), reload(), UrlPath((id,)), IfMatch(None), Json(request)).await).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body["data"].clone()
    }

    fn written(&self) -> String {
        self.scratch.read(self.main)
    }

    /// the files load again without errors
    fn reloads(&self) {
        let ledger = self.scratch.ledger().expect("the files load");
        assert!(ledger.errors.is_empty(), "{}: {:?}\n{}", self.main, ledger.errors, self.written());
    }
}

/// An update or create request with `postings`, on `datetime`.
fn request(datetime: &str, postings: Value, metas: Value) -> Value {
    json!({
        "datetime": datetime,
        "payee": "Cafe",
        "narration": "coffee",
        "postings": postings,
        "metas": metas,
        "tags": [],
        "links": [],
    })
}

/// The journal gives the flag of each posting with the rest of its line, `written.flag`; an update keeps the flag of a
/// posting when the request leaves the field out, removes it with `null` and sets one given.
#[tokio::test]
async fn posting_flags_are_read_kept_removed_and_set() {
    for main in ["main.zhang", "main.bean"] {
        let ledger = format!("{OPENS}2024-01-15 * \"Cafe\" \"coffee\"\n  ! Assets:Cash -5 CNY\n  & Expenses:Food\n");
        let served = Served::new(main, &ledger);
        let transaction = served.transaction().await;
        let flags = |transaction: &Value| {
            transaction["postings"]
                .as_array()
                .unwrap()
                .iter()
                .map(|it| it["written"]["flag"].clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(flags(&transaction), vec![json!("!"), json!("&")], "{main}");

        // left out: kept
        let postings = json!([{"account": "Assets:Cash", "unit": "-6 CNY"}, {"account": "Expenses:Food", "unit": null}]);
        let (status, body) = served.update(request("2024-01-15T12:00:00", postings, json!([]))).await;
        assert_eq!(status, StatusCode::OK, "{main}: {body}");
        assert!(
            served.written().contains("\n  ! Assets:Cash -6 CNY\n  & Expenses:Food"),
            "{main}: {}",
            served.written()
        );

        // `null` removes it, a flag sets one
        let postings = json!([
            {"account": "Assets:Cash", "unit": "-6 CNY", "flag": null},
            {"account": "Expenses:Food", "unit": null, "flag": "?"},
        ]);
        let (status, body) = served.update(request("2024-01-15T12:00:00", postings, json!([]))).await;
        assert_eq!(status, StatusCode::OK, "{main}: {body}");
        assert!(
            served.written().contains("\n  Assets:Cash -6 CNY\n  ? Expenses:Food"),
            "{main}: {}",
            served.written()
        );
        assert_eq!(flags(&served.transaction().await), vec![Value::Null, json!("?")], "{main}");
        served.reloads();
    }
}

/// A posting flag the ledger would not read back is refused as a field of the posting: `#` starts a comment in a zhang
/// ledger, and is a flag in a beancount one; a flag is one character.
#[tokio::test]
async fn a_posting_flag_the_ledger_would_not_read_back_is_refused() {
    for (main, hash) in [("main.zhang", false), ("main.bean", true)] {
        let ledger = format!("{OPENS}2024-01-15 * \"Cafe\" \"coffee\"\n  Assets:Cash -5 CNY\n  Expenses:Food\n");
        let served = Served::new(main, &ledger);
        for (flag, readable) in [("#", hash), ("!!", false), ("x", false), ("", false), ("A", true)] {
            let postings = json!([{"account": "Assets:Cash", "unit": "-5 CNY"}, {"account": "Expenses:Food", "unit": null, "flag": flag}]);
            let preview = served.preview(request("2024-01-15T12:00:00", postings, json!([]))).await;
            if readable {
                assert_eq!(preview["field_errors"], json!([]), "{main} {flag:?}: {preview}");
                continue;
            }
            let error = &preview["field_errors"][0];
            assert_eq!(
                (&error["posting"], &error["field"], &error["kind"], &error["value"]),
                (&json!(1), &json!("flag"), &json!("invalid_flag"), &json!(flag)),
                "{main} {flag:?}: {preview}"
            );
        }
        // a create takes a flag too, and answers the first error of a field
        let postings = json!([{"account": "Assets:Cash", "unit": "-5 CNY", "flag": "!"}, {"account": "Expenses:Food", "unit": null, "flag": "!!"}]);
        let (status, body) = served.create(request("2024-01-16T12:00:00", postings, json!([]))).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{main}: {body}");
        assert!(body["message"].as_str().unwrap().starts_with("invalid flag \"!!\""), "{main}: {body}");
    }
}

/// Metadata reads back in the order it is written, of the transaction and of each posting, and is written in the order
/// a request gives it: the values of a repeated key together, after the first of them.
#[tokio::test]
async fn metadata_keeps_its_order() {
    for main in ["main.zhang", "main.bean"] {
        let ledger = format!(
            "{OPENS}2024-01-15 * \"Cafe\" \"coffee\"\n  zeta: \"1\"\n  alpha: \"2\"\n  mid: \"3\"\n  Assets:Cash -5 CNY\n    \
             receipt: \"r\"\n    beta: \"b\"\n  Expenses:Food\n"
        );
        let served = Served::new(main, &ledger);
        let pairs = |metas: &Value| {
            metas
                .as_array()
                .unwrap()
                .iter()
                .map(|it| format!("{}={}", it["key"].as_str().unwrap(), it["value"].as_str().unwrap()))
                .collect::<Vec<_>>()
        };
        let transaction = served.transaction().await;
        assert_eq!(pairs(&transaction["metas"]), ["zeta=1", "alpha=2", "mid=3"], "{main}");
        assert_eq!(pairs(&transaction["postings"][0]["metas"]), ["receipt=r", "beta=b"], "{main}");

        // sent back in another order, with a repeated key
        let metas = json!([{"key": "mid", "value": "3"}, {"key": "zeta", "value": "1"}, {"key": "tag", "value": "x"}, {"key": "zeta", "value": "9"}]);
        let postings = json!([
            {"account": "Assets:Cash", "unit": "-5 CNY", "metas": [{"key": "beta", "value": "b"}, {"key": "receipt", "value": "r"}]},
            {"account": "Expenses:Food", "unit": null},
        ]);
        let (status, body) = served.update(request("2024-01-15T12:00:00", postings, metas)).await;
        assert_eq!(status, StatusCode::OK, "{main}: {body}");
        let written = served.written();
        assert!(
            written.contains("  mid: \"3\"\n  zeta: \"1\"\n  zeta: \"9\"\n  tag: \"x\"\n"),
            "{main}: {written}"
        );
        assert!(written.contains("\n    beta: \"b\"\n    receipt: \"r\"\n"), "{main}: {written}");
        let transaction = served.transaction().await;
        assert_eq!(pairs(&transaction["metas"]), ["mid=3", "zeta=1", "zeta=9", "tag=x"], "{main}");
        assert_eq!(pairs(&transaction["postings"][0]["metas"]), ["beta=b", "receipt=r"], "{main}");
        served.reloads();
    }
}
