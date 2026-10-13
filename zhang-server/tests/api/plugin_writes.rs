//! The writes of router plugins over HTTP, on a ledger on the disk: `zhang_write_preview` and `zhang_write_commit`
//! driven by the `router_write.wat` fixture, which passes the request body to the host function and answers its raw
//! result. The contract: a plugin writes only what it declares and is granted; a preview is the API's preview; a
//! commit appends exactly the previewed text, once, and the ledger reloads; and the ledger's other writes and the
//! plugin's interleave without losing one another.

use std::path::Path;
use std::sync::Arc;

use axum::http::Method;
use axum::routing::{any, get, post};
use axum::Router;
use serde_json::{json, Value};
use tokio::sync::RwLock;
use zhang_core::ledger::Ledger;
use zhang_server::broadcast::Broadcaster;
use zhang_server::routes::plugin::plugin_list;
use zhang_server::routes::plugin_router::{route_to_plugin, ROUTE};
use zhang_server::routes::transaction::{create_new_transaction, get_journals, preview_new_transaction};
use zhang_server::state::{PluginRouteState, SharedLedger};
use zhang_server::{start_reload_listener, ReloadSender};
use zhang_testkit::fixtures::data_source_for;
use zhang_testkit::http::{json_request, send, Reply};
use zhang_testkit::ledger::Scratch;

const FIXTURE: &str = include_str!("../../../zhang-core/tests/plugins/router_write.wat");
const LUA_FIXTURE: &str = include_str!("../../../zhang-core/tests/plugins/router_write.lua");

const OPENS: &str = "1970-01-01 open Assets:Cash\n1970-01-01 open Expenses:Food USD\n";

/// a ledger with `main` holding `OPENS` and the fixture declared with `declared` (its `writes` config, a JSON array)
/// and the `allowed_writes` grants `granted`
fn ledger_with(main: &str, declared: &str, granted: &[&str]) -> Scratch {
    ledger_with_module(main, "router_write.wat", FIXTURE, declared, granted)
}

/// [`ledger_with`], declaring the Lua twin of the fixture, `router_write.lua`
fn lua_ledger_with(main: &str, declared: &str, granted: &[&str]) -> Scratch {
    ledger_with_module(main, "router_write.lua", LUA_FIXTURE, declared, granted)
}

fn ledger_with_module(main: &str, module: &str, source: &str, declared: &str, granted: &[&str]) -> Scratch {
    let scratch = Scratch::with_main(main, "");
    std::fs::write(scratch.dir().join(module), source).unwrap();
    let mut content = format!(
        "option \"features.plugin\" \"true\"\noption \"operating_currency\" \"USD\"\nplugin \"{}\"\n  writes: \"{}\"\n  timeout: \"20s\"\n",
        scratch.dir().join(module).display(),
        declared.replace('"', "\\\"")
    );
    for grant in granted {
        content.push_str(&format!("  allowed_writes: \"{grant}\"\n"));
    }
    content.push_str(OPENS);
    scratch.write(main, &content);
    scratch
}

/// the server over `scratch`: the plugin routes, the plugin list, the transaction writes and the journal, with the
/// reload listener running when `reloads` is set (otherwise a reload is asked for and never runs)
async fn server(scratch: &Scratch, reloads: bool) -> (Router, Arc<RwLock<Ledger>>, Arc<ReloadSender>) {
    let ledger = Ledger::load(scratch.dir().to_path_buf(), scratch.main().to_owned(), data_source_for(scratch.main()))
        .unwrap_or_else(|error| panic!("ledger should load: {error}"));
    let ledger = Arc::new(RwLock::new(ledger));
    let (sender, receiver) = tokio::sync::mpsc::channel(8);
    let reload_sender = Arc::new(ReloadSender::new(sender));
    if reloads {
        start_reload_listener(ledger.clone(), Broadcaster::create(), reload_sender.clone(), receiver);
    } else {
        std::mem::forget(receiver);
    }
    let router = Router::new()
        .route("/api/plugins", get(plugin_list))
        .route(ROUTE, any(route_to_plugin))
        .route("/api/transactions", post(create_new_transaction))
        .route("/api/transactions/preview", post(preview_new_transaction))
        .route("/api/journals", get(get_journals))
        .with_state(PluginRouteState::new(SharedLedger(ledger.clone()), reload_sender.clone()));
    (router, ledger, reload_sender)
}

/// the raw answer of `zhang_write_preview` for `directives`
async fn preview(router: &Router, directives: Value) -> Value {
    let reply = send(
        router,
        json_request(Method::POST, "/api/plugins/router-write/preview", &[], Some(&json!({"directives": directives}))),
    )
    .await;
    assert_eq!(reply.status, 200, "{}", reply.text());
    reply.json()
}

/// the raw answer of `zhang_write_commit` for `directives` with `token`
async fn commit(router: &Router, directives: Value, token: &Value) -> Value {
    let body = json!({"directives": directives, "token": token});
    let reply = send(router, json_request(Method::POST, "/api/plugins/router-write/commit", &[], Some(&body))).await;
    assert_eq!(reply.status, 200, "{}", reply.text());
    reply.json()
}

/// the `Ok` of a host function answer
fn ok(answer: &Value) -> &Value {
    assert!(answer["Ok"].is_object(), "{answer}");
    &answer["Ok"]
}

/// the kind and message of an `Err` answer
fn err(answer: &Value) -> (String, String) {
    assert!(answer["Err"].is_object(), "{answer}");
    (
        answer["Err"]["kind"].as_str().unwrap().to_owned(),
        answer["Err"]["message"].as_str().unwrap().to_owned(),
    )
}

fn coffee(datetime: &str, account: &str) -> Value {
    json!({"kind": "transaction", "datetime": datetime, "payee": "Blue Bottle", "narration": "coffee", "flag": "*",
        "postings": [{"account": account, "unit": {"number": "4.50", "commodity": "USD"}}, {"account": "Assets:Cash"}],
        "tags": ["mcp"], "links": [], "metas": []})
}

async fn journal_payees(router: &Router) -> Vec<String> {
    let reply: Reply = send(router, json_request(Method::GET, "/api/journals", &[], None)).await;
    assert_eq!(reply.status, 200, "{}", reply.text());
    reply.json()["data"]["records"]
        .as_array()
        .unwrap()
        .iter()
        .map(|record| record["payee"].as_str().unwrap_or_default().to_owned())
        .collect()
}

fn read(dir: &Path, relative: &str) -> String {
    std::fs::read_to_string(dir.join(relative)).unwrap_or_else(|error| panic!("{relative}: {error}"))
}

#[tokio::test]
async fn a_plugin_writes_only_what_it_declares_and_is_granted() {
    // declared, not granted: denied, and the plugin list shows both sides
    let scratch = ledger_with("main.zhang", r#"["transaction","open"]"#, &[]);
    let (router, _, _) = server(&scratch, false).await;
    let (kind, message) = err(&preview(&router, json!([coffee("2026-10-13", "Expenses:Food")])).await);
    assert_eq!(kind, "denied");
    assert!(message.contains("add `allowed_writes: \"transaction\"`"), "{message}");
    let plugins = send(&router, json_request(Method::GET, "/api/plugins", &[], None)).await.json();
    assert_eq!(plugins["data"][0]["writes_requested"], json!(["transaction", "open"]));
    assert_eq!(plugins["data"][0]["writes_granted"], json!([]));

    // granted a kind, not the other
    let scratch = ledger_with("main.zhang", r#"["transaction","open"]"#, &["open"]);
    let (router, _, _) = server(&scratch, false).await;
    let (kind, _) = err(&preview(&router, json!([coffee("2026-10-13", "Expenses:Food")])).await);
    assert_eq!(kind, "denied");
    let plugins = send(&router, json_request(Method::GET, "/api/plugins", &[], None)).await.json();
    assert_eq!(plugins["data"][0]["writes_granted"], json!(["open"]));

    // granted, not declared: the plugin is not written to write
    let scratch = ledger_with("main.zhang", "[]", &["transaction", "open"]);
    let (router, _, _) = server(&scratch, false).await;
    let (kind, message) = err(&preview(&router, json!([coffee("2026-10-13", "Expenses:Food")])).await);
    assert_eq!(
        (kind.as_str(), message.as_str()),
        ("unavailable", "plugin did not declare writes: its `capabilities` export lists no `writes`")
    );
}

#[tokio::test]
async fn a_preview_is_the_api_preview_and_a_commit_appends_exactly_it_then_reloads() {
    let scratch = ledger_with("main.zhang", r#"["transaction","open"]"#, &["transaction", "open"]);
    let (router, _, reload_sender) = server(&scratch, true).await;
    let batch = json!([coffee("2026-10-13T08:00:00", "Expenses:Food")]);

    let previewed = preview(&router, batch.clone()).await;
    let previewed = ok(&previewed);
    assert_eq!(previewed["ok"], true, "{previewed}");
    assert_eq!(previewed["file"], "data/2026/10.zhang");
    assert_eq!(previewed["generation"], 1);
    let entry = &previewed["entries"][0];
    assert_eq!(
        (&entry["index"], &entry["kind"], &entry["file"]),
        (&json!(0), &json!("transaction"), &json!("data/2026/10.zhang"))
    );
    assert_eq!(
        (&entry["field_errors"], &entry["unbalanced"], &entry["errors"]),
        (&json!([]), &json!([]), &json!([]))
    );
    // the API's preview of the same request gives the same text
    let mut request = batch[0].clone();
    request.as_object_mut().unwrap().remove("kind");
    let api = send(&router, json_request(Method::POST, "/api/transactions/preview", &[], Some(&request))).await;
    assert_eq!(api.status, 200, "{}", api.text());
    assert_eq!(entry["text"], api.json()["data"]["text"]);
    let text = entry["text"].as_str().unwrap().to_owned();
    assert_eq!(
        text,
        "2026-10-13 08:00:00 * \"Blue Bottle\" \"coffee\" #mcp\n  Expenses:Food 4.50 USD\n  Assets:Cash"
    );

    let committed = commit(&router, batch.clone(), &previewed["token"]).await;
    let committed = ok(&committed);
    assert_eq!(committed["file"], "data/2026/10.zhang");
    assert_eq!(committed["entries"], 1);
    assert_eq!(committed["generation"], 2);
    // the file holds exactly the previewed text, on a line of its own, and the main file includes it once
    assert_eq!(read(scratch.dir(), "data/2026/10.zhang"), format!("\n{text}\n"));
    let main = read(scratch.dir(), "main.zhang");
    assert_eq!(main.matches("include \"data/2026/10.zhang\"").count(), 1, "{main}");
    // what was appended: the text and the include line, each on a line of its own after a blank one
    assert_eq!(committed["bytes"], (text.len() + 2 + "include \"data/2026/10.zhang\"".len() + 2) as u64);
    // the reload the commit asked for lands (and the one waited for here, after it), and the journal shows the entry
    reload_sender.reload_and_wait().await.unwrap();
    assert_eq!(journal_payees(&router).await, vec!["Blue Bottle"]);
    // every reload moved the generation on, so the token is void: the ledger changed since the preview
    let (kind, message) = err(&commit(&router, batch, &previewed["token"]).await);
    assert_eq!(kind, "conflict");
    assert!(message.contains("it is at generation 4, the token is for generation 1"), "{message}");
    assert_eq!(ok(&preview(&router, json!([coffee("2026-10-14", "Expenses:Food")])).await)["generation"], 4);
}

#[tokio::test]
async fn a_commit_is_one_per_reload_and_a_stale_ledger_refuses_writes() {
    // no reload listener: a reload asked for never runs, so the ledger stays as the write left it
    let scratch = ledger_with("main.zhang", r#"["transaction","open"]"#, &["transaction", "open"]);
    let (router, _, _) = server(&scratch, false).await;
    let batch = json!([coffee("2026-10-13", "Expenses:Food")]);
    let first = preview(&router, batch.clone()).await;
    let token = first["Ok"]["token"].clone();
    assert!(token.is_string(), "{first}");
    ok(&commit(&router, batch.clone(), &token).await);

    // the same token again, and a new preview: both wait for the reload that follows a commit
    let (kind, message) = err(&commit(&router, batch.clone(), &token).await);
    assert_eq!(kind, "conflict");
    assert_eq!(message, "the ledger is reloading after a write: preview again once it has reloaded");
    let (kind, message) = err(&preview(&router, batch.clone()).await);
    assert_eq!(kind, "conflict");
    assert_eq!(message, "the ledger is reloading after a write: preview again once it has reloaded");
    assert_eq!(read(scratch.dir(), "data/2026/10.zhang").matches("Blue Bottle").count(), 1, "written once");

    // an API write leaves the ledger stale too, until its reload
    let scratch = ledger_with("main.zhang", r#"["transaction","open"]"#, &["transaction", "open"]);
    let (router, _, _) = server(&scratch, false).await;
    let mut request = coffee("2026-10-13", "Expenses:Food");
    request.as_object_mut().unwrap().remove("kind");
    let reply = send(&router, json_request(Method::POST, "/api/transactions", &[], Some(&request))).await;
    assert_eq!(reply.status, 200, "{}", reply.text());
    let (kind, _) = err(&preview(&router, batch).await);
    assert_eq!(kind, "conflict");
}

#[tokio::test]
async fn a_batch_opens_an_account_then_posts_to_it_and_skips_an_open_that_changes_nothing() {
    let scratch = ledger_with("main.zhang", r#"["transaction","open"]"#, &["transaction", "open"]);
    let (router, _, reload_sender) = server(&scratch, true).await;
    let open = json!({"kind": "open", "date": "2026-10-13", "account": "Expenses:Coffee", "commodities": ["USD"]});

    // the transaction alone posts to an account that is not open
    let alone = preview(&router, json!([coffee("2026-10-13", "Expenses:Coffee")])).await;
    assert_eq!(alone["Ok"]["ok"], false);
    assert_eq!(alone["Ok"]["entries"][0]["errors"][0]["error_type"], "AccountDoesNotExist");
    assert_eq!(alone["Ok"]["token"], Value::Null);
    // with the open before it, the batch is clean
    let batch = json!([open, coffee("2026-10-13", "Expenses:Coffee")]);
    let previewed = preview(&router, batch.clone()).await;
    let previewed = ok(&previewed);
    assert_eq!(previewed["ok"], true, "{previewed}");
    assert_eq!(previewed["entries"][0]["text"], "2026-10-13 open Expenses:Coffee USD");
    assert_eq!(previewed["entries"][0]["already_open"], false);
    assert_eq!(previewed["entries"][1]["errors"], json!([]));
    let committed = commit(&router, batch, &previewed["token"]).await;
    assert_eq!(ok(&committed)["entries"], 2);
    let file = read(scratch.dir(), "data/2026/10.zhang");
    assert!(
        file.starts_with("\n2026-10-13 open Expenses:Coffee USD\n\n2026-10-13 * \"Blue Bottle\""),
        "{file}"
    );
    reload_sender.reload_and_wait().await.unwrap();
    assert_eq!(journal_payees(&router).await, vec!["Blue Bottle"]);

    // the account is open now: the same open changes nothing, and is left out of a batch
    let again = preview(&router, json!([open])).await;
    let again = ok(&again);
    assert_eq!((&again["ok"], &again["token"]), (&json!(true), &Value::Null));
    assert_eq!(again["entries"][0]["already_open"], true);
    assert_eq!(again["entries"][0]["text"], Value::Null);
    let with_tea = preview(&router, json!([open, coffee("2026-10-14", "Expenses:Coffee")])).await;
    let with_tea = ok(&with_tea);
    assert_eq!(with_tea["ok"], true);
    assert!(with_tea["token"].is_string());
    let committed = commit(&router, json!([open, coffee("2026-10-14", "Expenses:Coffee")]), &with_tea["token"]).await;
    assert_eq!(ok(&committed)["entries"], 1, "only the transaction is written");
    assert_eq!(read(scratch.dir(), "data/2026/10.zhang").matches("open Expenses:Coffee").count(), 1);
    reload_sender.reload_and_wait().await.unwrap();
    // another commodity set is refused, at any date, and so is an open of a closed account (#803 review, P5)
    for (date, account, commodities) in [
        ("2026-10-13", "Expenses:Coffee", json!(["EUR"])),
        ("1969-01-01", "Expenses:Coffee", json!([])),
        ("2030-01-01", "Expenses:Food", json!(["EUR"])),
    ] {
        let other = preview(&router, json!([{"kind": "open", "date": date, "account": account, "commodities": commodities}])).await;
        assert_eq!(other["Ok"]["entries"][0]["field_errors"][0]["kind"], "already_open", "{other}");
    }
    // and a booking method given twice
    let twice = preview(
        &router,
        json!([{"kind": "open", "date": "2026-10-13", "account": "Assets:Broker", "booking_method": "FIFO", "metas": [{"key": "booking_method", "value": "LIFO"}]}]),
    )
    .await;
    assert_eq!(twice["Ok"]["entries"][0]["field_errors"][0]["kind"], "invalid_booking_method", "{twice}");
}

#[tokio::test]
async fn bounds_and_floats_are_refused_before_the_ledger_sees_them() {
    let scratch = ledger_with("main.zhang", r#"["transaction","open"]"#, &["transaction", "open"]);
    let (router, _, _) = server(&scratch, false).await;
    let open = json!({"kind": "open", "date": "2026-10-13", "account": "Expenses:Coffee"});
    let (kind, message) = err(&preview(&router, Value::Array(vec![open; 51])).await);
    assert_eq!(
        (kind.as_str(), message.as_str()),
        ("invalid", "the batch holds 51 directives, over the limit of 50")
    );
    let mut float = coffee("2026-10-13", "Expenses:Food");
    float["postings"][0]["unit"]["number"] = json!(4.5);
    let (kind, message) = err(&preview(&router, json!([float])).await);
    assert_eq!(kind, "invalid");
    assert!(message.starts_with("directives[0].postings[0].unit.number is the number 4.5"), "{message}");
}

#[tokio::test]
async fn a_beancount_ledger_gets_beancount_text() {
    let scratch = ledger_with("main.bean", r#"["transaction","open"]"#, &["transaction", "open"]);
    let (router, _, reload_sender) = server(&scratch, true).await;
    let batch = json!([
        {"kind": "open", "date": "2026-10-13", "account": "Expenses:Coffee", "commodities": ["USD"], "booking_method": "FIFO"},
        coffee("2026-10-13T08:00:00", "Expenses:Coffee"),
    ]);

    let previewed = preview(&router, batch.clone()).await;
    let previewed = ok(&previewed);
    assert_eq!(previewed["ok"], true, "{previewed}");
    assert_eq!(previewed["file"], "data/2026/10.bean");
    // the booking method is metadata, in both formats
    assert_eq!(
        previewed["entries"][0]["text"],
        "2026-10-13 open Expenses:Coffee USD\n  booking_method: \"FIFO\""
    );
    // beancount has no time on a transaction: it is metadata
    assert_eq!(
        previewed["entries"][1]["text"],
        "2026-10-13 * \"Blue Bottle\" \"coffee\" #mcp\n  time: \"08:00:00\"\n  Expenses:Coffee 4.50 USD\n  Assets:Cash"
    );
    ok(&commit(&router, batch, &previewed["token"]).await);
    assert!(read(scratch.dir(), "main.bean").contains("include \"data/2026/10.bean\""));
    reload_sender.reload_and_wait().await.unwrap();
    assert_eq!(journal_payees(&router).await, vec!["Blue Bottle"]);
}

#[tokio::test]
async fn api_writes_and_plugin_commits_interleave_without_losing_one_another() {
    let scratch = ledger_with("main.zhang", r#"["transaction","open"]"#, &["transaction", "open"]);
    let (router, _, reload_sender) = server(&scratch, true).await;
    let plugin_write = |day: u32| json!([coffee(&format!("2026-10-{day:02}"), "Expenses:Food")]);

    // the plugin writes the first entry of the month: the file and its include
    let previewed = preview(&router, plugin_write(1)).await;
    ok(&commit(&router, plugin_write(1), &previewed["Ok"]["token"]).await);
    reload_sender.reload_and_wait().await.unwrap();
    // the API writes the next one into the same file
    let mut request = coffee("2026-10-02", "Expenses:Food");
    request.as_object_mut().unwrap().remove("kind");
    request["payee"] = json!("Tea House");
    let reply = send(&router, json_request(Method::POST, "/api/transactions", &[], Some(&request))).await;
    assert_eq!(reply.status, 200, "{}", reply.text());
    reload_sender.reload_and_wait().await.unwrap();
    // and the plugin the third
    let previewed = preview(&router, plugin_write(3)).await;
    ok(&commit(&router, plugin_write(3), &previewed["Ok"]["token"]).await);
    reload_sender.reload_and_wait().await.unwrap();

    let file = read(scratch.dir(), "data/2026/10.zhang");
    assert_eq!(file.matches("2026-10-01 * \"Blue Bottle\"").count(), 1, "{file}");
    assert_eq!(file.matches("2026-10-02 * \"Tea House\"").count(), 1, "{file}");
    assert_eq!(file.matches("2026-10-03 * \"Blue Bottle\"").count(), 1, "{file}");
    let main = read(scratch.dir(), "main.zhang");
    assert_eq!(main.matches("include \"data/2026/10.zhang\"").count(), 1, "{main}");
    assert_eq!(journal_payees(&router).await, vec!["Blue Bottle", "Tea House", "Blue Bottle"]);
}

/// The Lua twin of the flow: the script answers the host's preview and commit tables as JSON, and the interpreter's
/// `capabilities` export declares the module table's `writes`.
#[tokio::test]
async fn a_lua_router_previews_and_commits_over_http() {
    let scratch = lua_ledger_with("main.zhang", r#"["transaction","open"]"#, &["transaction", "open"]);
    let (router, _, reload_sender) = server(&scratch, true).await;
    let plugins = send(&router, json_request(Method::GET, "/api/plugins", &[], None)).await.json();
    assert_eq!(plugins["data"][0]["writes_requested"], json!(["transaction", "open"]));
    assert_eq!(plugins["data"][0]["writes_granted"], json!(["transaction", "open"]));
    let batch = json!([
        {"kind": "open", "date": "2026-10-13", "account": "Expenses:Coffee", "commodities": ["USD"]},
        coffee("2026-10-13", "Expenses:Coffee"),
    ]);

    let reply = send(
        &router,
        json_request(Method::POST, "/api/plugins/router-write/preview", &[], Some(&json!({"directives": batch}))),
    )
    .await;
    assert_eq!(reply.status, 200, "{}", reply.text());
    let previewed = reply.json();
    assert_eq!(previewed["ok"], true, "{previewed}");
    assert_eq!(previewed["entries"][0]["text"], "2026-10-13 open Expenses:Coffee USD");
    let text = previewed["entries"][1]["text"].as_str().unwrap().to_owned();
    assert!(previewed["token"].is_string(), "{previewed}");

    let body = json!({"directives": batch, "token": previewed["token"]});
    let reply = send(&router, json_request(Method::POST, "/api/plugins/router-write/commit", &[], Some(&body))).await;
    assert_eq!(reply.status, 200, "{}", reply.text());
    let committed = reply.json();
    assert_eq!(committed["entries"], 2, "{committed}");
    assert_eq!(
        read(scratch.dir(), "data/2026/10.zhang"),
        format!("\n2026-10-13 open Expenses:Coffee USD\n\n{text}\n")
    );
    reload_sender.reload_and_wait().await.unwrap();
    assert_eq!(journal_payees(&router).await, vec!["Blue Bottle"]);

    // the same token again: the ledger changed
    let reply = send(&router, json_request(Method::POST, "/api/plugins/router-write/commit", &[], Some(&body))).await;
    let again = reply.json();
    assert!(again["error"].as_str().unwrap().starts_with("conflict: "), "{again}");
}

/// A write that fails after some of its files landed (#803 review, P2): the files written whole are named with their
/// entries, the rest is untouched, the ledger reloads and every token is void, so a client retries only what is not
/// written and nothing is recorded twice. The include of the file that failed dangles until the retry writes it.
#[tokio::test]
async fn a_write_that_fails_midway_names_what_landed_and_voids_the_batch() {
    let scratch = ledger_with("main.zhang", r#"["transaction","open"]"#, &["transaction", "open"]);
    let (router, _, reload_sender) = server(&scratch, true).await;
    // the November file cannot be written: a non-empty directory in its place
    std::fs::create_dir_all(scratch.dir().join("data/2026/11.zhang/blocker")).unwrap();
    let batch = json!([coffee("2026-10-13", "Expenses:Food"), coffee("2026-11-13", "Expenses:Food")]);
    let previewed = preview(&router, batch.clone()).await;
    let token = previewed["Ok"]["token"].clone();
    assert!(token.is_string(), "{previewed}");

    let (kind, message) = err(&commit(&router, batch.clone(), &token).await);
    assert_eq!(kind, "io");
    assert!(
        message.contains("Already written, do not send again: data/2026/10.zhang (entries 0)"),
        "{message}"
    );
    assert!(message.contains("(and the include in main.zhang)"), "{message}");
    assert!(message.contains("Not written: data/2026/11.zhang (entries 1)"), "{message}");
    assert_eq!(read(scratch.dir(), "data/2026/10.zhang").matches("Blue Bottle").count(), 1);
    // the batch cannot be sent again as it is: the ledger moved on
    let (kind, _) = err(&commit(&router, batch.clone(), &token).await);
    assert_eq!(kind, "conflict");
    // the include of November dangles, and the directory in its place even fails the reload: the ledger served stays
    // the one before, previews say so, and the write stays pending
    assert!(reload_sender.reload_and_wait().await.is_err());
    let (kind, message) = err(&preview(&router, batch.clone()).await);
    assert_eq!(kind, "conflict");
    assert!(message.contains("failed to reload after a write"), "{message}");
    // the owner fixes the disk: the reload lands with the dangling include reported, and only the entry that is not
    // written is sent again, into the file the include names already
    std::fs::remove_dir_all(scratch.dir().join("data/2026/11.zhang")).unwrap();
    reload_sender.reload_and_wait().await.unwrap();
    assert_eq!(journal_payees(&router).await, vec!["Blue Bottle"]);
    let rest = json!([coffee("2026-11-13", "Expenses:Food")]);
    let previewed = preview(&router, rest.clone()).await;
    ok(&commit(&router, rest, &previewed["Ok"]["token"]).await);
    reload_sender.reload_and_wait().await.unwrap();
    assert_eq!(journal_payees(&router).await.len(), 2);
    let main = read(scratch.dir(), "main.zhang");
    assert_eq!(main.matches("include \"data/2026/10.zhang\"").count(), 1, "{main}");
    assert_eq!(main.matches("include \"data/2026/11.zhang\"").count(), 1, "{main}");
}

/// The include of a new file is written before the file (#803 review, P3): when the main file cannot be written,
/// nothing is written, by a plugin or by `POST /api/transactions`, and the retry records the entry once.
#[cfg(unix)]
#[tokio::test]
async fn an_include_that_cannot_be_written_writes_nothing_and_a_retry_records_once() {
    use std::os::unix::fs::PermissionsExt;
    for via_api in [false, true] {
        let scratch = ledger_with("main.zhang", r#"["transaction","open"]"#, &["transaction", "open"]);
        let (router, _, reload_sender) = server(&scratch, true).await;
        let main = scratch.dir().join("main.zhang");
        let batch = json!([coffee("2026-10-13", "Expenses:Food")]);
        let mut request = batch[0].clone();
        request.as_object_mut().unwrap().remove("kind");
        std::fs::set_permissions(&main, std::fs::Permissions::from_mode(0o000)).unwrap();
        if std::fs::read(&main).is_ok() {
            eprintln!("skipping: this user reads a file without permissions (root)");
            return;
        }

        if via_api {
            let reply = send(&router, json_request(Method::POST, "/api/transactions", &[], Some(&request))).await;
            assert_ne!(reply.status, 200, "{}", reply.text());
            assert!(reply.text().contains("main.zhang"), "{}", reply.text());
        } else {
            let previewed = preview(&router, batch.clone()).await;
            let (kind, message) = err(&commit(&router, batch.clone(), &previewed["Ok"]["token"]).await);
            assert_eq!(kind, "io");
            assert!(message.contains("Nothing was written"), "{message}");
        }
        assert!(
            !scratch.dir().join("data/2026/10.zhang").exists(),
            "via_api={via_api}: the data file is not written without its include"
        );

        std::fs::set_permissions(&main, std::fs::Permissions::from_mode(0o644)).unwrap();
        let _ = reload_sender.reload_and_wait().await;
        if via_api {
            let reply = send(&router, json_request(Method::POST, "/api/transactions", &[], Some(&request))).await;
            assert_eq!(reply.status, 200, "{}", reply.text());
        } else {
            let previewed = preview(&router, batch.clone()).await;
            ok(&commit(&router, batch.clone(), &previewed["Ok"]["token"]).await);
        }
        reload_sender.reload_and_wait().await.unwrap();
        assert_eq!(journal_payees(&router).await, vec!["Blue Bottle"], "via_api={via_api}");
        assert_eq!(read(scratch.dir(), "main.zhang").matches("include").count(), 1, "via_api={via_api}");
    }
}

/// A write keeps a symbolic link a link and a private file private (#803 review, P6), on the API path as on a
/// plugin's: the real file behind the link gets the include.
#[cfg(unix)]
#[tokio::test]
async fn a_write_keeps_a_symlinked_main_file_and_a_private_files_mode() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    std::fs::create_dir_all(root.join("real")).unwrap();
    std::fs::create_dir_all(root.join("ledger/data/2026")).unwrap();
    std::fs::write(root.join("real/main.zhang"), format!("option \"operating_currency\" \"USD\"\n{OPENS}")).unwrap();
    std::os::unix::fs::symlink(root.join("real/main.zhang"), root.join("ledger/main.zhang")).unwrap();
    let private = root.join("ledger/data/2026/10.zhang");
    std::fs::write(&private, "").unwrap();
    std::fs::set_permissions(&private, std::fs::Permissions::from_mode(0o600)).unwrap();
    let ledger = Ledger::load(root.join("ledger"), "main.zhang".to_owned(), data_source_for("main.zhang")).unwrap();
    let ledger = Arc::new(RwLock::new(ledger));
    let (sender, receiver) = tokio::sync::mpsc::channel(8);
    let reload_sender = Arc::new(ReloadSender::new(sender));
    start_reload_listener(ledger.clone(), Broadcaster::create(), reload_sender.clone(), receiver);
    let router = Router::new()
        .route("/api/transactions", post(create_new_transaction))
        .with_state(PluginRouteState::new(SharedLedger(ledger.clone()), reload_sender.clone()));
    let mut request = coffee("2026-10-13", "Expenses:Food");
    request.as_object_mut().unwrap().remove("kind");

    let reply = send(&router, json_request(Method::POST, "/api/transactions", &[], Some(&request))).await;

    assert_eq!(reply.status, 200, "{}", reply.text());
    assert!(
        std::fs::symlink_metadata(root.join("ledger/main.zhang")).unwrap().file_type().is_symlink(),
        "the main file stays a link"
    );
    assert!(
        std::fs::read_to_string(root.join("real/main.zhang"))
            .unwrap()
            .contains("include \"data/2026/10.zhang\""),
        "the real file got the include"
    );
    assert_eq!(
        std::fs::metadata(&private).unwrap().permissions().mode() & 0o777,
        0o600,
        "the data file stays private"
    );
    assert!(std::fs::read_to_string(&private).unwrap().contains("Blue Bottle"));
}

/// An API write queued behind a plugin's commit reloads first (#803 review, P7): the commit marks the write pending
/// before the plugin answers, so the write never plans a second include of the file the commit created.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_api_write_queued_behind_a_plugin_commit_sees_the_commit() {
    let script = r#"
local M = { name = "router-write", version = "0.1.0", writes = { "transaction" } }
function M.router(request)
  local batch = json.decode(request.body).directives
  local preview = zhang.write.preview(batch)
  local committed = zhang.write.commit(batch, preview.token)
  local n = 0
  for i = 1, 40000000 do n = n + i end
  return { json = committed }
end
return M
"#;
    let scratch = ledger_with_module("main.zhang", "router_write.lua", script, r#"["transaction"]"#, &["transaction"]);
    let (router, _, reload_sender) = server(&scratch, true).await;
    let batch = json!([coffee("2026-10-13", "Expenses:Food")]);
    let plugin_router = router.clone();
    let plugin = tokio::spawn(async move {
        send(
            &plugin_router,
            json_request(Method::POST, "/api/plugins/router-write/x", &[], Some(&json!({"directives": batch}))),
        )
        .await
        .status
    });
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    let mut request = coffee("2026-10-14", "Expenses:Food");
    request.as_object_mut().unwrap().remove("kind");
    request["payee"] = json!("Tea House");
    let api_router = router.clone();
    let api = tokio::spawn(async move {
        send(&api_router, json_request(Method::POST, "/api/transactions", &[], Some(&request)))
            .await
            .status
    });

    assert_eq!(plugin.await.unwrap(), 200);
    assert_eq!(api.await.unwrap(), 200);
    reload_sender.reload_and_wait().await.unwrap();
    let main = read(scratch.dir(), "main.zhang");
    assert_eq!(main.matches("include \"data/2026/10.zhang\"").count(), 1, "{main}");
    let mut payees = journal_payees(&router).await;
    payees.sort();
    assert_eq!(payees, vec!["Blue Bottle", "Tea House"]);
}
