use std::collections::HashMap;
use std::path::PathBuf;

use chrono::NaiveDate;
use itertools::{Either, Itertools};
use latestmap::LatestMap;

use zhang_ast::*;
use zhang_core::data_type::text::exporter::{append_meta, ZhangDataTypeExportable};
use zhang_core::data_type::text::ZhangDataType;
use zhang_core::data_type::DataType;
use zhang_core::{ZhangError, ZhangResult};

use crate::directives::{BalanceDirective, BeancountDirective, BeancountOnlyDirective, PadDirective};
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
        let path = source.map(PathBuf::from);
        let directives = parse(&raw_data, path).map_err(|it| ZhangError::PestError(it.to_string()))?;

        let mut ret = vec![];
        let mut tags_stack: Vec<String> = vec![];

        let mut pad_info: LatestMap<NaiveDate, HashMap<String, Account>> = LatestMap::default();

        for directives in directives {
            let Spanned { span, mut data } = directives;
            self.extract_time_from_meta(&mut data);
            match data {
                Either::Left(zhang_directive) => match zhang_directive {
                    Directive::Transaction(mut trx) => {
                        for tag in &tags_stack {
                            trx.tags.insert(tag.to_owned());
                        }
                        ret.push(Spanned {
                            span,
                            data: Directive::Transaction(trx),
                        });
                    }
                    _ => ret.push(Spanned { span, data: zhang_directive }),
                },
                Either::Right(beancount_directive) => match beancount_directive {
                    BeancountOnlyDirective::PushTag(tag) => tags_stack.push(tag),
                    BeancountOnlyDirective::PopTag(tag) => tags_stack = tags_stack.into_iter().filter(|it| it.ne(&tag)).collect_vec(),
                    BeancountOnlyDirective::Pad(pad) => {
                        let date = pad.date.naive_date();
                        if !pad_info.contains_key(&date) {
                            pad_info.insert(date, HashMap::new());
                        }
                        let target_date_pad_info = pad_info.get_mut(&date).expect("pad info must contains the key");
                        target_date_pad_info.insert(pad.account.content, pad.pad);
                    }
                    BeancountOnlyDirective::Balance(balance) => {
                        let date = balance.date.naive_date();
                        let pad_account = pad_info.get_latest(&date).and_then(|it| it.get(&balance.account.content));

                        if let Some(pad_account) = pad_account {
                            // balance pad
                            ret.push(Spanned {
                                span,
                                data: Directive::BalancePad(BalancePad {
                                    date: balance.date,
                                    account: balance.account,
                                    amount: balance.amount,
                                    pad: pad_account.clone(),
                                    meta: balance.meta,
                                }),
                            });
                        } else {
                            //balance check
                            ret.push(Spanned {
                                span,
                                data: Directive::BalanceCheck(BalanceCheck {
                                    date: balance.date,
                                    account: balance.account,
                                    amount: balance.amount,
                                    meta: balance.meta,
                                }),
                            });
                        }
                    }
                },
            }
        }
        Ok(ret)
    }

    fn export(&self, directive: Spanned<Directive>) -> Self::Carrier {
        let zhang_data_type = ZhangDataType {};
        let directive = convert_datetime_to_date(directive);

        let Spanned { data, span } = directive;
        match data {
            Directive::BalanceCheck(check) => BalanceDirective {
                date: check.date,
                account: check.account,
                amount: check.amount,

                meta: check.meta,
            }
            .bc_to_string(),
            Directive::BalancePad(pad) => {
                let balance_date = pad.date.naive_date();
                let pad_date = balance_date.pred_opt().unwrap_or(balance_date);
                let pad_directive = PadDirective {
                    date: Date::Date(pad_date),
                    account: pad.account.clone(),
                    pad: pad.pad,
                    meta: Meta::default(),
                };
                let balance_directive = BalanceDirective {
                    date: pad.date,
                    account: pad.account,
                    amount: pad.amount,

                    meta: pad.meta,
                };
                [pad_directive.bc_to_string(), balance_directive.bc_to_string()].join("\n")
            }
            Directive::Budget(budget) => zhang_data_type.export(Spanned::new(
                Directive::Custom(Custom {
                    date: budget.date,
                    custom_type: ZhangString::unquote("budget"),
                    values: vec![
                        StringOrAccount::String(ZhangString::unquote(budget.name)),
                        StringOrAccount::String(ZhangString::unquote(budget.commodity)),
                    ],
                    meta: budget.meta,
                }),
                span,
            )),
            Directive::BudgetAdd(budget) => zhang_data_type.export(Spanned::new(
                Directive::Custom(Custom {
                    date: budget.date,
                    custom_type: ZhangString::unquote("budget-add"),
                    values: vec![
                        StringOrAccount::String(ZhangString::unquote(budget.name)),
                        StringOrAccount::String(ZhangString::unquote(budget.amount.number.to_string())),
                        StringOrAccount::String(ZhangString::unquote(budget.amount.currency)),
                    ],
                    meta: budget.meta,
                }),
                span,
            )),
            Directive::BudgetTransfer(budget) => zhang_data_type.export(Spanned::new(
                Directive::Custom(Custom {
                    date: budget.date,
                    custom_type: ZhangString::unquote("budget-transfer"),
                    values: vec![
                        StringOrAccount::String(ZhangString::unquote(budget.from)),
                        StringOrAccount::String(ZhangString::unquote(budget.to)),
                        StringOrAccount::String(ZhangString::unquote(budget.amount.number.to_string())),
                        StringOrAccount::String(ZhangString::unquote(budget.amount.currency)),
                    ],
                    meta: budget.meta,
                }),
                span,
            )),
            Directive::BudgetClose(budget) => zhang_data_type.export(Spanned::new(
                Directive::Custom(Custom {
                    date: budget.date,
                    custom_type: ZhangString::unquote("budget-close"),
                    values: vec![StringOrAccount::String(ZhangString::unquote(budget.name))],
                    meta: budget.meta,
                }),
                span,
            )),
            _ => zhang_data_type.export(Spanned::new(data, span)),
        }
    }
}

trait BeancountOnlyExportable {
    fn bc_to_string(self) -> String;
}

impl BeancountOnlyExportable for BalanceDirective {
    fn bc_to_string(self) -> String {
        let line = [
            ZhangDataTypeExportable::export(self.date),
            "balance".to_string(),
            ZhangDataTypeExportable::export(self.account),
            ZhangDataTypeExportable::export(self.amount),
        ]
        .join(" ");
        append_meta(self.meta, line)
    }
}

impl BeancountOnlyExportable for PadDirective {
    fn bc_to_string(self) -> String {
        let line = [
            ZhangDataTypeExportable::export(self.date),
            "pad".to_string(),
            ZhangDataTypeExportable::export(self.account),
            ZhangDataTypeExportable::export(self.pad),
        ]
        .join(" ");
        append_meta(self.meta, line)
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
        Directive::Note(mut directive) => Directive::Note(convert_to_datetime!(directive)),
        Directive::Document(mut directive) => Directive::Document(convert_to_datetime!(directive)),
        Directive::Price(mut directive) => Directive::Price(convert_to_datetime!(directive)),
        Directive::Event(mut directive) => Directive::Event(convert_to_datetime!(directive)),
        Directive::Custom(mut directive) => Directive::Custom(convert_to_datetime!(directive)),
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
                Directive::Note(directive) => extract_time!(directive),
                Directive::Document(directive) => extract_time!(directive),
                Directive::Price(directive) => extract_time!(directive),
                Directive::Event(directive) => extract_time!(directive),
                Directive::Custom(directive) => extract_time!(directive),
                _ => {}
            },
            Either::Right(beancount_onyly_directive) => match beancount_onyly_directive {
                BeancountOnlyDirective::Pad(directive) => extract_time!(directive),
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
    use zhang_ast::{Account, BalanceCheck, BalancePad, Date, Directive, Meta, Open, SpanInfo, Spanned};
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
    fn should_transform_to_non_given_pad_directive() {
        let beancount_data_type = Beancount::default();
        let directives = beancount_data_type
            .transform(
                indoc! {r#"
                1970-01-01 pad Assets:BankAccount Equity:Open-Balances
            "#}
                .to_string(),
                None,
            )
            .unwrap();

        assert_eq!(directives.len(), 0);
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
                meta: Default::default(),
            })
        );
    }

    #[test]
    fn should_transform_to_balance_pad_directive_given_pad_and_balance_directive() {
        let beancount_data_type = Beancount::default();
        let mut directives = beancount_data_type
            .transform(
                indoc! {r#"
                1970-01-01 pad Assets:BankAccount Equity:Open-Balances
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
            Directive::BalancePad(BalancePad {
                date: Date::Date(NaiveDate::from_ymd_opt(1970, 1, 2).unwrap()),
                account: Account::from_str("Assets:BankAccount").unwrap(),
                amount: Amount::new(BigDecimal::from(100i32), "CNY"),
                pad: Account::from_str("Equity:Open-Balances").unwrap(),
                meta: Default::default(),
            })
        );
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
