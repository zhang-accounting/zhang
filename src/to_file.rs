use crate::{
    models::{Amount, Directive, Flag},
    utils::escape_with_quote,
};
use itertools::Itertools;

pub trait ToAvaroFile {
    fn to_text(&self) -> String;
}

impl ToAvaroFile for Amount {
    fn to_text(&self) -> String {
        format!("{} {}", self.0, self.1)
    }
}

impl ToAvaroFile for crate::models::Flag {
    fn to_text(&self) -> String {
        match self {
            Flag::Complete => "*".to_owned(),
            Flag::Incomplete => "!".to_owned(),
        }
    }
}

impl ToAvaroFile for crate::models::AccountType {
    fn to_text(&self) -> String {
        self.to_string()
    }
}

impl ToAvaroFile for crate::models::TransactionLine {
    fn to_text(&self) -> String {
        let mut builder = String::new();
        if self.flag != Flag::Complete {
            builder.push_str("! ");
        }
        builder.push_str(&self.account.to_string());
        if let Some(amount_inner) = &self.amount {
            builder.push_str(&format!(" {}", amount_inner.to_text()));
        };
        if let Some((amount, note)) = &self.cost {
            builder.push_str(" { ");
            builder.push_str(&amount.to_text());
            if let Some(note_inner) = note {
                builder.push_str(", ");
                builder.push_str(&escape_with_quote(note_inner));
            }
            builder.push_str(" }");
        };
        if let Some(single) = &self.single_price {
            builder.push_str(&format!(" @ {}", single.to_text()));
        };
        if let Some(inner) = &self.total_price {
            builder.push_str(&format!(" @@ {}", inner.to_text()));
        };
        builder
    }
}

impl ToAvaroFile for crate::models::Transaction {
    fn to_text(&self) -> String {
        let mut builder = String::new();
        builder.push_str(&self.date.to_string());
        builder.push_str(" ");
        builder.push_str(&self.flag.to_text());
        let pn = match (&self.payee, &self.narration) {
            (Some(payee), Some(narration)) => format!(
                " {} {}",
                escape_with_quote(payee),
                escape_with_quote(narration)
            ),
            (None, Some(narration)) => format!(" {}", escape_with_quote(narration)),
            _ => format!(""),
        };
        builder.push_str(&pn);

        let tags = self
            .tags
            .iter()
            .map(|inner| format!(" #{}", inner))
            .join("");
        builder.push_str(&tags);
        let links = self
            .links
            .iter()
            .map(|inner| format!(" ^{}", inner))
            .join("");
        builder.push_str(&links);

        let lines = self
            .lines
            .iter()
            .map(|line| format!("\n  {}", line.to_text()))
            .join("");
        builder.push_str(&lines);

        builder
    }
}

impl ToAvaroFile for crate::models::Directive {
    fn to_text(&self) -> String {
        match self {
            Directive::Open {
                date,
                account,
                commodities,
            } => {
                let mut string = format!(
                    "{date} open {account}",
                    date = &date.to_string(),
                    account = &account.to_string()
                );
                if let Some(commodities_data) = commodities {
                    string.push(' ');
                    string.push_str(&commodities_data.iter().join(", "));
                };
                string
            }

            Directive::Close { date, account } => format!(
                "{date} close {account}",
                date = &date.to_string(),
                account = &account.to_string()
            ),
            Directive::Commodity { date, name, metas } => {
                let meta_info = metas
                    .iter()
                    .map(|(key, value)| {
                        format!("\n  {}: {}", key.clone(), escape_with_quote(value))
                    })
                    .join("");
                format!(
                    "{date} commodity {name}{meta_info}",
                    date = &date.to_string(),
                    name = name,
                    meta_info = meta_info
                )
            }
            Directive::Transaction(model) => model.to_text(),
            Directive::Balance {
                date,
                account,
                amount,
            } => format!(
                "{date} balance {account} {amount}",
                date = date.to_string(),
                account = account.to_string(),
                amount = amount.to_text()
            ),
            Directive::Pad { date, from, to } => format!(
                "{date} pad {from} {to}",
                date = date.to_string(),
                from = from.to_string(),
                to = to.to_string()
            ),
            Directive::Note {
                date,
                account,
                description,
            } => format!(
                "{date} note {account} {description}",
                date = date.to_string(),
                account = account.to_string(),
                description = escape_with_quote(description)
            ),
            Directive::Document {
                date,
                account,
                path,
            } => format!(
                "{date} document {account} {path}",
                date = date.to_string(),
                account = account.to_string(),
                path = escape_with_quote(path)
            ),
            Directive::Price {
                date,
                commodity,
                amount,
            } => format!(
                "{date} price {commodity} {amount}",
                date = date.to_string(),
                commodity = commodity,
                amount = amount.to_text()
            ),
            Directive::Event { date, name, value } => format!(
                "{date} event {name} {value}",
                date = date.to_string(),
                name = escape_with_quote(name),
                value = escape_with_quote(value),
            ),
            Directive::Custom {
                date,
                type_name,
                values,
            } => format!(
                "{date} custom {type_name} {value}",
                date = date.to_string(),
                type_name = escape_with_quote(type_name),
                value = values.iter().map(|v| escape_with_quote(v)).join(" ")
            ),
            Directive::Option { key, value } => format!(
                "option {} {}",
                escape_with_quote(key),
                escape_with_quote(value)
            ),
            Directive::Plugin { module, value } => {
                let mut builder = format!("plugin {}", escape_with_quote(module),);
                if let Some(inner) = value {
                    builder.push_str(&format!(" {}", escape_with_quote(inner)));
                }
                builder
            }
            Directive::Include { file } => format!("include {}", escape_with_quote(file)),
            Directive::Comment { content } => content.to_owned(),
        }
    }
}

#[cfg(test)]
mod test {
    use crate::{models::Directive, parser::DirectiveExpressionParser, to_file::ToAvaroFile};

    fn parse(from: &str) -> String {
        let direct: Directive = DirectiveExpressionParser::new().parse(from).unwrap();
        direct.to_text()
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
            "1970-01-01 * \"Narration\"\n  Assets:123 -1 CNY { 0.1 USD, \"TEST\" }\n  Expenses:TestCategory:One 1 CNY { 0.1 USD }",
            parse(r#"1970-01-01 * "Narration"
                  Assets:123  -1 CNY {0.1 USD , "TEST"}
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
    #[ignore]
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
