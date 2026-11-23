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

/// A ledger of `text`, its main file, its budgets interpreted by the native code through RFC #698's test switch (a bare
/// load runs no budget code): the native provider of the budget tables these tests read.
// TODO(P5): the native provider goes, and with it its tests
pub(crate) fn ledger(text: &str) -> Ledger {
    let dir = tempfile::tempdir().expect("tempdir").keep().canonicalize().expect("tempdir");
    std::fs::write(dir.join("main.zhang"), text).expect("write ledger");
    let source = Arc::new(LocalFileSystemDataSource::new(ZhangDataType {}));
    let options = zhang_core::analysis::LoadOptions::default().with_native_budgets();
    Ledger::load_with_options(dir, "main.zhang".to_owned(), source, zhang_core::clock::Clock::System, options).expect("cannot load ledger")
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
                published_as: "items".to_owned(),
                description: "Items.".to_owned(),
                kind: TableKind::Records(RecordOrder::Published),
                columns: vec![column("item", DataType::Str, None), column("price", DataType::Amount, None)],
                wildcard: vec!["item".to_owned()],
            },
            TableDecl {
                name: "shop_stock".to_owned(),
                published_as: "stock".to_owned(),
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
            published_as: "tags".to_owned(),
            description: "Tags.".to_owned(),
            predicate: "account IN :accounts AND number >= :min".to_owned(),
            params: vec![("accounts".to_owned(), DataType::Set), ("min".to_owned(), DataType::Decimal)],
        }],
        queries: vec![],
    }
}

/// The provider of `manifest`, whose snapshot is published into the ledger.
pub(crate) fn published(manifest: Manifest) -> Provider {
    Provider {
        manifest,
        source: Source::Published,
    }
}

/// Why a catalog of `providers` skips the first it skips; `None` when it skips none.
fn skipped(providers: &[Provider]) -> Option<String> {
    build(providers).skipped.into_iter().next().map(|skipped| skipped.message)
}

fn build_error(change: impl FnOnce(&mut Manifest)) -> String {
    let mut manifest = manifest();
    change(&mut manifest);
    skipped(&[published(manifest)]).expect("the manifest is refused")
}

fn series(manifest: &mut Manifest) -> &mut TableDecl {
    &mut manifest.tables[1]
}

#[test]
fn a_manifest_is_checked_before_its_tables_exist() {
    assert_eq!(skipped(&[published(manifest())]), None);
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
        (
            Box::new(|m| m.tables[0].kind = TableKind::Records(RecordOrder::Columns(vec!["nope".to_owned()]))),
            "its rows are ordered by 'nope', which is no column",
        ),
        (Box::new(|m| m.labels.push(m.labels[0].clone())), "postings already has a column 'shop_tags'"),
        (Box::new(|m| m.tables.push(m.tables[0].clone())), "table 'shop_items' is already a table"),
    ];
    for (change, expected) in cases {
        let err = build_error(change);
        assert!(err.starts_with("test:shop: "), "{err}");
        assert!(err.contains(expected), "{err}");
    }
}

#[test]
fn of_two_providers_declaring_one_name_the_first_keeps_it_and_the_later_is_skipped() {
    let mut other = manifest();
    other.provider = "test:other".to_owned();
    other.tables.remove(0);
    let mut unrelated = manifest();
    unrelated.provider = "test:unrelated".to_owned();
    unrelated.tables = vec![];
    unrelated.labels[0].name = "unrelated_tags".to_owned();
    let extensions = build(&[published(manifest()), published(other.clone()), published(unrelated)]);
    assert_eq!(
        extensions.skipped,
        [Skipped {
            index: 1,
            provider: "test:other".to_owned(),
            message: "test:other: table 'shop_stock' is already a table".to_owned(),
        }]
    );
    // the later provider has none of its tables and columns, the others keep theirs
    let names = extensions.tables.iter().map(|table| table.name().to_owned()).collect::<Vec<_>>();
    assert_eq!(names, ["shop_items", "shop_stock"]);
    let labels = extensions
        .labels
        .iter()
        .map(|(def, column)| (def.name.to_string(), column.provider.to_string()))
        .collect::<Vec<_>>();
    assert_eq!(
        labels,
        [
            ("shop_tags".to_owned(), "test:shop".to_owned()),
            ("unrelated_tags".to_owned(), "test:unrelated".to_owned())
        ]
    );
    assert_eq!(
        extensions.providers.iter().map(|it| it.manifest.provider.as_str()).collect::<Vec<_>>(),
        ["test:shop", "test:unrelated"]
    );
    // its label column too
    other.tables.clear();
    assert_eq!(
        skipped(&[published(manifest()), published(other)]).unwrap(),
        "test:other: column postings.shop_tags: postings already has a column 'shop_tags'"
    );
    // a core name is never a provider's
    let mut core = manifest();
    core.tables[1].name = "accounts".to_owned();
    assert_eq!(skipped(&[published(core)]).unwrap(), "test:shop: table 'accounts' is already a table");
}

#[test]
fn a_catalog_lists_the_extensions_tables_before_errors_and_their_columns_after_the_cores() {
    let catalog = Catalog::with_extensions(&[published(manifest())]).unwrap();
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
fn a_digest_follows_the_manifests_and_their_sources() {
    let digest = |manifests: &[Manifest]| {
        let providers = manifests.iter().cloned().map(published).collect::<Vec<_>>();
        Catalog::with_extensions(&providers).unwrap().digest()
    };
    assert_eq!(digest(&[manifest()]), digest(&[manifest()]));
    assert_eq!(Catalog::core().digest(), digest(&[]));
    assert_eq!(Catalog::standard().digest(), Catalog::with_extensions(native_providers()).unwrap().digest());
    assert_ne!(Catalog::standard().digest(), Catalog::core().digest());
    let mut described = manifest();
    described.tables[0].columns[0].description = "Another.".to_owned();
    assert_ne!(digest(&[manifest()]), digest(&[described]));
    let mut predicate = manifest();
    predicate.labels[0].predicate = "account IN :accounts".to_owned();
    assert_ne!(digest(&[manifest()]), digest(&[predicate]));
    // the native provider's tables served from a snapshot published into the ledger are other tables
    let republished = native_providers().iter().map(|native| published(native.manifest.clone())).collect::<Vec<_>>();
    assert_ne!(Catalog::with_extensions(&republished).unwrap().digest(), Catalog::standard().digest());
    assert_ne!(published(manifest()).digest(), published(predicate_changed()).digest());
}

fn predicate_changed() -> Manifest {
    let mut manifest = manifest();
    manifest.labels[0].predicate = "account IN :accounts".to_owned();
    manifest
}

#[test]
fn a_query_of_an_extensions_data_runs_only_on_a_catalog_with_its_digest() {
    let ledger = ledger("1970-01-01 open Assets:Bank\n");
    let today = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let catalog = Catalog::with_extensions(&[published(manifest())]).unwrap();
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
        published_as: "tags".to_owned(),
        description: String::new(),
        predicate: predicate.to_owned(),
        params: vec![("accounts".to_owned(), DataType::Set), ("min".to_owned(), DataType::Decimal)],
    };
    let provider = published(manifest());
    let column = Arc::new(LabelColumn::compile(&Arc::from("test:shop"), provider.digest(), provider.source, &decl).unwrap());
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
    let catalog = Catalog::with_extensions(&[published(widest)]).unwrap();
    assert_eq!(catalog.find("shop_items").unwrap().columns().len(), 64);
    let err = build_error(|m| {
        m.tables[0].columns = columns(65);
        m.tables[0].wildcard = vec!["c0".to_owned()];
    });
    assert_eq!(err, "test:shop: table #shop_items: a table has at most 64 columns");
}

mod published;

#[test]
fn a_catalog_has_at_most_16_label_columns_shared_by_its_providers() {
    let label = |name: String| LabelDecl {
        table: "postings".to_owned(),
        published_as: name.clone(),
        name,
        description: String::new(),
        predicate: "account IN :accounts".to_owned(),
        params: vec![("accounts".to_owned(), DataType::Set)],
    };
    let labels = |provider: &str, from: usize, to: usize| {
        published(Manifest {
            provider: provider.to_owned(),
            tables: vec![],
            labels: (from..to).map(|it| label(format!("tag{it}"))).collect(),
            queries: vec![],
        })
    };
    let catalog = Catalog::with_extensions(&[labels("test:one", 0, 16)]).unwrap();
    assert_eq!(catalog.postings().columns().len(), crate::table::COLUMNS.len() + 16);
    let err = skipped(&[labels("test:one", 0, 17)]).unwrap();
    assert_eq!(err, "test:one: column postings.tag16: a catalog has at most 16 label columns");
    // the limit is the catalog's, whichever provider reaches it: that one is skipped, and the others keep theirs
    let extensions = build(&[labels("test:one", 0, 10), labels("test:two", 10, 17), labels("test:three", 20, 26)]);
    let messages = extensions.skipped.iter().map(|it| it.message.as_str()).collect::<Vec<_>>();
    assert_eq!(messages, ["test:two: column postings.tag16: a catalog has at most 16 label columns"]);
    assert_eq!(extensions.labels.len(), 16);
    assert_eq!(extensions.labels.last().unwrap().0.name, "tag25");
    // the native providers leave room: zhang:budget has one
    assert_eq!(native_providers().iter().map(|it| it.manifest.labels.len()).sum::<usize>(), 1);
}

/// A named query `name` of `bql` with `params`.
fn named(name: &str, bql: &str, params: &[(&str, DataType)]) -> NamedQuery {
    NamedQuery {
        name: name.to_owned(),
        description: format!("The {name}."),
        bql: bql.to_owned(),
        params: params.iter().map(|(name, ty)| ((*name).to_owned(), *ty)).collect(),
    }
}

/// The test provider with two named queries: one of its table, one of its label column.
fn with_queries() -> Manifest {
    let mut manifest = manifest();
    manifest.queries = vec![
        named(
            "shop.item",
            "SELECT item, price FROM #shop_items WHERE item = :item",
            &[("item", DataType::Str)],
        ),
        named("shop.tagged", "SELECT account, number WHERE :tag IN shop_tags", &[("tag", DataType::Str)]),
    ];
    manifest
}

/// Why a catalog of `providers` skips the first it skips, its named queries compiled against it; `None` when it skips
/// none.
fn catalog_skips(providers: &[Provider]) -> Option<String> {
    Catalog::build(providers).1.into_iter().next().map(|skipped| skipped.message)
}

#[test]
fn a_providers_named_queries_are_the_catalogs_each_compiling_against_it() {
    let (catalog, skipped) = Catalog::build(&[published(with_queries())]);
    assert_eq!(skipped, vec![]);
    let names = catalog
        .named_queries()
        .map(|(provider, query)| (provider, query.name.as_str()))
        .collect::<Vec<_>>();
    assert_eq!(names, [("test:shop", "shop.item"), ("test:shop", "shop.tagged")]);
    let ledger = ledger("1970-01-01 open Assets:Cash\n");
    for (_, query) in catalog.named_queries() {
        let compiled = Query::compile_in(&catalog, &query.bql, &query.param_types()).unwrap();
        let params = Params::new().bind(query.params[0].0.as_str(), "apple");
        // it reads the provider's data, which a ledger without the catalog does not have
        assert!(compiled.execute(&ledger, &params).is_err(), "{}", query.name);
    }
    // they are part of the digest
    assert_ne!(catalog.digest(), Catalog::with_extensions(&[published(manifest())]).unwrap().digest());
    // a catalog without the provider has none
    assert_eq!(Catalog::core().named_queries().count(), 0);

    type Change = Box<dyn FnOnce(&mut Manifest)>;
    let cases: Vec<(Change, &str)> = vec![
        (
            Box::new(|m| m.queries[0].name = "item".to_owned()),
            "test:shop: query item: a query's name is dotted lower-case words",
        ),
        (
            Box::new(|m| m.queries[0].name = "shop.Item".to_owned()),
            "test:shop: query shop.Item: a query's name is dotted lower-case words",
        ),
        (
            Box::new(|m| m.queries[1].name = "shop.item".to_owned()),
            "test:shop: query shop.item is already a named query",
        ),
        (
            Box::new(|m| m.queries[0].params.push(("item".to_owned(), DataType::Str))),
            "test:shop: query shop.item: it declares the parameter :item twice",
        ),
        (
            Box::new(|m| m.queries[0].params[0].1 = DataType::Amount),
            "test:shop: query shop.item: parameter :item is a amount, which a JSON value cannot give",
        ),
        (
            Box::new(|m| m.queries[0].params.push(("unused".to_owned(), DataType::Int))),
            "test:shop: query shop.item: it declares the parameter :unused, which it does not use",
        ),
        (
            Box::new(|m| m.queries[0].bql = "SELECT nope FROM #shop_items".to_owned()),
            "test:shop: query shop.item: unknown column",
        ),
        (Box::new(|m| m.queries[0].params.clear()), "test:shop: query shop.item: "),
    ];
    for (change, message) in cases {
        let mut manifest = with_queries();
        change(&mut manifest);
        let error = catalog_skips(&[published(manifest)]).expect("the manifest is refused");
        assert!(error.starts_with(message), "{error}");
    }
}

#[test]
fn a_provider_whose_query_does_not_compile_has_nothing_and_the_others_keep_theirs() {
    let mut broken = with_queries();
    broken.queries[1].bql = "SELECT account WHERE :tag IN no_such_column".to_owned();
    let other = Manifest {
        provider: "test:other".to_owned(),
        tables: vec![],
        labels: vec![],
        queries: vec![named("other.accounts", "SELECT account FROM #accounts", &[])],
    };
    let (catalog, skipped) = Catalog::build(&[published(broken), published(other)]);
    assert_eq!(
        skipped.iter().map(|it| (it.index, it.provider.as_str())).collect::<Vec<_>>(),
        [(0, "test:shop")]
    );
    assert!(skipped[0].message.starts_with("test:shop: query shop.tagged: "), "{}", skipped[0].message);
    assert!(catalog.find("shop_items").is_none(), "a skipped provider has no table");
    let names = catalog.named_queries().map(|(_, query)| query.name.as_str()).collect::<Vec<_>>();
    assert_eq!(names, ["other.accounts"]);
    // a query of a later provider may read the tables of an earlier one
    let mut reader = Manifest {
        provider: "test:reader".to_owned(),
        tables: vec![],
        labels: vec![],
        queries: vec![named("reader.items", "SELECT count(*) FROM #shop_items", &[])],
    };
    assert_eq!(catalog_skips(&[published(manifest()), published(reader.clone())]), None);
    // and is skipped with it, when the earlier one is
    reader.queries[0].bql = "SELECT count(*) FROM #shop_items WHERE item = 'x'".to_owned();
    let mut broken = manifest();
    broken.queries = vec![named("shop.nope", "SELECT nope", &[])];
    let (_, skipped) = Catalog::build(&[published(broken), published(reader)]);
    assert_eq!(skipped.iter().map(|it| it.index).collect::<Vec<_>>(), [0, 1]);
}

#[test]
fn an_analyzers_queries_are_named_after_its_namespace_but_the_budget_pages() {
    use zhang_shared::plugin_abi::analysis as wire;
    let declared = |namespace: &str, name: &str, ty: &str| -> wire::Manifest {
        serde_json::from_value(serde_json::json!({
            "analysis_abi": 1,
            "namespace": namespace,
            "queries": [{"name": name, "description": "Items.", "bql": "SELECT account FROM #accounts WHERE account = :x",
                         "params": [{"name": "x", "type": ty}]}]
        }))
        .unwrap()
    };
    let read = abi::manifest("shop-plugin", &declared("shop", "items", "str"), false).unwrap();
    assert_eq!(
        read.queries,
        [named("shop.items", "SELECT account FROM #accounts WHERE account = :x", &[("x", DataType::Str)])].map(|it| NamedQuery {
            description: "Items.".to_owned(),
            ..it
        })
    );
    let error = |namespace: &str, name: &str, ty: &str, legacy: bool| abi::manifest("p", &declared(namespace, name, ty), legacy).unwrap_err();
    assert_eq!(
        error("shop", "items", "amount", false),
        "query items: parameter :x: its type is 'amount', which is not one of bool, int, decimal, str, date, set"
    );
    assert!(error("shop", "shop.items", "str", false).starts_with("query shop.items: a name is a lower-case letter"));
    // a namespace of zhang's own queries is not one an analyzer's queries have
    assert_eq!(
        error("journals", "items", "str", false),
        "query items: it would be named journals.items, in the namespace of zhang's built-in queries: declare another namespace"
    );
    // the budget pages' queries keep their names for zhang:budget, and no other analyzer has them
    let legacy = abi::manifest(abi::BUDGET, &declared("zhang_budget", "budgets.month", "str"), true).unwrap();
    assert_eq!(legacy.queries[0].name, "budgets.month");
    assert!(error("budgets", "month", "str", false).contains("in the namespace of zhang's built-in queries"));
    assert!(error("zhang_budget", "budgets.month", "str", false).starts_with("query budgets.month: a name is a lower-case letter"));
    let other = abi::manifest(abi::BUDGET, &declared("zhang_budget", "spent", "str"), true).unwrap();
    assert_eq!(other.queries[0].name, "zhang_budget.spent");
}

#[test]
fn the_standard_catalog_has_the_queries_of_the_budget_pages_from_zhang_budget() {
    let catalog = Catalog::standard();
    let names = catalog
        .named_queries()
        .map(|(provider, query)| (provider, query.name.as_str()))
        .collect::<Vec<_>>();
    let budget = abi::BUDGET;
    assert_eq!(
        names,
        [
            (budget, "budgets.month"),
            (budget, "budgets.budget"),
            (budget, "budgets.budget_month"),
            (budget, "budgets.events"),
            (budget, "budgets.postings")
        ]
    );
    assert_eq!(names.iter().map(|(_, name)| *name).collect::<Vec<_>>(), abi::LEGACY_QUERIES);
    // a native provider's queries are checked here, as the catalog checks an analyzer's
    for (_, query) in catalog.named_queries() {
        query.check(catalog).unwrap_or_else(|err| panic!("{err}"));
    }
}
