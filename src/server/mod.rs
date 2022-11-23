use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::Arc;

use async_graphql::{EmptySubscription, Schema};
use axum::extract::Extension;
use axum::routing::get;
use axum::Router;
use log::{debug, error, info};
use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc::{channel, Receiver};
use tokio::sync::RwLock;
use tower_http::cors::CorsLayer;

use model::query::QueryRoot;

use crate::cli::ServerOpts;
use crate::core::ledger::Ledger;
use crate::error::ZhangResult;
use crate::server::model::mutation::MutationRoot;
use crate::server::route::get_account_list;

pub mod model;
pub mod route;
pub mod request;
pub mod response;

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

pub async fn serve(opts: ServerOpts) -> ZhangResult<()> {
    let database = opts.database.clone().unwrap_or_else(|| opts.path.join("data.db"));
    let ledger = Ledger::load_with_database(opts.path.clone(), opts.endpoint.clone(), database).await?;
    let ledger_data = Arc::new(RwLock::new(ledger));

    let cloned_ledger = ledger_data.clone();
    tokio::spawn(async move {
        let (mut watcher, mut rx) = async_watcher().unwrap();

        let entry_path = &cloned_ledger.read().await.entry.0;
        info!("watching {}", &entry_path.to_str().unwrap_or(""));
        watcher
            .watch(entry_path, RecursiveMode::Recursive)
            .expect("cannot watch entry path");

        while let Some(res) = rx.recv().await {
            match res {
                Ok(event) => {
                    debug!("receive file event: {:?}", event);
                    let mut guard = cloned_ledger.write().await;
                    let is_visited_file_updated = guard.visited_files.iter().any(|file| event.paths.contains(file));
                    if is_visited_file_updated {
                        match guard.reload().await {
                            Ok(_) => {
                                info!("reloaded")
                            }
                            Err(err) => {
                                error!("error on reload: {}", err)
                            }
                        };
                    } else {
                        debug!("ignore file event: {:?}", event);
                    }
                }
                Err(e) => println!("watch error: {:?}", e),
            }
        }
    });
    start_server(opts, ledger_data).await
}

async fn start_server(opts: ServerOpts, ledger_data: Arc<RwLock<Ledger>>) -> ZhangResult<()> {
    let schema = Schema::build(QueryRoot, MutationRoot, EmptySubscription)
        .data(ledger_data.clone())
        .finish();

    let app = Router::new()
        .route("/graphql", get(route::graphql_playground).post(route::graphql_handler))
        .route("/files/:filename/preview", get(route::file_preview))
        .route("/api/accounts", get(get_account_list))
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
