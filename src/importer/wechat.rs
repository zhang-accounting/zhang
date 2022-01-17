use std::collections::{HashMap, HashSet};
use std::fs::{File, read};
use std::io::{BufRead, BufReader};
use std::ops::Neg;
use std::path::PathBuf;
use std::str::FromStr;

use bigdecimal::BigDecimal;
use chrono::{DateTime, FixedOffset, NaiveDate, NaiveDateTime, TimeZone};
use log::warn;
use serde::Deserialize;

use crate::account::Account;
use crate::amount::Amount;
use crate::data::{Posting, Transaction};
use crate::error::AvaroResult;
use crate::models::Flag;

static CURRENCY: &str = "CNY";
static COMMENT_STR: &str = "收款方备注:二维码收款";

#[derive(Debug, Deserialize)]
struct Config {
    wechat_account: String,
    pay_ways: HashMap<String, String>,
    payees: HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
struct Record {
    #[serde(rename = "交易时间")]
    datetime: String,
    #[serde(rename = "交易类型")]
    txn_type: String,
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
    #[serde(rename = "当前状态")]
    status: String,
    #[serde(rename = "交易单号")]
    txn_no: String,
    #[serde(rename = "商户单号")]
    payee_no: String,
    #[serde(rename = "备注")]
    comment: String,
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
        let option = self.amount.strip_prefix("¥").unwrap_or(&self.amount);
        let result = BigDecimal::from_str(option).unwrap();
        let value = if self.is_income() {
            result
        } else {
            result.neg()
        };
        Amount::new(value, CURRENCY)
    }
    fn payee(&self) -> &str {
        &self.payee.trim()
    }
    fn narration(&self) -> &str {
        if let Some(content) = self.narration.strip_prefix(COMMENT_STR) {
            content.trim()
        } else if self.narration == "/" {
            ""
        } else {
            &self.narration.trim()
        }
    }
    fn transaction_no(&self) -> &str {
        self.txn_no.trim()
    }
    fn payee_no(&self) -> &str {
        self.payee_no.trim()
    }
}

pub fn run(file: PathBuf, config: PathBuf) -> AvaroResult<()> {
    let config_content = std::fs::read_to_string(config)?;
    let config: Config = toml::from_str(&config_content)?;

    let file1 = File::open(file)?;
    let mut reader = BufReader::new(file1);
    let mut string_buffer = String::new();
    for i in 0..=15 {
        reader.read_line(&mut string_buffer);
    }
    let mut reader1 = csv::Reader::from_reader(reader);

    for result in reader1.deserialize().skip(16) {
        let result: Record = result?;

        let pay_way = config
            .pay_ways
            .get(&result.pay_type)
            .map(|it| it.to_string())
            .unwrap_or_else(|| {
                warn!("pay way [{}] is not configurated", &result.pay_type);
                "Account:FixMe".to_string()
            });
        let pay_way = Account::from_str(&pay_way)?;
        let payee = config
            .payees
            .get(result.payee())
            .map(|it| it.to_string())
            .unwrap_or_else(|| {
                warn!("payee [{}] is not configurated", result.payee());
                "Expenses:FixMe".to_string()
            });
        let payee = Account::from_str(&payee)?;

        let mut meta = HashMap::new();
        meta.insert("transaction_no".to_string(), result.transaction_no().to_string());
        meta.insert("payee_no".to_string(), result.payee_no().to_string());

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
            date: result.datetime().naive_local(),
            flag: Some(Flag::Okay),
            payee: Some(result.payee().to_string()),
            narration: Some(result.narration().to_string()),
            tags: HashSet::new(),
            links: HashSet::new(),
            postings: postings,
            meta: meta,
        };

    }

    Ok(())
}
