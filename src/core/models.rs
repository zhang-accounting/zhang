use serde::{Deserialize, Serialize};
use strum_macros::EnumString;

use crate::core::account::Account;
use crate::core::amount::Amount;
use crate::core::data::{
    Balance, Close, Commodity, Custom, Document, Event, Note, Open, Pad, Price, Transaction,
};

#[derive(Debug, PartialEq)]
pub enum Directive {
    Open(Open),
    Close(Close),
    Commodity(Commodity),
    Transaction(Transaction),
    Balance(Balance),
    Pad(Pad),
    Note(Note),
    Document(Document),
    Price(Price),
    Event(Event),
    Custom(Custom),
    Option {
        key: ZhangString,
        value: ZhangString,
    },
    Plugin {
        module: ZhangString,
        value: Vec<ZhangString>,
    },
    Include {
        file: ZhangString,
    },
    Comment {
        content: String,
    },
}

#[derive(Debug, PartialEq)]
pub enum StringOrAccount {
    String(ZhangString),
    Account(Account),
}

#[derive(Debug, PartialEq)]
pub enum ZhangString {
    UnquoteString(String),
    QuoteString(String),
}
impl ZhangString {
    pub fn to_plain_string(self) -> String {
        match self {
            ZhangString::UnquoteString(unquote) => unquote,
            ZhangString::QuoteString(quote) => quote,
        }
    }
}

#[derive(Debug, PartialEq, Clone)]
pub enum SingleTotalPrice {
    Single(Amount),
    Total(Amount),
}

#[derive(EnumString, Debug, PartialEq, strum_macros::ToString, Deserialize, Serialize)]
pub enum Flag {
    #[strum(serialize = "*", to_string = "*")]
    Okay,
    #[strum(serialize = "!", to_string = "!")]
    Warning,
}
//
// #[cfg(test)]
// mod test {
//     use crate::models::Directive;
//     use crate::p::parse_zhang;
//
//     fn single_directive_parser(content: &str) -> Directive {
//         let result = parse_zhang(content);
//         let vec = result.unwrap();
//         vec.into_iter().next().unwrap()
//     }
//
//     mod open {
//         use chrono::NaiveDate;
//
//         use crate::account::AccountType;
//         use crate::models::test::single_directive_parser;
//         use crate::models::{Account, Directive};
//
//         #[test]
//         fn test_open_directive() {
//             let directive = Directive::Open {
//                 date: NaiveDate::from_ymd(1970, 1, 1),
//                 account: Account::new(
//                     AccountType::Assets,
//                     vec![
//                         "123".to_owned(),
//                         "234".to_owned(),
//                         "English".to_owned(),
//                         "中文".to_owned(),
//                         "日本語".to_owned(),
//                         "한국어".to_owned(),
//                     ],
//                 ),
//                 commodities: vec![],
//             };
//             let x = single_directive_parser(
//                 "1970-01-01 open Assets:123:234:English:中文:日本語:한국어",
//             );
//             assert_eq!(directive, x);
//         }
//
//         #[test]
//         fn test_open_with_commodity() {
//             let directive = Directive::Open {
//                 date: NaiveDate::from_ymd(1970, 1, 1),
//                 account: Account::new(
//                     AccountType::Assets,
//                     vec![
//                         "123".to_owned(),
//                         "234".to_owned(),
//                         "English".to_owned(),
//                         "中文".to_owned(),
//                         "日本語".to_owned(),
//                         "한국어".to_owned(),
//                     ],
//                 ),
//                 commodities: vec!["CNY".to_owned()],
//             };
//             let x = single_directive_parser(
//                 "1970-01-01 open Assets:123:234:English:中文:日本語:한국어 CNY",
//             );
//             assert_eq!(directive, x);
//         }
//
//         #[test]
//         fn test_open_with_commodities() {
//             let directive = Directive::Open {
//                 date: NaiveDate::from_ymd(1970, 1, 1),
//                 account: Account::new(
//                     AccountType::Assets,
//                     vec![
//                         "123".to_owned(),
//                         "234".to_owned(),
//                         "English".to_owned(),
//                         "中文".to_owned(),
//                         "日本語".to_owned(),
//                         "한국어".to_owned(),
//                     ],
//                 ),
//                 commodities: vec!["CNY".to_owned(), "USD".to_owned(), "CAD".to_owned()],
//             };
//             let x = single_directive_parser(
//                 "1970-01-01 open Assets:123:234:English:中文:日本語:한국어 CNY, USD,CAD",
//             );
//             assert_eq!(directive, x);
//         }
//     }
//
//     mod close {
//         use chrono::NaiveDate;
//
//         use crate::account::AccountType;
//         use crate::models::test::single_directive_parser;
//         use crate::models::{Account, Directive};
//
//         #[test]
//         fn test_close() {
//             let directive = Directive::Close {
//                 date: NaiveDate::from_ymd(1970, 1, 1),
//                 account: Account::new(
//                     AccountType::Assets,
//                     vec!["123".to_owned(), "456".to_owned()],
//                 ),
//             };
//             let x = single_directive_parser(r#"1970-01-01 close Assets:123:456"#);
//             assert_eq!(directive, x);
//         }
//     }
//
//     mod note {
//         use chrono::NaiveDate;
//
//         use crate::account::AccountType;
//         use crate::models::test::single_directive_parser;
//         use crate::models::{Account, Directive};
//
//         #[test]
//         fn test_note_directive() {
//             let directive = Directive::Note {
//                 date: NaiveDate::from_ymd(1970, 1, 1),
//                 account: Account::new(AccountType::Assets, vec!["123".to_owned()]),
//                 description: "你 好 啊\\".to_owned(),
//             };
//             let x = single_directive_parser(r#"1970-01-01 note Assets:123 "你 好 啊\\""#);
//             assert_eq!(directive, x);
//         }
//     }
//
//     mod commodity {
//         use chrono::NaiveDate;
//
//         use crate::models::test::single_directive_parser;
//         use crate::models::ZhangString;
//         use crate::models::Directive;
//
//         #[test]
//         fn test_commodity_without_attribute() {
//             let directive = Directive::Commodity {
//                 date: NaiveDate::from_ymd(1970, 1, 1),
//                 name: "CNY".to_owned(),
//                 metas: vec![],
//             };
//             let x = single_directive_parser(r#"1970-01-01 commodity CNY"#);
//             assert_eq!(directive, x);
//         }
//
//         #[test]
//         fn test_commodity_with_single_attribute() {
//             let metas = vec![(
//                 ZhangString::UnquoteString("a".to_owned()),
//                 ZhangString::QuoteString("b".to_owned()),
//             )];
//             let directive = Directive::Commodity {
//                 date: NaiveDate::from_ymd(1970, 1, 1),
//                 name: "CNY".to_owned(),
//                 metas,
//             };
//
//             let x = single_directive_parser(
//                 r#"1970-01-01 commodity CNY
//                   a: "b""#,
//             );
//             assert_eq!(directive, x);
//         }
//
//         #[test]
//         fn test_commodity_with_attributes() {
//             let x = single_directive_parser(
//                 r#"1970-01-01 commodity CNY
//                   a: "b"
//                   中文-test  :  "한국어 我也不知道我在说啥""#,
//             );
//
//             let metas = vec![
//                 (
//                     ZhangString::UnquoteString("a".to_owned()),
//                     ZhangString::QuoteString("b".to_owned()),
//                 ),
//                 (
//                     ZhangString::UnquoteString("中文-test".to_owned()),
//                     ZhangString::QuoteString("한국어 我也不知道我在说啥".to_owned()),
//                 ),
//             ];
//
//             let directive = Directive::Commodity {
//                 date: NaiveDate::from_ymd(1970, 1, 1),
//                 name: "CNY".to_owned(),
//                 metas,
//             };
//             assert_eq!(directive, x);
//         }
//     }
//
//     mod transaction {
//         use bigdecimal::{BigDecimal, FromPrimitive};
//         use chrono::NaiveDate;
//
//         use crate::account::AccountType;
//         use crate::models::test::single_directive_parser;
//         use crate::models::{Account, Directive, Flag, Transaction, TransactionLine};
//         use crate::models::{ZhangString, Price};
//
//         #[test]
//         fn simple_test() {
//             let x = single_directive_parser(
//                 r#"1970-01-01 * "Payee" "Narration"
//                   Assets:123  -1 CNY
//                   Expenses:TestCategory:One 1 CNY"#,
//             );
//
//             let a = TransactionLine {
//                 flag: None,
//                 account: Account::new(AccountType::Assets, vec!["123".to_owned()]),
//                 amount: Some((BigDecimal::from(-1i16), "CNY".to_string())),
//                 cost: None,
//                 cost_date: None,
//                 price: None,
//             };
//             let b = TransactionLine {
//                 flag: None,
//                 account: Account::new(
//                     AccountType::Expenses,
//                     vec!["TestCategory".to_owned(), "One".to_owned()],
//                 ),
//                 amount: Some((BigDecimal::from(1i16), "CNY".to_string())),
//                 cost: None,
//                 cost_date: None,
//                 price: None,
//             };
//
//             let transaction = Transaction {
//                 date: NaiveDate::from_ymd(1970, 1, 1),
//                 flag: Some(Flag::Okay),
//                 payee: Some(ZhangString::QuoteString("Payee".to_owned())),
//                 narration: Some(ZhangString::QuoteString("Narration".to_owned())),
//                 tags: vec![],
//                 links: vec![],
//                 lines: vec![a, b],
//                 metas: vec![],
//             };
//             let x1 = Directive::Transaction(transaction);
//
//             assert_eq!(x1, x);
//         }
//
//         #[test]
//         fn without_payee_with_narration() {
//             let x = single_directive_parser(
//                 r#"1970-01-01 * "Narration"
//                   Assets:123  -1 CNY
//                   Expenses:TestCategory:One 1 CNY"#,
//             );
//
//             let a = TransactionLine {
//                 flag: None,
//                 account: Account::new(AccountType::Assets, vec!["123".to_owned()]),
//                 amount: Some((BigDecimal::from(-1i16), "CNY".to_string())),
//                 cost: None,
//                 cost_date: None,
//                 price: None,
//             };
//             let b = TransactionLine {
//                 flag: None,
//                 account: Account::new(
//                     AccountType::Expenses,
//                     vec!["TestCategory".to_owned(), "One".to_owned()],
//                 ),
//                 amount: Some((BigDecimal::from(1i16), "CNY".to_string())),
//                 cost: None,
//                 cost_date: None,
//                 price: None,
//             };
//
//             let transaction = Transaction {
//                 date: NaiveDate::from_ymd(1970, 1, 1),
//                 flag: Some(Flag::Okay),
//                 payee: None,
//                 narration: Some(ZhangString::QuoteString("Narration".to_owned())),
//                 tags: vec![],
//                 links: vec![],
//                 lines: vec![a, b],
//                 metas: vec![],
//             };
//             let x1 = Directive::Transaction(transaction);
//
//             assert_eq!(x1, x);
//         }
//
//         #[test]
//         fn cost_and_cost_comment() {
//             let x = single_directive_parser(
//                 r#"1970-01-01 * "Narration"
//                   Assets:123  -1 CNY {0.1 USD , 2111-11-11}
//                   Expenses:TestCategory:One 1 CNY {0.1 USD}"#,
//             );
//
//             let a = TransactionLine {
//                 flag: None,
//                 account: Account::new(AccountType::Assets, vec!["123".to_owned()]),
//                 amount: Some((BigDecimal::from(-1i16), "CNY".to_string())),
//                 cost: Some((BigDecimal::from_f32(0.1f32).unwrap(), "USD".to_owned())),
//                 cost_date: Some(NaiveDate::from_ymd(2111, 11, 11)),
//                 price: None,
//             };
//             let b = TransactionLine {
//                 flag: None,
//                 account: Account::new(
//                     AccountType::Expenses,
//                     vec!["TestCategory".to_owned(), "One".to_owned()],
//                 ),
//                 amount: Some((BigDecimal::from(1i16), "CNY".to_string())),
//                 cost: Some((BigDecimal::from_f32(0.1f32).unwrap(), "USD".to_owned())),
//                 cost_date: None,
//                 price: None,
//             };
//
//             let transaction = Transaction {
//                 date: NaiveDate::from_ymd(1970, 1, 1),
//                 flag: Some(Flag::Okay),
//                 payee: None,
//                 narration: Some(ZhangString::QuoteString("Narration".to_owned())),
//                 tags: vec![],
//                 links: vec![],
//                 lines: vec![a, b],
//                 metas: vec![],
//             };
//             let x1 = Directive::Transaction(transaction);
//
//             assert_eq!(x1, x);
//         }
//
//         #[test]
//         fn multiple_transaction_lines() {
//             let x = single_directive_parser(
//                 r#"1970-01-01 * "Payee" "Narration"
//                   Assets:123  -1 CNY
//                   Expenses:TestCategory:One 0.5 CNY
//                   Expenses:TestCategory:Two 0.5 CNY"#,
//             );
//
//             let a = TransactionLine {
//                 flag: None,
//                 account: Account::new(AccountType::Assets, vec!["123".to_owned()]),
//                 amount: Some((BigDecimal::from(-1i16), "CNY".to_string())),
//                 cost: None,
//                 cost_date: None,
//                 price: None,
//             };
//             let b = TransactionLine {
//                 flag: None,
//                 account: Account::new(
//                     AccountType::Expenses,
//                     vec!["TestCategory".to_owned(), "One".to_owned()],
//                 ),
//                 amount: Some((BigDecimal::from_f32(0.5f32).unwrap(), "CNY".to_string())),
//                 cost: None,
//                 cost_date: None,
//                 price: None,
//             };
//             let c = TransactionLine {
//                 flag: None,
//                 account: Account::new(
//                     AccountType::Expenses,
//                     vec!["TestCategory".to_owned(), "Two".to_owned()],
//                 ),
//                 amount: Some((BigDecimal::from_f32(0.5f32).unwrap(), "CNY".to_string())),
//                 cost: None,
//                 cost_date: None,
//                 price: None,
//             };
//
//             let transaction = Transaction {
//                 date: NaiveDate::from_ymd(1970, 1, 1),
//                 flag: Some(Flag::Okay),
//                 payee: Some(ZhangString::QuoteString("Payee".to_owned())),
//                 narration: Some(ZhangString::QuoteString("Narration".to_owned())),
//                 tags: vec![],
//                 links: vec![],
//                 lines: vec![a, b, c],
//                 metas: vec![],
//             };
//             let x1 = Directive::Transaction(transaction);
//
//             assert_eq!(x1, x);
//         }
//
//         #[test]
//         fn optional_amount_in_line() {
//             let x = single_directive_parser(
//                 r#"1970-01-01 * "Payee" "Narration"
//                   Assets:123  -1 CNY
//                   Expenses:TestCategory:One"#,
//             );
//
//             let a = TransactionLine {
//                 flag: None,
//                 account: Account::new(AccountType::Assets, vec!["123".to_owned()]),
//                 amount: Some((BigDecimal::from(-1i16), "CNY".to_string())),
//                 cost: None,
//                 cost_date: None,
//                 price: None,
//             };
//             let b = TransactionLine {
//                 flag: None,
//                 account: Account::new(
//                     AccountType::Expenses,
//                     vec!["TestCategory".to_owned(), "One".to_owned()],
//                 ),
//                 amount: None,
//                 cost: None,
//                 cost_date: None,
//                 price: None,
//             };
//
//             let transaction = Transaction {
//                 date: NaiveDate::from_ymd(1970, 1, 1),
//                 flag: Some(Flag::Okay),
//                 payee: Some(ZhangString::QuoteString("Payee".to_owned())),
//                 narration: Some(ZhangString::QuoteString("Narration".to_owned())),
//                 tags: vec![],
//                 links: vec![],
//                 lines: vec![a, b],
//                 metas: vec![],
//             };
//             let x1 = Directive::Transaction(transaction);
//
//             assert_eq!(x1, x);
//         }
//
//         #[test]
//         fn optional_single_price() {
//             let x = single_directive_parser(
//                 r#"1970-01-01 * "Payee" "Narration"
//                   Assets:123  -1 CNY
//                   Expenses:TestCategory:One 1 CCC @ 1 CNY"#,
//             );
//
//             let a = TransactionLine {
//                 flag: None,
//                 account: Account::new(AccountType::Assets, vec!["123".to_owned()]),
//                 amount: Some((BigDecimal::from(-1i16), "CNY".to_string())),
//                 cost: None,
//                 cost_date: None,
//                 price: None,
//             };
//             let b = TransactionLine {
//                 flag: None,
//                 account: Account::new(
//                     AccountType::Expenses,
//                     vec!["TestCategory".to_owned(), "One".to_owned()],
//                 ),
//                 amount: Some((BigDecimal::from(1i16), "CCC".to_string())),
//                 cost: None,
//                 cost_date: None,
//                 price: Some(Price::Single((BigDecimal::from(1i16), "CNY".to_string()))),
//             };
//
//             let transaction = Transaction {
//                 date: NaiveDate::from_ymd(1970, 1, 1),
//                 flag: Some(Flag::Okay),
//                 payee: Some(ZhangString::QuoteString("Payee".to_owned())),
//                 narration: Some(ZhangString::QuoteString("Narration".to_owned())),
//                 tags: vec![],
//                 links: vec![],
//                 lines: vec![a, b],
//                 metas: vec![],
//             };
//             let x1 = Directive::Transaction(transaction);
//
//             assert_eq!(x1, x);
//         }
//
//         #[test]
//         fn optional_total_price() {
//             let x = single_directive_parser(
//                 r#"1970-01-01 * "Payee" "Narration"
//                   Assets:123  -1 CNY
//                   Expenses:TestCategory:One 1 CCC @@ 1 CNY"#,
//             );
//
//             let a = TransactionLine {
//                 flag: None,
//                 account: Account::new(AccountType::Assets, vec!["123".to_owned()]),
//                 amount: Some((BigDecimal::from(-1i16), "CNY".to_string())),
//                 cost: None,
//                 cost_date: None,
//                 price: None,
//             };
//             let b = TransactionLine {
//                 flag: None,
//                 account: Account::new(
//                     AccountType::Expenses,
//                     vec!["TestCategory".to_owned(), "One".to_owned()],
//                 ),
//                 amount: Some((BigDecimal::from(1i16), "CCC".to_string())),
//                 cost: None,
//                 cost_date: None,
//                 price: Some(Price::Total((BigDecimal::from(1i16), "CNY".to_string()))),
//             };
//
//             let transaction = Transaction {
//                 date: NaiveDate::from_ymd(1970, 1, 1),
//                 flag: Some(Flag::Okay),
//                 payee: Some(ZhangString::QuoteString("Payee".to_owned())),
//                 narration: Some(ZhangString::QuoteString("Narration".to_owned())),
//                 tags: vec![],
//                 links: vec![],
//                 lines: vec![a, b],
//                 metas: vec![],
//             };
//             let x1 = Directive::Transaction(transaction);
//
//             assert_eq!(x1, x);
//         }
//
//         #[test]
//         fn with_optional_tags_without_payee() {
//             let x = single_directive_parser(
//                 r#"1970-01-01 *  "Narration" #mytag #tag2
//                   Assets:123  -1 CNY
//                   Expenses:TestCategory:One 1 CCC @@ 1 CNY"#,
//             );
//
//             let a = TransactionLine {
//                 flag: None,
//                 account: Account::new(AccountType::Assets, vec!["123".to_owned()]),
//                 amount: Some((BigDecimal::from(-1i16), "CNY".to_string())),
//                 cost: None,
//                 cost_date: None,
//                 price: None,
//             };
//             let b = TransactionLine {
//                 flag: None,
//                 account: Account::new(
//                     AccountType::Expenses,
//                     vec!["TestCategory".to_owned(), "One".to_owned()],
//                 ),
//                 amount: Some((BigDecimal::from(1i16), "CCC".to_string())),
//                 cost: None,
//                 cost_date: None,
//                 price: Some(Price::Total((BigDecimal::from(1i16), "CNY".to_string()))),
//             };
//
//             let transaction = Transaction {
//                 date: NaiveDate::from_ymd(1970, 1, 1),
//                 flag: Some(Flag::Okay),
//                 payee: None,
//                 narration: Some(ZhangString::QuoteString("Narration".to_owned())),
//                 tags: vec![
//                     ZhangString::UnquoteString("mytag".to_owned()),
//                     ZhangString::UnquoteString("tag2".to_owned()),
//                 ],
//                 links: vec![],
//                 lines: vec![a, b],
//                 metas: vec![],
//             };
//             let x1 = Directive::Transaction(transaction);
//
//             assert_eq!(x1, x);
//         }
//
//         #[test]
//         fn optional_tags() {
//             let x = single_directive_parser(
//                 r#"1970-01-01 * "Payee" "Narration" #mytag #tag2
//                   Assets:123  -1 CNY
//                   Expenses:TestCategory:One 1 CCC @@ 1 CNY"#,
//             );
//
//             let a = TransactionLine {
//                 flag: None,
//                 account: Account::new(AccountType::Assets, vec!["123".to_owned()]),
//                 amount: Some((BigDecimal::from(-1i16), "CNY".to_string())),
//                 cost: None,
//                 cost_date: None,
//                 price: None,
//             };
//             let b = TransactionLine {
//                 flag: None,
//                 account: Account::new(
//                     AccountType::Expenses,
//                     vec!["TestCategory".to_owned(), "One".to_owned()],
//                 ),
//                 amount: Some((BigDecimal::from(1i16), "CCC".to_string())),
//                 cost: None,
//                 cost_date: None,
//                 price: Some(Price::Total((BigDecimal::from(1i16), "CNY".to_string()))),
//             };
//
//             let transaction = Transaction {
//                 date: NaiveDate::from_ymd(1970, 1, 1),
//                 flag: Some(Flag::Okay),
//                 payee: Some(ZhangString::QuoteString("Payee".to_owned())),
//                 narration: Some(ZhangString::QuoteString("Narration".to_owned())),
//                 tags: vec![
//                     ZhangString::UnquoteString("mytag".to_owned()),
//                     ZhangString::UnquoteString("tag2".to_owned()),
//                 ],
//                 links: vec![],
//                 lines: vec![a, b],
//                 metas: vec![],
//             };
//             let x1 = Directive::Transaction(transaction);
//
//             assert_eq!(x1, x);
//         }
//
//         #[test]
//         fn optional_links() {
//             let x = single_directive_parser(
//                 r#"1970-01-01 * "Payee" "Narration" ^link1 ^link-2
//                   Assets:123  -1 CNY
//                   Expenses:TestCategory:One 1 CCC @@ 1 CNY"#,
//             );
//
//             let a = TransactionLine {
//                 flag: None,
//                 account: Account::new(AccountType::Assets, vec!["123".to_owned()]),
//                 amount: Some((BigDecimal::from(-1i16), "CNY".to_string())),
//                 cost: None,
//                 cost_date: None,
//                 price: None,
//             };
//             let b = TransactionLine {
//                 flag: None,
//                 account: Account::new(
//                     AccountType::Expenses,
//                     vec!["TestCategory".to_owned(), "One".to_owned()],
//                 ),
//                 amount: Some((BigDecimal::from(1i16), "CCC".to_string())),
//                 cost: None,
//                 cost_date: None,
//                 price: Some(Price::Total((BigDecimal::from(1i16), "CNY".to_string()))),
//             };
//
//             let transaction = Transaction {
//                 date: NaiveDate::from_ymd(1970, 1, 1),
//                 flag: Some(Flag::Okay),
//                 payee: Some(ZhangString::QuoteString("Payee".to_owned())),
//                 narration: Some(ZhangString::QuoteString("Narration".to_owned())),
//                 tags: vec![],
//                 links: vec![
//                     ZhangString::UnquoteString("link1".to_owned()),
//                     ZhangString::UnquoteString("link-2".to_owned()),
//                 ],
//                 lines: vec![a, b],
//                 metas: vec![],
//             };
//             let x1 = Directive::Transaction(transaction);
//
//             assert_eq!(x1, x);
//         }
//     }
//
//     mod pad {
//         use chrono::NaiveDate;
//
//         use crate::account::AccountType;
//         use crate::models::test::single_directive_parser;
//         use crate::models::{Account, Directive};
//
//         #[test]
//         fn pad_directive() {
//             let x = single_directive_parser(
//                 "1970-01-01 pad Assets:123:234:English:中文:日本語:한국어 Equity:ABC",
//             );
//             let directive = Directive::Pad {
//                 date: NaiveDate::from_ymd(1970, 1, 1),
//                 from: Account::new(
//                     AccountType::Assets,
//                     vec![
//                         "123".to_owned(),
//                         "234".to_owned(),
//                         "English".to_owned(),
//                         "中文".to_owned(),
//                         "日本語".to_owned(),
//                         "한국어".to_owned(),
//                     ],
//                 ),
//                 to: Account::new(AccountType::Equity, vec!["ABC".to_owned()]),
//             };
//
//             assert_eq!(directive, x);
//         }
//     }
//
//     mod balance {
//         use bigdecimal::BigDecimal;
//         use chrono::NaiveDate;
//
//         use crate::account::AccountType;
//         use crate::models::test::single_directive_parser;
//         use crate::models::{Account, Directive};
//
//         #[test]
//         fn balance_directive() {
//             let x = single_directive_parser(
//                 "1970-01-01 balance Assets:123:234:English:中文:日本語:한국어  1 CNY",
//             );
//             let directive = Directive::Balance {
//                 date: NaiveDate::from_ymd(1970, 1, 1),
//                 account: Account::new(
//                     AccountType::Assets,
//                     vec![
//                         "123".to_owned(),
//                         "234".to_owned(),
//                         "English".to_owned(),
//                         "中文".to_owned(),
//                         "日本語".to_owned(),
//                         "한국어".to_owned(),
//                     ],
//                 ),
//                 amount: (BigDecimal::from(1i16), "CNY".to_owned()),
//             };
//
//             assert_eq!(directive, x);
//         }
//     }
//
//     mod document {
//         use chrono::NaiveDate;
//
//         use crate::account::AccountType;
//         use crate::models::test::single_directive_parser;
//         use crate::models::{Account, Directive};
//
//         #[test]
//         fn empty_string() {
//             let x = single_directive_parser(r#"1970-01-01 document Assets:123 """#);
//             let directive = Directive::Document {
//                 date: NaiveDate::from_ymd(1970, 1, 1),
//                 account: Account::new(AccountType::Assets, vec!["123".to_owned()]),
//                 path: "".to_owned(),
//             };
//
//             assert_eq!(directive, x);
//         }
//
//         #[test]
//         fn has_document_content() {
//             let x = single_directive_parser(r#"1970-01-01 document Assets:123 "here I am""#);
//             let directive = Directive::Document {
//                 date: NaiveDate::from_ymd(1970, 1, 1),
//                 account: Account::new(AccountType::Assets, vec!["123".to_owned()]),
//                 path: "here I am".to_owned(),
//             };
//
//             assert_eq!(directive, x);
//         }
//     }
//
//     mod price {
//         use bigdecimal::BigDecimal;
//         use chrono::NaiveDate;
//
//         use crate::models::test::single_directive_parser;
//         use crate::models::Directive;
//
//         #[test]
//         fn test() {
//             let x = single_directive_parser(r#"1970-01-01 price USD   7 CNY"#);
//             let directive = Directive::Price {
//                 date: NaiveDate::from_ymd(1970, 1, 1),
//                 commodity: "USD".to_owned(),
//                 amount: (BigDecimal::from(7i16), "CNY".to_owned()),
//             };
//
//             assert_eq!(directive, x);
//         }
//     }
//
//     mod event {
//         use chrono::NaiveDate;
//
//         use crate::models::test::single_directive_parser;
//         use crate::models::Directive;
//
//         #[test]
//         fn test() {
//             let x = single_directive_parser(r#"1970-01-01 event "location"  "China""#);
//             let directive = Directive::Event {
//                 date: NaiveDate::from_ymd(1970, 1, 1),
//                 name: "location".to_owned(),
//                 value: "China".to_owned(),
//             };
//
//             assert_eq!(directive, x);
//         }
//     }
//
//     mod option {
//         use crate::models::test::single_directive_parser;
//         use crate::models::Directive;
//
//         #[test]
//         fn test() {
//             let x = single_directive_parser(r#"option "title"  "Personal""#);
//
//             let directive = Directive::Option {
//                 key: "title".to_owned(),
//                 value: "Personal".to_owned(),
//             };
//
//             assert_eq!(directive, x);
//         }
//     }
//
//     mod plugin {
//         use crate::models::test::single_directive_parser;
//         use crate::models::Directive;
//
//         #[test]
//         fn has_plugin_data() {
//             let x = single_directive_parser(r#"plugin "module name"  "config data""#);
//             let directive = Directive::Plugin {
//                 module: "module name".to_owned(),
//                 value: vec!["config data".to_owned()],
//             };
//
//             assert_eq!(directive, x);
//         }
//
//         #[test]
//         fn do_not_has_plugin_config_data() {
//             let x = single_directive_parser(r#"plugin "module name""#);
//             let directive = Directive::Plugin {
//                 module: "module name".to_owned(),
//                 value: vec![],
//             };
//
//             assert_eq!(directive, x);
//         }
//     }
//
//     mod include {
//         use crate::models::test::single_directive_parser;
//         use crate::models::Directive;
//
//         #[test]
//         fn has_plugin_data() {
//             let x = single_directive_parser(r#"include "file path""#);
//             let directive = Directive::Include {
//                 file: "file path".to_owned(),
//             };
//
//             assert_eq!(directive, x);
//         }
//     }
//
//     mod custom {
//         use std::str::FromStr;
//
//         use chrono::NaiveDate;
//
//         use crate::models::test::single_directive_parser;
//         use crate::models::Directive;
//         use crate::models::{Account, ZhangString, StringOrAccount};
//
//         #[test]
//         fn custom() {
//             let x = single_directive_parser(
//                 r#"2015-05-01 custom "budget" Expenses:Electricity  "quarterly"    85.00 EUR"#,
//             );
//             let directive = Directive::Custom {
//                 date: NaiveDate::from_ymd(2015, 5, 1),
//                 type_name: ZhangString::QuoteString("budget".to_owned()),
//                 values: vec![
//                     StringOrAccount::Account(Account::from_str("Expenses:Electricity").unwrap()),
//                     StringOrAccount::String(ZhangString::QuoteString("quarterly".to_owned())),
//                     StringOrAccount::String(ZhangString::UnquoteString("85.00".to_owned())),
//                     StringOrAccount::String(ZhangString::UnquoteString("EUR".to_owned())),
//                 ],
//             };
//
//             assert_eq!(directive, x);
//         }
//     }
//
//     mod comment {
//         use crate::models::test::single_directive_parser;
//         use crate::models::Directive;
//
//         #[test]
//         fn comma() {
//             let x = single_directive_parser(";你好啊");
//             let directive = Directive::Comment {
//                 content: ";你好啊".to_owned(),
//             };
//             assert_eq!(directive, x);
//         }
//
//         #[test]
//         fn two() {
//             let x = single_directive_parser("* 你好啊");
//             let directive = Directive::Comment {
//                 content: "* 你好啊".to_owned(),
//             };
//             assert_eq!(directive, x);
//         }
//     }
//
//     mod entry {
//         use chrono::NaiveDate;
//
//         use crate::account::AccountType;
//         use crate::models::{Account, Directive};
//         use crate::p::parse_zhang;
//
//         #[test]
//         fn conbine_test() {
//             let content: String = vec!["\n\n;你好啊", "1970-01-01 open Assets:Book\n"].join("\n");
//
//             let entry = parse_zhang(&content).unwrap();
//
//             let directives = vec![
//                 Directive::Comment {
//                     content: ";你好啊".to_owned(),
//                 },
//                 Directive::Open {
//                     date: NaiveDate::from_ymd(1970, 1, 1),
//                     account: Account {
//                         account_type: AccountType::Assets,
//                         value: vec!["Book".to_owned()],
//                     },
//                     commodities: vec![],
//                 },
//             ];
//
//             assert_eq!(directives, entry);
//         }
//     }
// }
