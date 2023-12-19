use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;
use thiserror::Error;
use zhang_ast::account::InvalidAccountError;
use zhang_core::ZhangError;

#[derive(Error, Debug)]
pub enum ServerError {
    #[error("core error: {0}")]
    CoreError(#[from] ZhangError),

    #[error("client error: {0}")]
    ClientError(#[from] reqwest::Error),

    #[error("io error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("io error: {0}")]
    StrumError(#[from] strum::ParseError),
}

impl From<InvalidAccountError> for ServerError {
    fn from(_value: InvalidAccountError) -> Self {
        Self::CoreError(ZhangError::InvalidAccount)
    }
}

impl IntoResponse for ServerError {
    fn into_response(self) -> Response {
        let payload = json!({
            "message": format!("{}", self),
            "origin": "with_rejection"
        });

        (StatusCode::INTERNAL_SERVER_ERROR, Json(payload)).into_response()
    }
}
