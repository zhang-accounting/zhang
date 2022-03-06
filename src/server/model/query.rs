use crate::core::amount::Amount;
use crate::core::data::{Balance, BalanceCheck, BalancePad, Transaction, TxnPosting};
use crate::core::ledger::{AccountInfo, AccountSnapshot, AccountStatus, CurrencyInfo};
use crate::core::models::Directive;
use crate::server::LedgerState;
use async_graphql::{Context, EmptyMutation, EmptySubscription, Interface, Object, Schema};
use itertools::Itertools;
use std::path::PathBuf;

pub struct QueryRoot;

#[Object]
impl QueryRoot {
    async fn entries(&self, ctx: &Context<'_>) -> Vec<FileEntryDto> {
        let ledger_stage = ctx.data_unchecked::<LedgerState>().read().await;
        ledger_stage
            .visited_files
            .iter()
            .map(|it| FileEntryDto(it.clone()))
            .collect_vec()
    }
    async fn entry(&self, ctx: &Context<'_>, name: String) -> Option<FileEntryDto> {
        let ledger_stage = ctx.data_unchecked::<LedgerState>().read().await;
        ledger_stage
            .visited_files
            .iter()
            .find(|it| {
                it.to_str()
                    .map(|path_str| name.eq(path_str))
                    .unwrap_or(false)
            })
            .map(|it| FileEntryDto(it.clone()))
    }
    async fn statistic(&self, _month_offset: i32) -> StatisticDto {
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

    async fn documents(&self) -> Vec<AccountDto> {
        todo!()
    }
    async fn document(&self) -> Vec<AccountDto> {
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
    async fn name(&self) -> String {
        self.name.to_string()
    }
    async fn status(&self) -> AccountStatus {
        self.info.status
    }
    async fn snapshot(&self, ctx: &Context<'_>) -> AccountSnapshot {
        let ledger_stage = ctx.data_unchecked::<LedgerState>().read().await;
        ledger_stage
            .snapshot
            .get(&self.name)
            .cloned()
            .unwrap_or_default()
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
    async fn postings<'a>(&'a self) -> Vec<PostingDto<'a>> {
        self.0
            .txn_postings()
            .into_iter()
            .map(PostingDto)
            .collect_vec()
    }
}

pub struct BalanceCheckDto(BalanceCheck);

#[Object]
impl BalanceCheckDto {
    async fn date(&self) -> String {
        self.0.date.naive_date().to_string()
    }
    async fn account(&self, ctx: &Context<'_>) -> Option<AccountDto> {
        let ledger_stage = ctx.data_unchecked::<LedgerState>().read().await;
        ledger_stage
            .accounts
            .get(self.0.account.name())
            .map(|info| AccountDto {
                name: self.0.account.name().to_string(),
                info: info.clone(),
            })
    }
    async fn balance_amount(&self) -> AmountDto {
        AmountDto(self.0.amount.clone())
    }
    async fn current_amount(&self) -> AmountDto {
        AmountDto(
            self.0
                .current_amount
                .clone()
                .expect("cannot get current amount"),
        )
    }
    async fn distance(&self) -> Option<AmountDto> {
        self.0.distance.clone().map(AmountDto)
    }
    async fn is_balanced(&self) -> bool {
        self.0.distance.is_none()
    }
}

pub struct BalancePadDto(BalancePad);

#[Object]
impl BalancePadDto {
    async fn date(&self) -> String {
        self.0.date.naive_date().to_string()
    }
}

pub struct PostingDto<'a>(TxnPosting<'a>);
#[Object]
impl<'a> PostingDto<'a> {
    async fn account(&self, ctx: &Context<'_>) -> Option<AccountDto> {
        let ledger_stage = ctx.data_unchecked::<LedgerState>().read().await;
        ledger_stage
            .accounts
            .get(self.0.posting.account.name())
            .map(|info| AccountDto {
                name: self.0.posting.account.name().to_string(),
                info: info.clone(),
            })
    }

    async fn unit(&self) -> AmountDto {
        AmountDto(self.0.units())
    }
}
pub struct AmountDto(Amount);

#[Object]
impl AmountDto {
    async fn number(&self) -> String {
        self.0.number.to_string()
    }
    async fn currency(&self) -> String {
        self.0.currency.clone()
    }
}

pub struct StatisticDto();

#[Object]
impl StatisticDto {
    async fn accounts(&self) -> Vec<AccountDto> {
        todo!()
    }
    async fn total(&self) -> AccountSnapshot {
        todo!()
    }

    async fn income(&self) -> AccountSnapshot {
        todo!()
    }
    async fn expense(&self) -> AccountSnapshot {
        todo!()
    }
}

pub struct FileEntryDto(PathBuf);

#[Object]
impl FileEntryDto {
    async fn name(&self) -> Option<&str> {
        self.0.to_str()
    }
    async fn content(&self) -> String {
        std::fs::read_to_string(&self.0).expect("Cannot open file")
    }
}
