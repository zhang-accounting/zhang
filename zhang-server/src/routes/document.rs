use std::path::Path;

use axum::extract::State;
use axum::http::header;
use axum::response::{AppendHeaders, IntoResponse};
use bytes::Bytes;
use gotcha::api;
use itertools::Itertools;
use log::{info, warn};
use zhang_core::ledger::Ledger;
use zhang_core::outcome::Detail;
use zhang_core::{data_source, ZhangError};

use crate::error::ServerError;
use crate::response::{DocumentEntity, InfoForNewDocument, ResponseWrapper};
use crate::routes::Base64Path;
use crate::state::SharedLedger;
use crate::util::{cache_document, cached_document, document_cache, document_cache_key};
use crate::{journals, ApiResult, ServerResult};

/// The document at a path within the ledger, given as its base64, as the documents are listed: the file at that path,
/// or at its alternate ([`Detail::Document`]) when there is
/// none, never a directory. A path that is not that of a file within the ledger's directory is a 400, or a 403 for one
/// outside it, also through a link, which a document may name but is not served; a document found nowhere is a 404.
// #[api(group = "document")]
pub async fn download_document(ledger: State<SharedLedger>, Base64Path(requested): Base64Path) -> ServerResult<impl IntoResponse> {
    let ledger = ledger.read().await;
    let path = path_in_ledger(&ledger, &requested)?;
    let alternate = ledger
        .outcomes
        .iter()
        .find_map(|it| match &it.detail {
            Detail::Document { path, alternate } if *path == requested => Some(alternate.as_deref()),
            _ => None,
        })
        .flatten()
        .and_then(|it| path_in_ledger(&ledger, it).ok());
    let file_name = path.rsplit('/').next().unwrap_or_default().to_owned();
    let candidates = std::iter::once(path).chain(alternate).collect_vec();
    let content = match ledger.data_source.local_root(&ledger.entry.0) {
        Some(root) => read_local(&root, &requested, &candidates).await?,
        None => read_remote(&ledger, &candidates).await?,
    };
    let Some(content) = content else {
        return Err(ServerError::NoSuchDocument(format!("the document {requested} does not exist")));
    };
    let headers = AppendHeaders([(header::CONTENT_DISPOSITION, format!("inline; filename=\"{}\"", file_name))]);
    Ok((headers, Bytes::from(content)))
}

/// `requested`, the path of a document, as the path of a file within the ledger's directory, written with `/`: an
/// absolute one within the directory is made relative to it
fn path_in_ledger(ledger: &Ledger, requested: &str) -> ServerResult<String> {
    let outside = || {
        ServerError::OutsideLedger(format!(
            "the document {requested} is outside the ledger's directory, so it cannot be downloaded"
        ))
    };
    if requested.contains(['\n', '\0']) {
        return Err(ServerError::InvalidInput(format!("{requested:?} is not the path of a document")));
    }
    // an absolute path is within the root the ledger was loaded from, or the directory the source reads on the disk
    let roots = [Some(ledger.entry.0.clone()), ledger.data_source.local_root(&ledger.entry.0)];
    let within = roots
        .iter()
        .flatten()
        .find_map(|root| data_source::path_in_ledger(root, Path::new(requested)))
        .ok_or_else(outside)?;
    match within.as_os_str().is_empty() {
        true => Err(ServerError::InvalidInput(format!("{requested:?} is not the path of a document"))),
        false => Ok(data_source::slashed(&within)),
    }
}

/// the content of the file at the first of `paths` there is one at, within the ledger's directory `root` on the local
/// disk, read as it is; `None` when there is none. A path leading outside the directory through a link is refused, as
/// one written outside it: only what the directory holds is served.
async fn read_local(root: &std::path::Path, requested: &str, paths: &[String]) -> ServerResult<Option<Vec<u8>>> {
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let refused = |error: std::io::Error, path: &str| match error.kind() {
        std::io::ErrorKind::PermissionDenied => ServerError::CoreError(ZhangError::ReadRefused(path.to_owned())),
        _ => ServerError::IoError(error),
    };
    for path in paths {
        // the file it names, through any link; none when there is nothing there
        let file = match root.join(path).canonicalize() {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => return Err(refused(error, path)),
            Err(_) => continue,
        };
        if !file.starts_with(&root) {
            return Err(ServerError::OutsideLedger(format!(
                "the document {requested} is outside the ledger's directory, so it cannot be downloaded"
            )));
        }
        // a regular file only, never a directory, a pipe or a device
        if file.is_file() {
            return tokio::fs::read(file).await.map(Some).map_err(|error| refused(error, path));
        }
    }
    Ok(None)
}

/// the content of the file at the first of `paths` there is one at, within `ledger` on a remote source; `None` when
/// there is none. A path read once is kept in the cache, by itself: a document read at its alternate is read at its
/// path again first next time, so a file put there since is served. The cache is a convenience: when it cannot be
/// read or written, as in a working directory the server cannot write to, the document is read from the source and
/// served all the same, and the log says so.
async fn read_remote(ledger: &Ledger, paths: &[String]) -> ServerResult<Option<Vec<u8>>> {
    for path in paths {
        let key = document_cache_key(&ledger.entry.0, path);
        match cached_document(&key).await {
            Ok(Some(content)) => return Ok(Some(content)),
            Ok(None) => {}
            Err(error) => warn!(
                "the copy of the document {path:?} in {} cannot be read, reading it from the source: {error}",
                document_cache().display()
            ),
        }
        info!("loading the document {:?} from the source...", path);
        if let Some(content) = ledger.data_source.get_existing(path.clone())? {
            if let Err(error) = cache_document(&key, &content).await {
                warn!(
                    "the document {path:?} is served but not kept in {}, which cannot be written to: {error}",
                    document_cache().display()
                );
            }
            return Ok(Some(content));
        }
    }
    Ok(None)
}

/// The accounts the document upload may name: every account opened by now, closed ones included, as a document only
/// records and may follow the close; not one opened later or never.
#[api(group = "document")]
pub async fn get_info_for_new_document(ledger: State<SharedLedger>) -> ApiResult<InfoForNewDocument> {
    ResponseWrapper::json(journals::info_for_new_document(&ledger).await?)
}

/// Every document of the ledger, newest first: the built-in query `journals.documents`.
#[api(group = "document")]
pub async fn get_documents(ledger: State<SharedLedger>) -> ApiResult<Vec<DocumentEntity>> {
    ResponseWrapper::json(journals::documents(&ledger).await?)
}

#[cfg(test)]
mod download_test {
    use std::sync::Arc;

    use axum::extract::State;
    use axum::response::IntoResponse;
    use tokio::sync::RwLock;
    use zhang_core::data_source::LocalFileSystemDataSource;
    use zhang_core::data_type::text::ZhangDataType;
    use zhang_core::ledger::Ledger;

    use super::download_document;
    use crate::routes::Base64Path;
    use crate::state::SharedLedger;

    /// the status and the body of the download of `path`, as the request names it once decoded
    async fn download(state: &State<SharedLedger>, path: &str) -> (u16, String) {
        let response = download_document(state.clone(), Base64Path(path.to_owned())).await.into_response();
        let status = response.status().as_u16();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body = String::from_utf8_lossy(&body).into_owned();
        // an answer that is not the document says why
        let body = match status {
            200 => body,
            _ => serde_json::from_str::<serde_json::Value>(&body).unwrap()["message"]
                .as_str()
                .unwrap()
                .to_owned(),
        };
        (status, body)
    }

    /// The download takes the path of a file within the ledger's directory, never one outside it, and answers why
    /// it does not serve one, without dropping the connection.
    #[tokio::test]
    async fn a_document_is_downloaded_by_a_path_within_the_ledger() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::create_dir_all(root.join("attachments")).unwrap();
        std::fs::write(root.join("attachments/a.pdf"), "the statement").unwrap();
        std::fs::write(
            root.join("main.zhang"),
            "1970-01-01 open Assets:Cash\n2024-01-01 document Assets:Cash \"attachments/a.pdf\"\n2024-01-02 document Assets:Cash \"/etc/hosts\"\n",
        )
        .unwrap();
        let source = Arc::new(LocalFileSystemDataSource::new(ZhangDataType {}));
        let ledger = Ledger::load(root.clone(), "main.zhang".to_owned(), source).unwrap();
        let state = State(SharedLedger(Arc::new(RwLock::new(ledger))));

        for within in [
            "attachments/a.pdf",
            "./attachments/b/../a.pdf",
            &root.join("attachments/a.pdf").to_string_lossy(),
        ] {
            assert_eq!(download(&state, within).await, (200, "the statement".to_owned()), "{within}");
        }
        for outside in ["/etc/hosts", "../outside.pdf", "attachments/../../outside.pdf"] {
            let (status, message) = download(&state, outside).await;
            assert_eq!(status, 403, "{outside}: {message}");
            assert_eq!(
                message,
                format!("the document {outside} is outside the ledger's directory, so it cannot be downloaded")
            );
        }
        for missing in ["attachments/missing.pdf", "attachments"] {
            assert_eq!(download(&state, missing).await, (404, format!("the document {missing} does not exist")));
        }
        // a path that is not base64 or UTF-8 is refused by `Base64Path`; one that names no file in the ledger here
        for (path, why) in [
            ("a\nb.pdf", "\"a\\nb.pdf\" is not the path of a document"),
            ("a\0b.pdf", "\"a\\0b.pdf\" is not the path of a document"),
            (".", "\".\" is not the path of a document"),
            ("", "\"\" is not the path of a document"),
        ] {
            assert_eq!(download(&state, path).await, (400, why.to_owned()));
        }
    }

    /// A link within the ledger's directory is followed; one leading out of it, to a file or through a directory, is
    /// refused, as a path written outside it.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_link_out_of_the_ledger_is_not_followed() {
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret.txt"), "outside").unwrap();
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::create_dir_all(root.join("attachments/u1")).unwrap();
        std::fs::write(root.join("attachments/a.pdf"), "the statement").unwrap();
        let link = |target: &std::path::Path, name: &str| std::os::unix::fs::symlink(target, root.join(name)).unwrap();
        link(&outside.path().join("secret.txt"), "attachments/u1/link.pdf");
        link(outside.path(), "attachments/u1/rootlink");
        link(&root.join("attachments/a.pdf"), "attachments/u1/inside.pdf");
        link(&root.join("attachments"), "attachments/u1/dirlink");
        link(&root.join("attachments/gone.pdf"), "attachments/u1/dangling.pdf");
        std::fs::write(root.join("main.zhang"), "1970-01-01 open Assets:Cash\n").unwrap();
        let source = Arc::new(LocalFileSystemDataSource::new(ZhangDataType {}));
        let ledger = Ledger::load(root.clone(), "main.zhang".to_owned(), source).unwrap();
        let state = State(SharedLedger(Arc::new(RwLock::new(ledger))));

        for within in ["attachments/u1/inside.pdf", "attachments/u1/dirlink/a.pdf"] {
            assert_eq!(download(&state, within).await, (200, "the statement".to_owned()), "{within}");
        }
        for out in ["attachments/u1/link.pdf", "attachments/u1/rootlink/secret.txt"] {
            let (status, message) = download(&state, out).await;
            assert_eq!(status, 403, "{out}: {message}");
            assert_eq!(
                message,
                format!("the document {out} is outside the ledger's directory, so it cannot be downloaded")
            );
        }
        assert_eq!(download(&state, "attachments/u1/dangling.pdf").await.0, 404);
    }

    /// A ledger opened through a link to its directory serves its documents: the directory is compared as it is on
    /// the disk, not as it was named.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_ledger_opened_through_a_link_serves_its_documents() {
        // not canonicalized: the temporary directory may itself be reached through a link
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real");
        std::fs::create_dir_all(real.join("attachments")).unwrap();
        std::fs::write(real.join("attachments/a.pdf"), "the statement").unwrap();
        std::fs::write(real.join("main.zhang"), "1970-01-01 open Assets:Cash\n").unwrap();
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let source = Arc::new(LocalFileSystemDataSource::new(ZhangDataType {}));
        let ledger = Ledger::load(link.clone(), "main.zhang".to_owned(), source).unwrap();
        let state = State(SharedLedger(Arc::new(RwLock::new(ledger))));
        for path in ["attachments/a.pdf".to_owned(), link.join("attachments/a.pdf").to_string_lossy().into_owned()] {
            assert_eq!(download(&state, &path).await, (200, "the statement".to_owned()), "{path}");
        }
    }

    /// Only a regular file is read: a pipe, which would block the read, or a device, is no document. A file the
    /// system refuses to read is refused, not missing.
    #[cfg(unix)]
    #[tokio::test]
    async fn only_a_readable_regular_file_is_served() {
        use std::io::Write as _;
        use std::os::unix::fs::PermissionsExt as _;

        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::create_dir_all(root.join("attachments")).unwrap();
        std::fs::write(root.join("main.zhang"), "1970-01-01 open Assets:Cash\n").unwrap();
        let pipe = root.join("attachments/pipe.pdf");
        assert!(std::process::Command::new("mkfifo").arg(&pipe).status().unwrap().success());
        // a writer, should anything read the pipe: that read then ends with what it writes
        let writer = pipe.clone();
        std::thread::spawn(move || {
            if let Ok(mut pipe) = std::fs::OpenOptions::new().write(true).open(writer) {
                pipe.write_all(b"from the pipe").ok();
            }
        });
        let secret = root.join("attachments/secret.pdf");
        std::fs::write(&secret, "secret").unwrap();
        std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(0o000)).unwrap();
        let source = Arc::new(LocalFileSystemDataSource::new(ZhangDataType {}));
        let ledger = Ledger::load(root.clone(), "main.zhang".to_owned(), source).unwrap();
        let state = State(SharedLedger(Arc::new(RwLock::new(ledger))));

        assert_eq!(download(&state, "attachments/pipe.pdf").await.0, 404);
        // a user the system lets read anything, as root, reads it
        if std::fs::read(&secret).is_err() {
            assert_eq!(
                download(&state, "attachments/secret.pdf").await,
                (403, "the storage refused to read attachments/secret.pdf".to_owned())
            );
        }
        std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(0o644)).unwrap();
    }
}
