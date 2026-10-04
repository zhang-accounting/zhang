use std::path::Component;

use axum::extract::{Path, State};
use axum::http::header;
use axum::response::{AppendHeaders, IntoResponse};
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;
use bytes::Bytes;
use gotcha::api;
use itertools::Itertools;
use log::info;
use zhang_core::ledger::Ledger;
use zhang_core::ZhangError;

use crate::error::ServerError;
use crate::response::{DocumentEntity, ResponseWrapper};
use crate::state::SharedLedger;
use crate::util::{cache_document, cached_document, document_cache_key};
use crate::{ApiResult, ServerResult};

/// The document at a path within the ledger, given as its base64, as the documents are listed: the file at that path,
/// or at its alternate ([`DocumentDomain::alternate`](zhang_core::store::DocumentDomain::alternate)) when there is
/// none, never a directory. A path that is not that of a file within the ledger's directory is a 400, or a 403 for one
/// outside it, also through a link, which a document may name but is not served; a document found nowhere is a 404.
// #[api(group = "document")]
pub async fn download_document(ledger: State<SharedLedger>, path: Path<(String,)>) -> ServerResult<impl IntoResponse> {
    let encoded = path.0 .0;
    let decoded = BASE64_STANDARD
        .decode(&encoded)
        .map_err(|_| ServerError::InvalidInput(format!("{encoded:?} is not the base64 of the path of a document")))?;
    let requested = String::from_utf8(decoded).map_err(|_| ServerError::InvalidInput("the path of the document is not UTF-8".to_owned()))?;
    let ledger = ledger.read().await;
    let path = path_in_ledger(&ledger, &requested)?;
    let alternate = ledger
        .store
        .read()
        .expect("poison lock detect")
        .documents
        .iter()
        .find(|it| it.path == requested)
        .and_then(|it| it.alternate.clone())
        .and_then(|it| path_in_ledger(&ledger, &it).ok());
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
    let requested_path = std::path::Path::new(requested);
    let relative = match requested_path.is_absolute() {
        true => {
            let roots = [Some(ledger.entry.0.clone()), ledger.data_source.local_root(&ledger.entry.0)];
            roots
                .iter()
                .flatten()
                .find_map(|root| requested_path.strip_prefix(root).ok())
                .ok_or_else(outside)?
        }
        false => requested_path,
    };
    let mut parts: Vec<String> = vec![];
    for component in relative.components() {
        match component {
            Component::Normal(part) => parts.push(part.to_string_lossy().into_owned()),
            Component::CurDir => {}
            Component::ParentDir if parts.pop().is_some() => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return Err(outside()),
        }
    }
    match parts.is_empty() {
        true => Err(ServerError::InvalidInput(format!("{requested:?} is not the path of a document"))),
        false => Ok(parts.join("/")),
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
/// path again first next time, so a file put there since is served.
async fn read_remote(ledger: &Ledger, paths: &[String]) -> ServerResult<Option<Vec<u8>>> {
    for path in paths {
        let key = document_cache_key(&ledger.entry.0, path);
        if let Some(content) = cached_document(&key).await {
            return Ok(Some(content));
        }
        info!("loading the document {:?} from the source...", path);
        if let Some(content) = ledger.data_source.async_get_existing(path.clone()).await? {
            cache_document(&key, &content).await?;
            return Ok(Some(content));
        }
    }
    Ok(None)
}

#[api(group = "document")]
pub async fn get_documents(ledger: State<SharedLedger>) -> ApiResult<Vec<DocumentEntity>> {
    let ledger = ledger.read().await;
    let operations = ledger.operations();
    let store = operations.read();

    let rows = store
        .documents
        .iter()
        .cloned()
        .rev()
        .map(|doc| DocumentEntity {
            datetime: doc.datetime.naive_local(),
            filename: doc.filename.unwrap_or_default(),
            path: doc.path.clone(),
            extension: mime_guess::from_path(doc.path).first().map(|it| it.to_string()),
            account: doc.document_type.as_account(),
            trx_id: doc.document_type.as_trx(),
        })
        .collect_vec();

    ResponseWrapper::json(rows)
}

#[cfg(test)]
mod download_test {
    use std::sync::Arc;

    use axum::extract::{Path, State};
    use axum::response::IntoResponse;
    use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
    use base64::Engine as _;
    use tokio::sync::RwLock;
    use zhang_core::data_source::LocalFileSystemDataSource;
    use zhang_core::data_type::text::ZhangDataType;
    use zhang_core::ledger::Ledger;

    use super::download_document;
    use crate::state::SharedLedger;

    async fn download(state: &State<SharedLedger>, encoded: String) -> (u16, String) {
        let response = download_document(state.clone(), Path((encoded,))).await.into_response();
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
        let ledger = Ledger::async_load(root.clone(), "main.zhang".to_owned(), source).await.unwrap();
        let state = State(SharedLedger(Arc::new(RwLock::new(ledger))));
        let path = |path: &str| BASE64_STANDARD.encode(path);

        for within in [
            "attachments/a.pdf",
            "./attachments/b/../a.pdf",
            &root.join("attachments/a.pdf").to_string_lossy(),
        ] {
            assert_eq!(download(&state, path(within)).await, (200, "the statement".to_owned()), "{within}");
        }
        for outside in ["/etc/hosts", "../outside.pdf", "attachments/../../outside.pdf"] {
            let (status, message) = download(&state, path(outside)).await;
            assert_eq!(status, 403, "{outside}: {message}");
            assert_eq!(
                message,
                format!("the document {outside} is outside the ledger's directory, so it cannot be downloaded")
            );
        }
        for missing in ["attachments/missing.pdf", "attachments"] {
            assert_eq!(download(&state, path(missing)).await, (404, format!("the document {missing} does not exist")));
        }
        for (encoded, why) in [
            (path("a\nb.pdf"), "\"a\\nb.pdf\" is not the path of a document"),
            (path("a\0b.pdf"), "\"a\\0b.pdf\" is not the path of a document"),
            (path("."), "\".\" is not the path of a document"),
            (path(""), "\"\" is not the path of a document"),
            ("not base64!".to_owned(), "\"not base64!\" is not the base64 of the path of a document"),
            (BASE64_STANDARD.encode([0xff, 0xfe]), "the path of the document is not UTF-8"),
        ] {
            assert_eq!(download(&state, encoded).await, (400, why.to_owned()));
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
        let ledger = Ledger::async_load(root.clone(), "main.zhang".to_owned(), source).await.unwrap();
        let state = State(SharedLedger(Arc::new(RwLock::new(ledger))));
        let path = |path: &str| BASE64_STANDARD.encode(path);

        for within in ["attachments/u1/inside.pdf", "attachments/u1/dirlink/a.pdf"] {
            assert_eq!(download(&state, path(within)).await, (200, "the statement".to_owned()), "{within}");
        }
        for out in ["attachments/u1/link.pdf", "attachments/u1/rootlink/secret.txt"] {
            let (status, message) = download(&state, path(out)).await;
            assert_eq!(status, 403, "{out}: {message}");
            assert_eq!(
                message,
                format!("the document {out} is outside the ledger's directory, so it cannot be downloaded")
            );
        }
        assert_eq!(download(&state, path("attachments/u1/dangling.pdf")).await.0, 404);
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
        let ledger = Ledger::async_load(link.clone(), "main.zhang".to_owned(), source).await.unwrap();
        let state = State(SharedLedger(Arc::new(RwLock::new(ledger))));
        for path in ["attachments/a.pdf".to_owned(), link.join("attachments/a.pdf").to_string_lossy().into_owned()] {
            assert_eq!(
                download(&state, BASE64_STANDARD.encode(&path)).await,
                (200, "the statement".to_owned()),
                "{path}"
            );
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
        let ledger = Ledger::async_load(root.clone(), "main.zhang".to_owned(), source).await.unwrap();
        let state = State(SharedLedger(Arc::new(RwLock::new(ledger))));

        assert_eq!(download(&state, BASE64_STANDARD.encode("attachments/pipe.pdf")).await.0, 404);
        // a user the system lets read anything, as root, reads it
        if std::fs::read(&secret).is_err() {
            assert_eq!(
                download(&state, BASE64_STANDARD.encode("attachments/secret.pdf")).await,
                (403, "the storage refused to read attachments/secret.pdf".to_owned())
            );
        }
        std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(0o644)).unwrap();
    }
}
