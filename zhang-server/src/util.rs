use std::path::Path;

/// Where the documents read from a remote source are kept on the local disk. Earlier versions kept them in
/// `.cache/data`, with a missing document kept as an empty file: that folder is not read, so a file kept here is the
/// document as read, an empty one too.
pub fn document_cache() -> std::path::PathBuf {
    Path::new(zhang_core::constants::CACHE_DIR).join("documents")
}

/// The name in the cache of the document at `path` of the ledger at `root`: the hash of both, each preceded by its
/// length, so no two of them share one.
pub fn document_cache_key(root: &std::path::Path, path: &str) -> String {
    use sha2::Digest;

    let mut hash = sha2::Sha256::new();
    for part in [root.to_string_lossy().as_bytes(), path.as_bytes()] {
        hash.update((part.len() as u64).to_le_bytes());
        hash.update(part);
    }
    hex(&hash.finalize())
}

/// The fingerprint of a file's content, its SHA-256 in hex: `GET /api/files/{path}` serves a file with it, and a save
/// through `PUT /api/files/{path}` sends it back as `expected_sha256`, to be refused when the file changed since.
pub fn sha256_hex(content: &[u8]) -> String {
    use sha2::Digest;

    hex(&sha2::Sha256::digest(content))
}

/// `bytes` in lowercase hex, two digits a byte
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The document kept in the cache under `key`: `None` when there is none. An error says the cache cannot be read,
/// which tells nothing of whether the document is there.
pub async fn cached_document(key: &str) -> std::io::Result<Option<Vec<u8>>> {
    cached_document_in(&document_cache(), key).await
}

/// [`cached_document`], in the cache at `folder`
async fn cached_document_in(folder: &Path, key: &str) -> std::io::Result<Option<Vec<u8>>> {
    match tokio::fs::read(folder.join(key)).await {
        Ok(content) => Ok(Some(content)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

/// Keep `content`, a document read, in the cache under `key`. It is written aside then moved, so a reader never finds
/// part of it. When it cannot be kept, as in a working directory that cannot be written to, nothing is left behind,
/// and the error says why: the document is served from the source all the same.
pub async fn cache_document(key: &str, content: &[u8]) -> std::io::Result<()> {
    cache_document_in(&document_cache(), key, content).await
}

/// [`cache_document`], in the cache at `folder`
async fn cache_document_in(folder: &Path, key: &str, content: &[u8]) -> std::io::Result<()> {
    tokio::fs::create_dir_all(folder).await?;
    let aside = folder.join(format!("{key}.{}", uuid::Uuid::new_v4()));
    let moved = async {
        tokio::fs::write(&aside, content).await?;
        tokio::fs::rename(&aside, folder.join(key)).await
    }
    .await;
    if moved.is_err() {
        // what was set aside, if anything, is no document: nothing reads it
        tokio::fs::remove_file(&aside).await.ok();
    }
    moved
}

#[cfg(test)]
mod cache_test {
    use std::path::Path;

    use super::{cache_document_in, cached_document_in};

    /// the names of the files in `folder`, sorted; none when there is no folder
    fn files_in(folder: &Path) -> Vec<String> {
        let Ok(entries) = std::fs::read_dir(folder) else {
            return vec![];
        };
        let mut names: Vec<String> = entries.map(|it| it.unwrap().file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        names
    }

    /// A document kept is read back, and nothing else is left in the folder; one not kept is not there, which is no
    /// error.
    #[tokio::test]
    async fn a_document_kept_is_read_back() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("documents");
        assert_eq!(cached_document_in(&folder, "k1").await.unwrap(), None);
        cache_document_in(&folder, "k1", b"the statement").await.unwrap();
        assert_eq!(cached_document_in(&folder, "k1").await.unwrap(), Some(b"the statement".to_vec()));
        assert_eq!(files_in(&folder), ["k1"]);
    }

    /// A document that cannot be kept, the cache folder being one that cannot be made or written to, is an error that
    /// says why, and leaves nothing behind: no folder, no file set aside.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_document_that_cannot_be_kept_leaves_nothing_behind() {
        use std::os::unix::fs::PermissionsExt as _;

        let dir = tempfile::tempdir().unwrap();
        let locked = dir.path().join("locked");
        std::fs::create_dir(&locked).unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o555)).unwrap();
        // a user the system lets write anywhere, as root, keeps it: nothing to check
        if std::fs::write(locked.join("probe"), "").is_ok() {
            return;
        }
        // the folder cannot be made
        let folder = locked.join("documents");
        let error = cache_document_in(&folder, "k1", b"the statement").await.unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied, "{error}");
        assert!(!folder.exists());
        // the folder is there, but cannot be written to
        let error = cache_document_in(&locked, "k1", b"the statement").await.unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied, "{error}");
        assert_eq!(files_in(&locked), Vec::<String>::new());
        assert_eq!(cached_document_in(&locked, "k1").await.unwrap(), None);
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// A document written aside that cannot be moved into place is removed: a failed write leaves no stray file. A
    /// cache that cannot be read is an error, not a document missing.
    #[tokio::test]
    async fn a_document_that_cannot_be_moved_into_place_is_removed() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("documents");
        // a directory stands where the document would go: nothing can be moved over it, or read as it
        std::fs::create_dir_all(folder.join("k1/taken")).unwrap();
        cache_document_in(&folder, "k1", b"the statement").await.unwrap_err();
        assert_eq!(files_in(&folder), ["k1"]);
        assert!(cached_document_in(&folder, "k1").await.is_err());
    }
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
