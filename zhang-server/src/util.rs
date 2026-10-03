use std::future::Future;
use std::path::PathBuf;
use std::str::FromStr;

use log::info;
use zhang_core::ZhangResult;

/// The content `fetch` gives for `key`, kept in the data cache once fetched, for documents of a remote source. When
/// `fetch` finds nothing, nothing is kept. An empty file kept is not used: earlier versions kept a missing document so.
/// The file of a key is named by its hash, so no key names a file outside the cache.
pub async fn cacheable_data<F>(key: &str, fetch: F) -> ZhangResult<Option<Vec<u8>>>
where
    F: Future<Output = ZhangResult<Option<Vec<u8>>>>,
{
    use sha2::Digest;

    let data_cache_folder = PathBuf::from_str(".cache/data").expect("Cannot create path");
    tokio::fs::create_dir_all(&data_cache_folder).await?;
    let hash = sha2::Sha256::digest(key.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let target_file = data_cache_folder.join(hash);

    match tokio::fs::read(&target_file).await {
        Ok(data) if !data.is_empty() => Ok(Some(data)),
        _ => {
            info!("missing cache with key [{}]...", key);
            let fetched = fetch.await?;
            if let Some(data) = &fetched {
                tokio::fs::write(&target_file, data).await?;
            }
            Ok(fetched)
        }
    }
}
