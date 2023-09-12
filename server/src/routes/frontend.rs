use std::path::PathBuf;
use std::str::FromStr;

use actix_web::{HttpRequest, HttpResponse, Responder};

pub async fn serve_frontend(uri: actix_web::http::Uri) -> impl Responder {
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

impl<T> Responder for StaticFile<T>
where
    T: Into<String>,
{
    type Body = actix_web::body::BoxBody;

    fn respond_to(self, _req: &HttpRequest) -> HttpResponse<Self::Body> {
        let path: String = self.0.into();
        match Asset::get(path.as_str()) {
            Some(content) => {
                let mime = mime_guess::from_path(path).first_or_octet_stream();
                HttpResponse::Ok()
                    .content_type(mime)
                    .body(actix_web::body::BoxBody::new(content.data.into_owned()))
            }
            None => HttpResponse::NotFound().finish(),
        }
    }
}
