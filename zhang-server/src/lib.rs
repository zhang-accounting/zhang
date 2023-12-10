use std::net::SocketAddrV4;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use actix_cors::Cors;
use actix_web::middleware::Condition;
use actix_web::web::Data;
use actix_web::{App, HttpServer};
use actix_web_httpauth::extractors::AuthenticationError;
use actix_web_httpauth::headers::authorization::Basic;
use actix_web_httpauth::headers::www_authenticate::basic::Basic as BasicChangelle;
use actix_web_httpauth::middleware::HttpAuthentication;
use itertools::Itertools;
use log::{debug, error, info, trace, warn};
use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};
use self_update::version::bump_is_greater;
use serde::Serialize;
use sha3::{Digest, Sha3_256};
use tokio::sync::mpsc::error::TryRecvError;
use tokio::sync::mpsc::{channel, Receiver};
use tokio::sync::RwLock;

use routes::account::{
    create_account_balance, create_batch_account_balances, get_account_documents, get_account_info, get_account_journals, get_account_list,
    upload_account_document,
};
use routes::commodity::{get_all_commodities, get_single_commodity};
use routes::common::{get_all_options, get_basic_info, get_errors, sse};
use routes::document::{download_document, get_documents};
use routes::file::{get_file_content, get_files, update_file_content};
use routes::transaction::{create_new_transaction, get_info_for_new_transactions, get_journals, upload_transaction_document};
use zhang_core::exporter::AppendableExporter;
use zhang_core::ledger::Ledger;
use zhang_core::transform::Transformer;
use zhang_core::utils::has_path_visited;
use zhang_core::ZhangResult;

use crate::broadcast::{BroadcastEvent, Broadcaster};
use crate::error::ServerError;
use crate::response::ResponseWrapper;

pub mod broadcast;
pub mod error;
pub mod request;
pub mod response;
pub mod routes;
pub mod util;

pub type ServerResult<T> = Result<T, ServerError>;

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

pub struct ServeConfig {
    pub path: PathBuf,
    pub endpoint: String,
    pub addr: String,
    pub port: u16,
    pub no_report: bool,
    pub exporter: Arc<dyn AppendableExporter>,
    pub transformer: Arc<dyn Transformer>,
    pub auth_credential: Option<String>,
}

pub async fn serve(opts: ServeConfig) -> ZhangResult<()> {
    info!("version: {}, build date: {}", env!("CARGO_PKG_VERSION"), env!("ZHANG_BUILD_DATE"));
    let ledger = Ledger::load_with_database(opts.path.clone(), opts.endpoint.clone(), opts.transformer.clone())?;
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
        watcher.watch(entry_path.as_path(), RecursiveMode::Recursive).expect("cannot watch entry path");
        'looper: loop {
            let mut all = vec![];
            match rx.recv().await {
                Some(event) => all.push(event),
                None => break 'looper,
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
            'each_time: loop {
                let result = rx.try_recv();
                match result {
                    Ok(event) => {
                        all.push(event);
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                    Err(TryRecvError::Empty) => break 'each_time,
                    Err(TryRecvError::Disconnected) => break 'looper,
                }
            }
            trace!("receive all file changes: {:?}", all);
            let guard = cloned_ledger.read().await;
            let is_visited_file_updated = all
                .into_iter()
                .filter_map(|event| event.ok())
                .filter(|event| {
                    let include_visited_files = event.paths.iter().any(|path| has_path_visited(&guard.visited_files, path));
                    include_visited_files && event.kind.is_modify()
                })
                .count()
                > 0;

            drop(guard);

            if is_visited_file_updated {
                debug!("gotcha event, start reloading...");
                let mut guard = cloned_ledger.write().await;
                debug!("watcher: got the lock");
                info!("receive file event and reload ledger");
                let start_time = Instant::now();
                match guard.reload() {
                    Ok(_) => {
                        let duration = start_time.elapsed();
                        info!("ledger is reloaded successfully in {:?}", duration);
                        cloned_broadcaster.broadcast(BroadcastEvent::Reload).await;
                    }
                    Err(err) => {
                        error!("error on reload: {}", err)
                    }
                };
            }
        }
    });
    let update_checker_broadcaster = broadcaster.clone();
    tokio::spawn(async move {
        let mut report_interval = tokio::time::interval(Duration::from_secs(60));
        loop {
            report_interval.tick().await;
            update_checker(update_checker_broadcaster.clone()).await.ok();
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
    start_server(opts, ledger_data, broadcaster.clone()).await
}

pub async fn start_server(opts: ServeConfig, ledger_data: Arc<RwLock<Ledger>>, broadcaster: Arc<Broadcaster>) -> ZhangResult<()> {
    let addr = SocketAddrV4::new(opts.addr.parse()?, opts.port);
    info!("zhang is listening on http://{}:{}/", opts.addr, opts.port);

    let basic_credential = opts.auth_credential.map(|credential| {
        let token_part = credential.splitn(2, ':').map(|it| it.to_owned()).collect_vec();
        Basic::new(
            token_part.get(0).cloned().expect("cannot retrieve credential user_id"),
            token_part.get(1).cloned(),
        )
    });
    if basic_credential.is_some() {
        info!("web basic auth is enabled");
    }
    let exporter: Data<dyn AppendableExporter> = Data::from(opts.exporter);
    let server = HttpServer::new(move || {
        let auth = HttpAuthentication::basic(|req, credentials| async move {
            let option = req.app_data::<Data<Option<Basic>>>().unwrap();
            let mut hasher = Sha3_256::new();
            hasher.update(credentials.password().unwrap_or_default().as_bytes());
            let array = hasher.finalize();
            let hex_hash = &array[..].into_iter().map(|it| format!("{:02x}", it)).join("");
            if let Some(basic) = option.as_ref() {
                let pass = credentials.user_id().eq(basic.user_id()) && hex_hash.eq(basic.password().unwrap_or_default());
                if pass {
                    Ok(req)
                } else {
                    warn!(
                        "web basic auth validation fail with user_id: {}, password: {}",
                        credentials.user_id(),
                        credentials.password().unwrap_or_default()
                    );
                    Err((AuthenticationError::new(BasicChangelle::new()).into(), req))
                }
            } else {
                Ok(req)
            }
        });

        let web_auth = Condition::new(basic_credential.is_some(), auth);
        let app = App::new()
            .app_data(Data::new(basic_credential.clone()))
            .wrap(Cors::permissive())
            .wrap(web_auth)
            .app_data(Data::from(broadcaster.clone()))
            .app_data(Data::new(ledger_data.clone()))
            .app_data(exporter.clone())
            .service(get_basic_info)
            .service(routes::common::get_store_data)
            .service(get_info_for_new_transactions)
            .service(get_journals)
            .service(create_new_transaction)
            .service(get_account_list)
            .service(get_account_info)
            .service(get_account_documents)
            .service(get_account_journals)
            .service(upload_account_document)
            .service(upload_transaction_document)
            .service(create_account_balance)
            .service(create_batch_account_balances)
            .service(get_documents)
            .service(download_document)
            .service(get_all_commodities)
            .service(get_single_commodity)
            .service(get_files)
            .service(get_file_content)
            .service(update_file_content)
            .service(get_errors)
            .service(get_all_options)
            .service(routes::statistics::get_statistic_summary)
            .service(routes::statistics::get_statistic_graph)
            .service(routes::statistics::get_statistic_rank_detail_by_account_type)
            .service(routes::budget::get_budget_list)
            .service(routes::budget::get_budget_info)
            .service(routes::budget::get_budget_interval_detail)
            .service(sse);

        #[cfg(feature = "frontend")]
        {
            app.default_service(actix_web::web::to(routes::frontend::serve_frontend))
        }

        #[cfg(not(feature = "frontend"))]
        {
            app
        }
    })
    .bind(addr)?;
    Ok(server.run().await?)
}

async fn version_report_task() -> ServerResult<()> {
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

async fn update_checker(broadcast: Arc<Broadcaster>) -> ServerResult<()> {
    if broadcast.client_number().await < 1 {
        return Ok(());
    }

    let latest_release = tokio::task::spawn_blocking(move || {
        self_update::backends::github::Update::configure()
            .repo_owner("zhang-accounting")
            .repo_name("zhang")
            .bin_name("zhang")
            .current_version(env!("CARGO_PKG_VERSION"))
            .build()
            .unwrap()
            .get_latest_release()
    })
    .await
    .expect("cannot spawn update checker task");
    if let Ok(release) = latest_release {
        if bump_is_greater(env!("CARGO_PKG_VERSION"), &release.version).unwrap_or(false) {
            broadcast.broadcast(BroadcastEvent::NewVersionFound { version: release.version }).await;
        }
    }
    Ok(())
}

pub type ApiResult<T> = ServerResult<ResponseWrapper<T>>;
