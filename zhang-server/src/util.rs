use std::path::PathBuf;

use zhang_core::ZhangResult;

/// Where the documents read from a remote source are kept on the local disk. Earlier versions kept them in
/// `.cache/data`, with a missing document kept as an empty file: that folder is not read, so a file kept here is the
/// document as read, an empty one too.
const DOCUMENT_CACHE: &str = ".cache/documents";

/// The name in the cache of the document at `path` of the ledger at `root`: the hash of both, each preceded by its
/// length, so no two of them share one.
pub fn document_cache_key(root: &std::path::Path, path: &str) -> String {
    use sha2::Digest;

    let mut hash = sha2::Sha256::new();
    for part in [root.to_string_lossy().as_bytes(), path.as_bytes()] {
        hash.update((part.len() as u64).to_le_bytes());
        hash.update(part);
    }
    hash.finalize().iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The document kept in the cache under `key`, if it is.
pub async fn cached_document(key: &str) -> Option<Vec<u8>> {
    tokio::fs::read(PathBuf::from(DOCUMENT_CACHE).join(key)).await.ok()
}

/// Keep `content`, a document read, in the cache under `key`. It is written aside then moved, so a reader never finds
/// part of it.
pub async fn cache_document(key: &str, content: &[u8]) -> ZhangResult<()> {
    let folder = PathBuf::from(DOCUMENT_CACHE);
    tokio::fs::create_dir_all(&folder).await?;
    let aside = folder.join(format!("{key}.{}", uuid::Uuid::new_v4()));
    tokio::fs::write(&aside, content).await?;
    tokio::fs::rename(&aside, folder.join(key)).await?;
    Ok(())
}

#[cfg(test)]
mod cache_key_test {
    use std::path::Path;

    use super::document_cache_key as key;

    /// The root and the path each count by themselves: no root and path written together the same way share a name.
    #[test]
    fn a_cache_key_tells_the_root_from_the_path() {
        assert_ne!(key(Path::new("/ledger"), "a/b.pdf"), key(Path::new("/ledger/a"), "b.pdf"));
        assert_ne!(key(Path::new("/l"), "ab.pdf"), key(Path::new("/la"), "b.pdf"));
        assert_ne!(key(Path::new("/ledger"), "a.pdf"), key(Path::new("/other"), "a.pdf"));
        assert_eq!(key(Path::new("/ledger"), "a/b.pdf"), key(Path::new("/ledger"), "a/b.pdf"));
        let name = key(Path::new("/ledger"), "a/b.pdf");
        assert!(name.len() == 64 && name.chars().all(|it| it.is_ascii_hexdigit()), "{name}");
    }
}
