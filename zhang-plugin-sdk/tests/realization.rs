//! Sparse realization over raw test directives and real booked ledgers.
#![cfg(not(target_arch = "wasm32"))]

use std::collections::BTreeMap;
use std::str::FromStr;
use std::sync::Arc;

use bigdecimal::BigDecimal;
use zhang_ast::{Date, Directive, PostingCost, Spanned};
use zhang_core::data_source::LocalFileSystemDataSource;
use zhang_core::data_type::text::ZhangDataType;
use zhang_core::data_type::DataType;
use zhang_core::ledger::Ledger;
use zhang_plugin_sdk::realization::{AccountBalance, AccountScope, SparseRealization, UnbookedPosting};

fn number(text: &str) -> BigDecimal {
    BigDecimal::from_str(text).unwrap()
}

fn amounts(values: &[(&str, &str)]) -> BTreeMap<String, BigDecimal> {
    values.iter().map(|(commodity, value)| (commodity.to_string(), number(value))).collect()
}

fn stream(content: &str) -> Vec<Spanned<Directive>> {
    ZhangDataType {}.transform(content.to_owned(), None).unwrap()
}

fn load(content: &str) -> Ledger {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("main.zhang"), content).unwrap();
    let source = Arc::new(LocalFileSystemDataSource::new(ZhangDataType {}));
    Ledger::load_with_data_source(dir.path().to_path_buf(), "main.zhang".to_owned(), source).unwrap()
}

#[test]
fn exact_and_subtree_scopes_keep_only_targets_and_match_account_boundaries() {
    let input = stream(
        r#"
2024-01-01 * "own"
  Assets:Bank 1 USD
2024-01-02 * "child"
  Assets:Bank:Cash 2 USD
2024-01-03 * "grandchild"
  Assets:Bank:Cash:Wallet 4 USD
2024-01-04 * "different prefix"
  Assets:Banking 8 USD
"#,
    );
    let targets = ["Assets:Bank", "Assets:Bank:Cash", "Assets:Empty", "Assets", "Assets:Bank"];
    let exact = SparseRealization::from_stream(&input, targets, AccountScope::Exact).unwrap();
    let subtree = SparseRealization::from_stream(&input, targets, AccountScope::Subtree).unwrap();
    assert_eq!(exact.get("Assets:Bank").unwrap().units, amounts(&[("USD", "1")]));
    assert_eq!(exact.get("Assets:Bank:Cash").unwrap().units, amounts(&[("USD", "2")]));
    assert_eq!(exact.get("Assets").unwrap(), &AccountBalance::default());
    assert_eq!(subtree.get("Assets:Bank").unwrap().units, amounts(&[("USD", "7")]));
    assert_eq!(subtree.get("Assets:Bank:Cash").unwrap().units, amounts(&[("USD", "6")]));
    assert_eq!(subtree.get("Assets").unwrap().units, amounts(&[("USD", "15")]));
    assert_eq!(subtree.get("Assets:Empty").unwrap(), &AccountBalance::default());
    assert!(subtree.get("Assets:Bank:Cash:Wallet").is_none());
    assert!(subtree.get("Assets:Banking").is_none());
    assert_eq!(
        subtree.accounts().map(|(name, _)| name).collect::<Vec<_>>(),
        vec!["Assets", "Assets:Bank", "Assets:Bank:Cash", "Assets:Empty"]
    );
    let empty = SparseRealization::from_stream(&input, [] as [&str; 0], AccountScope::Subtree).unwrap();
    assert_eq!(empty.accounts().count(), 0);
}

#[test]
fn running_balances_keep_currencies_separate_and_prune_cancelled_totals() {
    let input = stream(
        r#"
2024-01-01 * "add"
  Assets:Bank 2.50 USD
  Assets:Bank 3 EUR
2024-01-02 * "cancel USD"
  Assets:Bank -2.50 USD
  Assets:Bank 0 GBP
"#,
    );
    let mut balances = SparseRealization::new(["Assets:Bank"], AccountScope::Exact);
    balances.apply(&input[0].data).unwrap();
    assert_eq!(balances.get("Assets:Bank").unwrap().units, amounts(&[("USD", "2.50"), ("EUR", "3")]));
    balances.apply(&input[1].data).unwrap();
    let balance = balances.get("Assets:Bank").unwrap();
    assert_eq!(balance.units, amounts(&[("EUR", "3")]));
    assert_eq!(balance.cost, amounts(&[("EUR", "3")]));
}

#[test]
fn costs_use_each_booked_leg_and_ignore_prices() {
    let input = stream(
        r#"
2024-01-01 * "booked legs"
  Assets:Broker 3 AAPL {2 USD, 2023-12-01} @ 99 GBP
  Assets:Broker 2 AAPL {3 USD, 2023-12-02}
  Assets:Broker 4 EUR @@ 88 GBP
  Assets:Broker 1 FREE {0 USD, 2023-12-03}
"#,
    );
    let balances = SparseRealization::from_stream(&input, ["Assets:Broker"], AccountScope::Exact).unwrap();
    let balance = balances.get("Assets:Broker").unwrap();
    assert_eq!(balance.units, amounts(&[("AAPL", "5"), ("EUR", "4"), ("FREE", "1")]));
    assert_eq!(balance.cost, amounts(&[("USD", "12"), ("EUR", "4")]));
}

#[test]
fn a_bad_matching_posting_leaves_every_target_unchanged_and_can_be_reported() {
    let input = stream(
        r#"
2024-01-01 * "initial"
  Assets:Bank 10 USD
2024-01-02 * "unbooked"
  Assets:Bank 5 USD
  Assets:Broker 2 AAPL {3 USD, 2023-12-01}
  Assets:Broker
2024-01-03 * "next"
  Assets:Bank 1 USD
"#,
    );
    let mut balances = SparseRealization::new(["Assets", "Assets:Bank", "Assets:Broker"], AccountScope::Subtree);
    balances.apply(&input[0].data).unwrap();
    let before = balances.clone();
    let error = balances.apply(&input[1].data).unwrap_err();
    assert_eq!(
        error,
        UnbookedPosting {
            account: "Assets:Broker".to_owned(),
            posting_index: 2
        }
    );
    assert_eq!(balances, before, "no partial transaction was applied");
    assert!(error.to_string().contains("requires booked postings"));
    let sdk_error: zhang_plugin_sdk::Error = error.into();
    assert!(sdk_error.message().contains("Assets:Broker"));
    balances.apply(&input[2].data).unwrap();
    assert_eq!(balances.get("Assets").unwrap().units, amounts(&[("USD", "11")]));
}

#[test]
fn unresolved_costs_are_rejected_but_irrelevant_unbooked_postings_are_ignored() {
    let input = stream(
        r#"
2024-01-01 * "raw"
  Assets:Bank 2 USD
  Assets:Other
"#,
    );
    let balances = SparseRealization::from_stream(&input, ["Assets:Bank"], AccountScope::Exact).unwrap();
    assert_eq!(balances.get("Assets:Bank").unwrap().units, amounts(&[("USD", "2")]));
    let Directive::Transaction(mut txn) = input[0].data.clone() else {
        unreachable!()
    };
    let base = txn.postings[0].units.clone();
    let date = Some(Date::Date(chrono::NaiveDate::from_ymd_opt(2024, 1, 1).unwrap()));
    for cost in [
        PostingCost::default(),
        PostingCost {
            base: base.clone(),
            ..Default::default()
        },
        PostingCost {
            date: date.clone(),
            ..Default::default()
        },
        PostingCost {
            base: base.clone(),
            date: date.clone(),
            total: true,
            ..Default::default()
        },
    ] {
        txn.postings[0].cost = Some(cost);
        let mut balances = SparseRealization::new(["Assets:Bank"], AccountScope::Exact);
        assert_eq!(balances.apply(&Directive::Transaction(txn.clone())).unwrap_err().posting_index, 0);
        assert_eq!(balances.get("Assets:Bank").unwrap(), &AccountBalance::default());
    }
}

#[test]
fn assertions_do_not_book_and_transaction_flags_do_not_change_arithmetic() {
    let input = stream(
        r#"
option "operating_currency" "USD"
1970-01-01 open Assets:Bank
2024-01-01 balance Assets:Bank 99 USD
2024-01-02 balance Assets:Bank 100 USD with pad Equity:Opening
2024-01-03 price USD 7 CNY
2024-01-04 P "padding already in the stream"
  Assets:Bank 3 USD
2024-01-05 C "ordinary conversion transaction"
  Assets:Bank -1 USD
"#,
    );
    let balances = SparseRealization::from_stream(&input, ["Assets:Bank"], AccountScope::Exact).unwrap();
    assert_eq!(balances.get("Assets:Bank").unwrap().units, amounts(&[("USD", "2")]));
}

#[test]
fn sums_and_cost_products_are_exact_and_keep_decimal_scale() {
    let input = stream(
        r#"
2024-01-01 * "more than 28 digits"
  Assets:Broker 123456789012345678901234567890123.00 AAPL {1.00 USD, 2023-12-01}
2024-01-02 * "one cent"
  Assets:Broker 0.01 AAPL {1.00 USD, 2023-12-01}
"#,
    );
    let balances = SparseRealization::from_stream(&input, ["Assets:Broker"], AccountScope::Exact).unwrap();
    let balance = balances.get("Assets:Broker").unwrap();
    assert_eq!(balance.units["AAPL"].to_string(), "123456789012345678901234567890123.01");
    assert_eq!(balance.cost["USD"].to_string(), "123456789012345678901234567890123.0100");
}

const LOTS: &str = r#"
option "operating_currency" "USD"
1970-01-01 commodity USD
1970-01-01 commodity AAPL
1970-01-01 open Assets:Broker
1970-01-01 open Assets:Cash
1970-01-01 open Income:Gains
2024-01-10 * "buy a"
  Assets:Broker 10 AAPL {100 USD}
  Assets:Cash -1000 USD
2024-01-20 * "buy b"
  Assets:Broker 10 AAPL {{1100 USD}}
  Assets:Cash -1100 USD
2024-02-01 * "sell across both"
  Assets:Broker -15 AAPL {}
  Assets:Cash 1800 USD
  Income:Gains
"#;

#[test]
fn real_booking_split_legs_and_implicit_units_agree_with_store_lots() {
    let ledger = load(LOTS);
    let store = ledger.store.read().unwrap();
    assert!(store.errors.is_empty());
    let balances = SparseRealization::from_stream(&ledger.directives, ["Assets", "Assets:Broker", "Income:Gains"], AccountScope::Subtree).unwrap();
    let broker = balances.get("Assets:Broker").unwrap();
    assert_eq!(broker.units, amounts(&[("AAPL", "5")]));
    assert_eq!(broker.cost, amounts(&[("USD", "550")]));
    assert_eq!(balances.get("Income:Gains").unwrap().units, amounts(&[("USD", "-250")]));
    assert_eq!(balances.get("Assets").unwrap().cost, amounts(&[("USD", "250")]));
    // An independent fold of the surviving store lots, not the booked sale legs.
    let lot_units: BigDecimal = store.commodity_lots["Assets:Broker"].iter().map(|lot| lot.amount.clone()).sum();
    let lot_cost: BigDecimal = store.commodity_lots["Assets:Broker"]
        .iter()
        .map(|lot| &lot.amount * &lot.cost.as_ref().unwrap().number)
        .sum();
    assert_eq!(broker.units["AAPL"], lot_units);
    assert_eq!(broker.cost["USD"], lot_cost);
    let sale = ledger
        .directives
        .iter()
        .find_map(|entry| match &entry.data {
            Directive::Transaction(txn) if txn.narration.as_ref().unwrap().as_str() == "sell across both" => Some(txn),
            _ => None,
        })
        .unwrap();
    assert_eq!(sale.postings.len(), 4);
    assert_eq!(
        sale.postings[0].written.as_ref().unwrap().index,
        sale.postings[1].written.as_ref().unwrap().index
    );
    assert_ne!(sale.postings[0].cost.as_ref().unwrap().date, sale.postings[1].cost.as_ref().unwrap().date);
}

#[test]
fn a_failing_assertion_keeps_lots_and_totals_and_a_real_pad_counts() {
    let ledger = load(&format!(
        r#"{LOTS}
1970-01-01 open Equity:Opening
2024-02-02 balance Assets:Broker 99 AAPL
2024-02-03 balance Assets:Cash 0 USD with pad Equity:Opening
"#
    ));
    let balances = SparseRealization::from_stream(&ledger.directives, ["Assets:Broker", "Assets:Cash", "Equity:Opening"], AccountScope::Exact).unwrap();
    assert_eq!(balances.get("Assets:Broker").unwrap().cost, amounts(&[("USD", "550")]));
    assert_eq!(balances.get("Assets:Cash").unwrap(), &AccountBalance::default());
    assert_eq!(balances.get("Equity:Opening").unwrap().units, amounts(&[("USD", "-300")]));
}
