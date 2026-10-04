//! Postings with a flag of their own in beancount files (#474), checked against Python beancount:
//! zhang reads the same flag on every posting of `posting_flags/ledger.bean` as beancount 3.2.3
//! does (`posting_flags/oracle.json`, written by `posting_flags/generate.py`), and writes each
//! transaction so that it reads back unchanged.

use std::path::PathBuf;

use beancount::Beancount;
use indoc::indoc;
use serde_json::{json, Value};
use zhang_ast::{Directive, SpanInfo, Spanned, Transaction};
use zhang_core::data_type::DataType;

fn transactions(text: &str) -> Vec<Transaction> {
    Beancount::default()
        .transform(text.to_owned(), None)
        .unwrap_or_else(|err| panic!("cannot parse {text:?}: {err}"))
        .into_iter()
        .filter_map(|directive| match directive.data {
            Directive::Transaction(transaction) => Some(transaction),
            _ => None,
        })
        .collect()
}

fn ledger() -> String {
    std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/posting_flags/ledger.bean")).unwrap()
}

fn export(transaction: Transaction) -> String {
    Beancount::default().export(Spanned::new(Directive::Transaction(transaction), SpanInfo::default()))
}

#[test]
fn posting_flags_are_read_as_beancount_reads_them() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/posting_flags");
    let oracle: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("oracle.json")).unwrap()).unwrap();

    let read = transactions(&ledger())
        .into_iter()
        .map(|transaction| {
            let postings = transaction
                .postings
                .iter()
                .map(|posting| json!({"account": posting.account.name(), "flag": posting.flag.as_ref().map(|it| it.to_string())}))
                .collect::<Vec<_>>();
            json!({
                "date": transaction.date.naive_date().to_string(),
                "flag": transaction.flag.as_ref().map(|it| it.to_string()),
                "narration": transaction.narration.map(|it| it.to_plain_string()),
                "postings": postings,
            })
        })
        .collect::<Vec<_>>();
    assert_eq!(Value::Array(read), oracle);
}

#[test]
fn every_transaction_with_flagged_postings_round_trips() {
    for transaction in transactions(&ledger()) {
        let exported = export(transaction.clone());
        let reparsed = transactions(&exported);
        assert_eq!(reparsed, vec![transaction], "exported as:\n{exported}");
    }
}

#[test]
fn a_posting_flag_is_written_before_the_account() {
    let text = indoc! {r#"
        2020-01-14 * "Broker" "sell at a price"
          ! Assets:Broker  -4 AAPL {} @ 110 USD ; check the gain
            receipt: "r-1"
          *	Assets:Bank  440 USD
          # Income:Gains
    "#};
    let transaction = transactions(text).pop().unwrap();
    // valid beancount: bean-check 3.2.3 accepts it, with the posting flags `!`, `*` and `#`
    assert_eq!(
        export(transaction),
        indoc! {r#"
            2020-01-14 * "Broker" "sell at a price"
              ! Assets:Broker -4 AAPL { } @ 110 USD ; check the gain
                receipt: "r-1"
              * Assets:Bank 440 USD
              # Income:Gains"#}
    );
}
