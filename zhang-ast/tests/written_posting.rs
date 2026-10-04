//! `Posting::written` (booking-split design, #423): advisory, absent when `None`, so a posting
//! booking left alone serializes exactly as before the field existed.

use std::str::FromStr;

use bigdecimal::BigDecimal;
use zhang_ast::amount::Amount;
use zhang_ast::{written_postings, Account, Date, Meta, Posting, PostingCost, WrittenPosting};

fn posting(account: &str, units: Option<Amount>, cost: Option<PostingCost>) -> Posting {
    Posting {
        flag: None,
        account: Account::from_str(account).unwrap(),
        units,
        cost,
        price: None,
        comment: None,
        meta: Meta::default(),
        written: None,
    }
}

fn amount(number: i64, commodity: &str) -> Amount {
    Amount::new(BigDecimal::from(number), commodity)
}

fn cost(base: Option<Amount>, date: Option<&str>) -> PostingCost {
    PostingCost {
        base,
        date: date.map(|it| Date::Date(chrono::NaiveDate::from_str(it).unwrap())),
        label: None,
        total: false,
    }
}

#[test]
fn a_posting_without_a_written_form_serializes_as_before_the_field_existed() {
    // the JSON of zhang-ast before `written` was added, byte for byte
    let explicit = posting("Assets:Cash", Some(amount(-1, "CNY")), Some(cost(Some(amount(1, "USD")), None)));
    assert_eq!(
        serde_json::to_string(&explicit).unwrap(),
        r#"{"flag":null,"account":{"account_type":"Assets","content":"Assets:Cash","components":["Cash"]},"units":{"number":"-1","commodity":"CNY"},"cost":{"base":{"number":"1","commodity":"USD"},"date":null,"label":null,"total":false},"price":null,"comment":null,"meta":{"inner":{}}}"#
    );
    let implicit = posting("Expenses:Food", None, None);
    assert_eq!(
        serde_json::to_string(&implicit).unwrap(),
        r#"{"flag":null,"account":{"account_type":"Expenses","content":"Expenses:Food","components":["Food"]},"units":null,"cost":null,"price":null,"comment":null,"meta":{"inner":{}}}"#
    );
}

#[test]
fn the_written_form_round_trips_and_a_missing_field_reads_as_none() {
    let mut leg = posting(
        "Assets:Broker",
        Some(amount(-10, "USD")),
        Some(cost(Some(amount(10, "CNY")), Some("2024-05-16"))),
    );
    leg.written = Some(WrittenPosting {
        index: 0,
        units: Some(amount(-15, "USD")),
        cost: Some(cost(None, None)),
    });
    let json = serde_json::to_string(&leg).unwrap();
    assert!(json.contains(r#""written":{"index":0,"units":{"number":"-15","commodity":"USD"},"cost":{"base":null,"date":null,"label":null,"total":false}}"#));
    assert_eq!(serde_json::from_str::<Posting>(&json).unwrap(), leg);

    // what a plugin built against an older zhang-ast writes back: no `written` key
    let old: Posting = serde_json::from_str(
        r#"{"flag":null,"account":{"account_type":"Expenses","content":"Expenses:Food","components":["Food"]},"units":null,"cost":null,"price":null,"comment":null,"meta":{"inner":{}}}"#,
    )
    .unwrap();
    assert_eq!(old, posting("Expenses:Food", None, None));
}

#[test]
fn written_postings_merge_the_legs_of_a_split_and_restore_the_written_form() {
    // `-15 USD {}` booked against two lots, then an interpolated posting, then one left as written
    let written_sale = WrittenPosting {
        index: 0,
        units: Some(amount(-15, "USD")),
        cost: Some(cost(None, None)),
    };
    let mut first = posting("Assets:S", Some(amount(-10, "USD")), Some(cost(Some(amount(10, "CNY")), Some("2024-05-16"))));
    first.written = Some(written_sale.clone());
    first.meta.insert("note".to_owned(), zhang_ast::ZhangString::quote("first leg"));
    let mut second = posting("Assets:S", Some(amount(-5, "USD")), Some(cost(Some(amount(11, "CNY")), Some("2024-05-17"))));
    second.written = Some(written_sale);
    let mut income = posting("Income:I", Some(amount(155, "CNY")), None);
    income.written = Some(WrittenPosting {
        index: 1,
        units: None,
        cost: None,
    });
    let fee = posting("Expenses:Fee", Some(amount(1, "CNY")), None);

    let written = written_postings(vec![first.clone(), second, income, fee.clone()]);

    let mut expected_sale = posting("Assets:S", Some(amount(-15, "USD")), Some(cost(None, None)));
    expected_sale.meta = first.meta.clone();
    assert_eq!(written, vec![expected_sale, posting("Income:I", None, None), fee]);
}

#[test]
fn written_postings_keep_legs_of_different_written_postings_apart() {
    // the legs of two postings that both carry a written form are not merged into one, and a
    // posting without one between them is left alone
    let mut a = posting("Assets:A", Some(amount(1, "USD")), None);
    a.written = Some(WrittenPosting {
        index: 0,
        units: None,
        cost: None,
    });
    let plain = posting("Assets:P", Some(amount(2, "USD")), None);
    let mut b = posting("Assets:B", Some(amount(3, "USD")), None);
    b.written = Some(WrittenPosting {
        index: 2,
        units: None,
        cost: None,
    });
    let written = written_postings(vec![a, plain.clone(), b]);
    assert_eq!(written, vec![posting("Assets:A", None, None), plain, posting("Assets:B", None, None)]);
}

fn written(index: usize, units: Option<Amount>, cost: Option<PostingCost>) -> Option<WrittenPosting> {
    Some(WrittenPosting { index, units, cost })
}

#[test]
fn legs_a_stage_moved_apart_are_kept_as_booked() {
    // a plugin put the income between the two legs of the sale: restoring the written posting per
    // run would double the sale, so the legs stay as booked, and the income still reads as written
    let mut first = posting("Assets:S", Some(amount(-10, "USD")), Some(cost(Some(amount(10, "CNY")), Some("2024-05-16"))));
    first.written = written(0, Some(amount(-15, "USD")), Some(cost(None, None)));
    let mut second = posting("Assets:S", Some(amount(-5, "USD")), Some(cost(Some(amount(11, "CNY")), Some("2024-05-17"))));
    second.written = first.written.clone();
    let mut income = posting("Income:I", Some(amount(155, "CNY")), None);
    income.written = written(1, None, None);

    let result = written_postings(vec![first.clone(), income, second.clone()]);

    let mut raw_first = first.clone();
    raw_first.written = None;
    let mut raw_second = second.clone();
    raw_second.written = None;
    assert_eq!(result, vec![raw_first, posting("Income:I", None, None), raw_second]);
}

#[test]
fn distinct_postings_sharing_an_index_are_kept_as_booked() {
    // a plugin duplicated a leg onto another account and kept its `written`: merging them would
    // drop the second account from the export, so both stay as booked
    let mut a = posting("Assets:A", Some(amount(-10, "USD")), Some(cost(Some(amount(10, "CNY")), Some("2024-05-16"))));
    a.written = written(0, Some(amount(-10, "USD")), Some(cost(None, None)));
    let mut b = posting("Assets:B", Some(amount(-10, "USD")), Some(cost(Some(amount(10, "CNY")), Some("2024-05-16"))));
    b.written = a.written.clone();
    let mut income = posting("Income:I", Some(amount(200, "CNY")), None);
    income.written = written(1, None, None);

    let result = written_postings(vec![a.clone(), b.clone(), income]);

    let mut raw_a = a.clone();
    raw_a.written = None;
    let mut raw_b = b.clone();
    raw_b.written = None;
    assert_eq!(result, vec![raw_a, raw_b, posting("Income:I", None, None)]);
}

#[test]
fn a_complete_split_is_restored_even_when_other_indexes_are_incomplete() {
    // the legs of one sale are complete; the index of another posting was duplicated
    let sale = written(0, Some(amount(-15, "USD")), Some(cost(None, None)));
    let mut first = posting("Assets:S", Some(amount(-10, "USD")), Some(cost(Some(amount(10, "CNY")), Some("2024-05-16"))));
    first.written = sale.clone();
    let mut second = posting("Assets:S", Some(amount(-5, "USD")), Some(cost(Some(amount(11, "CNY")), Some("2024-05-17"))));
    second.written = sale;
    let mut fee = posting("Expenses:Fee", Some(amount(1, "CNY")), None);
    fee.written = written(1, None, None);
    let mut copied_fee = posting("Expenses:Other", Some(amount(1, "CNY")), None);
    copied_fee.written = written(1, None, None);

    let result = written_postings(vec![first, second, fee.clone(), copied_fee.clone()]);

    fee.written = None;
    copied_fee.written = None;
    assert_eq!(
        result,
        vec![posting("Assets:S", Some(amount(-15, "USD")), Some(cost(None, None))), fee, copied_fee]
    );
}
