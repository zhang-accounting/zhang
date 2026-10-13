//! `zhang-mcp` (`plugins/mcp/mcp.lua`), the MCP server plugin, end to end: the script is copied from the repository
//! into a fixture ledger and driven through the real server router with whole JSON-RPC sessions, legacy
//! (`initialize`) and modern (2026-07-28), plus the parity test that pins every BQL the plugin copies from a
//! built-in query to the server's own text (read through the plugin's hidden `GET /queries` route), the write
//! sessions (preview → open → preview → add → the file holds the text → a replay conflicts) and the read-only ones.
//!
//! No build step: the script runs on the Lua interpreter the host embeds. The legacy, modern and write sessions also
//! run against `plugin "zhang:mcp"`, the copy of the script built into zhang, with no file in the ledger.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Arc;

use axum::http::Method;
use axum::Router;
use serde_json::{json, Value};
use tempfile::TempDir;
use tokio::sync::RwLock;
use zhang_core::data_source::{DataSource, LocalFileSystemDataSource};
use zhang_core::data_type::text::ZhangDataType;
use zhang_core::ledger::Ledger;
use zhang_query::{DataType, ParamRef, ParamTypes, Query};
use zhang_server::broadcast::Broadcaster;
use zhang_server::builtin::BUILTINS;
use zhang_server::{create_server_app, start_reload_listener, ReloadSender};
use zhang_testkit::http::{call, router_of, serve_config, Reply, Settings};

const ENDPOINT: &str = "/api/plugins/zhang-mcp/mcp";
const META_VERSION: &str = "io.modelcontextprotocol/protocolVersion";
const META_CAPABILITIES: &str = "io.modelcontextprotocol/clientCapabilities";
const META_SERVER_INFO: &str = "io.modelcontextprotocol/serverInfo";

/// the twelve read tools, in the order `tools/list` returns them
const READ_TOOLS: [&str; 12] = [
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
    "prices",
];
const WRITE_TOOLS: [&str; 3] = ["preview_transaction", "add_transaction", "open_account"];

// ---- the fixture ledger -----------------------------------------------------------------------------------------------

/// the script as it is in the repository
fn script() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../plugins/mcp/mcp.lua"))
}

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
{metas}
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

/// how the fixture ledger gets the plugin
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Install {
    /// a copy of the script under `plugins/`, declared by its path
    Copied,
    /// the copy built into zhang, declared as `plugin "zhang:mcp"`: no file in the ledger
    Bundled,
}

/// a ledger directory with the script under `plugins/` and the fixture as `main.zhang`, `metas` being extra meta
/// lines of the `plugin` directive (indented, newline-terminated)
fn ledger_dir(metas: &str) -> TempDir {
    ledger_dir_with(Install::Copied, metas)
}

/// [`ledger_dir`], with the plugin installed as `install` says
fn ledger_dir_with(install: Install, metas: &str) -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    let module = match install {
        Install::Copied => {
            std::fs::create_dir_all(dir.path().join("plugins")).unwrap();
            let copied = dir.path().join("plugins").join("mcp.lua");
            std::fs::copy(script(), &copied).unwrap_or_else(|e| panic!("cannot copy {}: {e}", script().display()));
            // a local ledger resolves a module against the working directory, so declare it by absolute path
            let absolute = dir.path().canonicalize().unwrap().join("plugins").join("mcp.lua");
            absolute.display().to_string()
        }
        Install::Bundled => "zhang:mcp".to_owned(),
    };
    let ledger = LEDGER.replace("{module}", &module).replace("{metas}\n", metas);
    std::fs::write(dir.path().join("main.zhang"), ledger).unwrap();
    dir
}

/// a ledger directory declaring the bundled plugin, which holds no copy of the script
fn bundled_ledger_dir(metas: &str) -> TempDir {
    let dir = ledger_dir_with(Install::Bundled, metas);
    let files: Vec<_> = std::fs::read_dir(dir.path()).unwrap().map(|entry| entry.unwrap().file_name()).collect();
    assert_eq!(files, ["main.zhang"], "nothing but the main file");
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

/// the text of a tool execution error
async fn error_text(router: &Router, name: &str, arguments: Value) -> String {
    let result = tool(router, name, arguments).await;
    assert_eq!(result["isError"], true, "{name}: {result}");
    assert!(result.get("structuredContent").is_none(), "{name}: {result}");
    result["content"][0]["text"].as_str().unwrap().to_owned()
}

/// the names `tools/list` answers, legacy era
async fn tool_names(router: &Router) -> Vec<String> {
    let reply = post(router, &[("mcp-protocol-version", "2025-06-18")], &request(3, "tools/list", json!({}))).await;
    assert_eq!(reply.status, 200, "{}", reply.text());
    reply.json()["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap().to_owned())
        .collect()
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

// ---- parity: the plugin runs the app's own queries ------------------------------------------------------------

fn data_type(ty: &str) -> DataType {
    match ty {
        "int" => DataType::Int,
        "str" => DataType::Str,
        "date" => DataType::Date,
        "set" => DataType::Set,
        other => panic!("the plugin declares a parameter of the unknown type {other}"),
    }
}

/// every query the plugin marks `builtin` is, text and parameters, the server's built-in query of that name; every
/// other one compiles with exactly the parameters it declares, as the built-in queries must
#[tokio::test]
async fn the_plugin_queries_are_the_builtins_or_compile_like_them() {
    let dir = ledger_dir("");
    let router = server(&dir, &Settings::default()).await;
    let reply = call(&router, Method::GET, "/api/plugins/zhang-mcp/queries", &[], None).await;
    assert_eq!(reply.status, 200, "{}", reply.text());
    let queries = reply.json()["queries"].as_array().unwrap().clone();
    assert_eq!(queries.len(), 14);

    let mut pinned = BTreeSet::new();
    for query in &queries {
        let name = query["name"].as_str().unwrap();
        let bql = query["bql"].as_str().unwrap();
        let declared: BTreeSet<(String, DataType)> = query["params"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(name, ty)| (name.clone(), data_type(ty.as_str().unwrap())))
            .collect();
        if query["builtin"] == true {
            let builtin = BUILTINS
                .iter()
                .find(|it| it.name == name)
                .unwrap_or_else(|| panic!("the plugin copies a built-in query named {name} but zhang-server has none"));
            assert_eq!(bql, builtin.bql, "{name}: the plugin's text is not the built-in query's");
            let theirs: BTreeSet<(String, DataType)> = builtin.params.iter().map(|(name, ty)| ((*name).to_owned(), *ty)).collect();
            assert_eq!(declared, theirs, "{name}: the parameters differ from the built-in query's");
            pinned.insert(name.to_owned());
        } else {
            assert!(name.starts_with("mcp."), "{name}: a query of the plugin's own is named mcp.*");
            assert!(BUILTINS.iter().all(|it| it.name != name));
        }
        let types = declared.iter().fold(ParamTypes::new(), |types, (name, ty)| types.bind(name, *ty));
        let compiled = Query::compile_with_params(bql, &types).unwrap_or_else(|err| panic!("{name} does not compile: {err}"));
        let used: BTreeSet<(String, DataType)> = compiled
            .params()
            .into_iter()
            .map(|(param, ty)| match param {
                ParamRef::Named(name) => (name, ty),
                ParamRef::Positional(_) => panic!("{name} uses a positional parameter"),
            })
            .collect();
        assert_eq!(used, declared, "{name}: the parameters its text uses, and those it declares");
    }
    assert_eq!(
        pinned,
        BTreeSet::from_iter(
            [
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
            ]
            .map(str::to_owned)
        )
    );
}

// ---- sessions -------------------------------------------------------------------------------------------------------

#[tokio::test]
async fn a_legacy_session_initializes_lists_and_calls_every_tool() {
    legacy_session(ledger_dir("")).await;
}

/// the same session against `plugin "zhang:mcp"`: the bundled copy is the repository's script
#[tokio::test]
async fn a_legacy_session_against_the_bundled_plugin() {
    legacy_session(bundled_ledger_dir("")).await;
}

async fn legacy_session(dir: TempDir) {
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
    assert!(result["instructions"].as_str().unwrap().starts_with("This server gives read-only access to"));

    let reply = post(&router, &[], &json!({"jsonrpc": "2.0", "method": "notifications/initialized"})).await;
    assert_eq!((reply.status.as_u16(), reply.text().as_str()), (202, ""));

    let reply = post(&router, &[("mcp-protocol-version", "2025-06-18")], &request(2, "ping", json!({}))).await;
    assert_eq!(reply.json(), json!({"jsonrpc": "2.0", "id": 2, "result": {}}));

    // tools/list: twelve tools with schemas
    let reply = post(&router, &[("mcp-protocol-version", "2025-06-18")], &request(3, "tools/list", json!({}))).await;
    let tools = reply.json()["result"]["tools"].clone();
    let names: Vec<&str> = tools.as_array().unwrap().iter().map(|tool| tool["name"].as_str().unwrap()).collect();
    assert_eq!(names, READ_TOOLS);
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
    // `limit` over the directive's `max_rows` is an error the model can correct
    let over = tool(&router, "query", json!({"bql": "SELECT account", "limit": 51})).await;
    assert_eq!(over["isError"], true);
    assert_eq!(over["content"][0]["text"], "the argument `limit` must be at most 50, this server's cap on rows");
    // an integral float is the integer it is; an empty query is refused before it reaches the host
    let float = structured(
        &router,
        "query",
        json!({"bql": "SELECT account GROUP BY account ORDER BY account", "limit": 2.0}),
    )
    .await;
    assert_eq!(float["row_count"], 2);
    assert_eq!(error_text(&router, "query", json!({"bql": "  "})).await, "the argument `bql` must not be empty");

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
    let backwards = tool(&router, "spending", json!({"from": "2024-03-31", "to": "2024-01-01"})).await;
    assert_eq!(backwards["isError"], true);
    assert!(backwards["content"][0]["text"]
        .as_str()
        .unwrap()
        .starts_with("the argument `from` (2024-03-31) is after `to` (2024-01-01)"));
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
    // an empty tag list is no filter
    let untagged = structured(&router, "transactions", json!({"tags": []})).await;
    assert_eq!(untagged["row_count"], 6);

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

    // arguments failing the tool's schema are tool execution errors naming the argument
    let wrong = tool(&router, "journal", json!({"account": 1})).await;
    assert_eq!(wrong["isError"], true);
    assert_eq!(wrong["content"][0]["text"], "the argument `account` must be a string");
    let missing = tool(&router, "journal", json!({})).await;
    assert_eq!(missing["content"][0]["text"], "the argument `account` is required");
    assert_eq!(
        error_text(&router, "payees", json!({"x": 1})).await,
        "unknown argument `x`; the arguments are none"
    );
    assert_eq!(
        error_text(&router, "spending", json!({"from": "2024-01-01", "to": "2024-01-31", "group_by": "week"})).await,
        "the argument `group_by` must be one of `account`, `month`, `currency`"
    );
    assert_eq!(
        error_text(&router, "net_worth", json!({"date": "2024-13-01"})).await,
        "the argument `date` must be a date written YYYY-MM-DD"
    );
    assert_eq!(
        error_text(&router, "journal", json!({"account": "Assets", "offset": -1})).await,
        "the argument `offset` must be an integer of at least 0"
    );
    assert_eq!(
        error_text(&router, "query", json!({"bql": "SELECT 1", "limit": "ten"})).await,
        "the argument `limit` must be an integer of at least 1"
    );

    // protocol errors: an unknown tool, a malformed call; the write tools are unknown without the grant
    let reply = post(&router, &[], &request(4, "tools/call", json!({"name": "nope"}))).await;
    assert_eq!(reply.status, 200);
    assert_eq!(reply.json()["error"]["code"], -32602);
    let reply = post(&router, &[], &request(5, "tools/call", json!({"arguments": {}}))).await;
    assert_eq!(reply.json()["error"]["code"], -32602);
    let reply = post(&router, &[], &request(5, "tools/call", json!({"name": "payees", "arguments": [1]}))).await;
    assert_eq!(reply.json()["error"]["code"], -32602);
    for name in WRITE_TOOLS {
        let reply = post(&router, &[], &request(5, "tools/call", json!({"name": name, "arguments": {}}))).await;
        assert_eq!(reply.json()["error"]["code"], -32602, "{name}");
        assert_eq!(reply.json()["error"]["message"], format!("Unknown tool: {name}"));
    }
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
    assert!(reply.json().get("id").is_none(), "a transport-level error has no id");
    // the request's own host counts only when it is this machine
    let reply = post(
        &router,
        &[("origin", "http://localhost:8000"), ("host", "localhost:8000")],
        &request(8, "ping", json!({})),
    )
    .await;
    assert_eq!(reply.status, 200, "{}", reply.text());
    let reply = post(
        &router,
        &[("origin", "http://ledger.lan:8000"), ("host", "ledger.lan:8000")],
        &request(8, "ping", json!({})),
    )
    .await;
    assert_eq!(reply.status, 403);
    let reply = post(&router, &[("origin", "https://ledger.example.com")], &request(8, "ping", json!({}))).await;
    assert_eq!(reply.status, 200, "an allowed origin");
    let reply = post(&router, &[], &json!([request(8, "ping", json!({}))])).await;
    assert_eq!((reply.status.as_u16(), reply.json()["error"]["code"].as_i64()), (400, Some(-32600)));
    assert!(reply.json().get("id").is_none());
    let reply = post(&router, &[], &json!({"jsonrpc": "2.0", "id": 1.5, "method": "ping"})).await;
    assert_eq!((reply.status.as_u16(), reply.json()["error"]["code"].as_i64()), (400, Some(-32600)));
    // params that are not an object: refused, with the id the request carried
    let reply = post(&router, &[], &json!({"jsonrpc": "2.0", "id": "p", "method": "ping", "params": []})).await;
    assert_eq!((reply.status.as_u16(), reply.json()["error"]["code"].as_i64()), (400, Some(-32600)));
    assert_eq!(reply.json()["id"], "p");
    let reply = post(&router, &[], &json!({"jsonrpc": "2.0", "id": 3, "result": {}})).await;
    assert_eq!(reply.status, 202);
    let reply = call(&router, Method::GET, "/api/plugins/zhang-mcp/nope", &[], None).await;
    assert_eq!(reply.status, 404);
}

#[tokio::test]
async fn a_modern_session_discovers_and_calls_with_headers_and_meta() {
    modern_session(ledger_dir("")).await;
}

/// the same session against `plugin "zhang:mcp"`
#[tokio::test]
async fn a_modern_session_against_the_bundled_plugin() {
    modern_session(bundled_ledger_dir("")).await;
}

async fn modern_session(dir: TempDir) {
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
    assert_eq!((result["ttlMs"].as_u64(), result["cacheScope"].as_str()), (Some(3_600_000), Some("private")));
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
    assert_eq!(result["cacheScope"], "public");

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
    // clientCapabilities must be an object, not merely present
    let bad_capabilities = post(
        &router,
        &[("mcp-protocol-version", "2026-07-28"), ("mcp-method", "tools/list")],
        &request(9, "tools/list", json!({"_meta": {META_VERSION: "2026-07-28", META_CAPABILITIES: "yes"}})),
    )
    .await;
    assert_eq!(
        (bad_capabilities.status.as_u16(), bad_capabilities.json()["error"]["code"].as_i64()),
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
    let dir = ledger_dir("");
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

// ---- writes ---------------------------------------------------------------------------------------------------------

/// the directive lines of a ledger that lists the write tools and grants both kinds
const GRANTED: &str = "  allow_writes: \"true\"\n  allowed_writes: \"transaction\"\n  allowed_writes: \"open\"\n";

/// a coffee at a new account, as the write session records it
fn coffee(token: Option<&str>) -> Value {
    let mut arguments = json!({
        "date": "2024-03-20",
        "payee": "Blue Bottle",
        "narration": "coffee",
        "postings": [
            {"account": "Expenses:Food:Coffee", "amount": "4.50", "currency": "CNY"},
            {"account": "Assets:Cash"}
        ],
    });
    if let Some(token) = token {
        arguments["token"] = json!(token);
    }
    arguments
}

#[tokio::test]
async fn without_the_grant_the_write_tools_are_hidden_and_with_allow_writes_alone_they_name_the_meta() {
    // read-only: `allow_writes` off (the default) hides the tools even with a grant
    let dir = ledger_dir("  allowed_writes: \"transaction\"\n  allowed_writes: \"open\"\n");
    let router = server(&dir, &Settings::default()).await;
    assert_eq!(tool_names(&router).await, READ_TOOLS);
    let reply = post(
        &router,
        &[],
        &request(4, "tools/call", json!({"name": "add_transaction", "arguments": coffee(Some("x"))})),
    )
    .await;
    assert_eq!(reply.json()["error"]["code"], -32602);

    // `allow_writes` on, no grant: listed, and every call says which meta line to add
    let dir = ledger_dir("  allow_writes: \"true\"\n");
    let router = server(&dir, &Settings::default()).await;
    assert_eq!(tool_names(&router).await, READ_TOOLS);
    let reply = post(
        &router,
        &[],
        &request(4, "tools/call", json!({"name": "preview_transaction", "arguments": coffee(None)})),
    )
    .await;
    assert_eq!(reply.json()["error"]["code"], -32602, "no grant at all: the tools stay hidden");

    // `allow_writes` on, only `open` granted: listed, and the transaction tools name the missing line
    let dir = ledger_dir("  allow_writes: \"true\"\n  allowed_writes: \"open\"\n");
    let router = server(&dir, &Settings::default()).await;
    let mut expected: Vec<&str> = READ_TOOLS.to_vec();
    expected.extend(WRITE_TOOLS);
    assert_eq!(tool_names(&router).await, expected);
    let denied = error_text(&router, "preview_transaction", coffee(None)).await;
    assert!(denied.contains("allowed_writes: \"transaction\""), "{denied}");
    let denied = error_text(&router, "add_transaction", coffee(Some("x"))).await;
    assert!(denied.contains("allowed_writes: \"transaction\""), "{denied}");
}

#[tokio::test]
async fn the_write_tools_refuse_floats_and_malformed_postings_before_the_host() {
    let dir = ledger_dir(GRANTED);
    let router = server(&dir, &Settings::default()).await;
    let mut float = coffee(None);
    float["postings"][0]["amount"] = json!(4.5);
    let text = error_text(&router, "preview_transaction", float).await;
    assert_eq!(
        text,
        "the argument `postings[1].amount` must be a decimal string such as \"4.50\", never a number: decimals stay exact"
    );
    let mut no_postings = coffee(None);
    no_postings["postings"] = json!([]);
    let text = error_text(&router, "preview_transaction", no_postings).await;
    assert!(text.starts_with("the argument `postings` must be a non-empty list"), "{text}");
    let text = error_text(
        &router,
        "preview_transaction",
        json!({"narration": "x", "postings": [{"account": "Assets:Cash"}]}),
    )
    .await;
    assert_eq!(text, "the argument `date` is required");
    let text = error_text(&router, "add_transaction", coffee(None)).await;
    assert!(text.starts_with("the argument `token` is required: call preview_transaction first"), "{text}");
    let text = error_text(
        &router,
        "open_account",
        json!({"date": "2024-03-20", "account": "Expenses:Food:Coffee", "commodities": "CNY"}),
    )
    .await;
    assert_eq!(text, "the argument `commodities` must be a list of strings");
}

/// the server as `zhang serve` runs it: the full router plus the reload listener, so that the reload a commit asks
/// for lands, and a handle to wait for it
async fn serving(dir: &TempDir, settings: &Settings) -> (Router, Arc<ReloadSender>) {
    let source: Arc<dyn DataSource> = Arc::new(LocalFileSystemDataSource::new(ZhangDataType {}));
    let root = dir.path().canonicalize().unwrap();
    let ledger = Ledger::load(root.clone(), "main.zhang".to_owned(), source.clone()).unwrap_or_else(|error| panic!("ledger should load: {error}"));
    let ledger = Arc::new(RwLock::new(ledger));
    let (sender, receiver) = tokio::sync::mpsc::channel(8);
    let reload_sender = Arc::new(ReloadSender::new(sender));
    start_reload_listener(ledger.clone(), Broadcaster::create(), reload_sender.clone(), receiver);
    let app = create_server_app(
        serve_config(&root, "main.zhang", source, settings),
        ledger,
        Broadcaster::create(),
        reload_sender.clone(),
    );
    (router_of(app).await, reload_sender)
}

#[tokio::test]
async fn a_write_session_previews_opens_adds_and_refuses_a_replay() {
    write_session(ledger_dir(GRANTED)).await;
}

/// the same session against `plugin "zhang:mcp"` with the same grant: the reloads after each commit load the bundled
/// copy again, and the grant comes from the directive as for a file
#[tokio::test]
async fn a_write_session_against_the_bundled_plugin() {
    write_session(bundled_ledger_dir(GRANTED)).await;
}

async fn write_session(dir: TempDir) {
    let (router, reload) = serving(&dir, &Settings::default()).await;

    // the instructions tell the model the write flow, and the catalogue has fifteen tools
    let reply = post(&router, &[], &request(1, "initialize", json!({"protocolVersion": "2025-11-25"}))).await;
    let instructions = reply.json()["result"]["instructions"].as_str().unwrap().to_owned();
    assert!(
        instructions.starts_with("This server gives access to the zhang ledger \"Home\""),
        "{instructions}"
    );
    assert!(
        instructions.contains("`preview_transaction` first") && instructions.contains("`open_account`"),
        "{instructions}"
    );
    let mut expected: Vec<&str> = READ_TOOLS.to_vec();
    expected.extend(WRITE_TOOLS);
    assert_eq!(tool_names(&router).await, expected);
    let reply = post(&router, &[("mcp-protocol-version", "2025-06-18")], &request(3, "tools/list", json!({}))).await;
    let tools = reply.json()["result"]["tools"].clone();
    let annotations = |name: &str| tools.as_array().unwrap().iter().find(|tool| tool["name"] == name).unwrap()["annotations"].clone();
    assert_eq!(
        annotations("preview_transaction"),
        json!({"readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false})
    );
    assert_eq!(
        annotations("add_transaction"),
        json!({"readOnlyHint": false, "destructiveHint": false, "idempotentHint": false, "openWorldHint": false})
    );
    assert_eq!(
        annotations("open_account"),
        json!({"readOnlyHint": false, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false})
    );

    // a preview against an account that does not exist: not an error of the tool, a finding for the model
    let preview = structured(&router, "preview_transaction", coffee(None)).await;
    assert_eq!(preview["ok"], false);
    assert!(preview["token"].is_null(), "{preview}");
    let kinds: Vec<&str> = preview["errors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|it| it["error_type"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["AccountDoesNotExist"]);
    assert_eq!(preview["errors"][0]["metas"]["account_name"], "Expenses:Food:Coffee");

    // a made-up token: refused before anything is written
    let refused = error_text(&router, "add_transaction", coffee(Some("made-up"))).await;
    assert!(refused.contains("preview_transaction"), "{refused}");

    // open the account explicitly, in one call
    let opened = structured(
        &router,
        "open_account",
        json!({"date": "2024-03-01", "account": "Expenses:Food:Coffee", "commodities": ["CNY"]}),
    )
    .await;
    assert_eq!((opened["recorded"].as_bool(), opened["already_open"].as_bool()), (Some(true), Some(false)));
    assert!(opened["text"].as_str().unwrap().contains("open Expenses:Food:Coffee CNY"), "{opened}");
    reload.reload_and_wait().await.unwrap();

    // the preview is clean now: the exact text, the file by the ledger's policy, a token
    let preview = structured(&router, "preview_transaction", coffee(None)).await;
    assert_eq!(preview["ok"], true, "{preview}");
    let text = preview["text"].as_str().unwrap().to_owned();
    assert!(text.contains("\"Blue Bottle\" \"coffee\" #mcp"), "{text}");
    assert!(text.contains("Expenses:Food:Coffee 4.50 CNY"), "{text}");
    assert_eq!(preview["file"], "data/2024/03.zhang");
    let token = preview["token"].as_str().expect("a token").to_owned();
    assert_eq!(preview["errors"], json!([]));

    // a model takes longer than a second between the preview and the commit, and a token embeds its expiry in
    // seconds: the token of the preview must still be the one the commit takes
    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
    // the same token with other arguments: refused by the host (the plan differs), nothing written
    let mut pricier = coffee(Some(&token));
    pricier["postings"][0]["amount"] = json!("5.00");
    let changed = error_text(&router, "add_transaction", pricier).await;
    assert!(
        changed.contains("the arguments differ") && changed.contains("preview_transaction again"),
        "{changed}"
    );
    assert!(
        !std::fs::read_to_string(dir.path().join("data/2024/03.zhang")).unwrap().contains("Blue Bottle"),
        "nothing was written"
    );

    // the commit: the file holds exactly the previewed text, the main file includes it
    let added = structured(&router, "add_transaction", coffee(Some(&token))).await;
    assert_eq!((added["recorded"].as_bool(), added["text"].as_str()), (Some(true), Some(text.as_str())));
    assert_eq!(added["file"], "data/2024/03.zhang");
    let written = std::fs::read_to_string(dir.path().join("data/2024/03.zhang")).unwrap();
    assert!(written.contains(&text), "the file holds the previewed text:\n{written}");
    assert_eq!(written.matches("Blue Bottle").count(), 1);
    let main = std::fs::read_to_string(dir.path().join("main.zhang")).unwrap();
    assert!(main.contains("include \"data/2024/03.zhang\""), "{main}");

    // the same token again: the ledger moved on, so the model is told it changed and to preview again (before the
    // reload lands the host refuses the stale ledger; after it the token is simply not the current preview's)
    let replay = error_text(&router, "add_transaction", coffee(Some(&token))).await;
    assert!(
        replay.contains("the ledger changed") && replay.contains("preview_transaction again"),
        "{replay}"
    );
    reload.reload_and_wait().await.unwrap();
    let replay = error_text(&router, "add_transaction", coffee(Some(&token))).await;
    assert!(
        replay.contains("the ledger changed") && replay.contains("preview_transaction again"),
        "{replay}"
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("data/2024/03.zhang")).unwrap(),
        written,
        "nothing else was written"
    );

    // the reloaded ledger shows the entry through the read tools, tagged
    let found = structured(&router, "transactions", json!({"keyword": "coffee"})).await;
    assert_eq!(found["row_count"], 1);
    assert_eq!(cell(&found, 0, "payee"), "Blue Bottle");
    assert_eq!(cell(&found, 0, "tags"), &json!(["mcp"]));
    let tagged = structured(&router, "transactions", json!({"tags": ["mcp"]})).await;
    assert_eq!(tagged["row_count"], 1);
    let journal = structured(&router, "journal", json!({"account": "Expenses:Food:Coffee"})).await;
    assert_eq!(journal["row_count"], 1);
    assert_eq!(number(cell(&journal, 0, "units")), 4.5);

    // opening an account that is open with the same commodities writes nothing
    let again = structured(
        &router,
        "open_account",
        json!({"date": "2024-03-01", "account": "Expenses:Food:Coffee", "commodities": ["CNY"]}),
    )
    .await;
    assert_eq!((again["recorded"].as_bool(), again["already_open"].as_bool()), (Some(false), Some(true)));
    assert_eq!(std::fs::read_to_string(dir.path().join("main.zhang")).unwrap(), main);
}
