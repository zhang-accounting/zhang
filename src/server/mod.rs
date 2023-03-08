use std::net::{Ipv4Addr, SocketAddrV4};
use std::sync::Arc;
use std::time::{Duration, Instant};

use actix_cors::Cors;
use actix_web::web::Data;
use actix_web::{web, App, HttpServer};
use log::{debug, error, info};
use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};
use serde::Serialize;
use tokio::sync::mpsc::{channel, Receiver};
use tokio::sync::RwLock;

use crate::cli::ServerOpts;
use crate::core::ledger::Ledger;
use crate::error::ZhangResult;
use crate::server::broadcast::Broadcaster;
use crate::server::route::{create_account_balance, create_new_transaction, current_statistic, download_document, get_account_documents, get_account_journals, get_account_list, get_all_commodities, get_basic_info, get_documents, get_errors, get_file_content, get_files, get_info_for_new_transactions, get_journals, get_report, get_single_commodity, get_statistic_data, serve_frontend, sse, update_file_content, upload_account_document, upload_transaction_document};

pub mod request;
pub mod response;
pub mod route;
pub mod broadcast;

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
    info!(
        "version: {}, build date: {}",
        env!("CARGO_PKG_VERSION"),
        env!("ZHANG_BUILD_DATE")
    );
    let database = opts.database.clone();
    let ledger = Ledger::load_with_database(opts.path.clone(), opts.endpoint.clone(), database).await?;
    let ledger_data = Arc::new(RwLock::new(ledger));

    let cloned_ledger = ledger_data.clone();
    let broadcaster = Broadcaster::create();
    let cloned_broadcaster = broadcaster.clone();
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
                        info!("receive file event and reload ledger: {:?}", event);
                        let start_time = Instant::now();
                        match guard.reload().await {
                            Ok(_) => {
                                let duration = start_time.elapsed();
                                info!("ledger is reloaded successfully in {:?}", duration);
                                cloned_broadcaster.broadcast("reloaded").await;
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

    if !opts.no_report {
        tokio::spawn(async {
            let mut report_interval = tokio::time::interval(Duration::from_secs(60 * 60));
            info!("start zhang's version report task");
            loop {
                report_interval.tick().await;
                match version_report_task().await {
                    Ok(_) => {
                        debug!("report zhang's version successfully");
                    }
                    Err(e) => {
                        debug!("fail to report zhang's version: {}", e);
                    }
                }
            }
        });
    }
    start_server(opts, ledger_data, broadcaster).await
}

async fn start_server(opts: ServerOpts, ledger_data: Arc<RwLock<Ledger>>, broadcaster: Arc<Broadcaster>) -> ZhangResult<()> {
    let addr = SocketAddrV4::new(Ipv4Addr::new(0, 0, 0, 0), opts.port);
    info!("zhang is listening on http://127.0.0.1:{}/", opts.port);
    Ok(HttpServer::new(move || {
        App::new()
            .wrap(Cors::permissive())
            .app_data(Data::from(broadcaster.clone()))
            .app_data(Data::new(ledger_data.clone()))
            .service(get_basic_info)
            .service(get_info_for_new_transactions)
            .service(get_statistic_data)
            .service(current_statistic)
            .service(get_journals)
            .service(create_new_transaction)
            .service(get_account_list)
            .service(get_account_documents)
            .service(get_account_journals)
            .service(upload_account_document)
            .service(upload_transaction_document)
            .service(create_account_balance)
            .service(get_documents)
            .service(download_document)
            .service(get_all_commodities)
            .service(get_single_commodity)
            .service(get_files)
            .service(get_file_content)
            .service(update_file_content)
            .service(get_report)
            .service(get_errors)
            .service(sse)
            .default_service(web::to(serve_frontend))
    })
    .bind(addr)?
    .run()
    .await?)
}

async fn version_report_task() -> ZhangResult<()> {
    #[derive(Serialize)]
    struct VersionReport<'a> {
        version: &'a str,
        build_date: &'a str,
    }
    debug!("reporting zhang's version");
    let client = reqwest::Client::new();
    client
        .post("https://zhang.resource.rs")
        .json(&VersionReport {
            version: env!("CARGO_PKG_VERSION"),
            build_date: env!("ZHANG_BUILD_DATE"),
        })
        .timeout(Duration::from_secs(10))
        .send()
        .await?;
    Ok(())
}
