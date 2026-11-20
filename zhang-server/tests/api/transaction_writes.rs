//! `POST` and `PUT /api/transactions`: what they leave in the files, and the ids they answer with. The ledger is
//! reloaded before the answer, so the next request, sent to the same server without a reload of its own, finds the
//! transaction under the id answered.

use axum::extract::{Path as UrlPath, State};
use axum::http::StatusCode;
use axum::Json;
use serde_json::{json, Value};
use zhang_server::request::JournalRequest;
use zhang_server::routes::transaction::{create_new_transaction, get_journals, preview_transaction_update, update_single_transaction};
use zhang_server::routes::Query as UrlQuery;
use zhang_server::state::SharedLedger;
use zhang_testkit::http::{reload, respond, shared};
use zhang_testkit::ledger::Scratch;

/// The opens of the ledgers, in either format.
const OPENS: &str = "option \"operating_currency\" \"CNY\"\n1970-01-01 commodity CNY\n1970-01-01 open Assets:Cash\n1970-01-01 open Expenses:Food\n";

/// Three transactions with the comments around them, in either format.
const MEALS: &str = "\n; breakfast, paid in cash\n\
    2024-01-01 * \"Bakery\" \"bread\"\n  Assets:Cash -3 CNY\n  Expenses:Food\n\n\
    2024-01-02 * \"Cafe\" \"coffee\"\n  ; an indented comment of the coffee\n  Assets:Cash -5 CNY\n  Expenses:Food\n\n\
    ; lunch\n\
    2024-01-03 * \"Noodles\" \"lunch\"\n  Assets:Cash -12 CNY\n  Expenses:Food\n";

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

    async fn create(&self, request: Value) -> (StatusCode, Value) {
        let request = serde_json::from_value(request).expect("a create request");
        respond(create_new_transaction(State(self.ledger.clone()), reload(), Json(request)).await).await
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

    /// The `(id, payee, narration)` of the transactions of the journal, oldest first.
    async fn transactions(&self) -> Vec<(String, String, String)> {
        let records = self.journal().await.into_iter().filter(|it| it["type"] == "Transaction").rev();
        let text = |value: &Value| value.as_str().unwrap_or_default().to_owned();
        records.map(|it| (text(&it["id"]), text(&it["payee"]), text(&it["narration"]))).collect()
    }

    /// The id the journal lists the transaction of `payee` under.
    async fn id_of(&self, payee: &str) -> String {
        let transactions = self.transactions().await;
        let found = transactions.into_iter().find(|(_, it, _)| it == payee);
        found.unwrap_or_else(|| panic!("the journal lists {payee}")).0
    }

    fn read(&self, relative: &str) -> String {
        self.scratch.read(relative)
    }

    /// the ledger loaded from the files anew has no errors
    fn reloads_without_errors(&self) {
        let ledger = self.scratch.ledger().expect("the files load");
        let errors = ledger.errors.iter().map(|it| format!("{:?}", it.error_type)).collect::<Vec<_>>();
        assert!(errors.is_empty(), "{errors:?}");
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

#[tokio::test]
async fn the_id_a_create_answers_is_the_journals_and_takes_an_update_right_away() {
    for main in ["main.zhang", "main.bean"] {
        // the transaction goes to a file the ledger does not read yet, which the main file then includes
        let served = Served::with_main(main, &format!("{OPENS}{MEALS}"));
        let (status, body) = served.create(meal(15, "Deli", "sandwich", "8")).await;
        assert_eq!(status, StatusCode::OK, "{main}: {body}");
        let id = body["data"]["id"].as_str().unwrap_or_else(|| panic!("{main}: an id in {body}")).to_owned();
        assert!(served.read(main).contains("include \"data/2024/01."), "{main}");
        assert!(
            served.transactions().await.contains(&(id.clone(), "Deli".to_owned(), "sandwich".to_owned())),
            "{main}: the journal lists the transaction created under its id"
        );

        let (status, body) = served.update(&id, meal(15, "Deli", "soup", "6")).await;
        assert_eq!(
            (status, &body),
            (StatusCode::OK, &json!({"data": {"id": id}})),
            "{main}: an update keeps the id"
        );
        assert!(
            served.transactions().await.contains(&(id.clone(), "Deli".to_owned(), "soup".to_owned())),
            "{main}: the journal lists the update under the id"
        );

        served.reloads_without_errors();
    }
}

/// The ids of an update and of a create of a transaction that is in a file already, before others: an edit of an
/// earlier transaction that changes its length moves the ones after it in its file, which take other ids, and the
/// answer gives the edited transaction's own, unchanged.
#[tokio::test]
async fn an_update_answers_the_id_it_kept_and_a_create_the_one_after_the_others() {
    let served = Served::with_main("main.zhang", &format!("{OPENS}{MEALS}"));
    let (bakery, noodles) = (served.id_of("Bakery").await, served.id_of("Noodles").await);
    let (status, body) = served
        .update(&bakery, meal(1, "Bakery", "bread and a croissant, and a long narration", "9"))
        .await;
    assert_eq!((status, &body), (StatusCode::OK, &json!({"data": {"id": bakery}})));
    assert_eq!(served.id_of("Bakery").await, bakery);
    assert_ne!(served.id_of("Noodles").await, noodles, "the transactions after it moved");

    // a create into the main file, as the option says, after everything in it
    let served = Served::with_main("main.zhang", &format!("option \"directive_output_path\" \"main.zhang\"\n{OPENS}{MEALS}"));
    let (status, body) = served.create(meal(2, "Cafe", "coffee", "5")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let ids = served.transactions().await;
    let cafes = ids.iter().filter(|(_, payee, _)| payee == "Cafe").collect::<Vec<_>>();
    assert_eq!(cafes.len(), 2, "the same text twice: {ids:?}");
    assert!(served
        .read("main.zhang")
        .ends_with("  Expenses:Food\n\n2024-01-02 12:00:00 * \"Cafe\" \"coffee\"\n  Assets:Cash -5 CNY\n  Expenses:Food\n"));
    // the one created, not the other with the same payee
    let created = body["data"]["id"].as_str().unwrap();
    assert!(
        cafes.iter().any(|(id, ..)| id == created) && cafes.iter().any(|(id, ..)| id != created),
        "{ids:?}"
    );
}

/// A transaction the load rejects, as one with two postings without an amount, is written, but in no journal: the
/// create answers no id.
#[tokio::test]
async fn a_create_the_load_rejects_answers_no_id() {
    let served = Served::with_main("main.zhang", OPENS);
    let mut request = meal(15, "Deli", "sandwich", "8");
    request["postings"][0]["unit"] = Value::Null;
    let (status, body) = served.create(request).await;
    assert_eq!((status, &body), (StatusCode::OK, &json!({"data": {"id": null}})));
    assert!(served.read("data/2024/01.zhang").contains("\"Deli\" \"sandwich\""));
    assert!(served.transactions().await.is_empty());
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
