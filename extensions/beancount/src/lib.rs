use std::path::PathBuf;

use itertools::{Either, Itertools};
use zhang_ast::*;
use zhang_core::data_type::text::exporter::ZhangDataTypeExportable;
use zhang_core::data_type::DataType;
use zhang_core::utils::string_::QuoteStyle;
use zhang_core::utils::{plain_decimal, read_time_of_day};
use zhang_core::{ZhangError, ZhangResult};

use crate::directives::BeancountOnlyDirective;
use crate::parser::parse;

#[allow(clippy::upper_case_acronyms)]
#[allow(clippy::type_complexity)]
pub mod parser;

pub mod directives;

#[derive(Clone, Default)]
pub struct Beancount {}

impl DataType for Beancount {
    type Carrier = String;

    fn transform(&self, raw_data: Self::Carrier, source: Option<String>) -> ZhangResult<Vec<Spanned<Directive>>> {
        let path = source.clone().map(PathBuf::from);
        let directives = parse(&raw_data, path).map_err(|it| ZhangError::PestError {
            path: source.unwrap_or_default(),
            msg: it.to_string(),
        })?;

        let mut ret = vec![];
        let mut tags_stack: Vec<String> = vec![];
        let mut meta_stack: Vec<(String, ZhangString)> = vec![];

        for directives in directives {
            let Spanned { span, data } = directives;
            match data {
                Either::Left(mut zhang_directive) => {
                    lift_time_from_meta(&mut zhang_directive);
                    // apply pushed metadata (pushmeta) without overriding explicit keys
                    if let Some(meta) = zhang_directive.meta_mut() {
                        for (key, value) in &meta_stack {
                            if meta.get_one(key).is_none() {
                                meta.insert(key.clone(), value.clone());
                            }
                        }
                    }
                    if let Directive::Transaction(trx) = &mut zhang_directive {
                        for tag in &tags_stack {
                            trx.tags.insert(tag.to_owned());
                        }
                    }
                    ret.push(Spanned { span, data: zhang_directive });
                }
                Either::Right(beancount_directive) => match beancount_directive {
                    BeancountOnlyDirective::PushTag(tag) => tags_stack.push(tag),
                    BeancountOnlyDirective::PopTag(tag) => tags_stack = tags_stack.into_iter().filter(|it| it.ne(&tag)).collect_vec(),
                    BeancountOnlyDirective::PushMeta(key, value) => meta_stack.push((key, value)),
                    BeancountOnlyDirective::PopMeta(key) => {
                        if let Some(pos) = meta_stack.iter().rposition(|(k, _)| k == &key) {
                            meta_stack.remove(pos);
                        }
                    }
                    // a `pad` is a directive of its own: the pad stage pairs it with the balances it serves, across
                    // the files of the ledger
                    BeancountOnlyDirective::Balance(balance) => ret.push(Spanned {
                        span,
                        data: Directive::BalanceCheck(BalanceCheck {
                            date: balance.date,
                            account: balance.account,
                            amount: balance.amount,
                            tolerance: balance.tolerance,
                            meta: balance.meta,
                        }),
                    }),
                },
            }
        }
        Ok(ret)
    }

    fn export(&self, directive: Spanned<Directive>) -> Self::Carrier {
        // the shared zhang exporter writes these directives in beancount syntax; quoted
        // strings use only the escapes beancount decodes, see `QuoteStyle::Beancount`
        const STYLE: QuoteStyle = QuoteStyle::Beancount;
        let mut data = time_into_meta(directive.data);
        quote_unreadable_meta(&mut data);
        match data {
            Directive::BalancePad(pad) => {
                let balance_date = pad.date.naive_date();
                let pad_date = balance_date.pred_opt().unwrap_or(balance_date);
                let pad_directive = Pad {
                    date: Date::Date(pad_date),
                    account: pad.account.clone(),
                    pad: pad.pad,
                    meta: Meta::default(),
                };
                let balance_directive = BalanceCheck {
                    date: pad.date,
                    account: pad.account,
                    amount: pad.amount,
                    tolerance: None,
                    meta: pad.meta,
                };
                [pad_directive.export_as(STYLE), balance_directive.export_as(STYLE)].join("\n")
            }
            // budgets are `custom` directives in the form beancount accepts (#500): a quoted type,
            // quoted names, a quoted commodity, and a plain `1000 CNY` as an amount
            Directive::Budget(budget) => Directive::Custom(Custom {
                date: budget.date,
                custom_type: ZhangString::quote("budget"),
                values: vec![
                    StringOrAccount::String(ZhangString::quote(budget.name)),
                    StringOrAccount::String(ZhangString::quote(budget.commodity)),
                ],
                meta: budget.meta,
            })
            .export_as(STYLE),
            Directive::BudgetAdd(budget) => Directive::Custom(Custom {
                date: budget.date,
                custom_type: ZhangString::quote("budget-add"),
                values: vec![
                    StringOrAccount::String(ZhangString::quote(budget.name)),
                    StringOrAccount::String(ZhangString::unquote(plain_decimal(&budget.amount.number))),
                    StringOrAccount::String(ZhangString::unquote(budget.amount.commodity)),
                ],
                meta: budget.meta,
            })
            .export_as(STYLE),
            Directive::BudgetTransfer(budget) => Directive::Custom(Custom {
                date: budget.date,
                custom_type: ZhangString::quote("budget-transfer"),
                values: vec![
                    StringOrAccount::String(ZhangString::quote(budget.from)),
                    StringOrAccount::String(ZhangString::quote(budget.to)),
                    StringOrAccount::String(ZhangString::unquote(plain_decimal(&budget.amount.number))),
                    StringOrAccount::String(ZhangString::unquote(budget.amount.commodity)),
                ],
                meta: budget.meta,
            })
            .export_as(STYLE),
            Directive::BudgetClose(budget) => Directive::Custom(Custom {
                date: budget.date,
                custom_type: ZhangString::quote("budget-close"),
                values: vec![StringOrAccount::String(ZhangString::quote(budget.name))],
                meta: budget.meta,
            })
            .export_as(STYLE),
            // beancount only accepts a quoted query name
            Directive::Query(query) => Directive::Query(Query {
                name: ZhangString::quote(query.name.to_plain_string()),
                ..query
            })
            .export_as(STYLE),
            _ => data.export_as(STYLE),
        }
    }
}

/// `directive` with a date beancount reads: a date and time, to the second or to the minute (a plugin can emit a
/// `Date::DateHour`), becomes the date alone plus `time` metadata, budget directives included: beancount reads no
/// time after a date. The directive's time is the first `time` entry, before any `time` its metadata already holds:
/// the first one is read back as the directive's time ([`lift_time_from_meta`]), so the real time wins and the
/// other stays metadata
fn time_into_meta(mut directive: Directive) -> Directive {
    let Some(date) = directive.date_mut() else { return directive };
    let (Date::Datetime(datetime) | Date::DateHour(datetime)) = *date else {
        return directive;
    };
    *date = Date::Date(datetime.date());
    if let Some(meta) = directive.meta_mut() {
        let written = std::iter::from_fn(|| meta.pop_one("time")).collect::<Vec<_>>();
        meta.insert("time".to_string(), ZhangString::QuoteString(datetime.time().format("%H:%M:%S").to_string()));
        for time in written {
            meta.insert("time".to_string(), time);
        }
    }
    directive
}

/// `directive` with every metadata value written bare that beancount cannot read bare quoted, on the directive and on
/// the postings of a transaction: zhang reads any bare value (`budget: Food`), beancount only those [`reads_bare`]
/// lists. Zhang reads the quoted value back as the same text.
fn quote_unreadable_meta(directive: &mut Directive) {
    fn requote(meta: &mut Meta) {
        let entries = std::mem::take(meta).get_flatten();
        *meta = entries
            .into_iter()
            .map(|(key, value)| match value {
                ZhangString::UnquoteString(text) if !reads_bare(&text) => (key, ZhangString::QuoteString(text)),
                value => (key, value),
            })
            .collect();
    }
    if let Some(meta) = directive.meta_mut() {
        requote(meta);
    }
    if let Directive::Transaction(transaction) = directive {
        for posting in &mut transaction.postings {
            requote(&mut posting.meta);
        }
    }
}

/// Whether beancount 3.2.3 reads `value`, a metadata value written without quotes, as that same text; any other value
/// is written quoted. The rules are those of beancount's lexer (`beancount/parser/lexer.l`) and of its account check
/// (`valid_account_regexp`, `beancount/parser/grammar.py:109-127`):
///
/// - `TRUE`, `FALSE` and `NULL` (lexer.l:154-164);
/// - a tag, `#[A-Za-z0-9\-_/.]+` (lexer.l:287): ASCII only, `#旅行` is a syntax error;
/// - a date, `[0-9]{4,}[\-/][0-9]+[\-/][0-9]+` (lexer.l:267), here a valid `YYYY-MM-DD`;
/// - a currency, `[A-Z][A-Z0-9'._-]*[A-Z0-9]` or a single capital (lexer.l:203, :240);
/// - a plain number, `([0-9]+|[0-9][0-9,]+[0-9])(\.[0-9]*)?` (lexer.l:282) after an optional sign, its commas only
///   between groups of three digits, as `^[0-9]{1,3}(,[0-9]{3})*` (beancount reports `1,5`, `1,00,000` or `1,,2` as an
///   "Invalid number format"), or an amount, such a number and a currency. Beancount evaluates the arithmetic of numbers
///   in a value, so `978-3-16` would read as 959 and `2024-01` as 2023, and two numbers (`12 34`) are a syntax error:
///   every other value made of digits is quoted;
/// - an account, `{ACCOUNTTYPE}(:{ACCOUNTNAME})+` (lexer.l:129-130, :272): components of ASCII letters, digits and
///   `-` and of any non-ASCII character, each starting with an ASCII capital or digit or a non-ASCII character. Beancount
///   then matches it from its start against its account pattern: the root is one of the five account types, and the
///   first component after it starts with an uppercase letter or a decimal digit (Unicode letters and digits follow).
///   `Expenses:Ü-Bahn` and `Expenses:Food:餐饮` are accounts; `Expenses:餐饮` is not ("Invalid account name").
fn reads_bare(value: &str) -> bool {
    fn currency(text: &str) -> bool {
        let mut chars = text.chars();
        let (Some(first), Some(last)) = (chars.next(), text.chars().last()) else {
            return false;
        };
        first.is_ascii_uppercase()
            && (last.is_ascii_uppercase() || last.is_ascii_digit())
            && chars.all(|it| it.is_ascii_uppercase() || it.is_ascii_digit() || matches!(it, '\'' | '.' | '_' | '-'))
    }
    fn number(text: &str) -> bool {
        let digits = |text: &str| !text.is_empty() && text.bytes().all(|it| it.is_ascii_digit());
        let unsigned = text.strip_prefix(['-', '+']).unwrap_or(text);
        let (whole, fraction) = unsigned.split_once('.').map_or((unsigned, ""), |(whole, fraction)| (whole, fraction));
        // digits, or groups of three digits after a first group of one to three: `^[0-9]{1,3}(,[0-9]{3})*`
        let mut groups = whole.split(',');
        let first = groups.next().unwrap_or_default();
        let grouped = digits(first) && (!whole.contains(',') || (first.len() <= 3 && groups.all(|group| group.len() == 3 && digits(group))));
        grouped && fraction.bytes().all(|it| it.is_ascii_digit())
    }
    fn account(text: &str) -> bool {
        let mut components = text.split(':');
        let root = components.next().unwrap_or_default();
        let components = components.collect::<Vec<_>>();
        let lexed = |component: &str| {
            component.starts_with(|it: char| it.is_ascii_uppercase() || it.is_ascii_digit() || !it.is_ascii())
                && component.chars().all(|it| it.is_ascii_alphanumeric() || it == '-' || !it.is_ascii())
        };
        matches!(root, "Assets" | "Liabilities" | "Equity" | "Income" | "Expenses")
            && components
                .first()
                .is_some_and(|first| first.starts_with(|it: char| it.is_ascii_digit() || it.is_uppercase()))
            && components.iter().all(|component| lexed(component))
    }
    let tag = |text: &str| {
        text.strip_prefix('#')
            .is_some_and(|name| !name.is_empty() && name.chars().all(|it| it.is_ascii_alphanumeric() || matches!(it, '-' | '_' | '/' | '.')))
    };
    let date = |text: &str| text.len() == 10 && chrono::NaiveDate::parse_from_str(text, "%Y-%m-%d").is_ok();
    let amount = |text: &str| {
        text.rsplit_once(' ')
            .is_some_and(|(number_part, unit)| number(number_part.trim_end()) && currency(unit))
    };
    matches!(value, "TRUE" | "FALSE" | "NULL") || tag(value) || date(value) || account(value) || currency(value) || number(value) || amount(value)
}

/// the `time` metadata of a directive as its time, which orders the directives of a day. Not for a `balance`, a `pad`
/// or a `close`, whose `time` stays plain metadata: beancount knows no times, checks a `balance` at the start of its
/// date, before the transactions of that day, orders the `pad`s of a day by their line, and keeps an account active
/// through the whole day of its `close`. A budget directive's `time` is its time, as zhang writes it: a `budget-close`
/// with a time closes its budget at that time
fn lift_time_from_meta(directive: &mut Directive) {
    if !matches!(
        directive,
        Directive::Open(_)
            | Directive::Commodity(_)
            | Directive::Transaction(_)
            | Directive::Note(_)
            | Directive::Document(_)
            | Directive::Price(_)
            | Directive::Event(_)
            | Directive::Custom(_)
            | Directive::Query(_)
            | Directive::Budget(_)
            | Directive::BudgetAdd(_)
            | Directive::BudgetTransfer(_)
            | Directive::BudgetClose(_)
    ) {
        return;
    }
    // a `time` that is no time of day stays plain metadata
    let Some(time) = directive
        .meta()
        .and_then(|meta| meta.get_one("time"))
        .and_then(|it| read_time_of_day(it.as_str()))
    else {
        return;
    };
    if let Some(meta) = directive.meta_mut() {
        meta.pop_one("time");
    }
    if let Some(date) = directive.date_mut() {
        *date = Date::Datetime(date.naive_date().and_time(time));
    }
}

#[cfg(test)]
mod test {
    use std::str::FromStr;

    use bigdecimal::BigDecimal;
    use chrono::NaiveDate;
    use indoc::indoc;
    use zhang_ast::amount::Amount;
    use zhang_ast::{Account, BalanceCheck, BalancePad, Date, Directive, Meta, Open, Pad, SpanInfo, Spanned, ZhangString};
    use zhang_core::data_type::DataType;

    use crate::directives::BeancountOnlyDirective;
    use crate::{parse, Beancount};

    macro_rules! test_parse_zhang {
        ($content: expr) => {{
            let directive = parse($content, None).unwrap().into_iter().next().unwrap().data;
            directive.left().unwrap()
        }};
    }
    macro_rules! test_parse_bc {
        ($content: expr) => {{
            let directive = parse($content, None).unwrap().into_iter().next().unwrap().data;
            directive.right().unwrap()
        }};
    }

    #[test]
    fn should_keep_time_into_meta_for_open_directive() {
        let mut directive = test_parse_zhang! {"1970-01-01 open Assets:BankAccount"};
        match &mut directive {
            Directive::Open(ref mut open) => open.date = Date::Datetime(open.date.naive_date().and_hms_nano_opt(1, 1, 1, 0).unwrap()),
            _ => unreachable!("only open directive"),
        }

        let beancount_exporter = Beancount {};
        assert_eq!(
            indoc! {r#"
                1970-01-01 open Assets:BankAccount
                  time: "01:01:01"
            "#}
            .trim(),
            beancount_exporter.export(Spanned::new(directive, SpanInfo::default())),
            "should persist time into meta"
        );
    }

    /// What beancount 3.2.3 reads bare as the same text, and what it reads otherwise or rejects (checked with
    /// bean-check and `beancount.loader`): an arithmetic of numbers is evaluated (`978-3-16` is 959), two numbers, a
    /// non-ASCII tag and an account whose first component starts with no uppercase letter or digit are errors.
    #[test]
    fn bare_values_are_those_beancount_reads_as_written() {
        for value in [
            "TRUE",
            "FALSE",
            "NULL",
            "#trip",
            "#a-b_c/d.e",
            "2024-01-10",
            "0024-01-01",
            "USD",
            "V",
            "NT.TO",
            "C345",
            "10",
            "10.50",
            "10.",
            "-3",
            "+3",
            "1,000.50",
            "1,000",
            "12,345,678.90",
            "-1,000.5",
            "1234",
            "10 USD",
            "-10 USD",
            "Assets:Cash",
            "Expenses:Ü-Bahn",
            "Expenses:Food:餐饮",
            "Liabilities:Card-1",
        ] {
            assert!(super::reads_bare(value), "{value} is read bare");
        }
        for value in [
            "Food",
            "978-3-16",
            "2024-01",
            "12 34",
            "1,2",
            "1,23",
            "1,,2",
            "1234,567",
            "1,000,00",
            "1,5",
            "1,00,000",
            "-1,5",
            "1,5 USD",
            "(1 + 2) * 3",
            "1 + 2 USD",
            "1/2",
            "#旅行",
            "#",
            "2024-13-45",
            "Expenses:餐饮",
            "Foo:Bar",
            "Expenses",
            "Expenses:food",
            "CAC_",
            "usd",
            "1,",
            ",1",
            "10 usd",
            "two words",
            "",
        ] {
            assert!(!super::reads_bare(value), "{value} is quoted");
        }
    }

    /// A metadata value zhang read bare is written bare to beancount only when beancount reads it as that text, and
    /// quoted otherwise; zhang reads either form back as the text it was. Before, `978-3-16` and `2024-01` stayed bare,
    /// and beancount read them as 959 and 2023.
    #[test]
    fn a_bare_metadata_value_round_trips_through_beancount_as_its_text() {
        let cases = [
            ("budget", "Food", true),
            ("isbn", "978-3-16", true),
            ("month", "2024-01", true),
            ("expression", "(1 + 2) * 3", true),
            ("expression-amount", "1 + 2 USD", true),
            ("tag", "#旅行", true),
            ("account", "Expenses:餐饮", true),
            ("ascii-tag", "#trip", false),
            ("date", "2024-01-10", false),
            ("number", "1,000.50", false),
            ("negative", "-3", false),
            ("amount", "10 USD", false),
            ("currency", "USD", false),
            ("account-ascii", "Assets:Cash", false),
            ("account-unicode", "Expenses:Food:餐饮", false),
            ("yes", "TRUE", false),
        ];
        let text = cases.iter().fold("2024-01-01 open Assets:Cash".to_owned(), |text, (key, value, _)| {
            format!("{text}\n  {key}: {value}")
        });
        let open = zhang_core::data_type::text::ZhangDataType {}.transform(text, None).unwrap().remove(0);
        for (key, value, _) in &cases {
            assert_eq!(
                open.data.meta().unwrap().get_one(*key),
                Some(&ZhangString::unquote(*value)),
                "{key} is read bare by zhang"
            );
        }
        let written = Beancount {}.export(open);
        for (key, value, quoted) in &cases {
            let line = if *quoted {
                format!("  {key}: \"{value}\"")
            } else {
                format!("  {key}: {value}")
            };
            assert!(written.lines().any(|it| it == line), "{line:?} in\n{written}");
        }
        let read = Beancount::default().transform(written, None).unwrap().remove(0);
        for (key, value, _) in &cases {
            assert_eq!(read.data.meta().unwrap().get_one(*key).map(|it| it.as_str()), Some(*value), "{key}");
        }
    }

    /// A number whose commas are not between groups of three digits is quoted: beancount reports it as an "Invalid
    /// number format" (`1,5`, a decimal comma, or `1,00,000`, Indian grouping), so written bare it would not load.
    /// Grouped by three it stays bare, and either way zhang reads it back as its text.
    #[test]
    fn a_number_is_bare_only_with_its_commas_in_groups_of_three() {
        let cases = [
            ("a", "1,2", true),
            ("b", "1,23", true),
            ("c", "1,,2", true),
            ("d", "1234,567", true),
            ("e", "1,000,00", true),
            ("f", "1,5", true),
            ("g", "1,00,000", true),
            ("h", "1,000", false),
            ("i", "12,345,678.90", false),
            ("j", "-1,000.5", false),
            ("k", "1234.5", false),
        ];
        let mut open = zhang_core::data_type::text::ZhangDataType {}
            .transform("2024-01-01 open Assets:Cash".to_owned(), None)
            .unwrap()
            .remove(0);
        for (key, value, _) in &cases {
            open.data.meta_mut().unwrap().insert(format!("value-{key}"), ZhangString::unquote(*value));
        }
        let written = Beancount {}.export(open);
        for (key, value, quoted) in &cases {
            let line = if *quoted {
                format!("  value-{key}: \"{value}\"")
            } else {
                format!("  value-{key}: {value}")
            };
            assert!(written.lines().any(|it| it == line), "{line:?} in\n{written}");
        }
        let read = Beancount::default().transform(written, None).unwrap().remove(0);
        for (key, value, _) in &cases {
            let read = read.data.meta().unwrap().get_one(format!("value-{key}").as_str()).map(|it| it.as_str());
            assert_eq!(read, Some(*value), "{key}");
        }
    }

    /// The `time` metadata of a directive in a beancount file is its time when it is a time of day, `H:M:S` or `H:M`;
    /// any other `time`, such as `evening` or `12:00:00+08:00`, stays metadata, and the directive keeps its date. Before,
    /// such a `time` was dropped, neither applied nor kept.
    #[test]
    fn a_time_metadata_entry_is_a_time_of_day_or_stays_metadata() {
        let read = |time: &str| {
            let text = format!("2024-03-10 custom \"budget-close\" \"Food\"\n  time: \"{time}\"\n2024-03-10 * \"Shop\"\n  time: \"{time}\"\n  Expenses:Food 1 CNY\n  Assets:Cash");
            Beancount::default()
                .transform(text, None)
                .unwrap()
                .into_iter()
                .map(|it| it.data)
                .collect::<Vec<_>>()
        };
        let noon = NaiveDate::from_ymd_opt(2024, 3, 10).unwrap().and_hms_opt(12, 0, 0).unwrap();
        for time in ["12:00:00", "12:00", " 12:00 "] {
            for directive in read(time) {
                assert_eq!(directive.datetime(), Some(noon), "{time}: {directive:?}");
                assert_eq!(directive.meta().unwrap().get_one("time"), None, "{time}");
            }
        }
        for time in ["evening", "12:00:00+08:00", "25:00", "12", "+1:30", "1:-30", "1::30"] {
            for directive in read(time) {
                assert!(matches!(directive.clone().date_mut(), Some(Date::Date(_))), "{time}: {directive:?}");
                assert_eq!(directive.meta().unwrap().get_one("time").map(|it| it.as_str()), Some(time), "{time}");
            }
        }
    }

    /// A zhang directive at a time whose metadata also holds a `time` of its own is written with its time first, then
    /// the other: read back, the first is its time and the other stays metadata, so neither is lost.
    #[test]
    fn the_time_of_a_directive_wins_over_a_time_in_its_metadata() {
        let text = "2024-03-01 18:00:00 budget-close Food\n  time: \"09:00:00\"\n  note: \"x\"";
        let close = zhang_core::data_type::text::ZhangDataType {}
            .transform(text.to_owned(), None)
            .unwrap()
            .remove(0);
        let written = Beancount {}.export(close.clone());
        assert_eq!(
            written,
            "2024-03-01 custom \"budget-close\" \"Food\"\n  note: \"x\"\n  time: \"18:00:00\"\n  time: \"09:00:00\""
        );
        let read = Beancount::default().transform(written, None).unwrap().remove(0).data;
        assert_eq!(read.datetime(), close.data.datetime());
        assert_eq!(
            read.meta().unwrap().get_all("time").into_iter().map(|it| it.as_str()).collect::<Vec<_>>(),
            vec!["09:00:00"]
        );
    }

    /// A directive dated to the minute (`Date::DateHour`, which a plugin can emit) was written as `1970-01-01 08:30`,
    /// which beancount cannot read: it is written like one dated to the second, the date plus `time` metadata, and
    /// reads back at that time.
    #[test]
    fn a_directive_dated_to_the_minute_round_trips_through_time_meta() {
        let at = NaiveDate::from_ymd_opt(1970, 1, 1).unwrap().and_hms_opt(8, 30, 0).unwrap();
        let mut open = test_parse_zhang! {"1970-01-01 open Assets:BankAccount"};
        let mut transaction = test_parse_zhang! {"1970-01-01 * \"lunch\"\n  Assets:BankAccount -10 CNY\n  Expenses:Food"};
        for directive in [&mut open, &mut transaction] {
            *directive.date_mut().unwrap() = Date::DateHour(at);
        }

        let exported = [&open, &transaction]
            .into_iter()
            .map(|it| Beancount {}.export(Spanned::new(it.clone(), SpanInfo::default())))
            .collect::<Vec<_>>();
        assert_eq!(
            exported,
            vec![
                "1970-01-01 open Assets:BankAccount\n  time: \"08:30:00\"",
                "1970-01-01 * \"lunch\"\n  time: \"08:30:00\"\n  Assets:BankAccount -10 CNY\n  Expenses:Food",
            ]
        );
        let read = Beancount::default().transform(exported.join("\n\n"), None).unwrap();
        assert_eq!(read.len(), 2);
        for (directive, original) in read.into_iter().zip([&open, &transaction]) {
            assert_eq!(directive.data.directive_type().to_string(), original.directive_type().to_string());
            assert_eq!(directive.data.datetime(), Some(at));
            assert_eq!(directive.data.meta().unwrap().get_one("time"), None);
        }
    }

    #[test]
    fn notes_and_documents_round_trip_their_tags_and_links() {
        let beancount_exporter = Beancount {};
        for line in [
            r#"2020-01-10 note Assets:Bank "x" #t1 ^ln"#,
            r#"2020-01-10 document Assets:Bank "a.pdf" #a #b ^l1"#,
        ] {
            let directive = test_parse_zhang! {line};
            assert_eq!(beancount_exporter.export(Spanned::new(directive, SpanInfo::default())), line);
        }
    }

    #[test]
    fn should_convert_to_pad_and_balance_directive_given_balance_pad_directive() {
        let directive = test_parse_bc! {"1970-01-02 balance Assets:BankAccount 2 CNY"};
        let directive = match directive {
            BeancountOnlyDirective::Balance(check) => Directive::BalancePad(BalancePad {
                date: check.date,
                account: check.account,
                amount: check.amount,
                pad: Account::from_str("Equity:Open-Balances").unwrap(),
                meta: Default::default(),
            }),
            _ => unreachable!("should only have balance directive"),
        };

        let beancount_exporter = Beancount {};
        assert_eq!(
            indoc! {r#"
                1970-01-01 pad Assets:BankAccount Equity:Open-Balances
                1970-01-02 balance Assets:BankAccount 2 CNY
            "#}
            .trim(),
            beancount_exporter.export(Spanned::new(directive, SpanInfo::default())),
        );
    }

    #[test]
    fn should_append_tag_to_transaction_directive_given_push_tag_directive() {
        let beancount_data_type = Beancount::default();
        let mut directives = beancount_data_type
            .transform(
                indoc! {r#"
                pushtag #onetag
                1970-01-01 "payee" "narration"
                  Assets:BancCard -100 CNY
            "#}
                .to_string(),
                None,
            )
            .unwrap();
        assert_eq!(directives.len(), 1);
        let directive = directives.pop().unwrap().data;
        match directive {
            Directive::Transaction(mut trx) => assert_eq!("onetag", trx.tags.pop().unwrap()),
            _ => unreachable!("find other directives than txn directive"),
        }
    }

    #[test]
    fn should_not_append_tag_to_transaction_directive_given_push_tag_directive() {
        let beancount_data_type = Beancount::default();

        let mut directives = beancount_data_type
            .transform(
                indoc! {r#"
                pushtag #onetag
                poptag #onetag
                1970-01-01 "payee" "narration"
                  Assets:BancCard -100 CNY
            "#}
                .to_string(),
                None,
            )
            .unwrap();

        assert_eq!(directives.len(), 1);
        let directive = directives.pop().unwrap().data;
        match directive {
            Directive::Transaction(mut trx) => assert_eq!(None, trx.tags.pop()),
            _ => unreachable!("find other directives than txn directive"),
        }
    }

    #[test]
    fn should_transform_a_pad_into_a_pad_directive() {
        let beancount_data_type = Beancount::default();
        let mut directives = beancount_data_type
            .transform(
                indoc! {r#"
                1970-01-01 pad Assets:BankAccount Equity:Open-Balances
                  time: "08:00:00"
            "#}
                .to_string(),
                None,
            )
            .unwrap();

        assert_eq!(directives.len(), 1);
        // its `time` stays plain metadata: beancount orders the pads of a day by their line
        let mut meta = Meta::default();
        meta.insert("time".to_owned(), zhang_ast::ZhangString::quote("08:00:00"));
        assert_eq!(
            directives.pop().unwrap().data,
            Directive::Pad(Pad {
                date: Date::Date(NaiveDate::from_ymd_opt(1970, 1, 1).unwrap()),
                account: Account::from_str("Assets:BankAccount").unwrap(),
                pad: Account::from_str("Equity:Open-Balances").unwrap(),
                meta,
            })
        );
    }

    #[test]
    fn should_check_a_balance_at_the_start_of_its_date_whatever_its_time() {
        // beancount checks a balance before the transactions of its day; its `time` stays plain metadata
        let directives = Beancount::default()
            .transform(
                indoc! {r#"
                1970-01-02 balance Assets:BankAccount 100 CNY
                  time: "20:00:00"
            "#}
                .to_string(),
                None,
            )
            .unwrap();
        let Directive::BalanceCheck(check) = &directives[0].data else {
            panic!("a balance is a balance check")
        };
        assert_eq!(check.date, Date::Date(NaiveDate::from_ymd_opt(1970, 1, 2).unwrap()));
        assert_eq!(check.meta.get_one("time").map(|it| it.as_str()), Some("20:00:00"));
        // and it is written back as it was read
        let exported = Beancount {}.export(directives[0].clone());
        assert_eq!(exported, "1970-01-02 balance Assets:BankAccount 100 CNY\n  time: \"20:00:00\"");
    }

    #[test]
    fn should_transform_to_balance_check_directive_given_balance_directive() {
        let beancount_data_type = Beancount::default();
        let mut directives = beancount_data_type
            .transform(
                indoc! {r#"
                1970-01-02 balance Assets:BankAccount 100 CNY
            "#}
                .to_string(),
                None,
            )
            .unwrap();

        assert_eq!(directives.len(), 1);

        let balance_pad_directive = directives.pop().unwrap().data;

        assert_eq!(
            balance_pad_directive,
            Directive::BalanceCheck(BalanceCheck {
                date: Date::Date(NaiveDate::from_ymd_opt(1970, 1, 2).unwrap()),
                account: Account::from_str("Assets:BankAccount").unwrap(),
                amount: Amount::new(BigDecimal::from(100i32), "CNY"),
                tolerance: None,
                meta: Default::default(),
            })
        );
    }

    #[test]
    fn should_keep_a_pad_and_the_balance_it_serves_apart() {
        // the pad stage pairs them, across the files of the ledger
        let directives = Beancount::default()
            .transform(
                indoc! {r#"
                1970-01-01 pad Assets:BankAccount Equity:Open-Balances
                1970-01-02 balance Assets:BankAccount 100 CNY
            "#}
                .to_string(),
                None,
            )
            .unwrap();
        let kinds = directives.iter().map(|it| it.data.directive_type().to_string()).collect::<Vec<_>>();
        assert_eq!(kinds, vec!["Pad", "BalanceCheck"]);
    }

    #[test]
    fn should_export_a_pad() {
        let pad = Directive::Pad(Pad {
            date: Date::Datetime(NaiveDate::from_ymd_opt(1970, 1, 1).unwrap().and_hms_opt(8, 0, 0).unwrap()),
            account: Account::from_str("Assets:BankAccount").unwrap(),
            pad: Account::from_str("Equity:Open-Balances").unwrap(),
            meta: Default::default(),
        });
        let exported = Beancount {}.export(Spanned::new(pad.clone(), SpanInfo::default()));
        assert_eq!(exported, "1970-01-01 pad Assets:BankAccount Equity:Open-Balances\n  time: \"08:00:00\"");
        // read back, the time is plain metadata
        let Directive::Pad(read) = Beancount::default().transform(exported, None).unwrap().pop().unwrap().data else {
            panic!("a pad is a pad")
        };
        assert_eq!(read.date.naive_date(), pad.datetime().unwrap().date());
        assert_eq!(read.meta.get_one("time").map(|it| it.as_str()), Some("08:00:00"));
    }

    #[test]
    fn should_parse_time_from_meta() {
        let beancount_data_type = Beancount::default();

        let mut directives = beancount_data_type
            .transform(
                indoc! {r#"
                1970-01-02 open Assets:BankAccount
                  time: "01:02:03"
            "#}
                .to_string(),
                None,
            )
            .unwrap();

        assert_eq!(directives.len(), 1);

        let balance_pad_directive = directives.pop().unwrap().data;

        assert_eq!(
            balance_pad_directive,
            Directive::Open(Open {
                date: Date::Datetime(NaiveDate::from_ymd_opt(1970, 1, 2).unwrap().and_hms_micro_opt(1, 2, 3, 0).unwrap()),
                account: Account::from_str("Assets:BankAccount").unwrap(),
                commodities: vec![],
                meta: Meta::default(),
            })
        );
    }
    /// numbers are written in plain notation, as they were read, never with an exponent, which beancount cannot read:
    /// the directives of `tests/balance_assertions/plain_decimals.bean`, which beancount reads (see its oracle), are
    /// written back as they are, and so are numbers left with an exponent
    #[test]
    fn numbers_are_written_in_plain_notation() {
        let fixture = include_str!("../tests/balance_assertions/plain_decimals.bean");
        let beancount = Beancount {};
        let mut written = 0;
        for directive in beancount.transform(fixture.to_owned(), None).unwrap() {
            if !matches!(directive.data, Directive::Transaction(_) | Directive::BalanceCheck(_)) {
                continue;
            }
            let text = directive.span.content.trim_end().to_owned();
            assert_eq!(beancount.export(directive).trim_end(), text);
            written += 1;
        }
        assert_eq!(written, 7);

        let balance = |number: &str, account: &str, tolerance: Option<&str>| {
            let balance = Directive::BalanceCheck(BalanceCheck {
                date: Date::Date(NaiveDate::from_ymd_opt(2024, 1, 3).unwrap()),
                account: Account::from_str(account).unwrap(),
                amount: Amount::new(BigDecimal::from_str(number).unwrap(), "CNY"),
                tolerance: tolerance.map(|it| BigDecimal::from_str(it).unwrap()),
                meta: Meta::default(),
            });
            beancount.export(Spanned::new(balance, SpanInfo::default()))
        };
        for (number, account) in [("1E-9", "Assets:Small"), ("1.2E+30", "Assets:Large")] {
            let written = balance(number, account, None);
            assert!(fixture.lines().any(|line| line == written), "{written}");
        }
        assert_eq!(
            balance("1E-9", "Assets:Small", Some("5E-10")),
            "2024-01-03 balance Assets:Small 0.000000001 ~ 0.0000000005 CNY"
        );
    }
}
