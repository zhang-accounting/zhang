use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{ensure, Context, Result};
use chrono::{DateTime, Utc};
use extism::{Manifest, Plugin, Wasm};
use serde_json::{json, Value};
use zhang_core::clock::{Clock, LoadClock};
use zhang_core::data_source::{DataSource, LocalFileSystemDataSource};
use zhang_core::data_type::text::ZhangDataType;
use zhang_core::ledger::{Ledger, LedgerProcessContext};
use zhang_core::plugin::host::PluginHost;
use zhang_core::plugin::http::PluginRequest;
use zhang_core::plugin::router::{unavailable_host_functions, QueryFailure, RouterHost};
use zhang_query::{Params, Query};

fn clock() -> Clock {
    Clock::Fixed(DateTime::parse_from_rfc3339("2026-10-05T00:00:00Z").unwrap().with_timezone(&Utc))
}

fn instance(path: &Path, timeout: Duration) -> Result<(Plugin, PluginHost)> {
    let host = PluginHost::new("script-spike", Default::default(), LoadClock::new(clock()), chrono_tz::Asia::Tokyo);
    let functions = host.functions().into_iter().chain(unavailable_host_functions());
    let plugin = Plugin::new(Manifest::new([Wasm::data(std::fs::read(path)?)]).with_timeout(timeout), functions, true)?;
    Ok((plugin, host))
}

struct QueryHost(Arc<Ledger>);

impl RouterHost for QueryHost {
    fn query(&self, bql: &str) -> Result<Value, QueryFailure> {
        let failure = |error: zhang_query::QueryError| QueryFailure {
            message: error.message,
            line: error.line,
            column: error.column,
        };
        let result = Query::compile(bql).map_err(failure)?.execute(&self.0, &Params::new()).map_err(failure)?;
        // This spike's query uses strings and decimals only; preserve decimal precision.
        let rows: Vec<Vec<Value>> = result.rows.iter().map(|row| row.iter().map(|cell| json!(cell.to_string())).collect()).collect();
        Ok(json!({"columns": result.columns.iter().map(|c| json!({"name": c.name, "type": c.ty.name()})).collect::<Vec<_>>(), "rows": rows}))
    }
}

fn real_ledger(runtime: &Path, script: &str) -> Result<Arc<Ledger>> {
    let dir = tempfile::tempdir()?;
    std::fs::write(dir.path().join("business.script"), script)?;
    let runtime = runtime.canonicalize()?;
    let source_literal = serde_json::to_string(script)?;
    let content = format!(
        r#"option "features.plugin" "true"
option "timezone" "Asia/Tokyo"
plugin "{}"
  script: "business.script"
  script_source: {}
  allowed_paths: "business.script"

2000-01-01 commodity CNY
2000-01-01 open Assets:Cash
2000-01-01 open Expenses:Food
2026-10-01 * "Cafe" "lunch"
  Assets:Cash -12.34 CNY
  Expenses:Food 12.34 CNY
2026-10-02 * "Cafe" "coffee"
  Assets:Cash -0.01 CNY
  Expenses:Food 0.01 CNY
"#,
        runtime.display(),
        source_literal
    );
    std::fs::write(dir.path().join("main.zhang"), content)?;
    let root = dir.path().canonicalize()?;
    let source = Arc::new(LocalFileSystemDataSource::new(ZhangDataType {}));
    let loaded = source.load(root.to_string_lossy().to_string(), "main.zhang".to_owned())?;
    let ledger = Ledger::process(LedgerProcessContext {
        directives: loaded.directives,
        entry: (root, "main.zhang".to_owned()),
        visited_files: loaded.visited_files,
        data_source: source,
        clock: clock(),
    })?;
    Ok(Arc::new(ledger))
}

fn verify(language: &str, runtime: &Path, sources: [&str; 2], bad: &str, looping: &str, business: &str) -> Result<Value> {
    eprintln!("{language}: loading {}", runtime.display());
    let create_started = Instant::now();
    let (mut plugin, host) = instance(runtime, Duration::from_secs(5))?;
    let create_ms = create_started.elapsed().as_millis();
    let mut outputs = vec![];
    let mut timings = vec![];
    for (source, expected) in sources.into_iter().zip([3, 30]) {
        let request = json!({"source": source, "input": [1, 2]});
        let started = Instant::now();
        let output: String = plugin.call("run", request.to_string())?;
        timings.push(started.elapsed().as_micros());
        let output: Value = serde_json::from_str(&output)?;
        ensure!(output["sum"] == expected, "{language} returned {output}");
        ensure!(output["now"]["today"] == "2026-10-05", "wrong host time: {output}");
        outputs.push(output);
    }
    ensure!(host.take_errors().len() == 2, "script did not call the real Zhang error host");
    let identity = if language == "python" {
        "def run(value):\n    return value\n"
    } else {
        "return {run = function(value) return value end}"
    };
    let payload = json!({"object": {}, "array": [], "nullable": null, "values": [null, false, {}],
        "amount": "9007199254740993.123456789", "integer": 9007199254740993u64});
    let round_trip: String = plugin.call("run", json!({"source":identity, "input":payload}).to_string())?;
    ensure!(
        serde_json::from_str::<Value>(&round_trip)? == payload,
        "{language} changed JSON types or precision: {round_trip}"
    );
    eprintln!("{language}: both dynamic sources and real Zhang hosts passed");
    let exception = plugin
        .call::<_, String>("run", json!({"source":bad,"input":[]}).to_string())
        .unwrap_err()
        .to_string();
    ensure!(exception.contains("intentional"), "wrong script error: {exception}");
    // A failed script must not prevent the same interpreter instance from running another source.
    let recovered: String = plugin.call("run", json!({"source":sources[0],"input":[1,2]}).to_string())?;
    ensure!(
        serde_json::from_str::<Value>(&recovered)?["sum"] == 3,
        "{language} did not recover after a script error"
    );
    let (mut limited, _) = instance(runtime, Duration::from_millis(100))?;
    let timeout_started = Instant::now();
    let timeout = limited
        .call::<_, String>("run", json!({"source":looping,"input":[]}).to_string())
        .unwrap_err()
        .to_string();
    ensure!(timeout.contains("timeout"), "loop not stopped by Extism: {timeout}");
    let timeout_ms = timeout_started.elapsed().as_millis();
    eprintln!("{language}: script exception, recovery and timeout passed");
    let ledger = real_ledger(runtime, business)?;
    let errors = ledger.operations().errors()?;
    ensure!(errors.len() == 1, "expected the processor's one reported error, got {errors:?}");
    ensure!(
        ledger.extra_inputs.contains(&zhang_core::inputs::ExtraInput::File("business.script".into())),
        "script file was not tracked as a ledger input"
    );
    let name = format!("{language}-script-runtime");
    let router = ledger.plugins.router(&name).context("runtime was not registered as a router")?;
    let request = PluginRequest::new("GET", "/", HashMap::new(), HashMap::new(), vec![]);
    let response = router.execute_as_router(&request, &ledger, Arc::new(QueryHost(ledger.clone())))?;
    ensure!(response.status() == 200);
    let report: Value = serde_json::from_slice(response.body())?;
    ensure!(report["rows"] == json!([["Expenses:Food", "12.35"]]), "wrong BQL report: {report}");
    eprintln!("{language}: real processor, tracked source file and BQL router passed");
    Ok(
        json!({"language":language, "wasm_bytes": std::fs::metadata(runtime)?.len(), "instance_ms":create_ms,
        "call_us":timings, "dynamic_sources":outputs, "exception":exception, "timeout_ms":timeout_ms,
        "json_round_trip":true, "ledger_errors":errors.len(), "report":report}),
    )
}

fn main() -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let artifact_dir = std::env::args().nth(1).unwrap_or_else(|| root.join("artifacts").display().to_string());
    let mut results = vec![];
    for language in ["python", "lua"] {
        let suffix = if language == "python" { "py" } else { "lua" };
        let read = |name: &str| std::fs::read_to_string(root.join("samples").join(format!("{name}.{suffix}")));
        let a = read("sum")?;
        let b = read("sum-times-ten")?;
        let bad = read("error")?;
        let looping = read("loop")?;
        let business = read("business")?;
        results.push(verify(
            language,
            &Path::new(&artifact_dir).join(format!("{language}-runtime.wasm")),
            [&a, &b],
            &bad,
            &looping,
            &business,
        )?);
        std::fs::write(Path::new(&artifact_dir).join("results.json"), serde_json::to_string_pretty(&results)?)?;
    }
    println!("{}", serde_json::to_string_pretty(&results)?);
    Ok(())
}
