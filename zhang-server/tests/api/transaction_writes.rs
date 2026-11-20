//! `POST`, `PUT` and `DELETE /api/transactions`: what they leave in the files, and the ids they answer with. The
//! ledger is reloaded before the answer, so the next request, sent to the same server without a reload of its own,
//! finds the transaction under the id answered, or no longer finds the one deleted.

use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Path as UrlPath, State};
use axum::http::{Method, Request, StatusCode};
use axum::Json;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;
use serde_json::{json, Value};
use zhang_server::request::JournalRequest;
use zhang_server::routes::transaction::{
    create_new_transaction, delete_single_transaction, get_journals, preview_transaction_update, update_single_transaction,
};
use zhang_server::routes::Query as UrlQuery;
use zhang_server::state::SharedLedger;
use zhang_testkit::fixtures::data_source_for;
use zhang_testkit::http::{call, get, reload, respond, router, send, shared, RootedFileSystem, Settings};
use zhang_testkit::ledger::Scratch;

/// The opens of the ledgers, in either format.
const OPENS: &str = "option \"operating_currency\" \"CNY\"\n1970-01-01 commodity CNY\n1970-01-01 open Assets:Cash\n1970-01-01 open Expenses:Food\n";

/// Three transactions with the comments around them, in either format.
const MEALS: &str = "\n; breakfast, paid in cash\n\
    2024-01-01 * \"Bakery\" \"bread\"\n  Assets:Cash -3 CNY\n  Expenses:Food\n\n\
    2024-01-02 * \"Cafe\" \"coffee\"\n  ; an indented comment of the coffee\n  Assets:Cash -5 CNY\n  Expenses:Food\n\n\
    ; lunch\n\
    2024-01-03 * \"Noodles\" \"lunch\"\n  Assets:Cash -12 CNY\n  Expenses:Food\n";

/// The text of the coffee in [`MEALS`], with the blank line after it.
const COFFEE: &str = "2024-01-02 * \"Cafe\" \"coffee\"\n  ; an indented comment of the coffee\n  Assets:Cash -5 CNY\n  Expenses:Food\n\n";

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

    async fn delete(&self, id: &str) -> (StatusCode, Value) {
        respond(delete_single_transaction(State(self.ledger.clone()), reload(), UrlPath((id.to_owned(),))).await).await
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
async fn a_deleted_transaction_leaves_its_file_and_the_journal_and_its_neighbours_stay() {
    for main in ["main.zhang", "main.bean"] {
        let served = Served::with_main(main, &format!("{OPENS}{MEALS}"));
        let bakery = served.id_of("Bakery").await;
        let coffee = served.id_of("Cafe").await;

        let (status, body) = served.delete(&coffee).await;
        assert_eq!((status, &body), (StatusCode::OK, &json!({"data": null})), "{main}");

        // the text of the coffee and the blank line after it are gone; the comments, and the rest, stay as they were
        assert_eq!(served.read(main), format!("{OPENS}{MEALS}").replace(COFFEE, ""), "{main}");
        served.reloads_without_errors();
        // the journal served lists the others, without a reload of the test: the one before keeps its id, the one
        // after moved in the file, and has another one
        let transactions = served.transactions().await;
        let payees = transactions.iter().map(|(_, payee, _)| payee.as_str()).collect::<Vec<_>>();
        assert_eq!(payees, vec!["Bakery", "Noodles"], "{main}");
        assert_eq!(transactions[0].0, bakery, "{main}");
        assert!(transactions.iter().all(|(id, ..)| *id != coffee), "{main}: {transactions:?}");
        // and the id deleted is no transaction any more
        let (status, body) = served.delete(&coffee).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{main}: {body}");
    }
}

#[tokio::test]
async fn transactions_created_then_deleted_leave_their_file_as_it_was() {
    for (main, data) in [("main.zhang", "data/2024/01.zhang"), ("main.bean", "data/2024/01.bean")] {
        let main_text = format!("{OPENS}include \"{data}\"\n");
        let served = Served::new(Scratch::with_files(main, &[(main, &main_text), (data, "")]));
        let mut ids = vec![];
        for (day, payee) in [(1, "Bakery"), (2, "Cafe"), (3, "Noodles")] {
            let (status, body) = served.create(meal(day, payee, "meal", "5")).await;
            assert_eq!(status, StatusCode::OK, "{main}: {body}");
            ids.push(body["data"]["id"].as_str().expect("the id of the transaction created").to_owned());
        }
        assert_eq!(served.read(data).matches("\n\n").count(), 2, "{main}: one blank line between the entries");
        assert_eq!(served.id_of("Noodles").await, ids[2], "{main}");

        // the middle one: the one after it moves to its place, and takes its id
        let (status, body) = served.delete(&ids[1]).await;
        assert_eq!(status, StatusCode::OK, "{main}: {body}");
        assert_eq!(served.id_of("Noodles").await, ids[1], "{main}");
        // the last one, by the id the journal gives it now, then the only one left
        for id in [&ids[1], &ids[0]] {
            let (status, body) = served.delete(id).await;
            assert_eq!(status, StatusCode::OK, "{main}: {body}");
        }
        assert_eq!(served.read(data), "", "{main}");
        assert_eq!(served.read(main), main_text, "{main}");
        assert!(served.transactions().await.is_empty(), "{main}");
    }
}

#[tokio::test]
async fn the_id_a_create_answers_is_the_journals_and_takes_an_update_and_a_delete_right_away() {
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

        let (status, body) = served.delete(&id).await;
        assert_eq!(status, StatusCode::OK, "{main}: {body}");
        assert!(served.transactions().await.iter().all(|(it, ..)| *it != id), "{main}");
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
    let (status, _) = served.delete(created).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        served.read("main.zhang"),
        format!("option \"directive_output_path\" \"main.zhang\"\n{OPENS}{MEALS}")
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

#[tokio::test]
async fn an_unknown_transaction_is_a_404_and_no_id_a_400() {
    let served = Served::with_main("main.zhang", &format!("{OPENS}{MEALS}"));
    let unknown = "3fa85f64-5717-4562-b3fc-2c963f66afa6";
    let (status, body) = served.delete(unknown).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert!(
        message(&body).starts_with(&format!("there is no transaction {unknown} in the ledger")),
        "{body}"
    );
    let (status, body) = served.delete("nope").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(message(&body), "\"nope\" is not the id of a transaction");
    assert_eq!(served.read("main.zhang"), format!("{OPENS}{MEALS}"), "nothing is deleted");
}

/// The files may change after the load, as an editor changes them: a transaction that is no longer where the ledger
/// loaded it is not deleted, and its id is the one of another place. Tried again, the ledger is reloaded first, and
/// the id the journal gives now deletes the right text.
#[tokio::test]
async fn a_transaction_in_a_file_changed_since_the_load_is_not_deleted() {
    let served = Served::with_main("main.zhang", &format!("{OPENS}{MEALS}"));
    let coffee = served.id_of("Cafe").await;
    let edited = format!("; an editor adds this line\n{OPENS}{MEALS}");
    served.scratch.write("main.zhang", &edited);

    let (status, body) = served.delete(&coffee).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(
        message(&body).contains("main.zhang changed since the ledger was loaded, so nothing was written"),
        "{body}"
    );
    assert_eq!(served.read("main.zhang"), edited, "nothing is deleted");

    let (status, body) = served.delete(&coffee).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "the ledger is reloaded, the coffee moved: {body}");
    assert_eq!(served.read("main.zhang"), edited, "nothing is deleted");

    let coffee = served.id_of("Cafe").await;
    let (status, body) = served.delete(&coffee).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(served.read("main.zhang"), edited.replace(COFFEE, ""));
}

/// The padding a `pad`, or a `balance` with a pad, books is at the place of that directive, whose text is no
/// transaction: it is neither deleted nor replaced as one, which would drop the directive (an update replaced a
/// `balance` with a pad by the transaction it was sent, and the assertion was gone).
#[tokio::test]
async fn the_padding_of_a_pad_is_neither_deleted_nor_replaced() {
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

        let (status, body) = served.delete(&padding).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{main}: {body}");
        assert!(message(&body).contains("is the padding booked by the"), "{main}: {body}");
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

/// Through the router, as a client calls it: a delete needs the credentials every other write needs, and the writes
/// answer as their contracts say; the id a create answers takes a document right away.
#[tokio::test]
async fn a_delete_needs_credentials_like_every_write() {
    let scratch = Scratch::with_main("main.zhang", &format!("{OPENS}{MEALS}"));
    let settings = Settings {
        auth_credential: Some("admin:secret".to_owned()),
        ..Settings::default()
    };
    let router = router(scratch.dir(), "main.zhang", data_source_for("main.zhang"), &settings).await;
    let basic = format!("Basic {}", BASE64_STANDARD.encode("admin:secret"));
    let credentials = [("Authorization", basic.as_str())];

    let created = call(
        &router,
        Method::POST,
        "/api/transactions",
        &credentials,
        Some(&meal(15, "Deli", "sandwich", "8")),
    )
    .await;
    assert_eq!(created.status, StatusCode::OK, "{}", created.text());
    let id = created.json()["data"]["id"].as_str().expect("the id of the transaction created").to_owned();
    let path = format!("/api/transactions/{id}");

    // a document goes to the transaction created, by the id answered, right away
    let upload = Request::builder()
        .method(Method::POST)
        .uri(format!("{path}/documents"))
        .header("Authorization", basic.as_str())
        .header("content-type", "multipart/form-data; boundary=X")
        .body(Body::from(
            "--X\r\nContent-Disposition: form-data; name=\"file\"; filename=\"receipt.pdf\"\r\n\r\n%PDF\r\n--X--\r\n",
        ))
        .unwrap();
    let uploaded = send(&router, upload).await;
    assert_eq!(uploaded.status, StatusCode::OK, "{}", uploaded.text());
    let written = scratch.read("data/2024/01.zhang");
    assert!(written.contains("\"Deli\" \"sandwich\"\n  document: \"attachments/"), "{written}");

    let refused = call(&router, Method::DELETE, &path, &[], None).await;
    assert_eq!(refused.status, StatusCode::UNAUTHORIZED, "{}", refused.text());
    assert!(scratch.read("data/2024/01.zhang").contains("\"Deli\""), "nothing is deleted");

    let updated = call(&router, Method::PUT, &path, &credentials, Some(&meal(15, "Deli", "soup", "6"))).await;
    assert_eq!(updated.status, StatusCode::OK, "{}", updated.text());
    assert_eq!(updated.json(), json!({"data": {"id": id}}));

    let deleted = call(&router, Method::DELETE, &path, &credentials, None).await;
    assert_eq!(deleted.status, StatusCode::OK, "{}", deleted.text());
    assert_eq!(deleted.json(), json!({"data": null}));
    assert!(
        !scratch.read("data/2024/01.zhang").contains("\"Deli\""),
        "{}",
        scratch.read("data/2024/01.zhang")
    );

    let again = call(&router, Method::DELETE, &path, &credentials, None).await;
    assert_eq!(again.status, StatusCode::NOT_FOUND, "{}", again.text());
    assert!(again.json()["message"].as_str().unwrap().starts_with("there is no transaction"));
}

/// A delete reads and saves the file as an update does, through the data source: on one like the one `zhang serve`
/// runs, which resolves the paths within the ledger against its root, as on the local one.
#[tokio::test]
async fn a_delete_writes_through_the_data_source_of_the_ledger() {
    let scratch = Scratch::with_main("main.zhang", &format!("{OPENS}{MEALS}"));
    let router = router(
        scratch.dir(),
        "main.zhang",
        Arc::new(RootedFileSystem::new(scratch.dir())),
        &Settings::default(),
    )
    .await;
    let journal = get(&router, "/api/journals", &[]).await.json();
    let coffee = journal["data"]["records"].as_array().unwrap().iter().find(|it| it["payee"] == "Cafe").unwrap()["id"].clone();

    let deleted = call(&router, Method::DELETE, &format!("/api/transactions/{}", coffee.as_str().unwrap()), &[], None).await;
    assert_eq!(deleted.status, StatusCode::OK, "{}", deleted.text());
    assert_eq!(scratch.read("main.zhang"), format!("{OPENS}{MEALS}").replace(COFFEE, ""));
    let journal = get(&router, "/api/journals", &[]).await.json();
    assert_eq!(journal["data"]["records"].as_array().unwrap().len(), 2, "{journal}");
}
