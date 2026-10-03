use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use itertools::{Either, Itertools};
use zhang_ast::*;
use zhang_core::data_type::text::exporter::{append_meta_as, ZhangDataTypeExportable};
use zhang_core::data_type::DataType;
use zhang_core::utils::string_::QuoteStyle;
use zhang_core::{ZhangError, ZhangResult};

use crate::directives::{BalanceDirective, BeancountDirective, BeancountOnlyDirective, PadDirective};
use crate::parser::{parse, parse_time};

#[allow(clippy::upper_case_acronyms)]
#[allow(clippy::type_complexity)]
pub mod parser;

pub mod directives;

#[derive(Clone, Default)]
pub struct Beancount {}

/// a directive of a beancount file, before its pads are paired with the balances they serve
enum Item {
    Directive(Spanned<Directive>),
    Pad(PadDirective),
    Balance(Spanned<BalanceDirective>),
}

/// The pad account serving each balance of `items` (by index), paired as beancount's `pad` plugin pairs them.
///
/// The pads and balances are taken in beancount's order: by date, a day's balances before its pads (a balance
/// on the day of a pad is checked before it), then in file order. A pad serves the first balance of its account
/// in each currency after it, until the account's next pad. The `balance ... with pad` a served balance becomes
/// brings the account to the asserted amount from its balance there, as beancount's padding transaction does.
fn pads_serving_balances(items: &[Item]) -> HashMap<usize, Account> {
    let mut order = items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| match item {
            Item::Balance(balance) => Some((balance.data.date.naive_date(), 0, index)),
            Item::Pad(pad) => Some((pad.date.naive_date(), 1, index)),
            Item::Directive(_) => None,
        })
        .collect_vec();
    order.sort();

    // account -> (the account it pads from, the currencies it served already)
    let mut active: HashMap<&str, (&Account, HashSet<&str>)> = HashMap::new();
    let mut served = HashMap::new();
    for (_, _, index) in order {
        match &items[index] {
            Item::Pad(pad) => {
                active.insert(pad.account.name(), (&pad.pad, HashSet::new()));
            }
            Item::Balance(balance) => {
                if let Some((pad_account, currencies)) = active.get_mut(balance.data.account.name()) {
                    if currencies.insert(balance.data.amount.commodity.as_str()) {
                        served.insert(index, (*pad_account).clone());
                    }
                }
            }
            Item::Directive(_) => {}
        }
    }
    served
}

impl DataType for Beancount {
    type Carrier = String;

    fn transform(&self, raw_data: Self::Carrier, source: Option<String>) -> ZhangResult<Vec<Spanned<Directive>>> {
        let path = source.clone().map(PathBuf::from);
        let directives = parse(&raw_data, path).map_err(|it| ZhangError::PestError {
            path: source.unwrap_or_default(),
            msg: it.to_string(),
        })?;

        let mut items = vec![];
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
                    items.push(Item::Directive(Spanned { span, data: zhang_directive }));
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
                    BeancountOnlyDirective::Pad(pad) => items.push(Item::Pad(pad)),
                    BeancountOnlyDirective::Balance(balance) => items.push(Item::Balance(Spanned { span, data: balance })),
                },
            }
        }

        let served = pads_serving_balances(&items);
        let ret = items
            .into_iter()
            .enumerate()
            .filter_map(|(index, item)| match item {
                Item::Directive(directive) => Some(directive),
                // a pad becomes the `balance ... with pad` of the balance it serves
                Item::Pad(_) => None,
                Item::Balance(Spanned { span, data: balance }) => Some(Spanned {
                    span,
                    data: match served.get(&index) {
                        Some(pad_account) => Directive::BalancePad(BalancePad {
                            date: balance.date,
                            account: balance.account,
                            amount: balance.amount,
                            pad: pad_account.clone(),
                            meta: balance.meta,
                        }),
                        None => Directive::BalanceCheck(BalanceCheck {
                            date: balance.date,
                            account: balance.account,
                            amount: balance.amount,
                            tolerance: balance.tolerance,
                            meta: balance.meta,
                        }),
                    },
                }),
            })
            .collect();
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
                    tolerance: None,

                    meta: pad.meta,
                };
                [pad_directive.bc_to_string(), balance_directive.bc_to_string()].join("\n")
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

impl BeancountOnlyExportable for PadDirective {
    fn bc_to_string(self) -> String {
        let line = [
            ZhangDataTypeExportable::export(self.date),
            "pad".to_string(),
            ZhangDataTypeExportable::export(self.account),
            ZhangDataTypeExportable::export(self.pad),
        ]
        .join(" ");
        append_meta_as(self.meta, line, QuoteStyle::Beancount)
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
                Directive::Note(directive) => extract_time!(directive),
                Directive::Document(directive) => extract_time!(directive),
                Directive::Price(directive) => extract_time!(directive),
                Directive::Event(directive) => extract_time!(directive),
                Directive::Custom(directive) => extract_time!(directive),
                Directive::Query(directive) => extract_time!(directive),
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
                tolerance: None,
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

    /// `(kind, account, currency)` of the balances a ledger turns into: `pad` for a `balance ... with pad`
    fn balances(content: &str) -> Vec<(&'static str, String, String)> {
        Beancount::default()
            .transform(content.to_string(), None)
            .unwrap()
            .into_iter()
            .filter_map(|directive| match directive.data {
                Directive::BalancePad(pad) => Some(("pad", pad.account.content, pad.amount.commodity)),
                Directive::BalanceCheck(check) => Some(("check", check.account.content, check.amount.commodity)),
                _ => None,
            })
            .collect()
    }

    fn balance(kind: &'static str, account: &str, currency: &str) -> (&'static str, String, String) {
        (kind, account.to_owned(), currency.to_owned())
    }

    #[test]
    fn should_pad_the_next_balance_of_the_account_in_each_currency() {
        let balances = balances(indoc! {r#"
            2024-01-01 pad Assets:Wallet Equity:Open
            2024-01-02 balance Assets:Wallet 300 CNY
            2024-01-02 balance Assets:Wallet 150 USD
            2024-01-02 balance Assets:Other 1 CNY
            2024-01-03 balance Assets:Wallet 300 CNY
        "#});
        assert_eq!(
            balances,
            vec![
                balance("pad", "Assets:Wallet", "CNY"),
                balance("pad", "Assets:Wallet", "USD"),
                balance("check", "Assets:Other", "CNY"),
                balance("check", "Assets:Wallet", "CNY"),
            ]
        );
    }

    #[test]
    fn should_not_pad_a_balance_on_the_day_of_the_pad() {
        // beancount orders a day's balances before its pads
        let balances = balances(indoc! {r#"
            2017-12-01 pad Assets:A Equity:Open
            2017-12-01 balance Assets:A 0.10 CNY
            2017-12-02 balance Assets:A 0.10 CNY
        "#});
        assert_eq!(balances, vec![balance("check", "Assets:A", "CNY"), balance("pad", "Assets:A", "CNY")]);
    }

    #[test]
    fn should_pair_pads_by_date_and_account_whatever_their_place_in_the_file() {
        let balances = balances(indoc! {r#"
            2024-01-06 balance Assets:Bank 1000 CNY
            2024-01-01 pad Assets:Bank Equity:Open
            2024-01-02 pad Assets:Cash Equity:Open
            2024-01-05 balance Assets:Cash 20 CNY
            2024-01-08 balance Assets:Bank 900 CNY
            2024-01-08 pad Assets:Bank Equity:Open
            2024-01-10 balance Assets:Bank 1500 CNY
        "#});
        assert_eq!(
            balances,
            vec![
                balance("pad", "Assets:Bank", "CNY"),
                balance("pad", "Assets:Cash", "CNY"),
                balance("check", "Assets:Bank", "CNY"),
                balance("pad", "Assets:Bank", "CNY"),
            ]
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
