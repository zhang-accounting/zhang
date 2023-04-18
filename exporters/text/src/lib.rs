use itertools::Itertools;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;

use zhang_ast::amount::Amount;
use zhang_ast::*;
use zhang_core::exporter::{AppendableExporter, Exporter};
use zhang_core::ledger::Ledger;
use zhang_core::utils::has_path_visited;
use zhang_core::utils::string_::escape_with_quote;
use zhang_core::ZhangResult;

pub(crate) fn create_folder_if_not_exist(filename: &std::path::Path) {
    std::fs::create_dir_all(filename.parent().unwrap()).expect("cannot create folder recursive");
}

pub struct TextExporter {}

impl AppendableExporter for TextExporter {
    fn append_directives(&self, ledger: &Ledger, file: PathBuf, directives: Vec<Directive>) -> ZhangResult<()> {
        let (entry, main_file_endpoint) = &ledger.entry;
        let endpoint = entry.join(file);

        create_folder_if_not_exist(&endpoint);

        if !has_path_visited(&ledger.visited_files, &endpoint) {
            let path = match endpoint.strip_prefix(entry) {
                Ok(relative_path) => relative_path.to_str().unwrap(),
                Err(_) => endpoint.to_str().unwrap(),
            };
            self.append_directives(
                ledger,
                entry.join(main_file_endpoint),
                vec![Directive::Include(Include {
                    file: ZhangString::QuoteString(path.to_string()),
                })],
            )?;
        }
        let directive_content = format!("\n{}\n", directives.into_iter().map(|it| it.export()).join("\n"));
        let mut ledger_base_file = OpenOptions::new().append(true).create(true).open(&endpoint).unwrap();
        Ok(ledger_base_file.write_all(directive_content.as_bytes())?)
    }
}

impl Exporter for TextExporter {
    type Output = String;

    fn export_directive(&self, directive: Directive) -> Self::Output {
        directive.export()
    }
}

pub trait TextExportable {
    type Output;
    fn export(self) -> Self::Output;
}

fn append_meta(meta: Meta, string: String) -> String {
    let mut metas = meta.export().into_iter().map(|it| format!("  {}", it)).collect_vec();
    metas.insert(0, string);
    metas.join("\n")
}

impl TextExportable for Date {
    type Output = String;
    fn export(self) -> String {
        match self {
            Date::Date(date) => date.format("%Y-%m-%d").to_string(),
            Date::Datetime(datetime) => datetime.format("%Y-%m-%d %H:%M:%S").to_string(),
            Date::DateHour(datehour) => datehour.format("%Y-%m-%d %H:%M").to_string(),
        }
    }
}

impl TextExportable for Flag {
    type Output = String;
    fn export(self) -> String {
        self.to_string()
    }
}

impl TextExportable for Account {
    type Output = String;
    fn export(self) -> String {
        self.content
    }
}
impl TextExportable for Amount {
    type Output = String;
    fn export(self) -> String {
        format!("{} {}", self.number, self.currency)
    }
}

impl TextExportable for Meta {
    type Output = Vec<String>;
    fn export(self) -> Vec<String> {
        self.get_flatten()
            .into_iter()
            .sorted_by(|entry_a, entry_b| entry_a.0.cmp(&entry_b.0))
            .map(|(k, v)| format!("{}: {}", k, v.export()))
            .collect_vec()
    }
}

impl TextExportable for ZhangString {
    type Output = String;
    fn export(self) -> String {
        match self {
            ZhangString::UnquoteString(unquote) => unquote,
            ZhangString::QuoteString(quote) => escape_with_quote(&quote).to_string(),
        }
    }
}

impl TextExportable for StringOrAccount {
    type Output = String;
    fn export(self) -> String {
        match self {
            StringOrAccount::String(s) => s.export(),
            StringOrAccount::Account(account) => account.export(),
        }
    }
}

impl TextExportable for Transaction {
    type Output = String;
    fn export(self) -> String {
        let mut vec1 = vec![
            Some(self.date.export()),
            self.flag.map(|it| it.export()),
            self.payee.map(|it| it.export()),
            self.narration.map(|it| it.export()),
        ];
        let mut tags = self.tags.into_iter().map(|it| Some(format!("#{}", it))).collect_vec();
        let mut links = self.links.into_iter().map(|it| Some(format!("^{}", it))).collect_vec();
        vec1.append(&mut tags);
        vec1.append(&mut links);

        let mut transaction = self
            .postings
            .into_iter()
            .map(|it| format!("  {}", it.export()))
            .collect_vec();
        transaction.insert(0, vec1.into_iter().flatten().join(" "));
        let mut vec2 = self
            .meta
            .export()
            .into_iter()
            .map(|it| format!("  {}", it))
            .collect_vec();
        transaction.append(&mut vec2);

        transaction.into_iter().join("\n")
    }
}

impl TextExportable for Posting {
    type Output = String;
    fn export(self) -> String {
        // todo cost and price
        let cost_string = if self.cost.is_some() || self.cost_date.is_some() {
            let vec2 = vec![self.cost.map(|it| it.export()), self.cost_date.map(|it| it.export())];
            Some(format!("{{ {} }}", vec2.into_iter().flatten().join(", ")))
        } else {
            None
        };
        let vec1 = vec![
            self.flag.map(|it| format!(" {}", it.export())),
            Some(self.account.export()),
            self.units.map(|it| it.export()),
            cost_string,
            self.price.map(|it| it.export()),
        ];

        vec1.into_iter().flatten().join(" ")
    }
}

impl TextExportable for SingleTotalPrice {
    type Output = String;
    fn export(self) -> String {
        match self {
            SingleTotalPrice::Single(single_price) => {
                format!("@ {}", single_price.export())
            }
            SingleTotalPrice::Total(total_price) => {
                format!("@@ {}", total_price.export())
            }
        }
    }
}

impl TextExportable for Open {
    type Output = String;
    fn export(self) -> String {
        let mut line = vec![self.date.export(), "open".to_string(), self.account.export()];
        let commodities = self.commodities.iter().join(", ");
        line.push(commodities);
        append_meta(self.meta, line.join(" "))
    }
}

impl TextExportable for Close {
    type Output = String;
    fn export(self) -> String {
        let line = vec![self.date.export(), "close".to_string(), self.account.export()];
        append_meta(self.meta, line.join(" "))
    }
}

impl TextExportable for Commodity {
    type Output = String;
    fn export(self) -> String {
        let line = vec![self.date.export(), "commodity".to_string(), self.currency];
        append_meta(self.meta, line.join(" "))
    }
}

impl TextExportable for Balance {
    type Output = String;
    fn export(self) -> String {
        match self {
            Balance::BalanceCheck(check) => {
                let line = vec![
                    check.date.export(),
                    "balance".to_string(),
                    check.account.export(),
                    check.amount.export(),
                ];
                append_meta(check.meta, line.join(" "))
            }
            Balance::BalancePad(pad) => {
                let line = vec![
                    pad.date.export(),
                    "balance".to_string(),
                    pad.account.export(),
                    pad.amount.export(),
                    "with pad".to_string(),
                    pad.pad.export(),
                ];
                append_meta(pad.meta, line.join(" "))
            }
        }
    }
}

impl TextExportable for Note {
    type Output = String;
    fn export(self) -> String {
        let line = vec![
            self.date.export(),
            "note".to_string(),
            self.account.export(),
            self.comment.export(),
        ];
        append_meta(self.meta, line.join(" "))
    }
}

impl TextExportable for Document {
    type Output = String;
    fn export(self) -> String {
        let line = vec![
            self.date.export(),
            "document".to_string(),
            self.account.export(),
            self.filename.export(),
        ];
        append_meta(self.meta, line.join(" "))
    }
}

impl TextExportable for Price {
    type Output = String;
    fn export(self) -> String {
        let line = vec![
            self.date.export(),
            "price".to_string(),
            self.currency,
            self.amount.export(),
        ];
        append_meta(self.meta, line.join(" "))
    }
}

impl TextExportable for Event {
    type Output = String;
    fn export(self) -> String {
        let line = vec![
            self.date.export(),
            "event".to_string(),
            self.event_type.export(),
            self.description.export(),
        ];
        append_meta(self.meta, line.join(" "))
    }
}

impl TextExportable for Custom {
    type Output = String;
    fn export(self) -> String {
        let mut line = vec![self.date.export(), "custom".to_string(), self.custom_type.export()];
        let mut values = self.values.into_iter().map(|it| it.export()).collect_vec();
        line.append(&mut values);
        append_meta(self.meta, line.join(" "))
    }
}

impl TextExportable for Options {
    type Output = String;
    fn export(self) -> String {
        let line = vec!["option".to_string(), self.key.export(), self.value.export()];
        line.join(" ")
    }
}
impl TextExportable for Plugin {
    type Output = String;
    fn export(self) -> String {
        let mut line = vec!["plugin".to_string(), self.module.export()];
        let mut values = self.value.into_iter().map(|it| it.export()).collect_vec();
        line.append(&mut values);
        line.join(" ")
    }
}
impl TextExportable for Include {
    type Output = String;
    fn export(self) -> String {
        let line = vec!["include".to_string(), self.file.export()];
        line.join(" ")
    }
}

impl TextExportable for Comment {
    type Output = String;
    fn export(self) -> String {
        self.content
    }
}

impl TextExportable for Directive {
    type Output = String;
    fn export(self) -> String {
        match self {
            Directive::Open(open) => open.export(),
            Directive::Close(close) => close.export(),
            Directive::Commodity(commodity) => commodity.export(),
            Directive::Transaction(txn) => txn.export(),
            Directive::Balance(balance) => balance.export(),
            Directive::Note(note) => note.export(),
            Directive::Document(document) => document.export(),
            Directive::Price(price) => price.export(),
            Directive::Event(event) => event.export(),
            Directive::Custom(custom) => custom.export(),
            Directive::Option(options) => options.export(),
            Directive::Plugin(plugin) => plugin.export(),
            Directive::Include(include) => include.export(),
            Directive::Comment(comment) => comment.export(),
        }
    }
}

impl TextExportable for Ledger {
    type Output = String;
    fn export(self) -> String {
        let vec = self.directives.into_iter().map(|it| it.data.export()).collect_vec();
        vec.join("\n\n")
    }
}

#[cfg(test)]
mod test {
    use std::option::Option::None;

    use indoc::indoc;
    use text_transformer::parse_zhang;

    use crate::TextExportable;

    fn parse(from: &str) -> String {
        let directive = parse_zhang(from, None).unwrap().into_iter().next().unwrap();
        directive.data.export()
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

        assert_parse!(
            "transaction directive with price",
            indoc! {r#"
            1970-01-01 * "Narration"
              Assets:123 -1 CNY { 0.1 USD, 2111-11-11 }
              Expenses:TestCategory:One 1 CNY { 0.1 USD }
        "#}
        );

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

        assert_parse!(
            "transaction directive with price",
            indoc! {r#"
            1970-01-01 * "Payee" "Narration"
              Assets:123 -1 CNY
              Expenses:TestCategory:One 1 CCC @ 1 CNY
        "#}
        );

        assert_parse!(
            "transaction directive with total price",
            indoc! {r#"
            1970-01-01 * "Payee" "Narration"
              Assets:123 -1 CNY
              Expenses:TestCategory:One 1 CCC @@ 1 CNY
        "#}
        );

        assert_parse!(
            "transaction directive with tags",
            indoc! {r#"
            1970-01-01 * "Narration" #mytag #tag2
              Assets:123 -1 CNY
              Expenses:TestCategory:One 1 CCC @@ 1 CNY
        "#}
        );

        assert_parse!(
            "transaction directive with tags",
            indoc! {r#"
            1970-01-01 * "Payee" "Narration" ^link1 ^link-2
              Assets:123 -1 CNY
              Expenses:TestCategory:One 1 CCC @@ 1 CNY
        "#}
        );
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
}
