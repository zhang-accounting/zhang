use std::collections::HashMap;

use log::trace;
use zhang_ast::error::ErrorKind;
use zhang_ast::*;

use crate::domains::schemas::AccountStatus;
use crate::ledger::Ledger;
use crate::utils::hashmap::HashMapOfExt;
use crate::ZhangResult;

pub(crate) mod balance;
pub(crate) mod budget;
pub(crate) mod close;
pub(crate) mod commodity;
pub(crate) mod document;
pub(crate) mod open;
pub(crate) mod options;
pub(crate) mod plugin;
pub(crate) mod price;
pub(crate) mod transaction;
/// Directive Process is used to handle how a directive be validated, how we process directives and store the result into [Store]
pub(crate) trait DirectiveProcess: std::fmt::Debug {
    fn handler(&mut self, ledger: &mut Ledger, span: &SpanInfo) -> ZhangResult<()> {
        trace!("[DirectiveProcess] processing: {:?}", &self);
        let should_process = DirectiveProcess::validate(self, ledger, span)?;
        trace!("[DirectiveProcess] validate logic return: {}", &should_process);
        if should_process {
            DirectiveProcess::process(self, ledger, span)
        } else {
            Ok(())
        }
    }

    /// validate method is used to check if the directive is invalid, or should the directive need to emit an error
    /// if the directive is invalid, we need to use [operations::new_error] to emit a new error into [Store]
    /// return `true` if the directive need to execute the [process] method
    fn validate(&mut self, _ledger: &mut Ledger, _span: &SpanInfo) -> ZhangResult<bool> {
        Ok(true)
    }

    /// process method is to handle the directive, and generate the result for storing in [Store]
    fn process(&mut self, ledger: &mut Ledger, span: &SpanInfo) -> ZhangResult<()>;
}

/// DirectivePreProcess is to do some logic before directive been executed via [DirectiveProcess]
/// zhang can be run in sync and async context, sync and async function are provided
#[async_trait::async_trait]
pub(crate) trait DirectivePreProcess {
    /// sync function of pre handler
    fn pre_process(&self, _ledger: &mut Ledger) -> ZhangResult<()> {
        Ok(())
    }
    /// async function of pre handler
    async fn async_pre_process(&self, ledger: &mut Ledger) -> ZhangResult<()> {
        self.pre_process(ledger)
    }
}

fn check_account_existed(account_name: &str, ledger: &mut Ledger, span: &SpanInfo) -> ZhangResult<()> {
    let mut operations = ledger.operations();
    let existed = operations.exist_account(account_name)?;

    if !existed {
        operations.new_error(ErrorKind::AccountDoesNotExist, span, HashMap::of("account_name", account_name.to_string()))?;
    }
    Ok(())
}

fn check_account_closed(account_name: &str, ledger: &mut Ledger, span: &SpanInfo) -> ZhangResult<()> {
    let mut operations = ledger.operations();

    let account = operations.account(account_name)?;
    if let Some(true) = account.map(|it| it.status == AccountStatus::Close) {
        operations.new_error(ErrorKind::AccountClosed, span, HashMap::of("account_name", account_name.to_string()))?;
    }
    Ok(())
}

fn check_commodity_define(commodity_name: &str, ledger: &mut Ledger, span: &SpanInfo) -> ZhangResult<()> {
    let mut operations = ledger.operations();
    let existed = operations.exist_commodity(commodity_name)?;
    if !existed {
        operations.new_error(
            ErrorKind::CommodityDoesNotDefine,
            span,
            HashMap::of("commodity_name", commodity_name.to_string()),
        )?;
    }
    Ok(())
}
