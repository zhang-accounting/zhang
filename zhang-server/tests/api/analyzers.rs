//! Analyzer plugins through the real load path (RFC #698 P1.4): what they publish, where their diagnostics go, and that
//! the ledger loads whatever goes wrong with one, with one error saying what.
//!
//! The analyzers are WAT modules written by [`analyzer`] for each test: a constant manifest, and an `analyze` export that
//! answers a constant output or fails the way a test asks. What an analyzer computes from what it reads is the Rust
//! analyzers' of `zhang-plugin-sdk` (`sdk_plugins`).

use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use zhang_ast::error::ErrorKind;
use zhang_core::analysis::LoadOptions;
use zhang_core::clock::Clock;
use zhang_core::data_source::LocalFileSystemDataSource;
use zhang_core::data_type::text::ZhangDataType;
use zhang_core::extensions::SnapshotStatus;
use zhang_core::inputs::ExtraInput;
use zhang_core::ledger::Ledger;
use zhang_core::outcome::Detail;
use zhang_core::plugin_policy::{PluginPolicy, MIB};
use zhang_query::{Catalog, ParamTypes, Params, Query};
use zhang_server::analysis::load_options;
use zhang_testkit::ledger::Scratch;

/// What an analyzer's `analyze` export does.
enum Analyze {
    /// answers this output
    Output(Value),
    /// reads the time, then answers this output
    ReadsTheClockThen(Value),
    /// traps
    Traps,
    /// runs until its time limit stops it
    Loops,
    /// grows its memory past any limit
    Hogs,
    /// answers this text, which is no output
    Answers(&'static str),
    /// is not exported
    Missing,
}

/// The WAT text of `name`, an analyzer whose `manifest` export answers `manifest` (none when `None`) and whose `analyze`
/// export does what `analyze` says.
fn analyzer(name: &str, manifest: Option<Value>, analyze: Analyze) -> String {
    let mut data = vec![];
    let mut segment = |text: &str| {
        let offset = data.iter().map(|(_, bytes): &(usize, Vec<u8>)| bytes.len() + 16).sum::<usize>() + 1024;
        data.push((offset, text.as_bytes().to_vec()));
        (offset, text.len())
    };
    let name = segment(&json!(name).to_string());
    let version = segment("\"0.1.0\"");
    let types = segment("[\"Analyzer\"]");
    let manifest = manifest.map(|manifest| segment(&manifest.to_string()));
    let output = match &analyze {
        Analyze::Output(output) | Analyze::ReadsTheClockThen(output) => Some(segment(&output.to_string())),
        Analyze::Answers(text) => Some(segment(text)),
        _ => None,
    };
    let pages = data.iter().map(|(offset, bytes)| offset + bytes.len()).max().unwrap_or(0) / 65536 + 1;
    let export = |name: &str, (offset, len): (usize, usize)| {
        format!("  (func (export \"{name}\") (result i32) (call $output_bytes (i32.const {offset}) (i32.const {len})) (i32.const 0))\n")
    };
    let mut wat = String::from(
        "(module\n\
         (import \"extism:host/env\" \"alloc\" (func $alloc (param i64) (result i64)))\n\
         (import \"extism:host/env\" \"store_u8\" (func $store_u8 (param i64 i32)))\n\
         (import \"extism:host/env\" \"output_set\" (func $output_set (param i64 i64)))\n\
         (import \"extism:host/user\" \"zhang_now\" (func $zhang_now (result i64)))\n",
    );
    wat.push_str(&format!("  (memory {pages})\n"));
    for (offset, bytes) in &data {
        let escaped = bytes.iter().map(|byte| format!("\\{byte:02x}")).collect::<String>();
        wat.push_str(&format!("  (data (i32.const {offset}) \"{escaped}\")\n"));
    }
    wat.push_str(
        "  (func $output_bytes (param $ptr i32) (param $len i32)\n\
           (local $block i64) (local $i i32)\n\
           (local.set $block (call $alloc (i64.extend_i32_u (local.get $len))))\n\
           (block $done (loop $copy\n\
             (br_if $done (i32.ge_u (local.get $i) (local.get $len)))\n\
             (call $store_u8 (i64.add (local.get $block) (i64.extend_i32_u (local.get $i))) (i32.load8_u (i32.add (local.get $ptr) (local.get $i))))\n\
             (local.set $i (i32.add (local.get $i) (i32.const 1)))\n\
             (br $copy)))\n\
           (call $output_set (local.get $block) (i64.extend_i32_u (local.get $len))))\n",
    );
    wat.push_str(&export("name", name));
    wat.push_str(&export("version", version));
    wat.push_str(&export("supported_type", types));
    if let Some(manifest) = manifest {
        wat.push_str(&export("manifest", manifest));
    }
    let body = match (&analyze, output) {
        (Analyze::Output(_) | Analyze::Answers(_), Some((offset, len))) => format!("(call $output_bytes (i32.const {offset}) (i32.const {len}))"),
        (Analyze::ReadsTheClockThen(_), Some((offset, len))) => {
            format!("(drop (call $zhang_now)) (call $output_bytes (i32.const {offset}) (i32.const {len}))")
        }
        (Analyze::Traps, _) => "unreachable".to_owned(),
        (Analyze::Loops, _) => "(loop $forever (br $forever))".to_owned(),
        (Analyze::Hogs, _) => "(if (i32.eq (memory.grow (i32.const 4096)) (i32.const -1)) (then (unreachable)))".to_owned(),
        _ => String::new(),
    };
    if !matches!(analyze, Analyze::Missing) {
        wat.push_str(&format!("  (func (export \"analyze\") (result i32) {body} (i32.const 0))\n"));
    }
    wat.push_str(")\n");
    wat
}

/// The manifest of an analyzer of the `custom "shop"` directives: a table of items, and two kinds of diagnostics.
fn shop_manifest() -> Value {
    json!({
        "analysis_abi": 1,
        "namespace": "shop",
        "consumes": ["custom:shop"],
        "diagnostics": [{"kind": "UnknownItem"}, {"kind": "Unpriced", "phase": "after_fold"}],
        "tables": [{"name": "items", "kind": "records", "columns": [{"name": "item", "type": "str"}, {"name": "price", "type": "amount"}]}]
    })
}

/// The ledger of the tests: the entries are numbered (`seq`) as the comments say, which the outputs name them by. The
/// two `budget-add`s of an undefined budget are errors of the fold, before and after the `custom "shop"` directive.
const LEDGER: &str = "option \"features.plugin\" \"true\"\noption \"operating_currency\" \"CNY\"\n\
1970-01-01 commodity CNY\n\
1970-01-01 open Assets:Cash\n\
1970-01-01 open Expenses:Food\n\
2024-01-01 budget-add Nope 10 CNY\n\
2024-01-02 custom \"shop\" \"apple\"\n\
2024-01-03 * \"lunch\"\n  Expenses:Food 10 CNY\n  Assets:Cash -10 CNY\n\
2024-01-04 budget-add Nope 10 CNY\n";

/// the `seq` of the entries of [`LEDGER`]: the commodity, the opens, the first add, the custom, the lunch, the second add
const CUSTOM: u32 = 4;
const LUNCH: u32 = 5;

/// A scratch ledger of [`LEDGER`] with the analyzers `modules` (file name, WAT text) declared after it, each with
/// `metas`.
fn scratch(modules: &[(&str, &str, &str)]) -> Scratch {
    let mut main = LEDGER.to_owned();
    let scratch = Scratch::with_main("main.zhang", "");
    for (file, wat, metas) in modules {
        scratch.write(&format!("plugins/{file}"), wat);
        main.push_str(&format!("plugin \"{}\"\n{metas}", scratch.dir().join("plugins").join(file).display()));
    }
    scratch.write("main.zhang", &main);
    scratch
}

fn load_with(scratch: &Scratch, options: LoadOptions, clock: Clock) -> Ledger {
    let source = Arc::new(LocalFileSystemDataSource::new(ZhangDataType {}));
    Ledger::load_with_options(scratch.dir().to_path_buf(), "main.zhang".to_owned(), source, clock, options)
        .unwrap_or_else(|error| panic!("the ledger loads: {error}"))
}

/// The ledger of `scratch`, loaded as the server loads one.
fn load(scratch: &Scratch) -> Ledger {
    load_with(scratch, load_options(Arc::new(PluginPolicy::default())), Clock::System)
}

/// The rows of `sql` on `ledger`, compiled against its catalog, each cell as text; or the error.
fn query(ledger: &Ledger, sql: &str) -> Result<Vec<Vec<String>>, String> {
    let query = Query::compile_in(&Catalog::of_ledger(ledger), sql, &ParamTypes::default()).map_err(|err| err.message)?;
    let result = query.execute(ledger, &Params::new()).map_err(|err| err.message)?;
    Ok(result.rows.iter().map(|row| row.iter().map(ToString::to_string).collect()).collect())
}

/// The errors of `ledger` in the order it reports them: kind, line and the metas but `txn_id`, sorted.
fn errors(ledger: &Ledger) -> Vec<String> {
    ledger
        .errors
        .iter()
        .map(|error| {
            let mut metas = error
                .metas
                .iter()
                .filter(|(key, _)| *key != "txn_id")
                .map(|(key, value)| format!("{key}={value}"))
                .collect::<Vec<_>>();
            metas.sort();
            let line = error.span.as_ref().and_then(|span| span.line).unwrap_or(0);
            format!("{} @{} {}", error.error_type, line, metas.join(" "))
        })
        .collect()
}

/// The errors of the kind `PluginAnalysisFailed`: their reason and message.
fn failures(ledger: &Ledger) -> Vec<(String, String)> {
    ledger
        .errors
        .iter()
        .filter(|error| error.error_type == ErrorKind::PluginAnalysisFailed)
        .map(|error| (error.metas["reason"].clone(), error.metas["message"].clone()))
        .collect()
}

fn shop_output() -> Value {
    json!({
        "diagnostics": [
            {"kind": "Unpriced", "target": {"seq": CUSTOM}, "metas": {"item": "apple"}},
            {"kind": "UnknownItem", "target": {"seq": CUSTOM}, "message": "apple is no item of the shop"},
            {"kind": "UnknownItem", "target": {"seq": LUNCH}, "message": "lunch names no item"},
            {"kind": "UnknownItem", "target": {"plugin": true}, "message": "the shop is closed"}
        ],
        "tables": {"items": [{"item": "apple", "price": {"number": "1.50", "currency": "CNY"}}]}
    })
}

#[test]
fn an_analyzers_diagnostics_go_where_the_folds_errors_of_their_directives_go() {
    let scratch = scratch(&[("shop.wat", &analyzer("shop", Some(shop_manifest()), Analyze::Output(shop_output())), "")]);
    let ledger = load(&scratch);
    // the entries are numbered as the outputs assume
    assert_eq!(
        query(&ledger, "SELECT seq, type FROM #entries WHERE type IN ('custom', 'transaction')").unwrap(),
        [vec![CUSTOM.to_string(), "custom".to_owned()], vec![LUNCH.to_string(), "transaction".to_owned()]]
    );
    let plugin_line = LEDGER.lines().count() + 1;
    assert_eq!(
        errors(&ledger),
        [
            "BudgetDoesNotExist @6 budget_name=Nope".to_owned(),
            // the fold's errors of the directives before it, its own, then those of the directives after it
            "PluginError @7 kind=UnknownItem message=apple is no item of the shop plugin=shop".to_owned(),
            "PluginError @8 kind=UnknownItem message=lunch names no item plugin=shop".to_owned(),
            "BudgetDoesNotExist @11 budget_name=Nope".to_owned(),
            // after the fold's errors of every directive, then those about the analyzer, on its directive
            "PluginError @7 item=apple kind=Unpriced message=Unpriced plugin=shop".to_owned(),
            format!("PluginError @{plugin_line} kind=UnknownItem message=the shop is closed plugin=shop"),
        ]
    );
    // a diagnostic at a transaction is one of its errors, which names it, as the fold's are
    let (index, lunch) = ledger.directives.iter().enumerate().find(|(_, it)| it.span.line == Some(8)).unwrap();
    let Detail::Transaction { id, errors } = &ledger.outcomes[index].detail else {
        panic!("a transaction: {lunch:?}")
    };
    assert_eq!(errors.len(), 1);
    assert_eq!(ledger.errors[errors[0]].metas["txn_id"], id.to_string());
    // its table
    assert_eq!(query(&ledger, "SELECT item, price FROM #shop_items").unwrap(), [["apple", "1.50 CNY"]]);
    assert_eq!(ledger.extensions.analyzers[0].snapshot.get().unwrap().status, SnapshotStatus::Ok);
    assert!(!ledger.extensions.analysing);
    // the query cache the analysis may have built is not the ledger's: the errors are read as placed
    assert_eq!(
        query(&ledger, "SELECT errors FROM #transactions").unwrap(),
        [["PluginError"]],
        "the transaction's errors as placed after the analysis"
    );
}

#[test]
fn an_analyzer_without_anything_to_read_does_not_run() {
    let scratch = scratch(&[("shop.wat", &analyzer("shop", Some(shop_manifest()), Analyze::Traps), "")]);
    scratch.write("main.zhang", &scratch.read("main.zhang").replace("2024-01-02 custom \"shop\" \"apple\"\n", ""));
    let ledger = load(&scratch);
    assert_eq!(failures(&ledger), []);
    assert_eq!(ledger.extensions.analyzers[0].snapshot.get().unwrap().status, SnapshotStatus::Skipped);
    assert_eq!(query(&ledger, "SELECT count(*) FROM #shop_items").unwrap(), [["0"]]);
}

/// A failure of an analysis: its name, the analyzer's manifest and analysis, the metas of its directive, the policy, and
/// the reason of its error.
type Case = (&'static str, Option<Value>, Analyze, &'static str, PluginPolicy, &'static str);

/// Whatever goes wrong with an analysis, the ledger loads, with one error on the analyzer's directive saying why, and
/// its tables are unavailable.
#[test]
fn every_failure_of_an_analysis_loads_the_ledger_with_one_error() {
    let output = |change: &dyn Fn(&mut Value)| {
        let mut output = shop_output();
        change(&mut output);
        Analyze::Output(output)
    };
    let cases: Vec<Case> = vec![
        ("trap", Some(shop_manifest()), Analyze::Traps, "", PluginPolicy::default(), "trap"),
        (
            "timeout",
            Some(shop_manifest()),
            Analyze::Loops,
            "  timeout: \"1\"\n",
            PluginPolicy::default(),
            "timeout",
        ),
        (
            "memory",
            Some(shop_manifest()),
            Analyze::Hogs,
            "",
            PluginPolicy {
                max_memory: 16 * MIB,
                ..PluginPolicy::default()
            },
            "memory",
        ),
        (
            "not json",
            Some(shop_manifest()),
            Analyze::Answers("{\"tables\": "),
            "",
            PluginPolicy::default(),
            "output",
        ),
        (
            "an undeclared kind",
            Some(shop_manifest()),
            output(&|o| o["diagnostics"][0]["kind"] = json!("Other")),
            "",
            PluginPolicy::default(),
            "output",
        ),
        (
            "a cell of another type",
            Some(shop_manifest()),
            output(&|o| o["tables"]["items"][0]["item"] = json!(3)),
            "",
            PluginPolicy::default(),
            "output",
        ),
        (
            "a target the ledger does not have",
            Some(shop_manifest()),
            output(&|o| o["diagnostics"][0]["target"] = json!({"seq": 99})),
            "",
            PluginPolicy::default(),
            "output",
        ),
        (
            "no analyze export",
            Some(shop_manifest()),
            Analyze::Missing,
            "",
            PluginPolicy::default(),
            "manifest",
        ),
        (
            "no manifest export",
            None,
            Analyze::Output(shop_output()),
            "",
            PluginPolicy::default(),
            "manifest",
        ),
        (
            "a manifest that does not hold",
            Some(json!({"analysis_abi": 1, "tables": [{"name": "Items", "kind": "records", "columns": [{"name": "item", "type": "str"}]}]})),
            Analyze::Output(shop_output()),
            "",
            PluginPolicy::default(),
            "manifest",
        ),
    ];
    for (case, manifest, analyze, metas, policy, reason) in cases {
        let scratch = scratch(&[
            ("shop.wat", &analyzer("shop", manifest, analyze), metas),
            // another analyzer, after it, is unaffected
            (
                "other.wat",
                &analyzer(
                    "other",
                    Some(
                        json!({"analysis_abi": 1, "consumes": ["custom:shop"], "tables": [{"name": "t", "kind": "records", "columns": [{"name": "c", "type": "str"}]}]}),
                    ),
                    Analyze::Output(json!({"tables": {"t": [{"c": "ok"}]}})),
                ),
                "",
            ),
        ]);
        let ledger = load_with(&scratch, load_options(Arc::new(policy)), Clock::System);
        let failed = failures(&ledger);
        assert_eq!(failed.len(), 1, "{case}: {failed:?}");
        assert_eq!(failed[0].0, reason, "{case}: {}", failed[0].1);
        let error = ledger.errors.iter().find(|it| it.error_type == ErrorKind::PluginAnalysisFailed).unwrap();
        assert_eq!(error.metas["plugin"], "shop", "{case}");
        assert_eq!(error.span.as_ref().unwrap().line, Some(LEDGER.lines().count() + 1), "{case}: on its directive");
        // no diagnostic of it, nothing of its tables
        assert!(
            !ledger.errors.iter().any(|it| it.error_type == ErrorKind::PluginError),
            "{case}: {:?}",
            errors(&ledger)
        );
        if reason != "manifest" {
            assert_eq!(
                query(&ledger, "SELECT count(*) FROM #shop_items").unwrap_err(),
                "table #shop_items is unavailable: plugin shop failed (see the ledger errors)",
                "{case}"
            );
        }
        assert_eq!(query(&ledger, "SELECT c FROM #other_t").unwrap(), [["ok"]], "{case}");
        // the rest of the ledger as without the analyzer
        assert_eq!(query(&ledger, "SELECT count(*)").unwrap(), [["2"]], "{case}");
    }
}

#[test]
fn of_two_analyzers_naming_a_table_alike_the_first_has_it_and_the_later_is_reported() {
    let module = |name: &str| {
        analyzer(
            name,
            Some(shop_manifest()),
            Analyze::Output(json!({"tables": {"items": [{"item": name, "price": null}]}})),
        )
    };
    let scratch = scratch(&[("first.wat", &module("first"), ""), ("second.wat", &module("second"), "")]);
    let ledger = load(&scratch);
    assert_eq!(
        failures(&ledger),
        [("manifest".to_owned(), "second: table 'shop_items' is already a table".to_owned())]
    );
    assert_eq!(query(&ledger, "SELECT item FROM #shop_items").unwrap(), [["first"]]);
    assert_eq!(ledger.extensions.analyzers.len(), 1, "the later one does not run");
}

#[test]
fn an_analyzer_does_not_run_without_an_analysis_host() {
    let scratch = scratch(&[("shop.wat", &analyzer("shop", Some(shop_manifest()), Analyze::Output(shop_output())), "")]);
    let ledger = load_with(&scratch, LoadOptions::default(), Clock::System);
    assert_eq!(
        failures(&ledger),
        [(
            "unavailable".to_owned(),
            "this zhang analyses no ledger: it runs without an analysis host".to_owned()
        )]
    );
    assert!(query(&ledger, "SELECT count(*) FROM #shop_items").is_err(), "no table of it");
}

/// The lifecycle of a snapshot: it belongs to its load, a reload analyses again (a failed snapshot is never reused), a
/// module or a setting changed counts at the next load, and an analysis that read the time makes the ledger depend on
/// the date.
#[test]
fn every_load_analyses_the_ledger_again() {
    let output = |item: &str| Analyze::Output(json!({"tables": {"items": [{"item": item, "price": null}]}}));
    let scratch = scratch(&[("shop.wat", &analyzer("shop", Some(shop_manifest()), output("first")), "")]);
    let module = scratch.dir().join("plugins/shop.wat");
    let mut ledger = load(&scratch);
    assert_eq!(query(&ledger, "SELECT item FROM #shop_items").unwrap(), [["first"]]);
    assert!(!ledger.extra_inputs.contains(&ExtraInput::Clock));

    // the module swapped
    std::fs::write(&module, analyzer("shop", Some(shop_manifest()), Analyze::Traps)).unwrap();
    ledger.reload().unwrap();
    assert_eq!(failures(&ledger).len(), 1);
    // and back: the failed snapshot is not reused
    std::fs::write(&module, analyzer("shop", Some(shop_manifest()), output("again"))).unwrap();
    ledger.reload().unwrap();
    assert_eq!(failures(&ledger), []);
    assert_eq!(query(&ledger, "SELECT item FROM #shop_items").unwrap(), [["again"]]);

    // a setting of its directive changed: a timeout that a looping analysis reaches
    std::fs::write(&module, analyzer("shop", Some(shop_manifest()), Analyze::Loops)).unwrap();
    let main = scratch.read("main.zhang");
    scratch.write("main.zhang", &format!("{main}  timeout: \"1\"\n"));
    ledger.reload().unwrap();
    assert_eq!(failures(&ledger)[0].0, "timeout");

    // an analysis that reads the time: the date is an input of the ledger, so the midnight reload analyses again
    std::fs::write(&module, analyzer("shop", Some(shop_manifest()), Analyze::ReadsTheClockThen(json!({})))).unwrap();
    let noon = Clock::Fixed(DateTime::parse_from_rfc3339("2024-03-16T04:00:00Z").unwrap().with_timezone(&Utc));
    let ledger = load_with(&scratch, load_options(Arc::new(PluginPolicy::default())), noon);
    assert_eq!(failures(&ledger), []);
    assert!(ledger.extra_inputs.contains(&ExtraInput::Clock));
}

/// A reload analyses the next ledger while the readers read the one served: every read of its table answers, from
/// either ledger, and none waits for an analysis.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_query_while_a_reload_analyses_reads_the_ledger_served() {
    let scratch = scratch(&[(
        "shop.wat",
        &analyzer(
            "shop",
            Some(shop_manifest()),
            Analyze::Output(json!({"tables": {"items": [{"item": "apple", "price": null}]}})),
        ),
        "",
    )]);
    let served = Arc::new(tokio::sync::RwLock::new(load(&scratch)));
    let reloads = {
        let served = served.clone();
        tokio::spawn(async move {
            for _ in 0..5 {
                let reload = served.read().await.reloader();
                let next = tokio::task::spawn_blocking(reload).await.unwrap().unwrap();
                *served.write().await = next;
            }
        })
    };
    let mut reads = 0;
    while !reloads.is_finished() || reads < 20 {
        let ledger = served.clone().read_owned().await;
        let rows = tokio::task::spawn_blocking(move || query(&ledger, "SELECT item FROM #shop_items"))
            .await
            .unwrap();
        assert_eq!(rows.unwrap(), [["apple"]]);
        reads += 1;
    }
    reloads.await.unwrap();
}
