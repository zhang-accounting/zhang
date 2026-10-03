//! Balance assertions and pads, checked against Python beancount: for every ledger in
//! `balance_assertions/`, zhang ends with the same balances, books the same pads and finds the
//! same assertions passing and failing, against the same balances, as beancount 3.2.3 does
//! (`balance_assertions/oracle.json`, written by `balance_assertions/generate.py`).
//!
//! A balance assertion never moves a balance: the balances are the sums of the postings, and a
//! pad is sized from them.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;

use beancount::Beancount;
use bigdecimal::{BigDecimal, Zero};
use serde_json::Value;
use zhang_ast::amount::Amount;
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

/// account -> currency -> units, zeros left out
type Balances = BTreeMap<String, BTreeMap<String, BigDecimal>>;

/// (padded account, units, account padded from)
type Pad = (String, (BigDecimal, String), String);

/// (date, account, asserted amount, balance it was checked against, passed)
type Assertion = (String, String, (BigDecimal, String), (BigDecimal, String), bool);

struct Outcome {
    balances: Balances,
    pads: Vec<Pad>,
    assertions: Vec<Assertion>,
}

fn oracle(case: &str) -> Outcome {
    let oracle: Value = serde_json::from_str(&std::fs::read_to_string(dir().join("oracle.json")).unwrap()).unwrap();
    let case = &oracle[case];
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
    let pads = case["pads"]
        .as_array()
        .unwrap()
        .iter()
        .map(|pad| {
            (
                pad["account"].as_str().unwrap().to_owned(),
                amount(&pad["units"]),
                pad["from"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    let assertions = case["assertions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|it| {
            (
                it["date"].as_str().unwrap().to_owned(),
                it["account"].as_str().unwrap().to_owned(),
                amount(&it["amount"]),
                amount(&it["balance"]),
                it["passed"].as_bool().unwrap(),
            )
        })
        .collect();
    Outcome { balances, pads, assertions }
}

fn zhang(case: &str) -> Outcome {
    let ledger = load(case);
    let operations = ledger.operations();
    let store = ledger.store.read().unwrap();

    // the sums of the postings, which is what the balances shown are
    let mut balances = Balances::new();
    for posting in &store.postings {
        let held = balances.entry(posting.account.name().to_owned()).or_default();
        let units = held.entry(posting.inferred_amount.commodity.clone()).or_insert_with(BigDecimal::zero);
        *units += &posting.inferred_amount.number;
    }
    for (account, held) in balances.iter_mut() {
        let shown = operations
            .single_account_latest_balances(account)
            .unwrap()
            .into_iter()
            .map(|it| (it.balance.commodity, it.balance.number.normalized()))
            .collect::<BTreeMap<_, _>>();
        held.retain(|_, units| !units.is_zero());
        for units in held.values_mut() {
            *units = units.normalized();
        }
        let shown = shown.into_iter().filter(|(_, units)| !units.is_zero()).collect::<BTreeMap<_, _>>();
        assert_eq!(&shown, held, "{case}: the balance shown for {account} is the sum of its postings");
    }
    balances.retain(|_, held| !held.is_empty());

    let mut padding = store.transactions.values().filter(|txn| txn.flag == Flag::BalancePad).collect::<Vec<_>>();
    padding.sort_by_key(|txn| txn.sequence);
    let pads = padding
        .into_iter()
        .map(|txn| {
            let [padded, from] = &txn.postings[..] else {
                panic!("{case}: a padding transaction has two postings");
            };
            (padded.account.name().to_owned(), of(&padded.inferred_amount), from.account.name().to_owned())
        })
        .collect();

    // every `balance` of the beancount ledger, a `balance ... with pad` where a pad serves it
    let assertions = ledger
        .directives
        .iter()
        .filter_map(|directive| match &directive.data {
            Directive::BalanceCheck(check) => {
                let record = store
                    .balance_assertions
                    .iter()
                    .find(|it| it.span == directive.span)
                    .expect("every check is kept");
                Some((
                    check.date.naive_date().to_string(),
                    check.account.name().to_owned(),
                    of(&check.amount),
                    of(&record.balance),
                    record.passed,
                ))
            }
            // the pad brings the account to the asserted amount, so the assertion holds
            Directive::BalancePad(pad) => Some((
                pad.date.naive_date().to_string(),
                pad.account.name().to_owned(),
                of(&pad.amount),
                of(&pad.amount),
                true,
            )),
            _ => None,
        })
        .collect();
    Outcome { balances, pads, assertions }
}

/// zhang's pads come in the order of the assertions they serve, beancount's in the order of the
/// `pad` directives: compare them per account, in order
fn by_account(mut pads: Vec<Pad>) -> Vec<Pad> {
    pads.sort_by(|a, b| a.0.cmp(&b.0));
    pads
}

fn check(case: &str) {
    let expected = oracle(case);
    let actual = zhang(case);
    assert_eq!(actual.balances, expected.balances, "{case}: balances");
    assert_eq!(by_account(actual.pads), by_account(expected.pads), "{case}: pads");
    assert_eq!(actual.assertions, expected.assertions, "{case}: assertions");
}

#[test]
fn a_failing_assertion_moves_no_balance_and_the_next_pad_starts_from_the_true_balance() {
    check("failing_assertion");
}

#[test]
fn a_pad_serves_only_the_next_assertion() {
    check("pad_then_failing_assertion");
}

#[test]
fn two_pads_are_each_sized_from_the_postings() {
    check("two_pads");
}

#[test]
fn an_assertion_within_its_tolerance_passes_and_moves_nothing() {
    check("tolerance");
}

#[test]
fn a_pad_serves_the_next_assertion_of_each_currency() {
    check("multi_currency");
}

#[test]
fn an_assertion_on_the_day_of_a_pad_is_not_padded() {
    check("same_day_pad");
}

#[test]
fn an_assertion_on_a_parent_account_checks_the_account_alone() {
    // a known difference: beancount sums the account and its sub-accounts, zhang the
    // postings of the account itself. Balances and pads agree
    let expected = oracle("parent_account");
    let actual = zhang("parent_account");
    assert_eq!(actual.balances, expected.balances);
    assert_eq!(actual.pads, expected.pads);
    let cny = |number: i32| (BigDecimal::from(number), "CNY".to_owned());
    let assertion =
        |date: &str, account: &str, asserted: i32, balance: i32, passed: bool| (date.to_owned(), account.to_owned(), cny(asserted), cny(balance), passed);
    assert_eq!(
        expected.assertions,
        vec![
            assertion("2024-01-02", "Assets:Bank", 70, 70, true),
            assertion("2024-01-03", "Assets:Bank", 50, 70, false),
            assertion("2024-01-04", "Assets:Bank:Checking", 20, 20, true),
        ]
    );
    assert_eq!(
        actual.assertions,
        vec![
            assertion("2024-01-02", "Assets:Bank", 70, 50, false),
            assertion("2024-01-03", "Assets:Bank", 50, 50, true),
            assertion("2024-01-04", "Assets:Bank:Checking", 20, 20, true),
        ]
    );
}

#[test]
fn every_ledger_has_an_oracle() {
    let oracle: Value = serde_json::from_str(&std::fs::read_to_string(dir().join("oracle.json")).unwrap()).unwrap();
    let mut ledgers = std::fs::read_dir(dir())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .filter_map(|name| name.strip_suffix(".bean").map(str::to_owned))
        .collect::<Vec<_>>();
    ledgers.sort();
    let cases = oracle.as_object().unwrap().keys().cloned().collect::<Vec<_>>();
    assert_eq!(ledgers, cases);
}
