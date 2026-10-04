use std::path::PathBuf;

use itertools::{Either, Itertools};
use zhang_ast::*;
use zhang_core::data_type::text::exporter::ZhangDataTypeExportable;
use zhang_core::data_type::DataType;
use zhang_core::utils::plain_decimal;
use zhang_core::utils::string_::QuoteStyle;
use zhang_core::{ZhangError, ZhangResult};

use crate::directives::{BeancountDirective, BeancountOnlyDirective};
use crate::parser::{parse, parse_time};

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
            let Spanned { span, mut data } = directives;
            self.extract_time_from_meta(&mut data);
            match data {
                Either::Left(mut zhang_directive) => {
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
        let Spanned { data, .. } = convert_datetime_to_date(directive);
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
            Directive::Budget(budget) => Directive::Custom(Custom {
                date: budget.date,
                custom_type: ZhangString::unquote("budget"),
                values: vec![
                    StringOrAccount::String(ZhangString::unquote(budget.name)),
                    StringOrAccount::String(ZhangString::unquote(budget.commodity)),
                ],
                meta: budget.meta,
            })
            .export_as(STYLE),
            Directive::BudgetAdd(budget) => Directive::Custom(Custom {
                date: budget.date,
                custom_type: ZhangString::unquote("budget-add"),
                values: vec![
                    StringOrAccount::String(ZhangString::unquote(budget.name)),
                    StringOrAccount::String(ZhangString::unquote(plain_decimal(&budget.amount.number))),
                    StringOrAccount::String(ZhangString::unquote(budget.amount.commodity)),
                ],
                meta: budget.meta,
            })
            .export_as(STYLE),
            Directive::BudgetTransfer(budget) => Directive::Custom(Custom {
                date: budget.date,
                custom_type: ZhangString::unquote("budget-transfer"),
                values: vec![
                    StringOrAccount::String(ZhangString::unquote(budget.from)),
                    StringOrAccount::String(ZhangString::unquote(budget.to)),
                    StringOrAccount::String(ZhangString::unquote(plain_decimal(&budget.amount.number))),
                    StringOrAccount::String(ZhangString::unquote(budget.amount.commodity)),
                ],
                meta: budget.meta,
            })
            .export_as(STYLE),
            Directive::BudgetClose(budget) => Directive::Custom(Custom {
                date: budget.date,
                custom_type: ZhangString::unquote("budget-close"),
                values: vec![StringOrAccount::String(ZhangString::unquote(budget.name))],
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

macro_rules! convert_to_datetime {
    ($directive: expr) => {
        if let Date::Datetime(datetime) = $directive.date {
            let (date, time) = (datetime.date(), datetime.time());
            $directive.date = Date::Date(date);
            $directive
                .meta
                .insert("time".to_string(), ZhangString::QuoteString(time.format("%H:%M:%S").to_string()));
            $directive
        } else {
            $directive
        }
    };
}

fn convert_datetime_to_date(directive: Spanned<Directive>) -> Spanned<Directive> {
    let Spanned { data, span } = directive;
    let data = match data {
        Directive::Open(mut directive) => Directive::Open(convert_to_datetime!(directive)),
        Directive::Close(mut directive) => Directive::Close(convert_to_datetime!(directive)),
        Directive::Commodity(mut directive) => Directive::Commodity(convert_to_datetime!(directive)),
        Directive::Transaction(mut directive) => Directive::Transaction(convert_to_datetime!(directive)),
        Directive::BalanceCheck(mut directive) => Directive::BalanceCheck(convert_to_datetime!(directive)),
        Directive::BalancePad(mut directive) => Directive::BalancePad(convert_to_datetime!(directive)),
        Directive::Pad(mut directive) => Directive::Pad(convert_to_datetime!(directive)),
        Directive::Note(mut directive) => Directive::Note(convert_to_datetime!(directive)),
        Directive::Document(mut directive) => Directive::Document(convert_to_datetime!(directive)),
        Directive::Price(mut directive) => Directive::Price(convert_to_datetime!(directive)),
        Directive::Event(mut directive) => Directive::Event(convert_to_datetime!(directive)),
        Directive::Custom(mut directive) => Directive::Custom(convert_to_datetime!(directive)),
        Directive::Query(mut directive) => Directive::Query(convert_to_datetime!(directive)),
        _ => data,
    };
    Spanned::new(data, span)
}

macro_rules! extract_time {
    ($directive: tt) => {{
        let time = $directive.meta.pop_one("time").and_then(|it| parse_time(it.as_str()).ok());
        if let Some(time) = time {
            $directive.date = Date::Datetime($directive.date.naive_date().and_time(time));
        }
    }};
}

impl Beancount {
    /// the `time` metadata of a directive as its time, which orders the directives of a day. Not for a `balance` or a
    /// `pad`, whose `time` stays plain metadata: beancount knows no times, and checks a `balance` at the start of its
    /// date, before the transactions of that day, and orders the `pad`s of a day by their line
    fn extract_time_from_meta(&self, directive: &mut BeancountDirective) {
        match directive {
            Either::Left(zhang_directive) => match zhang_directive {
                Directive::Open(directive) => extract_time!(directive),
                Directive::Close(directive) => extract_time!(directive),
                Directive::Commodity(directive) => extract_time!(directive),
                Directive::Transaction(directive) => extract_time!(directive),
                Directive::Note(directive) => extract_time!(directive),
                Directive::Document(directive) => extract_time!(directive),
                Directive::Price(directive) => extract_time!(directive),
                Directive::Event(directive) => extract_time!(directive),
                Directive::Custom(directive) => extract_time!(directive),
                Directive::Query(directive) => extract_time!(directive),
                _ => {}
            },
            Either::Right(_) => {}
        }
    }
}

#[cfg(test)]
mod test {
    use std::str::FromStr;

    use bigdecimal::BigDecimal;
    use chrono::NaiveDate;
    use indoc::indoc;
    use zhang_ast::amount::Amount;
    use zhang_ast::{Account, BalanceCheck, BalancePad, Date, Directive, Meta, Open, Pad, SpanInfo, Spanned};
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
