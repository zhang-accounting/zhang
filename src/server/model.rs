use crate::core::account::Account;
use crate::core::amount::Amount;
use crate::core::data::{Balance, BalanceCheck, BalancePad, Date, Transaction};
use crate::core::inventory::Currency;
use crate::core::ledger::{AccountInfo, AccountSnapshot, AccountStatus, CurrencyInfo};
use crate::core::models::Directive;
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
        let ledger_stage = ctx.data_unchecked::<LedgerState>().read().await;
        ledger_stage
            .directives
            .iter()
            .filter_map(|directive| match directive {
                Directive::Transaction(trx) => {
                    Some(JournalDto::Transaction(TransactionDto(trx.clone())))
                }
                Directive::Balance(balance) => match balance {
                    Balance::BalanceCheck(check) => {
                        Some(JournalDto::BalanceCheck(BalanceCheckDto(check.clone())))
                    }
                    Balance::BalancePad(pad) => {
                        Some(JournalDto::BalancePad(BalancePadDto(pad.clone())))
                    }
                },
                _ => None,
            })
            .rev()
            .collect_vec()
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
#[graphql(field(name = "date", type = "String"))]
pub enum JournalDto {
    Transaction(TransactionDto),
    BalanceCheck(BalanceCheckDto),
    BalancePad(BalancePadDto),
}

pub struct TransactionDto(Transaction);

#[Object]
impl TransactionDto {
    async fn date(&self) -> String {
        self.0.date.naive_date().to_string()
    }
    async fn payee(&self) -> Option<String> {
        self.0.payee.clone().map(|it| it.to_plain_string())
    }
    async fn narration(&self) -> Option<String> {
        self.0.narration.clone().map(|it| it.to_plain_string())
    }
}

pub struct BalanceCheckDto(BalanceCheck);

#[Object]
impl BalanceCheckDto {
    async fn date(&self) -> String {
        self.0.date.naive_date().to_string()
    }
}

pub struct BalancePadDto(BalancePad);

#[Object]
impl BalancePadDto {
    async fn date(&self) -> String {
        self.0.date.naive_date().to_string()
    }
}

//
// pub struct AccountSnapshotDto(AccountSnapshot);
//
// #[Object]
// impl AccountSnapshot {
//     async fn
// }
