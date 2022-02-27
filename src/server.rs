use crate::cli::ServerOpts;
use crate::error::ZhangResult;
use axum::body::{boxed, Full};
use axum::extract::Extension;
use axum::http::{header, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::{routing::get, AddExtensionLayer, Router, Json};
use log::info;
use rust_embed::RustEmbed;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::Arc;
use tokio::sync::RwLock;
use crate::core::ledger::Ledger;

pub type LedgerState = Arc<RwLock<Ledger>>;

async fn hello_world(Extension(ledger): Extension<LedgerState>) -> impl IntoResponse {
    let x = ledger.read().await;
    Json(x.snapshot.clone())
}

async fn serve_frontend(uri: Uri) -> impl IntoResponse {
    let mut path = uri.path().trim_start_matches('/').to_string();
    if path.is_empty() {
        path = "index.html".to_string();
    }
    StaticFile(path)
}

#[derive(RustEmbed)]
#[folder = "zhang-frontend/dist/modern"]
struct Asset;

pub struct StaticFile<T>(pub T);

impl<T> IntoResponse for StaticFile<T>
where
    T: Into<String>,
{
    fn into_response(self) -> Response {
        let path = self.0.into();
        match Asset::get(path.as_str()) {
            Some(content) => {
                let body = boxed(Full::from(content.data));
                let mime = mime_guess::from_path(path).first_or_octet_stream();
                Response::builder()
                    .header(header::CONTENT_TYPE, mime.as_ref())
                    .body(body)
                    .unwrap()
            }
            None => Response::builder()
                .status(StatusCode::NOT_FOUND)
                // .body(())
                .body(boxed(Full::from("404")))
                .unwrap(),
        }
    }
}

pub fn serve(opts: ServerOpts) -> ZhangResult<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    runtime.block_on(start_server(opts))
}
async fn start_server(opts: ServerOpts) -> ZhangResult<()> {
    let ledger = Ledger::load(opts.file)?;
    let ledger_data = Arc::new(RwLock::new(ledger));

    let app = Router::new()
        .route("/api", get(hello_world))
        .fallback(get(serve_frontend))
        .layer(AddExtensionLayer::new(ledger_data));

    let addr = SocketAddrV4::new(Ipv4Addr::new(0, 0, 0, 0), opts.port);
    info!("zhang is listening on http://127.0.0.1:{}/", opts.port);
    axum::Server::bind(&SocketAddr::from(addr))
        .serve(app.into_make_service())
        .await
        .unwrap();
    Ok(())
}
