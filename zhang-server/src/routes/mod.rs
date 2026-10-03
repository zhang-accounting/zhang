pub mod account;
pub mod budget;
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
use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::StatusCode;
use gotcha::oas::{Parameter, ParameterIn, Referenceable, RequestBody};
use gotcha::{Either, ParameterProvider, Schematic};
use serde::de::DeserializeOwned;
use serde_qs;

pub struct Query<T>(pub T);

/// the files of an upload, each with its name, read before the ledger is held to write them. A file is saved by its
/// name under the ledger's attachments, so the name is a plain file name: the upload is a 400 otherwise, or when it
/// cannot be read
pub(crate) async fn uploaded_files(multipart: &mut axum::extract::Multipart) -> crate::ServerResult<Vec<(String, axum::body::Bytes)>> {
    let unreadable = |e: axum::extract::multipart::MultipartError| crate::error::ServerError::InvalidInput(format!("the upload cannot be read: {e}"));
    let mut files = vec![];
    while let Some(field) = multipart.next_field().await.map_err(unreadable)? {
        let file_name = field.file_name().unwrap_or_default().to_owned();
        if std::path::Path::new(&file_name).file_name().and_then(|it| it.to_str()) != Some(file_name.as_str()) {
            return Err(crate::error::ServerError::InvalidInput(format!("{file_name:?} is not the name of a file")));
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
    type Rejection = StatusCode;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let query = parts.uri.query().unwrap_or_default();
        let params = serde_qs::from_str(query).map_err(|_| StatusCode::BAD_REQUEST)?;
        Ok(Query(params))
    }
}

impl<T> ParameterProvider for Query<T>
where
    T: Schematic,
{
    fn generate(_url: String) -> Either<Vec<Parameter>, RequestBody> {
        let mut ret = vec![];
        let mut schema = T::generate_schema();
        if let Some(mut properties) = schema.schema.extras.remove("properties") {
            if let Some(properties) = properties.as_object_mut() {
                properties.iter_mut().for_each(|(key, value)| {
                    let schema = serde_json::from_value(value.clone()).unwrap();
                    let param = Parameter {
                        name: key.to_string(),
                        _in: ParameterIn::Query,
                        description: T::doc(),
                        required: Some(T::required()),
                        deprecated: None,
                        allow_empty_value: None,
                        style: None,
                        explode: None,
                        allow_reserved: None,
                        schema: Some(Referenceable::Data(schema)),
                        example: None,
                        examples: None,
                        content: None,
                    };
                    ret.push(param);
                })
            }
        }
        Either::Left(ret)
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
    }
}
