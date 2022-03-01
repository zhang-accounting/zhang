use crate::cli::ServerOpts;
use crate::core::ledger::Ledger;
use crate::error::ZhangResult;
use async_graphql::http::{playground_source, GraphQLPlaygroundConfig};
use async_graphql::{Context, EmptyMutation, EmptySubscription, Object, Request, Schema};
use axum::body::{boxed, Full};
use axum::extract::Extension;
use axum::http::{header, Method, StatusCode, Uri};
use axum::response::{Html, IntoResponse, Response};
use axum::{routing::get, AddExtensionLayer, Json, Router};
use log::info;
use model::{LedgerSchema, QueryRoot};
use rust_embed::RustEmbed;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::RwLock;
use tower_http::cors::{CorsLayer, Origin};

pub mod model;

pub type LedgerState = Arc<RwLock<Ledger>>;

pub fn serve(opts: ServerOpts) -> ZhangResult<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    runtime.block_on(start_server(opts))
}
async fn start_server(opts: ServerOpts) -> ZhangResult<()> {
    let ledger = Ledger::load(opts.file)?;
    let ledger_data = Arc::new(RwLock::new(ledger));

    let schema = Schema::build(QueryRoot, EmptyMutation, EmptySubscription)
        .data(ledger_data.clone())
        .finish();

    let app = Router::new()
        .route("/graphql", get(graphql_playground).post(graphql_handler))
        .fallback(get(serve_frontend))
        .layer(AddExtensionLayer::new(ledger_data))
        .layer(AddExtensionLayer::new(schema))
        .layer(CorsLayer::permissive());

    let addr = SocketAddrV4::new(Ipv4Addr::new(0, 0, 0, 0), opts.port);
    info!("zhang is listening on http://127.0.0.1:{}/", opts.port);
    axum::Server::bind(&SocketAddr::from(addr))
        .serve(app.into_make_service())
        .await
        .unwrap();
    Ok(())
}

async fn graphql_playground() -> impl IntoResponse {
    Html(playground_source(GraphQLPlaygroundConfig::new("/graphql")))
}

async fn graphql_handler(
    schema: Extension<LedgerSchema>,
    req: Json<Request>,
) -> Json<async_graphql::Response> {
    schema.execute(req.0).await.into()
}

async fn serve_frontend(uri: Uri) -> impl IntoResponse {
    let mut path = uri.path().trim_start_matches('/').to_string();
    let buf = PathBuf::from_str(&path).unwrap();
    if buf.extension().is_some() {
        StaticFile(path)
    } else {
        StaticFile("index.html".to_string())
    }
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
