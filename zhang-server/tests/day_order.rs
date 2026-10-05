//! The order of a day: zhang lists the entries of a ledger in the order it checks and books them, the order `seq`
//! numbers. A zhang ledger orders a day by time; a beancount ledger as beancount does, with a balance at the start of
//! its day and a `close` at its end. The journal, `#entries` and the other directive tables all follow that order.

use std::sync::Arc;

use chrono::NaiveDate;
use zhang_ast::error::ErrorKind;
use zhang_core::data_source::{DataSource, LocalFileSystemDataSource};
use zhang_core::data_type::text::ZhangDataType;
use zhang_core::ledger::Ledger;
use zhang_query::{Params, Query, Value};

/// The ledger of one file named `main`, read in the format of its extension.
fn load(main: &str, content: &str) -> Ledger {
    let dir = tempfile::tempdir().expect("tempdir").keep();
    std::fs::write(dir.join(main), content).expect("write the ledger");
    let source: Arc<dyn DataSource> = if main.ends_with(".bean") {
        Arc::new(LocalFileSystemDataSource::new(beancount::Beancount {}))
    } else {
        Arc::new(LocalFileSystemDataSource::new(ZhangDataType {}))
    };
    Ledger::load_with_data_source(dir, main.to_owned(), source).unwrap_or_else(|error| panic!("{main} should load: {error}"))
}

fn run(ledger: &Ledger, sql: &str) -> Vec<Vec<String>> {
    let today = NaiveDate::from_ymd_opt(2025, 1, 1).unwrap();
    let result = Query::compile(sql)
        .and_then(|query| query.execute_at(ledger, &Params::new(), today))
        .unwrap_or_else(|error| panic!("{sql}: {error}"));
    result.rows.iter().map(|row| row.iter().map(Value::to_string).collect()).collect()
}

fn rows(rows: &[&[&str]]) -> Vec<Vec<String>> {
    rows.iter().map(|row| row.iter().map(|it| (*it).to_owned()).collect()).collect()
}

/// The `#entries` rows of `day`, as listed and as ordered by `seq`: the same rows in the same order, `seq` counting up
/// one by one. Returns them without `seq`.
fn day(ledger: &Ledger, day: &str, columns: &str) -> Vec<Vec<String>> {
    let listed = run(ledger, &format!("SELECT seq, {columns} FROM #entries WHERE date = {day}"));
    assert_eq!(
        listed,
        run(ledger, &format!("SELECT seq, {columns} FROM #entries WHERE date = {day} ORDER BY seq")),
        "the entries of {day} are listed in the order zhang processed them"
    );
    let seqs = listed.iter().map(|row| row[0].parse::<i64>().unwrap()).collect::<Vec<_>>();
    assert!(seqs.windows(2).all(|it| it[1] == it[0] + 1), "{day}: {seqs:?}");
    listed.into_iter().map(|row| row[1..].to_vec()).collect()
}

fn errors(ledger: &Ledger) -> Vec<ErrorKind> {
    ledger.store.read().unwrap().errors.iter().map(|it| it.error_type.clone()).collect()
}

/// A zhang ledger checks a balance at its time: after the transaction of the morning, before the one of the evening.
/// The journal lists it there too, where its amount, which includes the morning, is checked.
#[test]
fn a_zhang_ledger_lists_a_timed_balance_after_the_transactions_before_its_time() {
    let ledger = load(
        "main.zhang",
        r#"
option "operating_currency" "CNY"
1970-01-01 commodity CNY
1970-01-01 open Assets:A
1970-01-01 open Equity:Open
2024-01-05 12:00:00 balance Assets:A 5 CNY
2024-01-05 18:00:00 * "evening"
  Assets:A 1 CNY
  Equity:Open
2024-01-05 09:00:00 * "morning"
  Assets:A 5 CNY
  Equity:Open
"#,
    );
    assert_eq!(
        day(&ledger, "2024-01-05", "type, time, narration"),
        rows(&[
            &["transaction", "09:00:00", "morning"],
            &["balance", "12:00:00", "NULL"],
            &["transaction", "18:00:00", "evening"],
        ])
    );
    assert_eq!(
        run(&ledger, "SELECT amount, actual, passed FROM #balances"),
        rows(&[&["5 CNY", "5 CNY", "TRUE"]])
    );
    assert_eq!(errors(&ledger), vec![]);
}

/// A `close` with only a date takes effect at the end of its day: zhang checks that the account is empty there, after
/// the transactions of that day, and lists it last. A `close` with a time is checked and listed at that time.
#[test]
fn a_zhang_ledger_closes_an_account_at_the_end_of_the_day_of_a_close_with_only_a_date() {
    let ledger = load(
        "main.zhang",
        r#"
option "operating_currency" "CNY"
1970-01-01 commodity CNY
1970-01-01 open Assets:A
1970-01-01 open Assets:B
1970-01-01 open Equity:Open
2024-01-04 * "fill"
  Assets:A 3 CNY
  Assets:B 3 CNY
  Equity:Open
2024-01-06 close Assets:A
2024-01-06 18:00:00 * "empty A"
  Assets:A -3 CNY
  Equity:Open
2024-01-07 12:00:00 close Assets:B
2024-01-07 18:00:00 * "empty B too late"
  Assets:B -3 CNY
  Equity:Open
"#,
    );
    assert_eq!(
        day(&ledger, "2024-01-06", "type, narration"),
        rows(&[&["transaction", "empty A"], &["close", "NULL"]])
    );
    assert_eq!(
        day(&ledger, "2024-01-07", "type, narration"),
        rows(&[&["close", "NULL"], &["transaction", "empty B too late"]])
    );
    // only the timed close finds money in its account, and the transaction after it a closed account
    assert_eq!(
        run(&ledger, "SELECT kind, line FROM #errors ORDER BY line"),
        rows(&[&["CloseNonZeroAccount", "15"], &["AccountClosed", "16"]])
    );
}

/// Within one date and time, a zhang ledger orders its directives as beancount orders a day: an `open` before a
/// `commodity` of the same date, which it may name.
#[test]
fn an_open_may_name_a_commodity_of_its_own_date_written_after_it() {
    let ledger = load(
        "main.zhang",
        r#"
1970-01-01 open Assets:A EUR
1970-01-01 commodity EUR
1970-01-02 open Assets:B USD
1970-01-03 commodity USD
"#,
    );
    assert_eq!(day(&ledger, "1970-01-01", "type"), rows(&[&["open"], &["commodity"]]));
    // a commodity defined only on a later day is still undefined
    assert_eq!(run(&ledger, "SELECT kind, line FROM #errors"), rows(&[&["CommodityDoesNotDefine", "4"]]));
}

/// A beancount ledger orders a day as beancount does: the balance first, at the start of the day, whatever its `time`
/// metadata, then the transactions by their `time`, then the `document`, and the `close` last, which is checked
/// after the transactions of its day.
#[test]
fn a_beancount_ledger_lists_a_day_in_beancount_order() {
    let ledger = load(
        "main.bean",
        r#"
option "operating_currency" "CNY"
1970-01-01 commodity CNY
1970-01-01 open Assets:A
1970-01-01 open Assets:B
1970-01-01 open Equity:Open
2024-01-04 * "fill"
  Assets:B 3 CNY
  Equity:Open
2024-01-05 close Assets:B
2024-01-05 * "evening"
  time: "18:00:00"
  Assets:B -3 CNY
  Equity:Open
2024-01-05 document Assets:A "statement.pdf"
2024-01-05 * "morning"
  time: "09:00:00"
  Assets:A 5 CNY
  Equity:Open
2024-01-05 balance Assets:A 0 CNY
  time: "12:00:00"
"#,
    );
    assert_eq!(
        day(&ledger, "2024-01-05", "type, narration"),
        rows(&[
            &["balance", "NULL"],
            &["transaction", "morning"],
            &["transaction", "evening"],
            &["document", "NULL"],
            &["close", "NULL"],
        ])
    );
    assert_eq!(
        run(&ledger, "SELECT amount, actual, passed FROM #balances"),
        rows(&[&["0 CNY", "0 CNY", "TRUE"]])
    );
    assert!(!errors(&ledger).contains(&ErrorKind::CloseNonZeroAccount), "{:?}", errors(&ledger));
}
