//! `POST /api/transactions` with an `Idempotency-Key` header: the key identifies the client's action, not its payload.
//! The transaction is written with the key as its `idempotency-key` metadata, and a create sent again with the key
//! writes nothing: it answers the transaction that has it, with `Idempotency-Replayed: true`, after a reload and a
//! restart of the server alike, as the key is in the ledger's files. Two creates with one key sent at once write once.

use axum::extract::State;
use axum::http::{Method, StatusCode};
use axum::Json;
use serde_json::{json, Value};
use zhang_server::request::JournalRequest;
use zhang_server::routes::transaction::{create_new_transaction, get_journals};
use zhang_server::routes::{IdempotencyKey, Query as UrlQuery};
use zhang_server::state::SharedLedger;
use zhang_testkit::fixtures::data_source_for;
use zhang_testkit::http::{answer, call, reload, router, shared, Answer, Settings};
use zhang_testkit::ledger::Scratch;

const OPENS: &str = "option \"operating_currency\" \"CNY\"\n1970-01-01 commodity CNY\n1970-01-01 open Assets:Cash\n1970-01-01 open Expenses:Food\n";

/// a create request for a coffee of `amount` CNY on 2024-01-15
fn coffee(amount: &str) -> Value {
    json!({
        "datetime": "2024-01-15T12:00:00",
        "payee": "Cafe",
        "narration": "coffee",
        "postings": [{"account": "Assets:Cash", "unit": format!("-{amount} CNY")}, {"account": "Expenses:Food", "unit": null}],
        "metas": [],
        "tags": [],
        "links": [],
    })
}

/// a create of `request` on `ledger`, with the `Idempotency-Key` `key` when there is one
async fn create(ledger: &SharedLedger, key: Option<&str>, request: Value) -> Answer {
    let request = serde_json::from_value(request).expect("a create request");
    answer(create_new_transaction(State(ledger.clone()), reload(), IdempotencyKey(key.map(str::to_owned)), Json(request)).await).await
}

/// the `(id, version)` a create answered, which must be a success
fn written(answer: &Answer) -> (Value, Value) {
    assert_eq!(answer.status, StatusCode::OK, "{}", answer.body);
    (answer.data()["id"].clone(), answer.data()["version"].clone())
}

/// whether the answer replays what an earlier create wrote
fn replayed(answer: &Answer) -> bool {
    match answer.header("Idempotency-Replayed").as_deref() {
        Some("true") => true,
        None => false,
        Some(other) => panic!("Idempotency-Replayed: {other}"),
    }
}

/// the transactions of the journal of `ledger`, each with its id and metadata
async fn transactions(ledger: &SharedLedger) -> Vec<Value> {
    let request = JournalRequest {
        page: None,
        size: Some(1000),
        keyword: None,
        tags: None,
        links: None,
    };
    let answer = answer(get_journals(State(ledger.clone()), UrlQuery(request)).await).await;
    let records = answer.data()["records"].as_array().unwrap().clone();
    records.into_iter().filter(|it| it["type"] == "Transaction").collect()
}

#[tokio::test]
async fn a_create_with_a_key_writes_it_once_and_one_sent_again_replays_it() {
    for (main, data) in [("main.zhang", "data/2024/01.zhang"), ("main.bean", "data/2024/01.bean")] {
        let scratch = Scratch::with_main(main, OPENS);
        let ledger = shared(scratch.ledger().unwrap());

        let first = create(&ledger, Some("coffee-1"), coffee("5")).await;
        let (id, version) = written(&first);
        assert!(!id.is_null() && !replayed(&first), "{main}: {}", first.body);
        let text = scratch.read(data);
        assert!(text.contains("\n  idempotency-key: \"coffee-1\"\n"), "{main}: {text}");
        let listed = transactions(&ledger).await;
        assert_eq!(listed[0]["metas"], json!([{"key": "idempotency-key", "value": "coffee-1"}]), "{main}");

        // sent again after the reload of the first, with another body: the key identifies the action, not the payload
        let again = create(&ledger, Some("coffee-1"), coffee("9")).await;
        assert_eq!(written(&again), (id.clone(), version.clone()), "{main}");
        assert!(replayed(&again), "{main}");
        assert_eq!(scratch.read(data), text, "{main}: nothing is written");

        // and after a restart of the server: the ledger is loaded from the files anew
        let restarted = shared(scratch.ledger().unwrap());
        let after_restart = create(&restarted, Some("coffee-1"), coffee("5")).await;
        assert_eq!(written(&after_restart), (id.clone(), version), "{main}");
        assert!(replayed(&after_restart), "{main}");
        assert_eq!(scratch.read(data), text, "{main}: nothing is written");

        // another key is another action
        let second = create(&restarted, Some("coffee-2"), coffee("5")).await;
        assert!(!replayed(&second) && written(&second).0 != id, "{main}: {}", second.body);
        assert_eq!(transactions(&restarted).await.len(), 2, "{main}");
        let reloaded = scratch.ledger().unwrap();
        assert!(reloaded.errors.is_empty(), "{main}: {:?}", reloaded.errors);
    }
}

/// The key is looked up while the create holds the ledger to write, in the ledger as reloaded after the write before
/// it: creates sent at once with one key write once, and every other answers the transaction written.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn creates_sent_at_once_with_one_key_write_once() {
    let scratch = Scratch::with_main("main.zhang", OPENS);
    let ledger = shared(scratch.ledger().unwrap());
    let creates = (0..8).map(|_| {
        let ledger = ledger.clone();
        tokio::spawn(async move { create(&ledger, Some("same-action"), coffee("5")).await })
    });
    let mut answers = vec![];
    for create in creates.collect::<Vec<_>>() {
        answers.push(create.await.unwrap());
    }
    let ids = answers.iter().map(|it| written(it).0).collect::<Vec<_>>();
    assert!(ids.iter().all(|id| *id == ids[0] && !id.is_null()), "{ids:?}");
    assert_eq!(answers.iter().filter(|it| !replayed(it)).count(), 1, "one create writes, the others replay it");
    assert_eq!(scratch.read("data/2024/01.zhang").matches("idempotency-key").count(), 1);
    assert_eq!(transactions(&ledger).await.len(), 1);
}

/// A request with the header and its own `idempotency-key` metadata is refused: the server writes that one. Without the
/// header the metadata is like any other, and a create with the key it holds then finds it.
#[tokio::test]
async fn the_metadata_of_the_key_is_the_servers_with_the_header() {
    let scratch = Scratch::with_main("main.zhang", OPENS);
    let ledger = shared(scratch.ledger().unwrap());
    let mut request = coffee("5");
    request["metas"] = json!([{"key": "idempotency-key", "value": "mine"}]);

    let refused = create(&ledger, Some("mine"), request.clone()).await;
    assert_eq!(refused.status, StatusCode::BAD_REQUEST, "{}", refused.body);
    assert!(refused.body["message"].as_str().unwrap().contains("\"idempotency-key\""), "{}", refused.body);
    assert!(!scratch.dir().join("data").exists(), "nothing is written");

    let plain = create(&ledger, None, request).await;
    let (id, _) = written(&plain);
    assert!(!replayed(&plain));
    let keyed = create(&ledger, Some("mine"), coffee("7")).await;
    assert_eq!(written(&keyed).0, id);
    assert!(replayed(&keyed));
}

/// Through the router: the header is checked, `Idempotency-Replayed` answered, and the previews ignore the header.
#[tokio::test]
async fn the_header_is_one_key_of_the_characters_of_a_token() {
    let scratch = Scratch::with_main("main.zhang", OPENS);
    let router = router(scratch.dir(), "main.zhang", data_source_for("main.zhang"), &Settings::default()).await;
    let long = "k".repeat(129);
    for key in ["", "two words", "a/b", "a,b", "%41", long.as_str()] {
        let refused = call(&router, Method::POST, "/api/transactions", &[("Idempotency-Key", key)], Some(&coffee("5"))).await;
        assert_eq!(refused.status, StatusCode::BAD_REQUEST, "{key:?}: {}", refused.text());
        assert!(
            refused.json()["message"].as_str().unwrap().starts_with("invalid_idempotency_key: "),
            "{}",
            refused.text()
        );
    }
    let two = [("Idempotency-Key", "a"), ("Idempotency-Key", "b")];
    let refused = call(&router, Method::POST, "/api/transactions", &two, Some(&coffee("5"))).await;
    assert_eq!(refused.status, StatusCode::BAD_REQUEST, "two keys: {}", refused.text());
    assert!(!scratch.dir().join("data").exists(), "nothing is written");

    let key = [("Idempotency-Key", "Az09._~-")];
    let first = call(&router, Method::POST, "/api/transactions", &key, Some(&coffee("5"))).await;
    assert_eq!(first.status, StatusCode::OK, "{}", first.text());
    assert_eq!(first.header("Idempotency-Replayed"), None);
    let again = call(&router, Method::POST, "/api/transactions", &key, Some(&coffee("5"))).await;
    assert_eq!(again.header("Idempotency-Replayed").as_deref(), Some("true"));
    assert_eq!(again.json()["data"], first.json()["data"]);

    // a preview ignores the header, a key the ledger has and one no create takes alike
    let id = first.json()["data"]["id"].as_str().unwrap().to_owned();
    for key in ["Az09._~-", "two words"] {
        let headers = [("Idempotency-Key", key)];
        let preview = call(&router, Method::POST, "/api/transactions/preview", &headers, Some(&coffee("5"))).await;
        assert_eq!(preview.status, StatusCode::OK, "{key}: {}", preview.text());
        assert!(
            preview.json()["data"]["text"].as_str().unwrap().contains("\"Cafe\" \"coffee\""),
            "{}",
            preview.text()
        );
        let path = format!("/api/transactions/{id}/preview");
        let preview = call(&router, Method::POST, &path, &headers, Some(&coffee("6"))).await;
        assert_eq!(preview.status, StatusCode::OK, "{key}: {}", preview.text());
    }
    assert_eq!(
        scratch.read("data/2024/01.zhang").matches("\"Cafe\" \"coffee\"").count(),
        1,
        "the previews write nothing"
    );
}
