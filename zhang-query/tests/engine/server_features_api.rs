//! Acceptance tests of the Rust API that the wave 1 query-engine features of issue #479 add
//! (track Y of the spec): `ExecuteOptions::count_total` and `QueryResult::total` (track L),
//! and the valuation API `Position::convert`, `Inventory::convert` and
//! `PriceMap::for_ledger` (track D1).
//!
//! This file does not compile until that API exists; the BQL-level tests are in
//! `server_features.rs`, which compiles on its own. Expected values are derived by hand from
//! `server_features/journal/main.zhang` and the valuation ledger below, and explained in
//! comments.

use std::path::PathBuf;
use std::str::FromStr;
use std::sync::OnceLock;

use bigdecimal::BigDecimal;
use chrono::NaiveDate;
use zhang_core::ledger::Ledger;
use zhang_query::decimal::to_plain_string;
use zhang_query::{ExecuteOptions, Inventory, Params, Position, PriceMap, Query, QueryErrorKind, QueryResult, Value};

fn journal() -> &'static Ledger {
    static CELL: OnceLock<Ledger> = OnceLock::new();
    CELL.get_or_init(|| {
        zhang_testkit::ledger::load_ledger(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/engine/server_features/journal"),
            "main.zhang",
        )
    })
}

fn today() -> NaiveDate {
    NaiveDate::from_ymd_opt(2024, 12, 31).unwrap()
}

fn date(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

fn options(count_total: bool) -> ExecuteOptions {
    ExecuteOptions {
        today: Some(today()),
        count_total,
        ..ExecuteOptions::default()
    }
}

fn run(ledger: &Ledger, sql: &str, params: &Params, options: &ExecuteOptions) -> QueryResult {
    Query::compile_with_params(sql, &params.types())
        .and_then(|query| query.execute_with_options(ledger, params, options))
        .unwrap_or_else(|err| panic!("{}: {:?} {}", sql, err.kind, err.message))
}

/// The rows (as text) and the total of a query run with `count_total`.
fn counted(sql: &str, params: &Params) -> (Vec<Vec<String>>, Option<u64>) {
    let result = run(journal(), sql, params, &options(true));
    let rows = result.rows.iter().map(|row| row.iter().map(Value::to_string).collect()).collect();
    (rows, result.total)
}

fn texts(cells: &[&str]) -> Vec<Vec<String>> {
    cells.iter().map(|it| vec![(*it).to_owned()]).collect()
}

// =======================================================================================
// track L: count_total
//
// The journal ledger has 15 transactions, 33 postings (2 per transaction, 3 for Movie night
// and 4 for Sell AAPL, whose sale is booked as two lots), and postings on 9 accounts:
// Assets:Bank 12, Assets:Bank:Savings 1, Assets:BankCard 1, Assets:Broker 4, Assets:Old 2,
// Expenses:Food 9, Expenses:Fun 2, Income:Gains 1, Income:Salary 1.

#[test]
fn l_total_is_the_row_count_before_limit_and_offset() {
    // 15 transactions, of which a page of 3
    let (rows, total) = counted("SELECT narration FROM #transactions ORDER BY date DESC, narration LIMIT 3", &Params::new());
    assert_eq!(rows, texts(&["ÄPFEL vom Markt", "Only a narration", "Closed and unbalanced"]));
    assert_eq!(total, Some(15));
    // with an offset (the transactions of 2024-01-15 come Bread, Groceries, Morning coffee by
    // narration, then December salary), and with an offset past the end: no rows, the same total
    let (rows, total) = counted(
        "SELECT narration FROM #transactions ORDER BY date DESC, narration LIMIT 3 OFFSET 13",
        &Params::new(),
    );
    assert_eq!(rows, texts(&["Morning coffee", "December salary"]));
    assert_eq!(total, Some(15));
    let (rows, total) = counted(
        "SELECT narration FROM #transactions ORDER BY date DESC, narration LIMIT 3 OFFSET 100",
        &Params::new(),
    );
    assert!(rows.is_empty());
    assert_eq!(total, Some(15));
    // parameters
    let (rows, total) = counted(
        "SELECT narration FROM #transactions LIMIT :limit OFFSET :offset",
        &Params::new().bind("limit", 2i64).bind("offset", 1i64),
    );
    assert_eq!(rows, texts(&["Groceries", "Bread"]));
    assert_eq!(total, Some(15));
    // without LIMIT, the total is the number of rows
    let (rows, total) = counted("SELECT narration FROM #transactions WHERE date >= 2024-03-01", &Params::new());
    assert_eq!(rows.len(), 6);
    assert_eq!(total, Some(6));
    // after WHERE: the 6 transactions from 2024-03-01 (Sell AAPL, Unbalanced, Closed account,
    // Closed and unbalanced, Only a narration, ÄPFEL vom Markt)
    let (rows, total) = counted(
        "SELECT narration FROM #transactions WHERE date >= 2024-03-01 ORDER BY narration LIMIT 1",
        &Params::new(),
    );
    assert_eq!(rows, texts(&["Closed account"]));
    assert_eq!(total, Some(6));
    // an empty result
    let (rows, total) = counted("SELECT narration FROM #transactions WHERE FALSE LIMIT 10", &Params::new());
    assert!(rows.is_empty());
    assert_eq!(total, Some(0));
    // the postings table, ordered
    let (rows, total) = counted("SELECT date, account FROM #postings ORDER BY date DESC, account LIMIT 1", &Params::new());
    assert_eq!(rows, vec![vec!["2024-03-11".to_owned(), "Assets:Bank".to_owned()]]);
    assert_eq!(total, Some(33));
}

#[test]
fn l_total_counts_groups_and_distinct_rows() {
    // GROUP BY: 9 accounts with postings
    let (rows, total) = counted(
        "SELECT account, count(*) FROM #postings GROUP BY account ORDER BY account LIMIT 2",
        &Params::new(),
    );
    assert_eq!(
        rows,
        vec![
            vec!["Assets:Bank".to_owned(), "12".to_owned()],
            vec!["Assets:Bank:Savings".to_owned(), "1".to_owned()]
        ]
    );
    assert_eq!(total, Some(9));
    // HAVING: the 3 accounts with more than 2 postings (Assets:Bank, Assets:Broker, Expenses:Food)
    let (rows, total) = counted(
        "SELECT account FROM #postings GROUP BY account HAVING count(*) > 2 ORDER BY account LIMIT 1 OFFSET 1",
        &Params::new(),
    );
    assert_eq!(rows, texts(&["Assets:Broker"]));
    assert_eq!(total, Some(3));
    // DISTINCT: 11 payees, NULL included (NULL, Bakery, Bank, Broker, Cafe, Card, Cinema,
    // Employer, Market, Shop, Straße)
    let (rows, total) = counted("SELECT DISTINCT payee FROM #transactions ORDER BY payee LIMIT 2", &Params::new());
    assert_eq!(rows, texts(&["NULL", "Bakery"]));
    assert_eq!(total, Some(11));
    // an aggregate without GROUP BY is one row
    let (rows, total) = counted("SELECT count(*) FROM #postings", &Params::new());
    assert_eq!(rows, texts(&["33"]));
    assert_eq!(total, Some(1));
}

#[test]
fn l_empty_aggregate_counts_its_group_before_paging() {
    for (suffix, visible) in [
        ("", true),
        ("LIMIT 1", true),
        ("LIMIT 0", false),
        ("LIMIT 1 OFFSET 1", false),
        ("ORDER BY count(*) LIMIT 1", true),
        ("ORDER BY count(*) LIMIT 1 OFFSET 1", false),
    ] {
        let sql = format!("SELECT count(*) FROM #postings WHERE FALSE {suffix}");
        let (rows, total) = counted(&sql, &Params::new());
        assert_eq!(rows, if visible { texts(&["0"]) } else { vec![] }, "{}", sql);
        assert_eq!(total, Some(1), "{}", sql);
    }
    let (rows, total) = counted("SELECT DISTINCT count(*) WHERE FALSE ORDER BY 1 LIMIT 1", &Params::new());
    assert_eq!(rows, texts(&["0"]));
    assert_eq!(total, Some(1));
    for sql in [
        "SELECT count(*) WHERE FALSE GROUP BY account LIMIT 1",
        "SELECT count(*) WHERE FALSE GROUP BY account HAVING count(*) = 0 LIMIT 1",
    ] {
        let (rows, total) = counted(sql, &Params::new());
        assert!(rows.is_empty(), "{}", sql);
        assert_eq!(total, Some(0), "{}", sql);
    }
}

#[test]
fn l_total_is_only_computed_when_asked() {
    let sql = "SELECT narration FROM #transactions LIMIT 3";
    assert_eq!(run(journal(), sql, &Params::new(), &options(false)).total, None);
    assert!(!ExecuteOptions::default().count_total);
    let query = Query::compile(sql).unwrap();
    assert_eq!(query.execute_at(journal(), &Params::new(), today()).unwrap().total, None);
    // asking for the total does not change the rows
    assert_eq!(
        run(journal(), sql, &Params::new(), &options(true)).rows,
        run(journal(), sql, &Params::new(), &options(false)).rows
    );
}

/// The total is counted without holding the rows beyond what the query needs: a LIMIT without
/// ORDER BY holds one row, so a result budget of 10 values is enough to count the 33 postings
/// of 3 values each. READING: the result budget (`max_result_values`) charges the rows a query
/// holds, so counting must not hold the rows it skips.
#[test]
fn l_total_does_not_materialise_the_skipped_rows() {
    let options = ExecuteOptions {
        today: Some(today()),
        count_total: true,
        max_result_values: Some(10),
        ..ExecuteOptions::default()
    };
    let result = run(journal(), "SELECT date, account, narration FROM #postings LIMIT 1", &Params::new(), &options);
    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.total, Some(33));
    // a page of the whole ledger is still too large
    let err = Query::compile("SELECT date, account, narration FROM #postings")
        .unwrap()
        .execute_with_options(journal(), &Params::new(), &options)
        .unwrap_err();
    assert_eq!(err.kind, QueryErrorKind::TooLarge);
}

// =======================================================================================
// track D1: the valuation API

/// The only prices of CNY are CNY in USD, so USD converts to CNY through the inverse rate:
/// 1 / 0.125 = 8 CNY per USD from 2024-01-01, 1 / 0.1 = 10 CNY per USD from 2024-03-01. AAPL has
/// a USD price only (150 from 2024-02-01), so AAPL held at a USD cost converts to CNY through
/// its cost currency: AAPL -> USD -> CNY.
const VALUATION: &str = r#"
1970-01-01 commodity CNY
1970-01-01 commodity USD
1970-01-01 commodity AAPL
1970-01-01 open Assets:Cash
1970-01-01 open Assets:Broker
1970-01-01 open Equity:Opening

2024-01-01 price CNY 0.125 USD
2024-03-01 price CNY 0.1 USD
2024-02-01 price AAPL 150 USD

2024-01-10 * "Opening"
  Assets:Cash          10 USD
  Equity:Opening      -10 USD

2024-01-10 * "Buy"
  Assets:Broker         2 AAPL {100 USD}
  Equity:Opening     -200 USD
"#;

fn valuation() -> &'static Ledger {
    static CELL: OnceLock<Ledger> = OnceLock::new();
    CELL.get_or_init(|| zhang_testkit::ledger::load_text(VALUATION))
}

/// An amount-like text with its number normalized (`80.00 CNY` is `80 CNY`); an inventory's
/// positions one by one.
fn normalize(text: &str) -> String {
    text.split(", ")
        .map(|part| match part.split_once(' ') {
            Some((number, rest)) if !rest.contains(' ') => match BigDecimal::from_str(number) {
                Ok(number) => format!("{} {}", to_plain_string(&number.normalized()), rest),
                Err(_) => part.to_owned(),
            },
            _ => part.to_owned(),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn single(ledger: &Ledger, sql: &str) -> Value {
    let mut result = run(ledger, sql, &Params::new(), &options(false));
    assert_eq!(result.rows.len(), 1, "{}", sql);
    result.rows.remove(0).remove(0)
}

fn position(ledger: &Ledger, account: &str) -> Position {
    match single(ledger, &format!("SELECT position FROM #postings WHERE account = '{}'", account)) {
        Value::Position(position) => position,
        other => panic!("not a position: {:?}", other),
    }
}

fn holdings(ledger: &Ledger) -> Inventory {
    match single(ledger, "SELECT sum(position) FROM #postings WHERE account ~ '^Assets'") {
        Value::Inventory(inventory) => inventory,
        other => panic!("not an inventory: {:?}", other),
    }
}

/// BQL's `convert()` of the same value, `expression` over the postings `filter` selects: the
/// API and the function have one implementation.
fn bql_convert(ledger: &Ledger, expression: &str, filter: &str, day: Option<NaiveDate>) -> String {
    let day = day.map(|it| format!(", {}", it)).unwrap_or_default();
    let sql = format!("SELECT convert({}, 'CNY'{}) FROM #postings WHERE {}", expression, day, filter);
    normalize(&single(ledger, &sql).to_string())
}

#[test]
fn d1_position_convert_uses_inverse_rates_cost_currencies_and_the_date() {
    let prices = PriceMap::for_ledger(valuation());
    let cash = position(valuation(), "Assets:Cash");
    let stock = position(valuation(), "Assets:Broker");
    let convert = |position: &Position, day: Option<NaiveDate>| normalize(&Value::from(position.convert("CNY", &prices, day)).to_string());
    // 10 USD at 8 CNY per USD in February, 10 in March and later (the latest price)
    assert_eq!(convert(&cash, Some(date(2024, 2, 15))), "80 CNY");
    assert_eq!(convert(&cash, Some(date(2024, 3, 15))), "100 CNY");
    assert_eq!(convert(&cash, None), "100 CNY");
    // the price of the conversion date itself counts
    assert_eq!(convert(&cash, Some(date(2024, 3, 1))), "100 CNY");
    // before any price: unchanged
    assert_eq!(convert(&cash, Some(date(2023, 12, 31))), "10 USD");
    // 2 AAPL: 2 × 150 USD × 8 = 2400 CNY in February, 2 × 150 × 10 = 3000 in March
    assert_eq!(convert(&stock, Some(date(2024, 2, 15))), "2400 CNY");
    assert_eq!(convert(&stock, Some(date(2024, 3, 15))), "3000 CNY");
    // the same as convert() in BQL, also where there is no price yet (2024-01-15 for AAPL)
    for day in [Some(date(2024, 1, 15)), Some(date(2024, 2, 15)), Some(date(2024, 3, 15)), None] {
        assert_eq!(
            convert(&stock, day),
            bql_convert(valuation(), "position", "account = 'Assets:Broker'", day),
            "{:?}",
            day
        );
    }
}

#[test]
fn d1_inventory_convert_converts_every_position() {
    let prices = PriceMap::for_ledger(valuation());
    let inventory = holdings(valuation());
    let convert = |day: Option<NaiveDate>| normalize(&Value::Inventory(inventory.convert("CNY", &prices, day)).to_string());
    // 80 + 2400 in February, 100 + 3000 from March
    assert_eq!(convert(Some(date(2024, 2, 15))), "2480 CNY");
    assert_eq!(convert(Some(date(2024, 3, 15))), "3100 CNY");
    assert_eq!(convert(None), "3100 CNY");
    for day in [Some(date(2024, 1, 15)), Some(date(2024, 2, 15)), None] {
        assert_eq!(convert(day), bql_convert(valuation(), "sum(position)", "account ~ '^Assets'", day), "{:?}", day);
    }
    // an empty inventory converts to an empty inventory
    assert_eq!(Inventory::new().convert("CNY", &prices, None).to_string(), "");
}

#[test]
fn d1_price_map_for_ledger_has_the_ledgers_prices() {
    let prices = PriceMap::for_ledger(valuation());
    let rate = |base: &str, quote: &str, day: Option<NaiveDate>| prices.rate(base, quote, day).map(|it| to_plain_string(&it.normalized()));
    assert_eq!(rate("CNY", "USD", None), Some("0.1".to_owned()));
    assert_eq!(rate("USD", "CNY", Some(date(2024, 2, 15))), Some("8".to_owned()));
    assert_eq!(rate("AAPL", "USD", Some(date(2024, 2, 15))), Some("150".to_owned()));
    assert_eq!(rate("AAPL", "USD", Some(date(2024, 1, 15))), None);
    assert_eq!(rate("AAPL", "CNY", None), None);
    // a ledger without prices
    assert!(PriceMap::for_ledger(&zhang_testkit::ledger::load_text("1970-01-01 commodity CNY\n1970-01-01 open Assets:Cash\n")).is_empty());
}

#[test]
fn the_cached_price_map_is_the_one_queries_use_and_is_shared() {
    // a ledger of its own, so no other test has filled its cache yet
    let ledger = zhang_testkit::ledger::load_text(VALUATION);
    let cached = PriceMap::cached(&ledger);
    // built once: a query and a later call use the same map
    single(&ledger, "SELECT convert(sum(position), 'CNY')");
    assert!(std::sync::Arc::ptr_eq(&cached, &PriceMap::cached(&ledger)));
    let fresh = PriceMap::for_ledger(&ledger);
    for (base, quote, day) in [
        ("CNY", "USD", None),
        ("USD", "CNY", Some(date(2024, 2, 15))),
        ("AAPL", "USD", Some(date(2024, 2, 15))),
    ] {
        assert_eq!(cached.rate(base, quote, day), fresh.rate(base, quote, day), "{} {} {:?}", base, quote, day);
    }
    // it outlives the ledger it came from
    drop(ledger);
    assert_eq!(
        cached.rate("CNY", "USD", None).map(|it| to_plain_string(&it.normalized())),
        Some("0.1".to_owned())
    );
}
