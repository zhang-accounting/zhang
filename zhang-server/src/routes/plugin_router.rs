//! `/api/plugins/{name}` and every path below it, for any HTTP method: requests routed to the
//! router plugin of that name. See [`zhang_core::plugin::router`] for the plugin side.
//!
//! The routes sit with the rest of the API, behind the same authentication. A request runs the
//! plugin on a blocking thread, under the ledger read lock. When the plugin cannot answer, the
//! response is a JSON `{"message": "..."}` with:
//! - 404 when no router plugin has that name
//! - 501 when the plugin does not export `router`
//! - 502 when it returns something that is not a valid response
//! - 504 when it runs longer than its timeout
//! - 500 when it traps, returns an error or cannot be loaded
//!
//! The details go to the log.

use std::sync::Arc;

use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use log::error;
use percent_encoding::{percent_decode_str, utf8_percent_encode, AsciiSet, CONTROLS};
use serde_json::Value;
use tokio::sync::OwnedRwLockReadGuard;
use zhang_core::ledger::Ledger;
use zhang_core::plugin::http::PluginRequest;
use zhang_core::plugin::router::{QueryFailure, RouterError, RouterHost};
use zhang_query::{Params, Query, QueryError};

use crate::error::error_response;
use crate::response::QueryResultEntity;
use crate::routes::query::{execute_options, max_result_values};
use crate::state::SharedLedger;

/// every router plugin's route and the paths below it: `/api/plugins/{name}`, `/api/plugins/{name}/`
/// and `/api/plugins/{name}/{*path}`. The handler splits the path itself, see [`split_route`]
pub const ROUTE: &str = "/api/plugins/*route";

/// what every router plugin's route starts with
const ROUTE_PREFIX: &str = "/api/plugins/";

/// the characters escaped in a plugin name to make it one path segment: the URL path set, plus
/// `/` and `%`
const SEGMENT: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'<')
    .add(b'>')
    .add(b'`')
    .add(b'?')
    .add(b'{')
    .add(b'}')
    .add(b'/')
    .add(b'%');

/// where the router plugin named `name` serves requests: `/api/plugins/{name}`
pub fn route_of(name: &str) -> String {
    format!("{ROUTE_PREFIX}{}", utf8_percent_encode(name, SEGMENT))
}

/// the plugin name and the path below its route in a request path: `/api/plugins/report/by-month`
/// gives `("report", "/by-month")`, and `/api/plugins/report` gives `("report", "/")`. The name is
/// percent-decoded, the path is kept as sent.
fn split_route(path: &str) -> Option<(String, String)> {
    let rest = path.strip_prefix(ROUTE_PREFIX)?;
    let (name, below) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
    let name = percent_decode_str(name).decode_utf8().ok()?.into_owned();
    let below = if below.is_empty() { "/" } else { below };
    Some((name, below.to_owned()))
}

/// Route a request to the router plugin its path names.
pub async fn route_to_plugin(State(ledger): State<SharedLedger>, method: Method, uri: Uri, headers: HeaderMap, body: Bytes) -> Response {
    let Some((name, path)) = split_route(uri.path()) else {
        return error_response(StatusCode::NOT_FOUND, "this is not the route of a router plugin".to_owned());
    };
    let query = form_urlencoded::parse(uri.query().unwrap_or_default().as_bytes()).into_owned();
    let headers = headers
        .iter()
        .map(|(name, value)| (name.as_str().to_owned(), String::from_utf8_lossy(value.as_bytes()).into_owned()));
    let request = PluginRequest::new(method.as_str(), path, query, headers, body.to_vec());
    dispatch(ledger, name, request).await
}

/// the [`RouterHost`] of one request: it owns the ledger read lock until the plugin returns
struct ServerRouterHost {
    ledger: OwnedRwLockReadGuard<Ledger>,
    max_result_values: u64,
}

impl RouterHost for ServerRouterHost {
    /// compiled and run like `POST /api/query`, with the same limits
    fn query(&self, bql: &str) -> Result<Value, QueryFailure> {
        let failure = |error: QueryError| QueryFailure {
            message: error.message,
            line: error.line,
            column: error.column,
        };
        let query = Query::compile(bql).map_err(failure)?;
        let result = query
            .execute_with_options(&self.ledger, &Params::new(), &execute_options(self.max_result_values))
            .map_err(failure)?;
        serde_json::to_value(QueryResultEntity::from(result)).map_err(|e| QueryFailure {
            message: format!("the result cannot be encoded: {e}"),
            line: None,
            column: None,
        })
    }
}

async fn dispatch(ledger: SharedLedger, name: String, request: PluginRequest) -> Response {
    // the plugin runs under the read lock, so the ledger cannot reload while it reads it
    let host = Arc::new(ServerRouterHost {
        ledger: ledger.0.clone().read_owned().await,
        max_result_values: max_result_values(),
    });
    let summary = format!("{} {}", request.method, request.path);
    let plugin_name = name.clone();
    // instantiating and running a plugin is CPU-bound and may block, so it stays off the async workers
    let result = tokio::task::spawn_blocking(move || {
        let plugin = host.ledger.plugins.router(&plugin_name)?;
        Some(plugin.execute_as_router(&request, &host.ledger, host.clone()))
    })
    .await;
    match result {
        Ok(Some(Ok(response))) => response.map(Body::from).into_response(),
        Ok(None) => error_response(StatusCode::NOT_FOUND, format!("no router plugin is named {name}")),
        Ok(Some(Err(e))) => {
            error!("router plugin {name} could not answer {summary}: {e}");
            let (status, message) = match e {
                RouterError::NoRouterExport => (StatusCode::NOT_IMPLEMENTED, format!("plugin {name} does not export `router`")),
                RouterError::BadResponse(reason) => (StatusCode::BAD_GATEWAY, format!("plugin {name} returned an invalid response: {reason}")),
                RouterError::Timeout => (StatusCode::GATEWAY_TIMEOUT, format!("plugin {name} ran longer than its timeout")),
                RouterError::Failed(_) => (StatusCode::INTERNAL_SERVER_ERROR, format!("plugin {name} failed while handling the request")),
                RouterError::Load(_) => (StatusCode::INTERNAL_SERVER_ERROR, format!("plugin {name} cannot be loaded")),
            };
            error_response(status, message)
        }
        Err(e) => {
            error!("router plugin {name} could not answer {summary}: the task failed: {e}");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, format!("plugin {name} failed while handling the request"))
        }
    }
}

#[cfg(test)]
mod test {
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use axum::body::Body;
    use axum::http::{header, Request, StatusCode};
    use axum::response::Response;
    use axum::routing::{any, get};
    use axum::Router;
    use serde_json::{json, Value};
    use tokio::sync::RwLock;
    use tower::ServiceExt;
    use zhang_core::clock::Clock;
    use zhang_core::data_source::LocalFileSystemDataSource;
    use zhang_core::data_type::text::ZhangDataType;
    use zhang_core::data_type::DataType;
    use zhang_core::ledger::{Ledger, LedgerProcessContext};
    use zhang_core::plugin::router::RouterHost;

    use super::{route_of, route_to_plugin, split_route, ServerRouterHost, ROUTE};
    use crate::routes::plugin::plugin_list;
    use crate::state::SharedLedger;

    /// the WAT fixtures of zhang-core's plugin tests, see `zhang-core/tests/wasm_plugins.rs`
    const FIXTURES: [(&str, &str); 6] = [
        ("router.wat", include_str!("../../../zhang-core/tests/plugins/router.wat")),
        ("router_query.wat", include_str!("../../../zhang-core/tests/plugins/router_query.wat")),
        ("router_no_export.wat", include_str!("../../../zhang-core/tests/plugins/router_no_export.wat")),
        ("router_malformed.wat", include_str!("../../../zhang-core/tests/plugins/router_malformed.wat")),
        ("router_trap.wat", include_str!("../../../zhang-core/tests/plugins/router_trap.wat")),
        ("router_loop.wat", include_str!("../../../zhang-core/tests/plugins/router_loop.wat")),
    ];

    /// the `timeout` of the looping fixture
    const LOOP_TIMEOUT: Duration = Duration::from_millis(500);

    /// A scratch ledger directory under the system temp dir, removed on drop.
    struct ScratchDir(PathBuf);

    impl Drop for ScratchDir {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).ok();
        }
    }

    /// the plugin list and the plugin routes, as the server registers them, over a ledger declaring every fixture
    async fn app() -> (ScratchDir, Router) {
        let dir = ScratchDir(std::env::temp_dir().join(format!("zhang-plugin-router-{}", uuid::Uuid::new_v4())));
        std::fs::create_dir_all(&dir.0).unwrap();
        let mut content = "option \"features.plugin\" \"true\"\noption \"title\" \"Home\"\noption \"timezone\" \"Asia/Shanghai\"\n".to_owned();
        for (fixture, wat) in FIXTURES {
            std::fs::write(dir.0.join(fixture), wat).unwrap();
            // a local ledger resolves a module against the working directory, so declare them by absolute path
            content.push_str(&format!("plugin \"{}\"\n", dir.0.join(fixture).display()));
            if fixture == "router_loop.wat" {
                content.push_str(&format!("  timeout: \"{}ms\"\n", LOOP_TIMEOUT.as_millis()));
            }
        }
        content.push_str(
            "1970-01-01 open Assets:Cash\n1970-01-01 open Expenses:Food\n\
             2024-01-02 * \"lunch\"\n  Assets:Cash -10 CNY\n  Expenses:Food 10 CNY\n",
        );
        std::fs::write(dir.0.join("main.zhang"), content).unwrap();
        let source = Arc::new(LocalFileSystemDataSource::new(ZhangDataType {}));
        let ledger = Ledger::async_load(dir.0.clone(), "main.zhang".to_owned(), source)
            .await
            .unwrap_or_else(|error| panic!("ledger should load: {error}"));
        let router = Router::new()
            .route("/api/plugins", get(plugin_list))
            .route(ROUTE, any(route_to_plugin))
            .with_state(SharedLedger(Arc::new(RwLock::new(ledger))));
        (dir, router)
    }

    async fn send(app: &Router, request: Request<Body>) -> Response {
        app.clone().oneshot(request).await.unwrap()
    }

    async fn json_body(response: Response) -> Value {
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&body).unwrap_or_else(|e| panic!("not JSON ({e}): {}", String::from_utf8_lossy(&body)))
    }

    #[tokio::test]
    async fn get_and_post_round_trip_through_the_plugin() {
        let (_dir, app) = app().await;

        let response = send(
            &app,
            Request::get("/api/plugins/router-echo/report/by%20month?k=v1&k=v2&q=a+b%26c")
                .header("X-Custom", "yes")
                .header(header::AUTHORIZATION, "Basic dXNlcjpwYXNz")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::CREATED);
        assert_eq!(response.headers()["x-echo"], "router");
        assert_eq!(response.headers()[header::CONTENT_TYPE], "application/json");
        assert_eq!(
            json_body(response).await,
            json!({
                "method": "GET",
                "path": "/report/by%20month",
                "query": {"k": ["v1", "v2"], "q": ["a b&c"]},
                // the credentials never reach the plugin
                "headers": {"x-custom": "yes"},
                "body": "",
                "body_encoding": "utf8",
            })
        );

        let response = send(
            &app,
            Request::post("/api/plugins/router-echo")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"note": "say \"hi\""}"#))
                .unwrap(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::CREATED);
        assert_eq!(
            json_body(response).await,
            json!({
                "method": "POST",
                "path": "/",
                "query": {},
                "headers": {"content-type": "application/json"},
                "body": r#"{"note": "say \"hi\""}"#,
                "body_encoding": "utf8",
            })
        );

        let response = send(&app, Request::put("/api/plugins/router-echo/").body(Body::from(vec![0xff, 0xfe])).unwrap()).await;
        let body = json_body(response).await;
        assert_eq!((&body["method"], &body["path"]), (&json!("PUT"), &json!("/")));
        assert_eq!((&body["body"], &body["body_encoding"]), (&json!("//4="), &json!("base64")));
    }

    #[tokio::test]
    async fn the_plugin_can_query_the_ledger() {
        let (_dir, app) = app().await;

        let response = send(&app, Request::get("/api/plugins/router-query/").body(Body::empty()).unwrap()).await;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            json_body(response).await,
            json!([
                {"Ok": {
                    "columns": [{"name": "account", "type": "str"}, {"name": "balance", "type": "inventory"}],
                    "rows": [
                        ["Assets:Cash", {"positions": [{"units": {"number": "-10", "currency": "CNY"}, "cost": null}]}],
                        ["Expenses:Food", {"positions": [{"units": {"number": "10", "currency": "CNY"}, "cost": null}]}],
                    ],
                }},
                {"Ok": {"title": "Home", "operating_currency": "CNY", "timezone": "Asia/Shanghai"}},
            ])
        );
    }

    #[tokio::test]
    async fn plugins_that_cannot_answer_get_a_json_error() {
        let (_dir, app) = app().await;

        for (path, status, message) in [
            ("/api/plugins/nobody/x", StatusCode::NOT_FOUND, "no router plugin is named nobody"),
            (
                "/api/plugins/router-no-export",
                StatusCode::NOT_IMPLEMENTED,
                "plugin router-no-export does not export `router`",
            ),
            (
                "/api/plugins/router-malformed/x",
                StatusCode::BAD_GATEWAY,
                "plugin router-malformed returned an invalid response: expected ident at line 1 column 2",
            ),
            (
                "/api/plugins/router-trap/x",
                StatusCode::INTERNAL_SERVER_ERROR,
                "plugin router-trap failed while handling the request",
            ),
        ] {
            let response = send(&app, Request::get(path).body(Body::empty()).unwrap()).await;
            assert_eq!(response.status(), status, "{path}");
            assert_eq!(response.headers()[header::CONTENT_TYPE], "application/json", "{path}");
            assert_eq!(json_body(response).await, json!({ "message": message }), "{path}");
        }

        // the server keeps serving after a plugin trapped
        let response = send(&app, Request::get("/api/plugins/router-echo").body(Body::empty()).unwrap()).await;
        assert_eq!(response.status(), StatusCode::CREATED);
        // and the plugin routes leave the plugin list alone
        let response = send(&app, Request::get("/api/plugins").body(Body::empty()).unwrap()).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(json_body(response).await["data"].as_array().unwrap().len(), FIXTURES.len());
    }

    #[tokio::test]
    async fn a_looping_plugin_answers_504_at_its_timeout() {
        let (_dir, app) = app().await;
        let started = Instant::now();

        let response = send(&app, Request::get("/api/plugins/router-loop/x").body(Body::empty()).unwrap()).await;

        let elapsed = started.elapsed();
        assert_eq!(response.status(), StatusCode::GATEWAY_TIMEOUT);
        assert_eq!(json_body(response).await, json!({"message": "plugin router-loop ran longer than its timeout"}));
        assert!(
            elapsed >= LOOP_TIMEOUT && elapsed < LOOP_TIMEOUT * 20,
            "answered after {elapsed:?}, not at the {LOOP_TIMEOUT:?} timeout"
        );
    }

    #[tokio::test]
    async fn plugin_queries_fail_as_api_queries_do() {
        let content = "1970-01-01 open Assets:Cash\n1970-01-01 open Expenses:Food\n\
                       2024-01-02 * \"lunch\"\n  Assets:Cash -10 CNY\n  Expenses:Food 10 CNY\n";
        let ledger = Ledger::process(LedgerProcessContext {
            directives: ZhangDataType {}.transform(content.to_owned(), None).unwrap(),
            entry: (PathBuf::from("."), "main.zhang".to_owned()),
            dialect: zhang_core::data_type::Dialect::Zhang,
            visited_files: vec![],
            data_source: Arc::new(LocalFileSystemDataSource::new(ZhangDataType {})),
            clock: Clock::System,
        })
        .unwrap();
        let ledger = Arc::new(RwLock::new(ledger));
        let host = |max_result_values| ServerRouterHost {
            ledger: ledger.clone().try_read_owned().unwrap(),
            max_result_values,
        };

        let failure = host(100).query("SELECT nope").unwrap_err();
        assert_eq!((failure.line, failure.column), (Some(1), Some(8)));
        // two postings of two cells each are four values
        let failure = host(3).query("SELECT account, position").unwrap_err();
        assert!(failure.message.starts_with("the result is too large"), "{}", failure.message);
        assert_eq!((failure.line, failure.column), (None, None));
        assert_eq!(
            host(4).query("SELECT account ORDER BY account").unwrap(),
            json!({"columns": [{"name": "account", "type": "str"}], "rows": [["Assets:Cash"], ["Expenses:Food"]]})
        );
    }

    #[test]
    fn a_route_names_the_plugin_and_the_path_below_it() {
        assert_eq!(split_route("/api/plugins/report"), Some(("report".to_owned(), "/".to_owned())));
        assert_eq!(split_route("/api/plugins/report/"), Some(("report".to_owned(), "/".to_owned())));
        assert_eq!(
            split_route("/api/plugins/my%20report/a/b%2Fc"),
            Some(("my report".to_owned(), "/a/b%2Fc".to_owned()))
        );
        assert_eq!(split_route("/api/other"), None);

        assert_eq!(route_of("report"), "/api/plugins/report");
        assert_eq!(route_of("my report/2024%"), "/api/plugins/my%20report%2F2024%25");
        let name = "my report/2024%";
        assert_eq!(split_route(&route_of(name)).map(|(name, _)| name).as_deref(), Some(name));
    }
}
