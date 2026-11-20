//! Tests of the extension tables: the checks of a manifest, the catalog, label columns, and the digest a query keeps.
//! The series generation is tested in [`super::series`], `zhang:budget` in [`super::budget`] and, through BQL, by the
//! engine's budget suites and #835's budget contract.

use std::sync::Arc;

use chrono::NaiveDate;
use zhang_core::data_source::LocalFileSystemDataSource;
use zhang_core::data_type::text::ZhangDataType;
use zhang_core::ledger::Ledger;

use super::*;
use crate::catalog::{Catalog, TableRef};
use crate::params::Params;
use crate::projector::{set_membership, Projection};
use crate::table::{Dataset, RowRef};
use crate::value::Value;
use crate::{ParamTypes, Query, QueryErrorKind};

/// A ledger of `text`, its main file.
pub(crate) fn ledger(text: &str) -> Ledger {
    let dir = tempfile::tempdir().expect("tempdir").keep();
    std::fs::write(dir.join("main.zhang"), text).expect("write ledger");
    let source = LocalFileSystemDataSource::new(ZhangDataType {});
    Ledger::load_with_data_source(dir, "main.zhang".to_owned(), Arc::new(source)).expect("cannot load ledger")
}

fn column(name: &str, ty: DataType, fill: Option<Fill>) -> ColumnDecl {
    ColumnDecl {
        name: name.to_owned(),
        ty,
        description: format!("The {name}."),
        fill,
    }
}

/// A test provider with a record table, a series table and a label column.
fn manifest() -> Manifest {
    Manifest {
        provider: "test:shop".to_owned(),
        tables: vec![
            TableDecl {
                name: "shop_items".to_owned(),
                description: "Items.".to_owned(),
                kind: TableKind::Records,
                columns: vec![column("item", DataType::Str, None), column("price", DataType::Amount, None)],
                wildcard: vec!["item".to_owned()],
            },
            TableDecl {
                name: "shop_stock".to_owned(),
                description: "Stock per month.".to_owned(),
                kind: TableKind::Series(SeriesDecl {
                    period: "date".to_owned(),
                    unit: PeriodUnit::Month,
                    derive: vec![("year".to_owned(), DatePart::Year)],
                    partition: vec!["item".to_owned()],
                    extend_to_today: false,
                }),
                columns: vec![
                    column("item", DataType::Str, Some(Fill::Carry)),
                    column("date", DataType::Date, None),
                    column("year", DataType::Int, None),
                    column("opening", DataType::Int, Some(Fill::Previous("closing".to_owned()))),
                    column("sold", DataType::Int, Some(Fill::Zero)),
                    column("closing", DataType::Int, Some(Fill::Carry)),
                ],
                wildcard: vec!["item".to_owned(), "date".to_owned()],
            },
        ],
        labels: vec![LabelDecl {
            table: "postings".to_owned(),
            name: "shop_tags".to_owned(),
            description: "Tags.".to_owned(),
            predicate: "account IN :accounts AND number >= :min".to_owned(),
            params: vec![("accounts".to_owned(), DataType::Set), ("min".to_owned(), DataType::Decimal)],
        }],
    }
}

fn build_error(change: impl FnOnce(&mut Manifest)) -> String {
    let mut manifest = manifest();
    change(&mut manifest);
    match build(&[manifest]) {
        Ok(_) => panic!("the manifest is accepted"),
        Err(err) => err,
    }
}

fn series(manifest: &mut Manifest) -> &mut TableDecl {
    &mut manifest.tables[1]
}

#[test]
fn a_manifest_is_checked_before_its_tables_exist() {
    assert!(build(&[manifest()]).is_ok());
    type Change = Box<dyn FnOnce(&mut Manifest)>;
    let cases: Vec<(Change, &str)> = vec![
        (Box::new(|m| m.tables[0].name = "prices".to_owned()), "table 'prices' is already a table"),
        (
            Box::new(|m| m.tables[1].name = "shop_items".to_owned()),
            "table 'shop_items' is already a table",
        ),
        (Box::new(|m| m.tables[0].name = "Shop".to_owned()), "a name is a lower-case letter"),
        (
            Box::new(|m| m.tables[0].wildcard.push("nope".to_owned())),
            "its wildcard names 'nope', which is no column",
        ),
        (
            Box::new(|m| m.tables[0].columns[1].fill = Some(Fill::Carry)),
            "only the columns of a series have",
        ),
        (Box::new(|m| m.tables[0].columns[1].name = "item".to_owned()), "two columns are named 'item'"),
        (Box::new(|m| series(m).columns[4].fill = None), "column 'sold' needs a fill"),
        (
            Box::new(|m| series(m).columns[1].fill = Some(Fill::Carry)),
            "is generated from the period and has no fill",
        ),
        (
            Box::new(|m| series(m).columns[0].fill = Some(Fill::Zero)),
            "column 'item' is a str, which has no zero",
        ),
        (
            Box::new(|m| series(m).columns[3].fill = Some(Fill::Previous("sold".to_owned()))),
            "must be another column of its type filled with carry",
        ),
        (
            Box::new(|m| series(m).columns[3].fill = Some(Fill::Previous("item".to_owned()))),
            "must be another column of its type filled with carry",
        ),
        (Box::new(|m| series(m).columns[2].ty = DataType::Str), "a part of the period, is an int"),
        (Box::new(|m| series(m).columns[1].ty = DataType::Str), "the period 'date' is a date"),
        (
            Box::new(|m| {
                let TableKind::Series(decl) = &mut series(m).kind else { unreachable!() };
                decl.partition = vec!["sold".to_owned()];
            }),
            "partition column 'sold' is filled with carry",
        ),
        (Box::new(|m| m.labels[0].name = "account".to_owned()), "postings already has a column 'account'"),
        (Box::new(|m| m.labels[0].table = "prices".to_owned()), "label columns are columns of postings"),
        (Box::new(|m| m.labels[0].predicate = "account IN".to_owned()), "its predicate does not parse"),
        (
            Box::new(|m| m.labels[0].predicate = "account".to_owned()),
            "its predicate does not compile: WHERE expects a boolean",
        ),
        (
            Box::new(|m| m.labels[0].predicate = "nope = 1".to_owned()),
            "its predicate does not compile: unknown column 'nope'",
        ),
        (Box::new(|m| m.labels[0].predicate = "'x' IN budgets".to_owned()), "unknown column 'budgets'"),
        (Box::new(|m| m.labels[0].predicate = "count(*) > 1".to_owned()), "does not compile"),
        (
            Box::new(|m| m.labels[0].predicate = "account_balance IS NOT NULL".to_owned()),
            "its predicate reads a running balance",
        ),
        (
            Box::new(|m| m.labels[0].params.push(("label".to_owned(), DataType::Str))),
            "a parameter is named label",
        ),
    ];
    for (change, expected) in cases {
        let err = build_error(change);
        assert!(err.starts_with("test:shop: "), "{err}");
        assert!(err.contains(expected), "{err}");
    }
    // two providers with one name: the first keeps it
    let err = build(&[manifest(), manifest()]).err().unwrap();
    assert!(err.contains("table 'shop_items' is already a table"), "{err}");
    let mut other = manifest();
    other.tables.clear();
    let err = build(&[manifest(), other]).err().unwrap();
    assert!(err.contains("postings already has a column 'shop_tags'"), "{err}");
}

#[test]
fn a_catalog_lists_the_extensions_tables_before_errors_and_their_columns_after_the_cores() {
    let catalog = Catalog::with_extensions(&[manifest()]).unwrap();
    let names = catalog.tables().iter().map(|table| table.name()).collect::<Vec<_>>();
    assert_eq!(
        names,
        [
            "postings",
            "entries",
            "transactions",
            "prices",
            "balances",
            "notes",
            "events",
            "documents",
            "accounts",
            "commodities",
            "shop_items",
            "shop_stock",
            "errors"
        ]
    );
    let postings = catalog.postings();
    assert_eq!(postings.columns().len(), crate::table::COLUMNS.len() + 1);
    assert_eq!(postings.columns().last().unwrap().name, "shop_tags");
    assert_eq!(postings.columns().last().unwrap().provided_by.as_deref(), Some("test:shop"));
    assert_eq!(postings.provided_by(), None);
    let items = catalog.find("shop_items").unwrap();
    assert_eq!(items.provided_by(), Some("test:shop"));
    assert!(items.columns().iter().all(|column| column.provided_by.as_deref() == Some("test:shop")));
    assert!(catalog.find("prices").unwrap().provided_by().is_none());
    // the standard catalog: the budget tables where the schema has always listed them
    let standard = Catalog::standard().tables().iter().map(|table| table.name()).collect::<Vec<_>>();
    assert_eq!(&standard[10..], ["budgets", "budget_events", "budget_definitions", "errors"]);
    assert_eq!(Catalog::standard().postings().columns().last().unwrap().name, "budgets");
}

#[test]
fn a_digest_follows_the_manifests() {
    let digest = |manifests: &[Manifest]| Catalog::with_extensions(manifests).unwrap().digest();
    assert_eq!(digest(&[manifest()]), digest(&[manifest()]));
    assert_eq!(Catalog::core().digest(), digest(&[]));
    assert_eq!(Catalog::standard().digest(), digest(native_manifests()));
    assert_ne!(Catalog::standard().digest(), Catalog::core().digest());
    let mut described = manifest();
    described.tables[0].columns[0].description = "Another.".to_owned();
    assert_ne!(digest(&[manifest()]), digest(&[described]));
    let mut predicate = manifest();
    predicate.labels[0].predicate = "account IN :accounts".to_owned();
    assert_ne!(digest(&[manifest()]), digest(&[predicate]));
}

#[test]
fn a_query_of_an_extensions_data_runs_only_on_a_catalog_with_its_digest() {
    let ledger = ledger("1970-01-01 open Assets:Bank\n");
    let today = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let catalog = Catalog::with_extensions(&[manifest()]).unwrap();
    let compile = |sql: &str| Query::compile_in(&catalog, sql, &ParamTypes::default()).unwrap();
    for sql in [
        "SELECT item FROM #shop_items",
        "SELECT count(*) FROM #shop_stock",
        "SELECT shop_tags",
        "SELECT count(*) WHERE 'x' IN shop_tags",
    ] {
        let err = compile(sql).execute_at(&ledger, &Params::new(), today).unwrap_err();
        assert_eq!(err.kind, QueryErrorKind::Compile, "{sql}");
        assert_eq!(
            err.message, "the ledger's tables changed since the query was compiled; compile it again",
            "{sql}"
        );
    }
    // the core's data runs on every ledger
    for sql in ["SELECT count(*)", "SELECT account FROM #accounts"] {
        assert!(compile(sql).execute_at(&ledger, &Params::new(), today).is_ok(), "{sql}");
    }
    // the ledger's own catalog
    let budgets = Query::compile("SELECT budgets").unwrap();
    assert!(budgets.execute_at(&ledger, &Params::new(), today).is_ok());
    assert_eq!(Catalog::of_ledger(&ledger).digest(), Catalog::standard().digest());
}

#[test]
fn a_query_compiled_once_reads_the_rows_of_the_ledger_it_runs_on() {
    // the tables of a provider, and postings with its label column, are tables the catalog owns and shares: a compiled
    // query keeps handles to them, never their rows
    let catalog = Catalog::standard();
    for table in ["budget_definitions", "budgets", "postings"] {
        assert!(matches!(catalog.find(table), Some(TableRef::Shared(_))), "{table}");
    }
    let today = NaiveDate::from_ymd_opt(2024, 3, 1).unwrap();
    let text = |name: &str| {
        format!(
            "1970-01-01 open Expenses:Food\n  budget: {name}\n1970-01-01 open Assets:Bank\n\
             2024-01-01 budget {name} CNY\n\
             2024-01-02 * \"lunch\"\n  Expenses:Food 10 CNY\n  Assets:Bank -10 CNY\n"
        )
    };
    let (before, after) = (ledger(&text("Food")), ledger(&text("Meals")));
    for sql in [
        "SELECT name FROM #budget_definitions",
        "SELECT name, activity FROM #budgets WHERE date = 2024-01-01",
        "SELECT budgets",
        "SELECT count(*) WHERE 'Food' IN budgets",
    ] {
        let query = Query::compile(sql).unwrap();
        let run = |ledger: &Ledger| format!("{:?}", query.execute_at(ledger, &Params::new(), today).unwrap().rows);
        let fresh = |ledger: &Ledger| format!("{:?}", Query::compile(sql).unwrap().execute_at(ledger, &Params::new(), today).unwrap().rows);
        assert_eq!(run(&before), fresh(&before), "{sql}");
        assert_eq!(run(&after), fresh(&after), "{sql}");
        assert_ne!(run(&before), run(&after), "{sql}");
    }
}

/// The labels of `predicate` on every row of a ledger: the column's value, and the in-place test of every label.
fn labels_of(predicate: &str, labels: &[Label]) -> (Option<String>, Vec<(String, Vec<String>)>) {
    let decl = LabelDecl {
        table: "postings".to_owned(),
        name: "shop_tags".to_owned(),
        description: String::new(),
        predicate: predicate.to_owned(),
        params: vec![("accounts".to_owned(), DataType::Set), ("min".to_owned(), DataType::Decimal)],
    };
    let column = Arc::new(LabelColumn::compile(&Arc::from("test:shop"), &decl).unwrap());
    let indexed = column.index.as_ref().map(|(column, param)| format!("{} {}", column.name(), param));
    let ledger = ledger(
        "1970-01-01 open Assets:Bank\n1970-01-01 open Expenses:Food\n1970-01-01 open Expenses:Fun\n\
         2024-01-02 * \"a\"\n  Expenses:Food 12.50 USD\n  Assets:Bank -12.50 USD\n\
         2024-01-03 * \"b\"\n  Expenses:Fun 3 USD\n  Expenses:Food 40 USD\n  Assets:Bank\n",
    );
    let all = Projection::all();
    let data = Dataset::new(&ledger, NaiveDate::from_ymd_opt(2026, 1, 1).unwrap().and_time(chrono::NaiveTime::MIN), &all);
    let resolved = Labels::resolve(&column, labels);
    let rows = data
        .rows
        .iter()
        .map(|row| {
            let set = resolved.of_row(&data, row).into_iter().collect::<Vec<_>>();
            for label in labels {
                assert_eq!(resolved.holds(&data, row, &label.label), set.contains(&label.label), "{}", label.label);
            }
            assert!(!resolved.holds(&data, row, "unknown"));
            (format!("{} {}", row.account, *row.units), set)
        })
        .collect();
    (indexed, rows)
}

#[test]
fn a_label_column_holds_the_labels_whose_predicate_holds() {
    let label = |name: &str, accounts: &[&str], min: &str| Label {
        label: name.to_owned(),
        params: Params::new()
            .bind("accounts", Value::Set(accounts.iter().map(|it| (*it).to_owned()).collect()))
            .bind("min", Value::Decimal(min.parse().unwrap())),
    };
    let labels = [
        label("food", &["Expenses:Food"], "0"),
        label("big", &["Expenses:Food", "Expenses:Fun"], "20"),
        label("all", &["Expenses:Food", "Expenses:Fun", "Assets:Bank"], "-1000"),
        Label {
            label: "none".to_owned(),
            params: Params::new().bind("accounts", Value::Null).bind("min", Value::Null),
        },
    ];
    let (indexed, rows) = labels_of("account IN :accounts AND number >= :min", &labels);
    assert_eq!(indexed.as_deref(), Some("account :accounts"));
    let set = |labels: &[&str]| labels.iter().map(|it| (*it).to_owned()).collect::<Vec<_>>();
    assert_eq!(
        rows,
        [
            ("Expenses:Food 12.50 USD".to_owned(), set(&["all", "food"])),
            ("Assets:Bank -12.50 USD".to_owned(), set(&["all"])),
            ("Expenses:Fun 3 USD".to_owned(), set(&["all"])),
            ("Expenses:Food 40 USD".to_owned(), set(&["all", "big", "food"])),
            ("Assets:Bank -43 USD".to_owned(), set(&["all"])),
        ]
    );
    // the index only skips the labels whose predicate cannot hold
    let (indexed, unindexed) = labels_of("number >= :min AND account IN :accounts", &labels);
    assert_eq!(indexed.as_deref(), Some("account :accounts"));
    assert_eq!(unindexed, rows);
    let (indexed, unindexed) = labels_of("number / :min >= 0 AND account IN :accounts", &labels);
    assert_eq!(indexed, None, "a conjunct that may fail comes first");
    // a predicate that is NULL on a row does not hold there: 12.50 / 0 for `food`
    let food = |rows: &[(String, Vec<String>)]| rows.iter().map(|(_, set)| set.contains(&"food".to_owned())).collect::<Vec<_>>();
    assert_eq!(food(&unindexed), [false, false, false, false, false]);
    let (_, by_label) = labels_of("account IN :accounts AND :label != 'big' AND number >= :min", &labels);
    assert!(by_label.iter().all(|(_, set)| !set.contains(&"big".to_owned())));
}

#[test]
fn the_budgets_column_reads_in_place_as_its_value() {
    let ledger = ledger(
        "1970-01-01 open Expenses:Food\n  budget: Food\n  budget: Treats\n1970-01-01 open Assets:Bank\n\
         2024-01-01 budget Food CNY\n2024-01-01 budget Treats CNY\n2024-01-05 budget-close Treats\n\
         2024-01-02 * \"a\"\n  Expenses:Food 10 CNY\n  Assets:Bank -10 CNY\n\
         2024-01-07 * \"b\"\n  Expenses:Food 10 CNY\n  Assets:Bank -10 CNY\n",
    );
    let all = Projection::all();
    let data = Dataset::new(&ledger, NaiveDate::from_ymd_opt(2026, 1, 1).unwrap().and_time(chrono::NaiveTime::MIN), &all);
    let budgets = Catalog::standard().postings().column("budgets").unwrap();
    let params = Params::new();
    let regexes = crate::executor::RegexCache::default();
    let mut seen = vec![];
    for row in &data.rows {
        let Value::Set(set) = budgets.value(&data, RowRef::Posting(row)) else {
            panic!("a set")
        };
        let env = crate::executor::Env {
            data: Some(&data),
            row: Some(RowRef::Posting(row)),
            aggregates: &[],
            cells: &[],
            running: None,
            params: &params,
            regexes: &regexes,
        };
        let expr = crate::compiler::CExpr::Column(budgets.clone());
        let contains = set_membership(&expr, &env).expect("an in-place test");
        for name in ["Food", "Treats", "Other"] {
            assert_eq!(contains(name), set.contains(name), "{name}");
        }
        seen.push(set.into_iter().collect::<Vec<_>>().join(","));
    }
    assert_eq!(seen, ["Food,Treats", "", "Food", ""]);
}

/// A label predicate that fails with an error is a provider's bug: a debug build panics (a release build reads the label
/// as not holding).
#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "the predicate of shop_tags fails for the label '('")]
fn a_label_predicate_that_fails_panics_in_a_debug_build() {
    let label = Label {
        label: "(".to_owned(),
        params: Params::new().bind("accounts", Value::Null).bind("min", Value::Null),
    };
    labels_of("account ~ :label", &[label]);
}

#[test]
fn an_extensions_table_has_at_most_64_columns() {
    let columns = |count: usize| (0..count).map(|it| column(&format!("c{it}"), DataType::Str, None)).collect::<Vec<_>>();
    let mut widest = manifest();
    widest.tables[0].columns = columns(64);
    widest.tables[0].wildcard = vec!["c0".to_owned()];
    let catalog = Catalog::with_extensions(&[widest]).unwrap();
    assert_eq!(catalog.find("shop_items").unwrap().columns().len(), 64);
    let err = build_error(|m| {
        m.tables[0].columns = columns(65);
        m.tables[0].wildcard = vec!["c0".to_owned()];
    });
    assert_eq!(err, "test:shop: table #shop_items: a table has at most 64 columns");
}

#[test]
fn a_catalog_has_at_most_16_label_columns_shared_by_its_providers() {
    let label = |name: String| LabelDecl {
        table: "postings".to_owned(),
        name,
        description: String::new(),
        predicate: "account IN :accounts".to_owned(),
        params: vec![("accounts".to_owned(), DataType::Set)],
    };
    let labels = |provider: &str, from: usize, to: usize| Manifest {
        provider: provider.to_owned(),
        tables: vec![],
        labels: (from..to).map(|it| label(format!("tag{it}"))).collect(),
    };
    let catalog = Catalog::with_extensions(&[labels("test:one", 0, 16)]).unwrap();
    assert_eq!(catalog.postings().columns().len(), crate::table::COLUMNS.len() + 16);
    let err = build(&[labels("test:one", 0, 17)]).err().unwrap();
    assert_eq!(err, "test:one: column postings.tag16: a catalog has at most 16 label columns");
    // the limit is the catalog's, whichever provider reaches it
    let err = build(&[labels("test:one", 0, 10), labels("test:two", 10, 17)]).err().unwrap();
    assert_eq!(err, "test:two: column postings.tag16: a catalog has at most 16 label columns");
    // the native providers leave room: zhang:budget has one
    assert_eq!(native_manifests().iter().map(|it| it.labels.len()).sum::<usize>(), 1);
}
