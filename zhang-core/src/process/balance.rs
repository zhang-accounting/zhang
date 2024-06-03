use std::collections::HashMap;
use std::ops::Sub;

use bigdecimal::{BigDecimal, Zero};
use zhang_ast::amount::Amount;
use zhang_ast::error::ErrorKind;
use zhang_ast::{BalanceCheck, BalancePad, Flag, Posting, SpanInfo, Transaction, ZhangString};

use crate::ledger::Ledger;
use crate::process::DirectiveProcess;
use crate::utils::hashmap::HashMapOfExt;
use crate::{process, ZhangResult};

impl DirectiveProcess for BalancePad {
    fn validate(&mut self, ledger: &mut Ledger, span: &SpanInfo) -> ZhangResult<bool> {
        process::check_account_existed(self.account.name(), ledger, span)?;
        process::check_account_existed(self.pad.name(), ledger, span)?;
        process::check_account_closed(self.account.name(), ledger, span)?;
        process::check_account_closed(self.pad.name(), ledger, span)?;
        Ok(true)
    }

    fn process(&mut self, ledger: &mut Ledger, span: &SpanInfo) -> ZhangResult<()> {
        let mut operations = ledger.operations();

        let option = operations.account_target_day_balance(
            self.account.name(),
            self.date.to_timezone_datetime(&ledger.options.timezone),
            &self.amount.currency,
        )?;

        let current_balance_amount = option.map(|it| it.number).unwrap_or_else(BigDecimal::zero);

        let distance = Amount::new((&self.amount.number).sub(&current_balance_amount), self.amount.currency.clone());
        let mut transformed_trx = Transaction {
            date: self.date.clone(),
            flag: Some(Flag::BalancePad),
            payee: Some(ZhangString::quote("Balance Pad")),
            narration: Some(ZhangString::quote(format!("pad {} to {}", self.account.name(), self.pad.name()))),
            tags: Default::default(),
            links: Default::default(),
            postings: vec![
                Posting {
                    flag: None,
                    account: self.account.clone(),
                    units: Some(distance.clone()),
                    cost: None,
                    price: None,
                    comment: None,
                    meta: Default::default(),
                },
                Posting {
                    flag: None,
                    account: self.pad.clone(),
                    units: None,
                    cost: None,
                    price: None,
                    comment: None,
                    meta: Default::default(),
                },
            ],
            meta: Default::default(),
        };

        transformed_trx.process(ledger, span)?;

        // let neg_distance = distance.neg();
        Ok(())
    }
}

impl DirectiveProcess for BalanceCheck {
    fn validate(&mut self, ledger: &mut Ledger, span: &SpanInfo) -> ZhangResult<bool> {
        process::check_account_existed(self.account.name(), ledger, span)?;
        process::check_account_closed(self.account.name(), ledger, span)?;
        Ok(true)
    }

    fn process(&mut self, ledger: &mut Ledger, span: &SpanInfo) -> ZhangResult<()> {
        let mut operations = ledger.operations();
        let option = operations.account_target_day_balance(
            self.account.name(),
            self.date.to_timezone_datetime(&ledger.options.timezone),
            &self.amount.currency,
        )?;

        let current_balance_amount = option.map(|it| it.number).unwrap_or_else(BigDecimal::zero);

        let distance = Amount::new((&self.amount.number).sub(&current_balance_amount), self.amount.currency.clone());
        if !distance.is_zero() {
            operations.new_error(
                ErrorKind::AccountBalanceCheckError,
                span,
                HashMap::of("account_name", self.account.name().to_string()),
            )?;
        }

        let mut transformed_trx = Transaction {
            date: self.date.clone(),
            flag: Some(Flag::BalanceCheck),
            payee: Some(ZhangString::quote("Balance Check")),
            narration: Some(ZhangString::quote(self.account.name())),
            tags: Default::default(),
            links: Default::default(),
            postings: vec![Posting {
                flag: None,
                account: self.account.clone(),
                units: Some(distance),
                cost: None,
                price: None,
                comment: None,
                meta: Default::default(),
            }],
            meta: Default::default(),
        };

        transformed_trx.process(ledger, span)?;
        Ok(())
    }
}
