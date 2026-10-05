//! Local sequential latency measurements; this is not a server load test.
use std::hint::black_box;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{ensure, Result};
use chrono::{DateTime, Utc};
use extism::{CompiledPlugin, Function, Manifest, Plugin, PluginBuilder, UserData, Wasm, EXTISM_USER_MODULE, PTR};
use serde_json::{json, Value};
use zhang_core::clock::{Clock, LoadClock};
use zhang_core::data_source::{DataSource, LocalFileSystemDataSource};
use zhang_core::data_type::text::ZhangDataType;
use zhang_core::ledger::{Ledger, LedgerProcessContext};
use zhang_core::plugin::host::PluginHost;
use zhang_core::plugin::http::PluginRequest;
use zhang_core::plugin::router::{unavailable_host_functions, QueryFailure, RouterHost};
use zhang_query::{Params, Query};

const BQL: &str = "SELECT account, sum(number) AS total WHERE account ~ '^Expenses' GROUP BY account ORDER BY account";

fn clock() -> Clock {
    Clock::Fixed(DateTime::parse_from_rfc3339("2026-10-05T00:00:00Z").unwrap().with_timezone(&Utc))
}

fn builder(bytes: &[u8]) -> PluginBuilder<'static> {
    let host = PluginHost::new("performance", Default::default(), LoadClock::new(clock()), chrono_tz::Asia::Tokyo);
    PluginBuilder::new(Manifest::new([Wasm::data(bytes.to_vec())]).with_timeout(Duration::from_secs(30)))
        .with_functions(host.functions().into_iter().chain(unavailable_host_functions()))
        .with_wasi(true)
}

// A benchmark-only query binding for the pooled scenario. Production routing creates
// a fresh RouterCall each time; this fixed ledger binding is not a proposed pool design.
fn query_builder(bytes: &[u8], ledger: Arc<Ledger>) -> PluginBuilder<'static> {
    let functions = unavailable_host_functions().into_iter().filter(|function| function.name() != "zhang_query");
    let host = PluginHost::new("performance", Default::default(), LoadClock::new(clock()), chrono_tz::Asia::Tokyo);
    let query = Function::new(
        "zhang_query",
        [PTR],
        [PTR],
        UserData::new(QueryHost(ledger)),
        |plugin, inputs, outputs, data| {
            let bql: String = plugin.memory_get_val(&inputs[0])?;
            let result = data.get()?.lock().unwrap().query(&bql).map_err(|error| anyhow::anyhow!(error.message))?;
            let handle = plugin.memory_new(json!({"Ok": result}).to_string())?;
            outputs[0] = plugin.memory_to_val(handle);
            Ok(())
        },
    )
    .with_namespace(EXTISM_USER_MODULE);
    PluginBuilder::new(Manifest::new([Wasm::data(bytes.to_vec())]).with_timeout(Duration::from_secs(30)))
        .with_functions(host.functions().into_iter().chain(functions).chain([query]))
        .with_wasi(true)
}

fn stats(mut samples: Vec<f64>) -> Value {
    samples.sort_by(f64::total_cmp);
    let percentile = |p: f64| samples[((samples.len() as f64 * p).ceil() as usize).saturating_sub(1).min(samples.len() - 1)];
    json!({"samples": samples.len(), "min_ms": samples[0], "p50_ms": percentile(0.50),
        "p95_ms": percentile(0.95), "max_ms": samples[samples.len()-1]})
}

fn measure(count: usize, mut action: impl FnMut() -> Result<()>) -> Result<Value> {
    action()?;
    let mut samples = vec![];
    for _ in 0..count {
        let started = Instant::now();
        action()?;
        samples.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    Ok(stats(samples))
}

// Encode once, and validate every response after stopping the timer. Guest JSON parsing,
// source compilation, execution, JSON encoding and Extism's output copy remain timed.
fn calls(plugin: &mut Plugin, source: &str, input: &Value, expected: &Value, count: usize) -> Result<Value> {
    let request = json!({"source": source, "input": input}).to_string();
    let mut samples = vec![];
    for index in 0..count + 2 {
        let started = Instant::now();
        let output: String = plugin.call("run", request.as_str())?;
        let elapsed = started.elapsed().as_secs_f64() * 1000.0;
        ensure!(
            serde_json::from_str::<Value>(&output)? == *expected,
            "incorrect script result: {output}; expected {expected}"
        );
        if index >= 2 {
            samples.push(elapsed);
        }
    }
    let mut result = stats(samples);
    result["request_bytes"] = json!(request.len());
    Ok(result)
}

fn source(language: &str, py: &str, lua: &str) -> String {
    if language == "python" { py } else { lua }.to_owned()
}

fn ledger_text(count: usize) -> String {
    let mut text = "option \"timezone\" \"Asia/Tokyo\"\n2000-01-01 commodity CNY\n2000-01-01 open Assets:Cash\n2000-01-01 open Expenses:Food\n".to_owned();
    for index in 0..count {
        text.push_str(&format!(
            "2026-10-01 * \"Cafe\" \"purchase {index}\"\n  Assets:Cash -12.34 CNY\n  Expenses:Food 12.34 CNY\n"
        ));
    }
    text
}

fn batch(count: usize) -> Result<Value> {
    let dir = tempfile::tempdir()?;
    std::fs::write(dir.path().join("main.zhang"), ledger_text(count))?;
    let root = dir.path().canonicalize()?;
    let source = LocalFileSystemDataSource::new(ZhangDataType {});
    let loaded = source.load(root.display().to_string(), "main.zhang".into())?;
    Ok(serde_json::to_value(loaded.directives)?)
}

fn ledger(count: usize, plugin: Option<(&Path, &str)>) -> Result<Arc<Ledger>> {
    let dir = tempfile::tempdir()?;
    let mut text = ledger_text(count);
    if let Some((runtime, script)) = plugin {
        std::fs::write(dir.path().join("business.script"), script)?;
        text = format!(
            "option \"features.plugin\" \"true\"\nplugin \"{}\"\n  script: \"business.script\"\n  allowed_paths: \"business.script\"\n  script_source: {}\n  timeout: \"30s\"\n{}",
            runtime.canonicalize()?.display(),
            serde_json::to_string(script)?,
            text
        );
    }
    std::fs::write(dir.path().join("main.zhang"), text)?;
    let root = dir.path().canonicalize()?;
    let source = Arc::new(LocalFileSystemDataSource::new(ZhangDataType {}));
    let loaded = source.load(root.display().to_string(), "main.zhang".into())?;
    Ok(Arc::new(Ledger::process(LedgerProcessContext {
        directives: loaded.directives,
        entry: (root, "main.zhang".into()),
        visited_files: loaded.visited_files,
        data_source: source,
        clock: clock(),
    })?))
}

struct QueryHost(Arc<Ledger>);
impl RouterHost for QueryHost {
    fn query(&self, bql: &str) -> Result<Value, QueryFailure> {
        let fail = |error: zhang_query::QueryError| QueryFailure {
            message: error.message,
            line: error.line,
            column: error.column,
        };
        let result = Query::compile(bql).map_err(fail)?.execute(&self.0, &Params::new()).map_err(fail)?;
        Ok(
            json!({"columns": result.columns.iter().map(|c| json!({"name": c.name, "type": c.ty.name()})).collect::<Vec<_>>(),
            "rows": result.rows.iter().map(|row| row.iter().map(|cell| json!(cell.to_string())).collect::<Vec<_>>()).collect::<Vec<_>>()}),
        )
    }
}

fn main() -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let count = 100_000u64;
    let expected_sum = json!({"sum": count * (count + 1) / 2});
    let payloads = [(1, batch(1)?), (1000, batch(1000)?), (10000, batch(10000)?)];
    let native_ledger = ledger(10000, None)?;
    let native_host = QueryHost(native_ledger);
    let native_bql = measure(20, || {
        let result = native_host.query(BQL).map_err(|error| anyhow::anyhow!(error.message))?;
        ensure!(result["rows"] == json!([["Expenses:Food", "123400.00"]]), "unexpected report: {result}");
        Ok(())
    })?;
    let native_loop = measure(100, || {
        let mut total = 0u64;
        for i in 1..=black_box(count) {
            total = black_box(total) + i;
        }
        ensure!(black_box(total) == count * (count + 1) / 2);
        Ok(())
    })?;
    let mut json_roundtrips = vec![];
    for (transactions, payload) in &payloads {
        let input = payload.to_string();
        let result = measure(10, || {
            let value: Value = serde_json::from_str(black_box(&input))?;
            black_box(serde_json::to_vec(&value)?);
            Ok(())
        })?;
        json_roundtrips.push(json!({"transactions": transactions, "bytes": input.len(), "timing": result}));
    }
    let mut results = json!({"date": "2026-10-05", "cpu": "Apple M4", "extism": "1.30.0",
        "native_integer_loop_100k": native_loop, "native_bql_10k_transactions": native_bql,
        "native_json_roundtrips": json_roundtrips, "runtimes": []});
    for language in ["python", "lua"] {
        eprintln!("{language}: uncached WASM compilation");
        let runtime = root.join("artifacts").join(format!("{language}-runtime.wasm"));
        let bytes = std::fs::read(&runtime)?;
        let started = Instant::now();
        let compiled: CompiledPlugin = builder(&bytes).with_cache_disabled().compile()?;
        let compile_ms = started.elapsed().as_secs_f64() * 1000.0;
        eprintln!("{language}: uncached compilation {compile_ms:.1} ms; measuring new instances");
        let precompiled_new = measure(10, || {
            black_box(Plugin::new_from_compiled(&compiled)?);
            Ok(())
        })?;
        let default_new = measure(3, || {
            black_box(builder(&bytes).build()?);
            Ok(())
        })?;
        let mut plugin = Plugin::new_from_compiled(&compiled)?;
        let identity = source(
            language,
            "def run(value):\n    return value\n",
            "return {run = function(value) return value end}",
        );
        let request = json!({"source": &identity, "input": null}).to_string();
        let started = Instant::now();
        let first: String = plugin.call("run", request.as_str())?;
        let first_call_ms = started.elapsed().as_secs_f64() * 1000.0;
        ensure!(serde_json::from_str::<Value>(&first)?.is_null());
        let noop = calls(&mut plugin, &identity, &Value::Null, &Value::Null, 100)?;
        eprintln!("{language}: reused no-op {noop}; measuring host calls and loops");
        let now = source(
            language,
            "def run(n):\n    today = None\n    for _ in range(n):\n        today = zhang.now()['today']\n    return today\n",
            "return {run = function(n) local today; for i = 1,n do today = zhang.now().today end; return today end}",
        );
        let host_once = calls(&mut plugin, &now, &json!(1), &json!("2026-10-05"), 100)?;
        let host_100 = calls(&mut plugin, &now, &json!(100), &json!("2026-10-05"), 20)?;
        let sum = source(
            language,
            "def run(n):\n    total = 0\n    for i in range(1, n + 1):\n        total += i\n    return {'sum': total}\n",
            "return {run = function(n) local total = 0; for i = 1,n do total = total + i end; return {sum = total} end}",
        );
        let sum_100k = calls(&mut plugin, &sum, &json!(count), &expected_sum, 20)?;
        let mut batches = vec![];
        for (transactions, payload) in &payloads {
            eprintln!("{language}: whole-directive JSON round-trip, {transactions} transactions");
            let timing = calls(&mut plugin, &identity, payload, payload, if *transactions == 10000 { 5 } else { 10 })?;
            eprintln!("{language}: {transactions} transactions {timing}");
            batches.push(json!({"transactions": transactions, "timing": timing}));
        }
        eprintln!("{language}: current production router path (new instance per request)");
        // No process entry point: registering still uses the real plugin store and invokes
        // its default identity processor; routing uses production execute_as_router.
        let report_source = source(language,
            &format!("import json\ndef router(request):\n    return {{'status': 200, 'headers': {{'content-type': 'application/json'}}, 'body': json.dumps(zhang.query({BQL:?}))}}\n"),
            &format!("return {{router = function(request) return {{status = 200, headers = {{['content-type'] = 'application/json'}}, body = json.encode(zhang.query({BQL:?}))}} end}}"));
        let report_ledger = ledger(10000, Some((&runtime, &report_source)))?;
        let router = report_ledger.plugins.router(&format!("{language}-script-runtime")).unwrap();
        let request = PluginRequest::new("GET", "/", Vec::<(String, String)>::new(), Vec::<(String, String)>::new(), vec![]);
        let query_host = Arc::new(QueryHost(report_ledger.clone()));
        let current_router = measure(3, || {
            let response = router.execute_as_router(&request, &report_ledger, query_host.clone())?;
            ensure!(response.status() == 200);
            ensure!(serde_json::from_slice::<Value>(response.body())?["rows"] == json!([["Expenses:Food", "123400.00"]]));
            Ok(())
        })?;
        eprintln!("{language}: reused instance with benchmark-only BQL host binding");
        let mut pooled = query_builder(&bytes, report_ledger.clone()).build()?;
        let query_source = source(
            language,
            &format!("def run(request):\n    return zhang.query({BQL:?})\n"),
            &format!("return {{run = function(request) return zhang.query({BQL:?}) end}}"),
        );
        let pooled_report = calls(
            &mut pooled,
            &query_source,
            &Value::Null,
            &json!({"columns": [{"name": "account", "type": "str"}, {"name": "total", "type": "decimal"}],
                "rows": [["Expenses:Food", "123400.00"]]}),
            20,
        )?;
        let result = json!({"language": language, "uncached_wasm_compile_ms": compile_ms, "default_cache_new_instance": default_new,
            "precompiled_new_instance": precompiled_new, "first_call_ms": first_call_ms, "reused_noop": noop,
            "reused_host_once": host_once, "reused_host_100": host_100, "reused_integer_loop_100k": sum_100k,
            "reused_whole_directive_roundtrips": batches, "current_router_bql_10k_transactions": current_router,
            "reused_bql_10k_transactions_benchmark_binding": pooled_report});
        results["runtimes"].as_array_mut().unwrap().push(result);
        std::fs::write(root.join("artifacts/performance.json"), serde_json::to_string_pretty(&results)?)?;
    }
    println!("{}", serde_json::to_string_pretty(&results)?);
    Ok(())
}
