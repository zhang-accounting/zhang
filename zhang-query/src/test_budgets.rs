//! Snapshots of `zhang:budget` for the query engine's own tests: the bundled script that computes the budget tables runs
//! in a server, with the plugin runtime, which a unit test of the query engine has not, so a test that reads the budget
//! tables (a series table, record tables, a label column) publishes a synthetic snapshot of them, as a load publishes
//! the script's ([`crate::analysis::publish`]). The figures are the script's way of seeing each test's ledger, written
//! out; what the tests check is how the engine reads them, not the budget rules, which the server's tests check.

use std::any::Any;
use std::sync::Arc;

use serde_json::{json, Value as Json};
use zhang_core::analysis::AnalyzerTables;
use zhang_core::extensions::{AnalyzerSlot, Snapshot};
use zhang_core::ledger::Ledger;
use zhang_shared::plugin_abi::analysis as abi;

use crate::catalog::Catalog;

/// Give `ledger` the snapshot `output` of `zhang:budget`, an analyzer's output, as a load does when the script
/// publishes it: the ledger's catalog with the budget tables of the plugin's manifest (the SDK's budget analyzer
/// declares it, as the bundled script does), and the snapshot checked against it and published.
pub(crate) fn publish_budgets(ledger: &mut Ledger, output: Json) {
    let manifest: abi::Manifest = serde_json::from_str(include_str!("../../zhang-plugin-sdk/examples/budget/manifest.json")).expect("the budget manifest");
    let tables = [AnalyzerTables {
        provider: crate::extension::abi::BUDGET,
        manifest: &manifest,
        legacy: true,
    }];
    let catalog = crate::analysis::ledger_catalog(&tables);
    assert_eq!(catalog.skipped, []);
    ledger.extensions.catalog = Some(catalog.catalog.clone() as Arc<dyn Any + Send + Sync>);
    ledger.extensions.analyzers = catalog.analyzers().map(AnalyzerSlot::new).collect();
    let output: abi::Output = serde_json::from_value(output).expect("an output");
    let data = crate::analysis::publish(ledger, crate::extension::abi::BUDGET, &output).unwrap_or_else(|err| panic!("{err}"));
    assert!(ledger.extensions.analyzers[0].snapshot.set(Snapshot::ok(data)).is_ok(), "published once");
    assert_ne!(Catalog::of_ledger(ledger).digest(), Catalog::standard().digest());
}

/// An amount of the output.
fn cny(number: &str) -> Json {
    json!({"number": number, "currency": "CNY"})
}

/// A month of `#budgets` of the output: what it was assigned, what was added, spent and left.
fn month(date: &str, assigned: &str, added: &str, activity: &str, available: &str, closed: bool, account: &str) -> Json {
    json!({
        "date": date, "alias": null, "category": null, "currency": "CNY", "assigned": cny(assigned), "added": cny(added),
        "activity": cny(activity), "available": cny(available), "accounts": [account], "closed": closed,
    })
}

/// The label of a budget of `postings.budgets`, of the accounts `accounts`, closed on `close` if at all.
fn label(name: &str, accounts: &[&str], close: Option<&str>) -> Json {
    json!({"label": name, "params": {"accounts": accounts, "defined_seq": 0, "close_date": close, "close_time": null, "commodity": "CNY"}})
}

/// The budgets of the ledger of the equivalence test of the data tables, a sample for any ledger with the accounts
/// `Expenses:Food` and `Expenses:Travel`: `food` assigned 100 CNY, spending 30
/// and 10 and moving 20 to `travel`, which spends 70 and is closed in March. The facts are the months with something;
/// the host fills the others up to the current month (2026-01, the executions' today).
pub(crate) fn data_table_budgets() -> Json {
    json!({
        "tables": {
            "budget_definitions": [
                {"name": "food", "date": "2024-01-01", "currency": "CNY", "alias": null, "category": null,
                 "accounts": ["Expenses:Food"], "close": null, "close_time": null},
                {"name": "travel", "date": "2024-01-01", "currency": "CNY", "alias": null, "category": null,
                 "accounts": ["Expenses:Travel"], "close": "2024-03-01", "close_time": null},
            ],
            "budget_events": [
                {"name": "food", "date": "2024-01-01", "time": "00:00:00", "timestamp": 1704067200, "type": "assign", "amount": cny("100")},
                {"name": "food", "date": "2024-02-01", "time": "00:00:00", "timestamp": 1706745600, "type": "transfer_out", "amount": cny("-20")},
                {"name": "travel", "date": "2024-02-01", "time": "00:00:00", "timestamp": 1706745600, "type": "transfer_in", "amount": cny("20")},
                {"name": "travel", "date": "2024-03-01", "time": "00:00:00", "timestamp": 1709251200, "type": "close", "amount": null},
            ],
        },
        "series": {"budgets": {"partitions": [
            {"key": {"name": "food"}, "until": "2024-03-01", "rows": [
                month("2024-01-01", "100", "100", "30", "70", false, "Expenses:Food"),
                month("2024-02-01", "50", "-20", "0", "50", false, "Expenses:Food"),
                month("2024-03-01", "50", "0", "10", "40", false, "Expenses:Food"),
            ]},
            {"key": {"name": "travel"}, "until": "2024-03-01", "rows": [
                month("2024-01-01", "0", "0", "0", "0", false, "Expenses:Travel"),
                month("2024-02-01", "20", "20", "70", "-50", false, "Expenses:Travel"),
                month("2024-03-01", "-50", "0", "0", "-50", true, "Expenses:Travel"),
            ]},
        ]}},
        "labels": {"budgets": [label("food", &["Expenses:Food"], None), label("travel", &["Expenses:Travel"], Some("2024-03-01"))]},
    })
}

/// The budgets of the ledger of the equivalence test of the date bounds: `food` from January, `travel` from March, closed in May.
pub(crate) fn date_bound_budgets() -> Json {
    json!({
        "tables": {
            "budget_definitions": [
                {"name": "food", "date": "2024-01-01", "currency": "CNY", "alias": null, "category": null,
                 "accounts": ["Expenses:Food"], "close": null, "close_time": null},
                {"name": "travel", "date": "2024-03-01", "currency": "CNY", "alias": null, "category": null,
                 "accounts": ["Expenses:Travel"], "close": "2024-05-01", "close_time": null},
            ],
            "budget_events": [
                {"name": "food", "date": "2024-01-01", "time": "00:00:00", "timestamp": 1704067200, "type": "assign", "amount": cny("100")},
                {"name": "food", "date": "2024-03-05", "time": "00:00:00", "timestamp": 1709596800, "type": "transfer_out", "amount": cny("-20")},
                {"name": "travel", "date": "2024-03-05", "time": "00:00:00", "timestamp": 1709596800, "type": "transfer_in", "amount": cny("20")},
                {"name": "travel", "date": "2024-05-01", "time": "00:00:00", "timestamp": 1714521600, "type": "close", "amount": null},
            ],
        },
        "series": {"budgets": {"partitions": [
            {"key": {"name": "food"}, "until": "2024-04-01", "rows": [
                month("2024-01-01", "100", "100", "30", "70", false, "Expenses:Food"),
                month("2024-02-01", "70", "0", "70", "0", false, "Expenses:Food"),
                month("2024-03-01", "-20", "-20", "0", "-20", false, "Expenses:Food"),
            ]},
            {"key": {"name": "travel"}, "until": "2024-05-01", "rows": [
                month("2024-03-01", "20", "20", "0", "20", false, "Expenses:Travel"),
                month("2024-04-01", "20", "0", "15", "5", false, "Expenses:Travel"),
                month("2024-05-01", "5", "0", "0", "5", true, "Expenses:Travel"),
            ]},
        ]}},
        "labels": {"budgets": [label("food", &["Expenses:Food"], None), label("travel", &["Expenses:Travel"], Some("2024-05-01"))]},
    })
}

/// The labels of `postings.budgets` of the ledger of the equivalence test of `open_meta_has()`: `food` and `fun` of
/// `Expenses:Food` until its close, `travel` of `Expenses:Travel` and of `Expenses:Food` from its reopening, and
/// `food` again of `Expenses:Travel` from its same-day reopening; `fun` is closed on 2024-02-01 at 11:00.
pub(crate) fn label_budgets() -> Json {
    json!({"labels": {"budgets": [
        label("food", &["Expenses:Food", "Expenses:Travel"], None),
        {"label": "fun", "params": {"accounts": ["Expenses:Food"], "defined_seq": 0, "close_date": "2024-02-01",
                                    "close_time": "11:00:00", "commodity": "CNY"}},
        label("travel", &["Expenses:Food", "Expenses:Travel"], None),
    ]}})
}
