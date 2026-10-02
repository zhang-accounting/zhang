//! End-to-end checks of the scalar function library on the fava demo ledger. Every expected
//! value was cross-checked with beanquery 0.2.0 (`bean-query` on the same `main.zhang`).

mod common;

use chrono::NaiveDate;
use zhang_core::ledger::Ledger;
use zhang_query::{Params, Query, Value};

fn render(value: Value) -> String {
    match value {
        Value::Null => "NULL".to_owned(),
        other => other.to_string(),
    }
}

fn rows(ledger: &Ledger, query: &str) -> Vec<Vec<String>> {
    let result = zhang_query::execute(ledger, query).unwrap_or_else(|err| panic!("{}: {:?}", query, err));
    result.rows.into_iter().map(|row| row.into_iter().map(render).collect()).collect()
}

fn expect(ledger: &Ledger, query: &str, expected: &[&[&str]]) {
    let expected = expected
        .iter()
        .map(|row| row.iter().map(|it| it.to_string()).collect::<Vec<_>>())
        .collect::<Vec<_>>();
    assert_eq!(rows(ledger, query), expected, "{}", query);
}

#[test]
fn valuation_of_holdings() {
    let ledger = common::fava_demo_ledger();
    expect(
        &ledger,
        "SELECT account, str(sum(position)), str(units(sum(position))), str(cost(sum(position))), str(value(sum(position))), \
         str(value(sum(position), 2016-01-01)), str(convert(sum(position), 'USD')), str(convert(sum(position), 'CAD')) \
         WHERE account ~ '^Assets:US:ETrade:(GLD|VHT)$' GROUP BY account ORDER BY account",
        &[
            &[
                "Assets:US:ETrade:GLD",
                "(2 GLD {205.78 USD, 2016-05-05}, 5 GLD {231.38 USD, 2016-09-23}, 2 GLD {278.99 USD, 2017-03-19}, 8 GLD {304.17 USD, 2017-08-27})",
                "(17 GLD)",
                "(4559.80 USD)",
                "(5217.47 USD)",
                "(3409.52 USD)",
                "(5217.47 USD)",
                "(17 GLD)",
            ],
            &[
                "Assets:US:ETrade:VHT",
                "(9 VHT {113.91 USD, 2015-09-15}, 6 VHT {115.55 USD, 2017-03-19}, 10 VHT {119.17 USD, 2016-09-23}, 20 VHT {119.85 USD, 2016-08-13}, 19 VHT {132.32 USD, 2017-08-27})",
                "(64 VHT)",
                "(7821.27 USD)",
                "(8382.08 USD)",
                "(7185.28 USD)",
                "(8382.08 USD)",
                "(64 VHT)",
            ],
        ],
    );
    expect(
        &ledger,
        "SELECT date, str(cost(position)), str(value(position)), str(value(position, 2016-01-01)), \
         str(convert(position, 'USD', 2016-01-01)), str(convert(position, 'CAD')) \
         WHERE account = 'Assets:US:ETrade:GLD' ORDER BY date LIMIT 2",
        &[
            &["2015-09-15", "985.30 USD", "1534.55 USD", "1002.80 USD", "1002.80 USD", "5 GLD"],
            &["2015-11-13", "2090.00 USD", "3069.10 USD", "2005.60 USD", "2005.60 USD", "10 GLD"],
        ],
    );
    expect(
        &ledger,
        "SELECT getprice('GLD', 'USD'), getprice('gld', 'usd', 2016-01-01), getprice('USD', 'GLD', 2016-01-01), getprice('USD', 'CAD') LIMIT 1",
        &[&["306.91", "200.56", "0.004986039090546469884323893099", "NULL"]],
    );
}

#[test]
fn amount_functions() {
    let ledger = common::fava_demo_ledger();
    expect(
        &ledger,
        "SELECT date, str(units(position)), number(units(position)), currency(units(position)), str(possign(units(position), account)), \
         str(abs(position)), str(neg(position)), filter_currency(position, 'USD') IS NULL, filter_currency(position, 'EUR') IS NULL \
         WHERE account = 'Income:US:Hoogle:Salary' AND date <= 2015-01-31 ORDER BY date",
        &[
            &[
                "2015-01-01",
                "-4615.38 USD",
                "-4615.38",
                "USD",
                "4615.38 USD",
                "4615.38 USD",
                "4615.38 USD",
                "FALSE",
                "TRUE",
            ],
            &[
                "2015-01-15",
                "-4615.38 USD",
                "-4615.38",
                "USD",
                "4615.38 USD",
                "4615.38 USD",
                "4615.38 USD",
                "FALSE",
                "TRUE",
            ],
            &[
                "2015-01-29",
                "-4615.38 USD",
                "-4615.38",
                "USD",
                "4615.38 USD",
                "4615.38 USD",
                "4615.38 USD",
                "FALSE",
                "TRUE",
            ],
        ],
    );
    expect(
        &ledger,
        "SELECT root(account) AS r, str(sum(position)), str(possign(sum(position), 'Income')), str(abs(sum(position))), \
         str(neg(sum(position))), str(only('USD', sum(position))), str(filter_currency(sum(position), 'VACHR')) \
         WHERE account ~ '^(Income|Liabilities)' GROUP BY r ORDER BY r",
        &[
            &[
                "Income",
                "(-357944.58 USD, -355 VACHR, -54500 IRAUSD)",
                "(357944.58 USD, 355 VACHR, 54500 IRAUSD)",
                "(357944.58 USD, 355 VACHR, 54500 IRAUSD)",
                "(357944.58 USD, 355 VACHR, 54500 IRAUSD)",
                "-357944.58 USD",
                "(-355 VACHR)",
            ],
            &[
                "Liabilities",
                "(-2703.29 USD)",
                "(2703.29 USD)",
                "(2703.29 USD)",
                "(2703.29 USD)",
                "-2703.29 USD",
                "()",
            ],
        ],
    );
}

#[test]
fn account_date_string_and_meta_functions() {
    let ledger = common::fava_demo_ledger();
    expect(
        &ledger,
        "SELECT DISTINCT parent(account), leaf(account), parent(root(account)) WHERE account ~ 'ETrade' ORDER BY 1, 2",
        &[
            &["Assets:US:ETrade", "Cash", ""],
            &["Assets:US:ETrade", "GLD", ""],
            &["Assets:US:ETrade", "ITOT", ""],
            &["Assets:US:ETrade", "VEA", ""],
            &["Assets:US:ETrade", "VHT", ""],
            &["Income:US:ETrade", "Dividends", ""],
            &["Income:US:ETrade", "Gains", ""],
        ],
    );
    expect(
        &ledger,
        "SELECT date, quarter(date), weekday(date), month(date), day(date), yearmonth(date) \
         WHERE account = 'Expenses:Home:Rent' AND year = 2016 AND month(date) <= 3 ORDER BY date",
        &[
            &["2016-01-06", "2016-Q1", "Wed", "1", "6", "2016-01-01"],
            &["2016-02-03", "2016-Q1", "Wed", "2", "3", "2016-02-01"],
            &["2016-03-06", "2016-Q1", "Sun", "3", "6", "2016-03-01"],
        ],
    );
    expect(
        &ledger,
        "SELECT DISTINCT str(year), str(number), str(TRUE), length(narration), length(tags) \
         WHERE account = 'Expenses:Financial:Fees' AND year = 2016",
        &[&["2016", "4.00", "TRUE", "16", "0"]],
    );
    expect(
        &ledger,
        "SELECT DISTINCT meta('x'), entry_meta('x'), any_meta('x') WHERE account ~ '^Expenses:Food'",
        &[&["NULL", "NULL", "NULL"]],
    );
}

#[test]
fn today_comes_from_the_execution() {
    let ledger = common::fava_demo_ledger();
    let today = NaiveDate::from_ymd_opt(2024, 2, 29).unwrap();
    let result = Query::compile("SELECT today(), quarter(today()) LIMIT 1")
        .unwrap()
        .execute_at(&ledger, &Params::default(), today)
        .unwrap();
    assert_eq!(result.rows, vec![vec![Value::Date(today), Value::from("2024-Q1")]]);
}
