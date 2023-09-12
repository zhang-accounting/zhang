use std::collections::HashMap;
use std::ops::{Add, AddAssign, Mul};
use std::path::PathBuf;

use bigdecimal::{BigDecimal, Zero};
use chrono::Local;
use itertools::Itertools;
use zhang_core::constants::KEY_OPERATING_CURRENCY;
use zhang_core::domains::Operations;
use zhang_core::error::IoErrorIntoZhangError;
use zhang_core::{ZhangError, ZhangResult};

use crate::response::{AmountResponse, CalculatedAmount};
use crate::util::AmountLike;
use crate::ServerResult;

pub mod account;
pub mod commodity;
pub mod common;
pub mod document;
pub mod file;
pub mod statistics;
pub mod transaction;

#[cfg(feature = "frontend")]
pub mod frontend;

pub(crate) fn create_folder_if_not_exist(filename: &std::path::Path) {
    std::fs::create_dir_all(filename.parent().unwrap()).expect("cannot create folder recursive");
}

pub fn group_and_calculate<T: AmountLike>(operations: &mut Operations, latest_account_balances: Vec<T>) -> ZhangResult<CalculatedAmount> {
    let operating_currency = operations
        .option(KEY_OPERATING_CURRENCY)?
        .ok_or(ZhangError::OptionNotFound(KEY_OPERATING_CURRENCY.to_owned()))?
        .value;

    let mut total_sum = BigDecimal::zero();

    let mut detail = HashMap::new();
    for (commodity, values) in &latest_account_balances.into_iter().group_by(|it| it.commodity().to_owned()) {
        let commodity_sum = values.fold(BigDecimal::zero(), |acc, item| acc.add(item.number()));

        if commodity.eq(&operating_currency) {
            total_sum.add_assign(&commodity_sum);
        } else {
            let target_price = operations.get_price(Local::now().naive_local(), &commodity, &operating_currency)?;
            if let Some(price) = target_price {
                total_sum.add_assign((&commodity_sum).mul(price.amount));
            }
        }
        detail.insert(commodity, commodity_sum);
    }
    Ok(CalculatedAmount {
        calculated: AmountResponse {
            number: total_sum,
            commodity: operating_currency.to_owned(),
        },
        detail,
    })
}

pub(crate) fn insert_line(file: PathBuf, content: &str, at: usize) -> ServerResult<()> {
    let mut file_content = std::fs::read_to_string(&file).with_path(&file)?;
    file_content.insert(at, '\n');
    file_content.insert_str(at + 1, content);
    Ok(std::fs::write(&file, file_content).with_path(&file)?)
}
