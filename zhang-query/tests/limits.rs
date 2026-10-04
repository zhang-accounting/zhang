//! The result size limit ([`ExecuteOptions::max_result_values`]): an execution that would hold
//! more values stops with a `TooLarge` error, and ordinary queries never reach the default.

mod common;

use std::sync::OnceLock;

use chrono::NaiveDate;
use zhang_core::ledger::Ledger;
use zhang_query::{ExecuteOptions, Params, Query, QueryError, QueryErrorKind, QueryResult, DEFAULT_MAX_RESULT_VALUES};

/// A broker account that buys one share a day at a new price, so `lots` lots stay open,
/// and a cash account that pays for them.
fn lots_ledger(lots: usize) -> String {
    let mut ledger = String::from(
        "option \"operating_currency\" \"USD\"\n\
         1970-01-01 commodity USD\n1970-01-01 commodity STK\n\
         1970-01-01 open Assets:Broker\n1970-01-01 open Assets:Cash\n",
    );
    let start = NaiveDate::from_ymd_opt(2020, 1, 1).unwrap();
    for day in 0..lots {
        let date = start + chrono::Duration::days(day as i64);
        ledger.push_str(&format!(
            "\n{date} * \"buy {day}\"\n  Assets:Broker 1 STK {{{price}.00 USD}}\n  Assets:Cash -{price}.00 USD\n",
            price = 100 + day
        ));
    }
    ledger
}

/// 200 open lots: a `JOURNAL` of 400 rows whose running balances hold 20,000 positions.
fn ledger() -> &'static Ledger {
    static LEDGER: OnceLock<Ledger> = OnceLock::new();
    LEDGER.get_or_init(|| common::load_text(&lots_ledger(200)))
}

fn run(sql: &str, max_result_values: Option<u64>) -> Result<QueryResult, QueryError> {
    let options = ExecuteOptions {
        today: Some(NaiveDate::from_ymd_opt(2026, 1, 1).unwrap()),
        timeout: None,
        max_result_values,
        count_total: false,
    };
    Query::compile(sql)?.execute_with_options(ledger(), &Params::new(), &options)
}

fn too_large(sql: &str, limit: u64) -> QueryError {
    let err = run(sql, Some(limit)).expect_err(sql);
    assert_eq!(err.kind, QueryErrorKind::TooLarge, "{}: {}", sql, err);
    assert_eq!(
        err.message,
        format!(
            "the result is too large: it would hold more than {} values (cells, plus the positions of inventories); \
             narrow the query with FROM or WHERE, or add a LIMIT",
            limit
        )
    );
    assert_eq!((err.line, err.column), (None, None));
    err
}

#[test]
fn a_journal_of_open_lots_stops_at_the_limit() {
    // 400 rows of 7 cells and running balances of up to 200 lots
    assert!(run("JOURNAL", None).unwrap().rows.len() == 400);
    too_large("JOURNAL", 10_000);
    too_large("JOURNAL 'Broker'", 10_000);
    too_large("SELECT date, balance WHERE account ~ 'Broker'", 10_000);
    // sorting on the balance needs every balance first
    too_large("SELECT date, balance WHERE account ~ 'Broker' ORDER BY balance DESC", 10_000);
    // text counts too: str(balance) is as large as the balance
    too_large("SELECT str(balance) WHERE account ~ 'Broker'", 10_000);
    // so do the groups of an aggregate query while they are built
    too_large("SELECT date, last(balance) WHERE account ~ 'Broker' GROUP BY date", 10_000);
    too_large("SELECT date, account, count(*) GROUP BY date, account", 500);
}

#[test]
fn small_results_stay_within_the_limit() {
    for sql in [
        // a LIMIT without ORDER BY stops early
        "JOURNAL 'Broker' AT cost",
        "JOURNAL 'Broker' AT units",
        "SELECT date, balance WHERE account ~ 'Broker' LIMIT 3",
        // only the rows ORDER BY and LIMIT keep build a balance
        "SELECT date, balance WHERE account ~ 'Broker' ORDER BY date DESC LIMIT 3",
        "SELECT date, balance WHERE account ~ 'Broker' ORDER BY balance DESC LIMIT 3",
        "SELECT account, last(balance) GROUP BY account",
        "BALANCES",
        "SELECT count(*), sum(position)",
    ] {
        let result = run(sql, Some(10_000)).unwrap_or_else(|err| panic!("{}: {}", sql, err));
        assert_eq!(result, run(sql, None).unwrap(), "{}", sql);
    }
    // the rows before OFFSET of a LIMIT without ORDER BY are only counted, not built: a page at the end of the
    // journal holds its own rows, with their running balance, and nothing more
    let sql = "SELECT date, number, units(balance) WHERE account ~ 'Broker' LIMIT 3 OFFSET 195";
    let page = run(sql, Some(3 * 4)).unwrap_or_else(|err| panic!("{}: {}", sql, err));
    assert_eq!(page, run(sql, None).unwrap());
    let units = page.rows.iter().map(|row| row[2].to_string()).collect::<Vec<_>>();
    assert_eq!(units, ["196 STK", "197 STK", "198 STK"]);
    too_large(sql, 3 * 4 - 1);
    // an offset past the rows leaves none
    assert!(run("SELECT date WHERE account ~ 'Broker' LIMIT 3 OFFSET 1000", Some(1))
        .unwrap()
        .rows
        .is_empty());
    // exactly at the limit is allowed: 200 rows of a date (1) and a one-position inventory (2)
    let sql = "SELECT date, units(balance) WHERE account ~ 'Broker'";
    assert_eq!(run(sql, Some(200 * 3)).unwrap().rows.len(), 200);
    too_large(sql, 200 * 3 - 1);
}

#[test]
fn the_default_limit_covers_the_fava_demo_ledger() {
    assert_eq!(ExecuteOptions::default().max_result_values, Some(DEFAULT_MAX_RESULT_VALUES));
    let ledger = common::fava_demo_ledger();
    for (sql, rows) in [("JOURNAL", 3209), ("BALANCES", 58), ("SELECT *", 3209)] {
        let result = Query::compile(sql).unwrap().execute(&ledger, &Params::new()).unwrap();
        assert_eq!(result.rows.len(), rows, "{}", sql);
    }
}

/// The smallest result size limit a query runs within: the most values it holds at once.
fn peak(sql: &str) -> u64 {
    let (mut low, mut high) = (0u64, 1u64);
    while run(sql, Some(high)).is_err() {
        low = high;
        high *= 2;
    }
    // `low` fails, `high` runs
    while high - low > 1 {
        let middle = low + (high - low) / 2;
        if run(sql, Some(middle)).is_ok() {
            high = middle;
        } else {
            low = middle;
        }
    }
    high
}

#[test]
fn an_aggregate_written_twice_is_computed_and_held_once() {
    let once = "SELECT date, last(balance) WHERE account ~ 'Broker' GROUP BY date";
    let thrice = "SELECT date, last(balance), units(last(balance)), cost(last(balance)) WHERE account ~ 'Broker' GROUP BY date";
    // one accumulation feeds the three targets
    let explain = Query::compile(thrice).unwrap().explain();
    assert!(explain.contains("agg#0: last(") && !explain.contains("agg#1"), "{}", explain);
    // with the same results as three accumulations
    let result = run(thrice, None).unwrap();
    let apart = run(
        "SELECT date, last(balance), units(last(balance)), cost(last(balance)) WHERE account ~ 'Broker' GROUP BY date ORDER BY date",
        None,
    )
    .unwrap();
    assert_eq!(result.rows, apart.rows);
    for (row, single) in result.rows.iter().zip(run(once, None).unwrap().rows) {
        assert_eq!(row[1], single[1]);
    }
    // the groups hold one balance each, not three: 200 balances of up to 200 lots, then the
    // rows add the two small targets
    let (once, thrice) = (peak(once), peak(thrice));
    assert!(thrice < once + once / 10, "held {} values for three targets, {} for one", thrice, once);
}

#[test]
fn having_drops_groups_before_they_hold_their_balance() {
    // the last 3 of 200 days: only their balances are built
    let kept = "SELECT date, last(balance) WHERE account ~ 'Broker' GROUP BY date HAVING max(date) >= 2020-07-16";
    let result = run(kept, None).unwrap();
    assert_eq!(result.rows.len(), 3);
    let all = "SELECT date, last(balance) WHERE account ~ 'Broker' GROUP BY date";
    // the 197 dropped groups hold about 600 values (a date, a pick and a max(date) each) until
    // HAVING drops them; once released, the 3 kept balances of about 200 lots fit in 700, which
    // the dropped groups' 600 would not leave room for
    assert!(run(kept, Some(700)).is_ok(), "{}", run(kept, Some(700)).unwrap_err());
    let (kept, all) = (peak(kept), peak(all));
    assert!(kept < 700, "the kept groups held {} values", kept);
    assert!(all > 20_000, "all the groups held {} values", all);
    // a HAVING that reads the deferred value itself still sees it
    let on_balance = "SELECT date, last(balance) WHERE account ~ 'Broker' GROUP BY date HAVING length(str(last(balance))) > 0";
    assert_eq!(run(on_balance, None).unwrap().rows.len(), 200);
}

/// The rows before OFFSET of a query without ORDER BY are not built, as those past LIMIT: an error that
/// only computing their targets would raise is not reported. They still go through WHERE, whose errors
/// are.
#[test]
fn rows_skipped_by_offset_raise_the_errors_of_the_filter_only() {
    // the first buy is on day 1: 9223372036854775807 + 2 - day overflows on it only
    let target = "SELECT 9223372036854775807 + (2 - day) WHERE account ~ 'Broker' LIMIT 2 OFFSET 1";
    assert_eq!(run(target, None).unwrap().rows.len(), 2);
    let whole = "SELECT 9223372036854775807 + (2 - day) WHERE account ~ 'Broker' LIMIT 2";
    assert_eq!(run(whole, None).unwrap_err().kind, QueryErrorKind::Eval);
    let filter = "SELECT date WHERE 9223372036854775807 + (2 - day) > 0 AND account ~ 'Broker' LIMIT 2 OFFSET 1";
    assert_eq!(run(filter, None).unwrap_err().kind, QueryErrorKind::Eval);
}
