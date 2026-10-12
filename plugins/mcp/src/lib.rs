//! `zhang-mcp`: an [MCP](https://modelcontextprotocol.io/) server for a zhang ledger, as a router plugin.
//!
//! Declared with
//!
//! ```zhang
//! option "features.plugin" "true"
//! plugin "plugins/zhang-mcp.wasm"
//!   timeout: "20s"
//! ```
//!
//! it serves the MCP endpoint `POST /api/plugins/zhang-mcp/mcp` behind zhang's own authentication, speaking both the
//! legacy `initialize` lifecycle (2025-03-26 … 2025-11-25) and the stateless 2026-07-28 revision, with twelve
//! read-only tools that map onto the same BQL the app runs. See `plugins/mcp/DESIGN.md` and the "MCP" guide of the
//! docs.
//!
//! The crate also builds natively: [`transport::handle`] runs against any [`host::Host`], so the dispatcher is unit
//! tested without zhang, and zhang-server's tests read [`tools::QUERIES`] to pin the copied BQL to its own.

pub mod config;
pub mod era;
pub mod host;
pub mod jsonrpc;
pub mod tools;
pub mod transport;

use zhang_plugin_sdk::config::Config;
use zhang_plugin_sdk::router::{Request, Response};
use zhang_plugin_sdk::{plugin, Error};

use crate::config::{McpConfig, PLUGIN_NAME, PLUGIN_VERSION};
use crate::host::SdkHost;

plugin! {
    name: PLUGIN_NAME,
    version: PLUGIN_VERSION,
    router: route,
}

/// the `router` export: every request to `/api/plugins/zhang-mcp` and below
fn route(request: Request) -> Result<Response, Error> {
    let config = McpConfig::from_sdk(&Config::load());
    Ok(transport::handle(&request, &SdkHost, &config))
}

#[cfg(test)]
mod test {
    use zhang_plugin_sdk::router::Request;

    use super::route;

    #[test]
    fn the_entry_answers_without_a_ledger_where_it_can() {
        let info = route(Request {
            method: "GET".to_owned(),
            path: "/".to_owned(),
            ..Request::default()
        })
        .unwrap();
        assert_eq!(info.status, 200);
        assert!(info.body.contains("\"zhang-mcp\""));
        let ping = route(Request {
            method: "POST".to_owned(),
            path: "/mcp".to_owned(),
            body: r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#.to_owned(),
            ..Request::default()
        })
        .unwrap();
        assert_eq!(ping.status, 200);
        assert!(ping.body.contains("\"result\":{}"));
        // natively the host functions are unavailable: a tool answers isError, never a failed export
        let call = route(Request {
            method: "POST".to_owned(),
            path: "/mcp".to_owned(),
            body: r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"payees"}}"#.to_owned(),
            ..Request::default()
        })
        .unwrap();
        assert_eq!(call.status, 200);
        assert!(call.body.contains("\"isError\":true"), "{}", call.body);
    }
}
