//! What a client reads of a transaction in the journal, and sends back in an update, is what is written: the
//! metadata in its order. An edit that sends back what it read leaves it as it was.

use axum::extract::{Path as UrlPath, State};
use axum::http::StatusCode;
use axum::Json;
use serde_json::{json, Value};
use zhang_server::request::JournalRequest;
use zhang_server::routes::transaction::{get_journals, update_single_transaction};
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

    /// an update of the only transaction
    async fn update(&self, request: Value) -> (StatusCode, Value) {
        let id = self.transaction().await["id"].as_str().unwrap().to_owned();
        let request = serde_json::from_value(request).expect("an update request");
        respond(update_single_transaction(State(self.ledger.clone()), reload(), UrlPath((id,)), IfMatch(None), Json(request)).await).await
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
