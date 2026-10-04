use itertools::Itertools;
use log::warn;
use zhang_ast::amount::Amount;
use zhang_ast::*;

use crate::data_type::text::parser::{is_flag_char, is_posting_flag_char, is_valid_meta_key};
use crate::utils::plain_decimal;
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

/// Appends `meta` to `string`, one indented line per entry, writing quoted metadata values in `style`.
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
        format!("{} {}", plain_decimal(&self.number), self.commodity)
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
        // indented deeper than the posting) both read as the posting's.
        // Postings booking changed are written as the user wrote them: the legs of a split
        // merged back, implicit units left out, the cost spec as written
        let meta = self.meta.export_as(style).into_iter().map(|it| format!("  {}", it));
        let postings = written_postings(self.postings).into_iter().flat_map(|mut posting| {
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
        let flag = self.flag.and_then(|flag| posting_flag(flag, &self.account, style));
        let vec1 = vec![
            // the posting's own flag goes before the account, a space apart: `! Assets:Cash -10 CNY`
            flag,
            Some(self.account.export_as(style)),
            self.units.map(|it| it.export_as(style)),
            self.cost.map(|it| it.export_as(style)),
            self.price.map(|it| it.export_as(style)),
            self.comment.map(posting_comment),
        ];
        vec1.into_iter().flatten().join(" ")
    }
}

/// The flag of a posting as written before its account, when the format reads it back as the
/// posting's flag: a single [flag character](is_flag_char), which in zhang is not `#`, as an
/// indented line starting with `#` is a comment there ([`is_posting_flag_char`]). `*` is written
/// in both styles: `* Assets:Cash -10 CNY` is a posting flagged `*` in either format.
///
/// Any other flag is left out, with a warning, and the posting is written without it: its line must
/// stay a posting, as a comment would drop its amount from the balances, and as an unreadable line
/// would stop the whole file from loading. The zhang parser never reads such a flag, so in a zhang
/// file it can only come from a plugin, which sets it again on every load.
fn posting_flag(flag: Flag, account: &Account, style: QuoteStyle) -> Option<String> {
    let text = flag.export_as(style);
    let mut chars = text.chars();
    let readable = match (chars.next(), chars.next(), style) {
        (Some(c), None, QuoteStyle::Zhang) => is_posting_flag_char(c),
        (Some(c), None, QuoteStyle::Beancount) => is_flag_char(c),
        _ => false,
    };
    if readable {
        Some(text)
    } else {
        warn!(
            "the flag {text:?} of a posting to {} is left out: the {style:?} format would not read it back as the posting's flag",
            account.name()
        );
        None
    }
}

/// The comment at the end of a posting line, after `;`, which zhang and beancount both read as a
/// comment. It stays on the line: a line break, which only a plugin can put in a comment, is
/// written as a space.
fn posting_comment(comment: String) -> String {
    let comment = comment.replace(['\r', '\n'], " ");
    if comment.is_empty() {
        ";".to_owned()
    } else {
        format!("; {comment}")
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
            Some(tolerance) => format!("{} ~ {} {}", plain_decimal(&amount.number), plain_decimal(&tolerance), amount.commodity),
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

#[cfg(test)]
mod test {

    use indoc::indoc;

    use super::ZhangDataTypeExportable;
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

    /// numbers are written in plain notation, as they were written, never with an exponent: very small, very large and
    /// of a high scale
    #[test]
    fn numbers_are_written_in_plain_notation() {
        for number in [
            "0.000000001",
            "-0.000000001",
            "1200000000000000000000000000000",
            "0.1234567890123456789012345678",
            "-12345678901234567890.123456789",
            "100.50",
        ] {
            for text in [
                format!("1970-01-01 balance Assets:A {number} CNY"),
                format!("1970-01-01 balance Assets:A {number} ~ 0.000000001 CNY"),
                format!("1970-01-01 balance Assets:A {number} CNY with pad Equity:Open"),
                format!("1970-01-01 price USD {number} CNY"),
                format!("1970-01-01 * \"Payee\" \"Narration\"\n  Assets:A {number} CNY\n  Assets:B 1 CCC @@ {number} CNY"),
                format!("1970-01-01 budget-add Food {number} CNY"),
            ] {
                assert_eq!(parse_and_export(&text), text);
            }
        }
        // numbers with an exponent, as arithmetic or a request can leave them
        let amount = |number: &str| zhang_ast::amount::Amount::new(number.parse().unwrap(), "CNY").export();
        assert_eq!(amount("1E-9"), "0.000000001 CNY");
        assert_eq!(amount("1.2E+30"), "1200000000000000000000000000000 CNY");
        assert_eq!(amount("123456789E-20"), "0.00000000000123456789 CNY");
        assert_eq!(amount("-5E+3"), "-5000 CNY");
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
    fn booked_postings_are_exported_as_written() {
        use std::str::FromStr;

        use bigdecimal::BigDecimal;
        use zhang_ast::amount::Amount;
        use zhang_ast::{Account, Date, Directive, Posting, PostingCost, WrittenPosting};

        use crate::data_type::text::exporter::ZhangDataTypeExportable;
        use crate::utils::string_::QuoteStyle;

        let source = indoc! {r#"
            2024-05-18 * "sell"
              Assets:S -15 USD {}
              Income:I
        "#};
        let directives = ZhangDataType {}.transform(source.to_owned(), None).unwrap();
        let Directive::Transaction(mut txn) = directives.into_iter().next().unwrap().data else {
            panic!("a transaction")
        };
        // what booking makes of it: one leg per lot, the implicit posting interpolated
        let leg = |units: i64, cost: i64, date: &str| Posting {
            flag: None,
            account: Account::from_str("Assets:S").unwrap(),
            units: Some(Amount::new(BigDecimal::from(units), "USD")),
            cost: Some(PostingCost {
                base: Some(Amount::new(BigDecimal::from(cost), "CNY")),
                date: Some(Date::Date(chrono::NaiveDate::from_str(date).unwrap())),
                label: None,
                total: false,
            }),
            price: None,
            comment: None,
            meta: Default::default(),
            written: Some(WrittenPosting {
                index: 0,
                units: txn.postings[0].units.clone(),
                cost: txn.postings[0].cost.clone(),
            }),
        };
        let income = Posting {
            units: Some(Amount::new(BigDecimal::from(155), "CNY")),
            written: Some(WrittenPosting {
                index: 1,
                units: None,
                cost: None,
            }),
            ..txn.postings[1].clone()
        };
        txn.postings = vec![leg(-10, 10, "2024-05-16"), leg(-5, 11, "2024-05-17"), income];

        // the legs merged back into the written posting, the implicit posting without units
        // (the exporter writes an empty cost spec as `{ }`)
        assert_eq!(
            Directive::Transaction(txn).export_as(QuoteStyle::Zhang),
            indoc! {r#"
                2024-05-18 * "sell"
                  Assets:S -15 USD { }
                  Income:I
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

    /// A posting's own flag is written before its account, a space apart, and its comment at the
    /// end of its line (#474): what is read is written and read back unchanged.
    #[test]
    fn posting_flags_and_comments_round_trip() {
        use zhang_ast::{Directive, Flag};

        use crate::utils::string_::QuoteStyle;

        let source = indoc! {r#"
            2024-01-10 * "Broker" "sell"
              note: "t"
              ! Assets:Broker -5 AAPL {} @ 200 USD   ; check the lot
                receipt: "r-1"
              &   Assets:Bank 1000 USD
              ? Assets:Broker 2 AAPL {{ 400 USD }} @@ 420 USD // fees
              X Income:Gains ;
              # Income:Gains -1 USD ; a posting commented out
              *	Expenses:Tax 1 USD
              Expenses:Fees 2 USD
        "#};
        let expected = indoc! {r#"
            2024-01-10 * "Broker" "sell"
              note: "t"
              ! Assets:Broker -5 AAPL { } @ 200 USD ; check the lot
                receipt: "r-1"
              & Assets:Bank 1000 USD
              ? Assets:Broker 2 AAPL {{ 400 USD }} @@ 420 USD ; fees
              X Income:Gains ;
              * Expenses:Tax 1 USD
              Expenses:Fees 2 USD
        "#}
        .trim();
        assert_eq!(parse_and_export(source.trim()), expected);

        let data_type = ZhangDataType {};
        let directive = data_type.transform(source.to_owned(), None).unwrap().pop().unwrap().data;
        let Directive::Transaction(txn) = &directive else { unreachable!() };
        let flags = txn.postings.iter().map(|it| it.flag.clone()).collect::<Vec<_>>();
        let custom = |flag: &str| Some(Flag::Custom(flag.to_owned()));
        assert_eq!(flags, vec![Some(Flag::Warning), custom("&"), custom("?"), custom("X"), Some(Flag::Okay), None]);
        assert_eq!(txn.postings[3].comment.as_deref(), Some(""));
        // the beancount style writes the same text
        assert_eq!(txn.clone().export_as(QuoteStyle::Beancount), expected);
        assert_round_trips(directive);
    }

    /// `#` starts a comment in a zhang file, so the zhang style leaves out a `#` posting flag, which
    /// only a plugin can set there, and keeps the posting: a comment would drop its amount. A `*`
    /// flag is written, as both formats read it back. The beancount style writes `#` too, as
    /// beancount reads it. A flag no format reads is left out of both.
    #[test]
    fn a_posting_flag_that_would_read_back_as_a_comment_is_left_out() {
        use zhang_ast::{Directive, Flag, SpanInfo, Spanned};

        use crate::utils::string_::QuoteStyle;

        let data_type = ZhangDataType {};
        let mut directive = data_type
            .transform(
                "2024-01-10 * \"Lunch\"\n  Assets:Cash -10 USD\n  Expenses:Food 6 USD\n  Expenses:Drinks\n".to_owned(),
                None,
            )
            .unwrap()
            .pop()
            .unwrap()
            .data;
        let Directive::Transaction(txn) = &mut directive else { unreachable!() };
        txn.postings[0].flag = Some(Flag::Okay);
        txn.postings[1].flag = Some(Flag::Custom("#".to_owned()));
        txn.postings[2].flag = Some(Flag::Custom("ab".to_owned()));

        let zhang = data_type.export(Spanned::new(directive.clone(), SpanInfo::default()));
        assert_eq!(
            zhang,
            "2024-01-10 * \"Lunch\"\n  * Assets:Cash -10 USD\n  Expenses:Food 6 USD\n  Expenses:Drinks"
        );
        let Directive::Transaction(reread) = data_type.transform(zhang, None).unwrap().pop().unwrap().data else {
            unreachable!()
        };
        let flags = reread.postings.iter().map(|it| it.flag.clone()).collect::<Vec<_>>();
        assert_eq!(flags, vec![Some(Flag::Okay), None, None]);

        let Directive::Transaction(txn) = directive else { unreachable!() };
        assert_eq!(
            txn.export_as(QuoteStyle::Beancount),
            "2024-01-10 * \"Lunch\"\n  * Assets:Cash -10 USD\n  # Expenses:Food 6 USD\n  Expenses:Drinks"
        );
    }

    #[test]
    fn a_line_break_in_a_posting_comment_is_written_as_a_space() {
        use zhang_ast::Directive;

        let data_type = ZhangDataType {};
        let mut directive = data_type
            .transform("2024-01-10 * \"Lunch\"\n  ! Assets:Cash -10 USD\n  Expenses:Food\n".to_owned(), None)
            .unwrap()
            .pop()
            .unwrap();
        let Directive::Transaction(txn) = &mut directive.data else { unreachable!() };
        txn.postings[0].comment = Some("from\na plugin".to_owned());
        assert_eq!(
            data_type.export(directive),
            "2024-01-10 * \"Lunch\"\n  ! Assets:Cash -10 USD ; from a plugin\n  Expenses:Food"
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
                        written: None,
                    },
                    Posting {
                        flag: None,
                        account: account("Expenses:Food"),
                        units: None,
                        cost: None,
                        price: None,
                        comment: None,
                        meta: Default::default(),
                        written: None,
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
