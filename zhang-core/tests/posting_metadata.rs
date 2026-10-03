//! Acceptance tests for posting-level metadata in zhang's own text format.
//!
//! The zhang rule: a metadata line belongs to the posting before it only when it is
//! indented deeper than that posting's line. Any other metadata line is transaction
//! metadata, wherever it appears, so ledgers written by older zhang versions, which put the
//! transaction metadata after the postings at the postings' indentation, keep their meaning.
//!
//! Export writes the transaction metadata first, then each posting followed by its own
//! metadata indented deeper (posting at 2 spaces, its metadata at 4), and reading the
//! export back gives the same transaction.
//!
//! Written against the contract, not the implementation: a posting's metadata is
//! `Posting.meta` (see `support::posting_meta`), with the same `Meta` type as
//! `Transaction.meta`, and deserialising a posting without `meta` gives empty metadata.

#[path = "support/posting_metadata.rs"]
mod support;

use std::path::Path;
use std::sync::Arc;

use indoc::indoc;
use support::*;
use zhang_ast::*;
use zhang_core::data_source::LocalFileSystemDataSource;
use zhang_core::data_type::text::exporter::ZhangDataTypeExportable;
use zhang_core::data_type::text::ZhangDataType;
use zhang_core::data_type::DataType;
use zhang_core::domains::schemas::MetaType;
use zhang_core::ledger::Ledger;
use zhang_core::utils::string_::QuoteStyle;

fn transactions(text: &str) -> Vec<Transaction> {
    ZhangDataType {}
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
    assert_eq!(Shape::of(&parse_one(text)), expected, "zhang format, text:\n{text}");
}

fn export(txn: Transaction) -> String {
    ZhangDataType {}.export(Spanned::new(Directive::Transaction(txn), SpanInfo::default()))
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
// which metadata line belongs to what
// ---------------------------------------------------------------------------

#[test]
fn metadata_before_the_first_posting_belongs_to_the_transaction() {
    for indent in [" ", "  ", "    ", "      "] {
        let text = format!("2024-01-02 * \"Shop\" \"lunch\"\n{indent}note: \"t\"\n  Assets:Cash -10 CNY\n  Expenses:Food 10 CNY\n");
        assert_shape(&text, Shape::new(&[("note", "t")], &[("Assets:Cash", &[]), ("Expenses:Food", &[])]));
    }
}

#[test]
fn metadata_deeper_than_the_posting_before_it_belongs_to_that_posting() {
    // (posting indentation, metadata indentation)
    for (posting, meta) in [(2, 4), (2, 3), (2, 8), (1, 2), (4, 6), (4, 5)] {
        let (p, m) = (" ".repeat(posting), " ".repeat(meta));
        let text = format!("2024-01-02 * \"Shop\" \"lunch\"\n{p}Assets:Cash -10 CNY\n{m}receipt: \"r-1\"\n{p}Expenses:Food 10 CNY\n{m}category: \"meal\"\n");
        assert_shape(
            &text,
            Shape::new(&[], &[("Assets:Cash", &[("receipt", "r-1")]), ("Expenses:Food", &[("category", "meal")])]),
        );
    }
}

#[test]
fn metadata_at_the_indentation_of_the_posting_before_it_belongs_to_the_transaction() {
    // between the postings and after the last one
    assert_shape(
        indoc! {r#"
            2024-01-02 * "Shop" "lunch"
              Assets:Cash -10 CNY
              receipt: "r-1"
              Expenses:Food 10 CNY
              category: "meal"
        "#},
        Shape::new(&[("category", "meal"), ("receipt", "r-1")], &[("Assets:Cash", &[]), ("Expenses:Food", &[])]),
    );
    assert_shape(
        indoc! {r#"
            2024-01-02 * "Shop" "lunch"
                Assets:Cash -10 CNY
                receipt: "r-1"
                Expenses:Food 10 CNY
                category: "meal"
        "#},
        Shape::new(&[("category", "meal"), ("receipt", "r-1")], &[("Assets:Cash", &[]), ("Expenses:Food", &[])]),
    );
}

#[test]
fn metadata_shallower_than_the_posting_before_it_belongs_to_the_transaction() {
    assert_shape(
        indoc! {r#"
            2024-01-02 * "Shop" "lunch"
                Assets:Cash -10 CNY
              receipt: "r-1"
                Expenses:Food 10 CNY
             category: "meal"
        "#},
        Shape::new(&[("category", "meal"), ("receipt", "r-1")], &[("Assets:Cash", &[]), ("Expenses:Food", &[])]),
    );
}

#[test]
fn every_posting_keeps_its_own_metadata() {
    // transaction metadata before, between and after the postings; the same key on the
    // transaction and on postings; a key with two values on one posting
    assert_shape(
        indoc! {r#"
            2024-01-02 * "Shop" "lunch"
              note: "before"
              Assets:Cash -10 CNY
                receipt: "r-1"
                note: "cash"
              category: "between"
              Expenses:Food 7 CNY
                category: "food"
                document: "a.png"
                document: "b.png"
              Expenses:Drink 3 CNY
                category: "drink"
              note: "after"
        "#},
        Shape::new(
            &[("category", "between"), ("note", "after"), ("note", "before")],
            &[
                ("Assets:Cash", &[("note", "cash"), ("receipt", "r-1")]),
                ("Expenses:Food", &[("category", "food"), ("document", "a.png"), ("document", "b.png")]),
                ("Expenses:Drink", &[("category", "drink")]),
            ],
        ),
    );
    // several values of one key keep their order
    let txn = parse_one(indoc! {r#"
        2024-01-02 * "Shop" "lunch"
          Assets:Cash -10 CNY
            document: "a.png"
            document: "b.png"
          Expenses:Food 10 CNY
    "#});
    let documents: Vec<&str> = posting_meta(&txn.postings[0]).get_all("document").into_iter().map(|it| it.as_str()).collect();
    assert_eq!(documents, vec!["a.png", "b.png"]);
}

#[test]
fn comment_lines_do_not_change_which_posting_metadata_belongs_to() {
    assert_shape(
        indoc! {r#"
            2024-01-02 * "Shop" "lunch" ; header comment
              ; before the postings
              note: "t" ; trailing
              Assets:Cash -10 CNY ; posting comment
              ; at the posting's indentation
                ; deeper
                receipt: "r-1" ; trailing
                    ; deeper still
                memo: "m"
              ; between
              Expenses:Food 10 CNY
              // another comment style
                category: "meal"
              legacy: "after"
              ; last
        "#},
        Shape::new(
            &[("legacy", "after"), ("note", "t")],
            &[
                ("Assets:Cash", &[("memo", "m"), ("receipt", "r-1")]),
                ("Expenses:Food", &[("category", "meal")]),
            ],
        ),
    );
}

#[test]
fn a_posting_without_an_amount_takes_its_metadata() {
    assert_shape(
        indoc! {r#"
            2024-01-02 * "Shop" "lunch"
              Assets:Cash -10 CNY
              Expenses:Food
                category: "meal"
        "#},
        Shape::new(&[], &[("Assets:Cash", &[]), ("Expenses:Food", &[("category", "meal")])]),
    );
    assert_shape(
        indoc! {r#"
            2024-01-02 * "Shop" "lunch"
              Expenses:Food
                category: "meal"
              note: "t"
              Assets:Cash -10 CNY
        "#},
        Shape::new(&[("note", "t")], &[("Expenses:Food", &[("category", "meal")]), ("Assets:Cash", &[])]),
    );
}

#[test]
fn postings_at_different_indentations_each_use_their_own() {
    // the second posting is at 4 spaces: 4 is not deeper than it, 6 is
    assert_shape(
        indoc! {r#"
            2024-01-02 * "Shop" "lunch"
              Assets:Cash -10 CNY
                receipt: "r-1"
                Expenses:Food 10 CNY
                  category: "meal"
                legacy: "t"
        "#},
        Shape::new(
            &[("legacy", "t")],
            &[("Assets:Cash", &[("receipt", "r-1")]), ("Expenses:Food", &[("category", "meal")])],
        ),
    );
}

#[test]
fn a_transaction_without_postings_keeps_all_its_metadata() {
    assert_shape(
        "2024-01-02 * \"Shop\" \"lunch\"\n  note: \"t\"\n    deeper: \"d\"\n",
        Shape::new(&[("deeper", "d"), ("note", "t")], &[]),
    );
}

#[test]
fn quoted_keys_and_bare_values_attach_like_any_metadata() {
    assert_shape(
        indoc! {r#"
            2024-01-02 * "Shop" "lunch"
              "my key": 12
              Assets:Cash -10 CNY
                "my key": "posting"
                ";path": USD
                Receipt: TRUE
              Expenses:Food 10 CNY
        "#},
        Shape::new(
            &[("my key", "12")],
            &[
                ("Assets:Cash", &[(";path", "USD"), ("Receipt", "TRUE"), ("my key", "posting")]),
                ("Expenses:Food", &[]),
            ],
        ),
    );
    let txn = parse_one("2024-01-02 * \"Shop\" \"lunch\"\n  Assets:Cash -10 CNY\n    amount: 12\n    label: \"12\"\n");
    assert_eq!(posting_meta(&txn.postings[0]).get_one("amount"), Some(&unquote("12")));
    assert_eq!(posting_meta(&txn.postings[0]).get_one("label"), Some(&quote("12")));
}

#[test]
fn random_layouts_attach_by_the_zhang_rule() {
    let mut rng = XorShift(0x7a68_616e_6701);
    for _ in 0..1000 {
        let (text, expected) = random_layout(&mut rng, Rule::Zhang);
        let mut txn = parse_one(&text);
        assert_eq!(Shape::of(&txn), expected, "zhang format, text:\n{text}");
        // and what was read is written and read back unchanged (the exporter drops posting
        // comments, which is older and unrelated)
        for posting in &mut txn.postings {
            posting.comment = None;
        }
        assert_round_trips(&txn);
    }
}

// ---------------------------------------------------------------------------
// ledgers written before posting metadata existed
// ---------------------------------------------------------------------------

#[test]
fn transaction_metadata_after_the_postings_at_their_indentation_stays_on_the_transaction() {
    // the layout zhang's exporter wrote before #457, and the one `upload_transaction_document`
    // still appends: the transaction metadata after the postings, at their indentation
    assert_shape(
        indoc! {r#"
            2024-01-15 * "Bob" "coffee"
              Assets:Cash -5 CNY
              Expenses:Food 5 CNY
              document: "attachments/7d1c/receipt.png"
              note: "n"
        "#},
        Shape::new(
            &[("document", "attachments/7d1c/receipt.png"), ("note", "n")],
            &[("Assets:Cash", &[]), ("Expenses:Food", &[])],
        ),
    );
}

#[test]
fn the_existing_inline_comment_fixture_keeps_its_transaction_metadata() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../integration-tests/inline-comment-and-trailling-space/data.zhang");
    let text = std::fs::read_to_string(&fixture).unwrap();
    let txns = transactions(&text);
    assert_eq!(txns.len(), 1, "{text}");
    assert_eq!(
        Shape::of(&txns[0]),
        Shape::new(&[("a", "b"), ("b", "c")], &[("Assets:BankCard", &[]), ("Expenses:Food", &[])]),
        "{text}"
    );
}

const LEDGER: &str = indoc! {r#"
    option "operating_currency" "CNY"
    1970-01-01 commodity CNY
    1970-01-01 open Assets:Cash
    1970-01-01 open Expenses:Food

    2024-01-02 * "Shop" "lunch"
      note: "t"
      Assets:Cash -10 CNY
        receipt: "r-1"
      Expenses:Food 10 CNY
      legacy: "after the postings"
"#};

fn load(text: &str) -> (tempfile::TempDir, Ledger) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("main.zhang"), text).unwrap();
    let source = LocalFileSystemDataSource::new(ZhangDataType {});
    let ledger = Ledger::load_with_data_source(dir.path().to_path_buf(), "main.zhang".to_owned(), Arc::new(source))
        .unwrap_or_else(|err| panic!("ledger should load: {err}"));
    (dir, ledger)
}

#[test]
fn a_loaded_ledger_keeps_posting_metadata_apart_from_transaction_metadata() {
    let (_dir, ledger) = load(LEDGER);
    let txns: Vec<&Transaction> = ledger
        .directives
        .iter()
        .filter_map(|it| match &it.data {
            Directive::Transaction(txn) => Some(txn),
            _ => None,
        })
        .collect();
    assert_eq!(txns.len(), 1);
    assert_eq!(
        Shape::of(txns[0]),
        Shape::new(
            &[("legacy", "after the postings"), ("note", "t")],
            &[("Assets:Cash", &[("receipt", "r-1")]), ("Expenses:Food", &[])],
        )
    );

    let operations = ledger.operations();
    let store = operations.read();
    assert!(
        store.errors.is_empty(),
        "{:?}",
        store.errors.iter().map(|it| &it.error_type).collect::<Vec<_>>()
    );
    let ids: Vec<String> = store.transactions.values().map(|it| it.id.to_string()).collect();
    drop(store);
    assert_eq!(ids.len(), 1);
    let keys: Vec<(String, String)> = operations
        .metas(MetaType::TransactionMeta, &ids[0])
        .unwrap()
        .into_iter()
        .map(|it| (it.key, it.value))
        .collect();
    for key in ["note", "legacy"] {
        assert!(keys.iter().any(|(k, _)| k == key), "transaction meta {key} is in the store: {keys:?}");
    }
    assert!(
        !keys.iter().any(|(k, _)| k == "receipt"),
        "the posting's metadata is not transaction metadata in the store: {keys:?}"
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
    assert_eq!(lunch().export_as(QuoteStyle::Zhang).trim_end_matches('\n'), LUNCH.trim_end_matches('\n'));
    assert_eq!(lunch().export().trim_end_matches('\n'), LUNCH.trim_end_matches('\n'));
    assert_eq!(Shape::of(&parse_one(LUNCH)), Shape::of(&lunch()));
}

#[test]
fn export_writes_every_metadata_line_of_a_posting_under_it() {
    let txn = transaction(
        vec![
            posting(
                "Assets:Broker",
                Some(("1", "HOOL")),
                &[
                    ("lot", quote("a")),
                    ("my key", quote("v")),
                    ("Receipt", unquote("12")),
                    ("document", quote("b.png")),
                ],
            ),
            posting("Assets:Cash", Some(("-2", "USD")), &[("document", quote("c.png"))]),
        ],
        &[("trip", quote("rome")), ("note", quote("t"))],
    );
    let mut txn = txn;
    txn.postings[0].cost = Some(PostingCost {
        base: Some(zhang_ast::amount::Amount::new(2.into(), "USD")),
        date: None,
        label: None,
        total: false,
    });
    txn.postings[0].price = Some(SingleTotalPrice::Single(zhang_ast::amount::Amount::new(3.into(), "USD")));
    let exported = export(txn.clone());
    let lines: Vec<&str> = exported.trim_end_matches('\n').split('\n').collect();
    assert_eq!(lines.len(), 1 + 2 + 1 + 4 + 1 + 1, "{exported}");
    fn sorted<'a>(lines: &[&'a str]) -> Vec<&'a str> {
        let mut lines = lines.to_vec();
        lines.sort();
        lines
    }
    assert_eq!(sorted(&lines[1..3]), vec!["  note: \"t\"", "  trip: \"rome\""], "{exported}");
    assert_eq!(lines[3], "  Assets:Broker 1 HOOL { 2 USD } @ 3 USD", "{exported}");
    assert_eq!(
        sorted(&lines[4..8]),
        vec!["    \"my key\": \"v\"", "    Receipt: 12", "    document: \"b.png\"", "    lot: \"a\""],
        "{exported}"
    );
    assert_eq!(lines[8], "  Assets:Cash -2 USD", "{exported}");
    assert_eq!(lines[9], "    document: \"c.png\"", "{exported}");
    assert_round_trips(&txn);
}

#[test]
fn a_posting_without_metadata_is_written_as_before() {
    let txn = transaction(
        vec![
            posting("Assets:Cash", Some(("-10", "CNY")), &[]),
            posting("Expenses:Food", Some(("10", "CNY")), &[]),
        ],
        &[("note", quote("t"))],
    );
    assert_eq!(
        export(txn),
        "2024-01-02 * \"Shop\" \"lunch\"\n  note: \"t\"\n  Assets:Cash -10 CNY\n  Expenses:Food 10 CNY"
    );
}

// ---------------------------------------------------------------------------
// round trip
// ---------------------------------------------------------------------------

#[test]
fn keys_and_values_that_need_quoting_or_escaping_round_trip() {
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
    let mut rng = XorShift(0x0b5e_55ed_7a68);
    for _ in 0..1000 {
        assert_round_trips(&random_transaction(&mut rng));
    }
}

#[test]
fn a_whole_exported_ledger_reads_back() {
    let mut rng = XorShift(0x1ed9_e700_7a68);
    let txns: Vec<Transaction> = (0..50).map(|_| random_transaction(&mut rng)).collect();
    let text = txns.iter().cloned().map(export).collect::<Vec<_>>().join("\n\n");
    assert_eq!(transactions(&format!("{text}\n")), txns, "{text}");
}

// ---------------------------------------------------------------------------
// plugins built against an older zhang-ast
// ---------------------------------------------------------------------------

/// A posting as zhang 7007dd0 (before posting metadata) serialised it for a plugin.
const OLD_POSTING_JSON: &str = r#"{"flag":null,"account":{"account_type":"Expenses","content":"Expenses:Food","components":["Food"]},"units":{"number":"10","commodity":"CNY"},"cost":{"base":{"number":"1","commodity":"USD"},"date":null,"label":null,"total":false},"price":{"Single":{"number":"2","commodity":"EUR"}},"comment":null}"#;

/// A directive stream as zhang 7007dd0 serialised it, `__FILE__` standing for the source file.
const OLD_STREAM_JSON: &str = r#"[{"data":{"Commodity":{"date":{"Date":"1970-01-01"},"currency":"CNY","meta":{"inner":{}}}},"span":{"start":0,"end":24,"content":"1970-01-01 commodity CNY","filename":"__FILE__"}},{"data":{"Open":{"date":{"Date":"1970-01-01"},"account":{"account_type":"Assets","content":"Assets:Cash","components":["Cash"]},"commodities":[],"meta":{"inner":{}}}},"span":{"start":25,"end":52,"content":"1970-01-01 open Assets:Cash","filename":"__FILE__"}},{"data":{"Open":{"date":{"Date":"1970-01-01"},"account":{"account_type":"Expenses","content":"Expenses:Food","components":["Food"]},"commodities":[],"meta":{"inner":{}}}},"span":{"start":53,"end":82,"content":"1970-01-01 open Expenses:Food","filename":"__FILE__"}},{"data":{"Transaction":{"date":{"Date":"2024-01-02"},"flag":"Okay","payee":{"QuoteString":"Shop"},"narration":{"QuoteString":"lunch"},"tags":["food"],"links":["inv-1"],"postings":[{"flag":null,"account":{"account_type":"Assets","content":"Assets:Cash","components":["Cash"]},"units":{"number":"-10","commodity":"CNY"},"cost":null,"price":null,"comment":null},{"flag":null,"account":{"account_type":"Expenses","content":"Expenses:Food","components":["Food"]},"units":{"number":"10","commodity":"CNY"},"cost":null,"price":null,"comment":null}],"meta":{"inner":{"note":[{"QuoteString":"from plugin"}]}}}},"span":{"start":84,"end":191,"content":"2024-01-02 * \"Shop\" \"lunch\" #food ^inv-1\n  note: \"from plugin\"\n  Assets:Cash -10 CNY\n  Expenses:Food 10 CNY","filename":"__FILE__"}}]"#;

#[test]
fn a_posting_from_an_older_plugin_has_no_metadata() {
    let posting: Posting = serde_json::from_str(OLD_POSTING_JSON).expect("an older plugin's posting deserialises");
    assert_eq!(posting.account.content, "Expenses:Food");
    assert_eq!(posting.units.as_ref().map(|it| it.commodity.as_str()), Some("CNY"));
    assert!(posting.cost.is_some() && posting.price.is_some());
    assert_eq!(posting_meta(&posting), &Meta::default());

    // what the plugin host decodes: the stream of a processor, a directive of a mapper
    let stream: Vec<Spanned<Directive>> = serde_json::from_str(OLD_STREAM_JSON).expect("an older plugin's stream deserialises");
    let Directive::Transaction(txn) = &stream[3].data else {
        panic!("expected a transaction, got {:?}", stream[3].data);
    };
    assert_eq!(
        Shape::of(txn),
        Shape::new(&[("note", "from plugin")], &[("Assets:Cash", &[]), ("Expenses:Food", &[])])
    );
    let value: serde_json::Value = serde_json::from_str(OLD_STREAM_JSON).unwrap();
    let single: Spanned<Directive> = serde_json::from_value(value[3].clone()).expect("a single older directive deserialises");
    assert_eq!(single.data, stream[3].data);
}

#[test]
fn posting_metadata_survives_the_plugin_json() {
    let txn = lunch();
    let json = serde_json::to_string(&vec![Spanned::new(Directive::Transaction(txn.clone()), SpanInfo::default())]).unwrap();
    let back: Vec<Spanned<Directive>> = serde_json::from_str(&json).unwrap();
    assert_eq!(back[0].data, Directive::Transaction(txn), "{json}");
}

/// The same older stream, returned by a real WASM processor plugin.
#[cfg(feature = "plugin_runtime")]
mod older_wasm_plugin {
    use super::*;

    /// A processor plugin, in WAT, that ignores its input and returns `output`.
    fn constant_processor(output: &str) -> String {
        let escaped: String = output
            .bytes()
            .map(|byte| match byte {
                b'"' | b'\\' => format!("\\{:02x}", byte),
                0x20..=0x7e => (byte as char).to_string(),
                _ => format!("\\{:02x}", byte),
            })
            .collect();
        format!(
            r#"(module
  (import "extism:host/env" "alloc" (func $alloc (param i64) (result i64)))
  (import "extism:host/env" "store_u8" (func $store_u8 (param i64 i32)))
  (import "extism:host/env" "output_set" (func $output_set (param i64 i64)))
  (memory 1)
  (data (i32.const 0) "\"legacy\"")
  (data (i32.const 16) "\"0.1.0\"")
  (data (i32.const 32) "[\"Processor\"]")
  (data (i32.const 64) "{escaped}")
  (func $output_bytes (param $ptr i32) (param $len i32)
    (local $block i64)
    (local $i i32)
    (local.set $block (call $alloc (i64.extend_i32_u (local.get $len))))
    (block $done
      (loop $copy
        (br_if $done (i32.ge_u (local.get $i) (local.get $len)))
        (call $store_u8
          (i64.add (local.get $block) (i64.extend_i32_u (local.get $i)))
          (i32.load8_u (i32.add (local.get $ptr) (local.get $i))))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $copy)))
    (call $output_set (local.get $block) (i64.extend_i32_u (local.get $len))))
  (func (export "name") (result i32)
    (call $output_bytes (i32.const 0) (i32.const 8))
    (i32.const 0))
  (func (export "version") (result i32)
    (call $output_bytes (i32.const 16) (i32.const 7))
    (i32.const 0))
  (func (export "supported_type") (result i32)
    (call $output_bytes (i32.const 32) (i32.const 13))
    (i32.const 0))
  (func (export "processor") (result i32)
    (call $output_bytes (i32.const 64) (i32.const {len}))
    (i32.const 0)))
"#,
            len = output.len()
        )
    }

    #[test]
    fn an_older_processor_plugin_stream_loads() {
        let dir = tempfile::tempdir().unwrap();
        let main = dir.path().join("main.zhang");
        let file = serde_json::to_string(&main.display().to_string()).unwrap();
        let stream = OLD_STREAM_JSON.replace("\"__FILE__\"", &file);
        let plugin = dir.path().join("legacy.wat");
        std::fs::write(&plugin, constant_processor(&stream)).unwrap();
        std::fs::write(
            &main,
            format!(
                "option \"features.plugin\" \"true\"\nplugin \"{}\"\n1970-01-01 commodity CNY\n",
                plugin.display()
            ),
        )
        .unwrap();
        let source = LocalFileSystemDataSource::new(ZhangDataType {});
        let ledger = Ledger::load_with_data_source(dir.path().to_path_buf(), "main.zhang".to_owned(), Arc::new(source))
            .unwrap_or_else(|err| panic!("the ledger with an older plugin should load: {err}"));

        let txns: Vec<&Transaction> = ledger
            .directives
            .iter()
            .filter_map(|it| match &it.data {
                Directive::Transaction(txn) => Some(txn),
                _ => None,
            })
            .collect();
        assert_eq!(txns.len(), 1, "the plugin's transaction is loaded");
        assert_eq!(
            Shape::of(txns[0]),
            Shape::new(&[("note", "from plugin")], &[("Assets:Cash", &[]), ("Expenses:Food", &[])])
        );
        let store = ledger.store.read().unwrap();
        assert!(
            store.errors.is_empty(),
            "{:?}",
            store.errors.iter().map(|it| &it.error_type).collect::<Vec<_>>()
        );
        assert_eq!(store.postings.len(), 2, "both postings reach the store");
    }
}
