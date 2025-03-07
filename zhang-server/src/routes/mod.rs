pub mod account;
pub mod budget;
pub mod commodity;
pub mod common;
pub mod document;
pub mod file;
pub mod statistics;
pub mod transaction;

pub mod plugin;

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
