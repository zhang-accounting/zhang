//! Acceptance suite for #442 in beancount mode: quoted strings must survive an export / parse
//! round trip through the beancount data type.
//!
//! The suite drives only public entry points (`DataType::export`, `DataType::transform` and
//! `Ledger` loading) and pins the contract agreed for the fix:
//!
//! 1. any string the exporter writes into a quoted field parses back unchanged;
//! 2. the exporter never writes the legacy escapes `\$`, `` \` `` or `\u{...}`;
//! 3. the known escapes `\" \\ \/ \b \f \n \r \t \uXXXX` decode, and so do the legacy escapes
//!    older zhang wrote (`\$`, `` \` ``, `\u{H...}` with 1 to 6 hex digits);
//! 4. any other escape `\X` is kept verbatim (backslash included), unlike Python beancount;
//! 5. malformed escapes are reported as positioned errors, never as panics.

use std::panic::{catch_unwind, AssertUnwindSafe};

use beancount::Beancount;
use zhang_ast::Directive;
use zhang_core::ledger::Ledger;
use zhang_core::ZhangResult;
use zhang_testkit::dialect::escaping::{
    self, directive_with, field_value, legacy_escapes_in, panic_message, snippets, Field, Outcome, Rng, ALPHABET, EXAMPLES, FIELDS,
};
use zhang_testkit::ledger::Scratch;

// ---------------------------------------------------------------------------
// helpers: the shared scenarios of `zhang_testkit::dialect::escaping`, run through this dialect
// ---------------------------------------------------------------------------

fn transform(text: &str) -> Outcome {
    escaping::transform(&Beancount {}, text)
}

fn export(directive: Directive) -> Result<String, String> {
    escaping::export(&Beancount {}, directive)
}

fn round_trip(field: Field, s: &str) -> Result<(), String> {
    escaping::round_trip(&Beancount {}, field, s)
}

fn check_decodes(cases: &[(&str, &str)]) {
    escaping::check_decodes(&Beancount {}, cases)
}

// ---------------------------------------------------------------------------
// 1. round trip
// ---------------------------------------------------------------------------

#[test]
fn hand_picked_strings_round_trip_through_every_quoted_field() {
    let failures: Vec<String> = FIELDS
        .iter()
        .flat_map(|field| EXAMPLES.iter().filter_map(move |s| round_trip(*field, s).err()))
        .collect();
    assert!(failures.is_empty(), "{} round trips failed:\n{}", failures.len(), failures.join("\n"));
}

#[test]
fn generated_strings_round_trip_through_every_quoted_field() {
    const ITERATIONS: usize = 1500;
    let mut failures = vec![];
    let mut failure_count = 0;
    for field in FIELDS {
        let mut rng = Rng(0xA076_1D64_78BD_642F ^ field as u64);
        for _ in 0..ITERATIONS {
            let s = rng.string(ALPHABET, 24);
            if let Err(failure) = round_trip(field, &s) {
                failure_count += 1;
                if failures.len() < 20 {
                    failures.push(failure);
                }
            }
        }
    }
    assert!(
        failure_count == 0,
        "{failure_count} of {} generated round trips failed; first ones:\n{}",
        ITERATIONS * FIELDS.len(),
        failures.join("\n")
    );
}

// ---------------------------------------------------------------------------
// 2. exporter output
// ---------------------------------------------------------------------------

#[test]
fn exporter_writes_no_legacy_escapes() {
    let mut rng = Rng(0xE703_7ED1_A0B4_28DB);
    let generated = (0..500).map(|_| rng.string(ALPHABET, 24)).collect::<Vec<_>>();
    let mut violations = vec![];
    for field in FIELDS {
        for s in EXAMPLES.iter().copied().chain(generated.iter().map(String::as_str)) {
            match export(directive_with(field, s)) {
                Ok(exported) => {
                    let found = legacy_escapes_in(&exported);
                    if !found.is_empty() {
                        violations.push(format!("{field:?} {s:?} exports as {exported:?}, which contains {found:?}"));
                    }
                }
                Err(panic) => violations.push(format!("{field:?} {s:?}: exporter panicked: {panic}")),
            }
        }
    }
    let shown = violations.iter().take(20).cloned().collect::<Vec<_>>().join("\n");
    assert!(
        violations.is_empty(),
        "{} exports contain legacy escapes; first ones:\n{shown}",
        violations.len()
    );
}

// ---------------------------------------------------------------------------
// 3. decoding: known and legacy escapes
// ---------------------------------------------------------------------------

#[test]
fn escapes_decode_like_python_beancount() {
    // The expected strings were produced by Python beancount 3.2.3 (`beancount.loader.load_string`)
    // from the same text in a narration, an `open` metadata value and a `query` text; it decoded
    // all three alike. Beancount itself has no `\$` or `` \` `` escape: it drops the backslash of
    // any escape it does not know, which lands on the same text as zhang's legacy rule here.
    check_decodes(&[
        (r#"a \" b"#, "a \" b"),
        (r"a \\ b", "a \\ b"),
        (r"a \n b", "a \n b"),
        (r"a \t b", "a \t b"),
        (r"a \r b", "a \r b"),
        (r"a \b b", "a \u{8} b"),
        (r"a \f b", "a \u{c} b"),
        (r"a \/ b", "a / b"),
        (r"coffee \$5", "coffee $5"),
        (r"a \` b", "a ` b"),
        (r"\\d+", r"\d+"),
    ]);
}

#[test]
fn unicode_escapes_decode() {
    // Python beancount 3.2.3 has no unicode escape at all: it reads `\u00e9` as `u00e9` and
    // `\u{a0}` as `u{a0}`. zhang decodes both forms, as agreed for #442.
    check_decodes(&[
        (r"\u00e9", "é"),
        (r"\u00A0", "\u{a0}"),
        (r"\u2028", "\u{2028}"),
        (r"x\u{a0}y", "x\u{a0}y"),
        (r"SELECT\u{a0}account", "SELECT\u{a0}account"),
        (r"\u{2028}", "\u{2028}"),
        (r"nul\u{0}", "nul\u{0}"),
        (r"\u{9}", "\t"),
        (r"\u{1F600}", "\u{1F600}"),
        (r"\u{01F600}", "\u{1F600}"),
        (r"\u{10FFFF}", "\u{10FFFF}"),
    ]);
}

/// Beancount text as older zhang exported it, with `\$`, `` \` `` and `\u{...}` escapes.
const LEGACY_LEDGER: &str = r#"1970-01-01 commodity CNY
1970-01-01 open Assets:Cash
1970-01-01 open Expenses:Food
2024-01-02 * "Cafe" "coffee \$5"
  memo: "a \` b"
  Assets:Cash -5 CNY
  Expenses:Food 5 CNY
2024-01-03 note Assets:Cash "x\u{a0}y"
2024-01-04 event "mood" "\u{1F600}"
2024-01-05 query "q" "SELECT\u{a0}account"
"#;

#[test]
fn legacy_ledger_text_parses_to_the_original_strings() {
    let directives = match transform(LEGACY_LEDGER) {
        Outcome::Parsed(directives) => directives,
        Outcome::Error(error) => panic!("legacy ledger fails to parse: {error}"),
        Outcome::Panicked(panic) => panic!("legacy ledger panics the parser: {panic}"),
    };
    let values = |field: Field| directives.iter().filter_map(|it| field_value(field, &it.data)).collect::<Vec<_>>();
    assert_eq!(values(Field::Narration), vec!["coffee $5"]);
    assert_eq!(values(Field::TransactionMeta), vec!["a ` b"]);
    assert_eq!(values(Field::NoteComment), vec!["x\u{a0}y"]);
    assert_eq!(values(Field::EventDescription), vec!["\u{1F600}"]);
    assert_eq!(values(Field::QueryText), vec!["SELECT\u{a0}account"]);
}

#[test]
fn legacy_ledger_loads_without_errors() {
    let ledger = load_ledger(LEGACY_LEDGER).unwrap_or_else(|error| panic!("legacy ledger should load: {error}"));
    assert_eq!(error_kinds(&ledger), Vec::<String>::new());
}

// ---------------------------------------------------------------------------
// 4. unknown escapes are kept verbatim
// ---------------------------------------------------------------------------

#[test]
fn unknown_escapes_are_kept_verbatim() {
    // Deliberate difference from Python beancount 3.2.3, which drops the backslash of an
    // unknown escape: it loads `\d+` as `d+`, `\'` as `'` and `C:\Data\x` as `C:Datax`.
    // zhang keeps `\X` as written, in beancount mode too.
    check_decodes(&[
        (r"\d+", r"\d+"),
        (r"\w+\s*", r"\w+\s*"),
        (r"a\.b", r"a\.b"),
        (r"\(x\)", r"\(x\)"),
        (r"\[0-9\]", r"\[0-9\]"),
        (r"\'", r"\'"),
        (r"C:\Data\x", r"C:\Data\x"),
        (r"\é", r"\é"),
        ("\\\u{1F600}", "\\\u{1F600}"),
        // escapes pair up left to right: `\d`, then the escaped quote
        (r#"\d\""#, r#"\d""#),
        // escaped backslash, then the unknown `\d`
        (r"x\\\d", r"x\\d"),
    ]);
}

// ---------------------------------------------------------------------------
// 5. malformed escapes never panic
// ---------------------------------------------------------------------------

const HEADER: &str = "1970-01-01 commodity CNY\n1970-01-01 open Assets:Cash\n1970-01-01 open Expenses:Food\n";

/// A ledger whose fourth line carries `inner` inside a narration.
fn ledger_with_narration(inner: &str) -> String {
    format!("{HEADER}2024-01-02 * \"payee\" \"bad {inner} narration\"\n  Assets:Cash -5 CNY\n  Expenses:Food 5 CNY\n")
}

/// The contract's malformed inputs, each placed on line 4 of a ledger.
fn malformed_ledgers() -> Vec<(&'static str, String)> {
    vec![
        (r"\u{110000}", ledger_with_narration(r"\u{110000}")),
        (r"\u{D800}", ledger_with_narration(r"\u{D800}")),
        (r"\u{DFFF}", ledger_with_narration(r"\u{DFFF}")),
        (r"\uZZ", ledger_with_narration(r"\uZZ")),
        ("lone trailing backslash", format!("{HEADER}2024-01-02 note Assets:Cash \"abc\\")),
    ]
}

#[test]
fn malformed_escapes_are_errors_not_panics() {
    let mut failures = vec![];
    for (name, text) in malformed_ledgers() {
        match transform(&text) {
            Outcome::Error(_) => {}
            Outcome::Parsed(directives) => failures.push(format!("{name}: parsed into {directives:?}")),
            Outcome::Panicked(panic) => failures.push(format!("{name}: panicked: {panic}")),
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn malformed_escapes_in_a_ledger_report_the_offending_line() {
    let mut failures = vec![];
    for (name, text) in malformed_ledgers() {
        match catch_unwind(AssertUnwindSafe(|| load_ledger(&text))) {
            Ok(Err(error)) => {
                let message = error.to_string();
                if !message.contains("line 4") || message.contains("line 1,") {
                    failures.push(format!("{name}: error is not positioned at line 4: {message}"));
                }
            }
            Ok(Ok(_)) => failures.push(format!("{name}: ledger loaded")),
            Err(payload) => failures.push(format!("{name}: ledger load panicked: {}", panic_message(payload))),
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn odd_escapes_never_panic() {
    // whether these are errors or text is left to the implementation; they must not panic
    let fragments = [
        r"\uD800",
        r"\uDFFF",
        r"\u{}",
        r"\u{1234567}",
        r"\u{GG}",
        r"\u12",
        r"\u{a0",
        r"\u{-1}",
        r"\u{+a0}",
        r"\u{ a0}",
        r"\u",
        r"\u{d800}\u{dc00}",
        r"\ud83d\ude00",
        r"\a\v\e\0",
        "\\\n",
        r"\ ",
    ];
    let mut failures = vec![];
    for fragment in fragments {
        for (_, text) in snippets(fragment) {
            if let Outcome::Panicked(panic) = transform(&text) {
                failures.push(format!("{fragment:?}: panicked: {panic}"));
            }
        }
    }
    let unterminated = format!("{HEADER}2024-01-02 note Assets:Cash \"abc\\\"\n2024-01-03 note Assets:Cash \"def\"\n");
    if let Outcome::Panicked(panic) = transform(&unterminated) {
        failures.push(format!("escaped closing quote: panicked: {panic}"));
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn generated_raw_escape_text_never_panics() {
    const PIECES: &[&str] = &[
        "\\",
        "\\",
        "\\",
        "u",
        "{",
        "}",
        "0",
        "9",
        "a",
        "F",
        "D800",
        "DFFF",
        "110000",
        "10FFFF",
        "\"",
        "$",
        "`",
        "/",
        "n",
        "d",
        "'",
        " ",
        "\n",
        "é",
        "\u{1F600}",
    ];
    let mut rng = Rng(0x8EBC_6AF0_9C88_C6E3);
    let mut failures = vec![];
    for _ in 0..3000 {
        let inner = rng.string(PIECES, 16);
        let text = format!("2024-01-02 query \"saved\" \"{inner}\"\n");
        if let Outcome::Panicked(panic) = transform(&text) {
            if failures.len() < 20 {
                failures.push(format!("{text:?}: {panic}"));
            }
        }
    }
    assert!(failures.is_empty(), "the parser panicked on raw quoted text:\n{}", failures.join("\n"));
}

// ---------------------------------------------------------------------------
// 6. saved queries with regex escapes
// ---------------------------------------------------------------------------

fn load_ledger(content: &str) -> ZhangResult<Ledger> {
    Scratch::beancount(content).ledger()
}

fn error_kinds(ledger: &Ledger) -> Vec<String> {
    ledger.errors.iter().map(|it| format!("{:?}", it.error_type)).collect()
}

#[test]
fn query_with_an_unknown_regex_escape_loads_without_errors() {
    // Python beancount 3.2.3 loads this query with the text `... ~ 'd+'`; zhang keeps `\d+`.
    let ledger = load_ledger("2014-01-01 query \"x\" \"SELECT narration WHERE narration ~ '\\d+'\"\n")
        .unwrap_or_else(|error| panic!("a query with '\\d+' should load: {error}"));
    assert_eq!(error_kinds(&ledger), Vec::<String>::new());
    let queries = ledger.queries();
    let stored = queries.iter().map(|query| (query.name.as_str(), query.query.as_str())).collect::<Vec<_>>();
    assert_eq!(stored, vec![("x", r"SELECT narration WHERE narration ~ '\d+'")]);
}

#[test]
fn both_regex_spellings_store_the_same_query_text() {
    let ledger = load_ledger("2014-01-01 query \"single\" \"narration ~ '\\d+'\"\n2014-01-01 query \"double\" \"narration ~ '\\\\d+'\"\n")
        .unwrap_or_else(|error| panic!("both spellings should load: {error}"));
    assert_eq!(error_kinds(&ledger), Vec::<String>::new());
    let queries = ledger.queries();
    let texts = queries.iter().map(|query| query.query.as_str()).collect::<Vec<_>>();
    assert_eq!(texts, vec![r"narration ~ '\d+'", r"narration ~ '\d+'"]);
}
