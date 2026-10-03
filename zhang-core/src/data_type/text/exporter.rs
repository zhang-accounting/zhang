use itertools::Itertools;
use zhang_ast::amount::Amount;
use zhang_ast::*;

use crate::data_type::text::parser::is_valid_meta_key;
use crate::ledger::Ledger;
use crate::utils::string_::{quote_as, QuoteStyle};

pub trait ZhangDataTypeExportable: Sized {
    type Output;

    /// Export as zhang text, writing quoted strings in [`QuoteStyle::Zhang`].
    fn export(self) -> Self::Output {
        self.export_as(QuoteStyle::Zhang)
    }

    /// Export as zhang text, writing quoted strings in `style`. The beancount data
    /// type reuses this exporter with [`QuoteStyle::Beancount`].
    fn export_as(self, style: QuoteStyle) -> Self::Output;
}

pub fn append_meta(meta: Meta, string: String) -> String {
    append_meta_as(meta, string, QuoteStyle::Zhang)
}

/// [`append_meta`], writing quoted metadata values in `style`.
pub fn append_meta_as(meta: Meta, string: String, style: QuoteStyle) -> String {
    let mut metas = meta.export_as(style).into_iter().map(|it| format!("  {}", it)).collect_vec();
    metas.insert(0, string);
    metas.join("\n")
}

impl ZhangDataTypeExportable for Date {
    type Output = String;
    fn export_as(self, _style: QuoteStyle) -> String {
        match self {
            Date::Date(date) => date.format("%Y-%m-%d").to_string(),
            Date::Datetime(datetime) => datetime.format("%Y-%m-%d %H:%M:%S").to_string(),
            Date::DateHour(datehour) => datehour.format("%Y-%m-%d %H:%M").to_string(),
        }
    }
}

impl ZhangDataTypeExportable for Flag {
    type Output = String;
    fn export_as(self, _style: QuoteStyle) -> String {
        self.to_string()
    }
}

impl ZhangDataTypeExportable for Account {
    type Output = String;
    fn export_as(self, _style: QuoteStyle) -> String {
        self.content
    }
}
impl ZhangDataTypeExportable for Amount {
    type Output = String;
    fn export_as(self, _style: QuoteStyle) -> String {
        format!("{} {}", self.number, self.commodity)
    }
}

impl ZhangDataTypeExportable for Meta {
    type Output = Vec<String>;
    fn export_as(self, style: QuoteStyle) -> Vec<String> {
        self.get_flatten()
            .into_iter()
            .sorted_by(|entry_a, entry_b| entry_a.0.cmp(&entry_b.0))
            .map(|(k, v)| format!("{}: {}", meta_key(k, style), v.export_as(style)))
            .collect_vec()
    }
}

/// A metadata key as written: bare when it reads back that way, quoted otherwise,
/// such as a key read from `"my key": "v"` or `";path": "v"`.
///
/// Python beancount has no quoted keys (beancount 3.2.3 reports a syntax error and
/// drops the directive), so no form of such a key is valid beancount. The beancount
/// style quotes it anyway: zhang's beancount parser reads it back exactly, and
/// beancount reports the line instead of silently reading a different key or a
/// comment.
fn meta_key(key: String, style: QuoteStyle) -> String {
    if is_valid_meta_key(&key) {
        key
    } else {
        quote_as(&key, style)
    }
}

impl ZhangDataTypeExportable for ZhangString {
    type Output = String;
    fn export_as(self, style: QuoteStyle) -> String {
        match self {
            ZhangString::UnquoteString(unquote) => unquote,
            ZhangString::QuoteString(quote) => quote_as(&quote, style),
        }
    }
}

impl ZhangDataTypeExportable for StringOrAccount {
    type Output = String;
    fn export_as(self, style: QuoteStyle) -> String {
        match self {
            StringOrAccount::String(s) => s.export_as(style),
            StringOrAccount::Account(account) => account.export_as(style),
        }
    }
}

impl ZhangDataTypeExportable for Transaction {
    type Output = String;
    fn export_as(self, style: QuoteStyle) -> String {
        // after a flag a single string is the narration, so a payee without a
        // narration is written with an empty narration to stay the payee
        let narration = match (&self.flag, &self.payee, self.narration) {
            (Some(_), Some(_), None) => Some(ZhangString::QuoteString(String::new())),
            (_, _, narration) => narration,
        };
        let mut header = vec![
            Some(self.date.export_as(style)),
            self.flag.map(|it| it.export_as(style)),
            self.payee.map(|it| it.export_as(style)),
            narration.map(|it| it.export_as(style)),
        ];
        let mut tags = self.tags.into_iter().map(|it| Some(format!("#{}", it))).collect_vec();
        let mut links = self.links.into_iter().map(|it| Some(format!("^{}", it))).collect_vec();
        header.append(&mut tags);
        header.append(&mut links);

        // the transaction's metadata goes before the postings: beancount attaches a
        // metadata line that follows a posting to that posting. A posting's own metadata
        // follows it two levels deeper, which beancount and zhang (where it must be
        // indented deeper than the posting) both read as the posting's
        let meta = self.meta.export_as(style).into_iter().map(|it| format!("  {}", it));
        let postings = self.postings.into_iter().flat_map(|mut posting| {
            let meta = std::mem::take(&mut posting.meta).export_as(style).into_iter().map(|it| format!("    {}", it));
            std::iter::once(format!("  {}", posting.export_as(style))).chain(meta)
        });
        std::iter::once(header.into_iter().flatten().join(" ")).chain(meta).chain(postings).join("\n")
    }
}

/// The posting line alone: its metadata is written by the transaction, under it.
impl ZhangDataTypeExportable for Posting {
    type Output = String;
    fn export_as(self, style: QuoteStyle) -> String {
        // todo cost and price
        let cost_string = self.cost.map(|it| it.export_as(style));
        let vec1 = vec![
            self.flag.map(|it| format!(" {}", it.export_as(style))),
            Some(self.account.export_as(style)),
            self.units.map(|it| it.export_as(style)),
            cost_string,
            self.price.map(|it| it.export_as(style)),
        ];
        vec1.into_iter().flatten().join(" ")
    }
}

impl ZhangDataTypeExportable for PostingCost {
    type Output = String;

    fn export_as(self, style: QuoteStyle) -> Self::Output {
        let (open, close) = if self.total { ("{{", "}}") } else { ("{", "}") };
        let mut string_builder = vec![open.to_string()];
        if let Some(cost_base) = self.base {
            string_builder.push(cost_base.export_as(style));
        };
        if let Some(date) = self.date {
            string_builder.push(",".to_string());
            string_builder.push(date.export_as(style));
        };
        if let Some(label) = self.label {
            string_builder.push(",".to_string());
            string_builder.push(quote_as(&label, style));
        };
        string_builder.push(close.to_string());
        string_builder.join(" ")
    }
}

impl ZhangDataTypeExportable for SingleTotalPrice {
    type Output = String;
    fn export_as(self, style: QuoteStyle) -> String {
        match self {
            SingleTotalPrice::Single(single_price) => {
                format!("@ {}", single_price.export_as(style))
            }
            SingleTotalPrice::Total(total_price) => {
                format!("@@ {}", total_price.export_as(style))
            }
        }
    }
}

impl ZhangDataTypeExportable for Open {
    type Output = String;
    fn export_as(self, style: QuoteStyle) -> String {
        let mut line = vec![self.date.export_as(style), "open".to_string(), self.account.export_as(style)];
        if !self.commodities.is_empty() {
            let commodities = self.commodities.iter().join(", ");
            line.push(commodities);
        }

        append_meta_as(self.meta, line.join(" "), style)
    }
}

impl ZhangDataTypeExportable for Close {
    type Output = String;
    fn export_as(self, style: QuoteStyle) -> String {
        let line = [self.date.export_as(style), "close".to_string(), self.account.export_as(style)];
        append_meta_as(self.meta, line.join(" "), style)
    }
}

impl ZhangDataTypeExportable for Commodity {
    type Output = String;
    fn export_as(self, style: QuoteStyle) -> String {
        let line = [self.date.export_as(style), "commodity".to_string(), self.currency];
        append_meta_as(self.meta, line.join(" "), style)
    }
}

impl ZhangDataTypeExportable for BalancePad {
    type Output = String;
    fn export_as(self, style: QuoteStyle) -> String {
        let line = [
            self.date.export_as(style),
            "balance".to_string(),
            self.account.export_as(style),
            self.amount.export_as(style),
            "with pad".to_string(),
            self.pad.export_as(style),
        ];
        append_meta_as(self.meta, line.join(" "), style)
    }
}
impl ZhangDataTypeExportable for BalanceCheck {
    type Output = String;
    fn export_as(self, style: QuoteStyle) -> String {
        let BalanceCheck {
            date,
            account,
            amount,
            tolerance,
            meta,
            ..
        } = self;
        let amount_str = match tolerance {
            Some(tolerance) => format!("{} ~ {} {}", amount.number, tolerance, amount.commodity),
            None => amount.export_as(style),
        };
        let line = [date.export_as(style), "balance".to_string(), account.export_as(style), amount_str];
        append_meta_as(meta, line.join(" "), style)
    }
}
impl ZhangDataTypeExportable for Pad {
    type Output = String;
    fn export_as(self, style: QuoteStyle) -> String {
        let line = [
            self.date.export_as(style),
            "pad".to_string(),
            self.account.export_as(style),
            self.pad.export_as(style),
        ];
        append_meta_as(self.meta, line.join(" "), style)
    }
}

/// The `#tag` and `^link` words of a note or document, each sorted by name (the AST
/// keeps them in sets).
fn tags_and_links(tags: Option<std::collections::HashSet<String>>, links: Option<std::collections::HashSet<String>>) -> Vec<String> {
    let words =
        |items: Option<std::collections::HashSet<String>>, prefix: char| items.into_iter().flatten().sorted().map(move |it| format!("{}{}", prefix, it));
    words(tags, '#').chain(words(links, '^')).collect()
}

impl ZhangDataTypeExportable for Note {
    type Output = String;
    fn export_as(self, style: QuoteStyle) -> String {
        let line = [
            self.date.export_as(style),
            "note".to_string(),
            self.account.export_as(style),
            self.comment.export_as(style),
        ];
        let line = line.into_iter().chain(tags_and_links(self.tags, self.links)).join(" ");
        append_meta_as(self.meta, line, style)
    }
}

impl ZhangDataTypeExportable for Document {
    type Output = String;
    fn export_as(self, style: QuoteStyle) -> String {
        let line = [
            self.date.export_as(style),
            "document".to_string(),
            self.account.export_as(style),
            self.filename.export_as(style),
        ];
        let line = line.into_iter().chain(tags_and_links(self.tags, self.links)).join(" ");
        append_meta_as(self.meta, line, style)
    }
}

impl ZhangDataTypeExportable for Price {
    type Output = String;
    fn export_as(self, style: QuoteStyle) -> String {
        let line = [self.date.export_as(style), "price".to_string(), self.currency, self.amount.export_as(style)];
        append_meta_as(self.meta, line.join(" "), style)
    }
}

impl ZhangDataTypeExportable for Event {
    type Output = String;
    fn export_as(self, style: QuoteStyle) -> String {
        let line = [
            self.date.export_as(style),
            "event".to_string(),
            self.event_type.export_as(style),
            self.description.export_as(style),
        ];
        append_meta_as(self.meta, line.join(" "), style)
    }
}

impl ZhangDataTypeExportable for Query {
    type Output = String;
    fn export_as(self, style: QuoteStyle) -> String {
        let line = [
            self.date.export_as(style),
            "query".to_string(),
            self.name.export_as(style),
            self.query_string.export_as(style),
        ];
        append_meta_as(self.meta, line.join(" "), style)
    }
}

impl ZhangDataTypeExportable for Custom {
    type Output = String;
    fn export_as(self, style: QuoteStyle) -> String {
        let mut line = vec![self.date.export_as(style), "custom".to_string(), self.custom_type.export_as(style)];
        let mut values = self.values.into_iter().map(|it| it.export_as(style)).collect_vec();
        line.append(&mut values);
        append_meta_as(self.meta, line.join(" "), style)
    }
}

impl ZhangDataTypeExportable for Options {
    type Output = String;
    fn export_as(self, style: QuoteStyle) -> String {
        let line = ["option".to_string(), self.key.export_as(style), self.value.export_as(style)];
        line.join(" ")
    }
}
impl ZhangDataTypeExportable for Plugin {
    type Output = String;
    fn export_as(self, style: QuoteStyle) -> String {
        let mut line = vec!["plugin".to_string(), self.module.export_as(style)];
        let mut values = self.value.into_iter().map(|it| it.export_as(style)).collect_vec();
        line.append(&mut values);

        append_meta_as(self.meta, line.join(" "), style)
    }
}
impl ZhangDataTypeExportable for Include {
    type Output = String;
    fn export_as(self, style: QuoteStyle) -> String {
        let line = ["include".to_string(), self.file.export_as(style)];
        line.join(" ")
    }
}

impl ZhangDataTypeExportable for Comment {
    type Output = String;
    fn export_as(self, _style: QuoteStyle) -> String {
        self.content
    }
}

impl ZhangDataTypeExportable for Budget {
    type Output = String;

    fn export_as(self, style: QuoteStyle) -> Self::Output {
        let line = [self.date.export_as(style), "budget".to_owned(), self.name, self.commodity];
        append_meta_as(self.meta, line.join(" "), style)
    }
}
impl ZhangDataTypeExportable for BudgetClose {
    type Output = String;

    fn export_as(self, style: QuoteStyle) -> Self::Output {
        let line = [self.date.export_as(style), "budget-close".to_owned(), self.name];
        append_meta_as(self.meta, line.join(" "), style)
    }
}

impl ZhangDataTypeExportable for BudgetAdd {
    type Output = String;

    fn export_as(self, style: QuoteStyle) -> Self::Output {
        let line = [self.date.export_as(style), "budget-add".to_owned(), self.name, self.amount.export_as(style)];
        append_meta_as(self.meta, line.join(" "), style)
    }
}

impl ZhangDataTypeExportable for BudgetTransfer {
    type Output = String;

    fn export_as(self, style: QuoteStyle) -> Self::Output {
        let line = [
            self.date.export_as(style),
            "budget-transfer".to_owned(),
            self.from,
            self.to,
            self.amount.export_as(style),
        ];
        append_meta_as(self.meta, line.join(" "), style)
    }
}

impl ZhangDataTypeExportable for Directive {
    type Output = String;
    fn export_as(self, style: QuoteStyle) -> String {
        match self {
            Directive::Open(open) => open.export_as(style),
            Directive::Close(close) => close.export_as(style),
            Directive::Commodity(commodity) => commodity.export_as(style),
            Directive::Transaction(txn) => txn.export_as(style),
            Directive::BalancePad(pad) => pad.export_as(style),
            Directive::BalanceCheck(check) => check.export_as(style),
            Directive::Pad(pad) => pad.export_as(style),
            Directive::Note(note) => note.export_as(style),
            Directive::Document(document) => document.export_as(style),
            Directive::Price(price) => price.export_as(style),
            Directive::Event(event) => event.export_as(style),
            Directive::Custom(custom) => custom.export_as(style),
            Directive::Query(query) => query.export_as(style),
            Directive::Option(options) => options.export_as(style),
            Directive::Plugin(plugin) => plugin.export_as(style),
            Directive::Include(include) => include.export_as(style),
            Directive::Comment(comment) => comment.export_as(style),
            Directive::Budget(budget) => budget.export_as(style),
            Directive::BudgetAdd(budget_add) => budget_add.export_as(style),
            Directive::BudgetTransfer(budget_transfer) => budget_transfer.export_as(style),
            Directive::BudgetClose(budget_close) => budget_close.export_as(style),
        }
    }
}

impl ZhangDataTypeExportable for Ledger {
    type Output = String;
    fn export_as(self, style: QuoteStyle) -> String {
        let vec = self.directives.into_iter().map(|it| it.data.export_as(style)).collect_vec();
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
        assert_parse!(
            "pad",
            indoc! {r#"
            1970-01-01 pad Assets:hello Equity:Opening-Balances
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
              time: "123"
              Assets:123 -1 CNY
        "#}
        );

        assert_parse!(
            "transaction posting and meta",
            indoc! {r#"
            1970-01-01 * "Payee" "Narration" ^link1 ^link-2
              a: b
              Assets:123 -1 CNY
              Expenses:TestCategory:One 1 CCC @@ 1 CNY
        "#}
        );
    }

    #[test]
    fn a_payee_without_narration_stays_the_payee() {
        use zhang_ast::{Date, Directive, Flag, SpanInfo, Spanned, Transaction, ZhangString};

        let data_type = ZhangDataType {};
        let date = Date::Date(chrono::NaiveDate::from_ymd_opt(2024, 1, 2).unwrap());
        let transaction = |flag: Option<Flag>| Transaction {
            date: date.clone(),
            flag,
            payee: Some(ZhangString::quote("Cafe")),
            narration: None,
            tags: Default::default(),
            links: Default::default(),
            postings: vec![],
            meta: Default::default(),
        };

        // with a flag, a single string would be read as the narration
        let exported = data_type.export(Spanned::new(Directive::Transaction(transaction(Some(Flag::Okay))), SpanInfo::default()));
        assert_eq!(exported, r#"2024-01-02 * "Cafe" """#);
        let Directive::Transaction(reparsed) = data_type.transform(format!("{exported}\n  Assets:Cash"), None).unwrap().remove(0).data else {
            panic!("expected a transaction");
        };
        assert_eq!(reparsed.payee, Some(ZhangString::quote("Cafe")));
        assert_eq!(reparsed.narration, Some(ZhangString::quote("")));

        // without a flag, a single string is the payee, and reads back exactly
        let exported = data_type.export(Spanned::new(Directive::Transaction(transaction(None)), SpanInfo::default()));
        assert_eq!(exported, r#"2024-01-02 "Cafe""#);
        let Directive::Transaction(reparsed) = data_type.transform(format!("{exported}\n  Assets:Cash"), None).unwrap().remove(0).data else {
            panic!("expected a transaction");
        };
        assert_eq!((reparsed.payee, reparsed.narration), (Some(ZhangString::quote("Cafe")), None));
    }

    #[test]
    fn metadata_keys_that_are_not_bare_words_are_quoted() {
        use zhang_ast::{Directive, Meta, SpanInfo, Spanned, ZhangString};

        use crate::utils::string_::test::{random_string, XorShift};

        let data_type = ZhangDataType {};
        let round_trip = |key: &str| {
            let mut meta = Meta::default();
            meta.insert(key.to_owned(), ZhangString::quote("v"));
            let mut directives = data_type
                .transform("2024-01-02 open Assets:Cash\n2024-01-02 * \"p\" \"n\"\n  Assets:Cash\n".to_owned(), None)
                .unwrap();
            for directive in &mut directives {
                *directive.data.meta_mut().unwrap() = meta.clone();
                if let Directive::Transaction(txn) = &mut directive.data {
                    for posting in &mut txn.postings {
                        posting.meta = meta.clone();
                    }
                }
            }
            for directive in directives {
                let exported = data_type.export(Spanned::new(directive.data.clone(), SpanInfo::default()));
                let reparsed = data_type
                    .transform(exported.clone(), None)
                    .unwrap_or_else(|err| panic!("key {key:?}: cannot parse {exported:?}: {err}"));
                assert_eq!(reparsed.len(), 1, "key {key:?}: {exported:?}");
                assert_eq!(reparsed[0].data, directive.data, "key {key:?}: {exported:?}");
            }
            let open = Directive::Open(zhang_ast::Open {
                date: zhang_ast::Date::Date(chrono::NaiveDate::from_ymd_opt(2024, 1, 2).unwrap()),
                account: std::str::FromStr::from_str("Assets:Cash").unwrap(),
                commodities: vec![],
                meta,
            });
            data_type.export(Spanned::new(open, SpanInfo::default()))
        };

        assert_eq!(round_trip("receipt-no"), "2024-01-02 open Assets:Cash\n  receipt-no: \"v\"");
        assert_eq!(round_trip("my key"), "2024-01-02 open Assets:Cash\n  \"my key\": \"v\"");
        assert_eq!(round_trip(";path"), "2024-01-02 open Assets:Cash\n  \";path\": \"v\"");
        for key in [
            "",
            "a:b",
            "#tag",
            "*x",
            "//x",
            "say \"hi\"",
            "back\\slash",
            "tab\tkey",
            "line\nkey",
            "新 键",
            "\u{2028}",
        ] {
            round_trip(key);
        }
        let mut rng = XorShift::new(0x6b65_7973);
        for _ in 0..500 {
            round_trip(&random_string(&mut rng));
        }
    }

    #[test]
    fn transaction_meta_is_written_before_the_postings() {
        // metadata after the postings is still read as transaction metadata, and is
        // written back before them, where beancount reads it as transaction metadata
        // too (checked with beancount 3.2.3: a metadata line after a posting belongs to
        // that posting)
        let source = indoc! {r#"
            1970-01-01 * "Payee" "Narration"
              Assets:123 -1 CNY
              Expenses:Food 1 CNY
              a: "b"
              c: "d"
        "#};
        assert_eq!(
            parse_and_export(source.trim()),
            indoc! {r#"
                1970-01-01 * "Payee" "Narration"
                  a: "b"
                  c: "d"
                  Assets:123 -1 CNY
                  Expenses:Food 1 CNY
            "#}
            .trim()
        );
    }

    #[test]
    fn posting_meta_is_written_under_its_posting_two_levels_deeper() {
        use zhang_ast::{Directive, SpanInfo, Spanned};

        use crate::data_type::text::exporter::ZhangDataTypeExportable;
        use crate::utils::string_::QuoteStyle;

        let source = indoc! {r#"
            1970-01-01 * "Payee" "Narration"
              Assets:123 -1 CNY
                receipt: "r-1"
                "my key": "say \"hi\""
              b: "transaction"
              Expenses:Food 1 CNY
                 category: lunch
              a: "transaction too"
        "#};
        let expected = indoc! {r#"
            1970-01-01 * "Payee" "Narration"
              a: "transaction too"
              b: "transaction"
              Assets:123 -1 CNY
                "my key": "say \"hi\""
                receipt: "r-1"
              Expenses:Food 1 CNY
                category: lunch
        "#}
        .trim();
        assert_eq!(parse_and_export(source.trim()), expected);

        // both styles write the same layout, which reads back to the same directive
        let data_type = ZhangDataType {};
        let directive = data_type.transform(source.to_owned(), None).unwrap().pop().unwrap().data;
        let Directive::Transaction(txn) = &directive else { unreachable!() };
        assert_eq!(txn.postings[0].meta.clone().get_flatten().len(), 2);
        assert_eq!(txn.postings[1].meta.clone().get_flatten().len(), 1);
        assert_eq!(txn.meta.clone().get_flatten().len(), 2);
        assert_eq!(txn.clone().export_as(QuoteStyle::Beancount), expected);
        let reparsed = data_type.transform(expected.to_owned(), None).unwrap().pop().unwrap();
        assert_eq!(reparsed.data, directive);
        assert_eq!(data_type.export(Spanned::new(reparsed.data, SpanInfo::default())), expected);
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
    fn note_and_document_tags_and_links() {
        assert_parse!(
            "note with tags and links",
            indoc! {r#"
            1970-01-01 note Assets:123 "x" #a #b ^l1
        "#}
        );
        assert_parse!(
            "document with links and metadata",
            indoc! {r#"
            1970-01-01 document Assets:123 "abc.jpg" #旅行 ^l1 ^l2
              k: "v"
        "#}
        );
        // tags and links are kept in sets, and written sorted
        assert_eq!(
            parse_and_export(r#"1970-01-01 note Assets:123 "x" ^z #b ^a #a ; comment"#),
            r#"1970-01-01 note Assets:123 "x" #a #b ^a ^z"#
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
    fn query() {
        assert_parse!(
            "query directive",
            indoc! {r#"
            2024-01-01 query "france-balances" "SELECT account, sum(position) WHERE 'trip-france' IN tags"
        "#}
        );
        assert_parse!(
            "query directive with an unquoted name and metadata",
            indoc! {r#"
            2024-01-01 query monthly "SELECT year, month, sum(position) GROUP BY year, month"
              category: "reports"
              owner: "alice"
        "#}
        );
        assert_parse!(
            "query directive with escaped quotes",
            indoc! {r#"
            2024-01-01 query "by payee" "SELECT payee WHERE narration ~ \"coffee\""
        "#}
        );
    }

    #[test]
    fn query_with_multi_line_text_round_trips() {
        let data_type = ZhangDataType {};
        let source = "2024-01-01 query \"q\" \"SELECT account,\n  sum(position)\n GROUP BY 1\"\n  owner: \"alice\"\n";
        let directive = data_type.transform(source.to_owned(), None).unwrap().into_iter().next().unwrap();
        let exported = data_type.export(directive.clone());
        let reparsed = data_type.transform(exported, None).unwrap().into_iter().next().unwrap();
        assert_eq!(directive.data, reparsed.data);
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

    /// Every place a quoted string can appear, filled from `next`.
    fn directives_with_strings(mut next: impl FnMut() -> String) -> Vec<zhang_ast::Directive> {
        use std::str::FromStr;

        use bigdecimal::BigDecimal;
        use zhang_ast::amount::Amount;
        use zhang_ast::*;

        let date = Date::Date(chrono::NaiveDate::from_ymd_opt(2024, 1, 2).unwrap());
        let account = |name: &str| Account::from_str(name).unwrap();
        let mut meta = |key: &str| {
            let mut meta = Meta::default();
            meta.insert(key.to_owned(), ZhangString::QuoteString(next()));
            meta
        };
        let txn_meta = meta("note");
        let posting_meta = meta("receipt");
        let open_meta = meta("name");
        let query_meta = meta("owner");
        vec![
            Directive::Transaction(Transaction {
                date: date.clone(),
                flag: Some(Flag::Okay),
                payee: Some(ZhangString::QuoteString(next())),
                narration: Some(ZhangString::QuoteString(next())),
                tags: Default::default(),
                links: Default::default(),
                postings: vec![
                    Posting {
                        flag: None,
                        account: account("Assets:Cash"),
                        units: Some(Amount::new(BigDecimal::from(-1), "CNY")),
                        cost: Some(PostingCost {
                            base: Some(Amount::new(BigDecimal::from(1), "USD")),
                            date: None,
                            label: Some(next()),
                            total: false,
                        }),
                        price: None,
                        comment: None,
                        meta: posting_meta,
                    },
                    Posting {
                        flag: None,
                        account: account("Expenses:Food"),
                        units: None,
                        cost: None,
                        price: None,
                        comment: None,
                        meta: Default::default(),
                    },
                ],
                meta: txn_meta,
            }),
            Directive::Open(Open {
                date: date.clone(),
                account: account("Assets:Cash"),
                commodities: vec![],
                meta: open_meta,
            }),
            Directive::Note(Note {
                date: date.clone(),
                account: account("Assets:Cash"),
                comment: ZhangString::QuoteString(next()),
                tags: None,
                links: None,
                meta: Meta::default(),
            }),
            Directive::Document(Document {
                date: date.clone(),
                account: account("Assets:Cash"),
                filename: ZhangString::QuoteString(next()),
                tags: None,
                links: None,
                meta: Meta::default(),
            }),
            Directive::Event(Event {
                date: date.clone(),
                event_type: ZhangString::QuoteString(next()),
                description: ZhangString::QuoteString(next()),
                meta: Meta::default(),
            }),
            Directive::Query(Query {
                date: date.clone(),
                name: ZhangString::QuoteString(next()),
                query_string: ZhangString::QuoteString(next()),
                meta: query_meta,
            }),
            Directive::Custom(Custom {
                date,
                custom_type: ZhangString::QuoteString(next()),
                values: vec![StringOrAccount::String(ZhangString::QuoteString(next()))],
                meta: Meta::default(),
            }),
            Directive::Option(Options {
                key: ZhangString::QuoteString(next()),
                value: ZhangString::QuoteString(next()),
            }),
            Directive::Plugin(Plugin {
                module: ZhangString::QuoteString(next()),
                value: vec![ZhangString::QuoteString(next())],
                meta: Meta::default(),
            }),
            Directive::Include(Include {
                file: ZhangString::QuoteString(next()),
            }),
        ]
    }

    fn assert_round_trips(directive: zhang_ast::Directive) {
        use zhang_ast::{SpanInfo, Spanned};

        let data_type = ZhangDataType {};
        let exported = data_type.export(Spanned::new(directive.clone(), SpanInfo::default()));
        let reparsed = data_type
            .transform(exported.clone(), None)
            .unwrap_or_else(|err| panic!("cannot parse the exported text {exported:?}: {err}"));
        assert_eq!(reparsed.len(), 1, "{exported:?}");
        assert_eq!(reparsed[0].data, directive, "exported as {exported:?}");
    }

    #[test]
    fn issue_442_strings_round_trip() {
        for s in [
            "bell\u{7} esc\u{1b} nul\u{0} backspace\u{8} form feed\u{c}",
            "coffee $5",
            "`cmd`",
            "SELECT\u{a0}account",
            "a\u{2028}b",
            "narration ~ '\\d+'",
            "ends with \\",
            "say \"hi\"",
            "two\nlines",
        ] {
            for directive in directives_with_strings(|| s.to_owned()) {
                assert_round_trips(directive);
            }
        }
    }

    #[test]
    fn exporting_then_parsing_random_strings_is_an_identity() {
        use crate::utils::string_::test::{random_string, XorShift};

        let mut rng = XorShift::new(0x2a2a_0442);
        for _ in 0..300 {
            for directive in directives_with_strings(|| random_string(&mut rng)) {
                assert_round_trips(directive);
            }
        }
    }
}
