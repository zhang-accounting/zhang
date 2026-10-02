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

    #[error("not found")]
    NotFound,

    #[error("bad request")]
    BadRequest,

    /// a query failed to parse, compile or run; answered with HTTP 400 and its source position
    #[error("query error: {0}")]
    QueryError(#[from] zhang_query::QueryError),
}

impl From<InvalidAccountError> for ServerError {
    fn from(_value: InvalidAccountError) -> Self {
        Self::CoreError(ZhangError::InvalidAccount)
    }
}

impl IntoResponse for ServerError {
    fn into_response(self) -> Response {
        if let ServerError::QueryError(error) = self {
            // the query error body is exactly `{message, line, column}`
            let payload = json!({
                "message": error.message,
                "line": error.line,
                "column": error.column,
            });
            return (StatusCode::BAD_REQUEST, Json(payload)).into_response();
        }
        let payload = json!({
            "message": format!("{}", self),
            "origin": "with_rejection"
        });

        let status = match self {
            ServerError::NotFound => StatusCode::NOT_FOUND,
            ServerError::BadRequest => StatusCode::BAD_REQUEST,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };

        (status, Json(payload)).into_response()
    }
}
