//! Lot booking of postings held at cost, checked against beanquery.
//!
//! Every expected value below was produced by beancount 3.2.3 / beanquery 0.2.0 on the same
//! ledger. zhang declares the booking method with `booking_method` account metadata, which
//! beancount ignores, so the oracle copies of these ledgers wrote it on the open line
//! instead (`1970-01-01 open Assets:Fifo "FIFO"`); they are otherwise identical.

mod common;

use chrono::NaiveDate;
use zhang_query::{Params, Query, Value};

const HEADER: &str = r#"
1970-01-01 commodity USD
1970-01-01 commodity AAPL

1970-01-01 open Assets:Bank
1970-01-01 open Income:Gains
"#;

/// Rows of `sql` over `ledger` (appended to the shared header), as strings.
fn query(ledger: &str, sql: &str) -> Vec<Vec<String>> {
    let ledger = common::load_text(&format!("{HEADER}{ledger}"));
    let today = NaiveDate::from_ymd_opt(2024, 12, 31).unwrap();
    let result = Query::compile(sql)
        .and_then(|query| query.execute_at(&ledger, &Params::new(), today))
        .unwrap_or_else(|err| panic!("{sql}: {err}"));
    result.rows.iter().map(|row| row.iter().map(Value::to_string).collect()).collect()
}

/// The postings of `account`: number, cost number, cost date, cost label and price.
fn postings(ledger: &str, account: &str) -> Vec<Vec<String>> {
    query(
        ledger,
        &format!("SELECT number, cost_number, cost_date, cost_label, price WHERE account = '{account}'"),
    )
}

/// The open lots of `account`.
fn holdings(ledger: &str, account: &str) -> String {
    let rows = query(ledger, &format!("SELECT sum(position) WHERE account = '{account}'"));
    assert_eq!(rows.len(), 1, "{rows:?}");
    rows[0][0].clone()
}

fn row(cells: &[&str]) -> Vec<String> {
    cells.iter().map(|it| (*it).to_owned()).collect()
}

/// Issue #434: a reduction giving the cost but no date reduces the lot bought at that cost
/// instead of opening a short lot dated by the sale.
#[test]
fn reduction_with_cost_and_no_date_reduces_the_open_lot() {
    let ledger = r#"
1970-01-01 open Assets:Broker

2024-01-01 * "Broker" "buy"
  Assets:Broker     10 AAPL {100 USD}
  Assets:Bank    -1000 USD

2024-06-01 * "Broker" "sell explicit cost, no date"
  Assets:Broker    -4 AAPL {100 USD} @ 150 USD
  Assets:Bank       600 USD
  Income:Gains     -200 USD
"#;
    // beanquery: 6 AAPL {100 USD}, the sale's cost_date is 2024-01-01
    assert_eq!(holdings(ledger, "Assets:Broker"), "6 AAPL {100 USD, 2024-01-01}");
    assert_eq!(
        postings(ledger, "Assets:Broker"),
        vec![
            row(&["10", "100", "2024-01-01", "NULL", "NULL"]),
            row(&["-4", "100", "2024-01-01", "NULL", "150 USD"]),
        ]
    );
}

const TWO_SAME_COST_LOTS: &str = r#"
1970-01-01 open Assets:Fifo
  booking_method: "FIFO"
1970-01-01 open Assets:Lifo
  booking_method: "LIFO"

2024-01-01 * "Broker" "buy"
  Assets:Fifo     5 AAPL {100 USD}
  Assets:Lifo     5 AAPL {100 USD}
  Assets:Bank -1000 USD

2024-02-01 * "Broker" "buy again"
  Assets:Fifo     5 AAPL {100 USD}
  Assets:Lifo     5 AAPL {100 USD}
  Assets:Bank -1000 USD

2024-06-01 * "Broker" "sell"
  Assets:Fifo    -7 AAPL {100 USD} @ 150 USD
  Assets:Lifo    -7 AAPL {100 USD} @ 150 USD
  Assets:Bank   2100 USD
  Income:Gains  -700 USD
"#;

#[test]
fn reduction_with_cost_spans_same_cost_lots_fifo() {
    // beanquery: -5 {2024-01-01}, -2 {2024-02-01}; 3 AAPL left in the 2024-02-01 lot
    assert_eq!(
        postings(TWO_SAME_COST_LOTS, "Assets:Fifo"),
        vec![
            row(&["5", "100", "2024-01-01", "NULL", "NULL"]),
            row(&["5", "100", "2024-02-01", "NULL", "NULL"]),
            row(&["-5", "100", "2024-01-01", "NULL", "150 USD"]),
            row(&["-2", "100", "2024-02-01", "NULL", "150 USD"]),
        ]
    );
    assert_eq!(holdings(TWO_SAME_COST_LOTS, "Assets:Fifo"), "3 AAPL {100 USD, 2024-02-01}");
}

#[test]
fn reduction_with_cost_spans_same_cost_lots_lifo() {
    // beanquery: -5 {2024-02-01}, -2 {2024-01-01}; 3 AAPL left in the 2024-01-01 lot
    assert_eq!(
        postings(TWO_SAME_COST_LOTS, "Assets:Lifo"),
        vec![
            row(&["5", "100", "2024-01-01", "NULL", "NULL"]),
            row(&["5", "100", "2024-02-01", "NULL", "NULL"]),
            row(&["-5", "100", "2024-02-01", "NULL", "150 USD"]),
            row(&["-2", "100", "2024-01-01", "NULL", "150 USD"]),
        ]
    );
    assert_eq!(holdings(TWO_SAME_COST_LOTS, "Assets:Lifo"), "3 AAPL {100 USD, 2024-01-01}");
}

const TRANSFERRED_IN_OLDER_LOT: &str = r#"
1970-01-01 open Assets:Fifo
  booking_method: "FIFO"
1970-01-01 open Assets:Lifo
  booking_method: "LIFO"

2024-03-01 * "Broker" "buy"
  Assets:Fifo     5 AAPL {100 USD}
  Assets:Lifo     5 AAPL {100 USD}
  Assets:Bank -1000 USD

2024-04-01 * "Broker" "transfer in a lot acquired earlier"
  Assets:Fifo     5 AAPL {90 USD, 2023-01-01}
  Assets:Lifo     5 AAPL {90 USD, 2023-01-01}
  Assets:Bank  -900 USD

2024-06-01 * "Broker" "sell"
  Assets:Fifo    -7 AAPL {} @ 150 USD
  Assets:Lifo    -7 AAPL {} @ 150 USD
  Assets:Bank   2100 USD
  Income:Gains
"#;

/// FIFO and LIFO order the lots by acquisition date, not by the order they were opened in.
#[test]
fn reduction_takes_lots_by_acquisition_date() {
    // beanquery: FIFO -5 {90 USD, 2023-01-01}, -2 {100 USD, 2024-03-01}; LIFO the reverse
    assert_eq!(
        postings(TRANSFERRED_IN_OLDER_LOT, "Assets:Fifo")[2..],
        [
            row(&["-5", "90", "2023-01-01", "NULL", "150 USD"]),
            row(&["-2", "100", "2024-03-01", "NULL", "150 USD"]),
        ]
    );
    assert_eq!(holdings(TRANSFERRED_IN_OLDER_LOT, "Assets:Fifo"), "3 AAPL {100 USD, 2024-03-01}");
    assert_eq!(
        postings(TRANSFERRED_IN_OLDER_LOT, "Assets:Lifo")[2..],
        [
            row(&["-5", "100", "2024-03-01", "NULL", "150 USD"]),
            row(&["-2", "90", "2023-01-01", "NULL", "150 USD"]),
        ]
    );
    assert_eq!(holdings(TRANSFERRED_IN_OLDER_LOT, "Assets:Lifo"), "3 AAPL {90 USD, 2023-01-01}");
    // beanquery: -770 USD, the 2100 USD of proceeds less the booked costs, 650 and 680 USD
    assert_eq!(
        query(TRANSFERRED_IN_OLDER_LOT, "SELECT number WHERE account = 'Income:Gains'"),
        vec![row(&["-770"])]
    );
}

#[test]
fn reduction_with_date_matches_only_that_lot() {
    let ledger = r#"
1970-01-01 open Assets:Broker
  booking_method: "FIFO"

2024-01-01 * "Broker" "buy"
  Assets:Broker     5 AAPL {100 USD}
  Assets:Bank    -500 USD

2024-02-01 * "Broker" "buy again"
  Assets:Broker     5 AAPL {100 USD}
  Assets:Bank    -500 USD

2024-06-01 * "Broker" "sell the second lot"
  Assets:Broker    -3 AAPL {100 USD, 2024-02-01} @ 150 USD
  Assets:Bank     450 USD
  Income:Gains   -150 USD
"#;
    // beanquery: the sale reduces the 2024-02-01 lot although FIFO would pick 2024-01-01
    assert_eq!(postings(ledger, "Assets:Broker")[2], row(&["-3", "100", "2024-02-01", "NULL", "150 USD"]));
    assert_eq!(holdings(ledger, "Assets:Broker"), "5 AAPL {100 USD, 2024-01-01}, 2 AAPL {100 USD, 2024-02-01}");
}

#[test]
fn reduction_with_label_matches_only_that_lot() {
    let ledger = r#"
1970-01-01 open Assets:Broker
  booking_method: "FIFO"

2024-01-01 * "Broker" "buy a"
  Assets:Broker     5 AAPL {100 USD, "a"}
  Assets:Bank    -500 USD

2024-02-01 * "Broker" "buy b"
  Assets:Broker     5 AAPL {100 USD, "b"}
  Assets:Bank    -500 USD

2024-06-01 * "Broker" "sell from b"
  Assets:Broker    -2 AAPL {100 USD, "b"} @ 150 USD
  Assets:Bank     300 USD
  Income:Gains   -100 USD
"#;
    // beanquery: the sale reduces lot "b" (dated 2024-02-01); 5 left in "a", 3 in "b"
    assert_eq!(postings(ledger, "Assets:Broker")[2], row(&["-2", "100", "2024-02-01", "b", "150 USD"]));
    assert_eq!(
        query(
            ledger,
            "SELECT cost_date, cost_label, sum(number) WHERE account = 'Assets:Broker' GROUP BY cost_date, cost_label ORDER BY cost_date"
        ),
        vec![row(&["2024-01-01", "a", "5"]), row(&["2024-02-01", "b", "3"])]
    );
}

/// Issue #498: a sale naming only a label reduces the lot of that label in the store as in
/// queries, so the gain is booked against that lot and the rows of the sale sum to zero.
#[test]
fn reduction_by_label_alone_reduces_that_lot_in_the_store_and_in_queries() {
    let ledger = r#"
1970-01-01 open Assets:Broker

2024-01-10 * "Broker" "buy a"
  Assets:Broker    10 AAPL {100 USD, "a"}
  Assets:Bank   -1000 USD

2024-01-20 * "Broker" "buy b"
  Assets:Broker    10 AAPL {110 USD, "b"}
  Assets:Bank   -1100 USD

2024-02-01 * "Broker" "sell one of b"
  Assets:Broker    -1 AAPL {, "b"}
  Assets:Bank     120 USD
  Income:Gains
"#;
    let loaded = common::load_text(&format!("{HEADER}{ledger}"));
    let store = loaded.store.read().unwrap();
    assert_eq!(store.errors.len(), 0);
    let lots = store.commodity_lots["Assets:Broker"]
        .iter()
        .map(|lot| {
            format!(
                "{} {} {{{}, {:?}}}",
                lot.amount,
                lot.commodity,
                lot.cost.as_ref().unwrap(),
                lot.label.as_deref().unwrap()
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(lots, vec!["10 AAPL {100 USD, \"a\"}", "9 AAPL {110 USD, \"b\"}"]);
    let gains = store.postings.iter().find(|it| it.account.name() == "Income:Gains").unwrap();
    assert_eq!(gains.inferred_amount.to_string(), "-10 USD");
    drop(store);

    // beanquery: the sale's row carries lot b, and its weights sum to zero
    assert_eq!(postings(ledger, "Assets:Broker")[2], row(&["-1", "110", "2024-01-20", "b", "NULL"]));
    assert_eq!(
        query(ledger, "SELECT account, weight WHERE narration = 'sell one of b'"),
        vec![
            row(&["Assets:Broker", "-110 USD"]),
            row(&["Assets:Bank", "120 USD"]),
            row(&["Income:Gains", "-10 USD"])
        ]
    );
    // an inventory summing to zero shows no position, as in beanquery
    assert_eq!(query(ledger, "SELECT sum(weight) WHERE narration = 'sell one of b'"), vec![row(&[""])]);
}

/// A reduction without a label matches labelled lots (a label is a wildcard when left out), in
/// the store and in queries alike.
#[test]
fn unlabelled_reduction_books_labelled_lots_in_the_store_and_in_queries() {
    let ledger = r#"
1970-01-01 open Assets:Broker

2024-01-10 * "Broker" "buy a"
  Assets:Broker    10 AAPL {100 USD, "a"}
  Assets:Bank   -1000 USD

2024-01-20 * "Broker" "buy b"
  Assets:Broker    10 AAPL {110 USD, "b"}
  Assets:Bank   -1100 USD

2024-02-01 * "Broker" "sell across both"
  Assets:Broker   -15 AAPL {}
  Assets:Bank    1800 USD
  Income:Gains
"#;
    let loaded = common::load_text(&format!("{HEADER}{ledger}"));
    let store = loaded.store.read().unwrap();
    assert_eq!(store.errors.len(), 0);
    let lots = store.commodity_lots["Assets:Broker"]
        .iter()
        .map(|lot| {
            format!(
                "{} {} {{{}, {:?}}}",
                lot.amount,
                lot.commodity,
                lot.cost.as_ref().unwrap(),
                lot.label.as_deref().unwrap()
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(lots, vec!["5 AAPL {110 USD, \"b\"}"]);
    let gains = store.postings.iter().find(|it| it.account.name() == "Income:Gains").unwrap();
    // 10 × 100 + 5 × 110 = 1550 at cost, sold for 1800
    assert_eq!(gains.inferred_amount.to_string(), "-250 USD");
    drop(store);

    assert_eq!(
        query(ledger, "SELECT account, number, cost_label, weight WHERE narration = 'sell across both'"),
        vec![
            row(&["Assets:Broker", "-10", "a", "-1000 USD"]),
            row(&["Assets:Broker", "-5", "b", "-550 USD"]),
            row(&["Assets:Bank", "1800", "", "1800 USD"]),
            row(&["Income:Gains", "-250", "", "-250 USD"])
        ]
    );
    assert_eq!(query(ledger, "SELECT sum(weight) WHERE narration = 'sell across both'"), vec![row(&[""])]);
}

#[test]
fn augmentation_with_cost_and_no_date_is_dated_by_its_transaction() {
    let ledger = r#"
1970-01-01 open Assets:Broker

2024-01-01 * "Broker" "buy"
  Assets:Broker     5 AAPL {100 USD}
  Assets:Bank    -500 USD

2024-02-01 * "Broker" "buy more at the same cost"
  Assets:Broker     5 AAPL {100 USD}
  Assets:Bank    -500 USD

2024-03-01 * "Broker" "buy with an explicit lot date"
  Assets:Broker     2 AAPL {110 USD, 2023-12-01}
  Assets:Bank    -220 USD
"#;
    // beanquery: three lots, the same-cost buys stay apart by their transaction dates
    assert_eq!(
        postings(ledger, "Assets:Broker"),
        vec![
            row(&["5", "100", "2024-01-01", "NULL", "NULL"]),
            row(&["5", "100", "2024-02-01", "NULL", "NULL"]),
            row(&["2", "110", "2023-12-01", "NULL", "NULL"]),
        ]
    );
    assert_eq!(
        query(
            ledger,
            "SELECT cost_number, cost_date, sum(number) WHERE account = 'Assets:Broker' GROUP BY cost_number, cost_date ORDER BY cost_date"
        ),
        vec![
            row(&["110", "2023-12-01", "2"]),
            row(&["100", "2024-01-01", "5"]),
            row(&["100", "2024-02-01", "5"]),
        ]
    );
}

/// The rows are zhang's own booking (#458): where the query engine's former lot logic and the
/// ledger's disagreed, the rows now follow the ledger. Two such cases, pinned.
#[test]
fn rows_follow_the_ledgers_booking() {
    // E6 of the booking-split design: an augmentation written `{}` infers its cost from the
    // counterposting and opens a lot on its own date.
    let ledger = r#"
1970-01-01 open Assets:Broker

2024-01-10 * "Broker" "buy"
  Assets:Broker    10 AAPL {100 USD}
  Assets:Bank   -1000 USD

2024-01-20 * "Broker" "add to the lot"
  Assets:Broker     3 AAPL {}
  Assets:Bank    -300 USD
"#;
    assert_eq!(
        query(ledger, "SELECT number, cost_number, cost_date, weight WHERE narration = 'add to the lot'"),
        vec![row(&["3", "100", "2024-01-20", "300 USD"]), row(&["-300", "NULL", "NULL", "-300 USD"])]
    );
    assert_eq!(holdings(ledger, "Assets:Broker"), "10 AAPL {100 USD, 2024-01-10}, 3 AAPL {100 USD, 2024-01-20}");

    // a posting written without units but with a cost spec books the default lot of the weight
    // commodity, its spec ignored, as the ledger has always booked it; the engine used to open a
    // lot at that cost for it
    let ledger = r#"
1970-01-01 open Assets:A

2024-05-16 * "an implicit posting that carries a cost spec"
  Income:Gains -100 USD
  Assets:A { 10 AAPL }
"#;
    assert_eq!(
        query(ledger, "SELECT account, number, currency, cost_number WHERE account = 'Assets:A'"),
        vec![row(&["Assets:A", "100", "USD", "NULL"])]
    );

    // a sale of more than the lot holds, on the day the lot was bought and at its cost: the part
    // the lot covers and the remainder name the same lot, so they are one row (the engine used to
    // list them as two rows of -10). The ledger reports the shortfall as NoEnoughCommodityLot
    let ledger = r#"
1970-01-01 open Assets:Broker

2024-01-10 * "Broker" "buy"
  Assets:Broker    10 AAPL {100 USD}
  Assets:Bank   -1000 USD

2024-01-10 * "Broker" "sell more than held"
  Assets:Broker   -20 AAPL {100 USD}
  Assets:Bank    2000 USD
"#;
    assert_eq!(
        query(
            ledger,
            "SELECT number, cost_number, cost_date WHERE narration = 'sell more than held' AND account = 'Assets:Broker'"
        ),
        vec![row(&["-20", "100", "2024-01-10"])]
    );
    assert_eq!(holdings(ledger, "Assets:Broker"), "-10 AAPL {100 USD, 2024-01-10}");
}
