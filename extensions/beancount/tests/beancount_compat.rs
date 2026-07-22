use std::path::PathBuf;
use std::str::FromStr;

use beancount::directives::BeancountOnlyDirective;
use beancount::parser::parse;
use bigdecimal::BigDecimal;
use itertools::Either;
use zhang_ast::{Directive, Flag, SpanInfo, Spanned, Transaction};
use zhang_core::data_type::text::ZhangDataType;
use zhang_core::data_type::DataType;
use zhang_core::inventory::TransactionInference;

fn txn(content: &str) -> Transaction {
    let d = parse(content, None::<PathBuf>).unwrap().into_iter().find_map(|s| s.data.left()).unwrap();
    match d {
        Directive::Transaction(t) => t,
        _ => panic!("expected txn"),
    }
}

#[test]
fn tolerance_and_txn_parse_correctly() {
    let d = parse("2014-01-01 balance Assets:Cash 100 ~ 0.5 USD\n", None::<PathBuf>)
        .unwrap()
        .pop()
        .unwrap()
        .data;
    match d {
        Either::Right(BeancountOnlyDirective::Balance(b)) => assert_eq!(b.tolerance, Some(BigDecimal::from_str("0.5").unwrap())),
        _ => panic!("expected balance"),
    }
    assert_eq!(txn("2014-01-01 txn \"p\" \"n\"\n  Assets:Cash 1 USD\n  Equity:X\n").flag, Some(Flag::Okay));
}

#[test]
fn cost_label_and_total_cost() {
    // label captured
    let t = txn("2014-01-01 * \"x\"\n  Assets:Cash 1 HOOL {100 USD, \"lot1\"}\n  Equity:X\n");
    assert_eq!(t.postings[0].cost.as_ref().unwrap().label, Some("lot1".to_string()));

    // total cost {{ }} keeps the raw total, and normalises to per-unit for lot bookkeeping
    let t = txn("2014-01-01 * \"x\"\n  Assets:Cash 10 HOOL {{1000 USD}}\n  Equity:X\n");
    let cost = t.postings[0].cost.as_ref().unwrap();
    assert!(cost.total, "total flag set");
    assert_eq!(cost.base.as_ref().unwrap().number, BigDecimal::from(1000), "raw total stored");
    let lot = t.txn_postings()[0].lot_meta();
    assert_eq!(
        lot.cost.unwrap().base.unwrap().number,
        BigDecimal::from(100),
        "1000/10 = 100 per-unit for the lot"
    );

    // round-trips through the zhang exporter
    let exported = ZhangDataType::default().export(Spanned::new(Directive::Transaction(t), SpanInfo::default()));
    assert!(exported.contains("{{"), "total cost re-exports as {{ }}: {exported}");
    let t2 = txn("2014-01-01 * \"x\"\n  Assets:Cash 1 HOOL {100 USD, \"lot1\"}\n  Equity:X\n");
    let exported2 = ZhangDataType::default().export(Spanned::new(Directive::Transaction(t2), SpanInfo::default()));
    assert!(exported2.contains("\"lot1\""), "label re-exports: {exported2}");
}

#[test]
fn pushmeta_applies_to_following_directives() {
    use beancount::Beancount;
    use zhang_core::data_type::DataType;
    let content = "pushmeta project: \"X\"\n2014-01-01 open Assets:Cash\npopmeta project:\n2014-01-02 open Assets:Bank\n";
    let dirs = Beancount::default().transform(content.to_string(), None).unwrap();
    let opens: Vec<_> = dirs
        .into_iter()
        .filter_map(|s| match s.data {
            Directive::Open(o) => Some(o),
            _ => None,
        })
        .collect();
    assert_eq!(opens.len(), 2);
    assert_eq!(
        opens[0].meta.get_one("project").map(|v| v.as_str().to_string()),
        Some("X".to_string()),
        "inside push/pop gets meta"
    );
    assert_eq!(opens[1].meta.get_one("project"), None, "after popmeta: no meta");
}
