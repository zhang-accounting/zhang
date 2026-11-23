//! Lua analyzers through the real load path and the server's analysis host (RFC #698 P1.6): a script's manifest is
//! fields of its module table, its `analyze` reads the ledger with queries and `zhang.convert` and adds up with
//! `zhang.decimal`, and what it returns is published as the SDK analyzers' output is: a record table, a series table, a
//! label column and diagnostics. What goes wrong in a script fails its analysis only, with one error.

use std::sync::Arc;

use zhang_ast::error::ErrorKind;
use zhang_core::analysis::LoadOptions;
use zhang_core::clock::Clock;
use zhang_core::data_source::LocalFileSystemDataSource;
use zhang_core::data_type::text::ZhangDataType;
use zhang_core::extensions::SnapshotStatus;
use zhang_core::inputs::ExtraInput;
use zhang_core::ledger::Ledger;
use zhang_core::plugin_policy::PluginPolicy;
use zhang_query::{Catalog, ParamTypes, Params, Query};
use zhang_server::analysis::load_options;
use zhang_testkit::ledger::Scratch;

/// The analyzer of the tests, `envelopes` (see its comment).
const ENVELOPES: &str = include_str!("../fixtures/plugins/envelopes.lua");

/// The envelopes Food (100.00 CNY, overspent in March by a dinner in USD and groceries), Travel and Pets, which no
/// account spends from.
const LEDGER: &str = r#"option "features.plugin" "true"
option "operating_currency" "CNY"
1970-01-01 commodity CNY
1970-01-01 commodity USD
1970-01-01 open Assets:Cash
1970-01-01 open Expenses:Food
  envelope: "Food"
1970-01-01 open Expenses:Travel
  envelope: "Travel"
2024-01-01 price USD 7.10 CNY
2024-01-01 custom "envelope" "Food" "100.00"
  note: "groceries"
2024-01-01 custom "envelope" "Travel" "1000"
2024-01-01 custom "envelope" "Pets" "50"
2024-01-05 * "lunch"
  Expenses:Food 25.50 CNY
  Assets:Cash
2024-02-10 * "dinner"
  Expenses:Food 10 USD
  Assets:Cash
2024-02-11 * "train"
  Expenses:Travel 120 CNY
  Assets:Cash
2024-03-02 * "groceries"
  Expenses:Food 5.00 CNY
  Assets:Cash
"#;

/// A scratch ledger of [`LEDGER`] with the script `script` declared after it as `plugins/<file>`.
fn scratch(file: &str, script: &str) -> Scratch {
    let scratch = Scratch::with_main("main.zhang", "");
    scratch.write(&format!("plugins/{file}"), script);
    let plugin = scratch.dir().join("plugins").join(file);
    scratch.write("main.zhang", &format!("{LEDGER}plugin \"{}\"\n", plugin.display()));
    scratch
}

/// The ledger of `scratch`, loaded as the server loads one.
fn load(scratch: &Scratch) -> Ledger {
    let source = Arc::new(LocalFileSystemDataSource::new(ZhangDataType {}));
    let options: LoadOptions = load_options(Arc::new(PluginPolicy::default()));
    Ledger::load_with_options(scratch.dir().to_path_buf(), "main.zhang".to_owned(), source, Clock::System, options)
        .unwrap_or_else(|error| panic!("the ledger loads: {error}"))
}

/// The rows of `sql` on `ledger`, compiled against its catalog, each cell as text.
fn query(ledger: &Ledger, sql: &str) -> Vec<Vec<String>> {
    let query = Query::compile_in(&Catalog::of_ledger(ledger), sql, &ParamTypes::default()).unwrap_or_else(|err| panic!("{sql}: {}", err.message));
    let result = query.execute(ledger, &Params::new()).unwrap_or_else(|err| panic!("{sql}: {}", err.message));
    result.rows.iter().map(|row| row.iter().map(ToString::to_string).collect()).collect()
}

fn texts(rows: &[&[&str]]) -> Vec<Vec<String>> {
    rows.iter().map(|row| row.iter().map(|it| (*it).to_owned()).collect()).collect()
}

/// The line of the first line of [`LEDGER`] that starts with `start`, from 1.
fn line(start: &str) -> usize {
    LEDGER.lines().position(|line| line.starts_with(start)).unwrap_or_else(|| panic!("{start}")) + 1
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

#[test]
fn a_lua_analyzer_publishes_a_record_table_a_series_table_and_a_label_column() {
    let ledger = load(&scratch("envelopes.lua", ENVELOPES));
    let snapshot = ledger.extensions.analyzers[0].snapshot.get().unwrap();
    assert_eq!(snapshot.status, SnapshotStatus::Ok, "{:?}", ledger.errors);

    // the record table: every envelope by name, its amount written as zhang.decimal wrote it, and its metadata
    assert_eq!(
        query(&ledger, "SELECT name, funded, since, meta('note') FROM #env_funds"),
        texts(&[
            &["Food", "100.00 CNY", "2024-01-01", "groceries"],
            &["Pets", "50 CNY", "2024-01-01", "NULL"],
            &["Travel", "1000 CNY", "2024-01-01", "NULL"],
        ])
    );
    // the series: the months with something, exact sums at the larger scale, 10 USD converted at 7.10
    assert_eq!(
        query(&ledger, "SELECT name, date, year, month, spent, left FROM #env_months ORDER BY name, date"),
        texts(&[
            &["Food", "2024-01-01", "2024", "1", "25.50 CNY", "74.50 CNY"],
            &["Food", "2024-02-01", "2024", "2", "71.00 CNY", "3.50 CNY"],
            &["Food", "2024-03-01", "2024", "3", "5.00 CNY", "-1.50 CNY"],
            &["Travel", "2024-01-01", "2024", "1", "0 CNY", "1000 CNY"],
            &["Travel", "2024-02-01", "2024", "2", "120 CNY", "880 CNY"],
        ])
    );
    // the label column: the envelope of each posting, by its predicate with each envelope's parameters
    assert_eq!(
        query(
            &ledger,
            "SELECT narration, env_envelope FROM #postings WHERE account ~ '^Expenses' ORDER BY date"
        ),
        texts(&[&["lunch", "Food"], &["dinner", "Food"], &["train", "Travel"], &["groceries", "Food"]])
    );
    assert_eq!(query(&ledger, "SELECT count(*) WHERE 'Food' IN env_envelope"), texts(&[&["3"]]));

    // the diagnostics, at the directives they name, in the ledger's order
    assert_eq!(
        errors(&ledger),
        [
            format!(
                "PluginError @{} envelope=Pets kind=UnknownEnvelope message=no account spends from the envelope Pets plugin=envelopes",
                line("2024-01-01 custom \"envelope\" \"Pets\"")
            ),
            format!(
                "PluginError @{} envelope=Food kind=Overspent left=-1.50 message=Overspent plugin=envelopes",
                line("2024-03-02 * \"groceries\"")
            ),
        ]
    );
    // it never read the time: the ledger does not depend on the date
    assert!(!ledger.extra_inputs.contains(&ExtraInput::Clock), "{:?}", ledger.extra_inputs);
}

/// What goes wrong in a script fails its analysis only: the ledger loads with one error saying what, and its tables
/// answer that they are unavailable.
#[test]
fn a_lua_analysis_that_fails_loads_the_ledger_with_one_error() {
    let raised = ENVELOPES
        .lines()
        .position(|line| line.contains("local currency = zhang.ledger_info()"))
        .unwrap()
        + 1;
    let cases = [
        // an error the script raises, with its line
        (
            ENVELOPES.replace("local currency = zhang.ledger_info().operating_currency", "error(\"no envelopes today\")"),
            "trap",
            format!("envelopes.lua:{raised}: no envelopes today"),
        ),
        // no analyze function
        (
            ENVELOPES.replace("function M.analyze(input)", "function M.other(input)"),
            "trap",
            "envelopes.lua: the module table has no `analyze` function".to_owned(),
        ),
        // a float where the output takes a decimal: refused by the host, naming the cell
        (
            ENVELOPES.replace("funded = { number = envelope.funded,", "funded = { number = 0.5,"),
            "output",
            "funded: a decimal is a string".to_owned(),
        ),
        // a query that fails, uncaught
        (
            ENVELOPES.replace("FROM #accounts WHERE", "FROM #nowhere WHERE"),
            "trap",
            "query error".to_owned(),
        ),
        // a multiplication
        (
            ENVELOPES.replace("left = left - amount.number", "left = left * amount.number"),
            "trap",
            "zhang.decimal has no multiplication".to_owned(),
        ),
    ];
    for (script, reason, message) in cases {
        let ledger = load(&scratch("envelopes.lua", &script));
        let failed = ledger
            .errors
            .iter()
            .filter(|error| error.error_type == ErrorKind::PluginAnalysisFailed)
            .map(|error| (error.metas["reason"].clone(), error.metas["message"].clone()))
            .collect::<Vec<_>>();
        assert_eq!(failed.len(), 1, "{reason} {message}: {:?}", ledger.errors);
        assert_eq!(failed[0].0, reason, "{message}: {}", failed[0].1);
        assert!(failed[0].1.contains(&message), "{}", failed[0].1);
        assert_eq!(
            ledger.extensions.analyzers[0].snapshot.get().unwrap().status,
            SnapshotStatus::Failed { reason: reason.to_owned() }
        );
        let unavailable = Query::compile_in(&Catalog::of_ledger(&ledger), "SELECT * FROM #env_funds", &ParamTypes::default())
            .unwrap()
            .execute(&ledger, &Params::new())
            .unwrap_err();
        assert!(unavailable.message.contains("unavailable"), "{}", unavailable.message);
    }
}

/// The shorter forms of the output and the last transaction of the input (RFC #698 G1): a record table's rows listed,
/// amounts as pairs, a series fact that leaves out what it carries and takes it from the fact before it, and the
/// ledger's last transaction, which is the latest date's last one, as `#transactions` orders them.
#[test]
fn a_lua_analyzer_reads_the_last_transaction_and_answers_in_the_shorter_forms() {
    const SHORT: &str = r#"
local M = {
  name = "short", version = "0.1.0", types = { "Analyzer" }, analysis_abi = 1, namespace = "short",
  consumes = { "custom:envelope" }, open_meta = {},
  tables = {
    { name = "funds", kind = "records", order = "published",
      columns = { { name = "name", type = "str" }, { name = "funded", type = "amount" },
                  { name = "last", type = "date" }, { name = "last_seq", type = "int" } } },
    { name = "months", kind = "series", partition = { "name" },
      period = { column = "date", unit = "month", derive = {} },
      columns = { { name = "name", type = "str", fill = "carry" }, { name = "date", type = "date" },
                  { name = "note", type = "str", fill = "carry" }, { name = "spent", type = "amount", fill = "zero" } } },
  },
}
function M.analyze(input)
  local last, funds = input.last_transaction, {}
  for _, record in ipairs(input.records) do
    local values = record.fields.values
    -- the cells in the order of the columns, then the metadata
    funds[#funds + 1] = { values[1], { values[2], "CNY" }, last.date, last.seq, record.meta }
  end
  local rows = {
    { date = "2024-01-01", note = "first", spent = { "25.50", "CNY" }, ["$meta"] = { note = { "january" } } },
    -- the note and the metadata left out: those of January
    { date = "2024-02-01", spent = { "71.00", "CNY" } },
    { date = "2024-03-01", note = "third", spent = { "5.00", "CNY" }, ["$meta"] = { note = { "march" } } },
  }
  return { tables = { funds = funds },
           series = { months = { partitions = { { key = { name = "Food" }, ["until"] = "2024-03-01", rows = rows } } } } }
end
return M
"#;
    let ledger = load(&scratch("short.lua", SHORT));
    let snapshot = ledger.extensions.analyzers[0].snapshot.get().unwrap();
    assert_eq!(snapshot.status, SnapshotStatus::Ok, "{:?}", ledger.errors);
    let last = query(&ledger, "SELECT date, seq FROM #transactions ORDER BY date DESC, seq DESC LIMIT 1");
    assert_eq!(last[0][0], "2024-03-02");
    assert_eq!(
        query(&ledger, "SELECT name, funded, last, last_seq, meta('note') FROM #short_funds"),
        texts(&[
            &["Food", "100.00 CNY", "2024-03-02", &last[0][1], "groceries"],
            &["Travel", "1000 CNY", "2024-03-02", &last[0][1], "NULL"],
            &["Pets", "50 CNY", "2024-03-02", &last[0][1], "NULL"],
        ])
    );
    assert_eq!(
        query(&ledger, "SELECT name, date, note, spent, meta('note') FROM #short_months"),
        texts(&[
            &["Food", "2024-01-01", "first", "25.50 CNY", "january"],
            &["Food", "2024-02-01", "first", "71.00 CNY", "january"],
            &["Food", "2024-03-01", "third", "5.00 CNY", "march"],
        ])
    );
}
