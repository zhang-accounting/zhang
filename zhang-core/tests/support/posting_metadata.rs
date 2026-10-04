//! Shared by the posting metadata acceptance tests of zhang-core and of the beancount
//! extension: a comparable view of where a transaction's metadata ended up, a small
//! deterministic PRNG, random transactions with metadata on the transaction and on its
//! postings, and random text layouts together with the metadata each line belongs to.
//!
//! Every access to a posting's metadata goes through [`posting_meta`] and [`posting`]: the
//! contract names the field `Posting.meta`, of the same `Meta` type as `Transaction.meta`.
#![allow(dead_code)]

use std::str::FromStr;

use bigdecimal::BigDecimal;
use chrono::NaiveDate;
use zhang_ast::amount::Amount;
use zhang_ast::*;

/// The metadata of `posting`: the contract's `Posting.meta`.
pub fn posting_meta(posting: &Posting) -> &Meta {
    &posting.meta
}

/// A posting on `account` with `units` (number, commodity) and the metadata `meta`.
pub fn posting(account: &str, units: Option<(&str, &str)>, meta: &[(&str, ZhangString)]) -> Posting {
    Posting {
        flag: None,
        account: Account::from_str(account).unwrap(),
        units: units.map(|(number, commodity)| Amount::new(BigDecimal::from_str(number).unwrap(), commodity)),
        cost: None,
        price: None,
        comment: None,
        meta: meta.iter().map(|(key, value)| (key.to_string(), value.clone())).collect(),
        written: None,
    }
}

pub fn quote(s: &str) -> ZhangString {
    ZhangString::QuoteString(s.to_owned())
}

pub fn unquote(s: &str) -> ZhangString {
    ZhangString::UnquoteString(s.to_owned())
}

pub fn date(year: i32, month: u32, day: u32) -> Date {
    Date::Date(NaiveDate::from_ymd_opt(year, month, day).unwrap())
}

/// `2024-01-02 * "Shop" "lunch"` with the given postings and transaction metadata.
pub fn transaction(postings: Vec<Posting>, meta: &[(&str, ZhangString)]) -> Transaction {
    Transaction {
        date: date(2024, 1, 2),
        flag: Some(Flag::Okay),
        payee: Some(quote("Shop")),
        narration: Some(quote("lunch")),
        tags: Default::default(),
        links: Default::default(),
        postings,
        meta: meta.iter().map(|(key, value)| (key.to_string(), value.clone())).collect(),
    }
}

/// Metadata as sorted `(key, value)` pairs, one per value.
pub type Pairs = Vec<(String, String)>;

pub fn pairs(meta: &Meta) -> Pairs {
    let mut pairs: Pairs = meta
        .clone()
        .get_flatten()
        .into_iter()
        .map(|(key, value)| (key, value.as_str().to_owned()))
        .collect();
    pairs.sort();
    pairs
}

fn owned(pairs: &[(&str, &str)]) -> Pairs {
    let mut pairs: Pairs = pairs.iter().map(|(key, value)| (key.to_string(), value.to_string())).collect();
    pairs.sort();
    pairs
}

/// Where the metadata of a transaction ended up: on the transaction, or on which posting.
#[derive(Debug, PartialEq, Eq)]
pub struct Shape {
    pub transaction: Pairs,
    /// per posting, in order: its account and its metadata
    pub postings: Vec<(String, Pairs)>,
}

impl Shape {
    pub fn of(txn: &Transaction) -> Shape {
        Shape {
            transaction: pairs(&txn.meta),
            postings: txn
                .postings
                .iter()
                .map(|posting| (posting.account.content.clone(), pairs(posting_meta(posting))))
                .collect(),
        }
    }

    pub fn new(transaction: &[(&str, &str)], postings: &[(&str, &[(&str, &str)])]) -> Shape {
        Shape {
            transaction: owned(transaction),
            postings: postings.iter().map(|(account, meta)| (account.to_string(), owned(meta))).collect(),
        }
    }
}

/// A small xorshift PRNG, so the property tests need no extra dependency and are
/// reproducible.
pub struct XorShift(pub u64);

impl XorShift {
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

    /// true `percent` times in a hundred
    pub fn chance(&mut self, percent: usize) -> bool {
        self.below(100) < percent
    }

    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }

    /// A random string drawn mostly from the characters that need care in a quoted string.
    pub fn string(&mut self) -> String {
        const INTERESTING: &[char] = &[
            '"', '\\', '$', '`', '\'', '/', 'u', '{', '}', 'd', 'n', ' ', '\n', '\r', '\t', '\u{0}', '\u{07}', '\u{08}', '\u{1b}', '\u{7f}', '\u{85}',
            '\u{a0}', '\u{200d}', '\u{2028}', '\u{2029}', '\u{3000}', '\u{feff}', '😀', '你', '好', 'é', 'a', ';', '#', ':', '*', '^', '@', '!',
        ];
        let len = self.below(16);
        (0..len)
            .map(|_| {
                if self.below(6) == 0 {
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

/// Keys written bare. `Receipt` and `x` are no beancount keys, but zhang's parsers read them.
pub const BARE_KEYS: &[&str] = &["receipt", "category", "note", "document", "trip", "x-y_z", "memo2", "Receipt", "x", "Assets"];

/// Keys that are no bare words: the exporter quotes them, and both of zhang's parsers read
/// them back exactly. Some look like a posting, a comment, a tag or a link.
pub const AWKWARD_KEYS: &[&str] = &[
    "my key",
    ";path",
    "",
    "a:b",
    "#tag",
    "^link",
    "*star",
    "//slash",
    "say \"hi\"",
    "tab\tkey",
    "two\nlines",
    "  padded",
    "Assets:Cash",
    "Expenses:Food 10 CNY",
    "back\\slash",
];

/// Values written bare; each is a value beancount accepts too.
pub const BARE_VALUES: &[&str] = &["12", "-3.5", "USD", "TRUE", "2024-01-02", "HOOL"];

/// Quoted values that need escaping or that look like something else.
pub const AWKWARD_VALUES: &[&str] = &[
    "",
    "say \"hi\"",
    "back\\slash",
    "ends with \\",
    "two\nlines",
    "tab\there",
    "coffee $5",
    "`cmd`",
    "a\u{a0}b\u{2028}c",
    "😀 你好",
    "; not a comment",
    "  padded  ",
    "Assets:Cash 10 CNY",
    "key: value",
];

pub const ACCOUNTS: &[&str] = &[
    "Assets:Cash",
    "Expenses:Food",
    "Liabilities:Card",
    "Income:Salary",
    "Equity:Opening",
    "Assets:Bank:Checking",
    "Assets:银行",
];

const COMMODITIES: &[&str] = &["CNY", "USD", "HOOL", "VBMPX"];

pub fn random_key(rng: &mut XorShift) -> String {
    match rng.below(4) {
        0 | 1 => rng.pick(BARE_KEYS).to_string(),
        2 => rng.pick(AWKWARD_KEYS).to_string(),
        _ => rng.string(),
    }
}

pub fn random_value(rng: &mut XorShift) -> ZhangString {
    match rng.below(10) {
        0..=2 => unquote(rng.pick(BARE_VALUES)),
        3..=4 => quote(rng.pick(AWKWARD_VALUES)),
        _ => ZhangString::QuoteString(rng.string()),
    }
}

/// Up to `max` metadata entries; keys repeat now and then, so some keys hold several values.
pub fn random_meta(rng: &mut XorShift, max: usize) -> Meta {
    let count = rng.below(max + 1);
    let mut meta = Meta::default();
    let mut keys: Vec<String> = vec![];
    for _ in 0..count {
        let key = if !keys.is_empty() && rng.chance(15) {
            rng.pick(&keys).clone()
        } else {
            random_key(rng)
        };
        keys.push(key.clone());
        meta.insert(key, random_value(rng));
    }
    meta
}

fn random_amount(rng: &mut XorShift) -> Amount {
    let sign = if rng.chance(50) { "-" } else { "" };
    let number = match rng.below(3) {
        0 => format!("{sign}{}", rng.below(1000)),
        1 => format!("{sign}{}.{}", rng.below(1000), rng.below(10)),
        _ => format!("{sign}{}.{:02}", rng.below(100), rng.below(100)),
    };
    Amount::new(BigDecimal::from_str(&number).unwrap(), *rng.pick(COMMODITIES))
}

fn random_posting(rng: &mut XorShift) -> Posting {
    let units = rng.chance(85).then(|| random_amount(rng));
    let cost = (units.is_some() && rng.chance(25)).then(|| PostingCost {
        base: Some(random_amount(rng)),
        date: rng.chance(30).then(|| date(2023, 1 + rng.below(12) as u32, 1 + rng.below(28) as u32)),
        label: rng.chance(30).then(|| rng.pick(&["lot-1", "a b", "say \"hi\""]).to_string()),
        total: rng.chance(20),
    });
    let price = (units.is_some() && rng.chance(20)).then(|| {
        if rng.chance(50) {
            SingleTotalPrice::Single(random_amount(rng))
        } else {
            SingleTotalPrice::Total(random_amount(rng))
        }
    });
    Posting {
        flag: None,
        account: Account::from_str(rng.pick(ACCOUNTS)).unwrap(),
        units,
        cost,
        price,
        comment: None,
        meta: random_meta(rng, 3),
        written: None,
    }
}

/// A random transaction with metadata on itself and on its postings.
pub fn random_transaction(rng: &mut XorShift) -> Transaction {
    let postings = (0..1 + rng.below(4)).map(|_| random_posting(rng)).collect();
    let mut txn = transaction(postings, &[]);
    txn.date = date(2024, 1 + rng.below(12) as u32, 1 + rng.below(28) as u32);
    txn.narration = Some(ZhangString::QuoteString(rng.string()));
    txn.meta = random_meta(rng, 3);
    txn
}

/// Which format's attachment rule a layout follows.
#[derive(Clone, Copy, Debug)]
pub enum Rule {
    /// a metadata line belongs to the posting before it only when it is indented deeper
    /// than that posting's line; otherwise it is transaction metadata
    Zhang,
    /// beancount 3.2.3: a metadata line before the first posting belongs to the
    /// transaction, one after a posting to the latest posting, whatever the indentation
    Beancount,
}

const LAYOUT_ACCOUNTS: &[&str] = &["Assets:Cash", "Expenses:Food", "Expenses:Drink", "Income:Other"];

/// A random transaction text: metadata before, between and after postings at assorted
/// indentations, indented comment lines and trailing comments in between, and the last
/// posting sometimes without an amount. Returns the text and where `rule` puts each line.
///
/// For [`Rule::Zhang`] the metadata after a posting first has the lines deeper than the
/// posting, then the lines at most as deep as the posting: no metadata deeper than a
/// posting follows a transaction metadata line under that posting.
pub fn random_layout(rng: &mut XorShift, rule: Rule) -> (String, Shape) {
    let spaces = |n: usize| " ".repeat(n);
    let mut lines = vec!["2024-01-02 * \"Shop\" \"layout\"".to_owned()];
    let mut transaction: Vec<(String, String)> = vec![];
    let mut postings: Vec<(String, Pairs)> = vec![];
    let mut counter = 0;

    // a metadata line at `indent`, maybe after an indented comment line and maybe with a
    // trailing comment; returns its (key, value)
    let mut meta_line = |rng: &mut XorShift, lines: &mut Vec<String>, indent: String, prefix: &str| {
        if rng.chance(20) {
            let comment_indent = match rule {
                Rule::Zhang => spaces(1 + rng.below(8)),
                Rule::Beancount => beancount_indent(rng),
            };
            lines.push(format!("{comment_indent}; comment {counter}"));
        }
        counter += 1;
        let (key, value) = (format!("{prefix}{counter}"), format!("v{counter}"));
        let trailer = if rng.chance(20) { " ; trailing" } else { "" };
        lines.push(format!("{indent}{key}: \"{value}\"{trailer}"));
        (key, value)
    };

    for _ in 0..rng.below(3) {
        let indent = match rule {
            Rule::Zhang => spaces(1 + rng.below(6)),
            Rule::Beancount => beancount_indent(rng),
        };
        transaction.push(meta_line(rng, &mut lines, indent, "tk"));
    }
    let count = 1 + rng.below(LAYOUT_ACCOUNTS.len());
    for (index, account) in LAYOUT_ACCOUNTS.iter().take(count).enumerate() {
        let posting_indent = *rng.pick(&[2, 2, 2, 4, 4, 1, 3]);
        let indent = match rule {
            Rule::Zhang => spaces(posting_indent),
            Rule::Beancount => beancount_indent(rng),
        };
        let amount = if index + 1 == count && count > 1 && rng.chance(40) {
            String::new()
        } else {
            format!(" {}{} USD", if index == 0 { "-" } else { "" }, 1 + rng.below(9))
        };
        let trailer = if rng.chance(15) { " ; posting comment" } else { "" };
        lines.push(format!("{indent}{account}{amount}{trailer}"));
        let mut own = vec![];
        for _ in 0..rng.below(4) {
            let indent = match rule {
                Rule::Zhang => spaces(posting_indent + 1 + rng.below(4)),
                Rule::Beancount => beancount_indent(rng),
            };
            own.push(meta_line(rng, &mut lines, indent, "pk"));
        }
        for _ in 0..rng.below(3) {
            let indent = match rule {
                Rule::Zhang => spaces(1 + rng.below(posting_indent)),
                Rule::Beancount => beancount_indent(rng),
            };
            let pair = meta_line(rng, &mut lines, indent, "tk");
            match rule {
                Rule::Zhang => transaction.push(pair),
                Rule::Beancount => own.push(pair),
            }
        }
        own.sort();
        postings.push((account.to_string(), own));
    }
    transaction.sort();
    let mut text = lines.join("\n");
    text.push('\n');
    (text, Shape { transaction, postings })
}

/// Indentation beancount 3.2.3 reads the same: one or more spaces, a tab, or both.
fn beancount_indent(rng: &mut XorShift) -> String {
    rng.pick(&[" ", "  ", "  ", "   ", "    ", "    ", "      ", "\t", "\t  ", "  \t"]).to_string()
}
