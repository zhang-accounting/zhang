//! One body for every API error, through the router the server runs: `{"message": …}` and nothing else, whether the
//! error is the server's, a rejection of an extractor (a body, a path or a query string that does not read), or a
//! malformed id. Extractor rejections were `text/plain`, a malformed transaction id a bare "bad request", and the server's
//! own errors carried an `origin` field no other error had.

use std::path::Path;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{header, Method, Request, StatusCode};
use axum::Router;
use gotcha::{GotchaApp, GotchaContext};
use serde_json::{json, Value};
use tokio::sync::RwLock;
use tower::ServiceExt;
use zhang_core::data_source::LocalFileSystemDataSource;
use zhang_core::data_type::text::ZhangDataType;
use zhang_core::ledger::Ledger;
use zhang_server::broadcast::Broadcaster;
use zhang_server::{create_server_app, ReloadSender, ServeConfig};

const MAIN: &str = "1970-01-01 commodity CNY\n1970-01-01 open Assets:Cash\n";

async fn server(dir: &Path) -> Router {
    std::fs::write(dir.join("main.zhang"), MAIN).unwrap();
    let source = Arc::new(LocalFileSystemDataSource::new(ZhangDataType {}));
    let ledger = Ledger::load(dir.to_path_buf(), "main.zhang".to_owned(), source.clone()).unwrap();
    let (sender, _receiver) = tokio::sync::mpsc::channel(8);
    let app = create_server_app(
        ServeConfig {
            path: dir.to_path_buf(),
            endpoint: "main.zhang".to_owned(),
            addr: "127.0.0.1".to_owned(),
            port: 0,
            no_report: true,
            data_source: source,
            auth_credential: None,
            passkey_secret: None,
            passkey_rp_id: None,
            passkey_origin: None,
            session_secret: None,
        },
        Arc::new(RwLock::new(ledger)),
        Broadcaster::create(),
        Arc::new(ReloadSender::new(sender)),
    );
    let config = app.config().await.unwrap();
    let state = app.state(&config).await.unwrap();
    app.build_router(GotchaContext { config, state }).await.unwrap()
}

/// the status, the content type and the JSON body of `method uri` with `body`, sent as JSON
async fn call(router: &Router, method: Method, uri: &str, body: Option<&str>) -> (StatusCode, String, Value) {
    let request = Request::builder().method(method).uri(uri);
    let request = match body {
        Some(body) => request.header(header::CONTENT_TYPE, "application/json").body(Body::from(body.to_owned())),
        None => request.body(Body::empty()),
    }
    .unwrap();
    let response = router.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .map(|it| it.to_str().unwrap().to_owned())
        .unwrap_or_default();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body = serde_json::from_slice(&bytes).unwrap_or_else(|_| json!({ "not json": String::from_utf8_lossy(&bytes) }));
    (status, content_type, body)
}

/// the message of an error body that holds a message and nothing else
fn message_only(body: &Value) -> &str {
    let object = body.as_object().unwrap_or_else(|| panic!("an object: {body}"));
    assert_eq!(object.keys().collect::<Vec<_>>(), vec!["message"], "{body}");
    object["message"].as_str().unwrap()
}

#[tokio::test]
async fn every_error_has_one_json_body_with_a_message() {
    let dir = tempfile::tempdir().unwrap();
    let router = server(dir.path()).await;
    for (method, uri, body, status) in [
        // the server's own error
        (Method::GET, "/api/accounts/Assets:Nowhere", None, StatusCode::NOT_FOUND),
        // a body that is not JSON, and one that is not the JSON the route takes
        (Method::POST, "/api/transactions", Some("{not json"), StatusCode::BAD_REQUEST),
        (Method::POST, "/api/transactions", Some("{\"payee\": 1}"), StatusCode::UNPROCESSABLE_ENTITY),
        // a path segment that is not a number
        (Method::GET, "/api/budgets/Food/interval/twenty/1", None, StatusCode::BAD_REQUEST),
        // a query string of axum's extractor missing a field
        (Method::GET, "/api/statistic/summary?from=2024-01-01", None, StatusCode::BAD_REQUEST),
        // a transaction id that is no id
        (Method::PUT, "/api/transactions/not-an-id", Some("{}"), StatusCode::UNPROCESSABLE_ENTITY),
    ] {
        let (answered, content_type, body) = call(&router, method.clone(), uri, body).await;
        assert_eq!(answered, status, "{method} {uri}: {body}");
        assert!(content_type.starts_with("application/json"), "{method} {uri}: {content_type}");
        assert!(!message_only(&body).is_empty(), "{method} {uri}: {body}");
    }
}

/// A transaction id that is no id is a 400 naming it, not a bare "bad request"
#[tokio::test]
async fn a_malformed_transaction_id_is_named() {
    let dir = tempfile::tempdir().unwrap();
    let router = server(dir.path()).await;
    let request = r#"{"datetime": "2024-01-01T00:00:00Z", "payee": "p", "postings": [], "metas": [], "tags": [], "links": []}"#;
    let (status, _, body) = call(&router, Method::PUT, "/api/transactions/not-an-id", Some(request)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(message_only(&body), "\"not-an-id\" is not the id of a transaction");
}
