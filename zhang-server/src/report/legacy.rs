//! The report as it was computed by hand from the store, before it moved onto the query
//! engine. Kept only to compare the two (the golden tests); wave 3 of #479 deletes it.
//!
//! Its known bugs, which the engine fixes, are listed in #479: instants in the browser's
//! timezone, a UTC day cut-off and end-of-range prices in the graph, Week and Month ignored,
//! the transaction count including pads, valuation without inverse prices or the cost
//! currency, and a top 10 sorted by raw numbers across currencies.

use std::collections::HashMap;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use itertools::Itertools;
use zhang_ast::amount::Amount;
use zhang_ast::{Account, AccountType, Flag};
use zhang_core::ledger::Ledger;
use zhang_core::utils::calculable::Calculable;
use zhang_core::utils::date_range::NaiveDateRange;

use crate::response::{ReportRankItemEntity, StatisticGraphEntity, StatisticRankEntity, StatisticSummaryEntity};
use crate::ServerResult;

pub fn summary(ledger: &Ledger, from: DateTime<Utc>, to: DateTime<Utc>) -> ServerResult<StatisticSummaryEntity> {
    let timezone = &ledger.options.timezone;
    let mut operations = ledger.operations();

    let accounts = operations.all_accounts()?;
    // balance
    let mut balances = vec![];
    for account_name in &accounts {
        let account = Account::from_str(account_name)?;
        if account.account_type == AccountType::Assets || account.account_type == AccountType::Liabilities {
            operations.account_target_date_balance(account_name, to)?.into_iter().for_each(|balance| {
                balances.push(balance.balance);
            });
        }
    }
    let balance = balances.calculate(to.with_timezone(timezone), &mut operations)?;

    let mut liability_amounts = vec![];
    for account_name in &accounts {
        let account = Account::from_str(account_name)?;
        if account.account_type == AccountType::Liabilities {
            operations.account_target_date_balance(account_name, to)?.into_iter().for_each(|balance| {
                liability_amounts.push(balance.balance);
            });
        }
    }
    let liability = liability_amounts.calculate(to.with_timezone(timezone), &mut operations)?;

    let income_amounts = operations
        .read()
        .postings
        .iter()
        .filter(|posting| posting.trx_datetime.ge(&from))
        .filter(|posting| posting.trx_datetime.le(&to))
        .filter(|posting| posting.account.account_type == AccountType::Income)
        .map(|posting| posting.inferred_amount.clone())
        .collect_vec();

    let income = income_amounts.calculate(to.with_timezone(timezone), &mut operations)?;

    let expense_amounts = operations
        .read()
        .postings
        .iter()
        .filter(|posting| posting.trx_datetime.ge(&from))
        .filter(|posting| posting.trx_datetime.le(&to))
        .filter(|posting| posting.account.account_type == AccountType::Expenses)
        .map(|posting| posting.inferred_amount.clone())
        .collect_vec();
    let expense = expense_amounts.calculate(to.with_timezone(timezone), &mut operations)?;

    // the old count: this filter is always true, so pads are counted (#479)
    #[allow(clippy::nonminimal_bool)]
    let trx_number = operations
        .read()
        .transactions
        .values()
        .filter(|trx| trx.flag != Flag::BalanceCheck || trx.flag != Flag::BalancePad)
        .filter(|trx| trx.datetime.ge(&from))
        .filter(|trx| trx.datetime.le(&to))
        .count();

    Ok(StatisticSummaryEntity {
        from,
        to,
        balance,
        liability,
        income,
        expense,
        transaction_number: trx_number as i64,
    })
}

pub fn graph(ledger: &Ledger, from: DateTime<Utc>, to: DateTime<Utc>) -> ServerResult<StatisticGraphEntity> {
    let timezone = &ledger.options.timezone;
    let mut operations = ledger.operations();

    let accounts = operations.all_accounts()?;

    let mut dated_balance = HashMap::new();
    for date in NaiveDateRange::new(from.date_naive(), to.date_naive()) {
        let mut balances = vec![];
        for account_name in &accounts {
            let account = Account::from_str(account_name)?;
            if account.account_type == AccountType::Assets || account.account_type == AccountType::Liabilities {
                operations
                    .account_target_date_balance(account_name, date.and_hms_opt(23, 59, 59).unwrap().and_local_timezone(Utc).unwrap())?
                    .into_iter()
                    .for_each(|balance| {
                        balances.push(balance.balance);
                    });
            }
        }
        let balance = balances.calculate(to.with_timezone(timezone), &mut operations)?;
        dated_balance.insert(date, balance);
    }

    let mut dated_change = HashMap::new();
    let postings = operations.dated_journals(from, to)?;

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

    Ok(StatisticGraphEntity {
        from: from.naive_local(),
        to: to.naive_local(),
        balances: dated_balance,
        changes: dated_change_ret,
    })
}

pub fn rank(ledger: &Ledger, account_type: AccountType, from: DateTime<Utc>, to: DateTime<Utc>) -> ServerResult<StatisticRankEntity> {
    let timezone = &ledger.options.timezone;
    let mut operations = ledger.operations();

    let income_transactions = operations.account_type_dated_journals(account_type, from, to)?;

    let mut account_detail: HashMap<String, Vec<Amount>> = HashMap::new();

    for posting in &income_transactions {
        let target_account = account_detail.entry(posting.account.clone()).or_default();
        target_account.push(posting.inferred_unit.clone());
    }

    let top_transactions = income_transactions
        .into_iter()
        .sorted_by(|a, b| {
            if !account_type.positive_type() {
                a.inferred_unit.number.cmp(&b.inferred_unit.number)
            } else {
                b.inferred_unit.number.cmp(&a.inferred_unit.number)
            }
        })
        .take(10)
        .collect_vec();

    let detail = account_detail
        .into_iter()
        .map(|(account, amounts)| ReportRankItemEntity {
            account,
            amount: amounts.calculate(to.with_timezone(timezone), &mut operations).expect("cannot calculate"),
        })
        .sorted_by(|a, b| a.amount.calculated.number.cmp(&b.amount.calculated.number))
        .collect_vec();
    Ok(StatisticRankEntity {
        from: from.naive_local(),
        to: to.naive_local(),
        detail,
        top_transactions,
    })
}
