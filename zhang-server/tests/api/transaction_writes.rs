//! `POST`, `PUT` and `DELETE /api/transactions`: what they leave in the files, and the ids and versions they answer
//! with. The ledger is reloaded before the answer, so the next request, sent to the same server without a reload of
//! its own, finds the transaction under the id answered, or no longer finds the one deleted. An update or a delete
//! that names the version of the transaction in `If-Match` (a delete must) writes nothing when the id names another
//! text by then.

use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Path as UrlPath, State};
use axum::http::{Method, Request, StatusCode};
use axum::Json;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;
use serde_json::{json, Value};
use sha2::Digest;
use zhang_server::request::JournalRequest;
use zhang_server::routes::account::get_account_journals;
use zhang_server::routes::transaction::{
    create_new_transaction, delete_single_transaction, get_journals, preview_transaction_update, update_single_transaction,
};
use zhang_server::routes::{IfMatch, Query as UrlQuery, RequiredIfMatch};
use zhang_server::state::{SharedLedger, SharedReloadSender};
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
    /// the one reload sender of the server, which keeps the tombstones of the writes
    reload: State<SharedReloadSender>,
}

impl Served {
    fn new(scratch: Scratch) -> Served {
        let ledger = shared(scratch.ledger().unwrap_or_else(|error| panic!("the ledger should load: {error}")));
        Served {
            scratch,
            ledger,
            reload: reload(),
        }
    }

    /// the reload sender of the server
    fn reload(&self) -> State<SharedReloadSender> {
        State(self.reload.0.clone())
    }

    /// `main.zhang` or `main.bean` holding `content`
    fn with_main(main: &str, content: &str) -> Served {
        Served::new(Scratch::with_main(main, content))
    }

    async fn create(&self, request: Value) -> (StatusCode, Value) {
        let request = serde_json::from_value(request).expect("a create request");
        respond(create_new_transaction(State(self.ledger.clone()), self.reload(), Json(request)).await).await
    }

    /// an update, without `If-Match`
    async fn update(&self, id: &str, request: Value) -> (StatusCode, Value) {
        self.update_if(id, None, request).await
    }

    /// an update, with `If-Match: <version>` when there is one
    async fn update_if(&self, id: &str, version: Option<&str>, request: Value) -> (StatusCode, Value) {
        let request = serde_json::from_value(request).expect("an update request");
        let if_match = IfMatch(version.map(|it| vec![it.to_owned()]));
        respond(update_single_transaction(State(self.ledger.clone()), self.reload(), UrlPath((id.to_owned(),)), if_match, Json(request)).await).await
    }

    /// a delete, with `If-Match: <version>`
    async fn delete(&self, id: &str, version: &str) -> (StatusCode, Value) {
        let if_match = RequiredIfMatch(vec![version.to_owned()]);
        respond(delete_single_transaction(State(self.ledger.clone()), self.reload(), UrlPath((id.to_owned(),)), if_match).await).await
    }

    /// a delete of the transaction of `payee`, by the id and the version the journal gives it
    async fn delete_payee(&self, payee: &str) -> (StatusCode, Value) {
        let (id, version) = self.entry(payee).await;
        self.delete(&id, &version).await
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
        self.entry(payee).await.0
    }

    /// The id and the version the journal lists the transaction of `payee` with.
    async fn entry(&self, payee: &str) -> (String, String) {
        let records = self.journal().await;
        let found = records.into_iter().find(|it| it["type"] == "Transaction" && it["payee"] == payee);
        let found = found.unwrap_or_else(|| panic!("the journal lists {payee}"));
        let text = |value: &Value| value.as_str().unwrap_or_else(|| panic!("{payee}: {found}")).to_owned();
        (text(&found["id"]), text(&found["version"]))
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

/// the version of a transaction written as `text`: the SHA-256 of the text, in hex
fn version_of(text: &str) -> String {
    sha2::Sha256::digest(text.as_bytes()).iter().map(|byte| format!("{byte:02x}")).collect()
}

#[tokio::test]
async fn a_deleted_transaction_leaves_its_file_and_the_journal_and_its_neighbours_stay() {
    for main in ["main.zhang", "main.bean"] {
        let served = Served::with_main(main, &format!("{OPENS}{MEALS}"));
        let bakery = served.id_of("Bakery").await;
        let (coffee, version) = served.entry("Cafe").await;
        assert_eq!(version, version_of(COFFEE.trim_end()), "{main}: the version is that of the text");

        let (status, body) = served.delete(&coffee, &version).await;
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
        // and the delete sent again finds no transaction at the place of the coffee, where the comment on the lunch is now
        let (status, body) = served.delete(&coffee, &version).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{main}: {body}");
        assert_eq!(served.read(main), format!("{OPENS}{MEALS}").replace(COFFEE, ""), "{main}");
    }
}

#[tokio::test]
async fn transactions_created_then_deleted_leave_their_file_as_it_was() {
    for (main, data) in [("main.zhang", "data/2024/01.zhang"), ("main.bean", "data/2024/01.bean")] {
        let main_text = format!("{OPENS}include \"{data}\"\n");
        let served = Served::new(Scratch::with_files(main, &[(main, &main_text), (data, "")]));
        let mut created = vec![];
        for (day, payee) in [(1, "Bakery"), (2, "Cafe"), (3, "Noodles")] {
            let (status, body) = served.create(meal(day, payee, "meal", "5")).await;
            assert_eq!(status, StatusCode::OK, "{main}: {body}");
            let text = |key: &str| body["data"][key].as_str().unwrap_or_else(|| panic!("{main}: {key} in {body}")).to_owned();
            created.push((text("id"), text("version")));
            // the id and the version the create answers are the journal's
            assert_eq!(served.entry(payee).await, created.last().unwrap().clone(), "{main}");
        }
        assert_eq!(served.read(data).matches("\n\n").count(), 2, "{main}: one blank line between the entries");

        // the middle one: the one after it moves to its place, and takes its id, with its own version
        let (status, body) = served.delete(&created[1].0, &created[1].1).await;
        assert_eq!(status, StatusCode::OK, "{main}: {body}");
        let after = served.read(data);
        assert_eq!(served.entry("Noodles").await, (created[1].0.clone(), created[2].1.clone()), "{main}");
        // so the delete, sent again as a client retries it, is refused, and the noodles stay
        let (status, body) = served.delete(&created[1].0, &created[1].1).await;
        assert_eq!(status, StatusCode::PRECONDITION_FAILED, "{main}: {body}");
        assert_eq!(served.read(data), after, "{main}: nothing is deleted");
        // the last one, by the id the journal gives it now, then the only one left
        for payee in ["Noodles", "Bakery"] {
            let (status, body) = served.delete_payee(payee).await;
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

        let version = body["data"]["version"].as_str().expect("the version of the transaction created").to_owned();

        let (status, body) = served.update_if(&id, Some(&version), meal(15, "Deli", "soup", "6")).await;
        assert_eq!(status, StatusCode::OK, "{main}: {body}");
        assert_eq!(body["data"]["id"], json!(id), "{main}: an update keeps the id");
        let version = body["data"]["version"].as_str().expect("the version of the update").to_owned();
        assert_eq!(served.entry("Deli").await, (id.clone(), version.clone()), "{main}");
        assert!(
            served.transactions().await.contains(&(id.clone(), "Deli".to_owned(), "soup".to_owned())),
            "{main}: the journal lists the update under the id"
        );

        let (status, body) = served.delete(&id, &version).await;
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
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["id"], json!(bakery));
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
    let (status, _) = served.delete(created, body["data"]["version"].as_str().unwrap()).await;
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
    assert_eq!((status, &body), (StatusCode::OK, &json!({"data": {"id": null, "version": null}})));
    assert!(served.read("data/2024/01.zhang").contains("\"Deli\" \"sandwich\""));
    assert!(served.transactions().await.is_empty());
}

#[tokio::test]
async fn an_unknown_transaction_is_a_404_and_no_id_a_400() {
    let served = Served::with_main("main.zhang", &format!("{OPENS}{MEALS}"));
    let unknown = "3fa85f64-5717-4562-b3fc-2c963f66afa6";
    let (status, body) = served.delete(unknown, &version_of("anything")).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert!(
        message(&body).starts_with(&format!("there is no transaction {unknown} in the ledger")),
        "{body}"
    );
    let (status, body) = served.delete("nope", &version_of("anything")).await;
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
    let (coffee, version) = served.entry("Cafe").await;
    let edited = format!("; an editor adds this line\n{OPENS}{MEALS}");
    served.scratch.write("main.zhang", &edited);

    let (status, body) = served.delete(&coffee, &version).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(
        message(&body).contains("main.zhang changed since the ledger was loaded, so nothing was written"),
        "{body}"
    );
    assert_eq!(served.read("main.zhang"), edited, "nothing is deleted");

    let (status, body) = served.delete(&coffee, &version).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "the ledger is reloaded, the coffee moved: {body}");
    assert_eq!(served.read("main.zhang"), edited, "nothing is deleted");

    let (status, body) = served.delete_payee("Cafe").await;
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

        let (status, body) = served.delete(&padding, &version_of("anything")).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{main}: {body}");
        assert!(message(&body).contains("is the padding booked by the"), "{main}: {body}");
        let (status, body) = served.update(&padding, meal(5, "Bank", "padding", "1")).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{main}: {body}");
        assert!(message(&body).contains("is the padding booked by the"), "{main}: {body}");
        // and the preview of the update says so
        let request = serde_json::from_value(meal(5, "Bank", "padding", "1")).unwrap();
        let (status, body) =
            respond(preview_transaction_update(State(served.ledger.clone()), UrlPath((padding.clone(),)), IfMatch(None), Json(request)).await).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{main}: {body}");
        assert_eq!(served.read(main), text, "{main}: nothing is written");
    }
}

/// Through the router, as a client calls it: a delete needs the credentials every other write needs and the version
/// of the transaction in `If-Match`, an update takes it, and the writes answer as their contracts say; the id a create
/// answers takes a document right away.
#[tokio::test]
async fn a_delete_needs_credentials_and_the_version_of_the_transaction() {
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

    // the upload changed the text, so the version the create answered is stale; the journal gives the new one
    let stale = created.json()["data"]["version"].as_str().unwrap().to_owned();
    let journal = call(&router, Method::GET, "/api/journals", &credentials, None).await.json();
    let version = journal["data"]["records"][0]["version"].as_str().unwrap().to_owned();
    assert_ne!(version, stale);
    let (stale_tag, current_tag) = (format!("\"{stale}\""), format!("\"{version}\""));
    let stale_headers = [("Authorization", basic.as_str()), ("If-Match", stale_tag.as_str())];
    let refused = call(&router, Method::PUT, &path, &stale_headers, Some(&meal(15, "Deli", "soup", "6"))).await;
    assert_eq!(refused.status, StatusCode::PRECONDITION_FAILED, "{}", refused.text());

    let current_headers = [("Authorization", basic.as_str()), ("If-Match", current_tag.as_str())];
    let updated = call(&router, Method::PUT, &path, &current_headers, Some(&meal(15, "Deli", "soup", "6"))).await;
    assert_eq!(updated.status, StatusCode::OK, "{}", updated.text());
    assert_eq!(updated.json()["data"]["id"], json!(id));
    let version = updated.json()["data"]["version"].as_str().unwrap().to_owned();

    // a delete needs the version
    let refused = call(&router, Method::DELETE, &path, &credentials, None).await;
    assert_eq!(refused.status, StatusCode::PRECONDITION_REQUIRED, "{}", refused.text());
    assert!(
        refused.json()["message"].as_str().unwrap().contains("needs an If-Match header"),
        "{}",
        refused.text()
    );
    assert!(scratch.read("data/2024/01.zhang").contains("\"Deli\""), "nothing is deleted");
    // a weak tag, or `*`, names no version
    for if_match in [format!("W/\"{version}\""), "*".to_owned()] {
        let headers = [("Authorization", basic.as_str()), ("If-Match", if_match.as_str())];
        let refused = call(&router, Method::DELETE, &path, &headers, None).await;
        assert_eq!(refused.status, StatusCode::PRECONDITION_FAILED, "{if_match}: {}", refused.text());
    }
    assert!(scratch.read("data/2024/01.zhang").contains("\"Deli\""), "nothing is deleted");

    // one of several versions, quoted or not
    let if_match = format!("\"{stale}\", {version}");
    let credentials = [("Authorization", basic.as_str()), ("If-Match", if_match.as_str())];
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

/// An update may name the version of the transaction in `If-Match`, as a delete must: when the transaction has another
/// version by then, the update and its preview are refused with a 412 and nothing is written. Without it, an update
/// is written as before, as the web UI sends it.
#[tokio::test]
async fn an_update_naming_a_stale_version_writes_nothing() {
    let served = Served::with_main("main.zhang", &format!("{OPENS}{MEALS}"));
    let (coffee, version) = served.entry("Cafe").await;
    let (status, body) = served.update_if(&coffee, Some(&version), meal(2, "Cafe", "latte", "6")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let latte = served.read("main.zhang");
    assert!(latte.contains("\"Cafe\" \"latte\""), "{latte}");

    // the version read before that update is stale now
    let (status, body) = served.update_if(&coffee, Some(&version), meal(2, "Cafe", "tea", "4")).await;
    assert_eq!(status, StatusCode::PRECONDITION_FAILED, "{body}");
    assert!(message(&body).contains("is not the version the If-Match header names"), "{body}");
    let request = serde_json::from_value(meal(2, "Cafe", "tea", "4")).unwrap();
    let if_match = IfMatch(Some(vec![version.clone()]));
    let (status, body) = respond(preview_transaction_update(State(served.ledger.clone()), UrlPath((coffee.clone(),)), if_match, Json(request)).await).await;
    assert_eq!(status, StatusCode::PRECONDITION_FAILED, "the preview says so: {body}");
    assert_eq!(served.read("main.zhang"), latte, "nothing is written");

    // without If-Match, as before
    let (status, body) = served.update(&coffee, meal(2, "Cafe", "tea", "4")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(served.read("main.zhang").contains("\"Cafe\" \"tea\""));
}

/// The rows of an account's journal give the version of their transaction, as the journal does; a balance
/// assertion has none.
#[tokio::test]
async fn the_account_journal_gives_the_versions_of_the_transactions() {
    let served = Served::with_main("main.zhang", &format!("{OPENS}{MEALS}\n2024-01-04 balance Assets:Cash -20 CNY\n"));
    let versions = served
        .journal()
        .await
        .into_iter()
        .filter(|it| it["type"] == "Transaction")
        .map(|it| (it["id"].clone(), it["version"].clone()))
        .collect::<Vec<_>>();
    let request = UrlQuery(Default::default());
    let (status, body) = respond(get_account_journals(State(served.ledger.clone()), UrlPath(("Assets:Cash".to_owned(),)), request).await).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let rows = body["data"].as_array().unwrap();
    assert_eq!(rows.len(), 4, "{body}");
    for row in rows {
        match row["asserted"].is_null() {
            true => assert!(versions.contains(&(row["trx_id"].clone(), row["trx_version"].clone())), "{row}"),
            false => assert!(row["trx_version"].is_null(), "{row}"),
        }
    }
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
    let coffee = journal["data"]["records"]
        .as_array()
        .unwrap()
        .iter()
        .find(|it| it["payee"] == "Cafe")
        .unwrap()
        .clone();
    let path = format!("/api/transactions/{}", coffee["id"].as_str().unwrap());

    let if_match = [("If-Match", coffee["version"].as_str().unwrap())];
    let deleted = call(&router, Method::DELETE, &path, &if_match, None).await;
    assert_eq!(deleted.status, StatusCode::OK, "{}", deleted.text());
    assert_eq!(scratch.read("main.zhang"), format!("{OPENS}{MEALS}").replace(COFFEE, ""));
    let journal = get(&router, "/api/journals", &[]).await.json();
    assert_eq!(journal["data"]["records"].as_array().unwrap().len(), 2, "{journal}");
}

/// Every write of the API reloads the ledger before it answers: the balances, the documents of an account and the
/// save of a file, as the writes of transactions. A request sent after the answer reads what it wrote.
#[tokio::test]
async fn every_write_reloads_before_it_answers() {
    let scratch = Scratch::with_main("main.zhang", &format!("{OPENS}1970-01-01 open Equity:Open\n{MEALS}"));
    let router = router(scratch.dir(), "main.zhang", data_source_for("main.zhang"), &Settings::default()).await;
    let journal = || async { get(&router, "/api/journals", &[]).await.json()["data"]["records"].as_array().unwrap().len() };
    assert_eq!(journal().await, 3);

    let balance = json!({"type": "Check", "amount": {"number": "-20", "commodity": "CNY"}});
    let answer = call(&router, Method::POST, "/api/accounts/Assets:Cash/balances", &[], Some(&balance)).await;
    assert!(answer.status.is_success(), "{}", answer.text());
    assert_eq!(journal().await, 4, "the balance is in the journal");

    let batch = json!([{"type": "Pad", "account_name": "Assets:Cash", "amount": {"number": "100", "commodity": "CNY"}, "pad": "Equity:Open"}]);
    let answer = call(&router, Method::POST, "/api/accounts/batch-balances", &[], Some(&batch)).await;
    assert!(answer.status.is_success(), "{}", answer.text());
    assert!(journal().await > 4, "the padding and its balance are in the journal");

    let upload = Request::builder()
        .method(Method::POST)
        .uri("/api/accounts/Assets:Cash/documents")
        .header("content-type", "multipart/form-data; boundary=X")
        .body(Body::from(
            "--X\r\nContent-Disposition: form-data; name=\"file\"; filename=\"statement.pdf\"\r\n\r\n%PDF\r\n--X--\r\n",
        ))
        .unwrap();
    let answer = send(&router, upload).await;
    assert!(answer.status.is_success(), "{}", answer.text());
    let documents = call(
        &router,
        Method::POST,
        "/api/query/builtins/journals.documents",
        &[],
        Some(&json!({"params": {}})),
    )
    .await;
    assert!(documents.text().contains("statement.pdf"), "the document is listed: {}", documents.text());

    let path = format!("/api/files/{}", BASE64_STANDARD.encode("main.zhang"));
    let content = format!("{OPENS}1970-01-01 open Equity:Open\n");
    let answer = call(&router, Method::PUT, &path, &[], Some(&json!({"content": content}))).await;
    assert!(answer.status.is_success(), "{}", answer.text());
    assert_eq!(journal().await, 0, "the journal is that of the file saved");
}
