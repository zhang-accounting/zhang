//! Posting metadata in beancount files, checked against Python beancount: zhang attaches
//! every metadata line of `posting_meta/ledger.bean` to the same transaction or posting,
//! with the same value, as beancount 3.2.3 does (`posting_meta/oracle.json`, written by
//! `posting_meta/generate.py`).

use std::collections::BTreeMap;
use std::path::PathBuf;

use beancount::Beancount;
use serde_json::{json, Value};
use zhang_ast::{Directive, Meta};
use zhang_core::data_type::DataType;

fn meta(meta: &Meta) -> Value {
    let pairs = meta
        .clone()
        .get_flatten()
        .into_iter()
        .map(|(key, value)| (key, value.to_plain_string()))
        .collect::<BTreeMap<_, _>>();
    json!(pairs)
}

#[test]
fn posting_metadata_attaches_where_beancount_attaches_it() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/posting_meta");
    let ledger = std::fs::read_to_string(dir.join("ledger.bean")).unwrap();
    let oracle: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("oracle.json")).unwrap()).unwrap();

    let transactions = Beancount::default()
        .transform(ledger, None)
        .expect("the ledger parses")
        .into_iter()
        .filter_map(|directive| match directive.data {
            Directive::Transaction(transaction) => Some(transaction),
            _ => None,
        })
        .map(|transaction| {
            let postings = transaction
                .postings
                .iter()
                .map(|posting| json!({"account": posting.account.name(), "meta": meta(&posting.meta)}))
                .collect::<Vec<_>>();
            json!({
                "date": transaction.date.naive_date().to_string(),
                "narration": transaction.narration.map(|it| it.to_plain_string()),
                "meta": meta(&transaction.meta),
                "postings": postings,
            })
        })
        .collect::<Vec<_>>();
    assert_eq!(Value::Array(transactions), oracle);
}

fn transaction(text: &str) -> zhang_ast::Transaction {
    match Beancount::default().transform(text.to_owned(), None).unwrap().pop().unwrap().data {
        Directive::Transaction(transaction) => transaction,
        other => panic!("expected a transaction, got {other:?}"),
    }
}

#[test]
fn posting_metadata_round_trips_through_the_beancount_exporter() {
    use zhang_ast::{SpanInfo, Spanned};
    use zhang_core::data_type::text::ZhangDataType;

    let text = concat!(
        "2024-01-02 * \"Shop\" \"mixed\"\n",
        "  memo: \"before\"\n",
        "  Assets:Cash -10 CNY\n",
        " receipt: \"say \\\"hi\\\" \\\\ back\"\n",
        "  Expenses:Food 6 CNY\n",
        "  Expenses:Drinks 4 CNY\n",
        "        rate: 1.5\n",
        "  note: \"after\"\n",
    );
    let original = transaction(text);
    assert_eq!(meta(&original.meta), json!({"memo": "before"}));
    let postings = original.postings.iter().map(|posting| meta(&posting.meta)).collect::<Vec<_>>();
    assert_eq!(
        postings,
        vec![json!({"receipt": "say \"hi\" \\ back"}), json!({}), json!({"note": "after", "rate": "1.5"})]
    );

    let expected = concat!(
        "2024-01-02 * \"Shop\" \"mixed\"\n",
        "  memo: \"before\"\n",
        "  Assets:Cash -10 CNY\n",
        "    receipt: \"say \\\"hi\\\" \\\\ back\"\n",
        "  Expenses:Food 6 CNY\n",
        "  Expenses:Drinks 4 CNY\n",
        "    note: \"after\"\n",
        "    rate: 1.5",
    );
    let exported = Beancount::default().export(Spanned::new(Directive::Transaction(original.clone()), SpanInfo::default()));
    assert_eq!(exported, expected);
    assert_eq!(transaction(&exported), original);
    // zhang reads the same text the same way: the posting metadata is indented deeper
    let zhang = ZhangDataType::default().transform(exported, None).unwrap().pop().unwrap().data;
    assert_eq!(zhang, Directive::Transaction(original));
}

#[test]
fn a_time_older_zhang_wrote_after_the_postings_stays_the_transactions_time() {
    use chrono::NaiveDate;
    use zhang_ast::{Date, ZhangString};

    // older zhang wrote a transaction's metadata, `time` included, after its postings
    let txn = transaction("2024-01-15 * \"Bob\" \"coffee\"\n  Assets:Cash -5 CNY\n  Expenses:Food 5 CNY\n  time: \"12:30:00\"\n  note: \"n\"\n");
    let noon = NaiveDate::from_ymd_opt(2024, 1, 15).unwrap().and_hms_opt(12, 30, 0).unwrap();
    assert_eq!(txn.date, Date::Datetime(noon));
    assert_eq!(meta(&txn.meta), json!({}));
    // any other metadata stays on the posting, where beancount reads it
    assert_eq!(meta(&txn.postings[1].meta), json!({"note": "n"}));

    // a transaction with a time of its own keeps it, and the posting keeps its `time`
    let txn = transaction("2024-01-15 * \"Bob\" \"coffee\"\n  time: \"09:00:00\"\n  Assets:Cash -5 CNY\n  Expenses:Food 5 CNY\n    time: \"12:30:00\"\n");
    let nine = NaiveDate::from_ymd_opt(2024, 1, 15).unwrap().and_hms_opt(9, 0, 0).unwrap();
    assert_eq!(txn.date, Date::Datetime(nine));
    assert_eq!(txn.postings[1].meta.get_one("time"), Some(&ZhangString::quote("12:30:00")));

    // only a time of day is taken
    let txn = transaction("2024-01-15 * \"Bob\" \"coffee\"\n  Assets:Cash -5 CNY\n  Expenses:Food 5 CNY\n  time: \"soon\"\n");
    assert!(matches!(txn.date, Date::Date(_)));
    assert_eq!(meta(&txn.postings[1].meta), json!({"time": "soon"}));
}

/// Where nothing says older zhang wrote it, a posting's `time` is the posting's, as
/// beancount 3.2.3 reads it.
#[test]
fn a_time_of_a_posting_stays_the_postings() {
    use zhang_ast::Date;

    let times = |txn: &zhang_ast::Transaction| txn.postings.iter().map(|posting| meta(&posting.meta)).collect::<Vec<_>>();
    // every posting has a time, under it or at its indentation
    for text in [
        "2024-01-15 * \"Bob\" \"coffee\"\n  Assets:Cash -5 USD\n    time: \"01:00:00\"\n  Expenses:Food 5 USD\n    time: \"02:00:00\"\n",
        "2024-01-15 * \"Bob\" \"coffee\"\n  Assets:Cash -5 USD\n  time: \"01:00:00\"\n  Expenses:Food 5 USD\n  time: \"02:00:00\"\n",
    ] {
        let txn = transaction(text);
        assert!(matches!(txn.date, Date::Date(_)), "{text}");
        assert_eq!(times(&txn), vec![json!({"time": "01:00:00"}), json!({"time": "02:00:00"})], "{text}");
    }

    // the last posting only, indented deeper than it: zhang never wrote that
    let txn = transaction("2024-01-15 * \"Bob\" \"coffee\"\n  Assets:Cash -5 USD\n  Expenses:Food 5 USD\n    time: \"02:00:00\"\n");
    assert!(matches!(txn.date, Date::Date(_)));
    assert_eq!(times(&txn), vec![json!({}), json!({"time": "02:00:00"})]);

    // the transaction has a time of its own
    let txn = transaction("2024-01-15 * \"Bob\" \"coffee\"\n  time: \"09:00:00\"\n  Assets:Cash -5 USD\n  Expenses:Food 5 USD\n  time: \"12:30:00\"\n");
    let nine = chrono::NaiveDate::from_ymd_opt(2024, 1, 15).unwrap().and_hms_opt(9, 0, 0).unwrap();
    assert_eq!(txn.date, Date::Datetime(nine));
    assert_eq!(times(&txn), vec![json!({}), json!({"time": "12:30:00"})]);

    // the only time is the first posting's: zhang wrote transaction metadata after the last
    let txn = transaction("2024-01-15 * \"Bob\" \"coffee\"\n  Assets:Cash -5 USD\n  time: \"01:00:00\"\n  Expenses:Food 5 USD\n");
    assert!(matches!(txn.date, Date::Date(_)));
    assert_eq!(times(&txn), vec![json!({"time": "01:00:00"}), json!({})]);

    // another posting has a time too
    let txn = transaction("2024-01-15 * \"Bob\" \"coffee\"\n  Assets:Cash -5 USD\n    time: \"01:00:00\"\n  Expenses:Food 5 USD\n  time: \"02:00:00\"\n");
    assert!(matches!(txn.date, Date::Date(_)));
    assert_eq!(times(&txn), vec![json!({"time": "01:00:00"}), json!({"time": "02:00:00"})]);
}
