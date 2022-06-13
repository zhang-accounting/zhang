use itertools::Itertools;
use std::fmt::format;

use crate::core::account::Account;
use crate::core::amount::Amount;
use crate::core::data::{
    Balance, Close, Comment, Commodity, Custom, Date, Document, Event, Include, Meta, Note, Open, Options, Plugin,
    Posting, Price, Transaction,
};
use crate::core::ledger::Ledger;
use crate::core::models::{Directive, Flag, SingleTotalPrice, StringOrAccount, ZhangString};
use crate::target::ZhangTarget;
use crate::utils::escape_with_quote;

fn append_meta(meta: Meta, string: String) -> String {
    let mut metas = meta.to_target().into_iter().map(|it| format!("  {}", it)).collect_vec();
    metas.insert(0, string);
    metas.join("\n")
}

impl ZhangTarget<String> for Date {
    fn to_target(self) -> String {
        match self {
            Date::Date(date) => date.format("%Y-%m-%d").to_string(),
            Date::Datetime(datetime) => datetime.format("%Y-%m-%d %H:%M:%S").to_string(),
            Date::DateHour(datehour) => datehour.format("%Y-%m-%d %H:%M").to_string(),
        }
    }
}

impl ZhangTarget<String> for Flag {
    fn to_target(self) -> String {
        self.to_string()
    }
}

impl ZhangTarget<String> for Account {
    fn to_target(self) -> String {
        self.content
    }
}
impl ZhangTarget<String> for Amount {
    fn to_target(self) -> String {
        format!("{} {}", self.number, self.currency)
    }
}

impl ZhangTarget<Vec<String>> for Meta {
    fn to_target(self) -> Vec<String> {
        self.get_flatten()
            .into_iter()
            .sorted_by(|entry_a, entry_b| entry_a.0.cmp(&entry_b.0))
            .map(|(k, v)| format!("{}: {}", k, v.to_target()))
            .collect_vec()
    }
}

impl ZhangTarget<String> for ZhangString {
    fn to_target(self) -> String {
        match self {
            ZhangString::UnquoteString(unquote) => unquote,
            ZhangString::QuoteString(quote) => escape_with_quote(&quote).to_string(),
        }
    }
}

impl ZhangTarget<String> for StringOrAccount {
    fn to_target(self) -> String {
        match self {
            StringOrAccount::String(s) => s.to_target(),
            StringOrAccount::Account(account) => account.to_target(),
        }
    }
}

impl ZhangTarget<String> for Transaction {
    fn to_target(self) -> String {
        let mut vec1 = vec![
            Some(self.date.to_target()),
            self.flag.map(|it| it.to_target()),
            self.payee.map(|it| it.to_target()),
            self.narration.map(|it| it.to_target()),
        ];
        let mut tags = self.tags.into_iter().map(|it| Some(format!("#{}", it))).collect_vec();
        let mut links = self.links.into_iter().map(|it| Some(format!("^{}", it))).collect_vec();
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

impl ZhangTarget<String> for Posting {
    fn to_target(self) -> String {
        // todo cost and price
        let vec1 = vec![
            self.flag.map(|it| format!(" {}", it.to_target())),
            Some(self.account.to_target()),
            self.units.map(|it| it.to_target()),
            // todo: cost
            self.price.map(|it| it.to_target()),
        ];

        vec1.into_iter().flatten().join(" ")
    }
}

impl ZhangTarget<String> for SingleTotalPrice {
    fn to_target(self) -> String {
        match self {
            SingleTotalPrice::Single(single_price) => {
                format!("@ {}", single_price.to_target())
            }
            SingleTotalPrice::Total(total_price) => {
                format!("@@ {}", total_price.to_target())
            }
        }
    }
}

impl ZhangTarget<String> for Open {
    fn to_target(self) -> String {
        let mut line = vec![self.date.to_target(), "open".to_string(), self.account.to_target()];
        let commodities = self.commodities.iter().join(", ");
        line.push(commodities);
        append_meta(self.meta, line.join(" "))
    }
}

impl ZhangTarget<String> for Close {
    fn to_target(self) -> String {
        let line = vec![self.date.to_target(), "close".to_string(), self.account.to_target()];
        append_meta(self.meta, line.join(" "))
    }
}

impl ZhangTarget<String> for Commodity {
    fn to_target(self) -> String {
        let line = vec![self.date.to_target(), "commodity".to_string(), self.currency];
        append_meta(self.meta, line.join(" "))
    }
}

impl ZhangTarget<String> for Balance {
    fn to_target(self) -> String {
        match self {
            Balance::BalanceCheck(check) => {
                let line = vec![
                    check.date.to_target(),
                    "balance".to_string(),
                    check.account.to_target(),
                    check.amount.to_target(),
                ];
                append_meta(check.meta, line.join(" "))
            }
            Balance::BalancePad(pad) => {
                let line = vec![
                    pad.date.to_target(),
                    "balance".to_string(),
                    pad.account.to_target(),
                    pad.amount.to_target(),
                    "with pad".to_string(),
                    pad.pad.to_target(),
                ];
                append_meta(pad.meta, line.join(" "))
            }
        }
    }
}

impl ZhangTarget<String> for Note {
    fn to_target(self) -> String {
        let line = vec![
            self.date.to_target(),
            "note".to_string(),
            self.account.to_target(),
            self.comment.to_target(),
        ];
        append_meta(self.meta, line.join(" "))
    }
}

impl ZhangTarget<String> for Document {
    fn to_target(self) -> String {
        let line = vec![
            self.date.to_target(),
            "document".to_string(),
            self.account.to_target(),
            self.filename.to_target(),
        ];
        append_meta(self.meta, line.join(" "))
    }
}

impl ZhangTarget<String> for Price {
    fn to_target(self) -> String {
        let line = vec![
            self.date.to_target(),
            "price".to_string(),
            self.currency,
            self.amount.to_target(),
        ];
        append_meta(self.meta, line.join(" "))
    }
}

impl ZhangTarget<String> for Event {
    fn to_target(self) -> String {
        let line = vec![
            self.date.to_target(),
            "event".to_string(),
            self.event_type.to_target(),
            self.description.to_target(),
        ];
        append_meta(self.meta, line.join(" "))
    }
}

impl ZhangTarget<String> for Custom {
    fn to_target(self) -> String {
        let mut line = vec![
            self.date.to_target(),
            "custom".to_string(),
            self.custom_type.to_target(),
        ];
        let mut values = self.values.into_iter().map(|it| it.to_target()).collect_vec();
        line.append(&mut values);
        append_meta(self.meta, line.join(" "))
    }
}

impl ZhangTarget<String> for Options {
    fn to_target(self) -> String {
        let line = vec!["option".to_string(), self.key.to_target(), self.value.to_target()];
        line.join(" ")
    }
}
impl ZhangTarget<String> for Plugin {
    fn to_target(self) -> String {
        let mut line = vec!["plugin".to_string(), self.module.to_target()];
        let mut values = self.value.into_iter().map(|it| it.to_target()).collect_vec();
        line.append(&mut values);
        line.join(" ")
    }
}
impl ZhangTarget<String> for Include {
    fn to_target(self) -> String {
        let line = vec!["include".to_string(), self.file.to_target()];
        line.join(" ")
    }
}

impl ZhangTarget<String> for Comment {
    fn to_target(self) -> String {
        self.content
    }
}

impl ZhangTarget<String> for Directive {
    fn to_target(self) -> String {
        match self {
            Directive::Open(open) => open.to_target(),
            Directive::Close(close) => close.to_target(),
            Directive::Commodity(commodity) => commodity.to_target(),
            Directive::Transaction(txn) => txn.to_target(),
            Directive::Balance(balance) => balance.to_target(),
            Directive::Note(note) => note.to_target(),
            Directive::Document(document) => document.to_target(),
            Directive::Price(price) => price.to_target(),
            Directive::Event(event) => event.to_target(),
            Directive::Custom(custom) => custom.to_target(),
            Directive::Option(options) => options.to_target(),
            Directive::Plugin(plugin) => plugin.to_target(),
            Directive::Include(include) => include.to_target(),
            Directive::Comment(comment) => comment.to_target(),
        }
    }
}

impl ZhangTarget<String> for Ledger {
    fn to_target(self) -> String {
        let vec = self.directives.into_iter().map(|it| it.data.to_target()).collect_vec();
        vec.join("\n\n")
    }
}

#[cfg(test)]
mod test {
    use crate::p::parse_zhang;
    use crate::target::ZhangTarget;
    use indoc::indoc;
    use std::option::Option::None;

    fn parse(from: &str) -> String {
        let directive = parse_zhang(from, None).unwrap().into_iter().next().unwrap();
        directive.data.to_target()
    }

    macro_rules! assert_parse {
        ($msg: expr, $content: expr) => {
            assert_eq!($content.trim(), parse($content).trim(), $msg);
        };
    }

    #[test]
    fn open_to_text() {
        assert_parse!(
            "open with single commodity",
            indoc! {r#"
            1970-01-01 open Equity:hello CNY
        "#}
        );
        assert_parse!(
            "open with multiple commodities",
            indoc! {r#"
            1970-01-01 open Equity:hello CNY, USD
        "#}
        );
    }

    #[test]
    fn balance() {
        assert_parse!(
            "balance check",
            indoc! {r#"
            1970-01-01 balance Equity:hello 10 CNY
        "#}
        );

        assert_parse!(
            "balance pad",
            indoc! {r#"
            1970-01-01 balance Assets:hello 10 CNY with pad Income:Salary
        "#}
        );
    }

    #[test]
    fn option() {
        assert_parse!(
            "option directive",
            indoc! {r#"
            option "hello" "value"
        "#}
        );
    }

    #[test]
    fn close() {
        assert_parse!(
            "close directive",
            indoc! {r#"
            1970-01-01 close Equity:hello
        "#}
        );
    }

    #[test]
    fn commodity() {
        assert_parse!(
            "commodity directive",
            indoc! {r#"
            1970-01-01 commodity CNY
        "#}
        );
        assert_parse!(
            "commodity directive with meta",
            indoc! {r#"
            1970-01-01 commodity CNY
              a: "b"
        "#}
        );
    }

    #[test]
    fn transaction() {
        assert_parse!(
            "transaction directive with payee and narration",
            indoc! {r#"
            1970-01-01 * "Payee" "Narration"
              Assets:123 -1 CNY
              Expenses:TestCategory:One 1 CNY
        "#}
        );
        assert_parse!(
            "transaction directive with narration",
            indoc! {r#"
            1970-01-01 * "Narration"
              Assets:123 -1 CNY
              Expenses:TestCategory:One 1 CNY
        "#}
        );
        //
        // assert_parse!(
        //     "transaction directive with price",
        //     indoc! {r#"
        //     1970-01-01 * "Narration"
        //       Assets:123 -1 CNY { 0.1 USD, 2111-11-11 }
        //       Expenses:TestCategory:One 1 CNY { 0.1 USD }
        // "#}
        // );

        assert_parse!(
            "transaction directive with multiple postings",
            indoc! {r#"
            1970-01-01 * "Payee" "Narration"
              Assets:123 -1 CNY
              Expenses:TestCategory:One 0.5 CNY
              Expenses:TestCategory:Two 0.5 CNY
        "#}
        );

        assert_parse!(
            "transaction directive with postings without cost",
            indoc! {r#"
            1970-01-01 * "Payee" "Narration"
              Assets:123 -1 CNY
              Expenses:TestCategory:One
        "#}
        );

        // assert_parse!(
        //     "transaction directive with price",
        //     indoc! {r#"
        //     1970-01-01 * "Payee" "Narration"
        //       Assets:123 -1 CNY
        //       Expenses:TestCategory:One 1 CCC @ 1 CNY
        // "#}
        // );

        // assert_parse!(
        //     "transaction directive with total price",
        //     indoc! {r#"
        //     1970-01-01 * "Payee" "Narration"
        //       Assets:123 -1 CNY
        //       Expenses:TestCategory:One 1 CCC @@ 1 CNY
        // "#}
        // );

        // assert_parse!(
        //     "transaction directive with tags",
        //     indoc! {r#"
        //     1970-01-01 *  "Narration" #mytag #tag2
        //       Assets:123 -1 CNY
        //       Expenses:TestCategory:One 1 CCC @@ 1 CNY
        // "#}
        // );

        // assert_parse!(
        //     "transaction directive with tags",
        //     indoc! {r#"
        //     1970-01-01 * "Payee" "Narration" ^link1 ^link-2
        //       Assets:123 -1 CNY
        //       Expenses:TestCategory:One 1 CCC @@ 1 CNY
        // "#}
        // );
    }

    #[test]
    fn note() {
        assert_parse!(
            "note directive",
            indoc! {r#"
            1970-01-01 note Assets:123 "你 好 啊"
        "#}
        );
    }

    #[test]
    fn document() {
        assert_parse!(
            "document directive",
            indoc! {r#"
            1970-01-01 document Assets:123 "abc.jpg"
        "#}
        );
    }

    #[test]
    fn price() {
        assert_parse!(
            "price directive ",
            indoc! {r#"
            1970-01-01 price USD 7 CNY
        "#}
        );
    }

    #[test]
    fn event() {
        assert_parse!(
            "event directive ",
            indoc! {r#"
            1970-01-01 event "location" "China"
        "#}
        );
    }

    #[test]
    fn custom() {
        assert_parse!(
            "custom directive ",
            indoc! {r#"
            1970-01-01 custom "budget" Expenses:Eat "monthly" "CNY"
        "#}
        );
    }

    #[test]
    fn plugin() {
        assert_parse!(
            "plugin with config",
            indoc! {r#"
            plugin "module name" "config data"
        "#}
        );
        assert_parse!(
            "plugin without config",
            indoc! {r#"
            plugin "module name"
        "#}
        );
    }

    #[test]
    fn include() {
        assert_parse!(
            "include directive ",
            indoc! {r#"
            include "file path"
        "#}
        );
    }

    #[test]
    fn comment() {
        assert_parse!(
            "comment directive ",
            indoc! {r#"
            ;你好啊
        "#}
        );
    }
}
