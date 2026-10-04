pub mod account;
pub mod budget;
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
use axum::http::StatusCode;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;
use gotcha::oas::{Parameter, ParameterIn, Referenceable, RequestBody};
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
