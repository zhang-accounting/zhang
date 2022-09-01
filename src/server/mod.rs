use crate::cli::ServerOpts;
use crate::core::ledger::Ledger;
use crate::error::ZhangResult;
use crate::server::model::mutation::MutationRoot;
use async_graphql::{EmptySubscription, Schema};
use axum::extract::Extension;
use axum::routing::get;
use axum::Router;
use log::{error, info};
use model::query::QueryRoot;
use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::Arc;
use tokio::sync::mpsc::{channel, Receiver};
use tokio::sync::RwLock;
use tower_http::cors::CorsLayer;

pub mod model;
pub mod route;

pub type LedgerState = Arc<RwLock<Ledger>>;

fn async_watcher() -> notify::Result<(RecommendedWatcher, Receiver<notify::Result<Event>>)> {
    let (tx, rx) = channel(1);

    // Automatically select the best implementation for your platform.
    // You can also access each implementation directly e.g. INotifyWatcher.
    let watcher = RecommendedWatcher::new(
        move |res| {
            futures::executor::block_on(async {
                tx.send(res).await.unwrap();
            })
        },
        Config::default(),
    )?;

    Ok((watcher, rx))
}

pub fn serve(opts: ServerOpts) -> ZhangResult<()> {
    let ledger = Ledger::load(opts.path.clone(), opts.endpoint.clone())?;
    let ledger_data = Arc::new(RwLock::new(ledger));

    let runtime = tokio::runtime::Runtime::new()?;
    let cloned_ledger = ledger_data.clone();
    runtime.spawn(async move {
        let (mut watcher, mut rx) = async_watcher().unwrap();
        for x in &cloned_ledger.read().await.visited_files {
            println!("watching {:?}", &x.to_str());
            watcher
                .watch(x, RecursiveMode::NonRecursive)
                .expect("cannot watch file");
        }
        while let Some(res) = rx.recv().await {
            match res {
                Ok(event) => {
                    println!("changed: {:?}", event);
                    let mut guard = cloned_ledger.write().await;
                    match guard.reload() {
                        Ok(_) => {
                            info!("reloaded")
                        }
                        Err(err) => {
                            error!("error on reload: {}", err)
                        }
                    };
                }
                Err(e) => println!("watch error: {:?}", e),
            }
        }
    });
    runtime.block_on(start_server(opts, ledger_data))
}
async fn start_server(opts: ServerOpts, ledger_data: Arc<RwLock<Ledger>>) -> ZhangResult<()> {
    let schema = Schema::build(QueryRoot, MutationRoot, EmptySubscription)
        .data(ledger_data.clone())
        .finish();

    let app = Router::new()
        .route("/graphql", get(route::graphql_playground).post(route::graphql_handler))
        .fallback(get(route::serve_frontend))
        .layer(Extension(ledger_data))
        .layer(Extension(schema))
        .layer(CorsLayer::permissive());

    let addr = SocketAddrV4::new(Ipv4Addr::new(0, 0, 0, 0), opts.port);
    info!("zhang is listening on http://127.0.0.1:{}/", opts.port);
    axum::Server::bind(&SocketAddr::from(addr))
        .serve(app.into_make_service())
        .await
        .unwrap();
    Ok(())
}
