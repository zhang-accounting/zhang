//! The HTTP side of the MCP endpoint: one POST in, one JSON object out.
//!
//! - `GET /` describes the plugin and where its MCP endpoint is, for a user who opens the plugin's link.
//! - `POST /mcp` is the MCP endpoint ([Streamable HTTP](https://modelcontextprotocol.io/specification/2026-07-28/basic/transports/streamable-http)):
//!   the `Origin` policy, then the method, the body size, the JSON-RPC framing, and the era dispatch of
//!   [`crate::era`]. Notifications and responses posted by a client are accepted with `202` and ignored; the server
//!   never streams (no SSE), never mints a session, and ignores `Mcp-Session-Id` and `Last-Event-ID`.
//! - anything else is `404`.

use serde_json::{json, Value};
use zhang_plugin_sdk::router::{Request, Response};

use crate::config::{normalize_origin, McpConfig, PLUGIN_NAME, PLUGIN_VERSION};
use crate::era::{self, SUPPORTED_VERSIONS};
use crate::host::Host;
use crate::jsonrpc::{self, Message, RpcError, INVALID_REQUEST, PARSE_ERROR};

/// the path of the MCP endpoint below the plugin's route: `/api/plugins/zhang-mcp/mcp`
pub const MCP_PATH: &str = "/mcp";
/// the largest request body the endpoint reads: JSON-RPC requests are small
pub const MAX_BODY_BYTES: usize = 1024 * 1024;

/// Answer one request to the plugin's route.
pub fn handle(request: &Request, host: &dyn Host, config: &McpConfig) -> Response {
    match request.path.trim_end_matches('/') {
        "" => info(request),
        MCP_PATH => mcp(request, host, config),
        _ => with_headers(json_response(404, &json!({"message": "not found"}))),
    }
}

/// `GET /`: what this plugin is and how to connect to it
fn info(request: &Request) -> Response {
    if request.method != "GET" {
        return with_headers(json_response(405, &json!({"message": "only GET is supported here"})).with_header("allow", "GET"));
    }
    with_headers(json_response(
        200,
        &json!({
            "name": PLUGIN_NAME,
            "version": PLUGIN_VERSION,
            "mcp_endpoint": format!("/api/plugins/{PLUGIN_NAME}{MCP_PATH}"),
            "supportedVersions": SUPPORTED_VERSIONS,
            "docs": "https://zhang-accounting.kilerd.me/guides/mcp/",
        }),
    ))
}

/// `POST /mcp`: the MCP endpoint
fn mcp(request: &Request, host: &dyn Host, config: &McpConfig) -> Response {
    if !origin_allowed(request.header("origin"), request.header("host"), &config.allowed_origins) {
        return error_response(
            403,
            None,
            &RpcError::new(INVALID_REQUEST, "this origin is not allowed to reach the MCP endpoint"),
        );
    }
    if request.method != "POST" {
        let error = RpcError::new(
            INVALID_REQUEST,
            "the MCP endpoint takes POST only: it offers no SSE stream and has no sessions to delete",
        );
        return error_response(405, None, &error).with_header("allow", "POST");
    }
    let body = match request.body_bytes() {
        Ok(body) => body,
        Err(_) => return error_response(400, None, &RpcError::new(PARSE_ERROR, "the body could not be decoded")),
    };
    if body.len() > MAX_BODY_BYTES {
        let error = RpcError::new(INVALID_REQUEST, format!("the body is larger than {MAX_BODY_BYTES} bytes"));
        return error_response(413, None, &error);
    }
    match jsonrpc::parse(&body) {
        Err(error) => error_response(400, None, &error),
        Ok(Message::Notification { .. } | Message::Response) => with_headers(Response::default().with_status(202)),
        Ok(Message::Request(rpc)) => {
            let reply = era::dispatch(request, &rpc, host, config);
            with_headers(json_response(reply.status, &reply.body))
        }
    }
}

/// Whether a request with this `Origin` header may reach the endpoint (the DNS rebinding check of the transport):
/// no `Origin` (a non-browser client) is allowed; an origin listed in `allowed` is allowed; an origin whose host and
/// port are the request's own `Host` is allowed only when that host is this machine (`localhost`, 127.0.0.0/8 or
/// `[::1]`), since a rebinding attack is precisely a `Host` the attacker controls; `null` and anything else is not.
pub fn origin_allowed(origin: Option<&str>, host: Option<&str>, allowed: &[String]) -> bool {
    let Some(origin) = origin.and_then(normalize_origin) else {
        return true;
    };
    if origin == "null" {
        return false;
    }
    if allowed.contains(&origin) {
        return true;
    }
    let Some(host) = host.and_then(normalize_origin) else {
        return false;
    };
    is_loopback(&host) && origin.split_once("://").is_some_and(|(_, origin_host)| origin_host == host)
}

/// whether `host` (`host[:port]`, lower case) names this machine: `localhost`, an IPv4 address in 127.0.0.0/8, or `[::1]`
fn is_loopback(host: &str) -> bool {
    let name = if let Some(bracketed) = host.strip_prefix('[') {
        bracketed.split(']').next().unwrap_or_default()
    } else if host.matches(':').count() > 1 {
        host
    } else {
        host.rsplit_once(':').map_or(host, |(name, _)| name)
    };
    name == "localhost" || name == "::1" || name.parse::<std::net::Ipv4Addr>().is_ok_and(|ip| ip.is_loopback())
}

fn json_response(status: u16, body: &Value) -> Response {
    Response::json(body).expect("a JSON value serializes").with_status(status)
}

fn error_response(status: u16, id: Option<&Value>, error: &RpcError) -> Response {
    with_headers(json_response(status, &jsonrpc::error_body(id, error)))
}

/// every answer of the endpoint: never cached, never sniffed
fn with_headers(response: Response) -> Response {
    response
        .with_header("cache-control", "no-store")
        .with_header("x-content-type-options", "nosniff")
}

#[cfg(test)]
mod test {
    use serde_json::{json, Value};
    use zhang_plugin_sdk::router::{Request, Response};

    use super::{handle, origin_allowed, MAX_BODY_BYTES};
    use crate::config::McpConfig;
    use crate::host::FakeHost;

    fn request(method: &str, path: &str, headers: &[(&str, &str)], body: &str) -> Request {
        Request {
            method: method.to_owned(),
            path: path.to_owned(),
            headers: headers.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect(),
            body: body.to_owned(),
            ..Request::default()
        }
    }

    fn post(body: &str) -> Response {
        handle(
            &request("POST", "/mcp", &[("host", "localhost:8000")], body),
            &FakeHost::new(),
            &McpConfig::default(),
        )
    }

    fn body(response: &Response) -> Value {
        serde_json::from_str(&response.body).unwrap_or_else(|e| panic!("not JSON ({e}): {}", response.body))
    }

    #[test]
    fn the_root_describes_the_endpoint_and_other_paths_are_404() {
        let root = handle(&request("GET", "/", &[], ""), &FakeHost::new(), &McpConfig::default());
        assert_eq!(root.status, 200);
        let info = body(&root);
        assert_eq!(info["name"], "zhang-mcp");
        assert_eq!(info["mcp_endpoint"], "/api/plugins/zhang-mcp/mcp");
        assert_eq!(info["supportedVersions"][0], "2026-07-28");
        assert_eq!(root.headers["content-type"], "application/json");
        assert_eq!(root.headers["cache-control"], "no-store");
        assert_eq!(root.headers["x-content-type-options"], "nosniff");
        let posted = handle(&request("POST", "/", &[], ""), &FakeHost::new(), &McpConfig::default());
        assert_eq!((posted.status, posted.headers["allow"].as_str()), (405, "GET"));
        let missing = handle(&request("GET", "/nope", &[], ""), &FakeHost::new(), &McpConfig::default());
        assert_eq!((missing.status, body(&missing)), (404, json!({"message": "not found"})));
    }

    #[test]
    fn non_post_methods_are_405_with_allow() {
        for method in ["GET", "DELETE", "PUT", "HEAD"] {
            let response = handle(&request(method, "/mcp", &[], ""), &FakeHost::new(), &McpConfig::default());
            assert_eq!(response.status, 405, "{method}");
            assert_eq!(response.headers["allow"], "POST");
            assert_eq!(body(&response)["error"]["code"], -32600);
            assert!(body(&response).get("id").is_none(), "a transport error has no id");
        }
        // `/mcp/` is the endpoint too
        let response = handle(&request("GET", "/mcp/", &[], ""), &FakeHost::new(), &McpConfig::default());
        assert_eq!(response.status, 405);
    }

    #[test]
    fn framing_errors_are_400_with_the_json_rpc_code() {
        let parse = post("{nope");
        assert_eq!((parse.status, body(&parse)["error"]["code"].as_i64()), (400, Some(-32700)));
        let batch = post(r#"[{"jsonrpc":"2.0","id":1,"method":"ping"}]"#);
        assert_eq!((batch.status, body(&batch)["error"]["code"].as_i64()), (400, Some(-32600)));
        assert!(body(&batch).get("id").is_none());
        assert!(body(&parse).get("id").is_none());
        let base64 = handle(&request("POST", "/mcp", &[], "not base64!"), &FakeHost::new(), &McpConfig::default());
        assert_eq!(base64.status, 400);
        let oversize = post(&format!(
            r#"{{"jsonrpc":"2.0","id":1,"method":"ping","params":{{"pad":"{}"}}}}"#,
            "x".repeat(MAX_BODY_BYTES)
        ));
        assert_eq!((oversize.status, body(&oversize)["error"]["code"].as_i64()), (413, Some(-32600)));
    }

    #[test]
    fn notifications_and_client_responses_are_202_without_a_body() {
        for message in [
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            r#"{"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":1}}"#,
            r#"{"jsonrpc":"2.0","id":5,"result":{}}"#,
        ] {
            let response = post(message);
            assert_eq!((response.status, response.body.as_str()), (202, ""), "{message}");
            assert!(!response.headers.contains_key("content-type"));
            assert_eq!(response.headers["cache-control"], "no-store");
        }
    }

    #[test]
    fn requests_are_answered_as_json_with_the_transport_headers() {
        let response = post(r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#);
        assert_eq!(response.status, 200);
        assert_eq!(body(&response), json!({"jsonrpc": "2.0", "id": 1, "result": {}}));
        assert_eq!(response.headers["content-type"], "application/json");
        assert_eq!(response.headers["cache-control"], "no-store");
        assert_eq!(response.headers["x-content-type-options"], "nosniff");
        assert!(!response.headers.contains_key("mcp-session-id"));
    }

    #[test]
    fn the_origin_policy() {
        let allowed = vec!["https://ledger.example.com".to_owned()];
        // absent: a CLI client
        assert!(origin_allowed(None, Some("localhost:8000"), &allowed));
        assert!(origin_allowed(None, Some("ledger.example.com"), &[]));
        // the same loopback host and port
        assert!(origin_allowed(Some("http://localhost:8000"), Some("localhost:8000"), &[]));
        assert!(origin_allowed(Some("http://LOCALHOST:8000/"), Some("localhost:8000"), &[]));
        assert!(origin_allowed(Some("http://127.0.0.1:8000"), Some("127.0.0.1:8000"), &[]));
        assert!(origin_allowed(Some("http://127.1.2.3"), Some("127.1.2.3"), &[]));
        assert!(origin_allowed(Some("http://[::1]:8000"), Some("[::1]:8000"), &[]));
        // the same host and port, but not this machine: a reverse proxy or a LAN address needs the allowlist
        assert!(!origin_allowed(Some("http://10.0.0.2:8000"), Some("10.0.0.2:8000"), &[]));
        assert!(!origin_allowed(Some("https://ledger.example.com"), Some("ledger.example.com"), &[]));
        assert!(!origin_allowed(Some("http://app.localhost:8000"), Some("app.localhost:8000"), &[]));
        assert!(!origin_allowed(Some("http://128.0.0.1:8000"), Some("128.0.0.1:8000"), &[]));
        // the allowlist, whatever the host
        assert!(origin_allowed(Some("https://ledger.example.com"), Some("10.0.0.2:8000"), &allowed));
        assert!(origin_allowed(Some("https://ledger.example.com"), Some("ledger.example.com"), &allowed));
        assert!(origin_allowed(Some("https://Ledger.Example.com/"), None, &allowed));
        // refused
        assert!(!origin_allowed(Some("http://localhost:5173"), Some("localhost:8000"), &[]));
        assert!(!origin_allowed(Some("http://localhost:8000"), Some("localhost"), &[]));
        assert!(!origin_allowed(Some("http://localhost:8000"), None, &[]));
        assert!(!origin_allowed(Some("null"), Some("localhost:8000"), &allowed));
        assert!(!origin_allowed(Some("https://evil.example.com"), Some("localhost:8000"), &allowed));
        assert!(!origin_allowed(Some("garbage"), Some("garbage"), &[]));

        let bad = handle(
            &request(
                "POST",
                "/mcp",
                &[("origin", "https://evil.example"), ("host", "localhost:8000")],
                r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#,
            ),
            &FakeHost::new(),
            &McpConfig::default(),
        );
        assert_eq!(bad.status, 403);
        assert_eq!(body(&bad)["error"]["code"], -32600);
        assert!(body(&bad).get("id").is_none());
        // the policy applies to every method
        let get = handle(&request("GET", "/mcp", &[("origin", "null")], ""), &FakeHost::new(), &McpConfig::default());
        assert_eq!(get.status, 403);
        let good = handle(
            &request(
                "POST",
                "/mcp",
                &[("origin", "http://localhost:8000"), ("host", "localhost:8000")],
                r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#,
            ),
            &FakeHost::new(),
            &McpConfig::default(),
        );
        assert_eq!(good.status, 200);
    }
}
