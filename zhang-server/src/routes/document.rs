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

use crate::error::ServerError;
use crate::response::{DocumentEntity, ResponseWrapper};
use crate::state::SharedLedger;
use crate::util::cacheable_data;
use crate::{ApiResult, ServerResult};

/// The document at a path within the ledger, given as its base64, as the documents are listed: at that path, or at its
/// alternate ([`DocumentDomain::alternate`](zhang_core::store::DocumentDomain::alternate)) when nothing is there. A path that is not that of a file within the
/// ledger's directory is a 400, or a 403 for one outside it, which a document may name but is not served; a document
/// found nowhere is a 404.
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
        // on the local disk, read as it is
        Some(root) => read_first(&ledger, Some(&root), &candidates).await?,
        None => {
            let key = format!("{}\n{}", ledger.entry.0.display(), candidates.join("\n"));
            cacheable_data(&key, async {
                info!("loading the document {:?} from the source...", candidates);
                read_first(&ledger, None, &candidates)
                    .await
                    .map_err(|it| zhang_core::ZhangError::CustomError(it.to_string()))
            })
            .await?
        }
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

/// the content of the file at the first of `paths`, within the ledger, there is one at; `None` when there is none.
/// `root` is the ledger's directory on the local disk, if it is there
async fn read_first(ledger: &Ledger, root: Option<&std::path::Path>, paths: &[String]) -> ServerResult<Option<Vec<u8>>> {
    for path in paths {
        let content = match root {
            Some(root) => match root.join(path).is_file() {
                true => Some(tokio::fs::read(root.join(path)).await?),
                false => None,
            },
            None => ledger.data_source.async_get_existing(path.clone()).await?,
        };
        if content.is_some() {
            return Ok(content);
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
            (path("."), "\".\" is not the path of a document"),
            (path(""), "\"\" is not the path of a document"),
            ("not base64!".to_owned(), "\"not base64!\" is not the base64 of the path of a document"),
            (BASE64_STANDARD.encode([0xff, 0xfe]), "the path of the document is not UTF-8"),
        ] {
            assert_eq!(download(&state, encoded).await, (400, why.to_owned()));
        }
    }
}
