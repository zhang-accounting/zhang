use std::future::Future;
use std::path::PathBuf;
use std::str::FromStr;

use bigdecimal::BigDecimal;
use log::info;
use zhang_core::domains::schemas::{AccountBalanceDomain, AccountDailyBalanceDomain};
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
    tokio::fs::create_dir_all(&data_cache_folder).await?;

    let target_file = data_cache_folder.join(id);

    let vec = match tokio::fs::read(&target_file).await {
        Ok(data) => data,
        _ => {
            info!("missing cache with id [{}]...", &id);
            let fetched_data = miss_fn.await?;
            tokio::fs::write(&target_file, &fetched_data).await?;
            fetched_data
        }
    };
    Ok(vec)
}
