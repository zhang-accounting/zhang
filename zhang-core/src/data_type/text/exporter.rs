use itertools::Itertools;
use zhang_ast::amount::Amount;
use zhang_ast::*;

use crate::ledger::Ledger;
use crate::utils::string_::escape_with_quote;

pub trait ZhangDataTypeExportable {
    type Output;
    fn export(self) -> Self::Output;
}

pub fn append_meta(meta: Meta, string: String) -> String {
    let mut metas = meta.export().into_iter().map(|it| format!("  {}", it)).collect_vec();
    metas.insert(0, string);
    metas.join("\n")
}

impl ZhangDataTypeExportable for Date {
    type Output = String;
    fn export(self) -> String {
        match self {
            Date::Date(date) => date.format("%Y-%m-%d").to_string(),
            Date::Datetime(datetime) => datetime.format("%Y-%m-%d %H:%M:%S").to_string(),
            Date::DateHour(datehour) => datehour.format("%Y-%m-%d %H:%M").to_string(),
        }
    }
}

impl ZhangDataTypeExportable for Flag {
    type Output = String;
    fn export(self) -> String {
        self.to_string()
    }
}

impl ZhangDataTypeExportable for Account {
    type Output = String;
    fn export(self) -> String {
        self.content
    }
}
impl ZhangDataTypeExportable for Amount {
    type Output = String;
    fn export(self) -> String {
        format!("{} {}", self.number, self.currency)
    }
}

impl ZhangDataTypeExportable for Meta {
    type Output = Vec<String>;
    fn export(self) -> Vec<String> {
        self.get_flatten()
            .into_iter()
            .sorted_by(|entry_a, entry_b| entry_a.0.cmp(&entry_b.0))
            .map(|(k, v)| format!("{}: {}", k, v.export()))
            .collect_vec()
    }
}

impl ZhangDataTypeExportable for ZhangString {
    type Output = String;
    fn export(self) -> String {
        match self {
            ZhangString::UnquoteString(unquote) => unquote,
            ZhangString::QuoteString(quote) => escape_with_quote(&quote).to_string(),
        }
    }
}

impl ZhangDataTypeExportable for StringOrAccount {
    type Output = String;
    fn export(self) -> String {
        match self {
            StringOrAccount::String(s) => s.export(),
            StringOrAccount::Account(account) => account.export(),
        }
    }
}

impl ZhangDataTypeExportable for Transaction {
    type Output = String;
    fn export(self) -> String {
        let mut header = vec![
            Some(self.date.export()),
            self.flag.map(|it| it.export()),
            self.payee.map(|it| it.export()),
            self.narration.map(|it| it.export()),
        ];
        let mut tags = self.tags.into_iter().map(|it| Some(format!("#{}", it))).collect_vec();
        let mut links = self.links.into_iter().map(|it| Some(format!("^{}", it))).collect_vec();
        header.append(&mut tags);
        header.append(&mut links);

        let mut transaction = self
            .postings
            .into_iter()
            .map(|posting| posting.export())
            .map(|it| format!("  {}", it))
            .collect_vec();
        transaction.insert(0, header.into_iter().flatten().join(" "));
        let mut txn_meta = self.meta.export().into_iter().map(|it| format!("  {}", it)).collect_vec();
        transaction.append(&mut txn_meta);

        transaction.into_iter().join("\n")
    }
}

impl ZhangDataTypeExportable for Posting {
    type Output = String;
    fn export(self) -> String {
        // todo cost and price
        let cost_string = self.cost.map(|it| it.export());
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

impl ZhangDataTypeExportable for PostingCost {
    type Output = String;

    fn export(self) -> Self::Output {
        let mut string_builder = vec!["{".to_string()];
        if let Some(cost_base) = self.base {
            string_builder.push(cost_base.export());
        };
        if let Some(date) = self.date {
            string_builder.push(",".to_string());
            string_builder.push(date.export());
        };
        string_builder.push("}".to_string());
        string_builder.join(" ")
    }
}

impl ZhangDataTypeExportable for SingleTotalPrice {
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

impl ZhangDataTypeExportable for Open {
    type Output = String;
    fn export(self) -> String {
        let mut line = vec![self.date.export(), "open".to_string(), self.account.export()];
        if !self.commodities.is_empty() {
            let commodities = self.commodities.iter().join(", ");
            line.push(commodities);
        }

        append_meta(self.meta, line.join(" "))
    }
}

impl ZhangDataTypeExportable for Close {
    type Output = String;
    fn export(self) -> String {
        let line = [self.date.export(), "close".to_string(), self.account.export()];
        append_meta(self.meta, line.join(" "))
    }
}

impl ZhangDataTypeExportable for Commodity {
    type Output = String;
    fn export(self) -> String {
        let line = [self.date.export(), "commodity".to_string(), self.currency];
        append_meta(self.meta, line.join(" "))
    }
}

impl ZhangDataTypeExportable for BalancePad {
    type Output = String;
    fn export(self) -> String {
        let line = [
            self.date.export(),
            "balance".to_string(),
            self.account.export(),
            self.amount.export(),
            "with pad".to_string(),
            self.pad.export(),
        ];
        append_meta(self.meta, line.join(" "))
    }
}
impl ZhangDataTypeExportable for BalanceCheck {
    type Output = String;
    fn export(self) -> String {
        let line = [self.date.export(), "balance".to_string(), self.account.export(), self.amount.export()];
        append_meta(self.meta, line.join(" "))
    }
}

impl ZhangDataTypeExportable for Note {
    type Output = String;
    fn export(self) -> String {
        let line = [self.date.export(), "note".to_string(), self.account.export(), self.comment.export()];
        append_meta(self.meta, line.join(" "))
    }
}

impl ZhangDataTypeExportable for Document {
    type Output = String;
    fn export(self) -> String {
        let line = [self.date.export(), "document".to_string(), self.account.export(), self.filename.export()];
        append_meta(self.meta, line.join(" "))
    }
}

impl ZhangDataTypeExportable for Price {
    type Output = String;
    fn export(self) -> String {
        let line = [self.date.export(), "price".to_string(), self.currency, self.amount.export()];
        append_meta(self.meta, line.join(" "))
    }
}

impl ZhangDataTypeExportable for Event {
    type Output = String;
    fn export(self) -> String {
        let line = [self.date.export(), "event".to_string(), self.event_type.export(), self.description.export()];
        append_meta(self.meta, line.join(" "))
    }
}

impl ZhangDataTypeExportable for Custom {
    type Output = String;
    fn export(self) -> String {
        let mut line = vec![self.date.export(), "custom".to_string(), self.custom_type.export()];
        let mut values = self.values.into_iter().map(|it| it.export()).collect_vec();
        line.append(&mut values);
        append_meta(self.meta, line.join(" "))
    }
}

impl ZhangDataTypeExportable for Options {
    type Output = String;
    fn export(self) -> String {
        let line = ["option".to_string(), self.key.export(), self.value.export()];
        line.join(" ")
    }
}
impl ZhangDataTypeExportable for Plugin {
    type Output = String;
    fn export(self) -> String {
        let mut line = vec!["plugin".to_string(), self.module.export()];
        let mut values = self.value.into_iter().map(|it| it.export()).collect_vec();
        line.append(&mut values);

        append_meta(self.meta, line.join(" "))
    }
}
impl ZhangDataTypeExportable for Include {
    type Output = String;
    fn export(self) -> String {
        let line = ["include".to_string(), self.file.export()];
        line.join(" ")
    }
}

impl ZhangDataTypeExportable for Comment {
    type Output = String;
    fn export(self) -> String {
        self.content
    }
}

impl ZhangDataTypeExportable for Budget {
    type Output = String;

    fn export(self) -> Self::Output {
        let line = [self.date.export(), "budget".to_owned(), self.name, self.commodity];
        append_meta(self.meta, line.join(" "))
    }
}
impl ZhangDataTypeExportable for BudgetClose {
    type Output = String;

    fn export(self) -> Self::Output {
        let line = [self.date.export(), "budget-close".to_owned(), self.name];
        append_meta(self.meta, line.join(" "))
    }
}

impl ZhangDataTypeExportable for BudgetAdd {
    type Output = String;

    fn export(self) -> Self::Output {
        let line = [self.date.export(), "budget-add".to_owned(), self.name, self.amount.export()];
        append_meta(self.meta, line.join(" "))
    }
}

impl ZhangDataTypeExportable for BudgetTransfer {
    type Output = String;

    fn export(self) -> Self::Output {
        let line = [self.date.export(), "budget-transfer".to_owned(), self.from, self.to, self.amount.export()];
        append_meta(self.meta, line.join(" "))
    }
}

impl ZhangDataTypeExportable for Directive {
    type Output = String;
    fn export(self) -> String {
        match self {
            Directive::Open(open) => open.export(),
            Directive::Close(close) => close.export(),
            Directive::Commodity(commodity) => commodity.export(),
            Directive::Transaction(txn) => txn.export(),
            Directive::BalancePad(pad) => pad.export(),
            Directive::BalanceCheck(check) => check.export(),
            Directive::Note(note) => note.export(),
            Directive::Document(document) => document.export(),
            Directive::Price(price) => price.export(),
            Directive::Event(event) => event.export(),
            Directive::Custom(custom) => custom.export(),
            Directive::Option(options) => options.export(),
            Directive::Plugin(plugin) => plugin.export(),
            Directive::Include(include) => include.export(),
            Directive::Comment(comment) => comment.export(),
            Directive::Budget(budget) => budget.export(),
            Directive::BudgetAdd(budget_add) => budget_add.export(),
            Directive::BudgetTransfer(budget_transfer) => budget_transfer.export(),
            Directive::BudgetClose(budget_close) => budget_close.export(),
        }
    }
}

impl ZhangDataTypeExportable for Ledger {
    type Output = String;
    fn export(self) -> String {
        let vec = self.directives.into_iter().map(|it| it.data.export()).collect_vec();
        vec.join("\n\n")
    }
}

#[cfg(test)]
mod test {

    use indoc::indoc;

    use crate::data_type::text::ZhangDataType;
    use crate::data_type::DataType;

    fn parse_and_export(from: &str) -> String {
        let data_type = ZhangDataType {};
        let directive = data_type.transform(from.to_owned(), None).unwrap().into_iter().next().unwrap();
        data_type.export(directive)
    }

    macro_rules! assert_parse {
        ($msg: expr, $content: expr) => {
            assert_eq!($content.trim(), parse_and_export($content.trim()), $msg);
        };
    }

    #[test]
    fn open_to_text() {
        assert_parse!(
            "open with single commodity",
            indoc! {r#"
            1970-01-01 open Equity:hello
        "#}
        );
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
              Assets:123 -1 CNY { 0.1 USD , 2111-11-11 }
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

        assert_parse!(
            "transaction directive with meta",
            indoc! {r#"
            1970-01-01 * "Payee" "Narration" ^link1 ^link-2
              Assets:123 -1 CNY
              time: "123"
        "#}
        );

        assert_parse!(
            "transaction posting and meta",
            indoc! {r#"
            1970-01-01 * "Payee" "Narration" ^link1 ^link-2
              Assets:123 -1 CNY
              Expenses:TestCategory:One 1 CCC @@ 1 CNY
              a: b
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

        assert_parse!(
            "plugin with meta",
            indoc! {r#"
            plugin "module name"
              a: "b"
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
    fn budget() {
        assert_parse!(
            "budget directive",
            indoc! {r#"
                1970-01-01 budget Diet CNY
            "#}
        );

        assert_parse!(
            "budget-add directive",
            indoc! {r#"
                1970-01-01 budget-add Diet 1 CNY
            "#}
        );
        assert_parse!(
            "budget-transfer directive",
            indoc! {r#"
                1970-01-01 budget-transfer Diet Saving 1 CNY
            "#}
        );
        assert_parse!(
            "budget-close directive",
            indoc! {r#"
                1970-01-01 budget-close Diet
            "#}
        );
    }
}
