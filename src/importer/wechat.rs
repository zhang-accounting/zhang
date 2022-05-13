use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::ops::Neg;
use std::path::PathBuf;
use std::str::FromStr;

use bigdecimal::BigDecimal;
use chrono::{DateTime, FixedOffset, NaiveDateTime, TimeZone};
use log::{error, warn};
use serde::{Deserialize, Serialize};

use crate::core::account::Account;
use crate::core::amount::Amount;
use crate::core::data::{Date, Posting, Transaction};
use crate::core::models::{Flag, ZhangString};
use crate::core::utils::multi_value_map::MultiValueMap;
use crate::error::ZhangResult;
use crate::target::ZhangTarget;
use itertools::Itertools;

static CURRENCY: &str = "CNY";
static COMMENT_STR: &str = "收款方备注:二维码收款";

#[derive(Debug, Deserialize, Serialize)]
struct Config {
    forbid_unknown_payee: bool,
    store_unknown_payee: bool,
    wechat_account: String,
    pay_ways: HashMap<String, String>,
    payees: HashMap<String, String>,
    #[serde(default)]
    unknown_payees: HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
struct Record {
    #[serde(rename = "交易时间")]
    datetime: String,
    // #[serde(rename = "交易类型")]
    // txn_type: String,
    #[serde(rename = "交易对方")]
    payee: String,
    #[serde(rename = "商品")]
    narration: String,
    #[serde(rename = "收/支")]
    is_income: String,
    #[serde(rename = "金额(元)")]
    amount: String,
    #[serde(rename = "支付方式")]
    pay_type: String,
    // #[serde(rename = "当前状态")]
    // status: String,
    #[serde(rename = "交易单号")]
    txn_no: String,
    #[serde(rename = "商户单号")]
    payee_no: String,
    // #[serde(rename = "备注")]
    // comment: String,
}

impl Record {
    fn datetime(&self) -> DateTime<FixedOffset> {
        let time = NaiveDateTime::parse_from_str(&self.datetime, "%Y-%m-%d %H:%M:%S").unwrap();
        let offset = FixedOffset::east(60 * 60 * 8);
        offset.from_local_datetime(&time).unwrap()
    }
    fn is_income(&self) -> bool {
        !matches!(self.is_income.as_str(), "支出" | "/")
    }
    fn amount(&self) -> Amount {
        let option = self.amount.strip_prefix('¥').unwrap_or(&self.amount);
        let result = BigDecimal::from_str(option).unwrap();
        let value = if self.is_income() { result } else { result.neg() };
        Amount::new(value, CURRENCY)
    }
    fn payee(&self) -> Option<&str> {
        let x = self.payee.trim();
        if x.is_empty() {
            None
        } else {
            Some(x)
        }
    }
    fn narration(&self) -> Option<&str> {
        if let Some(content) = self.narration.strip_prefix(COMMENT_STR) {
            Some(content.trim())
        } else if self.narration == "/" {
            None
        } else {
            Some(self.narration.trim())
        }
    }
    fn transaction_no(&self) -> Option<&str> {
        let x = self.txn_no.trim();
        if x.is_empty() || x.eq("/") {
            None
        } else {
            Some(x)
        }
    }
    fn payee_no(&self) -> Option<&str> {
        let x = self.payee_no.trim();
        if x.is_empty() || x.eq("/") {
            None
        } else {
            Some(x)
        }
    }
}

pub fn run(file: PathBuf, config: PathBuf) -> ZhangResult<()> {
    let config_content = std::fs::read_to_string(&config)?;
    let mut loaded_config: Config = toml::from_str(&config_content)?;

    let file1 = File::open(file)?;
    let mut reader = BufReader::new(file1);
    let mut string_buffer = String::new();
    for _ in 0..=15 {
        reader.read_line(&mut string_buffer)?;
    }
    let mut reader1 = csv::Reader::from_reader(reader);

    let mut unknown_payees = HashSet::new();

    let mut ret = vec![];
    for result in reader1.deserialize() {
        let result: Record = result?;

        let payee = result.payee().unwrap();
        let pay_way = {
            let pay_type = result.pay_type.as_str();
            let option = loaded_config.pay_ways.get(pay_type).map(|it| it.to_string());

            if let Some(value) = option {
                value
            } else {
                unknown_payees.insert(pay_type.to_string());
                "Expenses:FixMe".to_string()
            }
        };
        let pay_way = Account::from_str(&pay_way)?;
        let payee = {
            let option = loaded_config.payees.get(payee).map(|it| it.to_string());
            if let Some(value) = option {
                value
            } else {
                unknown_payees.insert(payee.to_string());
                "Expenses:FixMe".to_string()
            }
        };
        let payee = Account::from_str(&payee)?;

        let mut meta = MultiValueMap::default();
        if let Some(txn_no) = result.transaction_no() {
            meta.insert(
                "transaction_no".to_string(),
                ZhangString::QuoteString(txn_no.to_string()),
            );
        }
        if let Some(payee_no) = result.payee_no() {
            meta.insert("payee_no".to_string(), ZhangString::QuoteString(payee_no.to_string()));
        }

        let postings = vec![
            Posting {
                flag: None,
                account: pay_way,
                units: Some(result.amount()),
                cost: None,
                price: None,
                meta: Default::default(),
            },
            Posting {
                flag: None,
                account: payee,
                units: Some(result.amount().neg()),
                cost: None,
                price: None,
                meta: Default::default(),
            },
        ];
        let transaction = Transaction {
            date: Date::Datetime(result.datetime().naive_local()),
            flag: Some(Flag::Okay),
            payee: result.payee().map(|it| ZhangString::QuoteString(it.to_string())),
            narration: result.narration().map(|it| ZhangString::QuoteString(it.to_string())),
            tags: HashSet::new(),
            links: HashSet::new(),
            postings,
            meta,
        };
        ret.push(transaction);
    }
    if !unknown_payees.is_empty() {
        if loaded_config.forbid_unknown_payee {
            error!("payee [{}] is not configurated", unknown_payees.iter().join(","));
        } else {
            warn!("payee [{}] is not configurated", unknown_payees.iter().join(","));
        }
    }
    if loaded_config.store_unknown_payee {
        for x in unknown_payees {
            loaded_config.unknown_payees.insert(x, "Expenses:FixMe".to_string());
        }
        let result1 = toml::to_string(&loaded_config)?;
        std::fs::write(&config, result1)?;
    }
    ret.reverse();
    for trx in ret.into_iter() {
        println!("{}\n", trx.to_target());
    }

    Ok(())
}
