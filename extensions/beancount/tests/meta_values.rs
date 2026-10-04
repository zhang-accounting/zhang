//! The metadata values beancount reads without quotes (#475), checked against Python beancount:
//! zhang reads every metadata entry of `meta_values/ledger.bean`, on a transaction, on its postings
//! and on the other directives, with the value beancount 3.2.3 gives it (`meta_values/oracle.json`,
//! written by `meta_values/generate.py`), and writes each one back as it was written.
//!
//! Zhang keeps a bare value as text, as written, so the file round-trips; beancount evaluates it.
//! The two show the same text for an account, a currency, a plain number, an amount and a date. For
//! the rest beancount shows its own rendering, which [`as_beancount_shows`] maps zhang's text to: a
//! tag without its `#` (`trip` for `#trip`), `True`, `False` and `None` for `TRUE`, `FALSE` and
//! `NULL`, and the result of an expression (`9` for `(1 + 2) * 3`, `1000.00` for `1,000.00`).

use std::collections::BTreeMap;
use std::path::PathBuf;

use beancount::Beancount;
use chrono::NaiveDate;
use serde_json::{json, Value};
use zhang_ast::{Directive, Meta, SpanInfo, Spanned};
use zhang_core::data_type::text::parser::{number_expr, posting_amount};
use zhang_core::data_type::DataType;

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/meta_values")
}

fn load() -> Vec<Spanned<Directive>> {
    let ledger = std::fs::read_to_string(fixture_dir().join("ledger.bean")).unwrap();
    Beancount::default().transform(ledger, None).expect("the ledger parses")
}

/// The text beancount shows for the value zhang keeps as `value`.
fn as_beancount_shows(value: &str) -> String {
    match value {
        "TRUE" => "True".to_owned(),
        "FALSE" => "False".to_owned(),
        "NULL" => "None".to_owned(),
        tag if tag.starts_with('#') => tag[1..].to_owned(),
        date if NaiveDate::parse_from_str(date, "%Y-%m-%d").is_ok() => date.to_owned(),
        other => match (posting_amount(other), number_expr(other)) {
            (Ok(("", amount)), _) => format!("{} {}", amount.number, amount.commodity),
            (_, Ok(("", number))) => number.to_string(),
            _ => other.to_owned(),
        },
    }
}

fn meta(meta: &Meta) -> Value {
    let pairs = meta
        .clone()
        .get_flatten()
        .into_iter()
        .map(|(key, value)| (key, as_beancount_shows(value.as_str())))
        .collect::<BTreeMap<_, _>>();
    json!(pairs)
}

/// The directive as a row of the oracle, `None` for the lines beancount has no entry for.
fn row(directive: &Directive) -> Option<Value> {
    let (kind, date, meta, account) = match directive {
        Directive::Open(it) => ("Open", &it.date, &it.meta, Some(&it.account)),
        Directive::Close(it) => ("Close", &it.date, &it.meta, Some(&it.account)),
        Directive::Commodity(it) => ("Commodity", &it.date, &it.meta, None),
        Directive::Note(it) => ("Note", &it.date, &it.meta, Some(&it.account)),
        Directive::Price(it) => ("Price", &it.date, &it.meta, None),
        Directive::Event(it) => ("Event", &it.date, &it.meta, None),
        Directive::Custom(it) => ("Custom", &it.date, &it.meta, None),
        Directive::Pad(it) => ("Pad", &it.date, &it.meta, Some(&it.account)),
        Directive::BalanceCheck(it) => ("Balance", &it.date, &it.meta, Some(&it.account)),
        Directive::Transaction(transaction) => {
            let postings = transaction
                .postings
                .iter()
                .map(|posting| json!({"account": posting.account.name(), "meta": self::meta(&posting.meta)}))
                .collect::<Vec<_>>();
            return Some(json!({
                "type": "Transaction",
                "date": transaction.date.naive_date().to_string(),
                "narration": transaction.narration.clone().map(|it| it.to_plain_string()),
                "meta": self::meta(&transaction.meta),
                "postings": postings,
            }));
        }
        Directive::Option(_) | Directive::Comment(_) => return None,
        other => panic!("unexpected directive {other:?}"),
    };
    let mut row = json!({"type": kind, "date": date.naive_date().to_string(), "meta": self::meta(meta)});
    if let Some(account) = account {
        row["account"] = json!(account.name());
    }
    Some(row)
}

#[test]
fn every_bare_metadata_value_reads_as_beancount_reads_it() {
    let oracle: Value = serde_json::from_str(&std::fs::read_to_string(fixture_dir().join("oracle.json")).unwrap()).unwrap();
    let rows = load().iter().filter_map(|directive| row(&directive.data)).collect::<Vec<_>>();
    assert_eq!(Value::Array(rows), oracle);
}

#[test]
fn every_bare_metadata_value_round_trips_through_the_beancount_exporter() {
    // the exporter writes a comment as its bare text, so comments are not round-tripped
    let original = load().into_iter().filter(|it| !matches!(it.data, Directive::Comment(_))).collect::<Vec<_>>();
    let exported = original
        .iter()
        .map(|directive| Beancount::default().export(Spanned::new(directive.data.clone(), SpanInfo::default())))
        .collect::<Vec<_>>()
        .join("\n\n");
    // each value is written back bare, as it was read
    for line in [
        "counterpart: Assets:Bank",
        "currency: USD",
        "number: 10.50",
        "negative: -3",
        "grouped: 1,000.00",
        "expression: (1 + 2) * 3",
        "amount: 10 USD",
        "amount-expression: 1 + 2 USD",
        "date: 2024-01-10",
        "tag: #trip",
        "yes: TRUE",
        "no: FALSE",
        "nothing: NULL",
    ] {
        assert!(
            exported.contains(&format!("\n  {line}\n")) || exported.contains(&format!("\n    {line}\n")),
            "{line}\n{exported}"
        );
    }
    let read = Beancount::default()
        .transform(exported.clone(), None)
        .unwrap_or_else(|error| panic!("{error}\n{exported}"));
    let data = |directives: &[Spanned<Directive>]| directives.iter().map(|it| it.data.clone()).collect::<Vec<_>>();
    assert_eq!(data(&read), data(&original));
}
