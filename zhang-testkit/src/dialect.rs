//! Dialect scenarios, shared by the parser and exporter tests of zhang's text format (`zhang-core/tests`) and of
//! the beancount extension (`extensions/beancount/tests`): a comparable view of where a transaction's metadata
//! ended up ([`Shape`]), transactions with metadata on the transaction and on its postings, and random text
//! layouts together with the metadata each line belongs to under each dialect's rule ([`random_layout`]).
//!
//! Every access to a posting's metadata goes through [`posting_meta`] and [`posting`]: the contract names the
//! field `Posting.meta`, of the same `Meta` type as `Transaction.meta`. [`Scenario`] runs a text through one
//! dialect's parser and exporter; [`escaping`] holds the string-escaping scenarios.

use std::str::FromStr;

use beancount::Beancount;
use bigdecimal::BigDecimal;
use chrono::NaiveDate;
use zhang_ast::amount::Amount;
use zhang_ast::*;
use zhang_core::data_type::text::ZhangDataType;
use zhang_core::data_type::DataType;

pub mod escaping;

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

pub use crate::XorShift;

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
        ..PostingCost::default()
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

/// A dialect's parser and exporter under test, named for failure messages: [`Scenario::zhang`] or
/// [`Scenario::beancount`]. The scenario tests of both dialects share these through one-line wrappers, so that a
/// test body reads the same in `zhang-core/tests/core/posting_metadata.rs` and
/// `extensions/beancount/tests/beancount/posting_metadata.rs`.
pub struct Scenario<D> {
    /// the parser and exporter
    pub data_type: D,
    /// the dialect's name in failure messages: `zhang` or `beancount`
    pub name: &'static str,
}

impl Scenario<ZhangDataType> {
    /// zhang's own text format
    pub fn zhang() -> Scenario<ZhangDataType> {
        Scenario {
            data_type: ZhangDataType {},
            name: "zhang",
        }
    }
}

impl Scenario<Beancount> {
    /// the beancount format
    pub fn beancount() -> Scenario<Beancount> {
        Scenario {
            data_type: Beancount::default(),
            name: "beancount",
        }
    }
}

impl<D: DataType<Carrier = String>> Scenario<D> {
    /// The transactions of `text`, in order; panics, quoting the text, when it does not parse.
    pub fn transactions(&self, text: &str) -> Vec<Transaction> {
        self.data_type
            .transform(text.to_owned(), None)
            .unwrap_or_else(|err| panic!("cannot parse {text:?}: {err}"))
            .into_iter()
            .filter_map(|it| match it.data {
                Directive::Transaction(txn) => Some(txn),
                _ => None,
            })
            .collect()
    }

    /// The one transaction of `text`.
    pub fn parse_one(&self, text: &str) -> Transaction {
        let mut txns = self.transactions(text);
        assert_eq!(txns.len(), 1, "expected one transaction in {text:?}");
        txns.remove(0)
    }

    /// `text` parses into a transaction whose metadata lands as `expected` says.
    pub fn assert_shape(&self, text: &str, expected: Shape) {
        assert_eq!(Shape::of(&self.parse_one(text)), expected, "{} format, text:\n{text}", self.name);
    }

    /// The text the exporter writes for `txn`.
    pub fn export(&self, txn: Transaction) -> String {
        self.data_type.export(Spanned::new(Directive::Transaction(txn), SpanInfo::default()))
    }

    /// Export then parse gives `txn` back, and the export is laid out as the contract says.
    pub fn assert_round_trips(&self, txn: &Transaction) {
        let exported = self.export(txn.clone());
        assert_export_layout(&exported, txn);
        let reparsed = self.parse_one(&exported);
        assert_eq!(&reparsed, txn, "exported as:\n{exported}");
        assert_eq!(Shape::of(&reparsed), Shape::of(txn), "exported as:\n{exported}");
    }
}

/// The exported text is the header, the transaction metadata at 2 spaces, then each posting
/// at 2 spaces followed by exactly its own metadata lines at 4 spaces.
pub fn assert_export_layout(exported: &str, txn: &Transaction) {
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

/// `2024-01-02 * "Shop" "lunch"`: a receipt on the cash posting, a plain food posting, and a tips posting without an
/// amount whose memo needs escaping.
pub fn lunch() -> Transaction {
    transaction(
        vec![
            posting("Assets:Cash", Some(("-10", "CNY")), &[("receipt", quote("r-1"))]),
            posting("Expenses:Food", Some(("7", "CNY")), &[]),
            posting("Expenses:Tips", None, &[("memo", quote("say \"hi\" \\ bye"))]),
        ],
        &[("note", quote("t"))],
    )
}
