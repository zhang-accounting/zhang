//! The data the read endpoints need from the zhang tables (#479): the true balance of
//! `#balances`, the documents of transactions in `#documents`, the ids and spans of `#errors`,
//! the converted figures of `#budgets` and the new `#budget_events`.
//!
//! These are zhang extensions, so there is no beanquery oracle: every expected value is
//! computed by hand from the ledger of its test, as the comments explain.
//! `zhang-server/tests/query_zhang_tables.rs` checks the same tables against the APIs.

mod common;

use std::path::{Path, PathBuf};

use chrono::NaiveDate;
use zhang_ast::{Date, Directive, Spanned};
use zhang_core::ledger::Ledger;
use zhang_query::{Params, Query, Value};

fn today() -> NaiveDate {
    NaiveDate::from_ymd_opt(2024, 12, 31).unwrap()
}

fn run(ledger: &Ledger, sql: &str) -> Vec<Vec<String>> {
    let result = Query::compile(sql)
        .and_then(|query| query.execute_at(ledger, &Params::new(), today()))
        .unwrap_or_else(|err| panic!("{}: {}", sql, err));
    result.rows.iter().map(|row| row.iter().map(Value::to_string).collect()).collect()
}

fn rows(rows: &[&[&str]]) -> Vec<Vec<String>> {
    rows.iter().map(|row| row.iter().map(|it| it.to_string()).collect()).collect()
}

/// A ledger of several files in a temporary directory; `{root}` in a file stands for the
/// directory. Returns the ledger and the directory.
fn load_files(files: &[(&str, &str)]) -> (Ledger, PathBuf) {
    // canonical, as the ledger names its directory
    let dir = tempfile::tempdir().expect("tempdir").into_path().canonicalize().unwrap();
    for (name, content) in files {
        let path = dir.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content.replace("{root}", &dir.to_string_lossy())).unwrap();
    }
    (common::load_ledger(dir.clone(), "main.zhang"), dir)
}

// ---------------------------------------------------------------------------------------
// #balances: actual and passed

const BALANCES: &str = r#"
option "operating_currency" "CNY"
option "timezone" "Asia/Shanghai"

1970-01-01 commodity CNY
1970-01-01 commodity USD
1970-01-01 open Assets:Bank
1970-01-01 open Assets:Bank:Checking
1970-01-01 open Assets:Wallet
1970-01-01 open Equity:Open
1970-01-01 open Income:Salary

2024-01-01 * "Employer" "salary"
  Assets:Bank:Checking 1000 CNY
  Income:Salary

2024-01-01 * "Self" "a posting to the parent account"
  Assets:Bank 50 CNY
  Equity:Open

2024-01-04 23:30:00 * "Late" "before the check of the 5th"
  Assets:Bank:Checking -10 CNY
  Income:Salary

2024-01-05 10:00:00 * "Morning" "after the check of the 5th, which asserts the start of the day"
  Assets:Bank:Checking -20 CNY
  Income:Salary

2024-01-05 balance Assets:Bank:Checking 990 CNY
2024-01-06 balance Assets:Bank:Checking 1000 CNY
2024-01-07 balance Assets:Bank:Checking 970 CNY
2024-01-08 balance Assets:Bank 50.004 ~ 0.01 CNY
2024-01-08 balance Assets:Bank 50.02 ~ 0.01 CNY

2024-01-09 * "FX" "buy dollars"
  Assets:Bank:Checking 100 USD @ 7 CNY
  Assets:Bank:Checking -700 CNY

2024-01-10 balance Assets:Bank:Checking 270 CNY
2024-01-10 balance Assets:Bank:Checking 100 USD
2024-01-11 balance Assets:Wallet 0 USD
2024-01-12 balance Assets:Wallet 25 CNY with pad Equity:Open
"#;

#[test]
fn balances_have_the_true_balance_and_whether_the_assertion_holds() {
    let ledger = common::load_text(BALANCES);
    assert_eq!(
        run(&ledger, "SELECT date, account, amount, tolerance, discrepancy, actual, passed FROM #balances"),
        rows(&[
            // 1000 - 10 at 23:30 the day before; the 10:00 posting of the 5th comes after it
            &["2024-01-05", "Assets:Bank:Checking", "990 CNY", "NULL", "NULL", "990 CNY", "TRUE"],
            // 1000 - 10 - 20 = 970: the assertion fails, the discrepancy is actual - amount
            &["2024-01-06", "Assets:Bank:Checking", "1000 CNY", "NULL", "-30 CNY", "970 CNY", "FALSE"],
            // the failed assertion did not move the balance: it is still 970
            &["2024-01-07", "Assets:Bank:Checking", "970 CNY", "NULL", "NULL", "970 CNY", "TRUE"],
            // the account's sub-accounts count too, as beancount checks a balance: its own 50 and
            // the 970 of Assets:Bank:Checking, far from both asserted amounts
            &["2024-01-08", "Assets:Bank", "50.004 CNY", "0.01", "969.996 CNY", "1020 CNY", "FALSE"],
            &["2024-01-08", "Assets:Bank", "50.02 CNY", "0.01", "969.98 CNY", "1020 CNY", "FALSE"],
            // one currency of an account holding two: 970 - 700 CNY, and 100 USD
            &["2024-01-10", "Assets:Bank:Checking", "270 CNY", "NULL", "NULL", "270 CNY", "TRUE"],
            &["2024-01-10", "Assets:Bank:Checking", "100 USD", "NULL", "NULL", "100 USD", "TRUE"],
            // no posting at all: zero in the asserted currency
            &["2024-01-11", "Assets:Wallet", "0 USD", "NULL", "NULL", "0 USD", "TRUE"],
            // a balance with pad is checked after its own padding transaction
            &["2024-01-12", "Assets:Wallet", "25 CNY", "NULL", "NULL", "25 CNY", "TRUE"],
        ])
    );
    // SELECT * keeps beanquery's columns
    assert_eq!(
        Query::compile("SELECT * FROM #balances")
            .unwrap()
            .columns()
            .iter()
            .map(|it| it.name.as_str())
            .collect::<Vec<_>>(),
        ["date", "account", "amount", "tolerance", "discrepancy"]
    );
    // the failed assertions per account
    assert_eq!(
        run(
            &ledger,
            "SELECT account, count(*) FROM #balances WHERE NOT passed GROUP BY account ORDER BY account"
        ),
        rows(&[&["Assets:Bank", "2"], &["Assets:Bank:Checking", "1"]])
    );
}

/// The assertions `passed` says fail are the errors of zhang's balance check, and the other way
/// round: assertions no longer move balances, so no check fails against a balance an earlier
/// failed check moved (the one of the 7th holds).
#[test]
fn failed_assertions_are_balance_check_errors() {
    let ledger = common::load_text(BALANCES);
    let failed = run(&ledger, "SELECT date, account FROM #balances WHERE NOT passed ORDER BY date, account");
    let errors = run(
        &ledger,
        "SELECT date, account FROM #errors WHERE kind = 'AccountBalanceCheckError' ORDER BY date, account",
    );
    assert_eq!(failed.len(), 3);
    assert_eq!(failed, errors);
}

const PARENT_ACCOUNTS: &str = r#"
option "operating_currency" "CNY"

1970-01-01 commodity CNY
1970-01-01 open Assets:Bank
1970-01-01 open Assets:Bank:Checking
1970-01-01 open Assets:Bank:Savings
1970-01-01 open Equity:Opening
1970-01-01 open Income:Salary

2024-01-02 * "Opening"
  Assets:Bank 345 CNY
  Assets:Bank:Checking 155 CNY
  Equity:Opening

2024-01-03 * "Salary"
  Assets:Bank:Savings 100 CNY
  Income:Salary

2024-01-04 balance Assets:Bank 600 CNY
2024-01-05 balance Assets:Bank 345 CNY
2024-01-06 balance Assets:Bank 600.004 ~ 0.01 CNY
2024-01-07 balance Assets:Bank:Checking 155 CNY
2024-01-08 balance Assets:Bank:Savings 99 CNY

2024-01-10 balance Assets:Bank 700 CNY with pad Equity:Opening
2024-01-10 balance Assets:Bank:Checking 200 CNY with pad Equity:Opening
2024-01-11 balance Assets:Bank 745 CNY
"#;

/// `passed` is zhang's balance check: an assertion passes exactly when zhang reports no
/// `AccountBalanceCheckError` for it, also for parent accounts, whose balance includes their
/// sub-accounts, for a tolerance, and for a `balance ... with pad`, which is checked once the
/// pads of its time are booked.
#[test]
fn passed_is_the_balance_check_of_zhang() {
    let ledger = common::load_text(PARENT_ACCOUNTS);
    assert_eq!(
        run(&ledger, "SELECT date, account, amount, discrepancy, actual, passed FROM #balances"),
        rows(&[
            // 345 of its own, 155 of Assets:Bank:Checking and 100 of Assets:Bank:Savings
            &["2024-01-04", "Assets:Bank", "600 CNY", "NULL", "600 CNY", "TRUE"],
            &["2024-01-05", "Assets:Bank", "345 CNY", "255 CNY", "600 CNY", "FALSE"],
            // 0.004 away, within the tolerance of 0.01
            &["2024-01-06", "Assets:Bank", "600.004 CNY", "NULL", "600 CNY", "TRUE"],
            &["2024-01-07", "Assets:Bank:Checking", "155 CNY", "NULL", "155 CNY", "TRUE"],
            &["2024-01-08", "Assets:Bank:Savings", "99 CNY", "1 CNY", "100 CNY", "FALSE"],
            // the pad of the parent brings it to 700 (+100), then the pad of its sub-account of
            // the same time adds 45 (155 to 200): checked after both, the parent is at 745
            &["2024-01-10", "Assets:Bank", "700 CNY", "45 CNY", "745 CNY", "FALSE"],
            &["2024-01-10", "Assets:Bank:Checking", "200 CNY", "NULL", "200 CNY", "TRUE"],
            // the failed check moved nothing
            &["2024-01-11", "Assets:Bank", "745 CNY", "NULL", "745 CNY", "TRUE"],
        ])
    );
    let failed = run(&ledger, "SELECT date, account FROM #balances WHERE NOT passed ORDER BY date, account");
    let errors = run(
        &ledger,
        "SELECT date, account FROM #errors WHERE kind = 'AccountBalanceCheckError' ORDER BY date, account",
    );
    assert_eq!(failed, errors);
    assert_eq!(failed.len(), 3);
}

/// An assertion has its place among the entries: `seq`, `id`, `time` and `timestamp` are those of its
/// `#entries` row, so its `seq` orders it with the `seq` of the postings, and `pad` is the account a
/// `balance ... with pad` pads from.
#[test]
fn balances_have_their_place_among_the_entries() {
    let ledger = common::load_text(PARENT_ACCOUNTS);
    assert_eq!(
        run(&ledger, "SELECT seq, id, date, time, timestamp FROM #balances"),
        run(&ledger, "SELECT seq, id, date, time, timestamp FROM #entries WHERE type = 'balance'")
    );
    // an assertion without a time is at midnight, and its timestamp is that of its day
    assert_eq!(run(&ledger, "SELECT time FROM #balances LIMIT 1"), rows(&[&["00:00:00"]]));
    let day = run(&ledger, "SELECT timestamp FROM #balances WHERE date = 2024-01-10 LIMIT 1")[0][0]
        .parse::<i64>()
        .unwrap();
    let next = run(&ledger, "SELECT timestamp FROM #balances WHERE date = 2024-01-11")[0][0]
        .parse::<i64>()
        .unwrap();
    assert_eq!(next - day, 24 * 60 * 60);
    // the salary of the 3rd comes before the assertion of the 4th, the opening of the 2nd before both
    let salary = run(&ledger, "SELECT seq FROM #postings WHERE narration = 'Salary' LIMIT 1")[0][0]
        .parse::<i64>()
        .unwrap();
    let first = run(&ledger, "SELECT seq FROM #balances LIMIT 1")[0][0].parse::<i64>().unwrap();
    assert!(salary < first, "{salary} {first}");
    assert_eq!(
        run(&ledger, "SELECT date, account, pad FROM #balances WHERE date >= 2024-01-10"),
        rows(&[
            &["2024-01-10", "Assets:Bank", "Equity:Opening"],
            &["2024-01-10", "Assets:Bank:Checking", "Equity:Opening"],
            &["2024-01-11", "Assets:Bank", "NULL"],
        ])
    );
}

/// `seq` follows the order zhang processes the ledger in, so merging the postings and the
/// assertions by `seq` puts every assertion right after the postings its balance includes:
/// - a balance with a time is checked after the transactions of its day that come before that
///   time (the 15:30 check after the lunch of midnight);
/// - a directive without a balance, such as a document, keeps its place in the stream;
/// - a `balance ... with pad` is checked after the other balance entries of its time, its
///   padding among them, while a plain balance after the pad is checked where it stands.
///
/// `#entries` itself keeps beancount's order of a day: a balance first, a document last.
#[test]
fn seq_is_the_order_zhang_processes_the_ledger_in() {
    let ledger = common::load_text(
        r#"
option "operating_currency" "CNY"
1970-01-01 commodity CNY
1970-01-01 open Assets:Cash
1970-01-01 open Assets:Cash:Sub
1970-01-01 open Equity:Open
1970-01-01 open Expenses:Food

2024-01-01 * "Self" "opening"
  Assets:Cash 100 CNY
  Equity:Open

2024-01-05 * "Shop" "lunch"
  Assets:Cash -10 CNY
  Expenses:Food

2024-01-05 15:30:00 balance Assets:Cash 90 CNY

2024-01-05 document Assets:Cash "a.pdf"

2024-01-06 balance Assets:Cash 90 CNY
2024-01-06 balance Assets:Cash:Sub 5 CNY with pad Equity:Open
2024-01-06 balance Assets:Cash 95 CNY

2024-01-06 * "Shop" "dinner"
  Assets:Cash -5 CNY
  Expenses:Food
"#,
    );
    assert_eq!(
        run(
            &ledger,
            "SELECT seq, type, time, narration, accounts FROM #entries WHERE date >= 2024-01-05 ORDER BY seq"
        ),
        rows(&[
            &["6", "transaction", "00:00:00", "lunch", "Assets:Cash, Expenses:Food"],
            &["7", "document", "00:00:00", "NULL", "Assets:Cash"],
            &["8", "balance", "15:30:00", "NULL", "Assets:Cash"],
            &["9", "balance", "00:00:00", "NULL", "Assets:Cash"],
            &[
                "10",
                "transaction",
                "00:00:00",
                "pad Assets:Cash:Sub to Equity:Open",
                "Assets:Cash:Sub, Equity:Open"
            ],
            &["11", "balance", "00:00:00", "NULL", "Assets:Cash"],
            &["12", "balance", "00:00:00", "NULL", "Assets:Cash:Sub, Equity:Open"],
            &["13", "transaction", "00:00:00", "dinner", "Assets:Cash, Expenses:Food"],
        ])
    );
    // beancount's order of the rows
    assert_eq!(
        run(&ledger, "SELECT seq, type FROM #entries WHERE date >= 2024-01-05"),
        rows(&[
            &["8", "balance"],
            &["6", "transaction"],
            &["7", "document"],
            &["9", "balance"],
            &["12", "balance"],
            &["11", "balance"],
            &["10", "transaction"],
            &["13", "transaction"],
        ])
    );
    // each assertion's balance is the running balance of the postings with a lower seq
    assert_eq!(
        run(&ledger, "SELECT seq, account, actual FROM #balances ORDER BY seq"),
        rows(&[
            &["8", "Assets:Cash", "90 CNY"],
            &["9", "Assets:Cash", "90 CNY"],
            &["11", "Assets:Cash", "95 CNY"],
            &["12", "Assets:Cash:Sub", "5 CNY"],
        ])
    );
    assert_eq!(
        run(&ledger, "SELECT seq, balance WHERE under(account, 'Assets:Cash')"),
        rows(&[&["5", "100 CNY"], &["6", "90 CNY"], &["10", "95 CNY"], &["13", "90 CNY"]])
    );
    // the documents of the stream have its order too
    assert_eq!(run(&ledger, "SELECT seq FROM #documents"), rows(&[&["7"]]));
}

/// More cases of the processing order, each assertion right after the postings its balance includes:
/// - a plain balance written after a pad of its time comes after the padding (it includes it), though
///   `#entries` lists every balance of a day before its transactions;
/// - a `balance ... with pad` of an account from itself books a padding that nets to zero, before
///   which a plain balance written after it is checked;
/// - two paddings of sub-accounts that net to zero come before the parent's check written after them;
/// - a time skipped by daylight saving (02:30 in New York, stored as 03:30) is processed at the time
///   written, before a balance at 03:15, while its timestamp is after that balance's.
#[test]
fn seq_puts_every_assertion_after_the_postings_it_includes() {
    let ledger = common::load_text(
        r#"
option "operating_currency" "CNY"
option "timezone" "America/New_York"
1970-01-01 commodity CNY
1970-01-01 open Assets:Bank
1970-01-01 open Assets:Bank:A
1970-01-01 open Assets:Bank:B
1970-01-01 open Assets:Cash
1970-01-01 open Equity:Open
1970-01-01 open Expenses:Food

2024-01-01 * "Self" "opening"
  Assets:Bank 100 CNY
  Equity:Open

2024-01-02 balance Assets:Bank 120 CNY with pad Assets:Bank
2024-01-02 balance Assets:Bank 100 CNY

2024-01-03 balance Assets:Bank:A 30 CNY with pad Equity:Open
2024-01-03 balance Assets:Bank:B -30 CNY with pad Equity:Open
2024-01-03 balance Assets:Bank 100 CNY
2024-01-03 balance Assets:Cash 0 CNY

2024-03-10 02:30:00 * "Shop" "in the gap"
  Assets:Bank -5 CNY
  Expenses:Food

2024-03-10 03:15:00 balance Assets:Bank 95 CNY
"#,
    );
    // the postings of Assets:Bank and its sub-accounts and the assertions on it, merged by seq
    let mut merged = run(&ledger, "SELECT seq, account, position, balance WHERE under(account, 'Assets:Bank')")
        .into_iter()
        .map(|row| (row[0].parse::<i64>().unwrap(), format!("{} {} -> {}", row[1], row[2], row[3])))
        .chain(
            run(
                &ledger,
                "SELECT seq, account, amount, actual, passed FROM #balances WHERE under(account, 'Assets:Bank')",
            )
            .into_iter()
            .map(|row| (row[0].parse::<i64>().unwrap(), format!("check {} {}: {} {}", row[1], row[2], row[3], row[4]))),
        )
        .collect::<Vec<_>>();
    merged.sort_by_key(|(seq, _)| *seq);
    assert_eq!(
        merged.into_iter().map(|(_, row)| row).collect::<Vec<_>>(),
        [
            "Assets:Bank 100 CNY -> 100 CNY",
            // the padding of Assets:Bank from itself, then the plain balance written after the pad, then the pad's check
            "Assets:Bank 20 CNY -> 120 CNY",
            "Assets:Bank -20 CNY -> 100 CNY",
            "check Assets:Bank 100 CNY: 100 CNY TRUE",
            "check Assets:Bank 120 CNY: 100 CNY FALSE",
            // the paddings of the sub-accounts net to zero before the parent's check
            "Assets:Bank:A 30 CNY -> 130 CNY",
            "Assets:Bank:B -30 CNY -> 100 CNY",
            "check Assets:Bank 100 CNY: 100 CNY TRUE",
            "check Assets:Bank:A 30 CNY: 30 CNY TRUE",
            "check Assets:Bank:B -30 CNY: -30 CNY TRUE",
            // the gap's transaction before the balance at 03:15, which includes it
            "Assets:Bank -5 CNY -> 95 CNY",
            "check Assets:Bank 95 CNY: 95 CNY TRUE",
        ]
    );
    assert_eq!(
        run(&ledger, "SELECT time, timestamp FROM #entries WHERE date = 2024-03-10 ORDER BY seq"),
        rows(&[&["03:30:00", "1710055800"], &["03:15:00", "1710054900"]])
    );
    // the balance of another account written after the pads comes after their paddings too
    let cash = run(&ledger, "SELECT seq FROM #balances WHERE account = 'Assets:Cash'")[0][0]
        .parse::<i64>()
        .unwrap();
    let paddings = run(&ledger, "SELECT max(seq) FROM #transactions WHERE flag = 'P' AND date = 2024-01-03")[0][0]
        .parse::<i64>()
        .unwrap();
    assert!(paddings < cash, "{paddings} {cash}");
}

/// An entry without a number of its own comes right after the assertion before it, also after a
/// `balance ... with pad`, whose number (where zhang checks it) is higher than that of its padding.
#[test]
fn an_entry_after_an_assertion_comes_after_its_check() {
    let ledger = common::load_text(
        r#"
option "operating_currency" "CNY"
1970-01-01 commodity CNY
1970-01-01 open Assets:A
1970-01-01 open Equity:Open
2024-01-01 * "Self" "t1"
  Assets:A 1 CNY
  Equity:Open
2024-01-02 balance Assets:A 1 CNY
2024-01-02 document Assets:A "after-balance.pdf"
2024-01-03 balance Assets:A 5 CNY with pad Equity:Open
2024-01-03 document Assets:A "after-pad.pdf"
"#,
    );
    assert_eq!(
        run(&ledger, "SELECT seq, type, date, flag FROM #entries WHERE year = 2024 ORDER BY seq"),
        rows(&[
            &["3", "transaction", "2024-01-01", "*"],
            &["4", "balance", "2024-01-02", "NULL"],
            &["5", "document", "2024-01-02", "NULL"],
            &["6", "transaction", "2024-01-03", "P"],
            &["7", "balance", "2024-01-03", "NULL"],
            &["8", "document", "2024-01-03", "NULL"],
        ])
    );
}

/// A ledger with a balance assertion whose directive a plugin copied a week later, as a plugin that
/// repeats an assertion does: the two share the position of the directive written.
fn a_balance_and_its_copy() -> Ledger {
    common::load_transformed(
        r#"
option "operating_currency" "CNY"
1970-01-01 commodity CNY
1970-01-01 open Assets:A
1970-01-01 open Equity:Open
2024-01-01 * "Self" "t1"
  Assets:A 1 CNY
  Equity:Open
2024-01-02 balance Assets:A 1 CNY
2024-01-03 * "Self" "t3"
  Assets:A 1 CNY
  Equity:Open
2024-01-04 * "Self" "t4"
  Assets:A 1 CNY
  Equity:Open
"#,
        |mut directives| {
            let copy = directives
                .iter()
                .find_map(|directive| match &directive.data {
                    Directive::BalanceCheck(check) => {
                        let mut check = check.clone();
                        check.date = Date::Date(NaiveDate::from_ymd_opt(2024, 1, 9).unwrap());
                        check.amount.number = 3.into();
                        Some(Spanned::new(Directive::BalanceCheck(check), directive.span.clone()))
                    }
                    _ => None,
                })
                .unwrap();
            directives.push(copy);
            directives
        },
    )
}

/// Assertions that share a position each have their own check: their place in the processing order,
/// their balance and whether they held, and the id zhang stored the check with.
#[test]
fn assertions_sharing_a_position_have_their_own_checks() {
    let ledger = a_balance_and_its_copy();
    assert_eq!(
        run(&ledger, "SELECT seq, type, date, narration FROM #entries WHERE year = 2024 ORDER BY seq"),
        rows(&[
            &["3", "transaction", "2024-01-01", "t1"],
            &["4", "balance", "2024-01-02", "NULL"],
            &["5", "transaction", "2024-01-03", "t3"],
            &["6", "transaction", "2024-01-04", "t4"],
            &["7", "balance", "2024-01-09", "NULL"],
        ])
    );
    assert_eq!(
        run(&ledger, "SELECT seq, date, amount, actual, passed FROM #balances ORDER BY seq"),
        rows(&[&["4", "2024-01-02", "1 CNY", "1 CNY", "TRUE"], &["7", "2024-01-09", "3 CNY", "3 CNY", "TRUE"]])
    );
}

/// A balance assertion has the id zhang stored its check with, the id `/api/journals` lists it with,
/// in `#balances` and in `#entries`.
#[test]
fn balances_have_the_id_of_their_check() {
    let ledger = common::load_text(PARENT_ACCOUNTS);
    let ids = run(&ledger, "SELECT id FROM #balances ORDER BY seq")
        .into_iter()
        .map(|row| row[0].clone())
        .collect::<Vec<_>>();
    let mut stored = ledger
        .store
        .read()
        .unwrap()
        .balance_assertions
        .iter()
        .map(|it| (it.sequence, it.id.to_string()))
        .collect::<Vec<_>>();
    stored.sort();
    assert_eq!(ids, stored.into_iter().map(|(_, id)| id).collect::<Vec<_>>());
    assert_eq!(
        run(&ledger, "SELECT id FROM #entries WHERE type = 'balance' ORDER BY seq"),
        run(&ledger, "SELECT id FROM #balances ORDER BY seq")
    );
}

/// A transaction whose directive a plugin copied with its position: zhang stores each of the two under
/// its own id, as it stores the paddings of a `pad` that share its position, and `#entries`,
/// `#transactions` and the postings list each once, with the id, date and `seq` it was stored with.
#[test]
fn transactions_sharing_a_position_are_each_the_one_zhang_stored() {
    let ledger = common::load_transformed(
        r#"
option "operating_currency" "CNY"
1970-01-01 commodity CNY
1970-01-01 open Assets:A
1970-01-01 open Equity:Open
2024-01-01 * "Self" "t1"
  Assets:A 1 CNY
  Equity:Open
2024-01-03 * "Self" "t3"
  Assets:A 1 CNY
  Equity:Open
"#,
        |mut directives| {
            let copy = directives
                .iter()
                .find_map(|directive| match &directive.data {
                    Directive::Transaction(txn) => {
                        let mut txn = txn.clone();
                        txn.date = Date::Date(NaiveDate::from_ymd_opt(2024, 1, 9).unwrap());
                        Some(Spanned::new(Directive::Transaction(txn), directive.span.clone()))
                    }
                    _ => None,
                })
                .unwrap();
            directives.push(copy);
            directives
        },
    );
    let stored = ledger.store.read().unwrap().transactions.len();
    assert_eq!(stored, 3);
    let entries = run(&ledger, "SELECT seq, date, narration, id FROM #entries WHERE type = 'transaction' ORDER BY seq");
    assert_eq!(
        entries.iter().map(|row| row[..3].to_vec()).collect::<Vec<_>>(),
        rows(&[&["3", "2024-01-01", "t1"], &["4", "2024-01-03", "t3"], &["5", "2024-01-09", "t1"]])
    );
    assert_ne!(entries[0][3], entries[2][3], "the copy has an id of its own");
    assert_eq!(run(&ledger, "SELECT seq, date, narration, id FROM #transactions ORDER BY seq"), entries);
    assert_eq!(
        run(&ledger, "SELECT DISTINCT seq, date, id FROM #postings"),
        entries
            .iter()
            .map(|row| vec![row[0].clone(), row[1].clone(), row[3].clone()])
            .collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------------------
// #documents: directives, then transaction and posting metadata

const DOCUMENTS_MAIN: &str = r#"
option "operating_currency" "CNY"
include "sub/more.zhang"

1970-01-01 commodity CNY
1970-01-01 open Assets:Bank
1970-01-01 open Expenses:Food

2024-01-02 document Assets:Bank "statements/jan.pdf" #monthly ^stmt-1
  source: "bank"

2024-01-03 * "Shop" "documents of the transaction and of a posting" #food ^r1
  document: "receipts/a.pdf"
  document: "receipts/b.pdf"
  Expenses:Food 10 CNY
    document: "receipts/c.pdf"
    note: "lunch"
  Assets:Bank

2024-01-04 * "Shop" "rejected: two implicit postings"
  document: "receipts/rejected.pdf"
  Expenses:Food
  Assets:Bank

2024-01-05 * "Shop" "an absolute path inside the ledger"
  document: "{root}/receipts/abs.pdf"
  Expenses:Food 5 CNY
  Assets:Bank
"#;

const DOCUMENTS_MORE: &str = r#"
2024-01-01 document Assets:Bank "w.pdf"

2024-01-06 * "Cafe" "in a file of a sub-directory"
  Expenses:Food 3 CNY
    document: "receipts/d.pdf"
  Assets:Bank
"#;

#[test]
fn documents_are_the_directives_then_the_metadata_of_transactions() {
    let (ledger, root) = load_files(&[("main.zhang", DOCUMENTS_MAIN), ("sub/more.zhang", DOCUMENTS_MORE)]);
    let shop = run(&ledger, "SELECT DISTINCT id FROM #postings WHERE narration ~ 'documents of'")[0][0].clone();
    let absolute = run(&ledger, "SELECT DISTINCT id FROM #postings WHERE narration ~ 'absolute'")[0][0].clone();
    let cafe = run(&ledger, "SELECT DISTINCT id FROM #postings WHERE payee = 'Cafe'")[0][0].clone();
    assert_eq!(
        run(&ledger, "SELECT date, account, source, path, transaction_id, tags, links, meta FROM #documents"),
        vec![
            // the directives first, in ledger order; `path` is as written: zhang resolves it
            // against the ledger's directory, not against the declaring file
            vec!["2024-01-01", "Assets:Bank", "directive", "w.pdf", "NULL", "", "", ""],
            vec![
                "2024-01-02",
                "Assets:Bank",
                "directive",
                "statements/jan.pdf",
                "NULL",
                "monthly",
                "stmt-1",
                "source: \"bank\""
            ],
            // then the metadata, in ledger order: a transaction's own (both values of the
            // repeated key, with its tags and links and its metadata), then its postings'
            vec![
                "2024-01-03",
                "NULL",
                "transaction",
                "receipts/a.pdf",
                &shop,
                "food",
                "r1",
                "document: \"receipts/a.pdf\", document: \"receipts/b.pdf\""
            ],
            vec![
                "2024-01-03",
                "NULL",
                "transaction",
                "receipts/b.pdf",
                &shop,
                "food",
                "r1",
                "document: \"receipts/a.pdf\", document: \"receipts/b.pdf\""
            ],
            vec![
                "2024-01-03",
                "Expenses:Food",
                "posting",
                "receipts/c.pdf",
                &shop,
                "food",
                "r1",
                "document: \"receipts/c.pdf\", note: \"lunch\""
            ],
            // the rejected transaction is not in the store, nor its document; an absolute path
            // inside the ledger's directory is made relative to it
            vec![
                "2024-01-05",
                "NULL",
                "transaction",
                "receipts/abs.pdf",
                &absolute,
                "",
                "",
                &format!("document: \"{}/receipts/abs.pdf\"", root.display())
            ],
            vec![
                "2024-01-06",
                "Expenses:Food",
                "posting",
                "receipts/d.pdf",
                &cafe,
                "",
                "",
                "document: \"receipts/d.pdf\""
            ],
        ]
    );
    // `filename` resolves a relative path against the declaring file, as beancount does
    let filenames = run(&ledger, "SELECT filename FROM #documents");
    let expected = [
        "sub/w.pdf",
        "statements/jan.pdf",
        "receipts/a.pdf",
        "receipts/b.pdf",
        "receipts/c.pdf",
        "receipts/abs.pdf",
        "sub/receipts/d.pdf",
    ];
    assert_eq!(filenames.len(), expected.len());
    for (filename, expected) in filenames.iter().zip(expected) {
        assert!(Path::new(&filename[0]).ends_with(expected), "{} {}", filename[0], expected);
    }
    // SELECT * keeps beanquery's columns
    assert_eq!(
        run(&ledger, "SELECT * FROM #documents WHERE source = 'posting'")
            .into_iter()
            .map(|row| row.len())
            .collect::<Vec<_>>(),
        [5, 5]
    );
    // the documents of an account, and of a transaction
    assert_eq!(
        run(&ledger, "SELECT path FROM #documents WHERE account = 'Expenses:Food' ORDER BY path"),
        rows(&[&["receipts/c.pdf"], &["receipts/d.pdf"]])
    );
    assert_eq!(
        run(&ledger, &format!("SELECT count(*) FROM #documents WHERE transaction_id = '{}'", shop)),
        rows(&[&["3"]])
    );
    // meta() reads the metadata of the directive, transaction or posting that holds it
    assert_eq!(
        run(
            &ledger,
            "SELECT path, meta('note'), meta('source') FROM #documents WHERE meta('note') IS NOT NULL OR meta('source') IS NOT NULL"
        ),
        rows(&[&["statements/jan.pdf", "NULL", "bank"], &["receipts/c.pdf", "lunch", "NULL"]])
    );
}

/// A document has the place of what declares it among the entries: a directive its own `#entries` row, a
/// document named in metadata that of its transaction.
#[test]
fn documents_have_the_place_of_what_declares_them() {
    let (ledger, _) = load_files(&[("main.zhang", DOCUMENTS_MAIN), ("sub/more.zhang", DOCUMENTS_MORE)]);
    let directives = run(&ledger, "SELECT seq, date, time, timestamp FROM #entries WHERE type = 'document'");
    assert_eq!(
        run(&ledger, "SELECT seq, date, time, timestamp FROM #documents WHERE source = 'directive'"),
        directives
    );
    let shop = run(
        &ledger,
        "SELECT DISTINCT seq, date, time, timestamp FROM #postings WHERE narration ~ 'documents of'",
    );
    assert_eq!(
        run(
            &ledger,
            "SELECT DISTINCT seq, date, time, timestamp FROM #documents WHERE path IN ('receipts/a.pdf', 'receipts/b.pdf', 'receipts/c.pdf')"
        ),
        shop
    );
    assert_eq!(run(&ledger, "SELECT time FROM #documents LIMIT 1"), rows(&[&["00:00:00"]]));
}

/// A ledger without document metadata has the rows of its directives only, as before.
#[test]
fn documents_without_metadata_are_the_directives() {
    let ledger = common::load_text(
        r#"
1970-01-01 open Assets:Bank
2024-01-02 document Assets:Bank "statements/jan.pdf"
2024-01-03 * "Shop"
  Assets:Bank -1 CNY
  Assets:Bank 1 CNY
"#,
    );
    assert_eq!(
        run(&ledger, "SELECT date, account, source, path, transaction_id FROM #documents"),
        rows(&[&["2024-01-02", "Assets:Bank", "directive", "statements/jan.pdf", "NULL"]])
    );
}

// ---------------------------------------------------------------------------------------
// #errors: id, span_start and span_end

#[test]
fn errors_identify_their_directive() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../integration-tests/query-zhang-tables");
    let ledger = common::load_ledger(dir.clone(), "main.zhang");
    let errors = run(&ledger, "SELECT file, id, span_start, span_end, source, meta('txn_id') FROM #errors");
    assert_eq!(errors.len(), 6);
    for error in &errors {
        let [file, id, start, end, source, txn_id] = &error[..] else { unreachable!() };
        // the span is where the directive is in its file
        let text = std::fs::read_to_string(dir.join(file)).unwrap();
        let (start, end) = (start.parse::<usize>().unwrap(), end.parse::<usize>().unwrap());
        assert_eq!(text[start..end].trim_end(), source, "{:?}", error);
        // the id of an error in a transaction is the transaction's
        assert_eq!(id.len(), 36, "{:?}", error);
        if txn_id != "NULL" {
            assert_eq!(id, txn_id, "{:?}", error);
        }
    }
    // the id of an error in a transaction is the `id` of its postings
    let unbalanced = run(&ledger, "SELECT id FROM #errors WHERE kind = 'UnbalancedTransaction'");
    assert_eq!(run(&ledger, "SELECT DISTINCT id FROM #postings WHERE narration = 'unbalanced'"), unbalanced);
}

const TWO_ERRORS: &str = r#"
1970-01-01 commodity CNY
1970-01-01 open Assets:Bank
2024-01-02 * "Shop" "an unknown account and an unbalanced transaction"
  Expenses:Unknown 10 CNY
  Assets:Bank -9 CNY
"#;

#[test]
fn errors_of_one_directive_share_its_id_and_span() {
    let ledger = common::load_text(TWO_ERRORS);
    let start = TWO_ERRORS.find("2024-01-02").unwrap();
    let end = TWO_ERRORS.trim_end().len();
    let span = |kind: &str| vec![kind.to_owned(), start.to_string(), end.to_string()];
    assert_eq!(
        run(&ledger, "SELECT kind, span_start, span_end FROM #errors ORDER BY kind"),
        vec![span("AccountDoesNotExist"), span("UnbalancedTransaction")]
    );
    let ids = run(&ledger, "SELECT DISTINCT id FROM #errors");
    assert_eq!(ids, run(&ledger, "SELECT DISTINCT id FROM #postings"));
    assert_eq!(ids.len(), 1);
}

// ---------------------------------------------------------------------------------------
// #budgets and #budget_events

/// Budgets in CNY spent in four currencies, a budget-add in USD, a transfer, a close, and
/// directives the ledger rejects because their budget does not exist yet.
const BUDGETS: &str = r#"
option "operating_currency" "CNY"
option "timezone" "Asia/Shanghai"

1970-01-01 commodity CNY
1970-01-01 commodity USD
1970-01-01 commodity JPY
1970-01-01 commodity EUR
1970-01-01 commodity AAPL

1970-01-01 open Assets:Bank
1970-01-01 open Assets:USBank
1970-01-01 open Assets:Savings
  budget: invest
1970-01-01 open Expenses:Food
  budget: food
1970-01-01 open Expenses:Travel
  budget: travel
1970-01-01 open Income:Cashback
  budget: food

2024-01-31 price USD 7 CNY
2024-02-01 price CNY 20 JPY
2024-02-01 price AAPL 150 USD
2024-02-20 price USD 7.2 CNY

2024-01-15 * "Market" "spent before the budget exists"
  Expenses:Food 7 CNY
  Assets:Bank

2024-01-20 budget-add food 100 CNY

2024-02-01 budget food CNY
  alias: "Food"
2024-02-01 budget travel CNY
2024-02-01 budget invest CNY
2024-02-01 budget-add food 1000 CNY
2024-02-01 10:30:00 budget-add travel 50 USD

2024-02-05 * "Market" "groceries"
  Expenses:Food 100 CNY
  Assets:Bank

2024-02-10 * "Sushi" "yen, priced only the other way"
  Expenses:Food 2000 JPY @@ 100 CNY
  Assets:Bank

2024-02-21 * "Airline" "dollars at the price of their date"
  Expenses:Travel 100 USD
  Assets:USBank

2024-02-22 * "Museum" "euros without a price"
  Expenses:Travel 10 EUR @ 1.1 USD
  Assets:USBank

2024-02-25 * "Broker" "a lot valued through its cost currency"
  Assets:Savings 2 AAPL {140 USD}
  Assets:USBank

2024-03-03 * "Market" "refund"
  Expenses:Food -30 CNY
  Assets:Bank

2024-03-04 * "Card" "cashback"
  Assets:Bank 5 CNY
  Income:Cashback

2024-03-05 budget-transfer food travel 200 CNY
2024-03-31 budget-close travel

2024-04-02 * "Market" "April"
  Expenses:Food 10 CNY
  Assets:Bank
"#;

const BUDGET_FIGURES: &str = "SELECT name, date, assigned, added, activity, available, closed FROM #budgets";

fn budget_figures() -> Vec<Vec<String>> {
    rows(&[
        // the budget exists from February on: the spending of January and the budget-add of
        // the 20th are not its own (zhang reports both). February spends 100 CNY plus
        // 2000 JPY, worth 2000 / 20 = 100.00 CNY with the inverse of the CNY price in JPY
        &["food", "2024-02-01", "1000 CNY", "1000 CNY", "200.00 CNY", "800.00 CNY", "FALSE"],
        // 200 transferred out; the refund (-30) and the cashback on the Income account,
        // counted negated as zhang does (-(-5) = 5), give -25
        &["food", "2024-03-01", "600.00 CNY", "-200 CNY", "-25 CNY", "625.00 CNY", "FALSE"],
        &["food", "2024-04-01", "625.00 CNY", "0 CNY", "10 CNY", "615.00 CNY", "FALSE"],
        // 2 AAPL have no CNY price: they are valued through their cost currency at the date,
        // 2 x 150 USD x 7.2 = 2160.0 CNY
        &["invest", "2024-02-01", "0 CNY", "0 CNY", "2160.0 CNY", "-2160.0 CNY", "FALSE"],
        &["invest", "2024-03-01", "-2160.0 CNY", "0 CNY", "0 CNY", "-2160.0 CNY", "FALSE"],
        &["invest", "2024-04-01", "-2160.0 CNY", "0 CNY", "0 CNY", "-2160.0 CNY", "FALSE"],
        // 50 USD added at 7 CNY (the price as of February 1st) is 350 CNY; 100 USD spent on
        // the 21st at 7.2 is 720.0 CNY; the 10 EUR have no price and are left out
        &["travel", "2024-02-01", "350 CNY", "350 CNY", "720.0 CNY", "-370.0 CNY", "FALSE"],
        // closed from the month of its budget-close on, not before
        &["travel", "2024-03-01", "-170.0 CNY", "200 CNY", "0 CNY", "-170.0 CNY", "TRUE"],
        &["travel", "2024-04-01", "-170.0 CNY", "0 CNY", "0 CNY", "-170.0 CNY", "TRUE"],
    ])
}

#[test]
fn budgets_convert_spending_and_additions_to_the_budget_commodity() {
    let ledger = common::load_text(BUDGETS);
    assert_eq!(run(&ledger, BUDGET_FIGURES), budget_figures());
    // the activity is what the postings table gives with convert() at each posting's date
    assert_eq!(
        run(
            &ledger,
            "SELECT year, month, sum(convert(position, 'CNY', date)) FROM #postings \
             WHERE account = 'Expenses:Travel' GROUP BY year, month"
        ),
        rows(&[&["2024", "2", "720.0 CNY, 10 EUR"]])
    );
    // a "current month" view: the last month of every budget
    assert_eq!(
        run(
            &ledger,
            "SELECT name, last(alias), last(available), last(closed) FROM #budgets GROUP BY name ORDER BY name"
        ),
        rows(&[
            &["food", "Food", "615.00 CNY", "FALSE"],
            &["invest", "NULL", "-2160.0 CNY", "FALSE"],
            &["travel", "NULL", "-170.0 CNY", "TRUE"],
        ])
    );
}

/// The table reads the directives and the booked postings: it gives the same rows without
/// the budgets zhang keeps in its store.
#[test]
fn budgets_do_not_read_the_store_budgets() {
    let ledger = common::load_text(BUDGETS);
    let all = "SELECT name, alias, category, currency, date, year, month, assigned, added, activity, available, accounts, closed FROM #budgets";
    let before = run(&ledger, all);
    assert_eq!(before.len(), 9);
    ledger.store.write().unwrap().budgets.clear();
    assert_eq!(run(&ledger, all), before);
    assert_eq!(run(&ledger, BUDGET_FIGURES), budget_figures());
}

/// The price of USD changes in the middle of March: from 7 CNY (set in February) to 8 CNY on
/// the 10th.
const MID_MONTH_PRICE: &str = r#"
option "operating_currency" "CNY"

1970-01-01 commodity CNY
1970-01-01 commodity USD
1970-01-01 open Assets:Bank
1970-01-01 open Expenses:Travel
  budget: travel

2024-02-25 price USD 7 CNY
2024-03-10 price USD 8 CNY

2024-03-01 budget travel CNY

2024-03-05 * "Taxi" "before the price change"
  Expenses:Travel 10 USD
  Assets:Bank

2024-03-20 budget-add travel 50 USD

2024-03-25 * "Train" "after the price change"
  Expenses:Travel 10 USD
  Assets:Bank
"#;

/// An amount converts at the price in force on its own date, not on the first day of its
/// month (7 CNY) nor at the latest price (8 CNY).
#[test]
fn budget_amounts_convert_at_their_own_date_within_the_month() {
    let ledger = common::load_text(MID_MONTH_PRICE);
    assert_eq!(
        run(&ledger, BUDGET_FIGURES),
        rows(&[
            // the budget-add of the 20th: 50 USD at 8 CNY = 400 CNY (350 at the price of the 1st).
            // The taxi of the 5th, before the change, at 7 CNY = 70 CNY, and the train of the
            // 25th at 8 CNY = 80 CNY: 150 CNY (140 at the price of the 1st, 160 at the latest)
            &["travel", "2024-03-01", "400 CNY", "400 CNY", "150 CNY", "250 CNY", "FALSE"],
        ])
    );
}

const DEFINED_MID_MONTH: &str = r#"
option "operating_currency" "CNY"

1970-01-01 commodity CNY
1970-01-01 open Assets:Bank
1970-01-01 open Expenses:Food
  budget: food

2024-01-10 * "Market" "before the budget"
  Expenses:Food 30 CNY
  Assets:Bank

2024-01-15 budget food CNY
2024-01-15 budget-add food 100 CNY

2024-01-20 * "Market" "after the budget"
  Expenses:Food 5 CNY
  Assets:Bank
"#;

/// A budget's activity starts at its definition, as zhang folds the ledger: the 30 CNY spent on
/// the 10th, before the budget of the 15th exists, are no activity of it, even in the same
/// month; the 5 CNY of the 20th are (35 CNY if both counted).
#[test]
fn budget_activity_starts_at_the_definition_of_the_budget() {
    let ledger = common::load_text(DEFINED_MID_MONTH);
    assert_eq!(
        run(&ledger, BUDGET_FIGURES),
        rows(&[&["food", "2024-01-01", "100 CNY", "100 CNY", "5 CNY", "95 CNY", "FALSE"]])
    );
}

#[test]
fn budget_events_are_the_effects_of_the_budget_directives() {
    let ledger = common::load_text(BUDGETS);
    assert_eq!(
        run(&ledger, "SELECT * FROM #budget_events"),
        rows(&[
            // the budget-add of January 20th has no effect: food does not exist yet.
            // 2024-02-01 00:00 in Asia/Shanghai is 2024-01-31T16:00:00Z
            &["food", "2024-02-01", "00:00:00", "1706716800", "assign", "1000 CNY"],
            // the amount as written, in USD; 10:30 is 37800 seconds later
            &["travel", "2024-02-01", "10:30:00", "1706754600", "assign", "50 USD"],
            // a transfer takes from one budget, then gives to the other
            &["food", "2024-03-05", "00:00:00", "1709568000", "transfer_out", "-200 CNY"],
            &["travel", "2024-03-05", "00:00:00", "1709568000", "transfer_in", "200 CNY"],
            &["travel", "2024-03-31", "00:00:00", "1711814400", "close", "NULL"],
        ])
    );
    // what each budget was given, as written
    assert_eq!(
        run(
            &ledger,
            "SELECT name, sum(amount) FROM #budget_events WHERE type != 'close' GROUP BY name ORDER BY name"
        ),
        rows(&[&["food", "800 CNY"], &["travel", "200 CNY, 50 USD"]])
    );
    // meta() reads the metadata of the directive
    let ledger = common::load_text("2024-01-01 budget food CNY\n2024-01-02 budget-add food 10 CNY\n  memo: \"start\"\n2024-01-03 budget-close food\n");
    assert_eq!(
        run(&ledger, "SELECT type, amount, meta('memo') FROM #budget_events"),
        rows(&[&["assign", "10 CNY", "start"], &["close", "NULL", "NULL"]])
    );
}

/// The examples of the query language reference for these tables run, and give what they say.
#[test]
fn the_documented_examples_run() {
    let ledger = common::load_text(BUDGETS);
    assert_eq!(
        run(
            &ledger,
            "SELECT name, last(available) AS available, last(closed) AS closed FROM #budgets GROUP BY name ORDER BY name"
        ),
        rows(&[
            &["food", "615.00 CNY", "FALSE"],
            &["invest", "-2160.0 CNY", "FALSE"],
            &["travel", "-170.0 CNY", "TRUE"]
        ])
    );
    assert_eq!(
        run(
            &ledger,
            "SELECT name, sum(amount) AS added FROM #budget_events WHERE year(date) = 2024 AND type != 'close' GROUP BY name ORDER BY name"
        ),
        rows(&[&["food", "800 CNY"], &["travel", "200 CNY, 50 USD"]])
    );
    // `activity` is what convert() gives over the budget's postings
    assert_eq!(
        run(
            &ledger,
            "SELECT sum(convert(position, 'CNY', date)) FROM #postings WHERE account = 'Expenses:Food' AND year = 2024 AND month = 2"
        ),
        run(&ledger, "SELECT activity FROM #budgets WHERE name = 'food' AND date = 2024-02-01")
    );
    let (ledger, _root) = load_files(&[("main.zhang", DOCUMENTS_MAIN), ("sub/more.zhang", DOCUMENTS_MORE)]);
    assert_eq!(
        run(
            &ledger,
            "SELECT date, account, path FROM #documents WHERE source != 'directive' ORDER BY date DESC"
        )
        .into_iter()
        .map(|row| row[2].clone())
        .collect::<Vec<_>>(),
        ["receipts/d.pdf", "receipts/abs.pdf", "receipts/a.pdf", "receipts/b.pdf", "receipts/c.pdf"]
    );
}
