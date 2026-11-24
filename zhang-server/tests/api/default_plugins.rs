//! The official default plugin `zhang:budget` through the server (RFC #698 §6, P1.5, P3.3): the server loads every
//! ledger with its bundled script, the only implementation of budgets. With plugins on (`ZHANG_PLUGINS` `read`, the
//! default, or `write`) the budget pages work as they always have, whatever the ledger says about plugins; with
//! `ZHANG_PLUGINS=off`, which stops every plugin, `zhang:budget` included, the budget tables and the budget pages'
//! queries answer that they are unavailable, saying why, the ledger has one `ExtensionUnavailable` error, and everything
//! else reads as before. `zhang:budget` has no options: what a directive naming it sets configures nothing.

use std::sync::Arc;

use axum::http::{Method, StatusCode};
use axum::Router;
use serde_json::{json, Value};
use tokio::sync::RwLock;
use zhang_ast::error::ErrorKind;
use zhang_core::clock::Clock;
use zhang_core::data_source::LocalFileSystemDataSource;
use zhang_core::data_type::text::ZhangDataType;
use zhang_core::extensions::SnapshotStatus;
use zhang_core::ledger::Ledger;
use zhang_core::plugin::PluginType;
use zhang_core::plugin_policy::{PluginLevel, PluginPolicy};
use zhang_server::routes::plugin_router::PluginSettings;
use zhang_testkit::http::{app, call, router_of, serve_config, Settings};
use zhang_testkit::ledger::Scratch;

const LEDGER: &str = "option \"timezone\" \"UTC\"
1970-01-01 open Assets:Bank
1970-01-01 open Expenses:Food
  budget: \"Food\"

2024-01-01 budget Food CNY
2024-01-01 budget-add Food 100 CNY
2024-01-05 * \"Cafe\" \"lunch\"
  Expenses:Food 30 CNY
  Assets:Bank -30 CNY
";

/// The router of a server on `LEDGER` with plugins at `level`, loaded as `zhang serve` loads a ledger.
async fn server(scratch: &Scratch, level: PluginLevel) -> Router {
    let policy = PluginPolicy {
        level,
        ..PluginPolicy::default()
    };
    let source = Arc::new(LocalFileSystemDataSource::new(ZhangDataType {}));
    let options = zhang_server::analysis::load_options(Arc::new(policy.clone()));
    let ledger = Ledger::load_with_options(scratch.dir().to_path_buf(), "main.zhang".to_owned(), source.clone(), Clock::System, options)
        .unwrap_or_else(|e| panic!("the ledger loads: {e}"));
    let settings = Settings {
        plugins: PluginSettings {
            policy,
            ..PluginSettings::default()
        },
        ..Settings::default()
    };
    router_of(app(serve_config(scratch.dir(), "main.zhang", source, &settings), Arc::new(RwLock::new(ledger)))).await
}

/// What the budget pages and a client of the budget tables send.
fn budget_requests() -> Vec<(Method, &'static str, Value)> {
    let month = json!({"name": "Food", "month": "2024-01-01"});
    vec![
        (Method::POST, "/api/query/builtins/budgets.month", json!({"params": {"month": "2024-01-01"}})),
        (Method::POST, "/api/query/builtins/budgets.budget", json!({"params": {"name": "Food"}})),
        (Method::POST, "/api/query/builtins/budgets.budget_month", json!({"params": month})),
        (Method::POST, "/api/query/builtins/budgets.events", json!({"params": month})),
        (Method::POST, "/api/query/builtins/budgets.postings", json!({"params": month})),
        (
            Method::POST,
            "/api/query/builtins",
            json!([{"name": "budgets.budget", "params": {"name": "Food"}}, {"name": "budgets.postings", "params": month}]),
        ),
        (
            Method::POST,
            "/api/query",
            json!({"query": "SELECT name, available FROM #budgets WHERE date = 2024-01-01"}),
        ),
        (Method::POST, "/api/query", json!({"query": "SELECT account, budgets FROM #postings"})),
    ]
}

/// What does not read budgets: a built-in query of the journals, the built-in queries' list, BQL over postings.
fn other_requests() -> Vec<(Method, &'static str, Value)> {
    vec![
        (
            Method::POST,
            "/api/query/builtins/postings.between",
            json!({"params": {"from": "2024-01-01", "to": "2024-12-31"}}),
        ),
        (Method::GET, "/api/query/builtins", Value::Null),
        (
            Method::POST,
            "/api/query",
            json!({"query": "SELECT account, sum(number) GROUP BY account ORDER BY account"}),
        ),
    ]
}

async fn answer(router: &Router, (method, uri, body): &(Method, &str, Value)) -> (StatusCode, Value) {
    let body = (!body.is_null()).then_some(body);
    let reply = call(router, method.clone(), uri, &[], body).await;
    (reply.status, reply.body)
}

/// The ledger's errors, as the errors page lists them: kind and metas.
async fn errors(router: &Router) -> Value {
    let request = (
        Method::POST,
        "/api/query/builtins/journals.errors",
        json!({"params": {"size": 100, "offset": 0}}),
    );
    let (status, body) = answer(router, &request).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let rows = body["data"]["rows"].as_array().unwrap();
    Value::Array(rows.iter().map(|row| json!([row[1], row[3], row[8]])).collect())
}

#[tokio::test]
async fn budgets_answer_alike_whether_plugins_read_or_write() {
    let scratch = Scratch::zhang(LEDGER);
    let read = server(&scratch, PluginLevel::Read).await;
    let write = server(&scratch, PluginLevel::Write).await;
    for request in budget_requests().iter().chain(&other_requests()) {
        let (status, body) = answer(&read, request).await;
        assert_eq!(status, StatusCode::OK, "{} {}: {body}", request.0, request.1);
        assert_eq!(answer(&write, request).await, (status, body), "{} {}", request.0, request.1);
    }
    let (_, month) = answer(&read, &budget_requests()[0]).await;
    assert_eq!(month["data"]["rows"][0][0], json!("Food"));
    assert_eq!(month["data"]["rows"][0][7], json!({"number": "70", "currency": "CNY"}), "{month}");
    assert_eq!(errors(&read).await, json!([]));
}

#[tokio::test]
async fn plugins_off_make_the_budget_tables_and_queries_unavailable_and_nothing_else() {
    let scratch = Scratch::zhang(LEDGER);
    let off = server(&scratch, PluginLevel::Off).await;
    let read = server(&scratch, PluginLevel::Read).await;
    let why = "plugin zhang:budget does not run: plugins are off (ZHANG_PLUGINS=off)";
    let expected = [
        format!("table #budgets is unavailable: {why}"),
        format!("table #budget_definitions is unavailable: {why}"),
        format!("table #budgets is unavailable: {why}"),
        format!("table #budget_events is unavailable: {why}"),
        format!("column postings.budgets is unavailable: {why}"),
        format!("table #budget_definitions is unavailable: {why}"),
        format!("table #budgets is unavailable: {why}"),
        format!("column postings.budgets is unavailable: {why}"),
    ];
    for (request, message) in budget_requests().iter().zip(&expected) {
        let (status, body) = answer(&off, request).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{} {}: {body}", request.0, request.1);
        assert_eq!(body["message"], json!(message), "{} {}", request.0, request.1);
    }
    // the rest of the ledger reads as with plugins on, the budget pages' queries listed with zhang's own
    for request in other_requests() {
        let (status, body) = answer(&off, &request).await;
        assert_eq!(status, StatusCode::OK, "{} {}: {body}", request.0, request.1);
        assert_eq!(answer(&read, &request).await, (status, body), "{} {}", request.0, request.1);
    }
    // one error says why, on the first directive that uses budgets
    assert_eq!(
        errors(&off).await,
        json!([[
            "ExtensionUnavailable",
            3,
            [{"key": "plugin", "value": "zhang:budget"}, {"key": "reason", "value": "plugins are off (ZHANG_PLUGINS=off)"}]
        ]])
    );
}

/// What a ledger may say about plugins before its budgets: `features.plugin` absent, off or on, and a `plugin` directive
/// naming `zhang:budget` or none.
fn headers() -> Vec<String> {
    let mut headers = vec![];
    for features in ["", "option \"features.plugin\" \"false\"\n", "option \"features.plugin\" \"true\"\n"] {
        for directive in ["", "plugin \"zhang:budget\"\n"] {
            headers.push(format!("{features}{directive}"));
        }
    }
    headers
}

/// `content` loaded as `zhang serve` loads it, with plugins at `level`.
fn served(dir: &std::path::Path, content: &str, level: PluginLevel) -> Ledger {
    std::fs::write(dir.join("main.zhang"), content).unwrap();
    let policy = PluginPolicy {
        level,
        ..PluginPolicy::default()
    };
    let options = zhang_server::analysis::load_options(Arc::new(policy));
    let source = Arc::new(LocalFileSystemDataSource::new(ZhangDataType {}));
    Ledger::load_with_options(dir.to_path_buf(), "main.zhang".to_owned(), source, Clock::System, options).unwrap_or_else(|e| panic!("{content}: {e}"))
}

/// Each error as its kind and the line it is on, the lines of `header` left out.
fn kinds_and_lines(ledger: &Ledger, header: &str) -> Vec<(ErrorKind, usize)> {
    let offset = header.lines().count();
    let mut errors = ledger
        .errors
        .iter()
        .map(|error| (error.error_type.clone(), error.span.as_ref().and_then(|span| span.line).unwrap_or(0) - offset))
        .collect::<Vec<_>>();
    errors.sort_by_key(|(_, line)| *line);
    errors
}

/// The server's `zhang:budget` is the bundled script, whatever the ledger says about plugins: registered first, as a
/// default plugin, it publishes the budgets with plugins `read` or `write`, and the budget error of the ledger is its
/// own. With plugins off nothing registers, and the ledger says once that its budgets are not interpreted.
#[test]
fn the_server_interprets_budgets_with_the_bundled_script_whatever_the_ledger_says() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = format!("{LEDGER}2024-01-06 budget-add Nope 1 CNY\n");
    for header in headers() {
        for level in [PluginLevel::Read, PluginLevel::Write] {
            let loaded = served(dir.path(), &format!("{header}{ledger}"), level);
            let (script, types) = &loaded.plugins.ordered[0];
            assert!(
                script.is_default() && script.is_budget_provider() && script.script().is_some(),
                "{level}: {header:?}"
            );
            assert_eq!(types, &[PluginType::Analyzer]);
            assert_eq!(loaded.plugins.ordered.len(), 1, "{level}: {header:?}");
            let status = &loaded
                .extensions
                .analyzer("zhang:budget")
                .expect("its tables")
                .snapshot
                .get()
                .expect("it ran")
                .status;
            assert_eq!(status, &SnapshotStatus::Ok, "{level}: {header:?}");
            assert_eq!(loaded.extensions.unavailable("zhang:budget"), None);
            assert_eq!(kinds_and_lines(&loaded, &header), [(ErrorKind::BudgetDoesNotExist, 11)], "{level}: {header:?}");
        }
        let off = served(dir.path(), &format!("{header}{ledger}"), PluginLevel::Off);
        assert!(off.plugins.ordered.is_empty(), "{header:?}");
        let mut expected = vec![(ErrorKind::ExtensionUnavailable, 3)];
        if header.contains("plugin \"zhang:budget\"") {
            expected.insert(0, (ErrorKind::PluginsDisabled, 0));
        }
        assert_eq!(kinds_and_lines(&off, &header), expected, "{header:?}");
    }
}

/// `zhang:budget` has no options with the script either (#869): each argument and meta of the directive naming it
/// configures nothing and says so with a `PluginError`, as does a second directive naming it, and the script runs as
/// without them (its timeout is the default one), its errors on the first directive naming it.
#[test]
fn the_budget_script_has_no_options() {
    let dir = tempfile::tempdir().unwrap();
    let header = "plugin \"zhang:budget\" \"monthly\"\n  timeout: \"5s\"\n  allowed_hosts: \"a.example\"\n  allowed_hosts: \"b.example\"\n  carry: \"zero\"\nplugin \"zhang:budget\"\n  timeout: \"9s\"\n";
    let ledger = served(dir.path(), &format!("{header}{LEDGER}"), PluginLevel::Read);
    let configured = ledger
        .errors
        .iter()
        .filter(|error| error.error_type == ErrorKind::PluginError)
        .map(|error| {
            assert_eq!(error.metas["plugin"], "zhang:budget");
            (error.span.as_ref().and_then(|span| span.line).unwrap(), error.metas["message"].clone())
        })
        .collect::<Vec<_>>();
    let says = |line: usize, message: &str| (line, message.to_owned());
    assert_eq!(
        configured,
        [
            says(1, "the argument \"monthly\" configures nothing: zhang:budget has no options. Remove it"),
            says(1, "the meta timeout configures nothing: zhang:budget has no options. Remove it"),
            says(1, "the meta allowed_hosts configures nothing: zhang:budget has no options. Remove it"),
            says(1, "the meta carry configures nothing: zhang:budget has no options. Remove it"),
            says(
                6,
                "this directive configures nothing: zhang:budget is configured by the directive at main.zhang:1. Remove this one"
            ),
        ]
    );
    // nothing else: the budgets are the script's, as without the directives
    assert_eq!(ledger.errors.len(), configured.len(), "{:?}", ledger.errors);
    let (script, _) = &ledger.plugins.ordered[0];
    assert!(script.is_default() && script.is_budget_provider() && script.script().is_some());
    assert_eq!(
        script.capabilities().timeout,
        zhang_core::plugin::capabilities::PluginCapabilities::default().timeout,
        "the directive's timeout does not apply"
    );
    assert!(script.capabilities().allowed_hosts.is_empty(), "nor its hosts");
    assert_eq!(script.span().line, Some(1), "its errors go on the first directive naming it");
    let status = &ledger
        .extensions
        .analyzer("zhang:budget")
        .expect("its tables")
        .snapshot
        .get()
        .expect("it ran")
        .status;
    assert_eq!(status, &SnapshotStatus::Ok);
}

/// The Settings page and `GET /api/plugins` list `zhang:budget`, the bundled script, as an analyzer: it is a plugin the
/// server runs for every ledger. With plugins off, it is not listed.
#[tokio::test]
async fn the_plugin_list_shows_the_budget_script() {
    let scratch = Scratch::zhang(LEDGER);
    let read = server(&scratch, PluginLevel::Read).await;
    let (status, body) = answer(&read, &(Method::GET, "/api/plugins", Value::Null)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let plugins = body["data"].as_array().expect("a list");
    assert_eq!(plugins.len(), 1, "{body}");
    assert_eq!(plugins[0]["name"], json!("zhang:budget"), "{body}");
    assert_eq!(plugins[0]["plugin_type"], json!(["Analyzer"]), "{body}");
    assert_eq!(plugins[0]["route"], Value::Null, "{body}");
    assert_eq!(plugins[0]["capabilities"]["allowed_hosts"], json!([]), "{body}");
    let off = server(&scratch, PluginLevel::Off).await;
    let (_, body) = answer(&off, &(Method::GET, "/api/plugins", Value::Null)).await;
    assert_eq!(body["data"], json!([]), "{body}");
}

/// The errors of the budget directives keep the order of the directives, each with its kind: a directive naming a
/// budget not defined yet, the second definition of a budget, a transfer from or to one not defined (moved from
/// zhang-core with the native budget checks, RFC #698 P5).
#[test]
fn the_budget_directives_report_in_their_order() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = served(
        dir.path(),
        "2024-01-01 budget-add food 10 CNY
2024-01-02 budget-close food
2024-01-03 budget-transfer absent other 1 CNY
2024-01-04 budget food CNY
2024-01-05 budget food USD
2024-01-06 budget-transfer food absent 1 CNY
2024-01-07 budget-add food 10 CNY
2024-01-08 budget-close food
2024-01-09 budget-close food
",
        PluginLevel::Read,
    );
    assert_eq!(
        kinds_and_lines(&ledger, ""),
        [
            (ErrorKind::BudgetDoesNotExist, 1),
            (ErrorKind::BudgetDoesNotExist, 2),
            (ErrorKind::BudgetDoesNotExist, 3),
            (ErrorKind::DefineDuplicatedBudget, 5),
            (ErrorKind::BudgetDoesNotExist, 6),
        ]
    );
}

/// A reload interprets the budgets of the files as they are now: a budget the files no longer define is not defined
/// any more.
#[test]
fn a_reload_interprets_the_budgets_anew() {
    let dir = tempfile::tempdir().unwrap();
    let mut ledger = served(dir.path(), "2024-01-01 budget food CNY\n2024-01-02 budget-add food 10 CNY\n", PluginLevel::Read);
    ledger.reload().unwrap();
    assert_eq!(kinds_and_lines(&ledger, ""), []);
    std::fs::write(dir.path().join("main.zhang"), "2024-01-02 budget-add food 10 CNY\n").unwrap();
    ledger.reload().unwrap();
    assert_eq!(kinds_and_lines(&ledger, ""), [(ErrorKind::BudgetDoesNotExist, 1)]);
}

/// The errors of a transaction's final booking come before the budget errors at it, which `zhang:budget` places with
/// the fold's, and both carry the transaction's span and id (moved from zhang-core with the native budget checks, RFC
/// #698 P5).
#[test]
fn final_validation_errors_precede_the_budget_errors_of_a_transaction() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = served(
        dir.path(),
        "1970-01-01 commodity CNY
1970-01-01 open Expenses:Food
  budget: missing
1970-01-01 open Income:I
2024-01-01 * \"unbalanced budget activity\"
  Expenses:Food 10 CNY
  Income:I -9 CNY
",
        PluginLevel::Read,
    );
    assert_eq!(
        ledger.errors.iter().map(|it| &it.error_type).collect::<Vec<_>>(),
        [&ErrorKind::UnbalancedTransaction, &ErrorKind::BudgetDoesNotExist]
    );
    let (id, _) = ledger.transactions()[0];
    assert_eq!(ledger.errors[0].metas["txn_id"], id.to_string());
    assert_eq!(ledger.errors[1].metas["budget_name"], "missing");
    assert_eq!(ledger.errors[1].metas["account_name"], "Expenses:Food");
    let span = &ledger.transaction_directive(&id).unwrap().span;
    assert!(ledger.errors.iter().all(|it| it.span.as_ref() == Some(span)));
}
