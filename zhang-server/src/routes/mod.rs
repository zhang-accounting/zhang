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

use axum::{
    async_trait,
    extract::FromRequestParts,
    http::{request::Parts, StatusCode},
};
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
