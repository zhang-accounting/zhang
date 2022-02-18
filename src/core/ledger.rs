use crate::core::account::Account;
use crate::core::inventory::Currency;
use crate::core::models::Directive;
use crate::error::{ZhangError, ZhangResult};
use crate::parse_zhang;
use itertools::Itertools;
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

#[derive(Debug)]
pub enum AccountStatus {
    Open,
    Close,
}
#[derive(Debug)]
pub struct AccountInfo {
    currencies: HashSet<Currency>,
    status: AccountStatus,
}

#[derive(Debug)]
pub struct Ledger {
    pub(crate) directives: Vec<Directive>,
    pub(crate) accounts: HashMap<Account, AccountInfo>,
}

impl Ledger {
    pub fn load(entry: PathBuf) -> ZhangResult<Ledger> {
        let content = std::fs::read_to_string(entry)?;
        let directives =
            parse_zhang(&content).map_err(|it| ZhangError::PestError(it.to_string()))?;

        let directives = Ledger::sort_directives_datetime(directives);
        // for directive in directives {
        //     match directive {
        //         Directive::Open(_) => {}
        //         Directive::Close(_) => {}
        //         Directive::Commodity(_) => {}
        //         Directive::Transaction(_) => {}
        //         Directive::Balance(_) => {}
        //         Directive::Pad(_) => {}
        //         Directive::Note(_) => {}
        //         Directive::Document(_) => {}
        //         Directive::Price(_) => {}
        //         Directive::Event(_) => {}
        //         Directive::Custom(_) => {}
        //         Directive::Option(_) => {}
        //         Directive::Plugin(_) => {}
        //         Directive::Include(_) => {}
        //         Directive::Comment(_) => {}
        //     }
        // }
        Ok(Self {
            directives,
            accounts: HashMap::default(),
        })
    }

    fn sort_directives_datetime(mut directives: Vec<Directive>) -> Vec<Directive> {
        directives.sort_by(|a, b| match (a.datetime(), b.datetime()) {
            (Some(a_datetime), Some(b_datetime)) => a_datetime.cmp(&b_datetime),
            _ => Ordering::Greater,
        });
        directives
    }

    pub fn apply(mut self, applier: impl Fn(Directive) -> Directive) -> Self {
        let vec = self.directives.into_iter().map(applier).collect_vec();
        self.directives = vec;
        self
    }
}

#[cfg(test)]
mod test {
    mod sort_directive_datetime {
        use crate::core::account::Account;
        use crate::core::data::{Date, Open, Options};
        use crate::core::ledger::Ledger;
        use crate::core::models::{Directive, ZhangString};
        use chrono::NaiveDate;
        use std::str::FromStr;

        #[test]
        fn should_keep_order_given_two_none_datetime() {
            let original = vec![
                Directive::Option(Options {
                    key: ZhangString::QuoteString("title".to_string()),
                    value: ZhangString::QuoteString("Title".to_string()),
                }),
                Directive::Option(Options {
                    key: ZhangString::QuoteString("description".to_string()),
                    value: ZhangString::QuoteString("Description".to_string()),
                }),
            ];
            let sorted = Ledger::sort_directives_datetime(original);
            assert_eq!(
                vec![
                    Directive::Option(Options {
                        key: ZhangString::QuoteString("title".to_string()),
                        value: ZhangString::QuoteString("Title".to_string()),
                    }),
                    Directive::Option(Options {
                        key: ZhangString::QuoteString("description".to_string()),
                        value: ZhangString::QuoteString("Description".to_string()),
                    }),
                ],
                sorted
            )
        }

        #[test]
        fn should_keep_original_order_given_none_datetime_and_datetime() {
            let original = vec![
                Directive::Open(Open {
                    date: Date::Date(NaiveDate::from_ymd(1970, 1, 1)),
                    account: Account::from_str("Assets:Hello").unwrap(),
                    commodities: vec![],
                    meta: Default::default(),
                }),
                Directive::Option(Options {
                    key: ZhangString::QuoteString("description".to_string()),
                    value: ZhangString::QuoteString("Description".to_string()),
                }),
            ];
            let sorted = Ledger::sort_directives_datetime(original);
            assert_eq!(
                vec![
                    Directive::Open(Open {
                        date: Date::Date(NaiveDate::from_ymd(1970, 1, 1)),
                        account: Account::from_str("Assets:Hello").unwrap(),
                        commodities: vec![],
                        meta: Default::default(),
                    }),
                    Directive::Option(Options {
                        key: ZhangString::QuoteString("description".to_string()),
                        value: ZhangString::QuoteString("Description".to_string()),
                    }),
                ],
                sorted
            );
            let original = vec![
                Directive::Option(Options {
                    key: ZhangString::QuoteString("description".to_string()),
                    value: ZhangString::QuoteString("Description".to_string()),
                }),
                Directive::Open(Open {
                    date: Date::Date(NaiveDate::from_ymd(1970, 1, 1)),
                    account: Account::from_str("Assets:Hello").unwrap(),
                    commodities: vec![],
                    meta: Default::default(),
                }),
            ];
            let sorted = Ledger::sort_directives_datetime(original);
            assert_eq!(
                vec![
                    Directive::Option(Options {
                        key: ZhangString::QuoteString("description".to_string()),
                        value: ZhangString::QuoteString("Description".to_string()),
                    }),
                    Directive::Open(Open {
                        date: Date::Date(NaiveDate::from_ymd(1970, 1, 1)),
                        account: Account::from_str("Assets:Hello").unwrap(),
                        commodities: vec![],
                        meta: Default::default(),
                    }),
                ],
                sorted
            )
        }

        #[test]
        fn should_order_by_datetime() {
            let original = vec![
                Directive::Open(Open {
                    date: Date::Date(NaiveDate::from_ymd(1970, 1, 1)),
                    account: Account::from_str("Assets:Hello").unwrap(),
                    commodities: vec![],
                    meta: Default::default(),
                }),
                Directive::Open(Open {
                    date: Date::Date(NaiveDate::from_ymd(1970, 2, 1)),
                    account: Account::from_str("Assets:Hello").unwrap(),
                    commodities: vec![],
                    meta: Default::default(),
                }),
            ];
            let sorted = Ledger::sort_directives_datetime(original);
            assert_eq!(
                vec![
                    Directive::Open(Open {
                        date: Date::Date(NaiveDate::from_ymd(1970, 1, 1)),
                        account: Account::from_str("Assets:Hello").unwrap(),
                        commodities: vec![],
                        meta: Default::default(),
                    }),
                    Directive::Open(Open {
                        date: Date::Date(NaiveDate::from_ymd(1970, 2, 1)),
                        account: Account::from_str("Assets:Hello").unwrap(),
                        commodities: vec![],
                        meta: Default::default(),
                    })
                ],
                sorted
            );
            let original = vec![
                Directive::Open(Open {
                    date: Date::Date(NaiveDate::from_ymd(1970, 2, 1)),
                    account: Account::from_str("Assets:Hello").unwrap(),
                    commodities: vec![],
                    meta: Default::default(),
                }),
                Directive::Open(Open {
                    date: Date::Date(NaiveDate::from_ymd(1970, 1, 1)),
                    account: Account::from_str("Assets:Hello").unwrap(),
                    commodities: vec![],
                    meta: Default::default(),
                }),
            ];
            let sorted = Ledger::sort_directives_datetime(original);
            assert_eq!(
                vec![
                    Directive::Open(Open {
                        date: Date::Date(NaiveDate::from_ymd(1970, 1, 1)),
                        account: Account::from_str("Assets:Hello").unwrap(),
                        commodities: vec![],
                        meta: Default::default(),
                    }),
                    Directive::Open(Open {
                        date: Date::Date(NaiveDate::from_ymd(1970, 2, 1)),
                        account: Account::from_str("Assets:Hello").unwrap(),
                        commodities: vec![],
                        meta: Default::default(),
                    })
                ],
                sorted
            )
        }
    }
}
