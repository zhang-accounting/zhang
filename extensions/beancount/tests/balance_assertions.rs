//! Balance assertions and pads, checked against Python beancount: for every ledger in
//! `balance_assertions/`, zhang ends with the same balances, books the same pads and finds the
//! same assertions passing and failing, against the same balances, as beancount 3.2.3 does
//! (`balance_assertions/oracle.json`, written by `balance_assertions/generate.py`).
//!
//! A balance assertion never moves a balance: the balances are the sums of the postings, and a
//! pad is sized from them. An assertion or a pad covers the account and all its sub-accounts. A
//! ledger the oracle marks with an `accepted_deviation` is checked against zhang's own rule instead.

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

/// (padded account, units, account padded from)
type Pad = (String, (BigDecimal, String), String);

/// (date, account, asserted amount, balance it was checked against, passed)
type Assertion = (String, String, (BigDecimal, String), (BigDecimal, String), bool);

#[derive(Debug, PartialEq)]
struct Outcome {
    balances: Balances,
    /// sorted: zhang books the paddings in the order of the assertions they serve, beancount in the order
    /// of the pads
    pads: Vec<Pad>,
    /// sorted: beancount orders the entries of a day by their line, whatever their file
    assertions: Vec<Assertion>,
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
        .map(|pad| (text(&pad["account"]), amount(&pad["units"]), text(&pad["from"])))
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
            (padded.account.name().to_owned(), of(&padded.inferred_amount), from.account.name().to_owned())
        })
        .collect::<Vec<_>>();
    pads.sort();

    // every `balance` of the beancount ledger, a `balance ... with pad` where a pad serves it
    let mut assertions = ledger
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
        .collect::<Vec<_>>();
    assertions.sort();
    Outcome { balances, pads, assertions }
}

/// the ledgers whose deviation from beancount the tests below check
const DEVIATIONS: &[&str] = &["child_assertion_after_parent_pad", "nested_pads"];

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
        assert_eq!(expected["unused_pads"], Value::Array(vec![]), "{case}: beancount uses every pad");
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

fn padding(account: &str, units: &str) -> Pad {
    (account.to_owned(), cny(units), "Equity:Open".to_owned())
}

/// the oracle of an accepted deviation, with the reason it records
fn deviation(case: &str) -> (Outcome, String, Value) {
    let expected = &oracle_cases()[case];
    let reason = expected["accepted_deviation"]
        .as_str()
        .unwrap_or_else(|| panic!("{case} is an accepted deviation"))
        .to_owned();
    (oracle(expected), reason, expected["unused_pads"].clone())
}

#[test]
fn a_pad_serves_the_assertions_on_its_own_account_only() {
    let (beancount, reason, unused) = deviation("child_assertion_after_parent_pad");
    assert!(reason.contains("its own account only"), "{reason}");
    // the assertion on the sub-account uses up the pad of the parent in beancount
    assert_eq!(unused, serde_json::json!([{"account": "Assets:Bank", "date": "2024-01-03"}]));
    assert!(beancount.pads.is_empty());
    assert_eq!(
        beancount.assertions,
        vec![
            assertion("2024-01-04", "Assets:Bank:Checking", "60", "60", true),
            assertion("2024-01-05", "Assets:Bank", "100", "60", false),
        ]
    );

    let zhang = zhang("child_assertion_after_parent_pad");
    assert_eq!(zhang.pads, vec![padding("Assets:Bank", "40")]);
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
    let (beancount, reason, _) = deviation("nested_pads");
    assert!(reason.contains("every padding before it"), "{reason}");
    // beancount pads the parent by 60 where 40 are missing
    assert_eq!(beancount.pads, vec![padding("Assets:Bank", "60"), padding("Assets:Bank:Checking", "60")]);
    assert_eq!(
        beancount.assertions,
        vec![
            assertion("2024-01-03", "Assets:Bank", "100", "120", false),
            assertion("2024-01-03", "Assets:Bank:Checking", "60", "60", true),
        ]
    );

    let zhang = zhang("nested_pads");
    assert_eq!(zhang.pads, vec![padding("Assets:Bank", "40"), padding("Assets:Bank:Checking", "60")]);
    assert_eq!(
        zhang.assertions,
        vec![
            assertion("2024-01-03", "Assets:Bank", "100", "100", true),
            assertion("2024-01-03", "Assets:Bank:Checking", "60", "60", true),
        ]
    );
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
