pub mod account;
#[cfg(test)]
mod budget_commodity_golden;
#[cfg(test)]
mod budget_reference;
pub mod commodity;
pub mod common;
pub mod document;
pub mod file;
pub mod statistics;
pub mod transaction;

pub mod plugin;
pub mod plugin_router;
pub mod query;

#[cfg(feature = "frontend")]
pub mod frontend;

use axum::async_trait;
use axum::extract::{FromRequestParts, MatchedPath, OriginalUri, Path};
use axum::http::request::Parts;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;
use gotcha::oas::{Parameter, RequestBody};
use gotcha::{Either, ParameterProvider, Schematic};
use percent_encoding::percent_decode_str;
use serde::de::DeserializeOwned;
use serde_qs;

use crate::error::ServerError;

/// A ledger file path at the end of the request path, base64 encoded with the standard alphabet,
/// as in `/api/files/{file_path}` and `/api/documents/{file_path}`.
///
/// Standard base64 can contain `/`, which splits the encoded path over more than one segment, so
/// its routes match the rest of the request path: `/*file_path`, or, where the OpenAPI document
/// needs the single `/:file_path`, that route next to `/:file_path/` and `/:file_path/*rest`. This
/// takes everything after the fixed prefix of the route that matched, percent decoded, so a client
/// may send the `/` as is or as `%2F`. An empty, non-base64 or non-UTF-8 path is a 400.
pub struct Base64Path(pub String);

#[async_trait]
impl<S: Send + Sync> FromRequestParts<S> for Base64Path {
    type Rejection = ServerError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let route = parts.extensions.get::<MatchedPath>().map(MatchedPath::as_str).unwrap_or_default();
        let prefix = &route[..route.find([':', '*']).unwrap_or(route.len())];
        // the matched path is the full route of a nested router too, so compare it to the full request path
        let uri = parts.extensions.get::<OriginalUri>().map_or(&parts.uri, |it| &it.0);
        let invalid = || ServerError::InvalidInput("the file path is not a base64 encoded path".to_owned());
        let encoded = uri.path().strip_prefix(prefix).ok_or_else(invalid)?;
        let encoded = percent_decode_str(encoded).decode_utf8().map_err(|_| invalid())?.into_owned();
        if encoded.is_empty() {
            return Err(ServerError::InvalidInput("the file path is empty".to_owned()));
        }
        let path = BASE64_STANDARD
            .decode(&encoded)
            .ok()
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .ok_or_else(invalid)?;
        Ok(Base64Path(path))
    }
}

impl ParameterProvider for Base64Path {
    fn generate(url: String) -> Either<Vec<Parameter>, RequestBody> {
        // documented like the `Path<(String,)>` it replaces: one string parameter, named after the `:file_path`
        <Path<(String,)> as ParameterProvider>::generate(url)
    }
}

pub struct Query<T>(pub T);

/// the longest name of a file, in bytes, most file systems hold
const MAX_FILE_NAME_BYTES: usize = 255;

/// where an uploaded file is saved, relative to the ledger's root: `attachments/<a new uuid v4>/<file name>`, with the
/// id of the folder
pub(crate) fn attachment_path(file_name: &str) -> (uuid::Uuid, String) {
    let id = uuid::Uuid::new_v4();
    let path = std::path::Path::new("attachments").join(id.to_string()).join(file_name);
    (id, path.to_string_lossy().to_string())
}

/// the files of an upload, each with its name, read before the ledger is held to write them. A file is saved by its
/// name under the ledger's attachments, so the name is a plain file name the file systems hold: the upload is a 400
/// otherwise, or when it cannot be read
pub(crate) async fn uploaded_files(multipart: &mut axum::extract::Multipart) -> crate::ServerResult<Vec<(String, axum::body::Bytes)>> {
    let unreadable = |e: axum::extract::multipart::MultipartError| crate::error::ServerError::InvalidInput(format!("the upload cannot be read: {e}"));
    let mut files = vec![];
    while let Some(field) = multipart.next_field().await.map_err(unreadable)? {
        let file_name = field.file_name().unwrap_or_default().to_owned();
        if std::path::Path::new(&file_name).file_name().and_then(|it| it.to_str()) != Some(file_name.as_str()) || file_name.contains('\0') {
            return Err(crate::error::ServerError::InvalidInput(format!("{file_name:?} is not the name of a file")));
        }
        if file_name.len() > MAX_FILE_NAME_BYTES {
            return Err(crate::error::ServerError::InvalidInput(format!(
                "the name of {file_name:?} is {} bytes long, longer than the {MAX_FILE_NAME_BYTES} a file name can be: rename it, and \
                 upload it again",
                file_name.len()
            )));
        }
        let content = field.bytes().await.map_err(unreadable)?;
        files.push((file_name, content));
    }
    Ok(files)
}

#[async_trait]
impl<T, S> FromRequestParts<S> for Query<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = ServerError;

    /// a query string that does not read as `T` is a 400 with the JSON error body of every other bad request, saying
    /// what it could not read
    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let query = parts.uri.query().unwrap_or_default();
        let params = serde_qs::from_str(query).map_err(|error| ServerError::InvalidInput(format!("the query string cannot be read: {error}")))?;
        Ok(Query(params))
    }
}

impl<T> ParameterProvider for Query<T>
where
    T: Schematic,
{
    fn generate(url: String) -> Either<Vec<Parameter>, RequestBody> {
        // documented like the `axum::extract::Query` it replaces: a parameter per field, required only when the field is
        <axum::extract::Query<T> as ParameterProvider>::generate(url)
    }
}

#[cfg(test)]
mod uploaded_files_test {
    use axum::body::Body;
    use axum::extract::{FromRequest, Multipart};
    use axum::http::Request;

    use super::uploaded_files;
    use crate::error::ServerError;

    async fn upload(file_names: &[&str]) -> crate::ServerResult<Vec<(String, axum::body::Bytes)>> {
        let mut body = String::new();
        for name in file_names {
            body.push_str(&format!(
                "--X\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{name}\"\r\nContent-Type: application/pdf\r\n\r\n%PDF {name}\r\n"
            ));
        }
        body.push_str("--X--\r\n");
        let request = Request::builder()
            .method("POST")
            .header("content-type", "multipart/form-data; boundary=X")
            .body(Body::from(body))
            .unwrap();
        let mut multipart = Multipart::from_request(request, &()).await.unwrap();
        uploaded_files(&mut multipart).await
    }

    #[tokio::test]
    async fn an_upload_is_read_as_its_files_each_with_a_plain_name() {
        let files = upload(&["statement.pdf", "收据 1.pdf"]).await.unwrap();
        let names = files.iter().map(|(name, content)| (name.as_str(), content.as_ref())).collect::<Vec<_>>();
        assert_eq!(
            names,
            vec![
                ("statement.pdf", b"%PDF statement.pdf".as_slice()),
                ("收据 1.pdf", "%PDF 收据 1.pdf".as_bytes())
            ]
        );
        // a name that would be saved elsewhere than the ledger's attachments is a 400, not a panic
        for name in ["../main.zhang", "/etc/passwd", "a/b.pdf", "..", ""] {
            match upload(&["ok.pdf", name]).await {
                Err(ServerError::InvalidInput(message)) => assert!(message.contains("is not the name of a file"), "{message}"),
                other => panic!("{name:?}: {:?}", other.map(|files| files.len())),
            }
        }
        // a NUL, which no file system takes, is refused already when the upload is read
        assert!(matches!(upload(&["a\0b.pdf"]).await, Err(ServerError::InvalidInput(_))));
        // so is a name too long for the file systems, in bytes: 85 characters of 3 bytes each are 255
        let longest = format!("{}.pdf", "a".repeat(251));
        assert_eq!(upload(&[&longest]).await.unwrap()[0].0, longest);
        assert_eq!(upload(&[&"收".repeat(85)]).await.unwrap().len(), 1);
        for name in [format!("{}.pdf", "a".repeat(252)), format!("{}.pdf", "收".repeat(84))] {
            match upload(&["ok.pdf", &name]).await {
                Err(ServerError::InvalidInput(message)) => {
                    assert!(message.contains(&format!("is {} bytes long, longer than the 255", name.len())), "{message}")
                }
                other => panic!("{name:?}: {:?}", other.map(|files| files.len())),
            }
        }
    }
}
