//! `now()`: the time of day of the execution by the ledger's clock in its timezone, next to `today()`, its date. Both
//! read one instant, so `account_status(account, today(), now())` is whether an account is open now.

mod common;

use chrono::NaiveDate;
use zhang_core::clock::Clock;
use zhang_query::{Params, Query, Value};

const LEDGER: &str = "option \"operating_currency\" \"CNY\"\n\
                      option \"timezone\" \"Asia/Shanghai\"\n\
                      1970-01-01 open Assets:Cash\n\
                      2025-06-15 10:00:00 close Assets:Cash\n";

fn rows(ledger: &zhang_core::ledger::Ledger, sql: &str) -> Vec<Vec<Value>> {
    Query::compile(sql).unwrap().execute(ledger, &Params::new()).unwrap().rows
}

fn date(text: &str) -> Value {
    Value::Date(NaiveDate::parse_from_str(text, "%Y-%m-%d").unwrap())
}

#[test]
fn now_is_the_ledgers_clock_in_its_timezone() {
    // 04:00 UTC is 12:00 in Shanghai
    let ledger = common::load_text_at(LEDGER, Clock::Fixed("2025-06-15T04:00:00Z".parse().unwrap()));
    assert_eq!(
        rows(&ledger, "SELECT today() AS date, now() AS time FROM #accounts LIMIT 1"),
        vec![vec![date("2025-06-15"), Value::from("12:00:00")]]
    );
    // the account closed at 10:00 that day is closed now
    assert_eq!(
        rows(&ledger, "SELECT account_status(account, today(), now()) FROM #accounts"),
        vec![vec![Value::from("closed")]]
    );
    // 20:30 UTC is already 04:30 the next day there: today() and now() describe the same instant
    let ledger = common::load_text_at(LEDGER, Clock::Fixed("2025-06-15T20:30:00Z".parse().unwrap()));
    assert_eq!(
        rows(&ledger, "SELECT today() AS date, now() AS time FROM #accounts LIMIT 1"),
        vec![vec![date("2025-06-16"), Value::from("04:30:00")]]
    );
    // one instant per execution: every row reads the same time
    let ledger = common::load_text_at(LEDGER, Clock::Fixed("2025-06-15T04:00:00.999Z".parse().unwrap()));
    assert_eq!(rows(&ledger, "SELECT DISTINCT now() FROM #accounts"), vec![vec![Value::from("12:00:00")]]);
}

#[test]
fn a_fixed_date_makes_now_midnight() {
    let ledger = common::load_text(LEDGER);
    let result = Query::compile("SELECT today() AS date, now() AS time FROM #accounts LIMIT 1")
        .unwrap()
        .execute_at(&ledger, &Params::new(), NaiveDate::from_ymd_opt(2024, 6, 30).unwrap())
        .unwrap();
    assert_eq!(result.rows, vec![vec![date("2024-06-30"), Value::from("00:00:00")]]);
}
