//! The example plugins of `zhang-plugin-sdk` (`zhang-plugin-sdk/examples/`), built for wasm32-unknown-unknown and
//! run through the real load path and the real router route: the SDK's end-to-end contract with this host.
//!
//! The plugins are built once per test run, with cargo, into their own target directory. Without the
//! wasm32-unknown-unknown target the tests are skipped locally with a message, and fail on CI (`CI` set).

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, OnceLock};

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use axum::routing::any;
use axum::Router;
use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use tempfile::TempDir;
use tokio::sync::RwLock;
use tower::ServiceExt;
use zhang_ast::error::ErrorKind;
use zhang_ast::{Directive, PluginType};
use zhang_core::clock::Clock;
use zhang_core::data_source::{DataSource, LocalFileSystemDataSource};
use zhang_core::data_type::text::ZhangDataType;
use zhang_core::inputs::ExtraInput;
use zhang_core::ledger::{Ledger, LedgerProcessContext};
use zhang_server::routes::plugin_router::{route_to_plugin, ROUTE};
use zhang_server::state::SharedLedger;

const WASM_TARGET: &str = "wasm32-unknown-unknown";

/// the example plugins, built for wasm32
struct Examples {
    guard: PathBuf,
    summary: PathBuf,
}

/// the example plugins, built on first use; `None` when the wasm32 target is missing and the tests are skipped
fn examples() -> Option<&'static Examples> {
    static EXAMPLES: OnceLock<Option<Examples>> = OnceLock::new();
    EXAMPLES.get_or_init(build_examples).as_ref()
}

fn build_examples() -> Option<Examples> {
    if !wasm_target_installed() {
        let reason = format!("the {WASM_TARGET} target is not installed; add it with `rustup target add {WASM_TARGET}`");
        if std::env::var_os("CI").is_some() {
            panic!("{reason}: CI must run the SDK plugin tests");
        }
        eprintln!("skipping the SDK plugin tests: {reason}");
        return None;
    }
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("zhang-server sits in the workspace root");
    let target_dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("sdk-plugins");
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| env!("CARGO").into());
    let output = Command::new(cargo)
        .current_dir(workspace)
        .args(["build", "--release", "--target", WASM_TARGET])
        .args(["-p", "zhang-plugin-example-guard", "-p", "zhang-plugin-example-summary"])
        .arg("--target-dir")
        .arg(&target_dir)
        .output()
        .expect("cargo should run");
    assert!(
        output.status.success(),
        "building the example plugins failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let built = target_dir.join(WASM_TARGET).join("release");
    Some(Examples {
        guard: built.join("zhang_plugin_example_guard.wasm"),
        summary: built.join("zhang_plugin_example_summary.wasm"),
    })
}

/// whether the toolchain cargo builds with has the standard library for wasm32-unknown-unknown
fn wasm_target_installed() -> bool {
    let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let Ok(output) = Command::new(rustc).args(["--print", "sysroot"]).output() else {
        return false;
    };
    let sysroot = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    output.status.success() && Path::new(&sysroot).join("lib/rustlib").join(WASM_TARGET).join("lib").is_dir()
}

/// 2024-03-16 12:00 in Asia/Shanghai, the ledger's timezone
fn noon() -> Clock {
    Clock::Fixed(DateTime::parse_from_rfc3339("2024-03-16T04:00:00Z").unwrap().with_timezone(&Utc))
}

/// a ledger directory holding `module` as `plugins/<file name>`, and `files` (path, content)
fn ledger_dir(module: &Path, files: &[(&str, &str)]) -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("plugins")).unwrap();
    std::fs::copy(module, dir.path().join("plugins").join(module.file_name().unwrap())).unwrap_or_else(|e| panic!("cannot copy {}: {e}", module.display()));
    for (path, content) in files {
        let path = dir.path().join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }
    dir
}

/// the `plugin` module path of `module` copied into `dir`: absolute, since a local ledger resolves a module against
/// the working directory
fn module_path(dir: &TempDir, module: &Path) -> String {
    dir.path()
        .canonicalize()
        .unwrap()
        .join("plugins")
        .join(module.file_name().unwrap())
        .display()
        .to_string()
}

/// load `content` as the main file of `dir` at noon on 2024-03-16
fn load(dir: &TempDir, content: &str) -> Ledger {
    std::fs::write(dir.path().join("main.zhang"), content).unwrap();
    let root = dir.path().canonicalize().unwrap();
    let source = Arc::new(LocalFileSystemDataSource::new(ZhangDataType {}));
    let loaded = source.load(root.to_string_lossy().to_string(), "main.zhang".to_owned()).unwrap();
    Ledger::process(LedgerProcessContext {
        directives: loaded.directives,
        entry: (root, "main.zhang".to_owned()),
        visited_files: loaded.visited_files,
        data_source: source,
        clock: noon(),
    })
    .unwrap_or_else(|e| panic!("ledger should load: {e}"))
}

const GUARD_LEDGER: &str = r#"option "features.plugin" "true"
option "timezone" "Asia/Shanghai"
plugin "{module}"
  threshold: "500 CNY"
  allowlist: "guard/allow.txt"
  allowed_paths: "guard"

1970-01-01 open Assets:Cash
1970-01-01 open Expenses:Food
1970-01-01 open Expenses:Rent

2024-01-01 custom "guard" "threshold" 100 CNY
2024-03-01 custom "guard" "threshold" "200 CNY"

2023-12-01 * "Landlord" "rent"
  Assets:Cash -600 CNY
  Expenses:Rent 600 CNY

2024-02-01 * "Cafe" "lunch"
  Assets:Cash -150 CNY
  Expenses:Food 150 CNY

2024-03-05 * "Cafe" "dinner"
  Assets:Cash -150 CNY
  Expenses:Food 150 CNY

2024-03-06 * "Cafe" "snack"
  threshold: "50 CNY"
  Assets:Cash -60 CNY
  Expenses:Food 60 CNY

2024-03-07 * "Market" "groceries"
  Assets:Cash -900 CNY
  Expenses:Food 900 CNY

2024-04-01 * "Cafe" "booked ahead"
  Assets:Cash -10 CNY
  Expenses:Food 10 CNY
"#;

/// the narration of every transaction with its `guard-id` meta
fn guard_ids(ledger: &Ledger) -> Vec<(String, String)> {
    ledger
        .directives
        .iter()
        .filter_map(|it| match &it.data {
            Directive::Transaction(txn) => Some((
                txn.narration.as_ref().unwrap().as_str().to_owned(),
                txn.meta.get_one("guard-id").map(|id| id.as_str().to_owned()).unwrap_or_default(),
            )),
            _ => None,
        })
        .collect()
}

#[test]
fn the_guard_processor_reads_config_custom_clock_and_files_and_reports_errors() {
    let Some(examples) = examples() else {
        return;
    };
    let dir = ledger_dir(&examples.guard, &[("guard/allow.txt", "Market\n")]);
    let content = GUARD_LEDGER.replace("{module}", &module_path(&dir, &examples.guard));
    let ledger = load(&dir, &content);

    let registered: Vec<_> = ledger
        .plugins
        .ordered
        .iter()
        .map(|(plugin, types)| (plugin.name.clone(), plugin.version.clone(), types.clone()))
        .collect();
    assert_eq!(registered, vec![("guard".to_owned(), "0.1.0".to_owned(), vec![PluginType::Processor])]);

    // each error is on its transaction, with the plugin's metas
    let store = ledger.store.read().unwrap();
    let errors: Vec<(String, HashMap<String, String>)> = store
        .errors
        .iter()
        .map(|error| {
            assert_eq!(error.error_type, ErrorKind::PluginError);
            let span = error.span.clone().expect("a plugin error has a span");
            (span.content.lines().next().unwrap().to_owned(), error.metas.clone())
        })
        .collect();
    drop(store);
    let summary: Vec<(&str, &str, &str)> = errors
        .iter()
        .map(|(line, metas)| (line.as_str(), metas["rule"].as_str(), metas["message"].as_str()))
        .collect();
    assert_eq!(
        summary,
        vec![
            // before the first `custom`: the plugin's meta, 500 CNY
            (
                "2023-12-01 * \"Landlord\" \"rent\"",
                "threshold",
                "Expenses:Rent 600 CNY is over the threshold of 500 CNY"
            ),
            // the `custom` of 2024-01-01
            (
                "2024-02-01 * \"Cafe\" \"lunch\"",
                "threshold",
                "Expenses:Food 150 CNY is over the threshold of 100 CNY"
            ),
            // the transaction's own meta wins over the `custom` of 2024-03-01
            (
                "2024-03-06 * \"Cafe\" \"snack\"",
                "threshold",
                "Expenses:Food 60 CNY is over the threshold of 50 CNY"
            ),
            // `zhang_now` is the load's clock, 2024-03-16 in Asia/Shanghai
            (
                "2024-04-01 * \"Cafe\" \"booked ahead\"",
                "future",
                "the transaction is dated 2024-04-01, after today (2024-03-16)"
            ),
        ],
        "dinner is under the 200 CNY of 2024-03-01, and Market is on the allowlist file"
    );
    assert!(errors.iter().all(|(_, metas)| metas["plugin"] == "guard"));
    assert_eq!(errors[0].1["threshold"], "500 CNY");

    // the date and the allowlist the plugin read make the ledger depend on them
    assert!(ledger.extra_inputs.contains(&ExtraInput::Clock));
    assert!(ledger.extra_inputs.contains(&ExtraInput::File("guard/allow.txt".into())));

    // every transaction gets an id from `rng_for`: distinct, and the same on the next load
    let ids = guard_ids(&ledger);
    assert_eq!(ids.len(), 6);
    assert!(ids.iter().all(|(_, id)| id.len() == 16 && id.chars().all(|c| c.is_ascii_hexdigit())), "{ids:?}");
    assert_eq!(ids.iter().map(|(_, id)| id).collect::<BTreeSet<_>>().len(), 6);
    assert_eq!(guard_ids(&load(&dir, &content)), ids);

    // an id depends on the transaction's own text only
    let edited = content.replace("2024-03-05 * \"Cafe\" \"dinner\"", "2024-03-05 * \"Cafe\" \"late dinner\"");
    let reloaded: HashMap<_, _> = guard_ids(&load(&dir, &edited)).into_iter().collect();
    let before: HashMap<_, _> = ids.into_iter().collect();
    assert_eq!(reloaded["lunch"], before["lunch"]);
    assert_ne!(reloaded["late dinner"], before["dinner"]);
}

const SUMMARY_LEDGER: &str = r#"option "features.plugin" "true"
option "title" "Home <books>"
option "timezone" "Asia/Shanghai"
plugin "{module}"

1970-01-01 open Assets:Cash
1970-01-01 open Expenses:Food
2024-01-02 * "lunch"
  Assets:Cash -10 CNY
  Expenses:Food 10 CNY
"#;

async fn send(app: &Router, request: Request<Body>) -> (StatusCode, String, String) {
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let content_type = response.headers()[header::CONTENT_TYPE].to_str().unwrap().to_owned();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, content_type, String::from_utf8(body.to_vec()).unwrap())
}

#[tokio::test]
async fn the_summary_router_queries_the_ledger_and_answers_html_and_json() {
    let Some(examples) = examples() else {
        return;
    };
    let dir = ledger_dir(&examples.summary, &[]);
    let ledger = load(&dir, &SUMMARY_LEDGER.replace("{module}", &module_path(&dir, &examples.summary)));
    let app = Router::new()
        .route(ROUTE, any(route_to_plugin))
        .with_state(SharedLedger(Arc::new(RwLock::new(ledger))));

    let (status, content_type, body) = send(&app, Request::get("/api/plugins/summary/balances").body(Body::empty()).unwrap()).await;
    assert_eq!((status, content_type.as_str()), (StatusCode::OK, "application/json"));
    assert_eq!(
        serde_json::from_str::<Value>(&body).unwrap(),
        json!([
            {"account": "Assets:Cash", "balance": "-10 CNY"},
            {"account": "Expenses:Food", "balance": "10 CNY"},
        ])
    );

    let (status, content_type, body) = send(&app, Request::get("/api/plugins/summary").body(Body::empty()).unwrap()).await;
    assert_eq!((status, content_type.as_str()), (StatusCode::OK, "text/html; charset=utf-8"));
    assert!(body.contains("<h1>Home &lt;books&gt;</h1>"), "{body}");
    // a router reads the ledger's clock afresh for every request
    assert!(body.contains("<p>Balances on 2024-03-16, in CNY</p>"), "{body}");
    assert!(body.contains("<tr><td>Assets:Cash</td><td>-10 CNY</td></tr>"), "{body}");

    let (status, _, body) = send(&app, Request::get("/api/plugins/summary/nope").body(Body::empty()).unwrap()).await;
    assert_eq!((status, body.as_str()), (StatusCode::NOT_FOUND, "no page at /nope"));
    let (status, _, _) = send(&app, Request::post("/api/plugins/summary/").body(Body::empty()).unwrap()).await;
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
}
