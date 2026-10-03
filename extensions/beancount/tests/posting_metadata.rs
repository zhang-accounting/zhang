//! Acceptance tests for posting-level metadata in the beancount format.
//!
//! The beancount rule, as beancount 3.2.3 reads it: a metadata line before the first posting
//! belongs to the transaction, and one after a posting belongs to the latest posting,
//! whatever its indentation (the same as the posting's, deeper, shallower, spaces or tabs)
//! and with indented comment lines in between. Every expectation marked as produced by
//! beancount 3.2.3 is what `beancount.parser.parser.parse_string` returned for the same text.
//!
//! Export writes the transaction metadata first, then each posting followed by its own
//! metadata indented deeper (posting at 2 spaces, its metadata at 4), and reading the
//! export back gives the same transaction.
//!
//! Written against the contract, not the implementation: a posting's metadata is
//! `Posting.meta` (see `support::posting_meta`), with the same `Meta` type as
//! `Transaction.meta`.

#[path = "../../../zhang-core/tests/support/posting_metadata.rs"]
mod support;

use beancount::Beancount;
use indoc::indoc;
use support::*;
use zhang_ast::*;
use zhang_core::data_type::text::ZhangDataType;
use zhang_core::data_type::DataType;

fn transactions(text: &str) -> Vec<Transaction> {
    Beancount::default()
        .transform(text.to_owned(), None)
        .unwrap_or_else(|err| panic!("cannot parse {text:?}: {err}"))
        .into_iter()
        .filter_map(|it| match it.data {
            Directive::Transaction(txn) => Some(txn),
            _ => None,
        })
        .collect()
}

fn parse_one(text: &str) -> Transaction {
    let mut txns = transactions(text);
    assert_eq!(txns.len(), 1, "expected one transaction in {text:?}");
    txns.remove(0)
}

fn assert_shape(text: &str, expected: Shape) {
    assert_eq!(Shape::of(&parse_one(text)), expected, "beancount format, text:\n{text}");
}

fn export(txn: Transaction) -> String {
    Beancount::default().export(Spanned::new(Directive::Transaction(txn), SpanInfo::default()))
}

/// Export then parse gives `txn` back, and the export is laid out as the contract says.
fn assert_round_trips(txn: &Transaction) {
    let exported = export(txn.clone());
    assert_export_layout(&exported, txn);
    let reparsed = parse_one(&exported);
    assert_eq!(&reparsed, txn, "exported as:\n{exported}");
    assert_eq!(Shape::of(&reparsed), Shape::of(txn), "exported as:\n{exported}");
}

/// The exported text is the header, the transaction metadata at 2 spaces, then each posting
/// at 2 spaces followed by exactly its own metadata lines at 4 spaces.
fn assert_export_layout(exported: &str, txn: &Transaction) {
    let lines: Vec<&str> = exported.trim_end_matches('\n').split('\n').collect();
    let indent = |line: &str| line.len() - line.trim_start_matches(' ').len();
    let txn_lines = txn.meta.clone().get_flatten().len();
    let mut expected_indents = vec![2; txn_lines];
    for posting in &txn.postings {
        expected_indents.push(2);
        expected_indents.extend(std::iter::repeat_n(4, posting_meta(posting).clone().get_flatten().len()));
    }
    let indents: Vec<usize> = lines.iter().skip(1).map(|line| indent(line)).collect();
    assert_eq!(indents, expected_indents, "indentation of each line after the header of:\n{exported}");
    for (line, posting) in lines.iter().skip(1 + txn_lines).filter(|line| indent(line) == 2).zip(&txn.postings) {
        assert!(
            line.trim_start().starts_with(&posting.account.content),
            "expected the posting on {} at {line:?} in:\n{exported}",
            posting.account.content
        );
    }
}

// ---------------------------------------------------------------------------
// which metadata line belongs to what, checked against beancount 3.2.3
// ---------------------------------------------------------------------------

#[test]
fn metadata_before_the_first_posting_belongs_to_the_transaction() {
    // produced by beancount 3.2.3: txn {note: t}, both postings without metadata, for each indentation
    for indent in [" ", "  ", "    "] {
        let text = format!("2024-01-02 * \"Shop\" \"lunch\"\n{indent}note: \"t\"\n  Assets:Cash -10 USD\n  Expenses:Food 10 USD\n");
        assert_shape(&text, Shape::new(&[("note", "t")], &[("Assets:Cash", &[]), ("Expenses:Food", &[])]));
    }
}

#[test]
fn metadata_after_a_posting_belongs_to_it_at_the_same_indentation() {
    // produced by beancount 3.2.3
    assert_shape(
        indoc! {r#"
            2024-01-02 * "Shop" "lunch"
              Assets:Cash -10 USD
              receipt: "r-1"
              Expenses:Food 10 USD
              category: "meal"
        "#},
        Shape::new(&[], &[("Assets:Cash", &[("receipt", "r-1")]), ("Expenses:Food", &[("category", "meal")])]),
    );
}

#[test]
fn metadata_after_a_posting_belongs_to_it_when_deeper() {
    // produced by beancount 3.2.3
    assert_shape(
        indoc! {r#"
            2024-01-02 * "Shop" "lunch"
              Assets:Cash -10 USD
                receipt: "r-1"
              Expenses:Food 10 USD
                category: "meal"
        "#},
        Shape::new(&[], &[("Assets:Cash", &[("receipt", "r-1")]), ("Expenses:Food", &[("category", "meal")])]),
    );
}

#[test]
fn metadata_after_a_posting_belongs_to_it_when_shallower_or_tab_indented() {
    // produced by beancount 3.2.3: the indentation does not matter
    let expected = || Shape::new(&[], &[("Assets:Cash", &[("receipt", "r-1")]), ("Expenses:Food", &[("category", "meal")])]);
    assert_shape(
        "2024-01-02 * \"Shop\" \"lunch\"\n    Assets:Cash -10 USD\n  receipt: \"r-1\"\n    Expenses:Food 10 USD\n category: \"meal\"\n",
        expected(),
    );
    assert_shape(
        "2024-01-02 * \"Shop\" \"lunch\"\n\tAssets:Cash -10 USD\n\treceipt: \"r-1\"\n  Expenses:Food 10 USD\n\t  category: \"meal\"\n",
        expected(),
    );
}

#[test]
fn every_posting_keeps_its_own_metadata() {
    // produced by beancount 3.2.3
    assert_shape(
        indoc! {r#"
            2024-01-02 * "Shop" "lunch"
              note: "before"
              Assets:Cash -10 USD
                receipt: "r-1"
              note: "cash"
              Expenses:Food 7 USD
              category: "food"
                  document: "a.png"
              Expenses:Drink 3 USD
                category: "drink"
              trip: "after"
        "#},
        Shape::new(
            &[("note", "before")],
            &[
                ("Assets:Cash", &[("note", "cash"), ("receipt", "r-1")]),
                ("Expenses:Food", &[("category", "food"), ("document", "a.png")]),
                ("Expenses:Drink", &[("category", "drink"), ("trip", "after")]),
            ],
        ),
    );
    // produced by beancount 3.2.3: the same key on the transaction and on a posting
    assert_shape(
        indoc! {r#"
            2024-01-02 * "Shop" "lunch"
              category: "txn"
              Assets:Cash -10 USD
                category: "posting"
                document: "a.png"
              Expenses:Food 10 USD
        "#},
        Shape::new(
            &[("category", "txn")],
            &[("Assets:Cash", &[("category", "posting"), ("document", "a.png")]), ("Expenses:Food", &[])],
        ),
    );
}

#[test]
fn comment_lines_do_not_change_which_posting_metadata_belongs_to() {
    // produced by beancount 3.2.3
    assert_shape(
        indoc! {r#"
            2024-01-02 * "Shop" "lunch"
              ; before the postings
              note: "t" ; trailing
              Assets:Cash -10 USD ; posting comment
              ; at the posting indentation
                ; deeper
                receipt: "r-1" ; trailing
                    ; deeper still
              memo: "m"
              ; between
              Expenses:Food 10 USD
              category: "meal"
              ; last
        "#},
        Shape::new(
            &[("note", "t")],
            &[
                ("Assets:Cash", &[("memo", "m"), ("receipt", "r-1")]),
                ("Expenses:Food", &[("category", "meal")]),
            ],
        ),
    );
}

#[test]
fn a_posting_without_an_amount_takes_its_metadata() {
    // produced by beancount 3.2.3
    assert_shape(
        indoc! {r#"
            2024-01-02 * "Shop" "lunch"
              Assets:Cash -10 USD
              Expenses:Food
              category: "meal"
        "#},
        Shape::new(&[], &[("Assets:Cash", &[]), ("Expenses:Food", &[("category", "meal")])]),
    );
    // produced by beancount 3.2.3
    assert_shape(
        indoc! {r#"
            2024-01-02 * "Shop" "lunch"
              Expenses:Food
                category: "meal"
              Assets:Cash -10 USD
              memo: "m"
        "#},
        Shape::new(&[], &[("Expenses:Food", &[("category", "meal")]), ("Assets:Cash", &[("memo", "m")])]),
    );
}

#[test]
fn a_transaction_without_postings_keeps_all_its_metadata() {
    // produced by beancount 3.2.3: txn {deeper: d, note: t}
    assert_shape(
        "2024-01-02 * \"Shop\" \"lunch\"\n  note: \"t\"\n    deeper: \"d\"\n",
        Shape::new(&[("deeper", "d"), ("note", "t")], &[]),
    );
}

#[test]
fn a_posting_with_cost_and_price_takes_its_metadata() {
    // produced by beancount 3.2.3
    assert_shape(
        "2024-01-02 * \"Shop\" \"lunch\"\n  Assets:Broker 1 HOOL {2 USD} @ 3 USD\n    lot: \"a\"\n  Assets:Cash -2 USD\n",
        Shape::new(&[], &[("Assets:Broker", &[("lot", "a")]), ("Assets:Cash", &[])]),
    );
}

#[test]
fn pushed_metadata_and_tags_go_to_the_transaction_only() {
    // produced by beancount 3.2.3: txn {source: import}, Assets:Cash {receipt: r-1}
    assert_shape(
        indoc! {r#"
            pushmeta source: "import"
            2024-01-02 * "Shop" "lunch"
              Assets:Cash -10 USD
                receipt: "r-1"
              Expenses:Food 10 USD
            popmeta source:
        "#},
        Shape::new(&[("source", "import")], &[("Assets:Cash", &[("receipt", "r-1")]), ("Expenses:Food", &[])]),
    );
    // produced by beancount 3.2.3: tags {trip}, Assets:Cash {receipt: r-1}
    let txn = parse_one(indoc! {r#"
        pushtag #trip
        2024-01-02 * "Shop" "lunch"
          Assets:Cash -10 USD
            receipt: "r-1"
          Expenses:Food 10 USD
        poptag #trip
    "#});
    assert_eq!(txn.tags.iter().collect::<Vec<_>>(), vec!["trip"]);
    assert_eq!(
        Shape::of(&txn),
        Shape::new(&[], &[("Assets:Cash", &[("receipt", "r-1")]), ("Expenses:Food", &[])])
    );
}

#[test]
fn random_layouts_attach_by_the_beancount_rule() {
    // the rule was checked on 600 random layouts of the same kind with beancount 3.2.3
    let mut rng = XorShift(0x6265_616e_0001);
    for _ in 0..1000 {
        let (text, expected) = random_layout(&mut rng, Rule::Beancount);
        let mut txn = parse_one(&text);
        assert_eq!(Shape::of(&txn), expected, "beancount format, text:\n{text:?}");
        // and what was read is written and read back unchanged (the exporter drops posting
        // comments, which is older and unrelated)
        for posting in &mut txn.postings {
            posting.comment = None;
        }
        assert_round_trips(&txn);
    }
}

#[test]
fn the_same_text_attaches_by_each_format_s_own_rule() {
    let text = indoc! {r#"
        2024-01-02 * "Shop" "lunch"
          note: "t"
          Assets:Cash -10 USD
          receipt: "r-1"
          Expenses:Food 10 USD
            category: "meal"
    "#};
    // produced by beancount 3.2.3
    assert_shape(
        text,
        Shape::new(
            &[("note", "t")],
            &[("Assets:Cash", &[("receipt", "r-1")]), ("Expenses:Food", &[("category", "meal")])],
        ),
    );
    // zhang: `receipt` is not deeper than its posting, so it stays on the transaction
    let zhang = ZhangDataType {}.transform(text.to_owned(), None).unwrap();
    let Directive::Transaction(txn) = &zhang[0].data else {
        panic!("expected a transaction, got {:?}", zhang[0].data);
    };
    assert_eq!(
        Shape::of(txn),
        Shape::new(
            &[("note", "t"), ("receipt", "r-1")],
            &[("Assets:Cash", &[]), ("Expenses:Food", &[("category", "meal")])],
        )
    );
}

// ---------------------------------------------------------------------------
// export
// ---------------------------------------------------------------------------

fn lunch() -> Transaction {
    transaction(
        vec![
            posting("Assets:Cash", Some(("-10", "CNY")), &[("receipt", quote("r-1"))]),
            posting("Expenses:Food", Some(("7", "CNY")), &[]),
            posting("Expenses:Tips", None, &[("memo", quote("say \"hi\" \\ bye"))]),
        ],
        &[("note", quote("t"))],
    )
}

/// beancount 3.2.3 reads this text as txn {note: t}, Assets:Cash {receipt: r-1},
/// Expenses:Food {} and Expenses:Tips {memo: say "hi" \ bye}.
const LUNCH: &str = indoc! {r#"
    2024-01-02 * "Shop" "lunch"
      note: "t"
      Assets:Cash -10 CNY
        receipt: "r-1"
      Expenses:Food 7 CNY
      Expenses:Tips
        memo: "say \"hi\" \\ bye"
"#};

#[test]
fn export_writes_transaction_metadata_first_then_each_posting_with_its_metadata_deeper() {
    assert_eq!(export(lunch()).trim_end_matches('\n'), LUNCH.trim_end_matches('\n'));
    assert_eq!(Shape::of(&parse_one(LUNCH)), Shape::of(&lunch()));
}

#[test]
fn export_writes_a_datetime_transaction_with_its_posting_metadata() {
    // the beancount exporter moves the time into the transaction's `time` metadata
    let mut txn = lunch();
    txn.date = Date::Datetime(chrono::NaiveDate::from_ymd_opt(2024, 1, 2).unwrap().and_hms_opt(8, 30, 0).unwrap());
    let exported = export(txn.clone());
    let lines: Vec<&str> = exported.trim_end_matches('\n').split('\n').collect();
    let mut head = lines[1..3].to_vec();
    head.sort();
    assert_eq!(head, vec!["  note: \"t\"", "  time: \"08:30:00\""], "{exported}");
    assert_eq!(
        lines[3..].join("\n"),
        LUNCH.trim_end_matches('\n').split('\n').skip(2).collect::<Vec<_>>().join("\n"),
        "{exported}"
    );
    assert_eq!(parse_one(&exported), txn, "{exported}");
}

// ---------------------------------------------------------------------------
// round trip
// ---------------------------------------------------------------------------

#[test]
fn keys_and_values_that_need_quoting_or_escaping_round_trip() {
    // beancount has no quoted keys; zhang's beancount parser reads them back, as it does
    // for transaction metadata
    let keys = BARE_KEYS.iter().chain(AWKWARD_KEYS);
    let values = BARE_VALUES.iter().map(|it| unquote(it)).chain(AWKWARD_VALUES.iter().map(|it| quote(it)));
    for (key, value) in keys.zip(values.cycle()) {
        let txn = transaction(
            vec![
                posting("Assets:Cash", Some(("-10", "CNY")), &[(key, value.clone())]),
                posting("Expenses:Food", None, &[(key, quote("second")), ("other", value.clone())]),
            ],
            &[(key, value.clone())],
        );
        assert_round_trips(&txn);
    }
}

#[test]
fn random_transactions_round_trip() {
    let mut rng = XorShift(0x0b5e_55ed_bea7);
    for _ in 0..1000 {
        assert_round_trips(&random_transaction(&mut rng));
    }
}

#[test]
fn a_whole_exported_ledger_reads_back() {
    let mut rng = XorShift(0x1ed9_e700_bea7);
    let txns: Vec<Transaction> = (0..50).map(|_| random_transaction(&mut rng)).collect();
    let text = txns.iter().cloned().map(export).collect::<Vec<_>>().join("\n\n");
    assert_eq!(transactions(&format!("{text}\n")), txns, "{text}");
}
