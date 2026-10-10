//! The string-escaping scenarios (#442), shared by `zhang-core/tests/core/string_escaping.rs` and
//! `extensions/beancount/tests/beancount/string_escaping.rs`: every quoted field a string can be exported into
//! ([`Field`]), a directive holding a string in one of them ([`directive_with`]) and the value read back
//! ([`field_value`]), the round trip through a dialect's exporter and parser with panics caught ([`round_trip`]),
//! the decoding of raw quoted text in every snippet position ([`check_decodes`]), the scan for the legacy escapes
//! the exporters must not write ([`legacy_escapes_in`]), and the generated and hand-picked inputs ([`Rng`],
//! [`ALPHABET`], [`EXAMPLES`]). What each dialect is expected to do with them stays in the tests.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::str::FromStr;

use bigdecimal::BigDecimal;
use chrono::NaiveDate;
use zhang_ast::amount::Amount;
use zhang_ast::{
    Account, Custom, Date, Directive, Event, Flag, Meta, Note, Open, Posting, Query, SpanInfo, Spanned, StringOrAccount, Transaction, ZhangString,
};
use zhang_core::data_type::DataType;

/// Every quoted field kind a string can be exported into.
#[derive(Clone, Copy, Debug)]
pub enum Field {
    Payee,
    Narration,
    TransactionMeta,
    OpenMeta,
    QueryName,
    QueryText,
    NoteComment,
    EventDescription,
    CustomValue,
}

pub const FIELDS: [Field; 9] = [
    Field::Payee,
    Field::Narration,
    Field::TransactionMeta,
    Field::OpenMeta,
    Field::QueryName,
    Field::QueryText,
    Field::NoteComment,
    Field::EventDescription,
    Field::CustomValue,
];

/// Outcome of handing text to the parser, with panics caught.
pub enum Outcome {
    Parsed(Vec<Spanned<Directive>>),
    Error(String),
    Panicked(String),
}

pub fn panic_message(payload: Box<dyn std::any::Any + Send>) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|message| message.to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "non-string panic payload".to_owned())
}

/// Hand `text` to the parser of `data_type`, with panics caught.
pub fn transform<D: DataType<Carrier = String>>(data_type: &D, text: &str) -> Outcome {
    match catch_unwind(AssertUnwindSafe(|| data_type.transform(text.to_owned(), None))) {
        Ok(Ok(directives)) => Outcome::Parsed(directives),
        Ok(Err(error)) => Outcome::Error(error.to_string()),
        Err(payload) => Outcome::Panicked(panic_message(payload)),
    }
}

/// Export `directive` with `data_type`, with a panic as the error.
pub fn export<D: DataType<Carrier = String>>(data_type: &D, directive: Directive) -> Result<String, String> {
    catch_unwind(AssertUnwindSafe(|| data_type.export(Spanned::new(directive, SpanInfo::default())))).map_err(panic_message)
}

pub fn date() -> Date {
    Date::Date(NaiveDate::from_ymd_opt(2024, 1, 2).unwrap())
}

pub fn account(name: &str) -> Account {
    Account::from_str(name).unwrap()
}

pub fn posting(name: &str, units: Option<i64>) -> Posting {
    Posting {
        flag: None,
        account: account(name),
        units: units.map(|number| Amount::new(BigDecimal::from(number), "CNY")),
        cost: None,
        price: None,
        comment: None,
        meta: Default::default(),
        written: None,
    }
}

pub fn memo(value: &str) -> Meta {
    let mut meta = Meta::default();
    meta.insert("memo".to_owned(), ZhangString::quote(value));
    meta
}

/// A directive whose `field` holds `s` as a quoted string; every other field is plain ASCII.
pub fn directive_with(field: Field, s: &str) -> Directive {
    match field {
        Field::Payee | Field::Narration | Field::TransactionMeta => {
            let (payee, narration, meta) = match field {
                Field::Payee => (s, "narration", Meta::default()),
                Field::Narration => ("payee", s, Meta::default()),
                _ => ("payee", "narration", memo(s)),
            };
            Directive::Transaction(Transaction {
                date: date(),
                flag: Some(Flag::Okay),
                payee: Some(ZhangString::quote(payee)),
                narration: Some(ZhangString::quote(narration)),
                tags: Default::default(),
                links: Default::default(),
                postings: vec![posting("Assets:Cash", Some(-5)), posting("Expenses:Food", Some(5))],
                meta,
            })
        }
        Field::OpenMeta => Directive::Open(Open {
            date: date(),
            account: account("Assets:Cash"),
            commodities: vec![],
            meta: memo(s),
        }),
        Field::QueryName | Field::QueryText => {
            let (name, text) = match field {
                Field::QueryName => (s, "SELECT account"),
                _ => ("saved", s),
            };
            Directive::Query(Query {
                date: date(),
                name: ZhangString::quote(name),
                query_string: ZhangString::quote(text),
                meta: Meta::default(),
            })
        }
        Field::NoteComment => Directive::Note(Note {
            date: date(),
            account: account("Assets:Cash"),
            comment: ZhangString::quote(s),
            tags: None,
            links: None,
            meta: Meta::default(),
        }),
        Field::EventDescription => Directive::Event(Event {
            date: date(),
            event_type: ZhangString::quote("mood"),
            description: ZhangString::quote(s),
            meta: Meta::default(),
        }),
        Field::CustomValue => Directive::Custom(Custom {
            date: date(),
            custom_type: ZhangString::quote("kind"),
            values: vec![StringOrAccount::String(ZhangString::quote(s))],
            meta: Meta::default(),
        }),
    }
}

/// The value of `field` in a parsed directive, if the directive has that shape.
pub fn field_value(field: Field, directive: &Directive) -> Option<String> {
    let memo = |meta: &Meta| meta.get_one("memo").map(|value| value.as_str().to_owned());
    match (field, directive) {
        (Field::Payee, Directive::Transaction(txn)) => txn.payee.as_ref().map(|it| it.as_str().to_owned()),
        (Field::Narration, Directive::Transaction(txn)) => txn.narration.as_ref().map(|it| it.as_str().to_owned()),
        (Field::TransactionMeta, Directive::Transaction(txn)) => memo(&txn.meta),
        (Field::OpenMeta, Directive::Open(open)) => memo(&open.meta),
        (Field::QueryName, Directive::Query(query)) => Some(query.name.as_str().to_owned()),
        (Field::QueryText, Directive::Query(query)) => Some(query.query_string.as_str().to_owned()),
        (Field::NoteComment, Directive::Note(note)) => Some(note.comment.as_str().to_owned()),
        (Field::EventDescription, Directive::Event(event)) => Some(event.description.as_str().to_owned()),
        (Field::CustomValue, Directive::Custom(custom)) => match custom.values.as_slice() {
            [StringOrAccount::String(value)] => Some(value.as_str().to_owned()),
            _ => None,
        },
        _ => None,
    }
}

/// Export a directive holding `s` in `field`, parse the output back and compare.
pub fn round_trip<D: DataType<Carrier = String>>(data_type: &D, field: Field, s: &str) -> Result<(), String> {
    let exported = export(data_type, directive_with(field, s)).map_err(|panic| format!("{field:?} {s:?}: exporter panicked: {panic}"))?;
    let directives = match transform(data_type, &exported) {
        Outcome::Parsed(directives) => directives,
        Outcome::Error(error) => return Err(format!("{field:?} {s:?}: exported {exported:?} fails to parse: {error}")),
        Outcome::Panicked(panic) => return Err(format!("{field:?} {s:?}: parser panicked on exported {exported:?}: {panic}")),
    };
    match directives.as_slice() {
        [directive] => match field_value(field, &directive.data) {
            Some(value) if value == s => Ok(()),
            other => Err(format!("{field:?} {s:?}: exported {exported:?} parses back as {other:?}")),
        },
        _ => Err(format!("{field:?} {s:?}: exported {exported:?} parses into {} directives", directives.len())),
    }
}

/// Escape sequences in exporter output that the contract forbids (`\$`, `` \` ``, `\u{`).
///
/// The scan pairs each backslash with the character after it, the way the parser reads escapes,
/// so `\\$` (an escaped backslash followed by a dollar) is not a violation.
pub fn legacy_escapes_in(exported: &str) -> Vec<&'static str> {
    let mut found = vec![];
    let mut chars = exported.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            continue;
        }
        match chars.next() {
            Some('$') => found.push(r"\$"),
            Some('`') => found.push(r"\`"),
            Some('u') if chars.peek() == Some(&'{') => found.push(r"\u{"),
            _ => {}
        }
    }
    found
}

/// Small deterministic xorshift PRNG, so the property tests need no extra dependency and every
/// run checks the same inputs.
pub struct Rng(pub u64);

impl Rng {
    /// the next value; no iterator, which would never end
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    pub fn below(&mut self, bound: usize) -> usize {
        (self.next() % bound as u64) as usize
    }

    /// A string of up to `max_pieces` pieces drawn from `alphabet`.
    pub fn string(&mut self, alphabet: &[&str], max_pieces: usize) -> String {
        let pieces = self.below(max_pieces + 1);
        (0..pieces).map(|_| alphabet[self.below(alphabet.len())]).collect()
    }
}

/// Pieces for generated field values: quoting and escaping characters, control and separator
/// characters, non-BMP and CJK text, and literal text that looks like an escape sequence.
pub const ALPHABET: &[&str] = &[
    "\"",
    "\\",
    "$",
    "`",
    "\n",
    "\t",
    "\r",
    "\u{0}",
    "\u{1}",
    "\u{7}",
    "\u{8}",
    "\u{b}",
    "\u{c}",
    "\u{1b}",
    "\u{7f}",
    "\u{85}",
    "\u{a0}",
    "\u{2028}",
    "\u{2029}",
    "\u{200b}",
    "\u{feff}",
    "\u{1F600}",
    "\u{1F468}\u{200D}\u{1F469}",
    "中",
    "账本",
    "a",
    "Z",
    "0",
    "9",
    " ",
    "{",
    "}",
    "u",
    "/",
    "'",
    "#",
    ";",
    ":",
    "~",
    "SELECT",
    r"\u{a0}",
    r"\$",
    r"\`",
    r"\d",
    r"\u00e9",
    "u{2028}",
    r#"\""#,
];

/// Hand-picked strings that must round-trip in every field.
pub const EXAMPLES: &[&str] = &[
    "",
    "coffee $5",
    "a ` b",
    "$`$`",
    "x\u{a0}y",
    "SELECT\u{a0}account",
    "line\u{2028}separator",
    r"\d+",
    r"\\d+",
    r"C:\Users\me",
    "\\",
    "ends with a backslash \\",
    "\"",
    "say \"hi\"",
    "\\\"",
    "tab\there",
    "line\nbreak",
    "carriage\rreturn",
    "bell\u{7} vt\u{b} esc\u{1b} nul\u{0} del\u{7f}",
    "\u{1F600} 账本",
    r"literal \u{a0} and \u00e9 and \$ and \`",
    "{}",
    "u{2028}",
];

/// Ledger snippets that put `inner` (raw ledger text, between the quotes) into one quoted field.
pub fn snippets(inner: &str) -> [(Field, String); 3] {
    [
        (
            Field::Narration,
            format!("2024-01-02 * \"payee\" \"{inner}\"\n  Assets:Cash -5 CNY\n  Expenses:Food 5 CNY\n"),
        ),
        (Field::OpenMeta, format!("2024-01-02 open Assets:Cash\n  memo: \"{inner}\"\n")),
        (Field::QueryText, format!("2024-01-02 query \"saved\" \"{inner}\"\n")),
    ]
}

/// Check that the raw quoted text `inner` decodes to `expected` in every snippet position.
pub fn check_decodes<D: DataType<Carrier = String>>(data_type: &D, cases: &[(&str, &str)]) {
    let mut failures = vec![];
    for (inner, expected) in cases {
        for (field, text) in snippets(inner) {
            let actual = match transform(data_type, &text) {
                Outcome::Parsed(directives) => match directives.as_slice() {
                    [directive] => field_value(field, &directive.data).ok_or_else(|| format!("no {field:?} in {:?}", directive.data)),
                    _ => Err(format!("{} directives", directives.len())),
                },
                Outcome::Error(error) => Err(format!("parse error: {error}")),
                Outcome::Panicked(panic) => Err(format!("panic: {panic}")),
            };
            if actual.as_deref() != Ok(*expected) {
                failures.push(format!("{field:?} \"{inner}\": expected {expected:?}, got {actual:?}"));
            }
        }
    }
    assert!(failures.is_empty(), "{} decodings failed:\n{}", failures.len(), failures.join("\n"));
}
