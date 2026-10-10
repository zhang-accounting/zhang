//! An account journal larger than a query may return: a 400 that asks for pages, while its pages work. The
//! result size limit is read once per process, from `ZHANG_QUERY_MAX_RESULT_VALUES`, so this test has a
//! binary of its own.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde_json::Value;
use zhang_server::request::AccountJournalRequest;
use zhang_server::routes::account::get_account_journals;
use zhang_server::routes::query::{max_result_values, MAX_RESULT_VALUES_ENV};
use zhang_server::routes::Query;
use zhang_testkit::http::{answer, shared};
use zhang_testkit::ledger::load_text;

/// the status, the `X-Total-Count` header and the JSON body
async fn respond(response: impl IntoResponse) -> (StatusCode, Option<String>, Value) {
    let answer = answer(response).await;
    (answer.status, answer.header("X-Total-Count"), answer.body)
}

#[tokio::test]
async fn a_journal_too_large_to_return_at_once_asks_for_pages() {
    // 2,000 postings of a dozen values each, against a limit of 20,000 values
    std::env::set_var(MAX_RESULT_VALUES_ENV, "20000");
    assert_eq!(max_result_values(), 20000);
    let mut text = String::from("option \"operating_currency\" \"CNY\"\n1970-01-01 commodity CNY\n1970-01-01 open Assets:Cash\n1970-01-01 open Equity:Open\n");
    for day in 0..2000 {
        let date = chrono::NaiveDate::from_ymd_opt(2000, 1, 1).unwrap() + chrono::Duration::days(day);
        text += &format!("{date} * \"Shop\" \"day {day}\"\n  Assets:Cash 1 CNY\n  Equity:Open\n");
    }
    let ledger = shared(load_text(&text));
    let path = || Path(("Assets:Cash".to_owned(),));

    let (status, _, body) = respond(get_account_journals(State(ledger.clone()), path(), Query(Default::default())).await).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(body["message"].as_str().unwrap().contains("`page` and `size`"), "{body}");

    let request = AccountJournalRequest {
        page: Some(3),
        size: Some(100),
    };
    let (status, total, body) = respond(get_account_journals(State(ledger.clone()), path(), Query(request)).await).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(total.as_deref(), Some("2000"));
    let rows = body["data"].as_array().unwrap();
    assert_eq!(rows.len(), 100);
    // newest first: page 3 starts at the 201st newest, day 1799, with the running balance after it
    assert_eq!(rows[0]["narration"], "day 1799");
    assert_eq!(rows[0]["account_after"]["number"], "1800");
    // the last page is fine too
    let request = AccountJournalRequest {
        page: Some(20),
        size: Some(100),
    };
    let (status, _, body) = respond(get_account_journals(State(ledger), path(), Query(request)).await).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"].as_array().unwrap().last().unwrap()["narration"], "day 0");
}
