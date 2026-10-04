//! Characterization tests for today's booking (booking-split design, #423).
//!
//! They pin booking and materialization behavior, so the booking refactor can prove it
//! changes only what it means to change. A `current_behavior_eN_*` test pins quirk EN of the
//! design and is expected to flip in the PR that fixes EN; an `eN_*` test is the fixed behavior.

use std::collections::BTreeMap;
use std::sync::Arc;

use indoc::{formatdoc, indoc};
use zhang_core::ast::error::ErrorKind;
use zhang_core::data_source::LocalFileSystemDataSource;
use zhang_core::data_type::text::ZhangDataType;
use zhang_core::ledger::Ledger;
use zhang_core::ZhangResult;

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

/// reported errors in store order, with the first line of their span and all their metas
fn error_details(ledger: &Ledger) -> Vec<(ErrorKind, String, BTreeMap<String, String>)> {
    let store = ledger.store.read().unwrap();
    store
        .errors
        .iter()
        .map(|it| {
            let span = it.span.as_ref().and_then(|span| span.content.lines().next()).unwrap_or_default();
            (it.error_type.clone(), span.to_owned(), it.metas.clone().into_iter().collect())
        })
        .collect()
}

fn metas<const N: usize>(pairs: [(&str, &str); N]) -> BTreeMap<String, String> {
    pairs.into_iter().map(|(key, value)| (key.to_owned(), value.to_owned())).collect()
}

/// lots of one account in store order, as `units {cost, acquisition date, "label"}`
fn lots(ledger: &Ledger, account: &str) -> Vec<String> {
    let store = ledger.store.read().unwrap();
    let lots = store.commodity_lots.get(account).cloned().unwrap_or_default();
    lots.iter()
        .map(|lot| {
            let label = lot.label.as_ref().map(|label| format!(", \"{label}\"")).unwrap_or_default();
            match (&lot.cost, &lot.acquisition_date) {
                (None, None) if label.is_empty() => format!("{} {}", lot.amount, lot.commodity),
                (None, None) => format!("{} {} {{{}}}", lot.amount, lot.commodity, label.trim_start_matches(", ")),
                (Some(cost), Some(date)) => format!("{} {} {{{cost}, {date}{label}}}", lot.amount, lot.commodity),
                (cost, date) => format!("{} {} {cost:?} {date:?}{label}", lot.amount, lot.commodity),
            }
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

/// two cost lots of `Assets:S`, bought on consecutive days
const TWO_LOTS: &str = indoc! {r#"
    2024-05-16 * "buy"
      Assets:S 10 USD { 10 CNY }
      Income:I -100 CNY
    2024-05-17 * "buy"
      Assets:S 10 USD { 11 CNY }
      Income:I -110 CNY
"#};

/// `Assets:S` opened with `booking_method: "{method}"`, holding [`TWO_LOTS`], then `sales` on 2024-05-18
fn load_two_lots(method: &str, sales: &str) -> Ledger {
    load(&formatdoc! {r#"
        1970-01-01 open Assets:S
          booking_method: "{method}"
        {TWO_LOTS}
        2024-05-18 * "sell"
        {sales}
    "#})
}

#[test]
fn e1_unsupported_booking_method_reports_error_and_falls_back() {
    // booking-split design E1, #423: these methods are not implemented. The `open` reports it once,
    // and the account books with the ledger's default method (LIFO here) instead of panicking
    for method in ["NONE", "AVERAGE", "AVERAGE_ONLY"] {
        let ledger = load(&formatdoc! {r#"
            option "default_booking_method" "LIFO"
            1970-01-01 open Assets:S
              booking_method: "{method}"
            {TWO_LOTS}
            2024-05-18 * "sell"
              Assets:S -5 USD {{}}
              Income:I
        "#});
        assert_eq!(
            error_details(&ledger),
            vec![(
                ErrorKind::UnsupportedBookingMethod,
                "1970-01-01 open Assets:S".to_owned(),
                metas([("account_name", "Assets:S"), ("booking_method", method)])
            )],
            "{method}"
        );
        assert_eq!(
            lots(&ledger, "Assets:S"),
            vec!["10 USD {10 CNY, 2024-05-16}", "5 USD {11 CNY, 2024-05-17}"],
            "{method}"
        );
    }
}

#[test]
fn e7_invalid_booking_method_reports_error_and_falls_back() {
    // booking-split design E7, #423: the load no longer aborts. The `open` reports the invalid meta,
    // and the account books with the ledger's default method (FIFO)
    let ledger = load(&formatdoc! {r#"
        1970-01-01 open Assets:S
          booking_method: "NON_EXIST"
        {TWO_LOTS}
        2024-05-18 * "sell"
          Assets:S -5 USD {{}}
          Income:I
        2024-05-18 * "plain"
          Assets:S 10 CNY
          Income:I -10 CNY
    "#});
    assert_eq!(
        error_details(&ledger),
        vec![(
            ErrorKind::ParseInvalidMeta,
            "1970-01-01 open Assets:S".to_owned(),
            metas([("account_name", "Assets:S"), ("booking_method", "NON_EXIST")])
        )]
    );
    assert_eq!(
        lots(&ledger, "Assets:S"),
        vec!["5 USD {10 CNY, 2024-05-16}", "10 USD {11 CNY, 2024-05-17}", "10 CNY"]
    );
}

#[test]
fn invalid_or_unsupported_default_booking_method_option_reports_error_and_falls_back_to_fifo() {
    for (value, kind) in [("NON_EXIST", ErrorKind::ParseInvalidMeta), ("AVERAGE", ErrorKind::UnsupportedBookingMethod)] {
        let ledger = load(&formatdoc! {r#"
            option "default_booking_method" "{value}"
            1970-01-01 open Assets:S
            {TWO_LOTS}
            2024-05-18 * "sell"
              Assets:S -5 USD {{}}
              Income:I
        "#});
        assert_eq!(
            error_details(&ledger),
            vec![(
                kind,
                format!(r#"option "default_booking_method" "{value}""#),
                metas([("booking_method", value)])
            )],
            "{value}"
        );
        assert_eq!(
            lots(&ledger, "Assets:S"),
            vec!["5 USD {10 CNY, 2024-05-16}", "10 USD {11 CNY, 2024-05-17}"],
            "{value}"
        );
        assert_eq!(ledger.store.read().unwrap().options["default_booking_method"], "FIFO", "{value}");
    }
}

#[test]
fn strict_reduction_matching_one_lot_reduces_it() {
    let ledger = load_two_lots("STRICT", "  Assets:S -5 USD { 10 CNY, 2024-05-16 }\n  Income:I");
    assert_eq!(errors(&ledger), vec![]);
    assert_eq!(lots(&ledger, "Assets:S"), vec!["5 USD {10 CNY, 2024-05-16}", "10 USD {11 CNY, 2024-05-17}"]);
}

#[test]
fn strict_empty_cost_reduction_with_a_single_cost_lot_reduces_it() {
    let ledger = load(&formatdoc! {r#"
        1970-01-01 open Assets:A
          booking_method: "STRICT"
        {BUY_10_AT_10}
        2024-05-18 * "sell"
          Assets:A -5 USD {{}}
          Income:I
    "#});
    assert_eq!(errors(&ledger), vec![]);
    assert_eq!(lots(&ledger, "Assets:A"), vec!["5 USD {10 CNY, 2024-05-16}"]);
}

#[test]
fn strict_ambiguous_reduction_reports_error_and_books_like_fifo() {
    let ledger = load_two_lots("STRICT", "  Assets:S -5 USD {}\n  Income:I");
    assert_eq!(
        error_details(&ledger),
        vec![(
            ErrorKind::AmbiguousLotMatch,
            r#"2024-05-18 * "sell""#.to_owned(),
            metas([
                ("account_name", "Assets:S"),
                ("matched_lots", "10 USD {10 CNY, 2024-05-16}, 10 USD {11 CNY, 2024-05-17}"),
                ("transaction_amount", "-5"),
            ])
        )]
    );
    assert_eq!(lots(&ledger, "Assets:S"), vec!["5 USD {10 CNY, 2024-05-16}", "10 USD {11 CNY, 2024-05-17}"]);
}

#[test]
fn strict_reduction_of_every_matching_lot_in_full_is_not_ambiguous() {
    // beancount's exception to STRICT: the total of all matching lots
    let ledger = load_two_lots("STRICT", "  Assets:S -20 USD {}\n  Income:I");
    assert_eq!(errors(&ledger), vec![]);
    assert_eq!(lots(&ledger, "Assets:S"), Vec::<String>::new());
}

#[test]
fn strict_insufficient_single_match_reports_no_enough_lot() {
    let ledger = load(&formatdoc! {r#"
        1970-01-01 open Assets:A
          booking_method: "STRICT"
        {BUY_10_AT_10}
        2024-05-16 * "sell more than held"
          Assets:A -15 USD {{ 10 CNY }}
          Income:I 150 CNY
    "#});
    assert_eq!(errors(&ledger), vec![(ErrorKind::NoEnoughCommodityLot, Some("-15".to_owned()))]);
    assert_eq!(lots(&ledger, "Assets:A"), vec!["-5 USD {10 CNY, 2024-05-16}"]);
}

#[test]
fn strict_reduction_beyond_several_matches_is_ambiguous_and_insufficient() {
    // like beancount, more than the total of several matching lots is ambiguous; booking it like
    // FIFO then runs out of lots. The unresolved remainder rejects the entire transaction.
    let ledger = load_two_lots("STRICT", "  Assets:S -25 USD {}\n  Income:I 210 CNY");
    assert_eq!(
        errors(&ledger),
        vec![
            (ErrorKind::AmbiguousLotMatch, Some("-25".to_owned())),
            (ErrorKind::NoEnoughCommodityLot, Some("-25".to_owned())),
            (ErrorKind::TransactionCannotInferTradeAmount, None),
        ]
    );
    assert_eq!(lots(&ledger, "Assets:S"), vec!["10 USD {10 CNY, 2024-05-16}", "10 USD {11 CNY, 2024-05-17}"]);
}

#[test]
fn insufficient_empty_cost_sale_with_implicit_posting_is_rejected_after_its_booking_errors() {
    // The 5 USD no lot covers have no cost. Reject the transaction and retain the lots;
    // report the booking problems before the failed inference.
    for (method, booking_errors) in [
        ("FIFO", vec![(ErrorKind::NoEnoughCommodityLot, Some("-25".to_owned()))]),
        (
            "STRICT",
            vec![
                (ErrorKind::AmbiguousLotMatch, Some("-25".to_owned())),
                (ErrorKind::NoEnoughCommodityLot, Some("-25".to_owned())),
            ],
        ),
    ] {
        let ledger = load_two_lots(method, "  Assets:S -25 USD {}\n  Income:I");
        let mut expected = booking_errors;
        expected.push((ErrorKind::TransactionCannotInferTradeAmount, None));
        assert_eq!(errors(&ledger), expected, "{method}");
        assert_eq!(
            lots(&ledger, "Assets:S"),
            vec!["10 USD {10 CNY, 2024-05-16}", "10 USD {11 CNY, 2024-05-17}"],
            "{method}"
        );
        assert_eq!(lots(&ledger, "Income:I"), vec!["-210 CNY"], "{method}");
        assert_eq!(ledger.store.read().unwrap().transactions.len(), 2, "{method}");
    }
}

#[test]
fn strict_augmentation_is_never_ambiguous() {
    // An augmentation infers its cost and opens a lot on its own date (E6).
    let ledger = load_two_lots("STRICT", "  Assets:S 3 USD {}\n  Income:I -30 CNY");
    assert_eq!(errors(&ledger), vec![]);
    assert_eq!(
        lots(&ledger, "Assets:S"),
        vec!["10 USD {10 CNY, 2024-05-16}", "10 USD {11 CNY, 2024-05-17}", "3 USD {10 CNY, 2024-05-18}"]
    );
}

#[test]
fn strict_as_default_booking_method() {
    let ledger = load(&formatdoc! {r#"
        option "default_booking_method" "STRICT"
        1970-01-01 open Assets:S
        {TWO_LOTS}
        2024-05-18 * "sell"
          Assets:S -5 USD {{}}
          Income:I
    "#});
    assert_eq!(errors(&ledger), vec![(ErrorKind::AmbiguousLotMatch, Some("-5".to_owned()))]);
    assert_eq!(ledger.store.read().unwrap().options["default_booking_method"], "STRICT");
}

#[test]
fn fifo_and_lifo_reductions_matching_several_lots_are_not_errors() {
    for (method, expected) in [
        ("FIFO", ["5 USD {10 CNY, 2024-05-16}", "10 USD {11 CNY, 2024-05-17}"]),
        ("LIFO", ["10 USD {10 CNY, 2024-05-16}", "5 USD {11 CNY, 2024-05-17}"]),
    ] {
        let ledger = load_two_lots(method, "  Assets:S -5 USD {}\n  Income:I");
        assert_eq!(errors(&ledger), vec![], "{method}");
        assert_eq!(lots(&ledger, "Assets:S"), expected, "{method}");
    }
}

/// two lots of `Assets:S` labelled `a` and `b`, bought on consecutive days
const TWO_LABELLED_LOTS: &str = indoc! {r#"
    2024-05-16 * "buy a"
      Assets:S 10 USD { 10 CNY, "a" }
      Income:I -100 CNY
    2024-05-17 * "buy b"
      Assets:S 10 USD { 11 CNY, "b" }
      Income:I -110 CNY
"#};

/// `Assets:S` opened with `booking_method: "{method}"`, holding [`TWO_LABELLED_LOTS`], then `sales` on 2024-05-18
fn load_two_labelled_lots(method: &str, sales: &str) -> Ledger {
    load(&formatdoc! {r#"
        1970-01-01 open Assets:S
          booking_method: "{method}"
        {TWO_LABELLED_LOTS}
        2024-05-18 * "sell"
        {sales}
    "#})
}

#[test]
fn label_reduction_reduces_the_lot_of_that_label() {
    // #498: the label selects the lot, whichever lot the booking method would take first; a label
    // matching a single lot is not ambiguous under STRICT, as in beancount
    for method in ["FIFO", "LIFO", "STRICT"] {
        let ledger = load_two_labelled_lots(method, "  Assets:S -1 USD {, \"b\"}\n  Income:I");
        assert_eq!(errors(&ledger), vec![], "{method}");
        assert_eq!(inferred(&ledger, 3), vec!["-1 USD", "11 CNY"], "{method}");
        assert_eq!(
            lots(&ledger, "Assets:S"),
            vec!["10 USD {10 CNY, 2024-05-16, \"a\"}", "9 USD {11 CNY, 2024-05-17, \"b\"}"],
            "{method}"
        );
    }
}

#[test]
fn label_reduction_with_a_cost_matches_the_lot_of_both() {
    let ledger = load_two_labelled_lots("STRICT", "  Assets:S -1 USD {11 CNY, \"b\"}\n  Income:I");
    assert_eq!(errors(&ledger), vec![]);
    assert_eq!(
        lots(&ledger, "Assets:S"),
        vec!["10 USD {10 CNY, 2024-05-16, \"a\"}", "9 USD {11 CNY, 2024-05-17, \"b\"}"]
    );

    // no lot is labelled `b` at that cost: like a sale of units not held, it opens a short lot of
    // exactly what it writes, dated by the sale, and is reported (beancount reports it too)
    let ledger = load_two_labelled_lots("FIFO", "  Assets:S -1 USD {10 CNY, \"b\"}\n  Income:I 10 CNY");
    assert_eq!(errors(&ledger), vec![(ErrorKind::NoEnoughCommodityLot, Some("-1".to_owned()))]);
    assert_eq!(
        lots(&ledger, "Assets:S"),
        vec![
            "10 USD {10 CNY, 2024-05-16, \"a\"}",
            "10 USD {11 CNY, 2024-05-17, \"b\"}",
            "-1 USD {10 CNY, 2024-05-18, \"b\"}"
        ]
    );
}

#[test]
fn unlabelled_reduction_matches_labelled_lots() {
    // a spec without a label is a wildcard on the label, like one without a date on the date
    let ledger = load_two_labelled_lots("FIFO", "  Assets:S -15 USD {}\n  Income:I");
    assert_eq!(errors(&ledger), vec![]);
    assert_eq!(inferred(&ledger, 3), vec!["-15 USD", "155 CNY"]);
    assert_eq!(lots(&ledger, "Assets:S"), vec!["5 USD {11 CNY, 2024-05-17, \"b\"}"]);

    let ledger = load_two_labelled_lots("LIFO", "  Assets:S -5 USD {10 CNY}\n  Income:I");
    assert_eq!(errors(&ledger), vec![]);
    assert_eq!(
        lots(&ledger, "Assets:S"),
        vec!["5 USD {10 CNY, 2024-05-16, \"a\"}", "10 USD {11 CNY, 2024-05-17, \"b\"}"]
    );
}

#[test]
fn augmentations_of_different_labels_open_distinct_lots() {
    // the same cost on the same day, labelled `a`, `b` and not at all: three lots. A later
    // purchase extends the lot of exactly its label
    let ledger = load(indoc! {r#"
        2024-05-16 * "buy"
          Assets:A 10 USD { 10 CNY, "a" }
          Assets:A 10 USD { 10 CNY, "b" }
          Assets:A 10 USD { 10 CNY }
          Income:I -300 CNY
        2024-05-16 * "buy more of a"
          Assets:A 3 USD { 10 CNY, "a" }
          Income:I -30 CNY
    "#});
    assert_eq!(errors(&ledger), vec![]);
    assert_eq!(
        lots(&ledger, "Assets:A"),
        vec![
            "13 USD {10 CNY, 2024-05-16, \"a\"}",
            "10 USD {10 CNY, 2024-05-16, \"b\"}",
            "10 USD {10 CNY, 2024-05-16}"
        ]
    );
}

#[test]
fn strict_label_matching_several_lots_is_ambiguous() {
    // two lots labelled `a` at different costs: the label alone does not pick one
    let ledger = load(indoc! {r#"
        1970-01-01 open Assets:S
          booking_method: "STRICT"
        2024-05-16 * "buy"
          Assets:S 10 USD { 10 CNY, "a" }
          Assets:S 10 USD { 11 CNY, "a" }
          Income:I -210 CNY
        2024-05-18 * "sell"
          Assets:S -5 USD {, "a"}
          Income:I
    "#});
    assert_eq!(errors(&ledger), vec![(ErrorKind::AmbiguousLotMatch, Some("-5".to_owned()))]);
    assert_eq!(
        lots(&ledger, "Assets:S"),
        vec!["5 USD {10 CNY, 2024-05-16, \"a\"}", "10 USD {11 CNY, 2024-05-16, \"a\"}"]
    );
}

#[test]
fn lifo_takes_the_newest_lot_whether_labelled_or_not() {
    // a labelled lot and an unlabelled one at the same cost: a reduction without a label matches
    // both, and the booking method picks among them by date, as among any lots
    let ledger = load(indoc! {r#"
        1970-01-01 open Assets:S
          booking_method: "LIFO"
        2024-05-16 * "buy"
          Assets:S 10 USD { 10 CNY, "a" }
          Income:I -100 CNY
        2024-05-17 * "buy"
          Assets:S 10 USD { 10 CNY }
          Income:I -100 CNY
        2024-05-18 * "sell"
          Assets:S -5 USD { 10 CNY }
          Income:I
    "#});
    assert_eq!(errors(&ledger), vec![]);
    assert_eq!(
        lots(&ledger, "Assets:S"),
        vec!["10 USD {10 CNY, 2024-05-16, \"a\"}", "5 USD {10 CNY, 2024-05-17}"]
    );
}

#[test]
fn a_labelled_lot_with_an_inferred_cost_is_not_the_default_lot() {
    // `{, "a"}` infers its cost and opens a labelled lot; units without a cost spec go to the
    // default lot, which carries no label
    let ledger = load(indoc! {r#"
        2024-05-16 * "buy"
          Assets:A 10 USD {, "a"}
          Income:I -10 USD
        2024-05-17 * "transfer"
          Assets:A 5 USD
          Income:I -5 USD
    "#});
    assert_eq!(errors(&ledger), vec![]);
    assert_eq!(lots(&ledger, "Assets:A"), vec!["10 USD {1 USD, 2024-05-16, \"a\"}", "5 USD"]);
}

#[test]
fn strict_ambiguous_match_lists_the_labels_of_the_lots() {
    let ledger = load(indoc! {r#"
        1970-01-01 open Assets:S
          booking_method: "STRICT"
        2024-05-16 * "buy"
          Assets:S 10 USD { 10 CNY, "a" }
          Assets:S 10 USD { 10 CNY, "b" }
          Income:I -200 CNY
        2024-05-18 * "sell"
          Assets:S -5 USD { 10 CNY }
          Income:I
    "#});
    assert_eq!(
        error_details(&ledger),
        vec![(
            ErrorKind::AmbiguousLotMatch,
            r#"2024-05-18 * "sell""#.to_owned(),
            metas([
                ("account_name", "Assets:S"),
                ("matched_lots", "10 USD {10 CNY, 2024-05-16, \"a\"}, 10 USD {10 CNY, 2024-05-16, \"b\"}"),
                ("transaction_amount", "-5"),
            ])
        )]
    );
}

#[test]
fn e2_cross_commodity_transaction_is_unbalanced() {
    // booking-split design E2, #423: each commodity balances on its own. The transaction is still
    // booked
    let ledger = load(indoc! {r#"
        2024-05-16 * "two commodities, no price"
          Assets:A 10 USD
          Income:I -10 CNY
    "#});
    assert_eq!(errors(&ledger), vec![(ErrorKind::UnbalancedTransaction, None)]);
    assert_eq!(lots(&ledger, "Assets:A"), vec!["10 USD"]);
    assert_eq!(lots(&ledger, "Income:I"), vec!["-10 CNY"]);
}

#[test]
fn e3_price_weight_is_balanced_per_commodity() {
    // booking-split design E3, #423: a posting with a price weighs its converted units
    for (postings, expected) in [
        ("Assets:A 10 USD @ 10 CNY\n  Income:I -10 CNY", vec![(ErrorKind::UnbalancedTransaction, None)]),
        ("Assets:A 10 USD @ 10 CNY\n  Income:I -100 CNY", vec![]),
        ("Assets:A 10 USD @@ 100 CNY\n  Income:I -100 CNY", vec![]),
        ("Assets:A -10 USD @@ 100 CNY\n  Income:I 100 CNY", vec![]),
        ("Assets:A 10 USD @@ 100 CNY\n  Income:I -90 CNY", vec![(ErrorKind::UnbalancedTransaction, None)]),
    ] {
        let ledger = load(&format!("2024-05-16 * \"with a price\"\n  {postings}\n"));
        assert_eq!(errors(&ledger), expected, "{postings}");
    }
}

#[test]
fn e4_implicit_posting_of_empty_cost_sale_gets_the_booked_cost() {
    // booking-split design E4, #423: the lots are matched before interpolating, so the implicit
    // posting gets the cost of the lots the sale reduces: 10 × 10 + 5 × 11 CNY with FIFO, and
    // 10 × 11 + 5 × 10 CNY with LIFO
    for (method, income, lot) in [
        ("FIFO", "155 CNY", "5 USD {11 CNY, 2024-05-17}"),
        ("LIFO", "160 CNY", "5 USD {10 CNY, 2024-05-16}"),
    ] {
        let ledger = load_two_lots(method, "  Assets:S -15 USD {}\n  Income:I");
        assert_eq!(errors(&ledger), vec![], "{method}");
        assert_eq!(inferred(&ledger, 3), vec!["-15 USD", income], "{method}");
        assert_eq!(lots(&ledger, "Assets:S"), vec![lot], "{method}");
    }

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
    assert_eq!(inferred(&ledger, 3), vec!["-15 USD", "155 CNY"]);
    assert_eq!(lots(&ledger, "Assets:A"), vec!["5 USD {11 CNY, 2024-05-17}"]);
    assert_eq!(lots(&ledger, "Income:I"), vec!["-55 CNY"]);
}

#[test]
fn empty_cost_sale_with_explicit_postings_balances_on_the_booked_cost() {
    for (postings, expected) in [
        // the booked cost: 10 × 10 + 5 × 11 CNY
        ("Income:I 155 CNY", vec![]),
        ("Income:I 150 CNY", vec![(ErrorKind::UnbalancedTransaction, None)]),
        // the price is not part of the weight; the implicit gain is cash minus booked cost
        ("Assets:Cash 180 CNY\n  Income:Gains", vec![]),
    ] {
        let ledger = load(&formatdoc! {r#"
            1970-01-01 open Assets:S
            1970-01-01 open Assets:Cash
            1970-01-01 open Income:Gains
            {TWO_LOTS}
            2024-05-18 * "sell"
              Assets:S -15 USD {{}} @ 12 CNY
              {postings}
        "#});
        assert_eq!(errors(&ledger), expected, "{postings}");
        assert_eq!(lots(&ledger, "Assets:S"), vec!["5 USD {11 CNY, 2024-05-17}"], "{postings}");
    }
    let ledger = load(&formatdoc! {r#"
        1970-01-01 open Assets:S
        1970-01-01 open Assets:Cash
        1970-01-01 open Income:Gains
        {TWO_LOTS}
        2024-05-18 * "sell"
          Assets:S -15 USD {{}} @ 12 CNY
          Assets:Cash 180 CNY
          Income:Gains
    "#});
    assert_eq!(inferred(&ledger, 3), vec!["-15 USD", "180 CNY", "-25 CNY"]);
}

#[test]
fn zero_gain_empty_cost_sale_books_a_zero_implicit_posting() {
    // a sale at cost (e.g. a constant-NAV fund): the cash equals the booked cost, 10 × 10 + 5 × 11
    // CNY, so the implicit gain books zero of the weight commodity and the journal keeps the
    // posting (beancount drops a zero auto-posting)
    let ledger = load(&formatdoc! {r#"
        1970-01-01 open Assets:S
        1970-01-01 open Assets:Cash
        1970-01-01 open Income:Gains
        {TWO_LOTS}
        2024-05-18 * "sell at cost"
          Assets:S -15 USD {{}}
          Assets:Cash 155 CNY
          Income:Gains
    "#});
    assert_eq!(errors(&ledger), vec![]);
    assert_eq!(inferred(&ledger, 3), vec!["-15 USD", "155 CNY", "0 CNY"]);
    assert_eq!(lots(&ledger, "Assets:S"), vec!["5 USD {11 CNY, 2024-05-17}"]);
    assert_eq!(lots(&ledger, "Assets:Cash"), vec!["155 CNY"]);
    assert_eq!(lots(&ledger, "Income:Gains"), Vec::<String>::new());
    assert_eq!(ledger.store.read().unwrap().transactions.len(), 3);
}

#[test]
fn implicit_posting_of_an_already_balanced_transaction_books_zero() {
    let ledger = load(indoc! {r#"
        1970-01-01 open Assets:B
        2024-05-16 * "balanced without the implicit posting"
          Assets:A 10 CNY
          Income:I -10 CNY
          Assets:B
        2024-05-17 * "a single zero posting"
          Assets:A 0 USD
          Assets:B
    "#});
    assert_eq!(errors(&ledger), vec![]);
    assert_eq!(inferred(&ledger, 1), vec!["10 CNY", "-10 CNY", "0 CNY"]);
    assert_eq!(inferred(&ledger, 2), vec!["0 USD", "0 USD"]);
}

#[test]
fn implicit_posting_of_a_transaction_balanced_in_several_commodities_cannot_be_inferred() {
    // no single commodity to give the implicit posting: the transaction is rejected, as is one with
    // nothing to infer from
    let ledger = load(indoc! {r#"
        1970-01-01 open Assets:B
        2024-05-16 * "balanced in two commodities"
          Assets:A 10 CNY
          Income:I -10 CNY
          Assets:A 5 USD
          Income:I -5 USD
          Assets:B
        2024-05-17 * "nothing but the implicit posting"
          Assets:B
    "#});
    assert_eq!(
        errors(&ledger),
        vec![
            (ErrorKind::TransactionCannotInferTradeAmount, None),
            (ErrorKind::TransactionCannotInferTradeAmount, None),
        ]
    );
    assert_eq!(ledger.store.read().unwrap().transactions.len(), 0);
    assert_eq!(lots(&ledger, "Assets:A"), Vec::<String>::new());
}

/// 3 AAPL bought for a total cost of 1000 USD: the lot's per-unit cost is 333.33… USD
const AAPL_TOTAL_COST: &str = indoc! {r#"
    1970-01-01 commodity EUR
    1970-01-01 open Assets:Cash
    2024-05-16 * "buy at a total cost"
      Assets:A 3 AAPL {{1000 USD}}
      Assets:Cash -1000 USD
"#};

#[test]
fn interpolated_units_drop_the_dust_of_a_total_cost_lot() {
    // selling the 3 AAPL at `{}` weighs 3 × 333.33… USD, which is 1000 USD less some dust; the
    // implicit posting books exactly 1000 USD (rounded at the USD precision, 2: nothing is written
    // in USD in the sale), and a balance assertion of 0 USD holds
    let ledger = load(&format!(
        "{AAPL_TOTAL_COST}{}",
        indoc! {r#"
            2024-05-17 * "sell, implicit cash"
              Assets:A -3 AAPL {}
              Assets:Cash
            2024-05-18 balance Assets:Cash 0 USD
        "#}
    ));
    assert_eq!(errors(&ledger), vec![]);
    assert_eq!(inferred(&ledger, 2), vec!["-3 AAPL", "1000 USD"]);
    assert_eq!(lots(&ledger, "Assets:Cash"), Vec::<String>::new());
    assert_eq!(lots(&ledger, "Assets:A"), Vec::<String>::new());

    // the gain is exactly 1000 - 1200 USD
    let ledger = load(&format!(
        "{AAPL_TOTAL_COST}{}",
        indoc! {r#"
            2024-05-17 * "sell, implicit gain"
              Assets:A -3 AAPL {}
              Assets:Cash 1200 USD
              Income:I
        "#}
    ));
    assert_eq!(errors(&ledger), vec![]);
    assert_eq!(inferred(&ledger, 2), vec!["-3 AAPL", "1200 USD", "-200 USD"]);

    // the USD dust rounds to zero, so EUR is the one commodity left to interpolate
    let ledger = load(&format!(
        "{AAPL_TOTAL_COST}{}",
        indoc! {r#"
            2024-05-17 * "sell, and a second commodity"
              Assets:A -3 AAPL {}
              Assets:Cash 1000 USD
              Assets:Cash 10 EUR
              Income:I
        "#}
    ));
    assert_eq!(errors(&ledger), vec![]);
    assert_eq!(inferred(&ledger, 2), vec!["-3 AAPL", "1000 USD", "10 EUR", "-10 EUR"]);
    assert_eq!(lots(&ledger, "Assets:Cash"), vec!["10 EUR"]);

    // a partial reduction carries the dust too: 333.33… USD rounds at the USD precision (2, round
    // half down) to 333.33 USD, and the 2 shares left to 666.666… = 666.67 USD, 1000 USD in all
    let ledger = load(&format!(
        "{AAPL_TOTAL_COST}{}",
        indoc! {r#"
            2024-05-17 * "sell one share, implicit cash"
              Assets:A -1 AAPL {}
              Assets:Cash
            2024-05-18 * "sell the other two, implicit cash"
              Assets:A -2 AAPL {}
              Assets:Cash
            2024-05-19 balance Assets:Cash 0 USD
        "#}
    ));
    assert_eq!(errors(&ledger), vec![]);
    assert_eq!(inferred(&ledger, 2), vec!["-1 AAPL", "333.33 USD"]);
    assert_eq!(inferred(&ledger, 3), vec!["-2 AAPL", "666.67 USD"]);
    assert_eq!(lots(&ledger, "Assets:Cash"), Vec::<String>::new());
}

#[test]
fn interpolated_units_keep_the_decimals_written_in_the_transaction() {
    // amounts from written numbers, their products included, are exact: nothing is rounded away,
    // whatever the commodity's precision (2 by default)
    for (header, postings, expected) in [
        ("", "Assets:A 10.50 CNY\n  Income:I", "-10.50 CNY"),
        ("1970-01-01 commodity BTC\n", "Assets:A 0.00123 BTC\n  Income:I", "-0.00123 BTC"),
        ("", "Assets:A 10.004 CNY\n  Income:I -10 CNY\n  Assets:Fee", "-0.004 CNY"),
        // the price counts for CNY: 100 × 7.12345 is exactly 712.345 (as 712.34500)
        ("", "Assets:A 100 USD @ 7.12345 CNY\n  Income:I", "-712.34500 CNY"),
        // a product finer than every written number is still exact: 1.5 × 3.333 = 4.9995
        ("", "Assets:A 1.5 USD @ 3.333 CNY\n  Income:I", "-4.9995 CNY"),
        // 8 decimals × 8 decimals keeps all 16: 0.12345678 × 0.87654321
        (
            "1970-01-01 commodity BTC\n1970-01-01 commodity ETH\n",
            "Assets:A 0.12345678 BTC @ 0.87654321 ETH\n  Income:I",
            "-0.1082152022374638 ETH",
        ),
        // an undefined commodity keeps its written decimals
        ("", "Assets:A 0.123 JPY\n  Income:I", "-0.123 JPY"),
    ] {
        let ledger = load(&format!("{header}2024-05-16 * \"scale\"\n  {postings}\n"));
        assert_eq!(inferred(&ledger, 1).last().unwrap(), expected, "{postings}");
    }

    // four fuel receipts of 1.004 CNY add up to exactly 4.016 CNY
    let ledger = load(&"2024-05-16 * \"fuel\"\n  Income:I 1.004 CNY\n  Assets:A\n".repeat(4));
    assert_eq!(errors(&ledger), vec![]);
    assert_eq!(lots(&ledger, "Assets:A"), vec!["-4.016 CNY"]);
    assert_eq!(lots(&ledger, "Income:I"), vec!["4.016 CNY"]);
}

#[test]
fn pad_transaction_stays_zero_sum_at_any_scale() {
    // the pad pads 100.0001 - 10.005 = 89.9951 CNY, finer than the CNY precision; its implicit leg
    // takes exactly that, so the pad source mirrors the padded account
    let ledger = load(indoc! {r#"
        1970-01-01 open Equity:Open
        2023-01-01 * "finer than the precision"
          Assets:A 10.005 CNY
          Equity:Open
        2023-01-02 balance Assets:A 100.0001 CNY with pad Equity:Open
        2023-01-03 balance Assets:A 100.0001 CNY
        2023-01-03 balance Equity:Open -100.0001 CNY
    "#});
    assert_eq!(errors(&ledger), vec![]);
    assert_eq!(inferred(&ledger, 1), vec!["10.005 CNY", "-10.005 CNY"]);
    assert_eq!(inferred(&ledger, 2), vec!["89.9951 CNY", "-89.9951 CNY"]);
    assert_eq!(lots(&ledger, "Equity:Open"), vec!["-100.0001 CNY"]);
}

#[test]
fn zero_units_with_a_total_cost_or_price_weigh_nothing() {
    // beancount's per-unit price of zero units is zero, so `0 USD @@ 100 CNY` weighs nothing, and
    // so does a zero-unit total cost; neither crashes. The written total does not silently vanish:
    // an explicit counter-leg is unbalanced
    for (postings, expected_errors, expected_inferred) in [
        (
            "Assets:A 0 USD @@ 100 CNY\n  Income:I -100 CNY",
            vec![(ErrorKind::UnbalancedTransaction, None)],
            "-100 CNY",
        ),
        ("Assets:A 0 USD @@ 100 CNY\n  Income:I", vec![], "0 CNY"),
        (
            "Assets:A 0 USD {{100 CNY}}\n  Income:I -100 CNY",
            vec![(ErrorKind::UnbalancedTransaction, None)],
            "-100 CNY",
        ),
        ("Assets:A 0 USD {{100 CNY}}\n  Income:I", vec![], "0 CNY"),
    ] {
        let ledger = load(&format!("2024-05-16 * \"zero units\"\n  {postings}\n"));
        assert_eq!(errors(&ledger), expected_errors, "{postings}");
        assert_eq!(inferred(&ledger, 1), vec!["0 USD", expected_inferred], "{postings}");
        assert_eq!(lots(&ledger, "Assets:A"), Vec::<String>::new(), "{postings}");
    }
}

#[test]
fn transaction_residual_rounds_at_the_commodity_precision() {
    // CNY has the default precision 2 and rounding RoundDown (half down); EUR rounds half up
    for (postings, expected) in [
        ("Assets:A 10.005 CNY\n  Income:I -10 CNY", vec![]),
        ("Assets:A 10.006 CNY\n  Income:I -10 CNY", vec![(ErrorKind::UnbalancedTransaction, None)]),
        ("Assets:A 10.004 EUR\n  Income:I -10 EUR", vec![]),
        ("Assets:A 10.005 EUR\n  Income:I -10 EUR", vec![(ErrorKind::UnbalancedTransaction, None)]),
    ] {
        let ledger = load(&format!(
            "1970-01-01 commodity EUR\n  rounding: \"RoundUp\"\n2024-05-16 * \"rounding\"\n  {postings}\n"
        ));
        assert_eq!(errors(&ledger), expected, "{postings}");
    }

    // the booked weight of an empty cost sale rounds the same way: 3 × 3.333 CNY against 10 CNY
    let ledger = load(indoc! {r#"
        2024-05-16 * "buy"
          Assets:A 3 USD { 3.333 CNY }
          Income:I -9.999 CNY
        2024-05-17 * "sell"
          Assets:A -3 USD {}
          Income:I 10 CNY
    "#});
    assert_eq!(errors(&ledger), vec![]);
}

#[test]
fn undefined_commodity_is_reported_before_unbalanced() {
    // #441: a transaction with an undefined weight commodity and an unbalanced one reports
    // `CommodityDoesNotDefine`, whatever the order of its commodities, on every load
    for postings in ["Assets:A 10 JPY\n  Income:I -9 CNY", "Assets:A 10 CNY\n  Income:I -9 JPY"] {
        for _ in 0..8 {
            let ledger = load(&format!("2024-05-16 * \"two problems\"\n  {postings}\n"));
            assert_eq!(errors(&ledger), vec![(ErrorKind::CommodityDoesNotDefine, None)], "{postings}");
        }
    }
}

#[test]
fn balance_stages_count_the_booked_amount_of_an_implicit_posting() {
    // pad and balance check size the account like final validation, with the same booking method:
    // the implicit income of the empty cost sale is 155 CNY with FIFO and 160 CNY with LIFO, so
    // Income:I holds -55 or -50 CNY before the pad
    for (option, meta, held) in [
        ("", "", "-55"),
        ("", "  booking_method: \"LIFO\"", "-50"),
        ("option \"default_booking_method\" \"LIFO\"", "", "-50"),
    ] {
        let ledger = load(&formatdoc! {r#"
            {option}
            1970-01-01 open Assets:S
            {meta}
            1970-01-01 open Equity:Open
            {TWO_LOTS}
            2024-05-18 * "sell"
              Assets:S -15 USD {{}}
              Income:I
            2024-05-19 balance Income:I {held} CNY
            2024-05-20 balance Income:I -40 CNY with pad Equity:Open
            2024-05-21 balance Income:I -40 CNY
        "#});
        assert_eq!(errors(&ledger), vec![], "{option}{meta}");
        assert_eq!(lots(&ledger, "Income:I"), vec!["-40 CNY"], "{option}{meta}");
    }
}

#[test]
fn e5_undated_cost_reduces_lots_of_any_date() {
    // booking-split design E5, #423: like beancount, a missing cost date is a wildcard, so the sale
    // reduces the lot bought the day before
    let ledger = load(&format!(
        "{BUY_10_AT_10}{}",
        indoc! {r#"
            2024-05-17 * "sell with an undated cost, next day"
              Assets:A -5 USD { 10 CNY }
              Income:I 50 CNY
        "#}
    ));
    assert_eq!(errors(&ledger), vec![]);
    assert_eq!(lots(&ledger, "Assets:A"), vec!["5 USD {10 CNY, 2024-05-16}"]);
}

#[test]
fn e5_dated_cost_only_reduces_the_lot_of_that_date() {
    // a given date is still a criterion: no lot was acquired on 2024-05-17, so nothing matches
    let ledger = load(&format!(
        "{BUY_10_AT_10}{}",
        indoc! {r#"
            2024-05-17 * "sell with a cost date no lot has"
              Assets:A -5 USD { 10 CNY, 2024-05-17 }
              Income:I 50 CNY
        "#}
    ));
    assert_eq!(errors(&ledger), vec![(ErrorKind::NoEnoughCommodityLot, Some("-5".to_owned()))]);
    assert_eq!(lots(&ledger, "Assets:A"), vec!["10 USD {10 CNY, 2024-05-16}", "-5 USD {10 CNY, 2024-05-17}"]);

    // also without a cost number: `{, date}` reduces the lot of that date, not the first one
    let ledger = load_two_lots("FIFO", "  Assets:S -5 USD {, 2024-05-17}\n  Income:I");
    assert_eq!(errors(&ledger), vec![]);
    assert_eq!(inferred(&ledger, 3), vec!["-5 USD", "55 CNY"]);
    assert_eq!(lots(&ledger, "Assets:S"), vec!["10 USD {10 CNY, 2024-05-16}", "5 USD {11 CNY, 2024-05-17}"]);
}

/// two lots of `Assets:S` at the same cost, bought on consecutive days
const TWO_SAME_COST_LOTS: &str = indoc! {r#"
    2024-05-16 * "buy"
      Assets:S 10 USD { 10 CNY }
      Income:I -100 CNY
    2024-05-17 * "buy again at the same cost"
      Assets:S 10 USD { 10 CNY }
      Income:I -100 CNY
"#};

/// `Assets:S` opened with `booking_method: "{method}"`, holding [`TWO_SAME_COST_LOTS`], then
/// `sales` on 2024-05-18
fn load_two_same_cost_lots(method: &str, sales: &str) -> Ledger {
    load(&formatdoc! {r#"
        1970-01-01 open Assets:S
          booking_method: "{method}"
        {TWO_SAME_COST_LOTS}
        2024-05-18 * "sell"
        {sales}
    "#})
}

#[test]
fn e5_undated_cost_augmentation_opens_a_lot_of_the_txn_date() {
    // an augmentation names the lot it adds to: the second buy keeps its own lot, dated by its
    // transaction, instead of joining the first one
    let ledger = load(&format!("1970-01-01 open Assets:S\n{TWO_SAME_COST_LOTS}"));
    assert_eq!(errors(&ledger), vec![]);
    assert_eq!(lots(&ledger, "Assets:S"), vec!["10 USD {10 CNY, 2024-05-16}", "10 USD {10 CNY, 2024-05-17}"]);
}

#[test]
fn e5_undated_cost_reduction_spans_same_cost_lots_by_date() {
    for (method, expected) in [("FIFO", "5 USD {10 CNY, 2024-05-17}"), ("LIFO", "5 USD {10 CNY, 2024-05-16}")] {
        let ledger = load_two_same_cost_lots(method, "  Assets:S -15 USD { 10 CNY }\n  Income:I");
        assert_eq!(errors(&ledger), vec![], "{method}");
        assert_eq!(inferred(&ledger, 3), vec!["-15 USD", "150 CNY"], "{method}");
        assert_eq!(lots(&ledger, "Assets:S"), vec![expected], "{method}");
    }

    // more than both lots: the part no lot covers is a short lot of the sale's date, as before
    let ledger = load_two_same_cost_lots("FIFO", "  Assets:S -25 USD { 10 CNY }\n  Income:I");
    assert_eq!(errors(&ledger), vec![(ErrorKind::NoEnoughCommodityLot, Some("-25".to_owned()))]);
    assert_eq!(lots(&ledger, "Assets:S"), vec!["-5 USD {10 CNY, 2024-05-18}"]);
}

#[test]
fn e5_strict_undated_cost_matching_lots_of_several_dates_is_ambiguous() {
    // beancount's STRICT: the sale matches both 10 CNY lots, so reducing only one of them is
    // ambiguous. It is still booked like FIFO
    let ledger = load_two_same_cost_lots("STRICT", "  Assets:S -5 USD { 10 CNY }\n  Income:I");
    assert_eq!(
        error_details(&ledger),
        vec![(
            ErrorKind::AmbiguousLotMatch,
            r#"2024-05-18 * "sell""#.to_owned(),
            metas([
                ("account_name", "Assets:S"),
                ("matched_lots", "10 USD {10 CNY, 2024-05-16}, 10 USD {10 CNY, 2024-05-17}"),
                ("transaction_amount", "-5"),
            ])
        )]
    );
    assert_eq!(lots(&ledger, "Assets:S"), vec!["5 USD {10 CNY, 2024-05-16}", "10 USD {10 CNY, 2024-05-17}"]);

    // a date names a single lot, and reducing every matching lot in full is not ambiguous
    let ledger = load_two_same_cost_lots("STRICT", "  Assets:S -5 USD { 10 CNY, 2024-05-17 }\n  Income:I");
    assert_eq!(errors(&ledger), vec![]);
    assert_eq!(lots(&ledger, "Assets:S"), vec!["10 USD {10 CNY, 2024-05-16}", "5 USD {10 CNY, 2024-05-17}"]);
    let ledger = load_two_same_cost_lots("STRICT", "  Assets:S -20 USD { 10 CNY }\n  Income:I");
    assert_eq!(errors(&ledger), vec![]);
    assert_eq!(lots(&ledger, "Assets:S"), Vec::<String>::new());
}

#[test]
fn e6_empty_cost_augmentation_infers_a_new_lot_or_rejects_ambiguous_input() {
    for (other, cost) in [("Income:I -30 CNY", Some("10 CNY")), ("Income:I -3 USD", Some("1 USD")), ("Income:I", None)] {
        let ledger = load(&formatdoc! {r#"
            {BUY_10_AT_10}
            2024-05-17 * "buy with an empty cost"
              Assets:A 3 USD {{}}
              {other}
        "#});
        if let Some(cost) = cost {
            assert_eq!(errors(&ledger), vec![], "{other}");
            assert_eq!(
                lots(&ledger, "Assets:A"),
                vec!["10 USD {10 CNY, 2024-05-16}".to_owned(), format!("3 USD {{{cost}, 2024-05-17}}")],
                "{other}"
            );
            assert_eq!(inferred(&ledger, 2), vec!["3 USD", other.strip_prefix("Income:I ").unwrap()], "{other}");
        } else {
            assert_eq!(errors(&ledger), vec![(ErrorKind::TransactionCannotInferTradeAmount, None)]);
            assert_eq!(lots(&ledger, "Assets:A"), vec!["10 USD {10 CNY, 2024-05-16}"]);
            assert!(inferred(&ledger, 2).is_empty());
        }
    }
}

#[test]
fn e8_a_failing_balance_check_leaves_the_lots_alone() {
    // booking-split design E8, #423: a balance check books nothing, as in beancount, so a failing
    // one on a cost account is an error and does not add a correcting lot
    let ledger = load(&format!("{BUY_10_AT_10}2024-05-17 balance Assets:A 8 USD\n"));
    assert_eq!(errors(&ledger), vec![(ErrorKind::AccountBalanceCheckError, None)]);
    assert_eq!(lots(&ledger, "Assets:A"), vec!["10 USD {10 CNY, 2024-05-16}"]);
}

#[test]
fn e9_empty_cost_reduction_without_cost_lots_leaves_the_holdings_unchanged() {
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
    assert_eq!(
        errors(&ledger),
        vec![
            (ErrorKind::NoEnoughCommodityLot, Some("-3".to_owned())),
            (ErrorKind::TransactionCannotInferTradeAmount, None)
        ]
    );
    assert_eq!(lots(&ledger, "Assets:A"), vec!["6 USD"]);
}

#[test]
fn an_unmatched_empty_cost_rolls_back_every_posting_and_is_ignored_by_balance_stages() {
    let ledger = load(&formatdoc! {r#"
        {BUY_10_AT_10}
        2024-05-17 * "cash first, then an insufficient sale"
          Income:I 150 CNY
          Assets:A -15 USD {{}}
        2024-05-18 balance Assets:A 10 USD
        2024-05-19 * "a valid transaction after the rejected sale"
          Assets:A 1 USD {{10 CNY}}
          Income:I -10 CNY
        2024-05-20 balance Assets:A 11 USD
    "#});
    assert_eq!(
        errors(&ledger),
        vec![
            (ErrorKind::NoEnoughCommodityLot, Some("-15".to_owned())),
            (ErrorKind::TransactionCannotInferTradeAmount, None)
        ]
    );
    assert_eq!(lots(&ledger, "Assets:A"), vec!["10 USD {10 CNY, 2024-05-16}", "1 USD {10 CNY, 2024-05-19}"]);
    assert_eq!(lots(&ledger, "Income:I"), vec!["-110 CNY"]);
}

#[test]
fn an_empty_cost_reduction_can_use_a_lot_bought_earlier_in_the_same_transaction() {
    let ledger = load(indoc! {r#"
        2024-05-16 * "buy and reduce"
          Assets:A 2 USD {10 CNY}
          Assets:A -1 USD {}
          Income:I -10 CNY
    "#});
    assert_eq!(errors(&ledger), vec![]);
    assert_eq!(lots(&ledger, "Assets:A"), vec!["1 USD {10 CNY, 2024-05-16}"]);
}

#[test]
fn a_sale_then_an_empty_cost_augmentation_infers_the_new_lot_in_written_order() {
    let ledger = load(&formatdoc! {r#"
        {BUY_10_AT_10}
        2024-05-17 * "sell all then buy"
          Assets:A -10 USD {{}}
          Assets:A 3 USD {{}}
          Income:I 70 CNY
    "#});
    assert_eq!(errors(&ledger), vec![]);
    assert_eq!(lots(&ledger, "Assets:A"), vec!["3 USD {10 CNY, 2024-05-17}"]);
    assert_eq!(lots(&ledger, "Income:I"), vec!["-30 CNY"]);
}

#[test]
fn a_missing_cost_uses_the_counterposting_not_the_written_price() {
    let ledger = load(indoc! {r#"
        2024-05-16 * "infer a labelled total cost"
          Assets:A 3 USD {{, "a"}} @ 100 CNY
          Income:I -30 CNY
        2024-05-17 * "sell that lot"
          Assets:A -1 USD {, "a"}
          Income:I
    "#});
    assert_eq!(errors(&ledger), vec![]);
    assert_eq!(lots(&ledger, "Assets:A"), vec!["2 USD {10 CNY, 2024-05-16, \"a\"}"]);
    assert_eq!(inferred(&ledger, 2), vec!["-1 USD", "10 CNY"]);
}

#[test]
fn missing_costs_that_are_not_uniquely_determined_leave_no_lots_or_store_rows() {
    for (body, kind) in [
        ("Assets:A 3 USD {}\n  Income:I", ErrorKind::TransactionCannotInferTradeAmount),
        (
            "Assets:A 3 USD {}\n  Assets:A 2 USD {}\n  Income:I -50 CNY",
            ErrorKind::TransactionCannotInferTradeAmount,
        ),
        ("Assets:A 0 USD {}\n  Income:I 0 CNY", ErrorKind::TransactionCannotInferTradeAmount),
        ("Assets:A 3 USD {}\n  Income:I 30 CNY", ErrorKind::TransactionCannotInferTradeAmount),
        (
            "Assets:A 3 USD {}\n  Income:I -20 CNY\n  Income:I -1 USD",
            ErrorKind::TransactionExplicitPostingHaveMultipleCommodity,
        ),
    ] {
        let ledger = load(&format!("2024-05-16 * \"undetermined cost\"\n  {body}\n"));
        assert_eq!(errors(&ledger), vec![(kind, None)], "{body}");
        assert!(lots(&ledger, "Assets:A").is_empty(), "{body}");
        assert!(lots(&ledger, "Income:I").is_empty(), "{body}");
        assert!(ledger.store.read().unwrap().transactions.is_empty(), "{body}");
    }
}

/// `Assets:S` opened with `booking_method: "{method}"`, holding a lot acquired on 2024-05-10, then
/// an older one transferred in, then `sales` on 2024-05-18
fn load_transferred_in_lot(method: &str, sales: &str) -> Ledger {
    load(&formatdoc! {r#"
        1970-01-01 open Assets:S
          booking_method: "{method}"
        2024-05-16 * "buy, later acquisition date first"
          Assets:S 10 USD {{ 10 CNY, 2024-05-10 }}
          Income:I -100 CNY
        2024-05-17 * "transfer in, earlier acquisition date second"
          Assets:S 10 USD {{ 11 CNY, 2024-01-01 }}
          Income:I -110 CNY
        2024-05-18 * "sell"
        {sales}
    "#})
}

#[test]
fn e10_fifo_and_lifo_pick_lots_by_acquisition_date() {
    // booking-split design E10, #423: like beancount, FIFO takes the oldest acquisition date first
    // and LIFO the newest, whatever order the lots were created in. The lots stay in creation order
    for (method, income, expected) in [
        ("FIFO", "55 CNY", ["10 USD {10 CNY, 2024-05-10}", "5 USD {11 CNY, 2024-01-01}"]),
        ("LIFO", "50 CNY", ["5 USD {10 CNY, 2024-05-10}", "10 USD {11 CNY, 2024-01-01}"]),
    ] {
        let ledger = load_transferred_in_lot(method, "  Assets:S -5 USD {}\n  Income:I");
        assert_eq!(errors(&ledger), vec![], "{method}");
        assert_eq!(inferred(&ledger, 3), vec!["-5 USD", income], "{method}");
        assert_eq!(lots(&ledger, "Assets:S"), expected, "{method}");
    }

    // a sale across both lots takes the older one in full first: 10 × 11 + 5 × 10 CNY
    let ledger = load_transferred_in_lot("FIFO", "  Assets:S -15 USD {}\n  Income:I");
    assert_eq!(errors(&ledger), vec![]);
    assert_eq!(inferred(&ledger, 3), vec!["-15 USD", "160 CNY"]);
    assert_eq!(lots(&ledger, "Assets:S"), vec!["5 USD {10 CNY, 2024-05-10}"]);

    // STRICT books an ambiguous reduction like FIFO, by date too
    let ledger = load_transferred_in_lot("STRICT", "  Assets:S -5 USD {}\n  Income:I");
    assert_eq!(errors(&ledger), vec![(ErrorKind::AmbiguousLotMatch, Some("-5".to_owned()))]);
    assert_eq!(lots(&ledger, "Assets:S"), vec!["10 USD {10 CNY, 2024-05-10}", "5 USD {11 CNY, 2024-01-01}"]);
}

#[test]
fn e10_lots_of_the_same_date_go_in_creation_order() {
    // two lots acquired the same day: FIFO takes the one created first, and LIFO, its exact
    // reverse, the one created last (beancount's LIFO keeps creation order among them instead)
    for (method, income, expected) in [
        ("FIFO", "50 CNY", ["5 USD {10 CNY, 2024-05-16}", "10 USD {11 CNY, 2024-05-16}"]),
        ("LIFO", "55 CNY", ["10 USD {10 CNY, 2024-05-16}", "5 USD {11 CNY, 2024-05-16}"]),
    ] {
        let ledger = load(&formatdoc! {r#"
            1970-01-01 open Assets:S
              booking_method: "{method}"
            2024-05-16 * "two lots, the same day"
              Assets:S 10 USD {{ 10 CNY }}
              Assets:S 10 USD {{ 11 CNY }}
              Income:I -210 CNY
            2024-05-18 * "sell"
              Assets:S -5 USD {{}}
              Income:I
        "#});
        assert_eq!(errors(&ledger), vec![], "{method}");
        assert_eq!(inferred(&ledger, 2), vec!["-5 USD", income], "{method}");
        assert_eq!(lots(&ledger, "Assets:S"), expected, "{method}");
    }
}

#[test]
fn e11_postings_to_missing_or_closed_accounts_are_reported() {
    // one error per account on the transaction, which is still booked
    let ledger = load(indoc! {r#"
        1970-01-01 open Assets:B
        1970-01-02 close Assets:B
        2024-05-16 * "to a missing and a closed account"
          Assets:Missing 10 CNY
          Assets:B -10 CNY
    "#});
    let span = r#"2024-05-16 * "to a missing and a closed account""#;
    assert_eq!(
        error_details(&ledger),
        vec![
            (ErrorKind::AccountDoesNotExist, span.to_owned(), metas([("account_name", "Assets:Missing")])),
            (ErrorKind::AccountClosed, span.to_owned(), metas([("account_name", "Assets:B")])),
        ]
    );
    assert_eq!(ledger.store.read().unwrap().transactions.len(), 1);
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
    let expense = |account: &str| {
        store
            .postings
            .iter()
            .find(|posting| posting.account.name() == account)
            .unwrap()
            .inferred_amount
            .to_string()
    };
    assert_eq!(expense("Expenses:Food"), "50 CNY");
    // booking-split design E4, #423: the implicit posting gets the booked cost, 5 × 10 CNY
    assert_eq!(expense("Expenses:Fun"), "50 CNY");
}

/// A split a stage broke apart: the legs of one written posting are no longer adjacent (a
/// plugin moved another posting between them). The store shows them as booked, one row per leg,
/// as the exporter does, instead of restoring the written posting twice.
#[test]
fn legs_a_stage_moved_apart_make_one_row_each_as_booked() {
    use std::str::FromStr;

    use bigdecimal::BigDecimal;
    use zhang_core::ast::amount::Amount;
    use zhang_core::ast::{Date, Directive, Posting, PostingCost, WrittenPosting};
    use zhang_core::clock::Clock;
    use zhang_core::data_source::DataSource;
    use zhang_core::ledger::LedgerProcessContext;

    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    std::fs::write(
        root.join("main.zhang"),
        format!(
            "{HEADER}{}",
            indoc! {r#"
                2024-05-16 * "buy"
                  Assets:A 10 USD { 10 CNY }
                  Income:I -100 CNY
                2024-05-17 * "buy"
                  Assets:A 10 USD { 11 CNY }
                  Income:I -110 CNY
                2024-05-18 * "sell across both lots"
                  Assets:A -15 USD {}
                  Income:I
            "#}
        ),
    )
    .unwrap();
    let source = Arc::new(LocalFileSystemDataSource::new(ZhangDataType {}));
    let mut loaded = source.load(root.to_string_lossy().to_string(), "main.zhang".to_owned()).unwrap();

    // what booking makes of the sale, with the income posting moved between the two legs
    let Directive::Transaction(sale) = &mut loaded.directives.last_mut().unwrap().data else {
        panic!("the sale")
    };
    let written_sale = WrittenPosting {
        index: 0,
        units: sale.postings[0].units.clone(),
        cost: sale.postings[0].cost.clone(),
    };
    let leg = |units: i64, cost: i64, date: &str| Posting {
        units: Some(Amount::new(BigDecimal::from(units), "USD")),
        cost: Some(PostingCost {
            base: Some(Amount::new(BigDecimal::from(cost), "CNY")),
            date: Some(Date::Date(chrono::NaiveDate::from_str(date).unwrap())),
            label: None,
            total: false,
        }),
        written: Some(written_sale.clone()),
        ..sale.postings[0].clone()
    };
    let income = Posting {
        units: Some(Amount::new(BigDecimal::from(155), "CNY")),
        written: Some(WrittenPosting {
            index: 1,
            units: None,
            cost: None,
        }),
        ..sale.postings[1].clone()
    };
    sale.postings = vec![leg(-10, 10, "2024-05-16"), income, leg(-5, 11, "2024-05-17")];

    let ledger = Ledger::process(LedgerProcessContext {
        directives: loaded.directives,
        entry: (root, "main.zhang".to_owned()),
        visited_files: loaded.visited_files,
        data_source: source,
        clock: Clock::System,
    })
    .unwrap();
    assert_eq!(errors(&ledger), vec![]);
    assert_eq!(lots(&ledger, "Assets:A"), vec!["5 USD {11 CNY, 2024-05-17}"]);

    let store = ledger.store.read().unwrap();
    let rows: Vec<String> = store
        .postings
        .iter()
        .skip(4)
        .map(|row| {
            format!(
                "{} {} {} = {}",
                row.account.name(),
                row.unit.as_ref().map_or("?".to_owned(), ToString::to_string),
                row.cost.as_ref().map_or("-".to_owned(), ToString::to_string),
                row.inferred_amount
            )
        })
        .collect();
    assert_eq!(
        rows,
        vec!["Assets:A -10 USD 10 CNY = -10 USD", "Income:I ? - = 155 CNY", "Assets:A -5 USD 11 CNY = -5 USD",]
    );
}

/// A total cost is spread over the absolute units, as in beancount: a sale written `-3 USD {{99 CNY}}`
/// looks for the lot bought at `33 CNY` and reduces it. Before, the per-unit cost was `-33 CNY`,
/// which matched no lot: the sale was reported as `NoEnoughCommodityLot` and opened a lot with a
/// negative cost next to the one it should have reduced.
#[test]
fn a_total_cost_on_negative_units_reduces_the_lot_bought_at_that_cost() {
    let ledger = load(indoc! {r#"
        2024-05-16 * "buy at a total cost"
          Assets:A 3 USD {{ 99 CNY }}
          Income:I -99 CNY
        2024-05-17 * "sell at the same total cost"
          Assets:A -3 USD {{ 99 CNY }}
          Income:I 99 CNY
    "#});
    assert_eq!(errors(&ledger), vec![]);
    assert_eq!(lots(&ledger, "Assets:A"), Vec::<String>::new());

    // the short lot a sale opens when nothing is held carries the positive per-unit cost
    let ledger = load(indoc! {r#"
        2024-05-19 * "negative units at a total cost, nothing held"
          Assets:A -3 USD {{ 99 CNY }}
          Income:I 99 CNY
    "#});
    assert_eq!(errors(&ledger), vec![(ErrorKind::NoEnoughCommodityLot, Some("-3".to_owned()))]);
    assert_eq!(lots(&ledger, "Assets:A"), vec!["-3 USD {33 CNY, 2024-05-19}"]);
}

#[test]
fn covering_a_short_lot_keeps_its_cost_date_and_label() {
    for (cost, label) in [("{}", ""), ("{10 CNY}", ""), ("{, \"short\"}", ", \"short\"")] {
        for cover in [4, 10] {
            for cash in [format!("Income:I -{} CNY", cover * 10), "Income:I".to_owned()] {
                let ledger = load(&formatdoc! {r#"
                    2024-05-16 * "open a short"
                      Assets:A -10 USD {{10 CNY{label}}}
                      Income:I 100 CNY
                    2024-05-17 * "cover the short"
                      Assets:A {cover} USD {cost}
                      {cash}
                "#});
                // Opening a short still reports the existing insufficient-lot warning. Covering
                // it must neither add a warning nor lose the cost basis (#614).
                assert_eq!(
                    errors(&ledger),
                    vec![(ErrorKind::NoEnoughCommodityLot, Some("-10".to_owned()))],
                    "{cost}, {cover}, {cash}"
                );
                let expected = if cover == 10 {
                    vec![]
                } else {
                    vec![format!("-6 USD {{10 CNY, 2024-05-16{label}}}")]
                };
                assert_eq!(lots(&ledger, "Assets:A"), expected, "{cost}, {cover}, {cash}");
                assert_eq!(inferred(&ledger, 2), vec![format!("{cover} USD"), format!("-{} CNY", cover * 10)]);
            }
        }
    }
}

#[test]
fn covering_several_short_lots_uses_the_booking_method() {
    for (method, cost, remaining, ambiguity) in [
        ("FIFO", 83, "-2 USD {11 CNY, 2024-05-17}", false),
        ("LIFO", 85, "-2 USD {10 CNY, 2024-05-16}", false),
        ("STRICT", 83, "-2 USD {11 CNY, 2024-05-17}", true),
    ] {
        let ledger = load(&formatdoc! {r#"
            1970-01-01 open Assets:A
              booking_method: "{method}"
            2024-05-16 * "first short"
              Assets:A -5 USD {{10 CNY}}
              Income:I 50 CNY
            2024-05-17 * "second short"
              Assets:A -5 USD {{11 CNY}}
              Income:I 55 CNY
            2024-05-18 * "cover both"
              Assets:A 8 USD {{}}
              Income:I
        "#});
        let mut expected_errors = vec![(ErrorKind::NoEnoughCommodityLot, Some("-5".to_owned())); 2];
        if ambiguity {
            expected_errors.push((ErrorKind::AmbiguousLotMatch, Some("8".to_owned())));
        }
        assert_eq!(errors(&ledger), expected_errors, "{method}");
        assert_eq!(lots(&ledger, "Assets:A"), vec![remaining], "{method}");
        assert_eq!(inferred(&ledger, 3), vec!["8 USD".to_owned(), format!("-{cost} CNY")], "{method}");
    }
}

#[test]
fn a_cover_reduces_short_lots_before_augmenting_matching_long_lots() {
    let ledger = load(indoc! {r#"
        2024-05-15 * "long lot"
          Assets:A 10 USD {10 CNY}
          Income:I -100 CNY
        2024-05-16 * "short of the same cost on another date"
          Assets:A -10 USD {10 CNY, 2024-05-16}
          Income:I 100 CNY
        2024-05-17 * "cover"
          Assets:A 4 USD {}
          Income:I
    "#});
    assert_eq!(errors(&ledger), vec![(ErrorKind::NoEnoughCommodityLot, Some("-10".to_owned()))]);
    assert_eq!(lots(&ledger, "Assets:A"), vec!["10 USD {10 CNY, 2024-05-15}", "-6 USD {10 CNY, 2024-05-16}"]);
    assert_eq!(inferred(&ledger, 3), vec!["4 USD", "-40 CNY"]);
}

#[test]
fn a_cover_larger_than_the_short_opens_only_the_remaining_units() {
    let ledger = load(indoc! {r#"
        2024-05-16 * "short at a total cost"
          Assets:A -10 USD {{100 CNY}}
          Income:I 100 CNY
        2024-05-17 * "cover and buy"
          Assets:A 14 USD {10 CNY} @ 12 CNY
          Income:I -140 CNY
    "#});
    assert_eq!(errors(&ledger), vec![(ErrorKind::NoEnoughCommodityLot, Some("-10".to_owned()))]);
    assert_eq!(lots(&ledger, "Assets:A"), vec!["4 USD {10 CNY, 2024-05-17}"]);
    assert_eq!(inferred(&ledger, 2), vec!["14 USD", "-140 CNY"]);
}

/// Booking runs as a stage before the plugins (design §2): the stream the ledger keeps holds the
/// booked postings, with the written form on those booking changed, and final validation's pass 2
/// completes what the stages left unbooked, such as the implicit leg of a padding transaction.
#[test]
fn the_ledger_keeps_the_booked_postings() {
    let ledger = load(indoc! {r#"
        1970-01-01 open Assets:S
        1970-01-01 open Equity:Open
        2024-05-16 * "buy"
          Assets:A 10 USD { 10 CNY }
          Income:I -100 CNY
        2024-05-17 * "buy"
          Assets:A 10 USD { 11 CNY }
          Income:I -110 CNY
        2024-05-18 * "sell across both lots"
          Assets:A -15 USD {}
          Income:I
        2024-05-19 pad Assets:S Equity:Open
        2024-05-20 balance Assets:S 7 USD
    "#});
    assert_eq!(errors(&ledger), vec![]);

    let show = |posting: &zhang_core::ast::Posting| {
        let units = posting.units.as_ref().map_or("?".to_owned(), ToString::to_string);
        let cost = posting
            .cost
            .as_ref()
            .map(|cost| {
                format!(
                    " {{{}{}}}",
                    cost.base.as_ref().map(ToString::to_string).unwrap_or_default(),
                    cost.date.as_ref().map(|date| format!(", {}", date.naive_date())).unwrap_or_default()
                )
            })
            .unwrap_or_default();
        let written = posting
            .written
            .as_ref()
            .map(|written| format!(" <- #{} {}", written.index, written.units.as_ref().map_or("?".to_owned(), ToString::to_string)))
            .unwrap_or_default();
        format!("{} {units}{cost}{written}", posting.account.name())
    };
    let transactions: Vec<Vec<String>> = ledger
        .directives
        .iter()
        .filter_map(|it| match &it.data {
            zhang_core::ast::Directive::Transaction(txn) => Some(txn.postings.iter().map(show).collect()),
            _ => None,
        })
        .collect();
    assert_eq!(
        transactions,
        vec![
            vec!["Assets:A 10 USD {10 CNY, 2024-05-16} <- #0 10 USD", "Income:I -100 CNY"],
            vec!["Assets:A 10 USD {11 CNY, 2024-05-17} <- #0 10 USD", "Income:I -110 CNY"],
            vec![
                "Assets:A -10 USD {10 CNY, 2024-05-16} <- #0 -15 USD",
                "Assets:A -5 USD {11 CNY, 2024-05-17} <- #0 -15 USD",
                "Income:I 155 CNY <- #1 ?",
            ],
            // the padding transaction the pad stage emits after booking: its implicit leg is
            // completed by final validation
            vec!["Assets:S 7 USD", "Equity:Open -7 USD <- #1 ?"],
        ]
    );

    // the store still has one row per written posting, as written
    let store = ledger.store.read().unwrap();
    let rows: Vec<String> = store
        .postings
        .iter()
        .map(|row| {
            format!(
                "{} {} = {}",
                row.account.name(),
                row.unit.as_ref().map_or("?".to_owned(), ToString::to_string),
                row.inferred_amount
            )
        })
        .collect();
    assert_eq!(
        rows,
        vec![
            "Assets:A 10 USD = 10 USD",
            "Income:I -100 CNY = -100 CNY",
            "Assets:A 10 USD = 10 USD",
            "Income:I -110 CNY = -110 CNY",
            "Assets:A -15 USD = -15 USD",
            "Income:I ? = 155 CNY",
            "Assets:S 7 USD = 7 USD",
            "Equity:Open ? = -7 USD",
        ]
    );
}

/// A transaction final validation rejects (its explicit postings weigh in several commodities) is
/// not stored, and the pad and balance-check stages skip it too, so an assertion on an account it
/// names agrees with the store, not with the rejected transaction.
#[test]
fn a_rejected_transaction_counts_for_no_balance_assertion() {
    let ledger = load(indoc! {r#"
        1970-01-01 open Assets:Broker
        1970-01-01 open Assets:X
        1970-01-01 open Assets:Short
        1970-01-01 open Equity:Open
        2024-01-02 * "buy"
          Assets:Broker 3 AAPL { 10 USD }
          Assets:A -30 USD
        2024-01-03 * "x"
          Assets:Broker -3 AAPL {}
          Assets:X 3 AAPL
          Assets:Short 5 CNY
          Equity:Open
        2024-01-04 balance Assets:Short 0 CNY
        2024-01-04 balance Assets:Broker 3 AAPL
        2024-01-04 balance Assets:X 0 AAPL
    "#});
    assert_eq!(
        errors(&ledger),
        vec![(ErrorKind::TransactionExplicitPostingHaveMultipleCommodity, None)],
        "the rejection is the only error: every assertion agrees with the store"
    );
    let store = ledger.store.read().unwrap();
    assert!(store.balance_assertions.iter().all(|it| it.passed));
    assert_eq!(store.transactions.len(), 1);
}

/// Final booking errors belong to the stage channel, before materialization errors. Both errors
/// retain the transaction's span and metadata, even when their order changes from the old fold.
#[test]
fn final_validation_errors_precede_undefined_budget_activity() {
    let ledger = load(indoc! {r#"
        1970-01-01 open Expenses:Food
          budget: missing
        2024-01-01 * "unbalanced budget activity"
          Expenses:Food 10 CNY
          Income:I -9 CNY
    "#});
    let store = ledger.store.read().unwrap();
    assert_eq!(
        store.errors.iter().map(|it| &it.error_type).collect::<Vec<_>>(),
        [&ErrorKind::UnbalancedTransaction, &ErrorKind::BudgetDoesNotExist]
    );
    let txn = store.transactions.values().next().unwrap();
    assert_eq!(store.errors[0].metas["txn_id"], txn.id.to_string());
    assert_eq!(store.errors[1].metas["budget_name"], "missing");
    assert_eq!(store.errors[1].metas["account_name"], "Expenses:Food");
    assert!(store.errors.iter().all(|it| it.span.as_ref() == Some(&txn.span)));
}
