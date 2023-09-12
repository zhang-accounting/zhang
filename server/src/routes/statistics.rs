use std::collections::HashMap;
use std::ops::{Add, AddAssign, Div};
use std::sync::Arc;

use actix_web::get;
use actix_web::web::{Data, Query};
use bigdecimal::{BigDecimal, Zero};
use chrono::NaiveDate;
use itertools::Itertools;
use tokio::sync::RwLock;
use zhang_ast::{AccountType, Flag};
use zhang_core::ledger::Ledger;
use zhang_core::utils::date_range::NaiveDateRange;

use crate::request::{ReportRequest, StatisticRequest};
use crate::response::{AmountResponse, CurrentStatisticResponse, ReportRankItemResponse, ReportResponse, ResponseWrapper, StatisticResponse};
use crate::{routes, ApiResult};

#[get("/api/statistic")]
pub async fn get_statistic_data(ledger: Data<Arc<RwLock<Ledger>>>, params: Query<StatisticRequest>) -> ApiResult<StatisticResponse> {
    let ledger = ledger.read().await;
    let mut operations = ledger.operations();
    let params = params.into_inner();

    let rows = operations.static_duration(params.from, params.to)?;

    // 构建每日的统计数据
    let mut ret: HashMap<NaiveDate, HashMap<String, AmountResponse>> = HashMap::new();
    for (date, dated_rows) in &rows.into_iter().group_by(|row| row.date) {
        let date_entry = ret.entry(date).or_insert_with(HashMap::new);
        for row in dated_rows {
            date_entry.insert(
                row.account_type,
                AmountResponse {
                    number: row.amount,
                    commodity: row.commodity,
                },
            );
        }
    }

    // 补充不存在的日期
    for day in NaiveDateRange::new(params.from.date_naive(), params.to.date_naive()) {
        ret.entry(day).or_insert_with(HashMap::new);
    }

    let accounts = operations.all_accounts()?;

    let mut existing_balances: HashMap<String, AmountResponse> = HashMap::default();
    for account in accounts {
        let balance = operations.account_target_date_balance(&account, params.from)?.into_iter().next();
        if let Some(balance) = balance {
            existing_balances.insert(
                account,
                AmountResponse {
                    number: balance.balance_number,
                    commodity: balance.balance_commodity,
                },
            );
        }
    }

    let detail_ret: HashMap<NaiveDate, HashMap<String, AmountResponse>> = HashMap::new();

    ResponseWrapper::json(StatisticResponse {
        changes: ret,
        details: detail_ret,
    })
}

#[get("/api/statistic/current")]
pub async fn current_statistic(ledger: Data<Arc<RwLock<Ledger>>>) -> ApiResult<CurrentStatisticResponse> {
    let ledger = ledger.read().await;

    let mut operations = ledger.operations();

    let latest_account_balances = operations.accounts_latest_balance()?;

    let balances = routes::group_and_calculate(
        &mut operations,
        latest_account_balances
            .iter()
            .filter(|it| it.account.starts_with("Assets") || it.account.starts_with("Liabilities"))
            .cloned()
            .collect_vec(),
    )?;

    let liability = routes::group_and_calculate(
        &mut operations,
        latest_account_balances
            .iter()
            .filter(|it| it.account.starts_with("Liabilities"))
            .cloned()
            .collect_vec(),
    )?;

    struct CurrentMonthBalance {
        account_type: String,
        amount: BigDecimal,
        commodity: String,
    }

    let current_month_balance = operations
        .accounts_latest_balance()?
        .into_iter()
        .map(|balance| CurrentMonthBalance {
            // todo use Account constructor
            account_type: balance.account.split(':').next().unwrap().to_owned(),
            amount: balance.balance_number,
            commodity: balance.balance_commodity,
        })
        .collect_vec();

    let income = current_month_balance
        .iter()
        .find(|it| it.account_type.eq("Income"))
        .map(|it| AmountResponse {
            number: it.amount.clone(),
            commodity: it.commodity.to_owned(),
        })
        .unwrap_or_else(|| AmountResponse {
            number: BigDecimal::zero(),
            commodity: ledger.options.operating_currency.to_owned(),
        });
    let expense = current_month_balance
        .iter()
        .find(|it| it.account_type.eq("Expenses"))
        .map(|it| AmountResponse {
            number: it.amount.clone(),
            commodity: it.commodity.to_owned(),
        })
        .unwrap_or_else(|| AmountResponse {
            number: BigDecimal::zero(),
            commodity: ledger.options.operating_currency.to_owned(),
        });

    ResponseWrapper::json(CurrentStatisticResponse {
        balance: balances,
        liability,
        income,
        expense,
    })
}

#[get("/api/report")]
pub async fn get_report(ledger: Data<Arc<RwLock<Ledger>>>, params: Query<ReportRequest>) -> ApiResult<ReportResponse> {
    let ledger = ledger.read().await;
    let mut operations = ledger.operations();

    let accounts = operations.all_accounts()?;

    let mut latest_account_balances = vec![];
    for account in accounts {
        let vec = operations.account_target_date_balance(&account, params.to)?;
        latest_account_balances.extend(vec);
    }

    let balance = latest_account_balances
        .iter()
        .filter(|it| it.account.starts_with("Assets") || it.account.starts_with("Liabilities"))
        .fold(BigDecimal::zero(), |acc, item| acc.add(&item.balance_number));

    let liability = latest_account_balances
        .iter()
        .filter(|it| it.account.starts_with("Liabilities"))
        .fold(BigDecimal::zero(), |acc, item| acc.add(&item.balance_number));

    let store = operations.read();

    let mut account_type_postings_map = HashMap::new();
    for (key, data) in &store
        .postings
        .iter()
        .filter(|posting| posting.trx_datetime.ge(&params.from))
        .filter(|posting| posting.trx_datetime.le(&params.to))
        .cloned()
        .group_by(|it| it.account.account_type)
    {
        // todo(high) calculate all postings amount
        account_type_postings_map.insert(key, data.collect_vec());
    }

    let income = account_type_postings_map
        .remove(&AccountType::Income)
        .unwrap_or_default()
        .into_iter()
        .next()
        .map(|it| AmountResponse {
            number: it.inferred_amount.number.clone(),
            commodity: it.inferred_amount.currency.to_owned(),
        })
        .unwrap_or_else(|| AmountResponse {
            number: BigDecimal::zero(),
            commodity: ledger.options.operating_currency.to_owned(),
        });
    let expense = account_type_postings_map
        .remove(&AccountType::Expenses)
        .unwrap_or_default()
        .into_iter()
        .next()
        .map(|it| AmountResponse {
            number: it.inferred_amount.number.clone(),
            commodity: it.inferred_amount.currency.to_owned(),
        })
        .unwrap_or_else(|| AmountResponse {
            number: BigDecimal::zero(),
            commodity: ledger.options.operating_currency.to_owned(),
        });

    let transaction_total = store
        .transactions
        .values()
        .filter(|trx| trx.flag != Flag::BalancePad && trx.flag != Flag::BalanceCheck)
        .filter(|trx| trx.datetime.ge(&params.from))
        .filter(|trx| trx.datetime.le(&params.to))
        .count();
    drop(store);

    let income_transactions = operations.account_dated_journals(AccountType::Income, params.from, params.to)?;

    let total_income = income_transactions
        .iter()
        .fold(BigDecimal::zero(), |accr, item| accr.add(&item.inferred_unit_number));

    let mut counter = HashMap::new();
    for item in &income_transactions {
        let x = counter.entry(item.account.to_owned()).or_insert_with(BigDecimal::zero);
        x.add_assign(&item.inferred_unit_number);
    }
    let income_rank = counter
        .into_iter()
        .sorted_by(|a, b| a.1.cmp(&b.1))
        .take(10)
        .map(|(account, account_total)| ReportRankItemResponse {
            account,
            percent: account_total.div(&total_income),
        })
        .collect_vec();

    let income_top_transactions = income_transactions
        .into_iter()
        .sorted_by(|a, b| a.inferred_unit_number.cmp(&b.inferred_unit_number))
        .take(10)
        .collect_vec();

    // --------

    let expense_transactions = operations.account_dated_journals(AccountType::Expenses, params.from, params.to)?;

    let total_expense = expense_transactions
        .iter()
        .fold(BigDecimal::zero(), |accr, item| accr.add(&item.inferred_unit_number));

    let mut counter = HashMap::new();
    for item in &expense_transactions {
        let x = counter.entry(item.account.to_owned()).or_insert_with(BigDecimal::zero);
        x.add_assign(&item.inferred_unit_number);
    }
    let expense_rank = counter
        .into_iter()
        .sorted_by(|a, b| a.1.cmp(&b.1))
        .rev()
        .take(10)
        .map(|(account, account_total)| ReportRankItemResponse {
            account,
            percent: account_total.div(&total_expense),
        })
        .collect_vec();

    let expense_top_transactions = expense_transactions
        .into_iter()
        .sorted_by(|a, b| a.inferred_unit_number.cmp(&b.inferred_unit_number))
        .rev()
        .take(10)
        .collect_vec();

    ResponseWrapper::json(ReportResponse {
        from: params.from.naive_local(),
        to: params.to.naive_local(),
        balance: AmountResponse {
            number: balance,
            commodity: ledger.options.operating_currency.to_owned(),
        },
        liability: AmountResponse {
            number: liability,
            commodity: ledger.options.operating_currency.to_owned(),
        },
        income,
        expense,
        transaction_number: transaction_total as i64,
        income_rank,
        income_top_transactions,
        expense_rank,
        expense_top_transactions,
    })
}
