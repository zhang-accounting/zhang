use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use chrono::Utc;
use itertools::Itertools;
use tokio::sync::RwLock;
use zhang_ast::amount::Amount;
use zhang_ast::{Account, AccountType, Flag};
use zhang_core::ledger::Ledger;
use zhang_core::utils::calculable::Calculable;
use zhang_core::utils::date_range::NaiveDateRange;

use crate::request::{StatisticGraphRequest, StatisticRequest};
use crate::response::{ReportRankItemResponse, ResponseWrapper, StatisticGraphResponse, StatisticRankResponse, StatisticSummaryResponse};
use crate::ApiResult;

pub async fn get_statistic_summary(ledger: State<Arc<RwLock<Ledger>>>, params: Query<StatisticRequest>) -> ApiResult<StatisticSummaryResponse> {
    let ledger = ledger.read().await;
    let timezone = &ledger.options.timezone;
    let mut operations = ledger.operations();

    let accounts = operations.all_accounts()?;
    // balance
    let mut balances = vec![];
    for account_name in &accounts {
        let account = Account::from_str(account_name)?;
        if account.account_type == AccountType::Assets || account.account_type == AccountType::Liabilities {
            operations
                .account_target_date_balance(account_name, params.to)?
                .into_iter()
                .for_each(|balance| {
                    balances.push(Amount::new(balance.balance_number, balance.balance_commodity));
                });
        }
    }
    let balance = balances.calculate(params.to.with_timezone(timezone), &mut operations)?;

    let mut liability_amounts = vec![];
    for account_name in &accounts {
        let account = Account::from_str(account_name)?;
        if account.account_type == AccountType::Liabilities {
            operations
                .account_target_date_balance(account_name, params.to)?
                .into_iter()
                .for_each(|balance| {
                    liability_amounts.push(Amount::new(balance.balance_number, balance.balance_commodity));
                });
        }
    }
    let liability = liability_amounts.calculate(params.to.with_timezone(timezone), &mut operations)?;

    let income_amounts = operations
        .read()
        .postings
        .iter()
        .filter(|posting| posting.trx_datetime.ge(&params.from))
        .filter(|posting| posting.trx_datetime.le(&params.to))
        .filter(|posting| posting.account.account_type == AccountType::Income)
        .map(|posting| posting.inferred_amount.clone())
        .collect_vec();

    let income = income_amounts.calculate(params.to.with_timezone(timezone), &mut operations)?;

    let expense_amounts = operations
        .read()
        .postings
        .iter()
        .filter(|posting| posting.trx_datetime.ge(&params.from))
        .filter(|posting| posting.trx_datetime.le(&params.to))
        .filter(|posting| posting.account.account_type == AccountType::Expenses)
        .map(|posting| posting.inferred_amount.clone())
        .collect_vec();
    let expense = expense_amounts.calculate(params.to.with_timezone(timezone), &mut operations)?;

    let trx_number = operations
        .read()
        .transactions
        .values()
        .filter(|trx| trx.flag != Flag::BalanceCheck || trx.flag != Flag::BalancePad)
        .filter(|trx| trx.datetime.ge(&params.from))
        .filter(|trx| trx.datetime.le(&params.to))
        .count();

    ResponseWrapper::json(StatisticSummaryResponse {
        from: params.from,
        to: params.to,
        balance,
        liability,
        income,
        expense,
        transaction_number: trx_number as i64,
    })
}
pub async fn get_statistic_graph(ledger: State<Arc<RwLock<Ledger>>>, params: Query<StatisticGraphRequest>) -> ApiResult<StatisticGraphResponse> {
    let ledger = ledger.read().await;
    let timezone = &ledger.options.timezone;
    let mut operations = ledger.operations();
    let params = params.0;

    let accounts = operations.all_accounts()?;

    let mut dated_balance = HashMap::new();
    for date in NaiveDateRange::new(params.from.date_naive(), params.to.date_naive()) {
        let mut balances = vec![];
        for account_name in &accounts {
            let account = Account::from_str(account_name)?;
            if account.account_type == AccountType::Assets || account.account_type == AccountType::Liabilities {
                operations
                    .account_target_date_balance(account_name, date.and_hms_opt(23, 59, 59).unwrap().and_local_timezone(Utc).unwrap())?
                    .into_iter()
                    .for_each(|balance| {
                        balances.push(Amount::new(balance.balance_number, balance.balance_commodity));
                    });
            }
        }
        let balance = balances.calculate(params.to.with_timezone(timezone), &mut operations)?;
        dated_balance.insert(date, balance);
    }

    let mut dated_change = HashMap::new();
    let postings = operations.dated_journals(params.from, params.to)?;

    for posting in postings {
        let date = posting.trx_datetime.naive_local().date();
        let account_type_store = dated_change.entry(date).or_insert_with(HashMap::new);
        let currency_store = account_type_store.entry(posting.account.account_type).or_insert_with(Vec::new);
        currency_store.push(posting.inferred_amount);
    }

    let mut dated_change_ret = HashMap::new();
    for (date, account_type_store) in dated_change.into_iter() {
        let datetime = date.and_hms_opt(23, 59, 59).unwrap().and_local_timezone(Utc).unwrap();
        let mut r = HashMap::new();
        for (account_type, currency_store) in account_type_store.into_iter() {
            let amount = currency_store.calculate(datetime.with_timezone(timezone), &mut operations)?;
            r.insert(account_type, amount);
        }
        dated_change_ret.insert(date, r);
    }

    ResponseWrapper::json(StatisticGraphResponse {
        from: params.from.naive_local(),
        to: params.to.naive_local(),
        balances: dated_balance,
        changes: dated_change_ret,
    })
}

pub async fn get_statistic_rank_detail_by_account_type(
    ledger: State<Arc<RwLock<Ledger>>>, paths: Path<(String,)>, params: Query<StatisticRequest>,
) -> ApiResult<StatisticRankResponse> {
    let account_type = AccountType::from_str(&paths.0 .0)?;
    let ledger = ledger.read().await;
    let timezone = &ledger.options.timezone;
    let mut operations = ledger.operations();

    let income_transactions = operations.account_type_dated_journals(account_type, params.from, params.to)?;

    let mut account_detail: HashMap<String, Vec<Amount>> = HashMap::new();

    for posting in &income_transactions {
        let target_account = account_detail.entry(posting.account.clone()).or_default();
        target_account.push(Amount::new(posting.inferred_unit_number.clone(), posting.inferred_unit_commodity.clone()));
    }

    let top_transactions = income_transactions
        .into_iter()
        .sorted_by(|a, b| {
            if !account_type.positive_type() {
                a.inferred_unit_number.cmp(&b.inferred_unit_number)
            } else {
                b.inferred_unit_number.cmp(&a.inferred_unit_number)
            }
        })
        .take(10)
        .collect_vec();

    let detail = account_detail
        .into_iter()
        .map(|(account, amounts)| ReportRankItemResponse {
            account,
            amount: amounts.calculate(params.to.with_timezone(timezone), &mut operations).expect("cannot calculate"),
        })
        .sorted_by(|a, b| a.amount.calculated.number.cmp(&b.amount.calculated.number))
        .collect_vec();
    ResponseWrapper::json(StatisticRankResponse {
        from: params.from.naive_local(),
        to: params.to.naive_local(),
        detail,
        top_transactions,
    })
}
