//! The zhang-specific tables, `#budgets` and `#errors`: golden results over the budget and
//! error fixtures of `integration-tests` and over a ledger that exercises every column.
//!
//! Beanquery has no such tables, so there is no oracle; the expected values follow the budget
//! and error APIs, and `zhang-server/tests/query_zhang_tables.rs` checks the same ledgers
//! against `GET /api/budgets` and `GET /api/errors`.

mod common;

use std::path::PathBuf;
use std::sync::OnceLock;

use chrono::NaiveDate;
use zhang_core::ledger::Ledger;
use zhang_query::{DataType, ExecuteOptions, Params, Query, QueryErrorKind, Value};

/// `integration-tests/query-zhang-tables`: a ledger in two files with budgets that carry over,
/// transfer, overspend and close, and errors of several kinds in both files.
fn ledger() -> &'static Ledger {
    static LEDGER: OnceLock<Ledger> = OnceLock::new();
    LEDGER.get_or_init(|| fixture("query-zhang-tables"))
}

fn fixture(name: &str) -> Ledger {
    common::load_ledger(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../integration-tests").join(name), "main.zhang")
}

/// `today()` of the executions: before every entry of the fixtures, so that the current month
/// does not extend the series of `#budgets` (`zhang-query/tests/data_tables.rs` tests that).
fn today() -> NaiveDate {
    NaiveDate::from_ymd_opt(2000, 1, 1).unwrap()
}

fn run(ledger: &Ledger, sql: &str) -> Vec<Vec<String>> {
    run_at(ledger, sql, today())
}

fn run_at(ledger: &Ledger, sql: &str, today: NaiveDate) -> Vec<Vec<String>> {
    let result = Query::compile(sql)
        .and_then(|query| query.execute_at(ledger, &Params::new(), today))
        .unwrap_or_else(|err| panic!("{}: {}", sql, err));
    result.rows.iter().map(|row| row.iter().map(Value::to_string).collect()).collect()
}

fn query(sql: &str) -> Vec<Vec<String>> {
    run(ledger(), sql)
}

fn rows(rows: &[&[&str]]) -> Vec<Vec<String>> {
    rows.iter().map(|row| row.iter().map(|it| it.to_string()).collect()).collect()
}

#[test]
fn budgets_of_the_budget_fixture_are_the_budget_pages() {
    let expected = rows(&[
        &["3C_Devices", "2023-11-01", "0 CNY", "0 CNY", "0 CNY"],
        &["3C_Devices", "2023-12-01", "0 CNY", "0 CNY", "0 CNY"],
        &["eat-with-friend", "2023-12-01", "0 CNY", "0 CNY", "0 CNY"],
        &["electric-fee", "2023-12-01", "0 CNY", "0 CNY", "0 CNY"],
        &["food", "2023-12-01", "1000 CNY", "50 CNY", "950 CNY"],
        &["game_expense", "2023-12-01", "200.1 CNY", "0 CNY", "200.1 CNY"],
        &["house-rent", "2023-12-01", "0 CNY", "0 CNY", "0 CNY"],
        &["insurance", "2023-12-01", "0 CNY", "0 CNY", "0 CNY"],
        &["vacation", "2023-12-01", "0 CNY", "0 CNY", "0 CNY"],
        &["water-fee", "2023-12-01", "0 CNY", "0 CNY", "0 CNY"],
    ]);
    for name in ["budget-sytem-syntax-and-category", "budget-sytem-syntax-and-category-multiple-file"] {
        let ledger = fixture(name);
        // the table's own order is by name, then month
        assert_eq!(run(&ledger, "SELECT * FROM #budgets"), expected, "{}", name);
        assert_eq!(run(&ledger, "SELECT * FROM #budgets ORDER BY name, date"), expected, "{}", name);
        assert_eq!(
            run(
                &ledger,
                "SELECT name, alias, category, currency, year, month, added, accounts, closed, meta('alias') FROM #budgets WHERE name IN ('food', 'game_expense')"
            ),
            rows(&[
                &["food", "外食", "生活开销｜55%", "CNY", "2023", "12", "1000 CNY", "Expenses:Food", "FALSE", "外食"],
                &["game_expense", "游戏消费", "精神食粮｜15%", "CNY", "2023", "12", "200.1 CNY", "", "FALSE", "游戏消费"],
            ]),
            "{}",
            name
        );
        // the UI groups budgets by category
        assert_eq!(
            run(
                &ledger,
                "SELECT category, count(*) AS budgets, sum(number(assigned)), sum(number(activity)), sum(number(available)) FROM #budgets WHERE date = 2023-12-01 GROUP BY category ORDER BY category"
            ),
            rows(&[
                &["生活开销｜55%", "5", "1000", "50", "950"],
                &["精神食粮｜15%", "2", "200.1", "0", "200.1"],
                &["长期持有｜10%", "2", "0", "0", "0"],
            ]),
            "{}",
            name
        );
    }
}

#[test]
fn budgets_carry_over_months_without_entries_and_count_transfers() {
    assert_eq!(
        query("SELECT name, alias, category, date, assigned, added, activity, available, accounts, closed FROM #budgets"),
        rows(&[
            &[
                "food",
                "NULL",
                "Living",
                "2024-01-01",
                "500 CNY",
                "500 CNY",
                "200 CNY",
                "300 CNY",
                "Expenses:Dining, Expenses:Food",
                "FALSE"
            ],
            // February only has the activity of a posting
            &[
                "food",
                "NULL",
                "Living",
                "2024-02-01",
                "300 CNY",
                "0 CNY",
                "10 CNY",
                "290 CNY",
                "Expenses:Dining, Expenses:Food",
                "FALSE"
            ],
            // 290 carried over, 200 added and 50 transferred in
            &[
                "food",
                "NULL",
                "Living",
                "2024-03-01",
                "540 CNY",
                "250 CNY",
                "650 CNY",
                "-110 CNY",
                "Expenses:Dining, Expenses:Food",
                "FALSE"
            ],
            // after the last transaction, through the month planned ahead
            &[
                "food",
                "NULL",
                "Living",
                "2024-04-01",
                "-110 CNY",
                "0 CNY",
                "0 CNY",
                "-110 CNY",
                "Expenses:Dining, Expenses:Food",
                "FALSE"
            ],
            &[
                "food",
                "NULL",
                "Living",
                "2024-05-01",
                "-110 CNY",
                "0 CNY",
                "0 CNY",
                "-110 CNY",
                "Expenses:Dining, Expenses:Food",
                "FALSE"
            ],
            &[
                "food",
                "NULL",
                "Living",
                "2024-06-01",
                "-10 CNY",
                "100 CNY",
                "0 CNY",
                "-10 CNY",
                "Expenses:Dining, Expenses:Food",
                "FALSE"
            ],
            &[
                "fun",
                "Fun money",
                "NULL",
                "2024-01-01",
                "100 CNY",
                "100 CNY",
                "30 CNY",
                "70 CNY",
                "Expenses:Fun",
                "FALSE"
            ],
            // a month without any entry carries the available amount over
            &[
                "fun",
                "Fun money",
                "NULL",
                "2024-02-01",
                "70 CNY",
                "0 CNY",
                "0 CNY",
                "70 CNY",
                "Expenses:Fun",
                "FALSE"
            ],
            &[
                "fun",
                "Fun money",
                "NULL",
                "2024-03-01",
                "20 CNY",
                "-50 CNY",
                "0 CNY",
                "20 CNY",
                "Expenses:Fun",
                "FALSE"
            ],
            // closed by the budget-close of 2024-04-01, its last entry; the price of May does
            // not extend the series
            &[
                "fun",
                "Fun money",
                "NULL",
                "2024-04-01",
                "20 CNY",
                "0 CNY",
                "0 CNY",
                "20 CNY",
                "Expenses:Fun",
                "TRUE"
            ],
        ])
    );
    // meta() reads the `budget` directive; the second, duplicated definition of `fun` is an error
    assert_eq!(
        query("SELECT DISTINCT name, meta('owner'), entry_meta('alias'), any_meta('category') FROM #budgets"),
        rows(&[&["food", "alice", "NULL", "Living"], &["fun", "NULL", "Fun money", "NULL"]])
    );
}

#[test]
fn budget_queries_filter_group_order_and_limit() {
    // what was budgeted and spent over the period, and what is left at its end
    assert_eq!(
        query(
            "SELECT name, sum(number(added)) AS budgeted, sum(number(activity)) AS spent, last(available) FROM #budgets WHERE year = 2024 AND month <= 3 GROUP BY name ORDER BY spent DESC"
        ),
        rows(&[&["food", "750", "860", "-110 CNY"], &["fun", "50", "30", "20 CNY"]])
    );
    // overspent months
    assert_eq!(
        query("SELECT date, name, available FROM #budgets WHERE number(available) < 0 AND NOT closed ORDER BY date DESC LIMIT 2"),
        rows(&[&["2024-06-01", "food", "-10 CNY"], &["2024-05-01", "food", "-110 CNY"]])
    );
    // budgets by account
    assert_eq!(
        query("SELECT DISTINCT name FROM #budgets WHERE 'Expenses:Dining' IN accounts"),
        rows(&[&["food"]])
    );
    // the budgets that were open in a month
    assert_eq!(
        query("SELECT name, available FROM #budgets WHERE date = 2024-04-01 AND NOT closed"),
        rows(&[&["food", "-110 CNY"]])
    );
    assert_eq!(
        query("SELECT count(*), min(date), max(date) FROM #budgets"),
        rows(&[&["10", "2024-01-01", "2024-06-01"]])
    );
}

#[test]
fn errors_of_the_error_fixtures() {
    let ledger = fixture("raise-error-if-posting-commodity-is-not-defined");
    assert_eq!(
        run(&ledger, "SELECT * FROM #errors ORDER BY file, line"),
        rows(&[&["main.zhang", "1970-01-01", "CommodityDoesNotDefine", "NULL", "Try to use a undefined commodity"]])
    );
    assert_eq!(
        run(&ledger, "SELECT kind, count(*) FROM #errors GROUP BY kind"),
        rows(&[&["CommodityDoesNotDefine", "1"]])
    );
    assert_eq!(
        run(&ledger, "SELECT source, line, column, meta('txn_id') IS NOT NULL FROM #errors"),
        rows(&[&["1970-01-01 \"\" \"\"\n  Assets:BankCard 1 C", "5", "1", "TRUE"]])
    );

    let ledger = fixture("should_raise_unbalance_error_for_unbalanced_txn");
    assert_eq!(
        run(&ledger, "SELECT * FROM #errors ORDER BY file, line"),
        rows(&[&["main.zhang", "1970-01-02", "UnbalancedTransaction", "NULL", "Transaction is Unbalanced"]])
    );
    assert_eq!(
        run(&ledger, "SELECT kind, count(*) FROM #errors GROUP BY kind"),
        rows(&[&["UnbalancedTransaction", "1"]])
    );
    assert_eq!(
        run(&ledger, "SELECT source FROM #errors"),
        rows(&[&["1970-01-02 \"A\" \"A\" ^group-txn-1970-01-02\n  Assets:A -10 CNY\n  Expenses:B 100 CNY"]])
    );

    // a ledger without errors has an empty table
    assert!(run(&fixture("budget-sytem-syntax-and-category"), "SELECT * FROM #errors").is_empty());
}

#[test]
fn errors_come_by_file_then_position() {
    assert_eq!(
        query("SELECT file, date, kind, account, source FROM #errors"),
        rows(&[
            // an option has no date
            &[
                "main.zhang",
                "NULL",
                "MultipleOperatingCurrencyDetect",
                "NULL",
                "option \"operating_currency\" \"USD\""
            ],
            &["main.zhang", "2024-01-01", "DefineDuplicatedBudget", "NULL", "2024-01-01 budget fun CNY"],
            &[
                "more.zhang",
                "2024-02-01",
                "AccountDoesNotExist",
                "Expenses:Fodo",
                "2024-02-01 * \"Shop\" \"typo\"\n  Expenses:Fodo 10 CNY\n  Assets:Bank"
            ],
            &[
                "more.zhang",
                "2024-02-02",
                "UnbalancedTransaction",
                "NULL",
                "2024-02-02 * \"Shop\" \"unbalanced\"\n  Expenses:Food 10 CNY\n  Assets:Bank -9 CNY"
            ],
            &[
                "more.zhang",
                "2024-02-03",
                "AccountBalanceCheckError",
                "Assets:Bank",
                "2024-02-03 balance Assets:Bank 0 CNY"
            ],
            &["more.zhang", "2024-02-04", "BudgetDoesNotExist", "NULL", "2024-02-04 budget-add travel 10 CNY"],
        ])
    );
    assert_eq!(query("SELECT kind FROM #errors"), query("SELECT kind FROM #errors ORDER BY file, line"));
    assert_eq!(
        query("SELECT file, count(*) AS n FROM #errors WHERE date IS NULL OR date < 2024-02-04 GROUP BY file ORDER BY n DESC"),
        rows(&[&["more.zhang", "3"], &["main.zhang", "2"]])
    );
    assert_eq!(
        query("SELECT kind, message FROM #errors WHERE account ~ '^Assets' OR kind = 'BudgetDoesNotExist' ORDER BY kind"),
        rows(&[
            &["AccountBalanceCheckError", "Account does not pass the balance check"],
            &["BudgetDoesNotExist", "Budget does not exist"],
        ])
    );
    // meta() reads what zhang records about the error, and metas lists it all, sorted by key
    assert_eq!(
        query("SELECT kind FROM #errors WHERE meta('txn_id') IS NOT NULL"),
        rows(&[&["UnbalancedTransaction"]])
    );
    assert_eq!(
        query("SELECT kind, metas FROM #errors WHERE kind IN ('AccountBalanceCheckError', 'MultipleOperatingCurrencyDetect')"),
        rows(&[
            &["MultipleOperatingCurrencyDetect", ""],
            &["AccountBalanceCheckError", "account_name: Assets:Bank"],
        ])
    );
    let txn_id = query("SELECT meta('txn_id') FROM #errors WHERE kind = 'UnbalancedTransaction'");
    assert_eq!(
        query("SELECT str(metas) FROM #errors WHERE kind = 'UnbalancedTransaction'"),
        rows(&[&[format!("txn_id: {}", txn_id[0][0]).as_str()]])
    );
    assert_eq!(
        query("SELECT kind, file FROM #errors ORDER BY date DESC LIMIT 1"),
        rows(&[&["BudgetDoesNotExist", "more.zhang"]])
    );
}

#[test]
fn explain_and_projection_of_the_zhang_tables() {
    let compiled = Query::compile("SELECT kind, count(*) FROM #errors WHERE date >= 2024-01-01 GROUP BY kind").unwrap();
    assert_eq!(compiled.table(), "errors");
    assert_eq!(compiled.referenced_columns(), vec!["date", "kind"]);
    assert_eq!(
        compiled.explain(),
        "table: #errors\n\
         target 0: kind = kind : str\n\
         target 1: count(*) = agg#0 : int\n\
         agg#0: count(*)\n\
         filter: (date >= 2024-01-01)\n\
         group by: [0]\n\
         project: [date, kind] (2 of 12 columns)\n"
    );
    let compiled = Query::compile("SELECT name, sum(number(activity)) FROM #budgets WHERE 'Expenses:Food' IN accounts GROUP BY name").unwrap();
    assert_eq!(compiled.table(), "budgets");
    assert!(
        compiled.explain().contains("project: [accounts, activity, name] (3 of 13 columns)"),
        "{}",
        compiled.explain()
    );
    // without `date` or `accounts`, the builders skip their lookups and the rows stay the same
    assert_eq!(
        query("SELECT kind FROM #errors"),
        query("SELECT kind, date FROM #errors")
            .into_iter()
            .map(|row| vec![row[0].clone()])
            .collect::<Vec<_>>()
    );
    assert_eq!(query("SELECT count(*) FROM #budgets"), rows(&[&["10"]]));
    // columns belong to their table
    let err = Query::compile("SELECT account FROM #budgets").err().unwrap();
    assert_eq!(err.kind, QueryErrorKind::Compile);
    assert!(
        err.message.starts_with("unknown column 'account' in #budgets (its columns are name, alias, "),
        "{}",
        err.message
    );
    let err = Query::compile("SELECT position FROM #errors").err().unwrap();
    assert!(err.message.contains("unknown column 'position' in #errors"), "{}", err.message);
}

#[test]
fn zhang_tables_export_to_csv_and_respect_the_result_budget() {
    let result = Query::compile("SELECT name, date, assigned, activity, available, accounts, closed FROM #budgets WHERE name = 'food' AND month <= 3")
        .unwrap()
        .execute_at(ledger(), &Params::new(), today())
        .unwrap();
    assert_eq!(
        zhang_query::export::to_csv(&result),
        "name,date,assigned (CNY),activity (CNY),available (CNY),accounts,closed\r\n\
         food,2024-01-01,500,200,300,\"Expenses:Dining,Expenses:Food\",FALSE\r\n\
         food,2024-02-01,300,10,290,\"Expenses:Dining,Expenses:Food\",FALSE\r\n\
         food,2024-03-01,540,650,-110,\"Expenses:Dining,Expenses:Food\",FALSE\r\n"
    );
    let result = Query::compile("SELECT kind, account, source FROM #errors WHERE file = 'more.zhang' LIMIT 2")
        .unwrap()
        .execute_at(ledger(), &Params::new(), today())
        .unwrap();
    assert_eq!(
        zhang_query::export::to_csv(&result),
        "kind,account,source\r\n\
         AccountDoesNotExist,Expenses:Fodo,\"2024-02-01 * \"\"Shop\"\" \"\"typo\"\"\n  Expenses:Fodo 10 CNY\n  Assets:Bank\"\r\n\
         UnbalancedTransaction,,\"2024-02-02 * \"\"Shop\"\" \"\"unbalanced\"\"\n  Expenses:Food 10 CNY\n  Assets:Bank -9 CNY\"\r\n"
    );

    let options = |limit| ExecuteOptions {
        today: Some(today()),
        timeout: None,
        max_result_values: Some(limit),
        count_total: false,
    };
    // 10 rows of 5 values plus the 10 generated budget rows, 6 rows of 4 values
    for (sql, values) in [("SELECT * FROM #budgets", 60), ("SELECT file, date, kind, account FROM #errors", 24)] {
        let compiled = Query::compile(sql).unwrap();
        assert!(compiled.execute_with_options(ledger(), &Params::new(), &options(values)).is_ok(), "{}", sql);
        let err = compiled.execute_with_options(ledger(), &Params::new(), &options(values - 1)).unwrap_err();
        assert_eq!(err.kind, QueryErrorKind::TooLarge, "{}", sql);
    }
}

#[test]
fn the_schema_describes_the_zhang_tables() {
    let schema = zhang_query::schema();
    let columns = |name: &str| {
        let table = schema
            .tables
            .iter()
            .find(|table| table.name == name)
            .unwrap_or_else(|| panic!("no table {}", name));
        assert!(!table.description.is_empty());
        table.columns.iter().map(|column| (column.name, column.ty)).collect::<Vec<_>>()
    };
    assert_eq!(
        columns("budgets"),
        vec![
            ("name", DataType::Str),
            ("alias", DataType::Str),
            ("category", DataType::Str),
            ("currency", DataType::Str),
            ("date", DataType::Date),
            ("year", DataType::Int),
            ("month", DataType::Int),
            ("assigned", DataType::Amount),
            ("added", DataType::Amount),
            ("activity", DataType::Amount),
            ("available", DataType::Amount),
            ("accounts", DataType::Set),
            ("closed", DataType::Bool),
        ]
    );
    assert_eq!(
        columns("errors"),
        vec![
            ("kind", DataType::Str),
            ("message", DataType::Str),
            ("file", DataType::Str),
            ("line", DataType::Int),
            ("column", DataType::Int),
            ("date", DataType::Date),
            ("account", DataType::Str),
            ("source", DataType::Str),
            ("id", DataType::Str),
            ("span_start", DataType::Int),
            ("span_end", DataType::Int),
            ("metas", DataType::Metas),
        ]
    );
    assert_eq!(
        columns("budget_events"),
        vec![
            ("name", DataType::Str),
            ("date", DataType::Date),
            ("time", DataType::Str),
            ("timestamp", DataType::Int),
            ("type", DataType::Str),
            ("amount", DataType::Amount),
        ]
    );
    let wildcard = |name: &str| {
        let query = Query::compile(&format!("SELECT * FROM #{}", name)).unwrap();
        query.columns().iter().map(|column| column.name.clone()).collect::<Vec<_>>()
    };
    assert_eq!(wildcard("budgets"), ["name", "date", "assigned", "activity", "available"]);
    assert_eq!(wildcard("errors"), ["file", "date", "kind", "account", "message"]);
}

/// The examples of the query language reference run, and what they promise holds.
#[test]
fn the_documented_examples_run() {
    assert_eq!(
        query("SELECT category, name, available FROM #budgets WHERE date = 2024-04-01 ORDER BY category, name"),
        rows(&[&["NULL", "fun", "20 CNY"], &["Living", "food", "-110 CNY"]])
    );
    assert_eq!(
        query("SELECT name, sum(added) AS budgeted, sum(activity) AS spent FROM #budgets WHERE year = 2024 GROUP BY name ORDER BY spent DESC"),
        rows(&[&["food", "850 CNY", "860 CNY"], &["fun", "50 CNY", "30 CNY"]])
    );
    assert_eq!(
        query("SELECT date, name, available FROM #budgets WHERE number(available) < 0 AND NOT closed"),
        rows(&[
            &["2024-03-01", "food", "-110 CNY"],
            &["2024-04-01", "food", "-110 CNY"],
            &["2024-05-01", "food", "-110 CNY"],
            &["2024-06-01", "food", "-10 CNY"]
        ])
    );
    assert_eq!(
        query("SELECT kind, count(*) AS errors FROM #errors GROUP BY kind ORDER BY errors DESC").len(),
        6
    );
    assert_eq!(
        query("SELECT date, kind, account FROM #errors WHERE file = 'main.zhang'"),
        rows(&[
            &["NULL", "MultipleOperatingCurrencyDetect", "NULL"],
            &["2024-01-01", "DefineDuplicatedBudget", "NULL"]
        ])
    );
    // the `txn_id` of an error in a transaction is the transaction's `id` in the postings table
    let txn_id = query("SELECT meta('txn_id') FROM #errors WHERE kind = 'UnbalancedTransaction'")
        .remove(0)
        .remove(0);
    assert_eq!(
        query(&format!("SELECT narration, account, position WHERE id = '{}'", txn_id)),
        rows(&[&["unbalanced", "Expenses:Food", "10 CNY"], &["unbalanced", "Assets:Bank", "-9 CNY"]])
    );
}

/// The ledger of #499: a budget in CNY with an amount assigned and spent in USD, without a price,
/// and closed on 2024-03-01, with spending after the close.
const BUDGET_499: &str = r#"
option "operating_currency" "CNY"
1970-01-01 commodity CNY
1970-01-01 commodity USD
2024-01-01 open Assets:Cash
2024-01-01 open Expenses:Food
  budget: "Food"

2024-01-01 budget Food CNY
2024-01-01 budget-add Food 1000 CNY
2024-01-02 budget-add Food 50 USD

2024-01-10 * "Lunch in CNY"
  Expenses:Food 100 CNY
  Assets:Cash

2024-01-11 * "Lunch abroad in USD"
  Expenses:Food 20 USD
  Assets:Cash

2024-03-01 budget-close Food

2024-04-05 * "Lunch after the budget is closed"
  Expenses:Food 30 CNY
  Assets:Cash
"#;

/// #499: an amount in another commodity than the budget's is never added as a number: without a
/// price it is left out and reported. The budget is open before its close, and takes no
/// activity after it: the lunch of April is left out and reported.
#[test]
fn a_budget_leaves_out_unconverted_amounts_and_spending_after_its_close() {
    let ledger = common::load_text(BUDGET_499);
    assert_eq!(
        run(&ledger, "SELECT date, assigned, added, activity, available, closed FROM #budgets"),
        rows(&[
            &["2024-01-01", "1000 CNY", "1000 CNY", "100 CNY", "900 CNY", "FALSE"],
            &["2024-02-01", "900 CNY", "0 CNY", "0 CNY", "900 CNY", "FALSE"],
            &["2024-03-01", "900 CNY", "0 CNY", "0 CNY", "900 CNY", "TRUE"],
            &["2024-04-01", "900 CNY", "0 CNY", "0 CNY", "900 CNY", "TRUE"],
        ])
    );
    assert_eq!(
        run(
            &ledger,
            "SELECT date, kind, account, meta('budget_name'), meta('commodity'), meta('budget_commodity') FROM #errors"
        ),
        rows(&[
            &["2024-01-02", "BudgetCommodityMismatch", "NULL", "Food", "USD", "CNY"],
            &["2024-01-11", "BudgetCommodityMismatch", "Expenses:Food", "Food", "USD", "CNY"],
            &["2024-04-05", "BudgetClosed", "Expenses:Food", "Food", "NULL", "NULL"],
        ])
    );
    assert_eq!(
        run(&ledger, "SELECT message FROM #errors"),
        rows(&[
            &["No price converts USD to CNY, the commodity of budget Food: the amount does not count toward it"],
            &["No price converts USD to CNY, the commodity of budget Food: the amount does not count toward it"],
            &["Budget Food is closed: postings to Expenses:Food after its close do not count toward it"],
        ])
    );
    assert_eq!(
        run(&ledger, "SELECT name, close, close_time FROM #budget_definitions"),
        rows(&[&["Food", "2024-03-01", "NULL"]])
    );

    // with a price, the amounts in USD are converted at their date and nothing is reported but
    // the spending after the close
    let ledger = common::load_text(&format!("{}\n2024-01-01 price USD 7 CNY\n", BUDGET_499));
    assert_eq!(
        run(
            &ledger,
            "SELECT date, assigned, added, activity, available, closed FROM #budgets WHERE date <= 2024-01-01"
        ),
        rows(&[&["2024-01-01", "1350 CNY", "1350 CNY", "240 CNY", "1110 CNY", "FALSE"]])
    );
    assert_eq!(run(&ledger, "SELECT kind, date FROM #errors"), rows(&[&["BudgetClosed", "2024-04-05"]]));
}

/// One rule of budget membership: a posting belongs to the budgets of its account's `open` in
/// effect at its date and time, from the budget's definition until its close, when a price
/// converts it. The activity, the postings' `budgets` column (the budget page's posting list) and
/// the errors all follow it, so the postings of a budget's month add up to its activity.
const ONE_MEMBERSHIP_RULE: &str = r#"
1970-01-01 commodity CNY
1970-01-01 commodity USD
1970-01-01 open Assets:Bank
1970-01-01 open Expenses:Food
  budget: food
1970-01-01 open Expenses:Lunch
  budget: A
2024-01-01 budget A CNY
2024-01-01 budget B CNY
2024-01-10 * "Market" "before the budget"
  Expenses:Food 10 CNY
  Assets:Bank
2024-01-15 budget food CNY
2024-01-15 budget-add food 100 CNY
2024-01-20 * "Market" "after the budget"
  Expenses:Food 7 CNY
  Assets:Bank
2024-01-21 * "Abroad" "no price converts it"
  Expenses:Food 3 USD
  Assets:Bank
2024-01-05 09:00:00 * "Cafe" "lunch before the reopen"
  Expenses:Lunch 10 CNY
  Assets:Bank
2024-01-05 09:30:00 close Expenses:Lunch
2024-01-05 10:00:00 open Expenses:Lunch
  budget: B
2024-01-05 11:00:00 * "Cafe" "lunch after the reopen"
  Expenses:Lunch 20 CNY
  Assets:Bank
"#;

#[test]
fn a_posting_counts_toward_a_budget_by_one_rule() {
    let ledger = common::load_text(ONE_MEMBERSHIP_RULE);
    // the lunch at 09:00 counts in A, the budget of the open in effect then, not in B, which
    // the account names from 10:00 on; the market before the budget's definition and the
    // dollars no price converts are not food's
    assert_eq!(
        run(&ledger, "SELECT name, activity FROM #budgets WHERE date = 2024-01-01"),
        rows(&[&["A", "10 CNY"], &["B", "20 CNY"], &["food", "7 CNY"]])
    );
    // the posting list of each budget is the postings of its activity, and adds up to it
    for (name, narrations, activity) in [
        ("A", vec!["lunch before the reopen"], "10"),
        ("B", vec!["lunch after the reopen"], "20"),
        ("food", vec!["after the budget"], "7"),
    ] {
        let listed = run(
            &ledger,
            &format!(
                "SELECT narration WHERE yearmonth(date) = 2024-01-01 AND '{}' IN budgets ORDER BY date, time",
                name
            ),
        );
        assert_eq!(listed, narrations.iter().map(|it| vec![(*it).to_owned()]).collect::<Vec<_>>(), "{}", name);
        let sum = run(
            &ledger,
            &format!("SELECT sum(number) WHERE yearmonth(date) = 2024-01-01 AND '{}' IN budgets", name),
        );
        assert_eq!(sum, rows(&[&[activity]]), "{}", name);
    }
    assert_eq!(
        run(&ledger, "SELECT narration, budgets WHERE account ~ 'Expenses' ORDER BY date, time"),
        rows(&[
            &["lunch before the reopen", "A"],
            &["lunch after the reopen", "B"],
            &["before the budget", ""],
            &["after the budget", "food"],
            &["no price converts it", ""],
        ])
    );
    // account_budgets() reads the same rule, at the start of the day
    assert_eq!(
        run(
            &ledger,
            "SELECT DISTINCT account_budgets('Expenses:Lunch', 2024-01-05) FROM #budget_definitions"
        ),
        rows(&[&["A"]])
    );
    // and the store fold reports what the rule leaves out, and nothing for the lunches
    assert_eq!(
        run(&ledger, "SELECT kind, date, account FROM #errors WHERE kind ~ '^Budget'"),
        rows(&[
            &["BudgetDoesNotExist", "2024-01-10", "Expenses:Food"],
            &["BudgetCommodityMismatch", "2024-01-21", "Expenses:Food"],
        ])
    );
}

/// A budget-close without a time leaves the budget open through its whole day; one with a time
/// closes it at that time.
#[test]
fn a_budget_takes_activity_until_its_close() {
    let ledger = common::load_text(
        r#"
1970-01-01 commodity CNY
1970-01-01 open Assets:Bank
1970-01-01 open Expenses:Food
  budget: food
1970-01-01 open Expenses:Fun
  budget: fun
2024-01-01 budget food CNY
2024-01-01 budget fun CNY
2024-03-01 budget-close food
2024-03-01 12:00:00 budget-close fun
2024-03-01 * "Market" "on the close day"
  Expenses:Food 10 CNY
  Assets:Bank
2024-03-02 * "Market" "the day after"
  Expenses:Food 20 CNY
  Assets:Bank
2024-03-01 11:59:59 * "Cinema" "before the close"
  Expenses:Fun 1 CNY
  Assets:Bank
2024-03-01 12:00:00 * "Cinema" "at the close"
  Expenses:Fun 2 CNY
  Assets:Bank
2024-03-01 12:00:01 * "Cinema" "after the close"
  Expenses:Fun 4 CNY
  Assets:Bank
"#,
    );
    assert_eq!(
        run(&ledger, "SELECT name, activity FROM #budgets WHERE date = 2024-03-01"),
        rows(&[&["food", "10 CNY"], &["fun", "3 CNY"]])
    );
    assert_eq!(
        run(&ledger, "SELECT name, close, close_time FROM #budget_definitions"),
        rows(&[&["food", "2024-03-01", "NULL"], &["fun", "2024-03-01", "12:00:00"]])
    );
    assert_eq!(
        run(&ledger, "SELECT kind, source FROM #errors"),
        rows(&[
            &[
                "BudgetClosed",
                "2024-03-02 * \"Market\" \"the day after\"\n  Expenses:Food 20 CNY\n  Assets:Bank"
            ],
            &[
                "BudgetClosed",
                "2024-03-01 12:00:01 * \"Cinema\" \"after the close\"\n  Expenses:Fun 4 CNY\n  Assets:Bank"
            ],
        ])
    );
}

/// A budget's months run through its own last entry or the ledger's last month with a
/// transaction, whichever is later; other directives do not extend them.
#[test]
fn budget_months_follow_the_budget_and_the_transactions() {
    let ledger = common::load_text(
        r#"
1970-01-01 commodity CNY
1970-01-01 open Assets:Bank
1970-01-01 open Expenses:Food
  budget: food
2024-01-01 budget food CNY
2024-01-01 budget plan CNY
2024-02-10 * "Market"
  Expenses:Food 10 CNY
  Assets:Bank
2024-05-01 budget-add plan 300 CNY
2024-09-01 price USD 7.1 CNY
9999-12-31 event "location" "a typo"
2030-01-01 note Assets:Bank "far ahead"
"#,
    );
    // food ends with the last transaction; plan with its budget-add planned for May
    assert_eq!(
        run(&ledger, "SELECT name, min(date), max(date), count(*) FROM #budgets GROUP BY name"),
        rows(&[&["food", "2024-01-01", "2024-02-01", "2"], &["plan", "2024-01-01", "2024-05-01", "5"]])
    );
}

/// The ledger of the review of #434: a hundred budgets from the year 1000 and an event in 9999.
/// The event does not extend the budgets, so the table stays small. The current month does: in
/// the year 2000, a hundred series of 12001 months are too many, and the error points at the
/// budget directive, whose date is the typo.
#[test]
fn a_far_future_event_does_not_generate_months() {
    let mut text = String::from("9999-12-31 event \"location\" \"a typo\"\n");
    for index in 0..100 {
        text.push_str(&format!("1000-01-01 budget b{} CNY\n", index));
    }
    let ledger = common::load_text(&text);
    let in_the_year_1000 = NaiveDate::from_ymd_opt(1000, 1, 15).unwrap();
    assert_eq!(
        run_at(&ledger, "SELECT count(*), min(date), max(date) FROM #budgets", in_the_year_1000),
        rows(&[&["100", "1000-01-01", "1000-01-01"]])
    );
    assert!(run_at(&ledger, "SELECT name FROM #budgets WHERE year = 2024", in_the_year_1000).is_empty());

    let err = Query::compile("SELECT count(*) FROM #budgets")
        .unwrap()
        .execute_at(&ledger, &Params::new(), today())
        .unwrap_err();
    assert_eq!(err.kind, QueryErrorKind::TooLarge, "{}", err.message);
    assert_eq!(
        err.message,
        "the #budgets table would generate too many rows (1200100 months in all, more than the result size limit): \
         budget 'b0' runs from 1000-01 until 2000-01, the current month; check the date of its budget directive (main.zhang)"
    );
}

/// A date typo on a transaction can still ask for millions of months: the generated rows are
/// charged to the result budget and the deadline is checked while they are built, so the query
/// stops instead of holding them all.
#[test]
fn generated_budget_months_are_bounded_by_the_result_budget_and_the_deadline() {
    let mut text = String::from("1970-01-01 open Assets:Bank\n1970-01-01 open Expenses:Food\n1970-01-01 commodity CNY\n");
    text.push_str("9999-12-31 * \"a typo\"\n  Expenses:Food 1 CNY\n  Assets:Bank\n");
    for index in 0..100 {
        text.push_str(&format!("1000-01-01 budget b{} CNY\n", index));
    }
    let ledger = common::load_text(&text);
    let compiled = Query::compile("SELECT count(*) FROM #budgets WHERE year = 2024").unwrap();
    // 100 budgets of 9000 years of months would be 10.8 million rows
    let err = compiled.execute_at(&ledger, &Params::new(), today()).unwrap_err();
    assert_eq!(err.kind, QueryErrorKind::TooLarge, "{}", err.message);
    // the error names the cause instead of suggesting a filter, which cannot help
    assert_eq!(
        err.message,
        "the #budgets table would generate too many rows (10800000 months in all, more than the result size limit): \
         budget 'b0' runs from 1000-01 until 9999-12 because of a transaction dated 9999-12-31 (main.zhang); check that date"
    );
    let options = |timeout, max_result_values| ExecuteOptions {
        today: Some(today()),
        timeout,
        max_result_values,
        count_total: false,
    };
    let err = compiled
        .execute_with_options(&ledger, &Params::new(), &options(None, Some(10_000)))
        .unwrap_err();
    assert_eq!(err.kind, QueryErrorKind::TooLarge, "{}", err.message);
    let err = compiled
        .execute_with_options(&ledger, &Params::new(), &options(Some(std::time::Duration::ZERO), None))
        .unwrap_err();
    assert_eq!(err.kind, QueryErrorKind::Timeout, "{}", err.message);
    // one budget fits: 9000 years of months, of which 12 are in 2024
    let mut text = String::from("1970-01-01 open Assets:Bank\n1970-01-01 open Expenses:Food\n1970-01-01 commodity CNY\n");
    text.push_str("9999-12-31 * \"a typo\"\n  Expenses:Food 1 CNY\n  Assets:Bank\n1000-01-01 budget b CNY\n");
    let ledger = common::load_text(&text);
    assert_eq!(run(&ledger, "SELECT count(*) FROM #budgets WHERE year = 2024"), rows(&[&["12"]]));
}

/// A typo such as 2204 for 2024 on one transaction or budget directive makes the months of
/// `#budgets` too many for the result budget; the error says which budget, which directive and
/// which file, so the user can fix the date.
#[test]
fn too_many_budget_months_name_the_directive_that_sets_the_end() {
    let options = ExecuteOptions {
        today: Some(today()),
        timeout: None,
        max_result_values: Some(1_000),
        count_total: false,
    };
    // a filter that keeps the months of a year still generates every month before it is applied
    let message = |text: &str| {
        let ledger = common::load_text(text);
        let err = Query::compile("SELECT name, available FROM #budgets WHERE year = 2024")
            .unwrap()
            .execute_with_options(&ledger, &Params::new(), &options)
            .unwrap_err();
        assert_eq!(err.kind, QueryErrorKind::TooLarge, "{}", err.message);
        // a filter that keeps only the months up to a date generates none after it, whatever
        // the typo: the budget pages ask for one month this way
        for bounded in [
            "SELECT name, available FROM #budgets WHERE date = 2024-06-01",
            "SELECT name, last(available) FROM #budgets WHERE date <= 2024-06-01 GROUP BY name",
            "SELECT name, available FROM #budgets WHERE name = 'food' AND yearmonth(date) = 2024-06-01",
        ] {
            let result = Query::compile(bounded).unwrap().execute_with_options(&ledger, &Params::new(), &options);
            assert!(result.is_ok(), "{bounded}: {:?}", result.err());
        }
        err.message
    };
    let header = "1970-01-01 commodity CNY\n1970-01-01 open Assets:Bank\n1970-01-01 open Expenses:Food\n  budget: food\n\
                  2024-01-01 budget food CNY\n2024-01-01 budget fun CNY\n2024-01-01 budget-add food 100 CNY\n\
                  2024-02-01 * \"Market\"\n  Expenses:Food 10 CNY\n  Assets:Bank\n";

    let typo = format!("{}2204-05-01 * \"Market\"\n  Expenses:Food 10 CNY\n  Assets:Bank\n", header);
    assert_eq!(
        message(&typo),
        "the #budgets table would generate too many rows (4330 months in all, more than the result size limit): \
         budget 'food' runs from 2024-01 until 2204-05 because of a transaction dated 2204-05-01 (main.zhang); check that date"
    );

    let typo = format!("{}2204-05-01 budget-close fun\n", header);
    assert_eq!(
        message(&typo),
        "the #budgets table would generate too many rows (2167 months in all, more than the result size limit): \
         budget 'fun' runs from 2024-01 until 2204-05 because of a budget-close dated 2204-05-01 (main.zhang); check that date"
    );

    let typo = format!("{}2204-05-01 budget-add food 10 CNY\n", header);
    assert!(
        message(&typo).ends_with("budget 'food' runs from 2024-01 until 2204-05 because of a budget-add dated 2204-05-01 (main.zhang); check that date"),
        "{}",
        message(&typo)
    );

    // without the typo, the same query is small
    let ledger = common::load_text(header);
    let result = Query::compile("SELECT name, available FROM #budgets WHERE date = 2024-02-01")
        .unwrap()
        .execute_with_options(&ledger, &Params::new(), &options)
        .unwrap();
    assert_eq!(result.rows.len(), 2);
}
