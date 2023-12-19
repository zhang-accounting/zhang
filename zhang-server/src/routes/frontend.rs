use std::path::PathBuf;
use std::str::FromStr;

use actix_web::http::header;
use actix_web::{HttpRequest, HttpResponse, Responder};
use axum::body::Body;
use axum::http::{HeaderValue, StatusCode, Uri};
use axum::response::{IntoResponse, Response};

pub async fn serve_frontend(uri: Uri) -> impl IntoResponse {
    let path = uri.path().trim_start_matches('/').to_string();
    let buf = PathBuf::from_str(&path).unwrap();
    if buf.extension().is_some() {
        StaticFile(path)
    } else {
        StaticFile("index.html".to_string())
    }
}

#[derive(rust_embed::RustEmbed)]
#[folder = "../frontend/build"]
struct Asset;

pub struct StaticFile<T>(pub T);

impl<T> IntoResponse for StaticFile<T>
where
    T: Into<String>,
{
    fn into_response(self) -> Response {
        let path: String = self.0.into();

        match Asset::get(path.as_str()) {
            Some(content) => {
                let mime = mime_guess::from_path(path).first_or_octet_stream();
                let result1 = HeaderValue::from_str(mime.as_ref()).unwrap();
                let mut response1 = Body::new(content).into_response();
                response1.headers_mut().append(header::CONTENT_TYPE, result1);
                response1
            }
            None => {
                let mut response = Response::default();
                *response.status_mut() = StatusCode::NOT_FOUND;
                response
            }
        }
    }
}
