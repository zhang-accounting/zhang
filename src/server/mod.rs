use crate::cli::ServerOpts;
use crate::core::ledger::Ledger;
use crate::error::ZhangResult;
use async_graphql::{EmptyMutation, EmptySubscription, Schema};
use axum::{routing::get, AddExtensionLayer, Router};
use log::info;
use model::QueryRoot;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::Arc;
use tokio::sync::RwLock;
use tower_http::cors::CorsLayer;

pub mod model;
pub mod route;

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
        .route(
            "/graphql",
            get(route::graphql_playground).post(route::graphql_handler),
        )
        .fallback(get(route::serve_frontend))
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
