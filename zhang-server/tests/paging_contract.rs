//! One paging contract for the two paged endpoints, `/api/journals` and `/api/accounts/{a}/journals`,
//! through the extractor and the handlers the server routes them to: pages count from 1, the first by default; a page
//! has 1 to 1000 rows, 100 by default; another page or size, or a query string that cannot be read, is a 400 with the
//! JSON `{message}` body of every other bad request.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use axum::routing::get;
use axum::Router;
use serde_json::{json, Value};
use tokio::sync::RwLock;
use tower::ServiceExt;
use zhang_core::data_source::LocalFileSystemDataSource;
use zhang_core::data_type::text::ZhangDataType;
use zhang_core::ledger::Ledger;
use zhang_server::routes::account::get_account_journals;
use zhang_server::routes::transaction::get_journals;
use zhang_server::state::SharedLedger;

/// three transactions on `Assets:Cash`, and one to accounts never opened, which are errors
const MAIN: &str = r#"
1970-01-01 commodity CNY
1970-01-01 open Assets:Cash
1970-01-01 open Expenses:Food
2024-01-01 * "one"
  Assets:Cash -1 CNY
  Expenses:Food 1 CNY
2024-01-02 * "two"
  Assets:Cash -2 CNY
  Expenses:Food 2 CNY
2024-01-03 * "three"
  Assets:Cash -3 CNY
  Expenses:Food 3 CNY
2024-01-04 * "unknown accounts"
  Assets:Nowhere -4 CNY
  Expenses:Nothing 4 CNY
"#;

/// the paged endpoints, routed as the server routes them
const ENDPOINTS: [&str; 2] = ["/api/journals", "/api/accounts/Assets:Cash/journals"];

async fn router() -> (tempfile::TempDir, Router) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("main.zhang"), MAIN).unwrap();
    let source = Arc::new(LocalFileSystemDataSource::new(ZhangDataType {}));
    let ledger = Ledger::load(dir.path().to_path_buf(), "main.zhang".to_owned(), source).unwrap();
    let router = Router::new()
        .route("/api/journals", get(get_journals))
        .route("/api/accounts/:account_name/journals", get(get_account_journals))
        .with_state(SharedLedger(Arc::new(RwLock::new(ledger))));
    (dir, router)
}

struct Reply {
    status: StatusCode,
    total_count: Option<String>,
    body: Value,
}

async fn call(router: &Router, uri: &str) -> Reply {
    let request = Request::builder().uri(uri).body(Body::empty()).unwrap();
    let response = router.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let total_count = response.headers().get("X-Total-Count").map(|it| it.to_str().unwrap().to_owned());
    let content_type = response.headers().get(header::CONTENT_TYPE).map(|it| it.to_str().unwrap().to_owned());
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body =
        serde_json::from_slice(&bytes).unwrap_or_else(|_| panic!("{uri}: expect a JSON body, got {content_type:?} {:?}", String::from_utf8_lossy(&bytes)));
    Reply { status, total_count, body }
}

/// A page of 0 is a 400 on every paged endpoint: `/api/journals` used to answer it with page 1, while an account's
/// journal refused it
#[tokio::test]
async fn a_page_of_zero_is_a_bad_request_on_every_paged_endpoint() {
    let (_dir, router) = router().await;
    for endpoint in ENDPOINTS {
        let reply = call(&router, &format!("{endpoint}?page=0&size=2")).await;
        assert_eq!(
            (reply.status, &reply.body["message"]),
            (StatusCode::BAD_REQUEST, &json!("page must be at least 1")),
            "{endpoint}"
        );
    }
}

/// A size of 0 or above 1000 is a 400 with one message on every paged endpoint
#[tokio::test]
async fn a_size_outside_1_to_1000_is_a_bad_request_with_one_message() {
    let (_dir, router) = router().await;
    for endpoint in ENDPOINTS {
        for size in [0, 1001] {
            let reply = call(&router, &format!("{endpoint}?page=1&size={size}")).await;
            assert_eq!(
                (reply.status, &reply.body["message"]),
                (StatusCode::BAD_REQUEST, &json!("size must be between 1 and 1000")),
                "{endpoint} size {size}"
            );
        }
    }
}

/// A query string that cannot be read is a 400 with a JSON `{message}` saying so, not an empty body (`/api/journals`)
#[tokio::test]
async fn a_query_string_that_cannot_be_read_is_a_bad_request_with_a_json_message() {
    let (_dir, router) = router().await;
    for endpoint in ENDPOINTS {
        for query in ["page=x", "size=-1"] {
            let reply = call(&router, &format!("{endpoint}?{query}")).await;
            assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{endpoint}?{query}");
            let message = reply.body["message"].as_str().unwrap_or_default();
            assert!(message.starts_with("the query string cannot be read"), "{endpoint}?{query}: {}", reply.body);
        }
    }
}

/// A good page keeps the shape each endpoint answered with: the `Pageable` body of `/api/journals`,
/// and the rows with the `X-Total-Count` header of an account's journal, which the frontend reads
#[tokio::test]
async fn a_page_keeps_the_shape_of_its_endpoint() {
    let (_dir, router) = router().await;
    let journals = call(&router, "/api/journals?page=2&size=2").await;
    assert_eq!(journals.status, StatusCode::OK);
    let data = &journals.body["data"];
    assert_eq!(
        (&data["total_count"], &data["total_page"], &data["page_size"], &data["current_page"]),
        (&json!(4), &json!(2), &json!(2), &json!(2))
    );
    assert_eq!(data["records"].as_array().unwrap().len(), 2);

    let account = call(&router, "/api/accounts/Assets:Cash/journals?page=1&size=2").await;
    assert_eq!((account.status, account.total_count.as_deref()), (StatusCode::OK, Some("3")));
    assert_eq!(account.body["data"].as_array().unwrap().len(), 2);
    // the default page and size
    let account = call(&router, "/api/accounts/Assets:Cash/journals?page=1").await;
    assert_eq!((account.total_count.as_deref(), account.body["data"].as_array().unwrap().len()), (Some("3"), 3));
}
