use crate::core::account::Account;
use crate::core::amount::Amount;
use crate::core::data::{Date, Posting, Transaction};
use crate::models::Flag;
use crate::target::AvaroTarget;
use crate::utils::escape_with_quote;
use itertools::Itertools;
use std::collections::HashMap;

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

impl AvaroTarget<Vec<String>> for HashMap<String, String> {
    fn to_target(self) -> Vec<String> {
        self.into_iter()
            .sorted()
            .map(|(k, v)| format!("{}: {}", k, v))
            .collect_vec()
    }
}

impl AvaroTarget<String> for Transaction {
    fn to_target(self) -> String {
        let mut vec1 = vec![
            Some(self.date.to_target()),
            self.flag.map(|it| format!(" {}", it.to_target())),
            self.payee.map(|it| escape_with_quote(&it).to_string()),
            self.narration.map(|it| escape_with_quote(&it).to_string()),
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
