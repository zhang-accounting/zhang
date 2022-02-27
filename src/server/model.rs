use crate::core::inventory::Currency;
use crate::core::ledger::{AccountInfo, AccountSnapshot, AccountStatus, CurrencyInfo};
use crate::server::LedgerState;
use async_graphql::{Context, EmptyMutation, EmptySubscription, Object, Schema};
use itertools::Itertools;

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

    async fn journals(&self, ctx: &Context<'_>) -> Vec<i32>{
        todo!()
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

//
// pub struct AccountSnapshotDto(AccountSnapshot);
//
// #[Object]
// impl AccountSnapshot {
//     async fn
// }
