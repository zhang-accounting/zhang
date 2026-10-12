//! `zhang-mcp` (`plugins/mcp`), the MCP server plugin, end to end: built for wasm32-unknown-unknown and driven
//! through the real server router with whole JSON-RPC sessions, legacy (`initialize`) and modern (2026-07-28),
//! plus the parity test that pins every BQL the plugin copies from a built-in query to the server's own text.
//!
//! The plugin is built once per test run, with cargo, into the same target directory as the SDK examples of
//! `sdk_plugins.rs`. Without the wasm32-unknown-unknown target the session tests are skipped locally with a message,
//! and fail on CI (`CI` set); the parity test needs no wasm.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, OnceLock};

use axum::http::Method;
use axum::Router;
use serde_json::{json, Value};
use tempfile::TempDir;
use zhang_core::data_source::LocalFileSystemDataSource;
use zhang_core::data_type::text::ZhangDataType;
use zhang_plugin_mcp::tools::queries::{ParamType, QUERIES};
use zhang_query::{DataType, ParamRef, ParamTypes, Query};
use zhang_server::builtin::BUILTINS;
use zhang_testkit::http::{call, Reply, Settings};

const WASM_TARGET: &str = "wasm32-unknown-unknown";
const ENDPOINT: &str = "/api/plugins/zhang-mcp/mcp";
const META_VERSION: &str = "io.modelcontextprotocol/protocolVersion";
const META_CAPABILITIES: &str = "io.modelcontextprotocol/clientCapabilities";
const META_SERVER_INFO: &str = "io.modelcontextprotocol/serverInfo";

// ---- parity: the plugin runs the app's own queries ------------------------------------------------------------

fn data_type(ty: ParamType) -> DataType {
    match ty {
        ParamType::Int => DataType::Int,
        ParamType::Str => DataType::Str,
        ParamType::Date => DataType::Date,
        ParamType::Set => DataType::Set,
    }
}

/// every query the plugin marks `builtin` is, text and parameters, the server's built-in query of that name; every
/// other one compiles with exactly the parameters it declares, as the built-in queries must
#[test]
fn the_plugin_queries_are_the_builtins_or_compile_like_them() {
    let mut pinned = BTreeSet::new();
    for query in QUERIES {
        let declared: BTreeSet<(String, DataType)> = query.params.iter().map(|(name, ty)| ((*name).to_owned(), data_type(*ty))).collect();
        if query.builtin {
            let builtin = BUILTINS
                .iter()
                .find(|it| it.name == query.name)
                .unwrap_or_else(|| panic!("the plugin copies a built-in query named {} but zhang-server has none", query.name));
            assert_eq!(query.bql, builtin.bql, "{}: the plugin's text is not the built-in query's", query.name);
            let theirs: BTreeSet<(String, DataType)> = builtin.params.iter().map(|(name, ty)| ((*name).to_owned(), *ty)).collect();
            assert_eq!(declared, theirs, "{}: the parameters differ from the built-in query's", query.name);
            pinned.insert(query.name);
        } else {
            assert!(query.name.starts_with("mcp."), "{}: a query of the plugin's own is named mcp.*", query.name);
            assert!(BUILTINS.iter().all(|it| it.name != query.name));
        }
        let types = query
            .params
            .iter()
            .fold(ParamTypes::new(), |types, (name, ty)| types.bind(*name, data_type(*ty)));
        let compiled = Query::compile_with_params(query.bql, &types).unwrap_or_else(|err| panic!("{} does not compile: {err}", query.name));
        let used: BTreeSet<(String, DataType)> = compiled
            .params()
            .into_iter()
            .map(|(param, ty)| match param {
                ParamRef::Named(name) => (name, ty),
                ParamRef::Positional(_) => panic!("{} uses a positional parameter", query.name),
            })
            .collect();
        assert_eq!(used, declared, "{}: the parameters its text uses, and those it declares", query.name);
    }
    assert_eq!(
        pinned,
        BTreeSet::from([
            "accounts.balances",
            "accounts.journal_page",
            "accounts.list",
            "accounts.subtree",
            "accounts.subtree_balances",
            "commodities.latest_price",
            "commodities.latest_prices",
            "journals.errors",
            "journals.payees",
            "report.account_totals",
            "report.net_worth",
        ])
    );
}

// ---- the wasm module ------------------------------------------------------------------------------------------------

/// the plugin built for wasm32, on first use; `None` when the target is missing and the session tests are skipped
fn module() -> Option<&'static PathBuf> {
    static MODULE: OnceLock<Option<PathBuf>> = OnceLock::new();
    MODULE.get_or_init(build_module).as_ref()
}

fn build_module() -> Option<PathBuf> {
    if !wasm_target_installed() {
        let reason = format!("the {WASM_TARGET} target is not installed; add it with `rustup target add {WASM_TARGET}`");
        if std::env::var_os("CI").is_some() {
            panic!("{reason}: CI must run the MCP plugin tests");
        }
        eprintln!("skipping the MCP plugin tests: {reason}");
        return None;
    }
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("zhang-server sits in the workspace root");
    // the SDK examples' directory too, so one build caches the SDK for both test binaries
    let target_dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("sdk-plugins");
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| env!("CARGO").into());
    let output = Command::new(cargo)
        .current_dir(workspace)
        .args(["build", "--release", "--target", WASM_TARGET, "-p", "zhang-plugin-mcp"])
        .arg("--target-dir")
        .arg(&target_dir)
        .output()
        .expect("cargo should run");
    assert!(
        output.status.success(),
        "building zhang-mcp failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    Some(target_dir.join(WASM_TARGET).join("release").join("zhang_plugin_mcp.wasm"))
}

fn wasm_target_installed() -> bool {
    let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let Ok(output) = Command::new(rustc).args(["--print", "sysroot"]).output() else {
        return false;
    };
    let sysroot = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    output.status.success() && Path::new(&sysroot).join("lib/rustlib").join(WASM_TARGET).join("lib").is_dir()
}

// ---- the fixture ledger -----------------------------------------------------------------------------------------------

/// Known figures: net worth 7610 CNY (Assets 7750 CNY, Liabilities -20 USD at 7 CNY), Expenses:Food 250 CNY + 20 USD
/// and Expenses:Rent 3000 CNY from February to March, one failing balance assertion, one closed account.
const LEDGER: &str = r#"option "features.plugin" "true"
option "title" "Home"
option "operating_currency" "CNY"
option "timezone" "Asia/Shanghai"
plugin "{module}"
  timeout: "20s"
  allowed_origins: "https://ledger.example.com, http://localhost:5173"
  max_rows: "50"

1970-01-01 commodity CNY
1970-01-01 commodity USD
1970-01-01 open Assets:Bank
1970-01-01 open Assets:Cash
1970-01-01 open Assets:Old
1970-01-01 open Equity:Opening
1970-01-01 open Liabilities:Card
1970-01-01 open Expenses:Food
1970-01-01 open Expenses:Rent
1970-01-01 open Income:Salary
2023-12-31 close Assets:Old

2024-01-01 * "Opening balance" "cash on hand"
  Assets:Cash 1000 CNY
  Equity:Opening -1000 CNY

2024-01-05 * "Employer" "salary"
  Assets:Bank 10000 CNY
  Income:Salary -10000 CNY

2024-02-01 * "Landlord" "rent" #housing
  Assets:Bank -3000 CNY
  Expenses:Rent 3000 CNY

2024-02-10 * "Cafe" "lunch" #food
  Assets:Cash -50 CNY
  Expenses:Food 50 CNY

2024-03-01 price USD 7 CNY

2024-03-05 * "Cafe" "dinner" #food
  Liabilities:Card -20 USD
  Expenses:Food 20 USD

2024-03-10 balance Assets:Bank 7000 CNY
2024-03-11 balance Assets:Cash 999 CNY

2024-03-12 * "Market" "groceries"
  Assets:Cash -200 CNY
  Expenses:Food 200 CNY
"#;

/// a ledger directory with the module under `plugins/` and the fixture as `main.zhang`
fn ledger_dir(module: &Path) -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("plugins")).unwrap();
    let copied = dir.path().join("plugins").join("zhang-mcp.wasm");
    std::fs::copy(module, &copied).unwrap_or_else(|e| panic!("cannot copy {}: {e}", module.display()));
    // a local ledger resolves a module against the working directory, so declare it by absolute path
    let absolute = dir.path().canonicalize().unwrap().join("plugins").join("zhang-mcp.wasm");
    std::fs::write(dir.path().join("main.zhang"), LEDGER.replace("{module}", &absolute.display().to_string())).unwrap();
    dir
}

/// the server's router over the fixture ledger, as `zhang serve` builds it
async fn server(dir: &TempDir, settings: &Settings) -> Router {
    let source = Arc::new(LocalFileSystemDataSource::new(ZhangDataType {}));
    zhang_testkit::http::router(&dir.path().canonicalize().unwrap(), "main.zhang", source, settings).await
}

// ---- JSON-RPC helpers -------------------------------------------------------------------------------------------------

async fn post(router: &Router, headers: &[(&str, &str)], body: &Value) -> Reply {
    call(router, Method::POST, ENDPOINT, headers, Some(body)).await
}

fn request(id: u64, method: &str, params: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})
}

/// a legacy `tools/call`, answered `200` with a result
async fn tool(router: &Router, name: &str, arguments: Value) -> Value {
    let reply = post(
        router,
        &[("mcp-protocol-version", "2025-11-25")],
        &request(9, "tools/call", json!({"name": name, "arguments": arguments})),
    )
    .await;
    assert_eq!(reply.status, 200, "{name}: {}", reply.text());
    let body = reply.json();
    assert_eq!(body["id"], 9);
    assert!(body.get("error").is_none(), "{name}: {body}");
    body["result"].clone()
}

/// the `structuredContent` of a successful tool call, checked against its text block
async fn structured(router: &Router, name: &str, arguments: Value) -> Value {
    let result = tool(router, name, arguments).await;
    assert!(result.get("isError").is_none(), "{name}: {result}");
    let text: Value = serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(text, result["structuredContent"]);
    result["structuredContent"].clone()
}

fn columns(rows: &Value) -> Vec<&str> {
    rows["columns"]
        .as_array()
        .unwrap()
        .iter()
        .map(|column| column["name"].as_str().unwrap())
        .collect()
}

/// the cell of `column` in `row`
fn cell<'a>(rows: &'a Value, row: usize, column: &str) -> &'a Value {
    let at = columns(rows)
        .iter()
        .position(|it| *it == column)
        .unwrap_or_else(|| panic!("no column {column} in {rows}"));
    &rows["rows"][row][at]
}

fn number(cell: &Value) -> f64 {
    cell.as_str().unwrap_or_else(|| panic!("not a decimal string: {cell}")).parse().unwrap()
}

/// an inventory cell as currency → units
fn inventory(cell: &Value) -> BTreeMap<String, f64> {
    cell["positions"]
        .as_array()
        .unwrap_or_else(|| panic!("not an inventory: {cell}"))
        .iter()
        .map(|position| (position["units"]["currency"].as_str().unwrap().to_owned(), number(&position["units"]["number"])))
        .collect()
}

// ---- sessions -------------------------------------------------------------------------------------------------------

#[tokio::test]
async fn a_legacy_session_initializes_lists_and_calls_every_tool() {
    let Some(module) = module() else {
        return;
    };
    let dir = ledger_dir(module);
    let router = server(&dir, &Settings::default()).await;

    // the plugin's own page
    let info = call(&router, Method::GET, "/api/plugins/zhang-mcp", &[], None).await;
    assert_eq!(info.status, 200, "{}", info.text());
    assert_eq!(info.json()["mcp_endpoint"], ENDPOINT);

    // initialize: the requested legacy version is echoed, no session is minted
    let reply = post(
        &router,
        &[],
        &request(
            1,
            "initialize",
            json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "test", "version": "0"}}),
        ),
    )
    .await;
    assert_eq!(reply.status, 200, "{}", reply.text());
    assert_eq!(reply.content_type(), "application/json");
    assert_eq!(reply.header("cache-control").as_deref(), Some("no-store"));
    assert_eq!(reply.header("x-content-type-options").as_deref(), Some("nosniff"));
    assert!(reply.header("mcp-session-id").is_none());
    let result = &reply.json()["result"];
    assert_eq!(result["protocolVersion"], "2025-06-18");
    assert_eq!(result["capabilities"], json!({"tools": {}}));
    assert_eq!(result["serverInfo"]["name"], "zhang-mcp");
    assert!(result["instructions"]
        .as_str()
        .unwrap()
        .contains("\"Home\" (operating currency CNY, timezone Asia/Shanghai)"));

    let reply = post(&router, &[], &json!({"jsonrpc": "2.0", "method": "notifications/initialized"})).await;
    assert_eq!((reply.status.as_u16(), reply.text().as_str()), (202, ""));

    let reply = post(&router, &[("mcp-protocol-version", "2025-06-18")], &request(2, "ping", json!({}))).await;
    assert_eq!(reply.json(), json!({"jsonrpc": "2.0", "id": 2, "result": {}}));

    // tools/list: twelve tools with schemas
    let reply = post(&router, &[("mcp-protocol-version", "2025-06-18")], &request(3, "tools/list", json!({}))).await;
    let tools = reply.json()["result"]["tools"].clone();
    let names: Vec<&str> = tools.as_array().unwrap().iter().map(|tool| tool["name"].as_str().unwrap()).collect();
    assert_eq!(
        names,
        [
            "ledger_info",
            "schema",
            "query",
            "accounts",
            "balances",
            "net_worth",
            "spending",
            "journal",
            "transactions",
            "payees",
            "errors",
            "prices"
        ]
    );
    for tool in tools.as_array().unwrap() {
        assert_eq!(tool["inputSchema"]["additionalProperties"], false);
        assert_eq!(tool["annotations"]["readOnlyHint"], true);
        assert!(tool["outputSchema"].is_object());
    }

    // ledger_info and schema read the host functions
    let info = structured(&router, "ledger_info", json!({})).await;
    assert_eq!(
        (&info["title"], &info["operating_currency"], &info["timezone"]),
        (&json!("Home"), &json!("CNY"), &json!("Asia/Shanghai"))
    );
    assert!(info["today"].as_str().unwrap().len() == 10 && info["now"].as_str().unwrap().contains('T'));
    let schema = structured(&router, "schema", json!({})).await;
    assert!(schema["tables"].as_array().unwrap().iter().any(|table| table["name"] == "transactions"));
    assert!(schema["functions"].as_array().unwrap().iter().any(|function| function["name"] == "convert"));
    assert!(!schema["columns"].as_array().unwrap().is_empty());

    // query: rows as the API encodes them, truncated at the limit
    let rows = structured(
        &router,
        "query",
        json!({"bql": "SELECT account, currency, sum(number) AS total WHERE under(account, 'Expenses') GROUP BY account, currency ORDER BY account, currency"}),
    )
    .await;
    assert_eq!(columns(&rows), ["account", "currency", "total"]);
    assert_eq!(
        rows["rows"],
        json!([
            ["Expenses:Food", "CNY", "250"],
            ["Expenses:Food", "USD", "20"],
            ["Expenses:Rent", "CNY", "3000"]
        ])
    );
    assert_eq!((rows["row_count"].as_u64(), rows["truncated"].as_bool()), (Some(3), Some(false)));
    let cut = structured(&router, "query", json!({"bql": "SELECT account GROUP BY account ORDER BY account", "limit": 2})).await;
    assert_eq!((cut["row_count"].as_u64(), cut["truncated"].as_bool()), (Some(2), Some(true)));

    // accounts: open ones now, the closed one on request
    let accounts = structured(&router, "accounts", json!({})).await;
    let listed: Vec<&str> = (0..accounts["row_count"].as_u64().unwrap() as usize)
        .map(|row| cell(&accounts, row, "account").as_str().unwrap())
        .collect();
    assert_eq!(
        listed,
        [
            "Assets:Bank",
            "Assets:Cash",
            "Equity:Opening",
            "Expenses:Food",
            "Expenses:Rent",
            "Income:Salary",
            "Liabilities:Card"
        ]
    );
    let closed = structured(&router, "accounts", json!({"prefix": "Assets", "status": "closed"})).await;
    assert_eq!(closed["rows"].as_array().unwrap().len(), 1);
    assert_eq!(cell(&closed, 0, "account"), "Assets:Old");
    assert_eq!(cell(&closed, 0, "status"), "closed");
    let all = structured(&router, "accounts", json!({"prefix": "Assets", "status": "all"})).await;
    assert_eq!(all["row_count"], 3);

    // balances: per account and currency, valued in CNY
    let balances = structured(&router, "balances", json!({"account": "Assets"})).await;
    assert_eq!(columns(&balances), ["account", "currency", "units", "value", "first_date"]);
    assert_eq!(cell(&balances, 0, "account"), "Assets:Bank");
    assert_eq!(number(cell(&balances, 0, "units")), 7000.0);
    assert_eq!(cell(&balances, 1, "account"), "Assets:Cash");
    assert_eq!(number(cell(&balances, 1, "units")), 750.0);
    let every = structured(&router, "balances", json!({})).await;
    assert!(every["row_count"].as_u64().unwrap() >= 6);

    // net worth: the app's figure
    let worth = structured(&router, "net_worth", json!({})).await;
    assert_eq!(columns(&worth), ["balance", "units", "value"]);
    assert_eq!(
        inventory(cell(&worth, 0, "units")),
        BTreeMap::from([("CNY".to_owned(), 7750.0), ("USD".to_owned(), -20.0)])
    );
    // `value` is the inventory converted into CNY: one position when every commodity has a price
    assert_eq!(inventory(cell(&worth, 0, "value")), BTreeMap::from([("CNY".to_owned(), 7610.0)]));
    let earlier = structured(&router, "net_worth", json!({"date": "2024-01-31"})).await;
    assert_eq!(inventory(cell(&earlier, 0, "value")), BTreeMap::from([("CNY".to_owned(), 11000.0)]));
    let usd = structured(&router, "net_worth", json!({"currency": "USD"})).await;
    let in_usd = inventory(cell(&usd, 0, "value"));
    assert_eq!(in_usd.keys().collect::<Vec<_>>(), ["USD"]);
    assert!((in_usd["USD"] - (7750.0 / 7.0 - 20.0)).abs() < 0.01, "{in_usd:?}");

    // spending: by account (the app's report), by month, by currency
    let spending = structured(&router, "spending", json!({"from": "2024-02-01", "to": "2024-03-31"})).await;
    assert_eq!(columns(&spending), ["account", "units", "value"]);
    assert_eq!(cell(&spending, 0, "account"), "Expenses:Food");
    assert_eq!(
        inventory(cell(&spending, 0, "units")),
        BTreeMap::from([("CNY".to_owned(), 250.0), ("USD".to_owned(), 20.0)])
    );
    assert_eq!(inventory(cell(&spending, 0, "value")), BTreeMap::from([("CNY".to_owned(), 390.0)]));
    assert_eq!(cell(&spending, 1, "account"), "Expenses:Rent");
    assert_eq!(inventory(cell(&spending, 1, "value")), BTreeMap::from([("CNY".to_owned(), 3000.0)]));
    let monthly = structured(
        &router,
        "spending",
        json!({"account": "Expenses:Food", "from": "2024-01-01", "to": "2024-03-31", "group_by": "month"}),
    )
    .await;
    assert_eq!(columns(&monthly), ["month", "units", "value"]);
    assert_eq!(monthly["rows"].as_array().unwrap().len(), 2, "{monthly}");
    assert_eq!(cell(&monthly, 0, "month"), "2024-02-01");
    assert_eq!(inventory(cell(&monthly, 0, "value")), BTreeMap::from([("CNY".to_owned(), 50.0)]));
    assert_eq!(cell(&monthly, 1, "month"), "2024-03-01");
    assert_eq!(inventory(cell(&monthly, 1, "value")), BTreeMap::from([("CNY".to_owned(), 340.0)]));
    let by_currency = structured(&router, "spending", json!({"from": "2024-01-01", "to": "2024-03-31", "group_by": "currency"})).await;
    assert_eq!(columns(&by_currency), ["currency", "units", "value"]);
    assert_eq!(cell(&by_currency, 0, "currency"), "CNY");
    assert_eq!(number(cell(&by_currency, 0, "units")), 3250.0);
    assert_eq!(cell(&by_currency, 1, "currency"), "USD");
    assert_eq!(number(cell(&by_currency, 1, "units")), 20.0);

    // journal: paged, with the running balance
    let journal = structured(&router, "journal", json!({"account": "Assets:Cash"})).await;
    assert_eq!(journal["row_count"], 3);
    assert_eq!(cell(&journal, 2, "payee"), "Market");
    assert_eq!(inventory(cell(&journal, 2, "balance")), BTreeMap::from([("CNY".to_owned(), 750.0)]));
    let page = structured(&router, "journal", json!({"account": "Assets:Cash", "limit": 2, "offset": 1})).await;
    assert_eq!((page["row_count"].as_u64(), page["truncated"].as_bool()), (Some(2), Some(false)));
    assert_eq!(cell(&page, 0, "payee"), "Cafe");
    let first = structured(&router, "journal", json!({"account": "Assets:Cash", "limit": 1})).await;
    assert_eq!((first["row_count"].as_u64(), first["truncated"].as_bool()), (Some(1), Some(true)));

    // transactions: newest first, every filter optional
    let found = structured(&router, "transactions", json!({"keyword": "cafe"})).await;
    assert_eq!(
        columns(&found),
        ["date", "time", "id", "flag", "payee", "narration", "tags", "links", "accounts"]
    );
    assert_eq!(found["row_count"], 2);
    assert_eq!(
        (cell(&found, 0, "date"), cell(&found, 0, "narration")),
        (&json!("2024-03-05"), &json!("dinner"))
    );
    assert_eq!(cell(&found, 1, "date"), "2024-02-10");
    let tagged = structured(&router, "transactions", json!({"tags": ["housing"], "from": "2024-01-01", "to": "2024-12-31"})).await;
    assert_eq!(tagged["row_count"], 1);
    assert_eq!(cell(&tagged, 0, "payee"), "Landlord");
    let everything = structured(&router, "transactions", json!({})).await;
    assert_eq!(everything["row_count"], 6);
    let none = structured(&router, "transactions", json!({"payee": "Nobody"})).await;
    assert_eq!(none["row_count"], 0);

    // payees, errors, prices
    let payees = structured(&router, "payees", json!({})).await;
    assert_eq!(payees["rows"], json!([["Cafe"], ["Employer"], ["Landlord"], ["Market"], ["Opening balance"]]));
    let errors = structured(&router, "errors", json!({})).await;
    assert_eq!(errors["row_count"], 1);
    assert!(columns(&errors).contains(&"kind"));
    assert!(cell(&errors, 0, "source").as_str().unwrap().contains("balance Assets:Cash 999 CNY"));
    let prices = structured(&router, "prices", json!({})).await;
    assert_eq!(columns(&prices), ["currency", "date", "time", "rate"]);
    assert_eq!((cell(&prices, 0, "currency"), cell(&prices, 0, "date")), (&json!("USD"), &json!("2024-03-01")));
    assert_eq!(number(cell(&prices, 0, "rate")), 7.0);
    let usd = structured(&router, "prices", json!({"commodity": "USD"})).await;
    assert_eq!(usd["row_count"], 1);

    // a failing query is a tool execution error, without structured content
    let failed = tool(&router, "query", json!({"bql": "SELECT nope"})).await;
    assert_eq!(failed["isError"], true);
    assert!(failed.get("structuredContent").is_none());
    assert!(
        failed["content"][0]["text"].as_str().unwrap().starts_with("query error at line 1, column 8:"),
        "{failed}"
    );

    // protocol errors
    let reply = post(&router, &[], &request(4, "tools/call", json!({"name": "nope"}))).await;
    assert_eq!(reply.status, 200);
    assert_eq!(reply.json()["error"]["code"], -32602);
    let reply = post(&router, &[], &request(5, "tools/call", json!({"name": "journal", "arguments": {"account": 1}}))).await;
    assert_eq!(reply.json()["error"]["code"], -32602);
    let reply = post(&router, &[], &request(6, "resources/list", json!({}))).await;
    assert_eq!((reply.status.as_u16(), reply.json()["error"]["code"].as_i64()), (200, Some(-32601)));
    let reply = post(&router, &[("mcp-protocol-version", "2024-11-05")], &request(7, "ping", json!({}))).await;
    assert_eq!((reply.status.as_u16(), reply.json()["error"]["code"].as_i64()), (400, Some(-32022)));

    // the transport
    let reply = call(&router, Method::GET, ENDPOINT, &[], None).await;
    assert_eq!((reply.status.as_u16(), reply.header("allow").as_deref()), (405, Some("POST")));
    let reply = call(&router, Method::DELETE, ENDPOINT, &[("mcp-session-id", "abc")], None).await;
    assert_eq!(reply.status, 405);
    let reply = post(&router, &[("origin", "https://evil.example")], &request(8, "ping", json!({}))).await;
    assert_eq!((reply.status.as_u16(), reply.json()["error"]["code"].as_i64()), (403, Some(-32600)));
    let reply = post(&router, &[("origin", "https://ledger.example.com")], &request(8, "ping", json!({}))).await;
    assert_eq!(reply.status, 200, "an allowed origin");
    let reply = post(&router, &[], &json!([request(8, "ping", json!({}))])).await;
    assert_eq!((reply.status.as_u16(), reply.json()["error"]["code"].as_i64()), (400, Some(-32600)));
    let reply = post(&router, &[], &json!({"jsonrpc": "2.0", "id": 3, "result": {}})).await;
    assert_eq!(reply.status, 202);
    let reply = call(&router, Method::GET, "/api/plugins/zhang-mcp/nope", &[], None).await;
    assert_eq!(reply.status, 404);
}

#[tokio::test]
async fn a_modern_session_discovers_and_calls_with_headers_and_meta() {
    let Some(module) = module() else {
        return;
    };
    let dir = ledger_dir(module);
    let router = server(&dir, &Settings::default()).await;
    let meta = json!({META_VERSION: "2026-07-28", META_CAPABILITIES: {}, "io.modelcontextprotocol/clientInfo": {"name": "test", "version": "0"}});

    let reply = post(
        &router,
        &[("mcp-protocol-version", "2026-07-28"), ("mcp-method", "server/discover")],
        &request(1, "server/discover", json!({"_meta": meta})),
    )
    .await;
    assert_eq!(reply.status, 200, "{}", reply.text());
    let result = &reply.json()["result"];
    assert_eq!(result["resultType"], "complete");
    assert_eq!(result["supportedVersions"], json!(["2026-07-28", "2025-11-25", "2025-06-18", "2025-03-26"]));
    assert_eq!(result["capabilities"], json!({"tools": {}}));
    assert_eq!(result["_meta"][META_SERVER_INFO]["name"], "zhang-mcp");
    assert_eq!((result["ttlMs"].as_u64(), result["cacheScope"].as_str()), (Some(3_600_000), Some("public")));
    assert!(reply.header("mcp-session-id").is_none());

    let reply = post(
        &router,
        &[("mcp-protocol-version", "2026-07-28"), ("mcp-method", "tools/list")],
        &request(2, "tools/list", json!({"_meta": meta})),
    )
    .await;
    let result = &reply.json()["result"];
    assert_eq!(result["tools"].as_array().unwrap().len(), 12);
    assert_eq!((result["resultType"].as_str(), result["ttlMs"].as_u64()), (Some("complete"), Some(3_600_000)));

    let reply = post(
        &router,
        &[("mcp-protocol-version", "2026-07-28"), ("mcp-method", "tools/call"), ("mcp-name", "net_worth")],
        &request(3, "tools/call", json!({"name": "net_worth", "arguments": {}, "_meta": meta})),
    )
    .await;
    assert_eq!(reply.status, 200, "{}", reply.text());
    let result = &reply.json()["result"];
    assert_eq!(result["resultType"], "complete");
    assert_eq!(result["_meta"][META_SERVER_INFO]["name"], "zhang-mcp");
    assert!(result.get("ttlMs").is_none());
    assert_eq!(
        inventory(cell(&result["structuredContent"], 0, "value")),
        BTreeMap::from([("CNY".to_owned(), 7610.0)])
    );
    // the base64 sentinel
    let reply = post(
        &router,
        &[
            ("mcp-protocol-version", "2026-07-28"),
            ("mcp-method", "tools/call"),
            ("mcp-name", "=?base64?bmV0X3dvcnRo?="),
        ],
        &request(3, "tools/call", json!({"name": "net_worth", "_meta": meta})),
    )
    .await;
    assert_eq!(reply.status, 200);

    // the error paths, each with the id of the request
    let mismatch = post(
        &router,
        &[("mcp-protocol-version", "2026-07-28"), ("mcp-method", "tools/call"), ("mcp-name", "balances")],
        &request(4, "tools/call", json!({"name": "net_worth", "_meta": meta})),
    )
    .await;
    assert_eq!((mismatch.status.as_u16(), mismatch.json()["error"]["code"].as_i64()), (400, Some(-32020)));
    assert_eq!(mismatch.json()["id"], 4);
    let no_header = post(&router, &[], &request(5, "tools/list", json!({"_meta": meta}))).await;
    assert_eq!((no_header.status.as_u16(), no_header.json()["error"]["code"].as_i64()), (400, Some(-32020)));
    let unsupported = post(
        &router,
        &[("mcp-protocol-version", "2027-01-01"), ("mcp-method", "tools/list")],
        &request(6, "tools/list", json!({"_meta": {META_VERSION: "2027-01-01", META_CAPABILITIES: {}}})),
    )
    .await;
    assert_eq!((unsupported.status.as_u16(), unsupported.json()["error"]["code"].as_i64()), (400, Some(-32022)));
    assert_eq!(unsupported.json()["error"]["data"]["supported"][0], "2026-07-28");
    let unknown = post(
        &router,
        &[("mcp-protocol-version", "2026-07-28"), ("mcp-method", "ping")],
        &request(7, "ping", json!({"_meta": meta})),
    )
    .await;
    assert_eq!((unknown.status.as_u16(), unknown.json()["error"]["code"].as_i64()), (404, Some(-32601)));
    let no_meta = post(
        &router,
        &[("mcp-protocol-version", "2026-07-28"), ("mcp-method", "tools/list")],
        &request(8, "tools/list", json!({})),
    )
    .await;
    assert_eq!((no_meta.status.as_u16(), no_meta.json()["error"]["code"].as_i64()), (400, Some(-32602)));
    let no_capabilities = post(
        &router,
        &[("mcp-protocol-version", "2026-07-28"), ("mcp-method", "tools/list")],
        &request(9, "tools/list", json!({"_meta": {META_VERSION: "2026-07-28"}})),
    )
    .await;
    assert_eq!(
        (no_capabilities.status.as_u16(), no_capabilities.json()["error"]["code"].as_i64()),
        (400, Some(-32602))
    );
    // a tool execution error keeps the modern envelope
    let failed = post(
        &router,
        &[("mcp-protocol-version", "2026-07-28"), ("mcp-method", "tools/call"), ("mcp-name", "query")],
        &request(10, "tools/call", json!({"name": "query", "arguments": {"bql": "SELECT nope"}, "_meta": meta})),
    )
    .await;
    let result = &failed.json()["result"];
    assert_eq!((result["isError"].as_bool(), result["resultType"].as_str()), (Some(true), Some("complete")));
    assert!(result.get("structuredContent").is_none());
}

#[tokio::test]
async fn the_endpoint_sits_behind_zhang_authentication() {
    let Some(module) = module() else {
        return;
    };
    let dir = ledger_dir(module);
    let settings = Settings {
        auth_credential: Some("admin:secret".to_owned()),
        session_secret: Some("0123456789abcdef0123456789abcdef".to_owned()),
        ..Settings::default()
    };
    let router = server(&dir, &settings).await;
    let ping = request(1, "ping", json!({}));

    let reply = post(&router, &[], &ping).await;
    assert_eq!(reply.status, 401);
    assert_eq!(reply.json(), json!({"message": "unauthorized"}));
    // Basic admin:secret
    let reply = post(&router, &[("authorization", "Basic YWRtaW46c2VjcmV0")], &ping).await;
    assert_eq!(reply.status, 200, "{}", reply.text());
    assert_eq!(reply.json()["result"], json!({}));
    let reply = post(&router, &[("authorization", "Basic YWRtaW46d3Jvbmc=")], &ping).await;
    assert_eq!(reply.status, 401);
}
