use std::future::Future;
use std::path::PathBuf;
use std::str::FromStr;

use bigdecimal::BigDecimal;
use zhang_core::domains::schemas::{AccountBalanceDomain, AccountDailyBalanceDomain};
use zhang_core::error::IoErrorIntoZhangError;
use zhang_core::ZhangResult;

pub trait AmountLike {
    fn number(&self) -> &BigDecimal;

    fn commodity(&self) -> &String;
}

impl AmountLike for AccountDailyBalanceDomain {
    fn number(&self) -> &BigDecimal {
        &self.balance_number
    }

    fn commodity(&self) -> &String {
        &self.balance_commodity
    }
}

impl AmountLike for AccountBalanceDomain {
    fn number(&self) -> &BigDecimal {
        &self.balance_number
    }

    fn commodity(&self) -> &String {
        &self.balance_commodity
    }
}

/// fetch the data from local if the cache exists, normally used for documents.
pub async fn cacheable_data<F>(id: &str, miss_fn: F) -> ZhangResult<Vec<u8>>
where
    F: Future<Output = ZhangResult<Vec<u8>>>,
{
    let data_cache_folder = PathBuf::from_str(".cache/data").expect("Cannot create path");

    // create data cache folder if not exist
    std::fs::create_dir_all(&data_cache_folder).with_path(data_cache_folder.as_path())?;

    let target_file = data_cache_folder.join(id);

    let vec = match std::fs::read(&target_file) {
        Ok(data) => data,
        _ => {
            let fetched_data = miss_fn.await?;
            std::fs::write(&target_file, &fetched_data).with_path(target_file.as_path())?;
            fetched_data
        }
    };
    Ok(vec)
}
