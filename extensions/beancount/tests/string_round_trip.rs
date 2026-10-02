//! Export → parse is an identity for every quoted string of the beancount data type
//! (issue #442), and the exported text uses only the escapes Python beancount
//! decodes. The beancount data type writes strings with zhang's exporter in
//! `QuoteStyle::Beancount` and reads them with zhang's string scanner, see
//! `zhang_core::utils::string_`.

use std::str::FromStr;

use beancount::Beancount;
use bigdecimal::BigDecimal;
use zhang_ast::amount::Amount;
use zhang_ast::*;
use zhang_core::data_type::DataType;

/// A small xorshift PRNG, so the property test needs no extra dependency and is
/// reproducible.
struct XorShift(u64);

impl XorShift {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn below(&mut self, bound: usize) -> usize {
        (self.next() % bound as u64) as usize
    }

    /// A random string drawn mostly from the characters that need care.
    fn string(&mut self) -> String {
        const INTERESTING: &[char] = &[
            '"', '\\', '$', '`', '\'', '/', 'u', '{', '}', 'd', 'n', ' ', '\n', '\r', '\t', '\u{0}', '\u{07}', '\u{08}', '\u{1b}', '\u{7f}', '\u{85}',
            '\u{a0}', '\u{200d}', '\u{2028}', '\u{2029}', '\u{3000}', '\u{feff}', '😀', '你', '好', 'é', 'a', ';', '#', ':',
        ];
        let len = self.below(24);
        (0..len)
            .map(|_| {
                if self.below(5) == 0 {
                    loop {
                        if let Some(c) = char::from_u32((self.next() % 0x11_0000) as u32) {
                            break c;
                        }
                    }
                } else {
                    INTERESTING[self.below(INTERESTING.len())]
                }
            })
            .collect()
    }
}

fn quote(s: String) -> ZhangString {
    ZhangString::QuoteString(s)
}

/// Every place a quoted string can appear in a beancount file, filled from `next`.
fn directives_with_strings(mut next: impl FnMut() -> String) -> Vec<Directive> {
    let date = Date::Date(chrono::NaiveDate::from_ymd_opt(2024, 1, 2).unwrap());
    let account = |name: &str| Account::from_str(name).unwrap();
    let mut meta = |key: &str| {
        let mut meta = Meta::default();
        meta.insert(key.to_owned(), quote(next()));
        meta
    };
    let txn_meta = meta("note");
    let query_meta = meta("owner");
    vec![
        Directive::Transaction(Transaction {
            date: date.clone(),
            flag: Some(Flag::Okay),
            payee: Some(quote(next())),
            narration: Some(quote(next())),
            tags: Default::default(),
            links: Default::default(),
            postings: vec![
                Posting {
                    flag: None,
                    account: account("Assets:Cash"),
                    units: Some(Amount::new(BigDecimal::from(-1), "HOOL")),
                    cost: Some(PostingCost {
                        base: Some(Amount::new(BigDecimal::from(1), "USD")),
                        date: None,
                        label: Some(next()),
                        total: false,
                    }),
                    price: None,
                    comment: None,
                },
                Posting {
                    flag: None,
                    account: account("Expenses:Food"),
                    units: None,
                    cost: None,
                    price: None,
                    comment: None,
                },
            ],
            meta: txn_meta,
        }),
        Directive::Note(Note {
            date: date.clone(),
            account: account("Assets:Cash"),
            comment: quote(next()),
            tags: None,
            links: None,
            meta: Meta::default(),
        }),
        Directive::Document(Document {
            date: date.clone(),
            account: account("Assets:Cash"),
            filename: quote(next()),
            tags: None,
            links: None,
            meta: Meta::default(),
        }),
        Directive::Event(Event {
            date: date.clone(),
            event_type: quote(next()),
            description: quote(next()),
            meta: Meta::default(),
        }),
        Directive::Query(Query {
            date: date.clone(),
            name: quote(next()),
            query_string: quote(next()),
            meta: query_meta,
        }),
        Directive::Custom(Custom {
            date,
            custom_type: quote(next()),
            values: vec![StringOrAccount::String(quote(next()))],
            meta: Meta::default(),
        }),
        Directive::Option(Options {
            key: quote(next()),
            value: quote(next()),
        }),
        Directive::Plugin(Plugin {
            module: quote(next()),
            value: vec![quote(next())],
            meta: Meta::default(),
        }),
        Directive::Include(Include { file: quote(next()) }),
    ]
}

/// Every escape in `exported` is one Python beancount decodes; it has no unicode escape.
fn assert_beancount_escapes_only(exported: &str) {
    let mut chars = exported.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            let escaped = chars.next();
            assert!(
                matches!(escaped, Some('"' | '\\' | 'n' | 't' | 'r' | 'b' | 'f')),
                "{exported:?} has the escape \\{escaped:?}"
            );
        }
    }
}

fn assert_round_trips(directive: Directive) {
    let beancount = Beancount::default();
    let exported = beancount.export(Spanned::new(directive.clone(), SpanInfo::default()));
    assert_beancount_escapes_only(&exported);
    let reparsed = beancount
        .transform(exported.clone(), None)
        .unwrap_or_else(|err| panic!("cannot parse the exported text {exported:?}: {err}"));
    assert_eq!(reparsed.len(), 1, "{exported:?}");
    assert_eq!(reparsed[0].data, directive, "exported as {exported:?}");
}

#[test]
fn issue_442_strings_round_trip() {
    for s in [
        "bell\u{7} esc\u{1b} nul\u{0} backspace\u{8} form feed\u{c}",
        "coffee $5",
        "`cmd`",
        "SELECT\u{a0}account",
        "a\u{2028}b",
        "narration ~ '\\d+'",
        "ends with \\",
        "say \"hi\"",
        "two\nlines",
    ] {
        for directive in directives_with_strings(|| s.to_owned()) {
            assert_round_trips(directive);
        }
    }
}

#[test]
fn exporting_then_parsing_random_strings_is_an_identity() {
    let mut rng = XorShift(0x2a2a_0442_beef);
    for _ in 0..300 {
        let mut strings = Vec::new();
        for _ in 0..20 {
            strings.push(rng.string());
        }
        let mut strings = strings.into_iter();
        for directive in directives_with_strings(|| strings.next().unwrap()) {
            assert_round_trips(directive);
        }
    }
}

#[test]
fn a_saved_query_with_an_unknown_escape_loads() {
    let directives = Beancount::default()
        .transform("2014-01-01 query \"x\" \"SELECT narration WHERE narration ~ '\\d+'\"\n".to_owned(), None)
        .unwrap();
    let Directive::Query(query) = &directives[0].data else {
        panic!("expected a query directive, got {:?}", directives[0].data);
    };
    assert_eq!(query.query_string.as_str(), r"SELECT narration WHERE narration ~ '\d+'");
}
