//! `PUT /api/transactions`: what it leaves in the files.

use axum::extract::{Path as UrlPath, State};
use axum::http::StatusCode;
use axum::Json;
use serde_json::{json, Value};
use zhang_server::request::JournalRequest;
use zhang_server::routes::transaction::{get_journals, preview_transaction_update, update_single_transaction};
use zhang_server::routes::Query as UrlQuery;
use zhang_server::state::SharedLedger;
use zhang_testkit::http::{reload, respond, shared};
use zhang_testkit::ledger::Scratch;

/// The opens of the ledgers, in either format.
const OPENS: &str = "option \"operating_currency\" \"CNY\"\n1970-01-01 commodity CNY\n1970-01-01 open Assets:Cash\n1970-01-01 open Expenses:Food\n";

/// A ledger in a scratch directory, served: the handlers are called with the one shared ledger, which the test never
/// reloads itself.
struct Served {
    scratch: Scratch,
    ledger: SharedLedger,
}

impl Served {
    fn new(scratch: Scratch) -> Served {
        let ledger = shared(scratch.ledger().unwrap_or_else(|error| panic!("the ledger should load: {error}")));
        Served { scratch, ledger }
    }

    /// `main.zhang` or `main.bean` holding `content`
    fn with_main(main: &str, content: &str) -> Served {
        Served::new(Scratch::with_main(main, content))
    }

    async fn update(&self, id: &str, request: Value) -> (StatusCode, Value) {
        let request = serde_json::from_value(request).expect("an update request");
        respond(update_single_transaction(State(self.ledger.clone()), reload(), UrlPath((id.to_owned(),)), Json(request)).await).await
    }

    /// The records of `GET /api/journals` on the ledger served.
    async fn journal(&self) -> Vec<Value> {
        let request = JournalRequest {
            page: None,
            size: Some(1000),
            keyword: None,
            tags: None,
            links: None,
        };
        let (status, body) = respond(get_journals(State(self.ledger.clone()), UrlQuery(request)).await).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body["data"]["records"].as_array().expect("journal records").clone()
    }

    fn read(&self, relative: &str) -> String {
        self.scratch.read(relative)
    }
}

/// A create or update request for a meal at noon on `day` of January 2024.
fn meal(day: u32, payee: &str, narration: &str, amount: &str) -> Value {
    json!({
        "datetime": format!("2024-01-{day:02}T12:00:00"),
        "payee": payee,
        "narration": narration,
        "postings": [
            {"account": "Assets:Cash", "unit": format!("-{amount} CNY")},
            {"account": "Expenses:Food", "unit": null},
        ],
        "metas": [],
        "tags": [],
        "links": [],
    })
}

/// the `message` of an error body
fn message(body: &Value) -> &str {
    body["message"].as_str().unwrap_or_else(|| panic!("an error body: {body}"))
}

/// The padding a `pad`, or a `balance` with a pad, books is at the place of that directive, whose text is no
/// transaction: it is not replaced as one, which would drop the directive (an update replaced a `balance` with a pad
/// by the transaction it was sent, and the assertion was gone).
#[tokio::test]
async fn the_padding_of_a_pad_is_not_replaced() {
    let ledgers = [
        (
            "main.zhang",
            "1970-01-01 open Equity:Open\n2024-01-05 balance Assets:Cash 100 CNY with pad Equity:Open\n",
        ),
        (
            "main.bean",
            "1970-01-01 open Equity:Open\n2024-01-04 pad Assets:Cash Equity:Open\n2024-01-05 balance Assets:Cash 100 CNY\n",
        ),
    ];
    for (main, pads) in ledgers {
        let text = format!("{OPENS}{pads}");
        let served = Served::with_main(main, &text);
        let padding = served.journal().await.into_iter().find(|it| it["type"] == "BalancePad");
        let padding = padding.unwrap_or_else(|| panic!("{main}: the journal lists the padding"))["id"]
            .as_str()
            .unwrap()
            .to_owned();

        let (status, body) = served.update(&padding, meal(5, "Bank", "padding", "1")).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{main}: {body}");
        assert!(message(&body).contains("is the padding booked by the"), "{main}: {body}");
        // and the preview of the update says so
        let request = serde_json::from_value(meal(5, "Bank", "padding", "1")).unwrap();
        let (status, body) = respond(preview_transaction_update(State(served.ledger.clone()), UrlPath((padding.clone(),)), Json(request)).await).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{main}: {body}");
        assert_eq!(served.read(main), text, "{main}: nothing is written");
    }
}
