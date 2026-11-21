//! Tests of the snapshots an analyzer publishes into a ledger, and of the catalog a ledger with analyzers has
//! ([`crate::analysis`]): names, the checks of an output, and what a query reads before and after an analysis.

use std::any::Any;
use std::sync::Arc;

use chrono::NaiveDate;
use serde_json::{json, Value as Json};
use zhang_core::extensions::{AnalyzerSlot, Snapshot as Published};
use zhang_core::ledger::Ledger;
use zhang_shared::plugin_abi::analysis as abi;

use super::ledger;
use crate::analysis::{self, AnalyzerTables, LedgerCatalog};
use crate::catalog::Catalog;
use crate::params::{ParamTypes, Params};
use crate::{Query, QueryErrorKind, QueryResult};

/// The manifest of an analyzer `shop`: items with metadata, the stock per month, and the items of a posting.
fn shop() -> abi::Manifest {
    serde_json::from_value(json!({
        "analysis_abi": 1,
        "consumes": ["custom:shop"],
        "tables": [
            {"name": "items", "kind": "records", "description": "Items.",
             "columns": [{"name": "item", "type": "str"}, {"name": "price", "type": "amount"}, {"name": "tags", "type": "set"}],
             "order": ["item"], "wildcard": ["item", "price"]},
            {"name": "stock", "kind": "series", "description": "Stock per month.",
             "columns": [{"name": "item", "type": "str", "fill": "carry"}, {"name": "date", "type": "date"},
                         {"name": "year", "type": "int"},
                         {"name": "opening", "type": "decimal", "fill": {"previous": "closing"}},
                         {"name": "sold", "type": "decimal", "fill": "zero"},
                         {"name": "closing", "type": "decimal", "fill": "carry"}],
             "period": {"column": "date", "unit": "month", "derive": {"year": "year"}},
             "partition": ["item"]}
        ],
        "columns": [{"table": "postings", "name": "items", "description": "The items of a posting.",
                     "predicate": "account IN :accounts AND number >= :min", "params": {"accounts": "set", "min": "decimal"}}]
    }))
    .expect("a manifest")
}

/// What `shop` finds.
fn shop_output() -> Json {
    json!({
        "tables": {"items": [
            {"item": "apple", "price": {"number": "1.50", "currency": "USD"}, "tags": ["fruit"], "$meta": {"origin": ["Fuji"]}},
            {"item": "pear", "price": null, "tags": null}
        ]},
        "series": {"stock": {"partitions": [
            {"key": {"item": "apple"}, "until": "2024-04-01", "rows": [
                {"date": "2024-01-01", "opening": "0", "sold": "0", "closing": "10"},
                {"date": "2024-03-01", "opening": "10", "sold": "4", "closing": "6"}
            ]}
        ]}},
        "labels": {"items": [
            {"label": "apple", "params": {"accounts": ["Expenses:Food"], "min": "0"}},
            {"label": "pear", "params": {"accounts": ["Expenses:Food", "Assets:Bank"], "min": "20"}}
        ]}
    })
}

const TEXT: &str = "1970-01-01 open Assets:Bank\n1970-01-01 open Expenses:Food\n\
                    2024-01-02 * \"a\"\n  Expenses:Food 12.50 USD\n  Assets:Bank -12.50 USD\n\
                    2024-01-03 * \"b\"\n  Expenses:Food 40 USD\n  Assets:Bank -40 USD\n";

/// Give `ledger` the catalog of its analyzers `analyzers` (provider, manifest, legacy) and an empty slot for each the
/// catalog has, as a load does before its analyzers run.
fn attach(ledger: &mut Ledger, analyzers: &[(&str, abi::Manifest, bool)]) -> LedgerCatalog {
    let tables = analyzers
        .iter()
        .map(|(provider, manifest, legacy)| AnalyzerTables {
            provider,
            manifest,
            legacy: *legacy,
        })
        .collect::<Vec<_>>();
    let catalog = analysis::ledger_catalog(&tables);
    ledger.extensions.catalog = Some(catalog.catalog.clone() as Arc<dyn Any + Send + Sync>);
    ledger.extensions.analyzers = catalog.analyzers().map(AnalyzerSlot::new).collect();
    catalog
}

/// Publish `output` as the snapshot of `provider` into `ledger`.
fn publish(ledger: &Ledger, provider: &str, output: Json) -> Result<(), String> {
    let output: abi::Output = serde_json::from_value(output).map_err(|err| err.to_string())?;
    let data = analysis::publish(ledger, provider, &output)?;
    let slot = ledger.extensions.analyzer(provider).expect("a slot");
    assert!(slot.snapshot.set(Published::ok(data)).is_ok(), "published once");
    Ok(())
}

fn run(ledger: &Ledger, sql: &str) -> Result<QueryResult, String> {
    let query = Query::compile_in(&Catalog::of_ledger(ledger), sql, &ParamTypes::default()).map_err(|err| err.message)?;
    query
        .execute_at(ledger, &Params::new(), NaiveDate::from_ymd_opt(2024, 2, 15).unwrap())
        .map_err(|err| err.message)
}

/// The rows of `sql` on `ledger`, each cell as text.
fn rows(ledger: &Ledger, sql: &str) -> Vec<Vec<String>> {
    run(ledger, sql)
        .unwrap_or_else(|err| panic!("{sql}: {err}"))
        .rows
        .iter()
        .map(|row| row.iter().map(ToString::to_string).collect())
        .collect()
}

fn texts(rows: &[&[&str]]) -> Vec<Vec<String>> {
    rows.iter().map(|row| row.iter().map(|it| (*it).to_owned()).collect()).collect()
}

/// A ledger whose analyzer `shop` published `shop_output`.
fn shop_ledger() -> Ledger {
    let mut ledger = ledger(TEXT);
    let catalog = attach(&mut ledger, &[("shop", shop(), false)]);
    assert_eq!(catalog.skipped, []);
    publish(&ledger, "shop", shop_output()).unwrap();
    ledger
}

#[test]
fn an_analyzers_tables_are_the_ledgers_own_and_read_its_snapshot() {
    let ledger = shop_ledger();
    let catalog = Catalog::of_ledger(&ledger);
    assert_ne!(catalog.digest(), Catalog::standard().digest());
    let table = catalog.find("shop_items").expect("the ledger's own table");
    assert_eq!(table.provided_by(), Some("shop"));
    assert_eq!(catalog.postings().columns().last().unwrap().name, "shop_items");
    assert_eq!(
        rows(&ledger, "SELECT item, price, tags, meta('origin') FROM #shop_items"),
        texts(&[&["apple", "1.50 USD", "fruit", "Fuji"], &["pear", "NULL", "NULL", "NULL"]])
    );
    assert_eq!(rows(&ledger, "SELECT * FROM #shop_items"), texts(&[&["apple", "1.50 USD"], &["pear", "NULL"]]));
    // the facts, and the months between and after them generated by the fills, up to `until`
    assert_eq!(
        rows(&ledger, "SELECT item, date, year, opening, sold, closing FROM #shop_stock"),
        texts(&[
            &["apple", "2024-01-01", "2024", "0", "0", "10"],
            &["apple", "2024-02-01", "2024", "10", "0", "10"],
            &["apple", "2024-03-01", "2024", "10", "4", "6"],
            &["apple", "2024-04-01", "2024", "6", "0", "6"],
        ])
    );
    assert_eq!(
        rows(&ledger, "SELECT narration, account, shop_items WHERE account = 'Expenses:Food'"),
        texts(&[&["a", "Expenses:Food", "apple"], &["b", "Expenses:Food", "apple, pear"]])
    );
    assert_eq!(rows(&ledger, "SELECT count(*) WHERE 'pear' IN shop_items"), texts(&[&["1"]]));
    // the native tables are the ledger's too
    assert_eq!(rows(&ledger, "SELECT count(*) FROM #budgets"), texts(&[&["0"]]));
    // a query compiled for another catalog does not read them
    let standard = Query::compile("SELECT count(*) FROM #budgets").unwrap();
    let err = standard
        .execute_at(&ledger, &Params::new(), NaiveDate::from_ymd_opt(2024, 1, 1).unwrap())
        .unwrap_err();
    assert_eq!(err.kind, QueryErrorKind::Compile);
    assert!(Query::compile("SELECT count(*)").unwrap().execute(&ledger, &Params::new()).is_ok());
    // nor does the standard catalog know them
    assert!(Query::compile("SELECT item FROM #shop_items").is_err());
}

#[test]
fn a_snapshot_is_read_once_its_analyzer_published_it() {
    let mut loaded = ledger(TEXT);
    let mut failed = ledger(TEXT);
    let mut second = shop();
    second.namespace = Some("more".to_owned());
    attach(&mut loaded, &[("shop", shop(), false), ("more", second, false)]);
    let analysed = &mut loaded;
    analysed.extensions.analysing = true;
    // the analyzer analysing the ledger, and those after it, have not published
    for sql in ["SELECT count(*) FROM #shop_items", "SELECT count(*) FROM #more_stock"] {
        let err = run(analysed, sql).unwrap_err();
        assert!(err.ends_with("is not available while shop analyses the ledger"), "{sql}: {err}");
    }
    assert_eq!(
        run(analysed, "SELECT shop_items").unwrap_err(),
        "column postings.shop_items is not available while shop analyses the ledger"
    );
    // a query that reads none of them runs
    assert_eq!(rows(analysed, "SELECT count(*)"), texts(&[&["4"]]));
    publish(analysed, "shop", shop_output()).unwrap();
    assert_eq!(rows(analysed, "SELECT count(*) FROM #shop_items"), texts(&[&["2"]]));
    let err = run(analysed, "SELECT count(*) FROM #more_items").unwrap_err();
    assert_eq!(err, "table #more_items is not available while more analyses the ledger");
    // skipped: its tables are empty
    analysed.extensions.analyzer("more").unwrap().snapshot.set(Published::skipped()).unwrap();
    analysed.extensions.analysing = false;
    assert_eq!(rows(analysed, "SELECT count(*) FROM #more_items"), texts(&[&["0"]]));
    assert_eq!(rows(analysed, "SELECT count(*) FROM #more_stock"), texts(&[&["0"]]));
    assert_eq!(rows(analysed, "SELECT count(*) WHERE length(more_items) > 0"), texts(&[&["0"]]));

    // failed: its tables and its label columns answer that they are unavailable, every query reading them
    attach(&mut failed, &[("shop", shop(), false)]);
    failed.extensions.analyzer("shop").unwrap().snapshot.set(Published::failed("timeout")).unwrap();
    assert_eq!(
        run(&failed, "SELECT * FROM #shop_items").unwrap_err(),
        "table #shop_items is unavailable: plugin shop failed (see the ledger errors)"
    );
    for sql in [
        "SELECT count(*) WHERE length(shop_items) > 0",
        "SELECT shop_items",
        "SELECT count(*) WHERE 'apple' IN shop_items",
    ] {
        assert_eq!(
            run(&failed, sql).unwrap_err(),
            "column postings.shop_items is unavailable: plugin shop failed (see the ledger errors)",
            "{sql}"
        );
    }
    // a query that reads none of them runs
    assert_eq!(rows(&failed, "SELECT count(*)"), texts(&[&["4"]]));
}

#[test]
fn a_snapshot_published_for_other_tables_is_never_read() {
    // published against one manifest of `shop`, then found in a ledger whose catalog has another
    let mut other = ledger(TEXT);
    let first = shop_ledger();
    let mut changed = shop();
    changed.tables[0].description = "Other items.".to_owned();
    attach(&mut other, &[("shop", changed, false)]);
    let data = first.extensions.analyzer("shop").unwrap().snapshot.get().unwrap().data.clone().unwrap();
    other.extensions.analyzer("shop").unwrap().snapshot.set(Published::ok(data)).unwrap();
    assert_eq!(
        run(&other, "SELECT count(*) FROM #shop_items").unwrap_err(),
        "table #shop_items is unavailable: shop published other tables"
    );
    assert_eq!(
        run(&other, "SELECT count(*) WHERE length(shop_items) > 0").unwrap_err(),
        "column postings.shop_items is unavailable: shop published other tables"
    );
    // and a provider the catalog does not have publishes nothing
    let err = analysis::publish(&other, "nope", &abi::Output::default()).unwrap_err();
    assert_eq!(err, "the catalog of the ledger has no tables of nope");
}

/// Why publishing `output` as `shop`'s fails.
fn refused(output: Json) -> String {
    let mut ledger = ledger(TEXT);
    attach(&mut ledger, &[("shop", shop(), false)]);
    publish(&ledger, "shop", output).expect_err("the output is refused")
}

#[test]
fn an_output_is_checked_whole_before_anything_of_it_is_published() {
    let with = |change: &dyn Fn(&mut Json)| {
        let mut output = shop_output();
        change(&mut output);
        output
    };
    let items = |output: &mut Json| output["tables"]["items"][0].as_object_mut().unwrap().clone();
    let cases: Vec<(Json, &str)> = vec![
        (with(&|o| o["tables"]["nope"] = json!([])), "its manifest declares no record table nope"),
        (
            with(&|o| o["series"]["items"] = json!({"partitions": []})),
            "its manifest declares no series table items",
        ),
        (with(&|o| o["labels"]["nope"] = json!([])), "its manifest declares no label column nope"),
        (
            with(&|o| {
                o["tables"]["items"][0].as_object_mut().unwrap().remove("tags");
            }),
            "table items, row 1: it has no tags",
        ),
        (
            with(&|o| o["tables"]["items"][0]["extra"] = json!(1)),
            "table items, row 1: it has a cell extra, which is no column",
        ),
        (
            with(&|o| o["tables"]["items"][0]["item"] = json!(1)),
            "table items, row 1: item: it must be a string or null",
        ),
        (
            with(&|o| o["tables"]["items"][0]["price"] = json!({"number": 1.5, "currency": "USD"})),
            "price: a decimal is a string",
        ),
        (with(&|o| o["tables"]["items"][0]["price"] = json!("1.50 USD")), "price: an amount is {"),
        (with(&|o| o["tables"]["items"][0]["$meta"] = json!({"k": "v"})), "its $meta is"),
        (
            with(&|o| {
                let first = items(o);
                o["tables"]["items"].as_array_mut().unwrap().push(Json::Object(first));
            }),
            "table shop_items: row 2 comes after row 3, but the rows are ordered by item",
        ),
        (
            with(&|o| o["series"]["stock"]["partitions"][0]["rows"][1]["date"] = json!("2024-01-01")),
            "the period of row 2 is not after that of row 1",
        ),
        (
            with(&|o| o["series"]["stock"]["partitions"][0]["rows"][1]["date"] = json!("2024-03-02")),
            "row 2: 2024-03-02 is not the first day of a month",
        ),
        (
            with(&|o| o["series"]["stock"]["partitions"][0]["rows"][1]["year"] = json!(2024)),
            "it has a cell year, which is no column it has",
        ),
        (
            with(&|o| o["series"]["stock"]["partitions"][0]["rows"][1]["item"] = json!("apple")),
            "it has a cell item, which is no column it has",
        ),
        (
            with(&|o| {
                o["series"]["stock"]["partitions"][0]["rows"][1].as_object_mut().unwrap().remove("date");
            }),
            "row 2: it has no date, the period",
        ),
        (
            with(&|o| o["series"]["stock"]["partitions"][0]["key"] = json!({})),
            "partition 1: its key: it has no item",
        ),
        (
            with(&|o| o["series"]["stock"]["partitions"][0]["key"] = json!({"item": null})),
            "partition 1 has no item",
        ),
        (
            with(&|o| {
                let first = o["series"]["stock"]["partitions"][0].clone();
                o["series"]["stock"]["partitions"].as_array_mut().unwrap().push(first);
            }),
            "partition 2 has the key of an earlier one",
        ),
        (with(&|o| o["series"]["stock"]["partitions"][0]["rows"] = json!([])), "partition 1 has no row"),
        (
            with(&|o| o["series"]["stock"]["partitions"][0]["until"] = json!("2024-02-01")),
            "partition 1 runs until 2024-02-01, which is not the first day of a month on or after its last row",
        ),
        (
            with(&|o| o["series"]["stock"]["partitions"][0]["until_cause"] = json!({"seq": 99})),
            "its until_cause names the entry 99, which the ledger does not have",
        ),
        (
            with(&|o| {
                o["labels"]["items"][0]["params"].as_object_mut().unwrap().remove("min");
            }),
            "label \"apple\": it does not bind :min",
        ),
        (
            with(&|o| o["labels"]["items"][0]["params"]["max"] = json!(1)),
            "it binds :max, which is no parameter of the column",
        ),
        (with(&|o| o["labels"]["items"][0]["params"]["min"] = json!(3)), ":min: a decimal is a string"),
        (
            with(&|o| {
                let first = o["labels"]["items"][0].clone();
                o["labels"]["items"].as_array_mut().unwrap().push(first);
            }),
            "the label \"apple\" is published twice",
        ),
    ];
    for (output, expected) in cases {
        let err = refused(output.clone());
        assert!(err.contains(expected), "{expected}\n  got: {err}\n  for: {output}");
    }
}

#[test]
fn an_output_holds_at_most_a_million_values() {
    let mut output = shop_output();
    output["tables"]["items"][1]["item"] = json!("x".repeat(64 * analysis::MAX_OUTPUT_VALUES as usize));
    assert_eq!(refused(output), "table items, row 2: the output holds more than 1000000 values");
}

/// Why `shop` with `change` made to its manifest is skipped, if it is.
fn skipped_with(change: &dyn Fn(&mut abi::Manifest), name: &str) -> Option<String> {
    let mut manifest = shop();
    change(&mut manifest);
    let mut ledger = ledger(TEXT);
    attach(&mut ledger, &[(name, manifest, false)])
        .skipped
        .into_iter()
        .next()
        .map(|(_, message)| message)
}

#[test]
fn an_analyzers_names_start_with_its_namespace() {
    let names = |name: &str, namespace: Option<&str>| {
        let mut manifest = shop();
        manifest.namespace = namespace.map(str::to_owned);
        let mut ledger = ledger(TEXT);
        let catalog = attach(&mut ledger, &[(name, manifest, false)]);
        assert_eq!(catalog.skipped, []);
        let catalog = Catalog::of_ledger(&ledger);
        let tables = catalog
            .tables()
            .iter()
            .filter(|it| it.provided_by() == Some(name))
            .map(|it| it.name().to_owned());
        tables
            .chain(std::iter::once(catalog.postings().columns().last().unwrap().name.to_string()))
            .collect::<Vec<_>>()
    };
    assert_eq!(names("shop", None), ["shop_items", "shop_stock", "shop_items"]);
    assert_eq!(names("fx-rate.v2", None), ["fx_rate_v2_items", "fx_rate_v2_stock", "fx_rate_v2_items"]);
    assert_eq!(names("My Shop", Some("mine")), ["mine_items", "mine_stock", "mine_items"]);
    assert_eq!(
        skipped_with(&|_| {}, "2shop").unwrap(),
        "2shop: its name gives the namespace '2shop', which is no name (a name is a lower-case letter, then lower-case letters, digits and '_'): declare a namespace"
    );
    assert!(skipped_with(&|m| m.namespace = Some("Shop".to_owned()), "shop")
        .unwrap()
        .starts_with("shop: its namespace 'Shop' is no name"));
    // the legacy names are zhang:budget's alone, whatever the namespace
    assert_eq!(
        skipped_with(
            &|m| {
                m.namespace = Some("budget".to_owned());
                m.tables[0].name = "definitions".to_owned();
            },
            "shop"
        )
        .unwrap(),
        "shop: table definitions would be named budget_definitions, which is kept for zhang:budget"
    );
    assert_eq!(
        skipped_with(&|m| m.analysis_abi = 2, "shop").unwrap(),
        "shop: its manifest is written for the analysis ABI 2, and this zhang reads 1"
    );
    assert_eq!(
        skipped_with(&|m| m.tables[0].columns[0].ty = "position".to_owned(), "shop").unwrap(),
        "shop: table 'items': column 'item': its type is 'position', which is not one of str, int, decimal, date, bool, amount, set, metas"
    );
    assert_eq!(
        skipped_with(&|m| m.tables[1].columns[4].fill = Some(abi::Fill::Named("last".to_owned())), "shop").unwrap(),
        "shop: table 'stock': column 'sold' is filled with 'last': a fill is carry, zero or {\"previous\": column}"
    );
    assert_eq!(
        skipped_with(
            &|m| {
                m.columns[0].params.insert("x".to_owned(), "amount".to_owned());
            },
            "shop"
        )
        .unwrap(),
        "shop: column postings.items: parameter :x: its type is 'amount', which is not one of bool, int, decimal, str, date, set"
    );
    // the core's names, and the native provider's
    assert_eq!(
        skipped_with(&|_| {}, "zhang:budget").unwrap(),
        "zhang:budget: a provider of tables is named zhang:budget already"
    );
    assert_eq!(
        skipped_with(&|m| m.namespace = Some("account".to_owned()), "shop"),
        None,
        "account_items is no core name"
    );
}

#[test]
fn of_two_analyzers_with_one_name_the_first_keeps_it() {
    let mut ledger = ledger(TEXT);
    let mut other = shop();
    other.namespace = Some("other".to_owned());
    let mut taken = shop();
    taken.namespace = Some("other".to_owned());
    let catalog = attach(&mut ledger, &[("shop", shop(), false), ("shop", other, false), ("again", taken, false)]);
    assert_eq!(catalog.skipped, [(1, "shop: a provider of tables is named shop already".to_owned())]);
    // `again` has other_items: the second analyzer named shop was skipped, with all of its tables
    assert_eq!(catalog.analyzers().collect::<Vec<_>>(), ["shop", "again"]);
}

/// The manifest of an analyzer standing in for the native budget provider's `#budget_definitions` and
/// `postings.budgets`.
fn budget_definitions() -> abi::Manifest {
    serde_json::from_value(json!({
        "analysis_abi": 1,
        "tables": [{"name": "budget_definitions", "kind": "records", "columns": [{"name": "name", "type": "str"}], "order": ["name"]}],
        "columns": [{"table": "postings", "name": "budgets", "predicate": "account IN :accounts", "params": {"accounts": "set"}}]
    }))
    .unwrap()
}

#[test]
fn zhang_budget_serves_the_legacy_names_instead_of_the_native_provider() {
    let text = format!("{TEXT}1970-01-01 open Expenses:Fun\n  budget: Fun\n2024-01-01 budget Fun USD\n");
    let mut fallback = ledger(&text);
    let mut ledger = ledger(&text);
    let catalog = attach(&mut ledger, &[("zhang:budget", budget_definitions(), true)]);
    assert_eq!(catalog.skipped, []);
    publish(
        &ledger,
        "zhang:budget",
        json!({"tables": {"budget_definitions": [{"name": "From the plugin"}]},
               "labels": {"budgets": [{"label": "Plugged", "params": {"accounts": ["Assets:Bank"]}}]}}),
    )
    .unwrap();
    let catalog = Catalog::of_ledger(&ledger);
    let names = catalog.tables().iter().map(|it| it.name()).collect::<Vec<_>>();
    assert_eq!(&names[10..], ["budgets", "budget_events", "budget_definitions", "errors"]);
    // from the plugin's snapshot
    assert_eq!(rows(&ledger, "SELECT * FROM #budget_definitions"), texts(&[&["From the plugin"]]));
    assert_eq!(
        rows(&ledger, "SELECT account, budgets WHERE 'Plugged' IN budgets"),
        texts(&[&["Assets:Bank", "Plugged"], &["Assets:Bank", "Plugged"]])
    );
    // from the native provider, which keeps the tables the plugin does not serve
    assert_eq!(rows(&ledger, "SELECT name FROM #budgets WHERE date = 2024-01-01"), texts(&[&["Fun"]]));
    // a zhang:budget whose tables the catalog cannot have leaves the native provider all of its own
    let mut refused = budget_definitions();
    refused.columns[0].predicate = "nope".to_owned();
    let catalog = attach(&mut fallback, &[("zhang:budget", refused, true)]);
    assert_eq!(catalog.skipped.len(), 1);
    assert!(
        catalog.skipped[0]
            .1
            .starts_with("zhang:budget: column postings.budgets: its predicate does not compile"),
        "{:?}",
        catalog.skipped
    );
    assert_eq!(rows(&fallback, "SELECT name FROM #budget_definitions"), texts(&[&["Fun"]]));
    assert_eq!(Catalog::of_ledger(&fallback).digest(), Catalog::standard().digest());
}
