//! Characterization tests for today's booking (booking-split design, #423).
//!
//! They pin what the store fold does now, bugs included, so the booking refactor can prove it
//! changes only what it means to change. A `current_behavior_eN_*` test pins quirk EN of the
//! design and is expected to flip in the PR that fixes EN.

use std::sync::Arc;

use indoc::{formatdoc, indoc};
use zhang_core::ast::error::ErrorKind;
use zhang_core::data_source::LocalFileSystemDataSource;
use zhang_core::data_type::text::ZhangDataType;
use zhang_core::ledger::Ledger;
use zhang_core::{ZhangError, ZhangResult};

const HEADER: &str = indoc! {r#"
    1970-01-01 commodity USD
    1970-01-01 commodity CNY
    1970-01-01 open Assets:A
    1970-01-01 open Income:I
"#};

const BUY_10_AT_10: &str = indoc! {r#"
    2024-05-16 * "buy"
      Assets:A 10 USD { 10 CNY }
      Income:I -100 CNY
"#};

/// load `HEADER` followed by `body` as a single-file ledger
fn try_load(body: &str) -> ZhangResult<Ledger> {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("main.zhang"), format!("{HEADER}{body}")).unwrap();
    let source = LocalFileSystemDataSource::new(ZhangDataType {});
    Ledger::load_with_data_source(dir.path().to_path_buf(), "main.zhang".to_owned(), Arc::new(source))
}

fn load(body: &str) -> Ledger {
    try_load(body).unwrap_or_else(|e| panic!("ledger should load: {e}"))
}

/// reported errors in store order, with their `transaction_amount` meta
fn errors(ledger: &Ledger) -> Vec<(ErrorKind, Option<String>)> {
    let store = ledger.store.read().unwrap();
    store
        .errors
        .iter()
        .map(|it| (it.error_type.clone(), it.metas.get("transaction_amount").cloned()))
        .collect()
}

/// lots of one account in store order, as `units {cost, acquisition date}`
fn lots(ledger: &Ledger, account: &str) -> Vec<String> {
    let store = ledger.store.read().unwrap();
    let lots = store.commodity_lots.get(account).cloned().unwrap_or_default();
    lots.iter()
        .map(|lot| match (&lot.cost, &lot.acquisition_date) {
            (None, None) => format!("{} {}", lot.amount, lot.commodity),
            (Some(cost), Some(date)) => format!("{} {} {{{cost}, {date}}}", lot.amount, lot.commodity),
            (cost, date) => format!("{} {} {cost:?} {date:?}", lot.amount, lot.commodity),
        })
        .collect()
}

/// inferred amounts of the transaction with the given sequence, in written order
fn inferred(ledger: &Ledger, sequence: i32) -> Vec<String> {
    let store = ledger.store.read().unwrap();
    store
        .postings
        .iter()
        .filter(|it| it.trx_sequence == sequence)
        .map(|it| it.inferred_amount.to_string())
        .collect()
}

fn load_with_booking_method(method: &str) -> ZhangResult<Ledger> {
    try_load(&formatdoc! {r#"
        1970-01-01 open Assets:S
          booking_method: "{method}"
        2024-05-16 * "buy"
          Assets:S 10 USD {{ 10 CNY }}
          Income:I -100 CNY
    "#})
}

#[test]
#[should_panic(expected = "not implemented")]
fn current_behavior_e1_strict_booking_panics() {
    // current behavior (booking-split design E1, #423); expected to change in the E1 fix PR
    let _ = load_with_booking_method("STRICT");
}

#[test]
#[should_panic(expected = "not implemented")]
fn current_behavior_e1_average_booking_panics() {
    // current behavior (booking-split design E1, #423); expected to change in the E1 fix PR
    let _ = load_with_booking_method("AVERAGE");
}

#[test]
#[should_panic(expected = "not implemented")]
fn current_behavior_e1_average_only_booking_panics() {
    // current behavior (booking-split design E1, #423); expected to change in the E1 fix PR
    let _ = load_with_booking_method("AVERAGE_ONLY");
}

#[test]
#[should_panic(expected = "not implemented")]
fn current_behavior_e1_none_booking_panics() {
    // current behavior (booking-split design E1, #423); expected to change in the E1 fix PR
    let _ = load_with_booking_method("NONE");
}

#[test]
fn current_behavior_e2_cross_commodity_transaction_is_accepted() {
    // current behavior (booking-split design E2, #423); expected to change in the E2/E3 fix PR
    let ledger = load(indoc! {r#"
        2024-05-16 * "two commodities, no price"
          Assets:A 10 USD
          Income:I -10 CNY
    "#});
    assert_eq!(errors(&ledger), vec![]);
}

#[test]
fn current_behavior_e3_price_weight_is_ignored_by_the_balance_check() {
    // current behavior (booking-split design E3, #423); expected to change in the E2/E3 fix PR
    let ledger = load(indoc! {r#"
        2024-05-16 * "weight 100 CNY against 10 CNY"
          Assets:A 10 USD @ 10 CNY
          Income:I -10 CNY
    "#});
    assert_eq!(errors(&ledger), vec![]);
}

#[test]
fn current_behavior_e4_implicit_posting_of_brace_sale_gets_units() {
    // current behavior (booking-split design E4, #423); expected to change in the E4 fix PR
    let ledger = load(indoc! {r#"
        2024-05-16 * "buy"
          Assets:A 10 USD { 10 CNY }
          Income:I -100 CNY
        2024-05-17 * "buy"
          Assets:A 10 USD { 11 CNY }
          Income:I -110 CNY
        2024-05-18 * "sell, implicit income"
          Assets:A -15 USD {}
          Income:I
    "#});
    assert_eq!(errors(&ledger), vec![]);
    assert_eq!(inferred(&ledger, 3), vec!["-15 USD", "15 USD"]);
    assert_eq!(lots(&ledger, "Assets:A"), vec!["5 USD {11 CNY, 2024-05-17}"]);
    assert_eq!(lots(&ledger, "Income:I"), vec!["-210 CNY", "15 USD"]);
}

#[test]
fn current_behavior_e5_undated_cost_only_matches_lots_of_the_txn_date() {
    // current behavior (booking-split design E5, #423); expected to change in the E5 fix PR
    let ledger = load(&format!(
        "{BUY_10_AT_10}{}",
        indoc! {r#"
            2024-05-17 * "sell with an undated cost, next day"
              Assets:A -5 USD { 10 CNY }
              Income:I 50 CNY
        "#}
    ));
    assert_eq!(errors(&ledger), vec![(ErrorKind::NoEnoughCommodityLot, Some("-5".to_owned()))]);
    assert_eq!(lots(&ledger, "Assets:A"), vec!["10 USD {10 CNY, 2024-05-16}", "-5 USD {10 CNY, 2024-05-17}"]);
}

#[test]
fn current_behavior_e6_empty_cost_augmentation_merges_into_the_first_cost_lot() {
    // current behavior (booking-split design E6, #423); expected to change in a follow-up fix PR
    let ledger = load(&format!(
        "{BUY_10_AT_10}{}",
        indoc! {r#"
            2024-05-17 * "buy with an empty cost"
              Assets:A 3 USD {}
              Income:I -30 CNY
        "#}
    ));
    assert_eq!(errors(&ledger), vec![]);
    assert_eq!(lots(&ledger, "Assets:A"), vec!["13 USD {10 CNY, 2024-05-16}"]);
}

#[test]
fn current_behavior_e7_invalid_booking_method_aborts_the_load() {
    // current behavior (booking-split design E7, #423); expected to change in the E7 fix PR
    let result = try_load(indoc! {r#"
        1970-01-01 open Assets:X
          booking_method: "NON_EXIST"
        2024-05-16 * "plain"
          Assets:X 10 CNY
          Income:I -10 CNY
    "#});
    assert!(matches!(
        result,
        Err(ZhangError::ProcessError {
            kind: ErrorKind::ParseInvalidMeta,
            ..
        })
    ));
}

#[test]
fn current_behavior_e8_balance_correction_on_cost_account_goes_to_the_default_lot() {
    // current behavior (booking-split design E8, #423); expected to change in a follow-up fix PR
    let ledger = load(&format!("{BUY_10_AT_10}2024-05-17 balance Assets:A 8 USD\n"));
    assert_eq!(errors(&ledger), vec![(ErrorKind::AccountBalanceCheckError, None)]);
    assert_eq!(lots(&ledger, "Assets:A"), vec!["10 USD {10 CNY, 2024-05-16}", "-2 USD"]);
}

#[test]
fn current_behavior_e9_empty_cost_reduction_without_cost_lots_adds_a_second_default_lot() {
    // current behavior (booking-split design E9, #423); expected to change in a follow-up fix PR
    let ledger = load(indoc! {r#"
        2024-05-16 * "plain units"
          Assets:A 5 USD
          Income:I -5 USD
        2024-05-17 * "sell at {} with no cost lots"
          Assets:A -3 USD {}
          Income:I 3 USD
        2024-05-18 * "plain units again"
          Assets:A 1 USD
          Income:I -1 USD
    "#});
    assert_eq!(errors(&ledger), vec![(ErrorKind::NoEnoughCommodityLot, Some("-3".to_owned()))]);
    assert_eq!(lots(&ledger, "Assets:A"), vec!["6 USD", "-3 USD"]);
}

#[test]
fn current_behavior_e10_fifo_follows_lot_creation_order() {
    // current behavior (booking-split design E10, #423); expected to change in the E10 fix PR
    let ledger = load(indoc! {r#"
        2024-05-16 * "buy, later acquisition date first"
          Assets:A 10 USD { 10 CNY, 2024-05-10 }
          Income:I -100 CNY
        2024-05-17 * "buy, earlier acquisition date second"
          Assets:A 10 USD { 11 CNY, 2024-01-01 }
          Income:I -110 CNY
        2024-05-18 * "sell FIFO"
          Assets:A -5 USD {}
          Income:I 50 CNY
    "#});
    assert_eq!(errors(&ledger), vec![]);
    assert_eq!(lots(&ledger, "Assets:A"), vec!["5 USD {10 CNY, 2024-05-10}", "10 USD {11 CNY, 2024-01-01}"]);
}

#[test]
fn current_behavior_e11_postings_to_missing_or_closed_accounts_are_not_reported() {
    // current behavior (booking-split design E11, #423, tracked in #444); expected to change in the #444 fix PR
    let ledger = load(indoc! {r#"
        1970-01-01 open Assets:B
        1970-01-02 close Assets:B
        2024-05-16 * "to a missing and a closed account"
          Assets:Missing 10 CNY
          Assets:B -10 CNY
    "#});
    assert_eq!(errors(&ledger), vec![]);
}

#[test]
fn no_enough_lot_reports_the_written_units_as_transaction_amount() {
    // the matched lot covers 10 of the 15 written units; the meta carries the written -15, not the -5 remainder
    let ledger = load(&format!(
        "{BUY_10_AT_10}{}",
        indoc! {r#"
            2024-05-16 * "sell more than held"
              Assets:A -15 USD { 10 CNY }
              Income:I 150 CNY
        "#}
    ));
    assert_eq!(errors(&ledger), vec![(ErrorKind::NoEnoughCommodityLot, Some("-15".to_owned()))]);
    assert_eq!(lots(&ledger, "Assets:A"), vec!["-5 USD {10 CNY, 2024-05-16}"]);
}

#[test]
fn errors_of_different_kinds_keep_stage_then_stream_order() {
    // each transaction has exactly one problem: with several, the reported kind depends on HashMap order (#441)
    let ledger = load(indoc! {r#"
        1970-01-01 open Assets:B
        2024-01-01 * "undefined commodity"
          Assets:A 10 JPY
          Income:I -10 JPY
        2024-01-02 * "unbalanced"
          Assets:A 10 CNY
          Income:I -9 CNY
        2024-01-03 * "sell a lot never bought"
          Assets:A -5 USD { 10 CNY }
          Income:I 50 CNY
        2024-01-04 * "two implicit postings"
          Assets:A
          Income:I
        2024-01-05 balance Assets:B 1 CNY
    "#});
    assert_eq!(
        errors(&ledger),
        vec![
            (ErrorKind::AccountBalanceCheckError, None),
            (ErrorKind::CommodityDoesNotDefine, None),
            (ErrorKind::UnbalancedTransaction, None),
            (ErrorKind::NoEnoughCommodityLot, Some("-5".to_owned())),
            (ErrorKind::TransactionHasMultipleImplicitPosting, None),
        ]
    );
}

#[test]
fn budget_activity_of_implicit_postings_next_to_cost_postings() {
    let ledger = load(&formatdoc! {r#"
        1970-01-01 open Expenses:Food
          budget: food
        1970-01-01 open Expenses:Fun
          budget: fun
        2024-05-01 budget food CNY
        2024-05-01 budget fun CNY
        {BUY_10_AT_10}
        2024-05-17 * "pay with a dated cost sale"
          Assets:A -5 USD {{ 10 CNY, 2024-05-16 }}
          Expenses:Food
        2024-05-18 * "pay with an empty cost sale"
          Assets:A -5 USD {{}}
          Expenses:Fun
    "#});
    let store = ledger.store.read().unwrap();
    let activity = |budget: &str| store.budgets[budget].detail[&202405].activity_amount.to_string();
    assert_eq!(activity("food"), "50 CNY");
    // current behavior (booking-split design E4, #423); expected to change in the E4 fix PR:
    // the implicit posting gets +5 USD and the CNY budget adds its number
    assert_eq!(activity("fun"), "5 CNY");
}
