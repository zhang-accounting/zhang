use crate::core::account::Account;
use crate::core::amount::Amount;
use crate::core::data::{BalanceCheck, Date, Transaction};
use crate::core::inventory::Currency;
use crate::core::ledger::{AccountInfo, AccountSnapshot, AccountStatus, CurrencyInfo};
use crate::server::LedgerState;
use async_graphql::{Context, EmptyMutation, EmptySubscription, Interface, Object, Schema, Union};
use bigdecimal::{BigDecimal, Zero};
use chrono::NaiveDate;
use itertools::Itertools;
use std::str::FromStr;

pub type LedgerSchema = Schema<QueryRoot, EmptyMutation, EmptySubscription>;

pub struct QueryRoot;

#[Object]
impl QueryRoot {
    async fn prices(&self, ctx: &Context<'_>) -> Vec<i32> {
        todo!()
    }
    async fn currencies(&self, ctx: &Context<'_>) -> Vec<CurrencyDto> {
        let ledger_stage = ctx.data_unchecked::<LedgerState>().read().await;
        ledger_stage
            .currencies
            .clone()
            .into_iter()
            .map(|(_, info)| CurrencyDto(info))
            .collect_vec()
    }
    async fn currency(&self, ctx: &Context<'_>, name: String) -> Option<CurrencyDto> {
        let ledger_stage = ctx.data_unchecked::<LedgerState>().read().await;
        ledger_stage
            .currencies
            .get(&name)
            .map(|info| CurrencyDto(info.clone()))
    }

    async fn accounts(&self, ctx: &Context<'_>) -> Vec<AccountDto> {
        let ledger_stage = ctx.data_unchecked::<LedgerState>().read().await;
        ledger_stage
            .accounts
            .clone()
            .into_iter()
            .map(|(name, info)| AccountDto { name, info })
            .collect_vec()
    }
    async fn account(&self, ctx: &Context<'_>) -> Vec<AccountDto> {
        let ledger_stage = ctx.data_unchecked::<LedgerState>().read().await;
        ledger_stage
            .accounts
            .clone()
            .into_iter()
            .map(|(name, info)| AccountDto { name, info })
            .collect_vec()
    }

    async fn documents(&self, ctx: &Context<'_>) -> Vec<AccountDto> {
        todo!()
    }
    async fn document(&self, ctx: &Context<'_>) -> Vec<AccountDto> {
        todo!()
    }

    async fn journals(&self, ctx: &Context<'_>) -> Vec<JournalDto> {
        vec![
            JournalDto::Transaction(TransactionDto(Transaction {
                date: Date::Date(NaiveDate::from_ymd(1970, 1, 1)),
                flag: None,
                payee: None,
                narration: None,
                tags: Default::default(),
                links: Default::default(),
                postings: vec![],
                meta: Default::default(),
            })),
            JournalDto::BalanceCheck(BalanceCheckDto(BalanceCheck {
                date: Date::Date(NaiveDate::from_ymd(1970, 1, 1)),
                account: Account::from_str("Assets::Hello").unwrap(),
                amount: Amount::new(BigDecimal::zero(), "CNY"),
                tolerance: None,
                diff_amount: None,
                meta: Default::default(),
            })),
        ]
    }
}

pub struct AccountDto {
    name: String,
    info: AccountInfo,
}

#[Object]
impl AccountDto {
    async fn name(&self, ctx: &Context<'_>) -> String {
        self.name.to_string()
    }
    async fn status(&self, ctx: &Context<'_>) -> AccountStatus {
        self.info.status.clone()
    }
    async fn snapshot(&self, ctx: &Context<'_>) -> AccountSnapshot {
        let ledger_stage = ctx.data_unchecked::<LedgerState>().read().await;
        ledger_stage
            .snapshot
            .get(&self.name)
            .cloned()
            .unwrap_or_else(AccountSnapshot::default)
    }
    async fn currencies(&self, ctx: &Context<'_>) -> Vec<CurrencyDto> {
        let ledger_stage = ctx.data_unchecked::<LedgerState>().read().await;
        ledger_stage
            .currencies
            .clone()
            .into_iter()
            .filter(|(name, _)| self.info.currencies.contains(name))
            .map(|(_, info)| CurrencyDto(info))
            .collect_vec()
    }
}

pub struct CurrencyDto(CurrencyInfo);

#[Object]
impl CurrencyDto {
    async fn name(&self) -> String {
        self.0.commodity.currency.to_string()
    }

    async fn precision(&self) -> i32 {
        self.0
            .commodity
            .meta
            .get("precision")
            .map(|it| it.clone().to_plain_string())
            .map(|it| it.parse::<i32>().unwrap_or(2))
            .unwrap_or(2)
    }
}

#[derive(Interface)]
#[graphql(field(name = "a", type = "String"))]
pub enum JournalDto {
    Transaction(TransactionDto),
    BalanceCheck(BalanceCheckDto),
}

pub struct TransactionDto(Transaction);

#[Object]
impl TransactionDto {
    async fn a(&self) -> String {
        "a".to_string()
    }
}

pub struct BalanceCheckDto(BalanceCheck);

#[Object]
impl BalanceCheckDto {
    async fn a(&self) -> String {
        "a".to_string()
    }
    async fn b(&self) -> String {
        "b".to_string()
    }
}

//
// pub struct AccountSnapshotDto(AccountSnapshot);
//
// #[Object]
// impl AccountSnapshot {
//     async fn
// }
