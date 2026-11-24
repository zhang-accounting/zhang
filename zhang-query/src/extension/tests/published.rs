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
use crate::extension::snapshot::Snapshot;
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

/// Publish `output` as the snapshot of `provider` into `ledger`. The output is read both ways, the long way
/// ([`analysis::publish`]) and the quick way ([`analysis::publish_raw`]), which must make the same snapshot of it, or
/// both refuse it.
fn publish(ledger: &Ledger, provider: &str, output: Json) -> Result<(), String> {
    let (long, quick) = both_ways(ledger, provider, &serde_json::to_string(&output).unwrap());
    match (&long, quick) {
        (Ok(_), None) => panic!("the quick way refused an output the long way took: {output}"),
        (Err(err), Some(_)) => panic!("the quick way took an output the long way refused ({err}): {output}"),
        (Ok((_, long)), Some(quick)) => assert_eq!(*long, quick, "both ways read {output}"),
        (Err(_), None) => {}
    }
    let (data, _) = long?;
    let slot = ledger.extensions.analyzer(provider).expect("a slot");
    assert!(slot.snapshot.set(Published::ok(data)).is_ok(), "published once");
    Ok(())
}

/// The output `text` of `provider` read the long way (the data and the snapshot written out, or the error) and the
/// quick way (the snapshot written out, if it took the output).
#[allow(clippy::type_complexity)]
fn both_ways(ledger: &Ledger, provider: &str, text: &str) -> (Result<(Arc<dyn Any + Send + Sync>, String), String>, Option<String>) {
    let describe = |data: &Arc<dyn Any + Send + Sync>| data.downcast_ref::<Snapshot>().expect("a snapshot").describe();
    let long = serde_json::from_str::<abi::Output>(text)
        .map_err(|err| err.to_string())
        .and_then(|output| analysis::publish(ledger, provider, &output))
        .map(|data| {
            let described = describe(&data);
            (data, described)
        });
    let quick = serde_json::from_str::<abi::RawOutput>(text)
        .ok()
        .and_then(|output| analysis::publish_raw(ledger, provider, &output))
        .map(|data| describe(&data));
    (long, quick)
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
    // the tables zhang declares are the ledger's too, and say that zhang:budget does not run for it
    let err = run(&ledger, "SELECT count(*) FROM #budgets").unwrap_err();
    assert!(err.starts_with("table #budgets is unavailable: plugin zhang:budget does not run: "), "{err}");
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

/// The tables of a provider, and postings with its label column, are tables the catalog owns and shares: a query
/// compiled once keeps handles to them, never their rows, and reads the snapshot of the ledger it runs on.
#[test]
fn a_query_compiled_once_reads_the_rows_of_the_ledger_it_runs_on() {
    let catalog = shop_ledger()
        .extensions
        .catalog
        .clone()
        .and_then(|it| it.downcast::<Catalog>().ok())
        .expect("a catalog");
    for table in ["shop_items", "shop_stock", "postings"] {
        assert!(matches!(catalog.find(table), Some(crate::catalog::TableRef::Shared(_))), "{table}");
    }
    let before = shop_ledger();
    let mut after = ledger(TEXT);
    attach(&mut after, &[("shop", shop(), false)]);
    let mut output = shop_output();
    output["tables"]["items"][0]["item"] = json!("banana");
    output["series"]["stock"]["partitions"][0]["key"]["item"] = json!("banana");
    output["labels"]["items"][0]["label"] = json!("banana");
    publish(&after, "shop", output).unwrap();
    assert_eq!(Catalog::of_ledger(&before).digest(), Catalog::of_ledger(&after).digest());
    for sql in [
        "SELECT item FROM #shop_items",
        "SELECT item, closing FROM #shop_stock WHERE date = 2024-03-01",
        "SELECT shop_items",
        "SELECT count(*) WHERE 'apple' IN shop_items",
    ] {
        let query = Query::compile_in(&catalog, sql, &ParamTypes::default()).unwrap();
        let today = NaiveDate::from_ymd_opt(2024, 3, 1).unwrap();
        let once = |ledger: &Ledger| format!("{:?}", query.execute_at(ledger, &Params::new(), today).unwrap().rows);
        let fresh = |ledger: &Ledger| {
            let query = Query::compile_in(&Catalog::of_ledger(ledger), sql, &ParamTypes::default()).unwrap();
            format!("{:?}", query.execute_at(ledger, &Params::new(), today).unwrap().rows)
        };
        assert_eq!(once(&before), fresh(&before), "{sql}");
        assert_eq!(once(&after), fresh(&after), "{sql}");
        assert_ne!(once(&before), once(&after), "{sql}");
    }
}

#[test]
fn a_fact_after_the_first_may_leave_out_its_carried_cells_and_metadata() {
    let mut stocked = ledger(TEXT);
    attach(&mut stocked, &[("shop", shop(), false)]);
    let output = |rows: Json| {
        let mut output = shop_output();
        output["series"]["stock"]["partitions"][0]["rows"] = rows;
        output
    };
    publish(
        &stocked,
        "shop",
        output(json!([
            {"date": "2024-01-01", "opening": "0", "sold": "0", "closing": "10", "$meta": {"origin": ["Fuji"]}},
            {"date": "2024-03-01", "opening": "10", "sold": "4"}
        ])),
    )
    .unwrap();
    assert_eq!(
        rows(
            &stocked,
            "SELECT date, opening, sold, closing, meta('origin') FROM #shop_stock WHERE date <= 2024-03-01"
        ),
        texts(&[
            &["2024-01-01", "0", "0", "10", "Fuji"],
            &["2024-02-01", "10", "0", "10", "Fuji"],
            &["2024-03-01", "10", "4", "10", "Fuji"],
        ])
    );
    // the first fact has every cell, and a cell filled otherwise is never left out
    let mut first = ledger(TEXT);
    attach(&mut first, &[("shop", shop(), false)]);
    let refused = publish(&first, "shop", output(json!([{"date": "2024-01-01", "opening": "0", "sold": "0"}]))).unwrap_err();
    assert!(refused.ends_with("row 1: it has no closing"), "{refused}");
    let refused = publish(
        &first,
        "shop",
        output(json!([
            {"date": "2024-01-01", "opening": "0", "sold": "0", "closing": "10"},
            {"date": "2024-03-01", "opening": "10", "closing": "6"}
        ])),
    )
    .unwrap_err();
    assert!(refused.ends_with("row 2: it has no sold"), "{refused}");
}

#[test]
fn a_row_may_list_its_cells_and_an_amount_be_a_pair() {
    let mut listed = ledger(TEXT);
    attach(&mut listed, &[("shop", shop(), false)]);
    let mut output = shop_output();
    output["tables"]["items"] = json!([
        ["apple", ["1.50", "USD"], ["fruit"], {"origin": ["Fuji"]}],
        ["pear", null, null]
    ]);
    publish(&listed, "shop", output).unwrap();
    assert_eq!(
        rows(&listed, "SELECT item, price, tags, meta('origin') FROM #shop_items"),
        texts(&[&["apple", "1.50 USD", "fruit", "Fuji"], &["pear", "NULL", "NULL", "NULL"]])
    );
    // as many cells as columns, and maybe the metadata
    let mut short = ledger(TEXT);
    attach(&mut short, &[("shop", shop(), false)]);
    let mut output = shop_output();
    output["tables"]["items"] = json!([["apple", ["1.50", "USD"]]]);
    let refused = publish(&short, "shop", output).unwrap_err();
    assert!(
        refused.ends_with("row 1: it lists 2 cells, where the table has 3 columns (and maybe the row's metadata after them)"),
        "{refused}"
    );
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
    // the core's names, and the declared provider's
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

/// The manifest of an analyzer standing in for `zhang:budget` with `#budget_definitions` and `postings.budgets` only.
fn budget_definitions() -> abi::Manifest {
    serde_json::from_value(json!({
        "analysis_abi": 1,
        "tables": [{"name": "budget_definitions", "kind": "records", "columns": [{"name": "name", "type": "str"}], "order": ["name"]}],
        "columns": [{"table": "postings", "name": "budgets", "predicate": "account IN :accounts", "params": {"accounts": "set"}}]
    }))
    .unwrap()
}

#[test]
fn zhang_budget_serves_the_legacy_names_instead_of_the_declared_ones() {
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
    // the tables the plugin does not serve stay those zhang declares, which have no data
    let err = run(&ledger, "SELECT name FROM #budgets WHERE date = 2024-01-01").unwrap_err();
    assert!(err.starts_with("table #budgets is unavailable: "), "{err}");
    // a zhang:budget whose tables the catalog cannot have leaves the declared ones all of theirs
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
    let err = run(&fallback, "SELECT name FROM #budget_definitions").unwrap_err();
    assert!(err.starts_with("table #budget_definitions is unavailable: "), "{err}");
    assert_eq!(Catalog::of_ledger(&fallback).digest(), Catalog::standard().digest());
}

#[test]
fn the_errors_of_a_ledger_its_analyzers_analyse_are_not_read() {
    let mut analysed = ledger("1970-01-01 open Assets:Bank\n2024-01-02 * \"a\"\n  Assets:Bank 1 USD\n  Expenses:Nope -1 USD\n");
    analysed.extensions.analysing = true;
    for sql in [
        "SELECT * FROM #errors",
        "SELECT errors FROM #transactions",
        "SELECT seq FROM #transactions WHERE balanced",
        "SELECT count(*) WHERE length(errors) > 0",
    ] {
        assert_eq!(
            run(&analysed, sql).unwrap_err(),
            "the ledger's errors are not available while plugins analyse the ledger",
            "{sql}"
        );
    }
    assert_eq!(rows(&analysed, "SELECT count(*) FROM #transactions"), texts(&[&["1"]]));
    analysed.extensions.analysing = false;
    assert_ne!(
        rows(&analysed, "SELECT count(*) FROM #errors"),
        texts(&[&["0"]]),
        "the ledger has errors, read once placed"
    );
}

/// A manifest of an analyzer `cells`: a record table with a column of every type a cell can have.
fn cells() -> abi::Manifest {
    serde_json::from_value(json!({
        "analysis_abi": 1,
        "tables": [{"name": "all", "kind": "records", "columns": [
            {"name": "s", "type": "str"}, {"name": "i", "type": "int"}, {"name": "d", "type": "decimal"},
            {"name": "t", "type": "date"}, {"name": "b", "type": "bool"}, {"name": "a", "type": "amount"},
            {"name": "e", "type": "set"}, {"name": "m", "type": "metas"}
        ]}]
    }))
    .expect("a manifest")
}

#[test]
fn the_quick_way_reads_an_output_as_the_long_way_does_or_leaves_it_to_it() {
    let mut ledger = ledger(TEXT);
    attach(&mut ledger, &[("shop", shop(), false), ("cells", cells(), false)]);
    // the quick way takes these, and makes the snapshot the long way makes
    let row = r#"{"s": "x", "i": 1, "d": "1.50", "t": "2024-01-31", "b": true, "a": {"number": "-0.10", "currency": "USD"}, "e": ["b", "a", "b"], "m": [{"key": "k", "value": "v"}]}"#;
    let taken = [
        format!(r#"{{"tables": {{"all": [{row}]}}}}"#),
        r#"{"tables": {"all": [{"s": null, "i": null, "d": null, "t": null, "b": null, "a": null, "e": null, "m": null, "$meta": {"z": ["1", "2"], "a": []}}]}}"#.to_owned(),
        // escaped keys and texts, keys in any order, the largest integers, a decimal with an exponent
        r#"{"tables": {"all": [{"m": [], "e": [], "a": {"currency": "USD", "number": "1e3", "other": [1]}, "b": false, "t": "2024-1-5", "d": " 2 ", "i": -9223372036854775808, "s": "\"é\""}]}}"#.to_owned(),
        r#"{"tables": {"all": [{"s": "", "i": 9223372036854775807, "d": "-0", "t": "2024-02-29", "b": true, "a": null, "e": ["é"], "m": [{"value": "v", "key": "k", "x": null}]}]}}"#.to_owned(),
        // a partition's fields in any order, and fields nobody reads, at every level
        r#"{"x": 1, "series": {"stock": {"y": [], "partitions": [{"rows": [{"date": "2024-01-01", "opening": "0", "sold": "1", "closing": "9", "$meta": {"k": ["v"]}}], "first_cause": {"seq": 0, "z": 1}, "until_cause": null, "until": "2024-03-01", "key": {"item": "apple"}, "w": {}}]}},
            "labels": {"items": [{"params": {"min": "1", "accounts": ["Assets:Bank"]}, "label": "apple", "v": 1}]}}"#.to_owned(),
        r#"{"series": {"stock": {}}, "labels": {"items": []}, "tables": {"items": []}}"#.to_owned(),
        // the shorter forms: a row listed, an amount as a pair, the carried cells and metadata left out
        r#"{"tables": {"all": [["x", 1, "1", "2024-01-31", true, ["1", "USD"], null, null], ["y", 2, "2", "2024-02-01", false, null, null, null, {"k": ["v"]}]]}}"#.to_owned(),
        r#"{"series": {"stock": {"partitions": [{"key": {"item": "a"}, "until": "2024-03-01", "rows": [{"date": "2024-01-01", "opening": "0", "sold": "1", "closing": "9", "$meta": {"k": ["v"]}}, {"date": "2024-02-01", "opening": "9", "sold": "1"}, {"date": "2024-03-01", "opening": "9", "sold": "2", "closing": "7", "$meta": {}}]}]}}}"#.to_owned(),
    ];
    for text in &taken {
        let (long, quick) = both_ways(&ledger, if text.contains("\"all\"") { "cells" } else { "shop" }, text);
        let (_, long) = long.unwrap_or_else(|err| panic!("{text}: {err}"));
        assert_eq!(Some(long), quick, "{text}");
    }
    // the long way takes these, and the quick way leaves them to it: a repeated key, a struct written as a list
    for text in [
        r#"{"tables": {"items": [{"item": "a", "item": "b", "price": null, "tags": null}]}}"#,
        r#"{"tables": {"items": [], "items": []}}"#,
        r#"{"tables": {"items": [{"item": "a", "price": null, "tags": null, "$meta": {"k": ["1"], "k": ["2"]}}]}}"#,
        r#"{"tables": {"items": [{"item": "a", "price": {"number": "1", "number": "2", "currency": "USD"}, "tags": null}]}}"#,
        r#"{"series": {"stock": {"partitions": [{"key": {"item": "a"}, "until": "2024-01-01", "until_cause": [0], "rows": [{"date": "2024-01-01", "opening": "0", "sold": "0", "closing": "0"}]}]}}}"#,
    ] {
        let (long, quick) = both_ways(&ledger, "shop", text);
        assert!(long.is_ok(), "{text}: {:?}", long.err());
        assert_eq!(quick, None, "{text}");
    }
    // and the quick way takes nothing the long way refuses
    for text in [
        r#"{"tables": {"all": [{"s": 1, "i": 1, "d": "1", "t": "2024-01-31", "b": true, "a": null, "e": null, "m": null}]}}"#,
        r#"{"tables": {"all": [{"s": "x", "i": 1.0, "d": "1", "t": "2024-01-31", "b": true, "a": null, "e": null, "m": null}]}}"#,
        r#"{"tables": {"all": [{"s": "x", "i": 9223372036854775808, "d": "1", "t": "2024-01-31", "b": true, "a": null, "e": null, "m": null}]}}"#,
        r#"{"tables": {"all": [{"s": "x", "i": 1, "d": 1, "t": "2024-01-31", "b": true, "a": null, "e": null, "m": null}]}}"#,
        r#"{"tables": {"all": [{"s": "x", "i": 1, "d": "1x", "t": "2024-01-31", "b": true, "a": null, "e": null, "m": null}]}}"#,
        r#"{"tables": {"all": [{"s": "x", "i": 1, "d": "1e100000000", "t": "2024-01-31", "b": true, "a": null, "e": null, "m": null}]}}"#,
        r#"{"tables": {"all": [{"s": "x", "i": 1, "d": "1", "t": "2024-02-30", "b": true, "a": null, "e": null, "m": null}]}}"#,
        r#"{"tables": {"all": [{"s": "x", "i": 1, "d": "1", "t": "2024-01-31", "b": 1, "a": null, "e": null, "m": null}]}}"#,
        r#"{"tables": {"all": [{"s": "x", "i": 1, "d": "1", "t": "2024-01-31", "b": true, "a": {"number": null, "currency": "USD"}, "e": null, "m": null}]}}"#,
        r#"{"tables": {"all": [{"s": "x", "i": 1, "d": "1", "t": "2024-01-31", "b": true, "a": {"number": "1"}, "e": null, "m": null}]}}"#,
        r#"{"tables": {"all": [{"s": "x", "i": 1, "d": "1", "t": "2024-01-31", "b": true, "a": null, "e": ["a", 1], "m": null}]}}"#,
        r#"{"tables": {"all": [{"s": "x", "i": 1, "d": "1", "t": "2024-01-31", "b": true, "a": null, "e": null, "m": [{"key": "k"}]}]}}"#,
        r#"{"tables": {"all": [{"s": "x", "i": 1, "d": "1", "t": "2024-01-31", "b": true, "a": null, "e": null, "m": null, "$meta": null}]}}"#,
        r#"{"tables": {"all": {}}}"#,
        r#"{"tables": {"all": 1, "all": []}}"#,
        r#"{"labels": {"items": [{"label": 1}], "items": []}}"#,
        r#"{"tables": {"all": [["x", 1, "1", "2024-01-31", true, ["1", "USD", "x"], null, null]]}}"#,
        r#"{"tables": {"all": [["x", 1, "1", "2024-01-31", true, [1, "USD"], null, null]]}}"#,
        r#"{"tables": {"all": [["x", 1, "1", "2024-01-31", true, null, null]]}}"#,
        r#"{"tables": {"all": [["x", 1, "1", "2024-01-31", true, null, null, null, {}, {}]]}}"#,
        r#"{"tables": {"all": [["x", 1, "1", "2024-01-31", true, null, null, null, null]]}}"#,
        r#"{"tables": {"all": ["x"]}}"#,
        r#"{"series": {"stock": {"partitions": [{"key": {"item": "a"}, "until": "2024-01-01", "rows": [{"date": "2024-01-01", "opening": "0", "sold": "0"}]}]}}}"#,
        r#"{"series": {"stock": {"partitions": [{"key": {"item": "a"}, "until": "2024-02-01", "rows": [{"date": "2024-01-01", "opening": "0", "sold": "0", "closing": "1"}, {"date": "2024-02-01", "opening": "0", "closing": "1"}]}]}}}"#,
        r#"{"series": {"stock": {"partitions": [{"key": {"item": "a"}, "until": "2024-01-01", "first_cause": {"seq": 1000}, "rows": [{"date": "2024-01-01", "opening": "0", "sold": "0", "closing": "0"}]}]}}}"#,
        r#"{"series": {"stock": {"partitions": [{"key": {"item": "a"}, "until": "2024-01-01", "rows": [{"date": "2024-01-01", "opening": "0", "sold": "0"}]}]}}}"#,
        r#"{"series": {"stock": {"partitions": [{"key": {"item": "a"}, "until": "2024-01-01", "rows": [{"date": null, "opening": "0", "sold": "0", "closing": "0"}]}]}}}"#,
        r#"{"series": {"stock": {"partitions": [{"key": {"item": "a", "x": 1}, "until": "2024-01-01", "rows": [{"date": "2024-01-01", "opening": "0", "sold": "0", "closing": "0"}]}]}}}"#,
        r#"{"series": {"stock": {"partitions": [{"until": "2024-01-01", "rows": []}]}}}"#,
        r#"{"labels": {"items": [{"label": "a", "params": {"accounts": null}}]}}"#,
        r#"{"labels": {"items": [{"label": "a", "params": {"accounts": null, "min": "1", "max": "2"}}]}}"#,
        r#"{"labels": {"items": [{"params": {"accounts": null, "min": "1"}}]}}"#,
    ] {
        let provider = if text.contains("\"all\"") { "cells" } else { "shop" };
        let (long, quick) = both_ways(&ledger, provider, text);
        assert!(long.is_err(), "{text}: the long way takes it");
        assert_eq!(quick, None, "{text}");
    }
}

#[test]
fn a_decimal_of_an_output_has_at_most_a_thousand_digits_written_out() {
    use crate::extension::abi::{output_decimal, MAX_OUTPUT_DIGITS};
    assert_eq!(MAX_OUTPUT_DIGITS, 1000);
    let digits = |n: usize| "9".repeat(n);
    for taken in [
        digits(1000),
        format!("-{}", digits(1000)),
        format!("{}.{}", digits(500), digits(500)),
        format!("0.{}", digits(999)),
        "1e999".to_owned(),
        "-12.50".to_owned(),
        format!(" {} ", digits(1000)),
    ] {
        assert!(output_decimal(&taken).is_ok(), "{taken:.40}");
    }
    for refused in [
        digits(1001),
        format!("{}.{}", digits(500), digits(501)),
        format!("0.{}", digits(1000)),
        "1e1000".to_owned(),
        "1e-1000".to_owned(),
        format!("1{}", "0".repeat(1000)),
        digits(999_999),
    ] {
        let err = output_decimal(&refused).unwrap_err();
        assert_eq!(
            err, "it has more than 1000 digits written out in full, the most an analyzer writes",
            "{refused:.40}"
        );
    }
}

#[test]
fn a_decimal_cell_of_999_999_digits_is_refused_at_once_both_ways() {
    let mut ledger = ledger(TEXT);
    attach(&mut ledger, &[("shop", shop(), false), ("cells", cells(), false)]);
    let long = "9".repeat(999_999);
    let row = |d: &str, a: &str| {
        format!(r#"{{"s": "x", "i": 1, "d": "{d}", "t": "2024-01-31", "b": true, "a": {{"number": "{a}", "currency": "USD"}}, "e": null, "m": null}}"#)
    };
    // 30 such cells, then a row the quick way leaves to the long way: both read them all
    let mut rows = (0..15).map(|_| row(&long, &long)).collect::<Vec<_>>();
    rows.push(r#"{"s": "x", "unknown": 1}"#.to_owned());
    for text in [
        format!(r#"{{"tables": {{"all": [{}]}}}}"#, row(&long, "1")),
        format!(r#"{{"tables": {{"all": [{}]}}}}"#, row("1", &long)),
        format!(r#"{{"tables": {{"all": [{}]}}}}"#, rows.join(", ")),
        format!(r#"{{"labels": {{"items": [{{"label": "a", "params": {{"accounts": null, "min": "{long}"}}}}]}}}}"#),
    ] {
        let started = std::time::Instant::now();
        let provider = if text.contains("\"all\"") { "cells" } else { "shop" };
        let (long_way, quick) = both_ways(&ledger, provider, &text);
        let err = match long_way {
            Ok(_) => panic!("the long way took a decimal of 999,999 digits"),
            Err(err) => err,
        };
        assert!(err.contains("more than 1000 digits written out in full"), "{err:.200}");
        assert_eq!(quick, None);
        // parsing them would take seconds each
        assert!(started.elapsed() < std::time::Duration::from_secs(2), "{:?}", started.elapsed());
    }
}

/// The worst an output costs now, both ways: as many decimals of 1000 digits as 32 MiB holds, and a row the quick way
/// leaves to the long way at the end. `cargo test --release -p zhang-query --lib -- --ignored --nocapture worst`
#[test]
#[ignore]
fn the_worst_output_of_decimals_costs() {
    let mut ledger = ledger(TEXT);
    attach(&mut ledger, &[("cells", cells(), false)]);
    let long = "9".repeat(1000);
    let row =
        format!(r#"{{"s": "x", "i": 1, "d": "{long}", "t": "2024-01-31", "b": true, "a": {{"number": "{long}", "currency": "USD"}}, "e": null, "m": null}}"#);
    let count = (abi::MAX_OUTPUT_BYTES - 1000) / (row.len() + 2);
    let mut rows = vec![row; count];
    rows.push(r#"{"s": "x", "unknown": 1}"#.to_owned());
    let text = format!(r#"{{"tables": {{"all": [{}]}}}}"#, rows.join(", "));
    assert!(text.len() <= abi::MAX_OUTPUT_BYTES);
    let started = std::time::Instant::now();
    let (long_way, quick) = both_ways(&ledger, "cells", &text);
    println!(
        "{} bytes, {} decimals of 1000 digits: both ways in {:?}",
        text.len(),
        2 * count,
        started.elapsed()
    );
    assert!(long_way.is_err() && quick.is_none());
}
