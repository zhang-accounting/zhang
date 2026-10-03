use std::path::PathBuf;

use itertools::{Either, Itertools};
use zhang_ast::*;
use zhang_core::data_type::text::exporter::{append_meta_as, ZhangDataTypeExportable};
use zhang_core::data_type::DataType;
use zhang_core::utils::string_::QuoteStyle;
use zhang_core::{ZhangError, ZhangResult};

use crate::directives::{BalanceDirective, BeancountDirective, BeancountOnlyDirective};
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
            Directive::BalanceCheck(check) => BalanceDirective {
                date: check.date,
                account: check.account,
                amount: check.amount,
                tolerance: check.tolerance,

                meta: check.meta,
            }
            .bc_to_string(),
            Directive::BalancePad(pad) => {
                let balance_date = pad.date.naive_date();
                let pad_date = balance_date.pred_opt().unwrap_or(balance_date);
                let pad_directive = Pad {
                    date: Date::Date(pad_date),
                    account: pad.account.clone(),
                    pad: pad.pad,
                    meta: Meta::default(),
                };
                let balance_directive = BalanceDirective {
                    date: pad.date,
                    account: pad.account,
                    amount: pad.amount,
                    tolerance: None,

                    meta: pad.meta,
                };
                [pad_directive.export_as(STYLE), balance_directive.bc_to_string()].join("\n")
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
                    StringOrAccount::String(ZhangString::unquote(budget.amount.number.to_string())),
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
                    StringOrAccount::String(ZhangString::unquote(budget.amount.number.to_string())),
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

trait BeancountOnlyExportable {
    fn bc_to_string(self) -> String;
}

impl BeancountOnlyExportable for BalanceDirective {
    fn bc_to_string(self) -> String {
        let BalanceDirective {
            date,
            account,
            amount,
            tolerance,
            meta,
            ..
        } = self;
        let amount_str = match tolerance {
            Some(tolerance) => format!("{} ~ {} {}", amount.number, tolerance, amount.commodity),
            None => ZhangDataTypeExportable::export(amount),
        };
        let line = [
            ZhangDataTypeExportable::export(date),
            "balance".to_string(),
            ZhangDataTypeExportable::export(account),
            amount_str,
        ]
        .join(" ");
        append_meta_as(meta, line, QuoteStyle::Beancount)
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
    fn extract_time_from_meta(&self, directive: &mut BeancountDirective) {
        match directive {
            Either::Left(zhang_directive) => match zhang_directive {
                Directive::Open(directive) => extract_time!(directive),
                Directive::Close(directive) => extract_time!(directive),
                Directive::Commodity(directive) => extract_time!(directive),
                Directive::Transaction(directive) => extract_time!(directive),
                Directive::BalanceCheck(balance_check) => extract_time!(balance_check),
                Directive::BalancePad(balance_pad) => extract_time!(balance_pad),
                Directive::Pad(directive) => extract_time!(directive),
                Directive::Note(directive) => extract_time!(directive),
                Directive::Document(directive) => extract_time!(directive),
                Directive::Price(directive) => extract_time!(directive),
                Directive::Event(directive) => extract_time!(directive),
                Directive::Custom(directive) => extract_time!(directive),
                Directive::Query(directive) => extract_time!(directive),
                _ => {}
            },
            Either::Right(beancount_onyly_directive) => match beancount_onyly_directive {
                BeancountOnlyDirective::Balance(directive) => extract_time!(directive),
                _ => {}
            },
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
        assert_eq!(
            directives.pop().unwrap().data,
            Directive::Pad(Pad {
                date: Date::Datetime(NaiveDate::from_ymd_opt(1970, 1, 1).unwrap().and_hms_opt(8, 0, 0).unwrap()),
                account: Account::from_str("Assets:BankAccount").unwrap(),
                pad: Account::from_str("Equity:Open-Balances").unwrap(),
                meta: Default::default(),
            })
        );
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
        assert_eq!(Beancount::default().transform(exported, None).unwrap().pop().unwrap().data, pad);
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
}
