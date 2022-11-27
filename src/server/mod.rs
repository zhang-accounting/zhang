use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::Arc;

use async_graphql::{EmptySubscription, Schema};
use log::{debug, error, info};
use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc::{channel, Receiver};
use tokio::sync::RwLock;

use model::query::QueryRoot;
use actix_web::{App, HttpServer, web};
use actix_web::web::Data;
use crate::cli::ServerOpts;
use crate::core::ledger::Ledger;
use crate::error::ZhangResult;
use crate::server::model::mutation::MutationRoot;
use crate::server::route::{create_account_balance, download_document, get_account_documents, get_account_journals, get_account_list, get_all_commodities, get_documents, get_files, get_single_commodity, serve_frontend, upload_account_document};

pub mod model;
pub mod request;
pub mod response;
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

pub async fn serve(opts: ServerOpts) -> ZhangResult<()> {
    let database = opts.database.clone().unwrap_or_else(|| opts.path.join("data.db"));
    let ledger = Ledger::load_with_database(opts.path.clone(), opts.endpoint.clone(), database).await?;
    let ledger_data = Arc::new(RwLock::new(ledger));

    let cloned_ledger = ledger_data.clone();
    tokio::spawn(async move {
        let (mut watcher, mut rx) = async_watcher().unwrap();

        let entry_path = {
            let guard1 = cloned_ledger.read().await;
            guard1.entry.0.clone()
        };
        info!("watching {}", &entry_path.to_str().unwrap_or(""));
        watcher
            .watch(entry_path.as_path(), RecursiveMode::Recursive)
            .expect("cannot watch entry path");

        while let Some(res) = rx.recv().await {
            match res {
                Ok(event) => {
                    debug!("receive file event: {:?}", event);
                    let is_visited_file_updated = {
                        let guard = cloned_ledger.read().await;
                        let x = guard.visited_files.iter().any(|file| event.paths.contains(file));
                        drop(guard);
                        x
                    };
                    if is_visited_file_updated {
                        debug!("gotcha event, start reloading...");
                        let mut guard = cloned_ledger.write().await;
                        debug!("watcher: got the lock");
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
    let addr = SocketAddrV4::new(Ipv4Addr::new(0, 0, 0, 0), opts.port);
    info!("zhang is listening on http://127.0.0.1:{}/", opts.port);
    HttpServer::new(move || {
        App::new()
            .app_data(Data::new(ledger_data.clone()))
            .service(get_account_list)
            .service(get_account_documents)
            .service(get_account_journals)
            .service(upload_account_document)
            .service(create_account_balance)
            .service(get_documents)
            .service(download_document)
            .service(get_all_commodities)
            .service(get_single_commodity)
            .service(get_files)
            .default_service(web::to(serve_frontend))
    })
        .bind(addr)?
        .run()
        .await;

    Ok(())
}
