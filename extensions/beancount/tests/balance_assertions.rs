//! Balance assertions and pads, checked against Python beancount: for every ledger in
//! `balance_assertions/`, zhang ends with the same balances, books the same pads on the same
//! dates, finds the same assertions passing and failing against the same balances, and reports
//! the same pads unused or padding a commodity held at cost, as beancount 3.2.3 does (`balance_assertions/oracle.json`, written by
//! `balance_assertions/generate.py`).
//!
//! A balance assertion never moves a balance: the balances are the sums of the postings, and a
//! pad is sized from them. An assertion or a pad covers the account and all its sub-accounts. A
//! ledger the oracle marks with an `accepted_deviation` is checked against zhang's own rule instead:
//! zhang never infers a tolerance, and pads to exactly the asserted amount.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;

use beancount::Beancount;
use bigdecimal::{BigDecimal, Zero};
use serde_json::Value;
use zhang_ast::amount::Amount;
use zhang_ast::error::ErrorKind;
use zhang_ast::{Directive, Flag};
use zhang_core::data_source::LocalFileSystemDataSource;
use zhang_core::ledger::Ledger;

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/balance_assertions")
}

fn load(case: &str) -> Ledger {
    let data_source = Arc::new(LocalFileSystemDataSource::new(Beancount::default()));
    Ledger::load_with_data_source(dir(), format!("{case}.bean"), data_source).expect("the ledger loads")
}

fn oracle_cases() -> serde_json::Map<String, Value> {
    let oracle: Value = serde_json::from_str(&std::fs::read_to_string(dir().join("oracle.json")).unwrap()).unwrap();
    oracle.as_object().unwrap().clone()
}

fn number(value: &Value) -> BigDecimal {
    BigDecimal::from_str(value.as_str().expect("a number is a string")).unwrap().normalized()
}

/// `{"number": .., "currency": ..}` as `(number, currency)`, the number normalized
fn amount(value: &Value) -> (BigDecimal, String) {
    (number(&value["number"]), value["currency"].as_str().unwrap().to_owned())
}

fn of(amount: &Amount) -> (BigDecimal, String) {
    (amount.number.normalized(), amount.commodity.clone())
}

fn text(value: &Value) -> String {
    value.as_str().unwrap().to_owned()
}

/// account -> currency -> units, zeros left out
type Balances = BTreeMap<String, BTreeMap<String, BigDecimal>>;

/// (date, padded account, units, account padded from)
type Pad = (String, String, (BigDecimal, String), String);

/// (date, account, asserted amount, balance it was checked against, passed)
type Assertion = (String, String, (BigDecimal, String), (BigDecimal, String), bool);

/// (date, account) of a `pad` reported unused, or of a `balance` whose padding pads a commodity held at cost
type Located = (String, String);

#[derive(Debug, PartialEq)]
struct Outcome {
    balances: Balances,
    /// sorted: zhang books the paddings in the order of the assertions they serve, beancount in the order
    /// of the pads
    pads: Vec<Pad>,
    /// sorted: beancount orders the entries of a day by their line, whatever their file
    assertions: Vec<Assertion>,
    unused_pads: Vec<Located>,
    pads_with_cost: Vec<Located>,
}

fn oracle(case: &Value) -> Outcome {
    assert_eq!(case["errors"], Value::Array(vec![]), "the oracle ledgers have no other errors");
    let balances = case["balances"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(account, held)| {
            let held = held
                .as_object()
                .unwrap()
                .iter()
                .map(|(currency, units)| (currency.clone(), number(units)))
                .collect();
            (account.clone(), held)
        })
        .collect();
    let mut pads = case["pads"]
        .as_array()
        .unwrap()
        .iter()
        .map(|pad| (text(&pad["date"]), text(&pad["account"]), amount(&pad["units"]), text(&pad["from"])))
        .collect::<Vec<_>>();
    pads.sort();
    let mut assertions = case["assertions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|it| {
            (
                text(&it["date"]),
                text(&it["account"]),
                amount(&it["amount"]),
                amount(&it["balance"]),
                it["passed"].as_bool().unwrap(),
            )
        })
        .collect::<Vec<_>>();
    assertions.sort();
    let located = |key: &str| {
        let mut located = case[key]
            .as_array()
            .unwrap()
            .iter()
            .map(|it| (text(&it["date"]), text(&it["account"])))
            .collect::<Vec<_>>();
        located.sort();
        located
    };
    Outcome {
        balances,
        pads,
        assertions,
        unused_pads: located("unused_pads"),
        pads_with_cost: located("pads_with_cost"),
    }
}

fn zhang(case: &str) -> Outcome {
    let ledger = load(case);
    // the lots the query engine lists (`commodities.lots`), those with units left, by account and commodity
    let lots = zhang_query::execute(
        &ledger,
        "SELECT account, currency, sum(number) \
         GROUP BY account, currency, cost_number, cost_currency, cost_date, cost_label HAVING sum(number) != 0",
    )
    .unwrap();
    let mut lots_held = BTreeMap::<String, BTreeMap<String, BigDecimal>>::new();
    for lot in &lots.rows {
        *lots_held.entry(lot[0].to_string()).or_default().entry(lot[1].to_string()).or_default() += lot[2].as_decimal().unwrap();
    }
    let store = ledger.store.read().unwrap();

    // the sums of the postings, which is what the balances shown are
    let mut balances = Balances::new();
    for posting in &store.postings {
        let held = balances.entry(posting.account.name().to_owned()).or_default();
        let units = held.entry(posting.inferred_amount.commodity.clone()).or_insert_with(BigDecimal::zero);
        *units += &posting.inferred_amount.number;
    }
    for (account, held) in balances.iter_mut() {
        let booked = lots_held.get(account).cloned().unwrap_or_default();
        let shown = booked
            .into_iter()
            .map(|(currency, units)| (currency, units.normalized()))
            .filter(|(_, units)| !units.is_zero())
            .collect::<BTreeMap<_, _>>();
        held.retain(|_, units| !units.is_zero());
        for units in held.values_mut() {
            *units = units.normalized();
        }
        assert_eq!(&shown, held, "{case}: the balance shown for {account} is the sum of its postings");
    }
    balances.retain(|_, held| !held.is_empty());

    let mut pads = store
        .transactions
        .values()
        .filter(|txn| txn.flag == Flag::BalancePad)
        .map(|txn| {
            let [padded, from] = &txn.postings[..] else {
                panic!("{case}: a padding transaction has two postings");
            };
            (
                txn.datetime.date_naive().to_string(),
                padded.account.name().to_owned(),
                of(&padded.inferred_amount),
                from.account.name().to_owned(),
            )
        })
        .collect::<Vec<_>>();
    pads.sort();

    let mut assertions = store
        .balance_assertions
        .iter()
        .map(|it| {
            (
                it.datetime.date_naive().to_string(),
                it.account.name().to_owned(),
                of(&it.amount),
                of(&it.balance),
                it.passed,
            )
        })
        .collect::<Vec<_>>();
    assertions.sort();

    // a `pad` reported unused, by the directive at the error's span
    let mut unused_pads = store
        .errors
        .iter()
        .filter(|error| error.error_type == ErrorKind::UnusedPad)
        .map(|error| {
            let pad = ledger
                .directives
                .iter()
                .find_map(|directive| match &directive.data {
                    Directive::Pad(pad) if Some(&directive.span) == error.span.as_ref() => Some(pad),
                    _ => None,
                })
                .expect("an unused pad error is on its pad");
            (pad.date.naive_date().to_string(), pad.account.name().to_owned())
        })
        .collect::<Vec<_>>();
    unused_pads.sort();

    // the `balance` a pad of a commodity held at cost serves, by the directive at the error's span
    let mut pads_with_cost = store
        .errors
        .iter()
        .filter(|error| error.error_type == ErrorKind::PadWithCost)
        .map(|error| {
            let balance = ledger
                .directives
                .iter()
                .find_map(|directive| match &directive.data {
                    Directive::BalanceCheck(check) if Some(&directive.span) == error.span.as_ref() => Some(check),
                    _ => None,
                })
                .expect("a pad with cost error is on the balance it serves");
            (balance.date.naive_date().to_string(), balance.account.name().to_owned())
        })
        .collect::<Vec<_>>();
    pads_with_cost.sort();

    let other_errors = store
        .errors
        .iter()
        .filter(|error| {
            !matches!(
                error.error_type,
                ErrorKind::UnusedPad | ErrorKind::PadWithCost | ErrorKind::AccountBalanceCheckError
                    // a warning, which changes no figure
                    | ErrorKind::BalanceTimeIgnored
            )
        })
        .map(|error| error.error_type.clone())
        .collect::<Vec<_>>();
    assert!(other_errors.is_empty(), "{case}: {other_errors:?}");
    let failed = store
        .errors
        .iter()
        .filter(|error| error.error_type == ErrorKind::AccountBalanceCheckError)
        .count();
    assert_eq!(
        failed,
        assertions.iter().filter(|it| !it.4).count(),
        "{case}: every failing assertion is an error"
    );

    Outcome {
        balances,
        pads,
        assertions,
        unused_pads,
        pads_with_cost,
    }
}

/// the ledgers whose deviation from beancount the tests below check
const DEVIATIONS: &[&str] = &[
    "child_assertion_after_parent_pad",
    "inferred_tolerance",
    "nested_pads",
    "pad_with_cost_lots",
    "pad_within_tolerance",
    "same_day_pads_in_two_files",
];

#[test]
fn zhang_agrees_with_beancount_on_every_ledger_without_an_accepted_deviation() {
    let mut differing = vec![];
    for (case, expected) in oracle_cases() {
        if !expected["accepted_deviation"].is_null() {
            assert!(
                DEVIATIONS.contains(&case.as_str()),
                "{case}: an accepted deviation needs a test of zhang's own result"
            );
            continue;
        }
        let expected = oracle(&expected);
        let actual = zhang(&case);
        if actual != expected {
            differing.push(format!("{case}:\n  zhang:     {actual:?}\n  beancount: {expected:?}"));
        }
    }
    assert!(differing.is_empty(), "{}", differing.join("\n"));
}

fn cny(number: &str) -> (BigDecimal, String) {
    (BigDecimal::from_str(number).unwrap().normalized(), "CNY".to_owned())
}

fn assertion(date: &str, account: &str, asserted: &str, balance: &str, passed: bool) -> Assertion {
    (date.to_owned(), account.to_owned(), cny(asserted), cny(balance), passed)
}

/// the oracle of an accepted deviation, with the reason it records
fn deviation(case: &str) -> (Outcome, String) {
    let expected = &oracle_cases()[case];
    let reason = expected["accepted_deviation"]
        .as_str()
        .unwrap_or_else(|| panic!("{case} is an accepted deviation"))
        .to_owned();
    (oracle(expected), reason)
}

#[test]
fn zhang_infers_no_tolerance() {
    let (beancount, reason) = deviation("inferred_tolerance");
    assert!(reason.contains("never infers a tolerance"), "{reason}");
    let zhang = zhang("inferred_tolerance");

    // beancount accepts 50.004 for 50.00, and pads nothing for the 0.005 Assets:B lacks
    assert_eq!(
        beancount.assertions,
        vec![
            assertion("2024-01-02", "Assets:A", "50.00", "50.004", true),
            assertion("2024-01-03", "Assets:A", "50.004", "50.004", true),
            assertion("2024-01-03", "Assets:B", "100.00", "99.995", true),
        ]
    );
    assert_eq!(beancount.unused_pads, vec![("2024-01-02".to_owned(), "Assets:B".to_owned())]);
    assert!(beancount.pads.is_empty());

    // zhang checks the exact amount, and pads the 0.005 exactly
    assert_eq!(
        zhang.assertions,
        vec![
            assertion("2024-01-02", "Assets:A", "50.00", "50.004", false),
            assertion("2024-01-03", "Assets:A", "50.004", "50.004", true),
            assertion("2024-01-03", "Assets:B", "100.00", "100.00", true),
        ]
    );
    assert_eq!(
        zhang.pads,
        vec![("2024-01-02".to_owned(), "Assets:B".to_owned(), cny("0.005"), "Equity:Open".to_owned())]
    );
    assert!(zhang.unused_pads.is_empty());
}

#[test]
fn zhang_pads_exactly_within_an_explicit_tolerance() {
    let (beancount, reason) = deviation("pad_within_tolerance");
    assert!(reason.contains("exactly the asserted amount"), "{reason}");
    let zhang = zhang("pad_within_tolerance");

    assert_eq!(beancount.assertions, vec![assertion("2024-01-03", "Assets:A", "100.00", "99.98", true)]);
    assert!(beancount.pads.is_empty());
    assert_eq!(beancount.unused_pads, vec![("2024-01-02".to_owned(), "Assets:A".to_owned())]);

    assert_eq!(zhang.assertions, vec![assertion("2024-01-03", "Assets:A", "100.00", "100.00", true)]);
    assert_eq!(
        zhang.pads,
        vec![("2024-01-02".to_owned(), "Assets:A".to_owned(), cny("0.02"), "Equity:Open".to_owned())]
    );
    assert!(zhang.unused_pads.is_empty());
}

fn padding(date: &str, account: &str, units: &str) -> Pad {
    (date.to_owned(), account.to_owned(), cny(units), "Equity:Open".to_owned())
}

#[test]
fn a_pad_serves_the_assertions_on_its_own_account_only() {
    let (beancount, reason) = deviation("child_assertion_after_parent_pad");
    assert!(reason.contains("its own account only"), "{reason}");
    // the assertion on the sub-account uses up the pad of the parent in beancount
    assert_eq!(beancount.unused_pads, vec![("2024-01-03".to_owned(), "Assets:Bank".to_owned())]);
    assert!(beancount.pads.is_empty());
    assert_eq!(
        beancount.assertions,
        vec![
            assertion("2024-01-04", "Assets:Bank:Checking", "60", "60", true),
            assertion("2024-01-05", "Assets:Bank", "100", "60", false),
        ]
    );

    let zhang = zhang("child_assertion_after_parent_pad");
    assert_eq!(zhang.pads, vec![padding("2024-01-03", "Assets:Bank", "40")]);
    assert!(zhang.unused_pads.is_empty());
    assert_eq!(
        zhang.assertions,
        vec![
            assertion("2024-01-04", "Assets:Bank:Checking", "60", "60", true),
            assertion("2024-01-05", "Assets:Bank", "100", "100", true),
        ]
    );
}

#[test]
fn a_pad_counts_the_padding_of_the_sub_accounts() {
    let (beancount, reason) = deviation("nested_pads");
    assert!(reason.contains("every assertion served before it"), "{reason}");
    // beancount pads the parent by 60 where 40 are missing
    assert_eq!(
        beancount.pads,
        vec![padding("2024-01-02", "Assets:Bank", "60"), padding("2024-01-02", "Assets:Bank:Checking", "60")]
    );
    assert_eq!(
        beancount.assertions,
        vec![
            assertion("2024-01-03", "Assets:Bank", "100", "120", false),
            assertion("2024-01-03", "Assets:Bank:Checking", "60", "60", true),
        ]
    );

    let zhang = zhang("nested_pads");
    assert_eq!(
        zhang.pads,
        vec![padding("2024-01-02", "Assets:Bank", "40"), padding("2024-01-02", "Assets:Bank:Checking", "60")]
    );
    assert_eq!(
        zhang.assertions,
        vec![
            assertion("2024-01-03", "Assets:Bank", "100", "100", true),
            assertion("2024-01-03", "Assets:Bank:Checking", "60", "60", true),
        ]
    );
}

#[test]
fn zhang_orders_the_pads_of_a_day_by_file() {
    let (beancount, reason) = deviation("same_day_pads_in_two_files");
    assert!(reason.contains("by file"), "{reason}");
    let from = |account: &str| vec![("2024-01-02".to_owned(), "Assets:A".to_owned(), cny("100"), account.to_owned())];
    let unused = vec![("2024-01-02".to_owned(), "Assets:A".to_owned())];
    // the pad on the later line, in the file included first, wins in beancount
    assert_eq!(beancount.pads, from("Equity:X"));
    assert_eq!(beancount.unused_pads, unused);

    let zhang = zhang("same_day_pads_in_two_files");
    // the pad of the file included last wins in zhang
    assert_eq!(zhang.pads, from("Equity:Y"));
    assert_eq!(zhang.unused_pads, unused);
    assert_eq!(zhang.assertions, beancount.assertions);
    assert_eq!(zhang.balances["Assets:A"], beancount.balances["Assets:A"]);
}

#[test]
fn zhang_reports_a_pad_with_cost_once_for_its_balance() {
    let (beancount, reason) = deviation("pad_with_cost_lots");
    assert!(reason.contains("once for each lot"), "{reason}");
    let balance = ("2024-01-05".to_owned(), "Assets:Broker".to_owned());
    // one error for each of the two lots in beancount, one for the balance in zhang
    assert_eq!(beancount.pads_with_cost, vec![balance.clone(), balance.clone()]);
    let zhang = zhang("pad_with_cost_lots");
    assert_eq!(zhang.pads_with_cost, vec![balance]);
    // the rest agrees
    assert_eq!(zhang.pads, beancount.pads);
    assert_eq!(zhang.assertions, beancount.assertions);
    assert_eq!(zhang.balances, beancount.balances);
    assert_eq!(zhang.unused_pads, beancount.unused_pads);
}

#[test]
fn a_balance_whose_time_zhang_ignores_is_reported() {
    // the balance of 2024-03-02 says 20:00, after lunch that day: beancount checks it before lunch, and so does zhang
    let ledger = load("balance_time_after_transactions");
    let store = ledger.store.read().unwrap();
    let ignored = store
        .errors
        .iter()
        .filter(|it| it.error_type == ErrorKind::BalanceTimeIgnored)
        .map(|it| it.span.as_ref().unwrap().content.lines().next().unwrap().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(ignored, vec!["2024-03-02 balance Assets:A  100 CNY"]);
    // a balance timed before the transactions of its day, or of an account without any, changes nothing
    let ledger = load("balance_time_on_pad_day");
    assert!(ledger.store.read().unwrap().errors.is_empty());
}

#[test]
fn only_a_balance_whose_meaning_changed_is_reported_for_its_ignored_time() {
    let ledger = load("balance_times");
    let store = ledger.store.read().unwrap();
    let reported = store
        .errors
        .iter()
        .filter(|it| it.error_type == ErrorKind::BalanceTimeIgnored)
        .map(|it| it.span.as_ref().unwrap().content.lines().next().unwrap().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(
        reported,
        vec![
            // a transaction of a sub-account before the time
            "2024-03-02 balance Assets:A 1 CNY",
            // a padding transaction written as such
            "2024-03-05 balance Assets:C 7 CNY",
            // a transaction without a time, at the start of the day
            "2024-03-06 balance Assets:D 3 CNY",
            // a time with spaces around it, and an hour of one digit, which earlier versions read
            "2024-03-08 balance Assets:F 5 CNY",
            "2024-03-09 balance Assets:G 6 CNY",
            // two transactions before the time: once
            "2024-03-11 balance Assets:A 4 CNY",
        ]
    );
    // not reported: a transaction at the same time, of an account only named like it (Assets:AB), in another
    // commodity, a time without seconds, which earlier versions did not read, and transactions netting to zero
}

#[test]
fn zhang_reads_a_document_relative_to_its_file() {
    // beancount finds both (the oracle has no error): zhang names them by the same path within the ledger
    let ledger = load("document_paths");
    let store = ledger.store.read().unwrap();
    assert!(store.errors.is_empty(), "{:?}", store.errors);
    let paths = store.documents.iter().map(|it| it.path.clone()).collect::<Vec<_>>();
    assert_eq!(paths, vec!["document_paths/attachments/statement.txt"; 2]);
    assert!(dir().join(&paths[0]).is_file());
}

#[test]
fn every_ledger_has_an_oracle() {
    let mut ledgers = std::fs::read_dir(dir())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .filter_map(|name| name.strip_suffix(".bean").map(str::to_owned))
        .collect::<Vec<_>>();
    ledgers.sort();
    let cases = oracle_cases().keys().cloned().collect::<Vec<_>>();
    assert_eq!(ledgers, cases);
}
