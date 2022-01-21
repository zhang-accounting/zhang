use itertools::Itertools;

use crate::core::account::Account;
use crate::core::amount::Amount;
use crate::core::data::{
    Balance, Close, Commodity, Custom, Date, Document, Event, Meta, Note, Open, Pad, Posting,
    Price, Transaction,
};
use crate::core::ledger::Ledger;
use crate::core::models::{AvaroString, Directive, Flag, StringOrAccount};
use crate::target::AvaroTarget;
use crate::utils::escape_with_quote;

fn append_meta(meta: Meta, string: String) -> String {
    let mut metas = meta
        .to_target()
        .into_iter()
        .map(|it| format!("  {}", it))
        .collect_vec();
    metas.insert(0, string);
    metas.join("\n")
}

impl AvaroTarget<String> for Date {
    fn to_target(self) -> String {
        match self {
            Date::Date(date) => date.format("%Y-%m-%d").to_string(),
            Date::Datetime(datetime) => datetime.format("%Y-%m-%d %H:%M:%S").to_string(),
        }
    }
}

impl AvaroTarget<String> for Flag {
    fn to_target(self) -> String {
        self.to_string()
    }
}

impl AvaroTarget<String> for Account {
    fn to_target(self) -> String {
        self.content
    }
}
impl AvaroTarget<String> for Amount {
    fn to_target(self) -> String {
        format!("{} {}", self.number, self.currency)
    }
}

impl AvaroTarget<Vec<String>> for Meta {
    fn to_target(self) -> Vec<String> {
        self.into_iter()
            .sorted_by(|entry_a, entry_b| entry_a.0.cmp(&entry_b.0))
            .map(|(k, v)| format!("{}: {}", k, v.to_target()))
            .collect_vec()
    }
}

impl AvaroTarget<String> for AvaroString {
    fn to_target(self) -> String {
        match self {
            AvaroString::UnquoteString(unquote) => unquote,
            AvaroString::QuoteString(quote) => escape_with_quote(&quote).to_string(),
        }
    }
}

impl AvaroTarget<String> for StringOrAccount {
    fn to_target(self) -> String {
        match self {
            StringOrAccount::String(s) => s.to_target(),
            StringOrAccount::Account(account) => account.to_target(),
        }
    }
}

impl AvaroTarget<String> for Transaction {
    fn to_target(self) -> String {
        let mut vec1 = vec![
            Some(self.date.to_target()),
            self.flag.map(|it| format!(" {}", it.to_target())),
            self.payee.map(|it| it.to_target()),
            self.narration.map(|it| it.to_target()),
        ];
        let mut tags = self
            .tags
            .into_iter()
            .map(|it| Some(format!("#{}", it)))
            .collect_vec();
        let mut links = self
            .links
            .into_iter()
            .map(|it| Some(format!("^{}", it)))
            .collect_vec();
        vec1.append(&mut tags);
        vec1.append(&mut links);

        let mut transaction = self
            .postings
            .into_iter()
            .map(|it| format!("  {}", it.to_target()))
            .collect_vec();
        transaction.insert(0, vec1.into_iter().flatten().join(" "));
        let mut vec2 = self
            .meta
            .to_target()
            .into_iter()
            .map(|it| format!("  {}", it))
            .collect_vec();
        transaction.append(&mut vec2);

        transaction.into_iter().join("\n")
    }
}

impl AvaroTarget<String> for Posting {
    fn to_target(self) -> String {
        // todo cost and price
        let vec1 = vec![
            self.flag.map(|it| format!(" {}", it.to_target())),
            Some(self.account.to_target()),
            self.units.map(|it| it.to_target()),
        ];

        vec1.into_iter().flatten().join(" ")
    }
}
impl AvaroTarget<String> for Open {
    fn to_target(mut self) -> String {
        let mut line = vec![
            self.date.to_target(),
            "open".to_string(),
            self.account.to_target(),
        ];
        line.append(&mut self.commodities);
        append_meta(self.meta, line.join(" "))
    }
}

impl AvaroTarget<String> for Close {
    fn to_target(self) -> String {
        let mut line = vec![
            self.date.to_target(),
            "close".to_string(),
            self.account.to_target(),
        ];
        append_meta(self.meta, line.join(" "))
    }
}

impl AvaroTarget<String> for Commodity {
    fn to_target(self) -> String {
        let mut line = vec![
            self.date.to_target(),
            "commodity".to_string(),
            self.currency,
        ];
        append_meta(self.meta, line.join(" "))
    }
}

impl AvaroTarget<String> for Balance {
    fn to_target(self) -> String {
        let mut line = vec![
            self.date.to_target(),
            "balance".to_string(),
            self.account.to_target(),
            self.amount.to_target(),
        ];
        append_meta(self.meta, line.join(" "))
    }
}

impl AvaroTarget<String> for Pad {
    fn to_target(self) -> String {
        let mut line = vec![
            self.date.to_target(),
            "pad".to_string(),
            self.account.to_target(),
            self.source.to_target(),
        ];
        append_meta(self.meta, line.join(" "))
    }
}

impl AvaroTarget<String> for Note {
    fn to_target(self) -> String {
        let mut line = vec![
            self.date.to_target(),
            "note".to_string(),
            self.account.to_target(),
            self.comment.to_target(),
        ];
        append_meta(self.meta, line.join(" "))
    }
}

impl AvaroTarget<String> for Document {
    fn to_target(self) -> String {
        let mut line = vec![
            self.date.to_target(),
            "document".to_string(),
            self.account.to_target(),
            self.filename.to_target(),
        ];
        append_meta(self.meta, line.join(" "))
    }
}

impl AvaroTarget<String> for Price {
    fn to_target(self) -> String {
        let mut line = vec![
            self.date.to_target(),
            "price".to_string(),
            self.currency,
            self.amount.to_target(),
        ];
        append_meta(self.meta, line.join(" "))
    }
}

impl AvaroTarget<String> for Event {
    fn to_target(self) -> String {
        let mut line = vec![
            self.date.to_target(),
            "event".to_string(),
            self.event_type.to_target(),
            self.description.to_target(),
        ];
        append_meta(self.meta, line.join(" "))
    }
}

impl AvaroTarget<String> for Custom {
    fn to_target(self) -> String {
        let mut line = vec![
            self.date.to_target(),
            "custom".to_string(),
            self.custom_type.to_target(),
        ];
        let mut values = self
            .values
            .into_iter()
            .map(|it| it.to_target())
            .collect_vec();
        line.append(&mut values);
        append_meta(self.meta, line.join(" "))
    }
}

impl AvaroTarget<String> for Directive {
    fn to_target(self) -> String {
        match self {
            Directive::Open(open) => open.to_target(),
            Directive::Close(close) => close.to_target(),
            Directive::Commodity(commodity) => commodity.to_target(),
            Directive::Transaction(txn) => txn.to_target(),
            Directive::Balance(balance) => balance.to_target(),
            Directive::Pad(pad) => pad.to_target(),
            Directive::Note(note) => note.to_target(),
            Directive::Document(document) => document.to_target(),
            Directive::Price(price) => price.to_target(),
            Directive::Event(event) => event.to_target(),
            Directive::Custom(cusotm) => cusotm.to_target(),
            Directive::Option { key, value } => {
                let mut line = vec!["option".to_string(), key.to_target(), value.to_target()];
                line.join(" ")
            }
            Directive::Plugin { module, value } => {
                let mut line = vec!["plugin".to_string(), module.to_target()];
                let mut values = value.into_iter().map(|it| it.to_target()).collect_vec();
                line.append(&mut values);
                line.join(" ")
            }
            Directive::Include { file } => {
                let mut line = vec!["include".to_string(), file.to_target()];
                line.join(" ")
            }
            Directive::Comment { content } => {
                let mut line = vec!["#".to_string(), content];
                line.join(" ")
            }
        }
    }
}

impl AvaroTarget<String> for Ledger {
    fn to_target(self) -> String {
        let vec = self
            .directives
            .into_iter()
            .map(|it| it.to_target())
            .collect_vec();
        vec.join("\n\n")
    }
}

//
//
// #[cfg(test)]
// mod test {
//     use crate::p::parse_avaro;
//
//     fn parse(from: &str) -> String {
//         let directive = parse_avaro(from).unwrap().into_iter().next().unwrap();
//         directive.to_target()
//     }
//
//     fn parse_and_test(from: &str) {
//         assert_eq!(from, parse(from));
//     }
//
//     #[test]
//     fn open_to_text() {
//         parse_and_test("1970-01-01 open Equity:hello CNY");
//     }
//
//     #[test]
//     fn balance() {
//         parse_and_test("1970-01-01 balance Equity:hello 10 CNY");
//     }
//
//     #[test]
//     fn option() {
//         parse_and_test("option \"hello\" \"value\"");
//     }
//
//     #[test]
//     fn close() {
//         parse_and_test("1970-01-01 close Equity:hello");
//     }
//
//     #[test]
//     fn commodity() {
//         parse_and_test("1970-01-01 commodity CNY");
//         parse_and_test("1970-01-01 commodity CNY\n  a: \"b\"");
//     }
//
//     #[test]
//     fn transaction() {
//         assert_eq!(
//             "1970-01-01 * \"Payee\" \"Narration\"\n  Assets:123 -1 CNY\n  Expenses:TestCategory:One 1 CNY",
//             parse(r#"1970-01-01 * "Payee" "Narration"
//                   Assets:123  -1 CNY
//                   Expenses:TestCategory:One 1 CNY"#)
//         );
//         assert_eq!(
//             "1970-01-01 * \"Narration\"\n  Assets:123 -1 CNY\n  Expenses:TestCategory:One 1 CNY",
//             parse(
//                 r#"1970-01-01 * "Narration"
//                   Assets:123  -1 CNY
//                   Expenses:TestCategory:One 1 CNY"#
//             )
//         );
//
//         assert_eq!(
//             "1970-01-01 * \"Narration\"\n  Assets:123 -1 CNY { 0.1 USD, 2111-11-11 }\n  Expenses:TestCategory:One 1 CNY { 0.1 USD }",
//             parse(r#"1970-01-01 * "Narration"
//                   Assets:123  -1 CNY {0.1 USD , 2111-11-11}
//                   Expenses:TestCategory:One 1 CNY {0.1 USD}"#)
//         );
//         assert_eq!(
//             "1970-01-01 * \"Payee\" \"Narration\"\n  Assets:123 -1 CNY\n  Expenses:TestCategory:One 0.5 CNY\n  Expenses:TestCategory:Two 0.5 CNY",
//             parse(r#"1970-01-01 * "Payee" "Narration"
//                   Assets:123  -1 CNY
//                   Expenses:TestCategory:One 0.5 CNY
//                   Expenses:TestCategory:Two 0.5 CNY"#)
//         );
//         assert_eq!(
//             "1970-01-01 * \"Payee\" \"Narration\"\n  Assets:123 -1 CNY\n  Expenses:TestCategory:One",
//             parse(r#"1970-01-01 * "Payee" "Narration"
//                   Assets:123  -1 CNY
//                   Expenses:TestCategory:One"#)
//         );
//         assert_eq!(
//             "1970-01-01 * \"Payee\" \"Narration\"\n  Assets:123 -1 CNY\n  Expenses:TestCategory:One 1 CCC @ 1 CNY",
//             parse(r#"1970-01-01 * "Payee" "Narration"
//                   Assets:123  -1 CNY
//                   Expenses:TestCategory:One 1 CCC @ 1 CNY"#)
//         );
//         assert_eq!(
//             "1970-01-01 * \"Payee\" \"Narration\"\n  Assets:123 -1 CNY\n  Expenses:TestCategory:One 1 CCC @@ 1 CNY",
//             parse(r#"1970-01-01 * "Payee" "Narration"
//                   Assets:123  -1 CNY
//                   Expenses:TestCategory:One 1 CCC @@ 1 CNY"#)
//         );
//         assert_eq!(
//             "1970-01-01 * \"Narration\" #mytag #tag2\n  Assets:123 -1 CNY\n  Expenses:TestCategory:One 1 CCC @@ 1 CNY",
//             parse(r#"1970-01-01 *  "Narration" #mytag #tag2
//                   Assets:123  -1 CNY
//                   Expenses:TestCategory:One 1 CCC @@ 1 CNY"#)
//         );
//         assert_eq!(
//             "1970-01-01 * \"Payee\" \"Narration\" #mytag #tag2\n  Assets:123 -1 CNY\n  Expenses:TestCategory:One 1 CCC @@ 1 CNY",
//             parse(r#"1970-01-01 * "Payee" "Narration" #mytag #tag2
//                   Assets:123  -1 CNY
//                   Expenses:TestCategory:One 1 CCC @@ 1 CNY"#)
//         );
//         assert_eq!(
//             "1970-01-01 * \"Payee\" \"Narration\" ^link1 ^link-2\n  Assets:123 -1 CNY\n  Expenses:TestCategory:One 1 CCC @@ 1 CNY",
//             parse(r#"1970-01-01 * "Payee" "Narration" ^link1 ^link-2
//                   Assets:123  -1 CNY
//                   Expenses:TestCategory:One 1 CCC @@ 1 CNY"#)
//         );
//     }
//
//     #[test]
//     fn pad() {
//         parse_and_test("1970-01-01 pad Assets:123:234:English:中文:日本語:한국어 Equity:ABC");
//     }
//
//     #[test]
//     fn note() {
//         parse_and_test(r#"1970-01-01 note Assets:123 "你 好 啊\\""#);
//     }
//
//     #[test]
//     fn document() {
//         parse_and_test("1970-01-01 document Assets:123 \"\"");
//         parse_and_test(r#"1970-01-01 document Assets:123 "here I am""#);
//     }
//
//     #[test]
//     fn price() {
//         parse_and_test(r#"1970-01-01 price USD 7 CNY"#);
//     }
//
//     #[test]
//     fn event() {
//         parse_and_test(r#"1970-01-01 event "location" "China""#);
//     }
//
//     #[test]
//     fn custom() {
//         parse_and_test(r#"1970-01-01 custom "budget" Expenses:Eat "monthly" CNY"#);
//     }
//
//     #[test]
//     fn plugin() {
//         parse_and_test(r#"plugin "module name" "config data""#);
//         parse_and_test(r#"plugin "module name""#);
//     }
//
//     #[test]
//     fn include() {
//         parse_and_test(r#"include "file path""#);
//     }
//
//     #[test]
//     fn comment() {
//         parse_and_test(";你好啊");
//     }
// }
