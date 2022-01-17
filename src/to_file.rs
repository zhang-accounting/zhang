use itertools::Itertools;

use crate::models::{ Directive};
use crate::{
    models::{ Flag},
    utils::escape_with_quote,
};
use crate::amount::Amount;

pub trait ToAvaroFile {
    fn to_text(&self) -> String;
}

impl ToAvaroFile for Amount {
    fn to_text(&self) -> String {
        format!("{} {}", self.number, self.currency)
    }
}

impl ToAvaroFile for crate::models::Flag {
    fn to_text(&self) -> String {
        match self {
            Flag::Okay => "*".to_owned(),
            Flag::Warning => "!".to_owned(),
        }
    }
}

impl ToAvaroFile for crate::account::AccountType {
    fn to_text(&self) -> String {
        self.to_string()
    }
}

impl ToAvaroFile for String {
    fn to_text(&self) -> String {
        self.to_string()
    }
}

//
//
// impl ToAvaroFile for crate::models::Directive {
//     fn to_text(&self) -> String {
//         match self {
//             Directive::Open {
//                 date,
//                 account,
//                 commodities,
//             } => {
//                 let mut string = format!(
//                     "{date} open {account}",
//                     date = &date.to_string(),
//                     account = &account.to_string()
//                 );
//                 if !commodities.is_empty() {
//                     string.push(' ');
//                     string.push_str(&commodities.iter().join(", "));
//                 };
//                 string
//             }
//
//             Directive::Close { date, account } => format!(
//                 "{date} close {account}",
//                 date = &date.to_string(),
//                 account = &account.to_string()
//             ),
//             Directive::Commodity { date, name, metas } => {
//                 let meta_info = metas
//                     .iter()
//                     .map(|(key, value)| format!("\n  {}: {}", key.to_text(), value.to_text()))
//                     .join("");
//                 format!(
//                     "{date} commodity {name}{meta_info}",
//                     date = &date.to_string(),
//                     name = name,
//                     meta_info = meta_info
//                 )
//             }
//             Directive::Transaction(model) => model.to_text(),
//             Directive::Balance {
//                 date,
//                 account,
//                 amount,
//             } => format!(
//                 "{date} balance {account} {amount}",
//                 date = date.to_string(),
//                 account = account.to_string(),
//                 amount = amount.to_text()
//             ),
//             Directive::Pad { date, from, to } => format!(
//                 "{date} pad {from} {to}",
//                 date = date.to_string(),
//                 from = from.to_string(),
//                 to = to.to_string()
//             ),
//             Directive::Note {
//                 date,
//                 account,
//                 description,
//             } => format!(
//                 "{date} note {account} {description}",
//                 date = date.to_string(),
//                 account = account.to_string(),
//                 description = escape_with_quote(description)
//             ),
//             Directive::Document {
//                 date,
//                 account,
//                 path,
//             } => format!(
//                 "{date} document {account} {path}",
//                 date = date.to_string(),
//                 account = account.to_string(),
//                 path = escape_with_quote(path)
//             ),
//             Directive::Price {
//                 date,
//                 commodity,
//                 amount,
//             } => format!(
//                 "{date} price {commodity} {amount}",
//                 date = date.to_string(),
//                 commodity = commodity,
//                 amount = amount.to_text()
//             ),
//             Directive::Event { date, name, value } => format!(
//                 "{date} event {name} {value}",
//                 date = date.to_string(),
//                 name = escape_with_quote(name),
//                 value = escape_with_quote(value),
//             ),
//             Directive::Custom {
//                 date,
//                 type_name,
//                 values,
//             } => format!(
//                 "{date} custom {type_name} {value}",
//                 date = date.to_string(),
//                 type_name = type_name.to_text(),
//                 value = values.iter().map(|v| v.to_string()).join(" ")
//             ),
//             Directive::Option { key, value } => format!(
//                 "option {} {}",
//                 escape_with_quote(key),
//                 escape_with_quote(value)
//             ),
//             Directive::Plugin { module, value } => {
//                 let mut builder = format!("plugin {}", escape_with_quote(module),);
//                 for item in value {
//                     builder.push_str(&format!(" {}", escape_with_quote(item)));
//                 }
//                 builder
//             }
//             Directive::Include { file } => format!("include {}", escape_with_quote(file)),
//             Directive::Comment { content } => content.to_owned(),
//         }
//     }
// }

#[cfg(test)]
mod test {
    use crate::p::parse_avaro;
    use crate::to_file::ToAvaroFile;

    fn parse(from: &str) -> String {
        let directive = parse_avaro(from).unwrap().into_iter().next().unwrap();
        directive.to_text()
    }

    fn parse_and_test(from: &str) {
        assert_eq!(from, parse(from));
    }

    #[test]
    fn open_to_text() {
        parse_and_test("1970-01-01 open Equity:hello CNY");
    }

    #[test]
    fn balance() {
        parse_and_test("1970-01-01 balance Equity:hello 10 CNY");
    }

    #[test]
    fn option() {
        parse_and_test("option \"hello\" \"value\"");
    }

    #[test]
    fn close() {
        parse_and_test("1970-01-01 close Equity:hello");
    }

    #[test]
    fn commodity() {
        parse_and_test("1970-01-01 commodity CNY");
        parse_and_test("1970-01-01 commodity CNY\n  a: \"b\"");
    }

    #[test]
    fn transaction() {
        assert_eq!(
            "1970-01-01 * \"Payee\" \"Narration\"\n  Assets:123 -1 CNY\n  Expenses:TestCategory:One 1 CNY",
            parse(r#"1970-01-01 * "Payee" "Narration"
                  Assets:123  -1 CNY
                  Expenses:TestCategory:One 1 CNY"#)
        );
        assert_eq!(
            "1970-01-01 * \"Narration\"\n  Assets:123 -1 CNY\n  Expenses:TestCategory:One 1 CNY",
            parse(
                r#"1970-01-01 * "Narration"
                  Assets:123  -1 CNY
                  Expenses:TestCategory:One 1 CNY"#
            )
        );

        assert_eq!(
            "1970-01-01 * \"Narration\"\n  Assets:123 -1 CNY { 0.1 USD, 2111-11-11 }\n  Expenses:TestCategory:One 1 CNY { 0.1 USD }",
            parse(r#"1970-01-01 * "Narration"
                  Assets:123  -1 CNY {0.1 USD , 2111-11-11}
                  Expenses:TestCategory:One 1 CNY {0.1 USD}"#)
        );
        assert_eq!(
            "1970-01-01 * \"Payee\" \"Narration\"\n  Assets:123 -1 CNY\n  Expenses:TestCategory:One 0.5 CNY\n  Expenses:TestCategory:Two 0.5 CNY",
            parse(r#"1970-01-01 * "Payee" "Narration"
                  Assets:123  -1 CNY
                  Expenses:TestCategory:One 0.5 CNY
                  Expenses:TestCategory:Two 0.5 CNY"#)
        );
        assert_eq!(
            "1970-01-01 * \"Payee\" \"Narration\"\n  Assets:123 -1 CNY\n  Expenses:TestCategory:One",
            parse(r#"1970-01-01 * "Payee" "Narration"
                  Assets:123  -1 CNY
                  Expenses:TestCategory:One"#)
        );
        assert_eq!(
            "1970-01-01 * \"Payee\" \"Narration\"\n  Assets:123 -1 CNY\n  Expenses:TestCategory:One 1 CCC @ 1 CNY",
            parse(r#"1970-01-01 * "Payee" "Narration"
                  Assets:123  -1 CNY
                  Expenses:TestCategory:One 1 CCC @ 1 CNY"#)
        );
        assert_eq!(
            "1970-01-01 * \"Payee\" \"Narration\"\n  Assets:123 -1 CNY\n  Expenses:TestCategory:One 1 CCC @@ 1 CNY",
            parse(r#"1970-01-01 * "Payee" "Narration"
                  Assets:123  -1 CNY
                  Expenses:TestCategory:One 1 CCC @@ 1 CNY"#)
        );
        assert_eq!(
            "1970-01-01 * \"Narration\" #mytag #tag2\n  Assets:123 -1 CNY\n  Expenses:TestCategory:One 1 CCC @@ 1 CNY",
            parse(r#"1970-01-01 *  "Narration" #mytag #tag2
                  Assets:123  -1 CNY
                  Expenses:TestCategory:One 1 CCC @@ 1 CNY"#)
        );
        assert_eq!(
            "1970-01-01 * \"Payee\" \"Narration\" #mytag #tag2\n  Assets:123 -1 CNY\n  Expenses:TestCategory:One 1 CCC @@ 1 CNY",
            parse(r#"1970-01-01 * "Payee" "Narration" #mytag #tag2
                  Assets:123  -1 CNY
                  Expenses:TestCategory:One 1 CCC @@ 1 CNY"#)
        );
        assert_eq!(
            "1970-01-01 * \"Payee\" \"Narration\" ^link1 ^link-2\n  Assets:123 -1 CNY\n  Expenses:TestCategory:One 1 CCC @@ 1 CNY",
            parse(r#"1970-01-01 * "Payee" "Narration" ^link1 ^link-2
                  Assets:123  -1 CNY
                  Expenses:TestCategory:One 1 CCC @@ 1 CNY"#)
        );
    }

    #[test]
    fn pad() {
        parse_and_test("1970-01-01 pad Assets:123:234:English:中文:日本語:한국어 Equity:ABC");
    }

    #[test]
    fn note() {
        parse_and_test(r#"1970-01-01 note Assets:123 "你 好 啊\\""#);
    }

    #[test]
    fn document() {
        parse_and_test("1970-01-01 document Assets:123 \"\"");
        parse_and_test(r#"1970-01-01 document Assets:123 "here I am""#);
    }

    #[test]
    fn price() {
        parse_and_test(r#"1970-01-01 price USD 7 CNY"#);
    }

    #[test]
    fn event() {
        parse_and_test(r#"1970-01-01 event "location" "China""#);
    }

    #[test]
    fn custom() {
        parse_and_test(r#"1970-01-01 custom "budget" Expenses:Eat "monthly" CNY"#);
    }

    #[test]
    fn plugin() {
        parse_and_test(r#"plugin "module name" "config data""#);
        parse_and_test(r#"plugin "module name""#);
    }

    #[test]
    fn include() {
        parse_and_test(r#"include "file path""#);
    }

    #[test]
    fn comment() {
        parse_and_test(";你好啊");
    }
}
