use std::net::SocketAddrV4;
use std::ops::Deref;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::DefaultBodyLimit;
use axum::routing::{get, post, put};
use axum::Router;
use gotcha::{ConfigWrapper, GotchaApp, GotchaContext, GotchaRouter, config::BasicConfig};
use itertools::Itertools;
use log::{debug, error, info, trace};
use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};
use routes::account::*;
use routes::budget::*;
use routes::commodity::*;
use routes::common::*;
use routes::document::*;
use routes::file::*;
use routes::statistics::*;
use routes::transaction::*;
use self_update::version::bump_is_greater;
use serde::Serialize;
use state::{SharedBroadcaster, SharedLedger, SharedReloadSender};
use tokio::sync::mpsc::error::TryRecvError;
use tokio::sync::mpsc::{channel, Receiver, Sender};
use tokio::sync::{mpsc, RwLock};
use tower_http::cors::CorsLayer;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::validate_request::ValidateRequestHeaderLayer;
use zhang_core::data_source::DataSource;
use zhang_core::ledger::Ledger;
use zhang_core::utils::has_path_visited;
use zhang_core::ZhangResult;

use crate::broadcast::{BroadcastEvent, Broadcaster};
use crate::error::ServerError;
use crate::response::ResponseWrapper;
use crate::state::AppState;

pub mod broadcast;
pub mod error;
pub mod request;
pub mod response;
pub mod routes;
pub mod util;
pub mod tasks;
pub mod state;

pub type LedgerState = Arc<RwLock<Ledger>>;

pub type ServerResult<T> = Result<T, ServerError>;

pub type ApiResult<T> = ServerResult<ResponseWrapper<T>>;

pub struct ServerApp {
    ledger: Arc<RwLock<Ledger>>,
    broadcaster: Arc<Broadcaster>,
    reload_sender: Arc<ReloadSender>,
    auth_credential: Option<String>,
}

impl GotchaApp for ServerApp {
    type State = AppState;

    type Config = ();

    async fn config(&self) -> Result<ConfigWrapper<Self::Config>, Box<dyn std::error::Error>> {
        Ok(ConfigWrapper{
            basic: BasicConfig{
                host: "0.0.0.0".to_string(),
                port: 8080,
            },
            application: (),
        })
    }

    fn routes(
        &self, router: GotchaRouter<GotchaContext<Self::State, Self::Config>>,
    ) -> GotchaRouter<GotchaContext<Self::State, Self::Config>> {

        let basic_credential = self.auth_credential.as_ref().map(|credential| {
            let token_part = credential.splitn(2, ':').map(|it| it.to_owned()).collect_vec();
            (
                token_part.first().cloned().expect("cannot retrieve credential user_id"),
                token_part.get(1).cloned(),
            )
        });

        let router = router
            .get("/api/sse", sse)
            .post("/api/reload", reload)
            .get("/api/info", get_basic_info)
            .get("/api/store", get_store_data)
            .get("/api/options", get_all_options)
            .get("/api/errors", get_errors)
            .get("/api/files", get_files)
            .get("/api/files/:file_path", get_file_content)
            .put("/api/files/:file_path", update_file_content)
            .get("/api/for-new-transaction", get_info_for_new_transactions)
            .get("/api/journals", get_journals)
            .post("/api/transactions", create_new_transaction)
            .put("/api/transactions/:transaction_id", update_single_transaction)
            .post("/api/transactions/:transaction_id/documents", upload_transaction_document)
            .get("/api/accounts", get_account_list)
            .get("/api/accounts/:account_name", get_account_info)
            .post("/api/accounts/:account_name/documents", upload_account_document)
            .get("/api/accounts/:account_name/documents", get_account_documents)
            .get("/api/accounts/:account_name/journals", get_account_journals)
            .get("/api/accounts/:account_name/balances", get_account_balance_data)
            .route("/api/accounts/:account_name/balances", post(create_account_balance))
            .post("/api/accounts/batch-balances", create_batch_account_balances)
            .get("/api/documents", get_documents)
            .get("/api/documents/:file_path", download_document)
            .get("/api/commodities", get_all_commodities)
            .get("/api/commodities/:commodity_name", get_single_commodity)
            .get("/api/statistic/summary", get_statistic_summary)
            .get("/api/statistic/graph", get_statistic_graph)
            .get("/api/statistic/:account_type", get_statistic_rank_detail_by_account_type)
            .get("/api/budgets", get_budget_list)
            .get("/api/budgets/:budget_name", get_budget_info)
            .get("/api/budgets/:budget_name/interval/:year/:month", get_budget_interval_detail)
            .get("/api/plugins", routes::plugin::plugin_list)
            .layer(CorsLayer::permissive())
            .layer(DefaultBodyLimit::disable())
            .layer(RequestBodyLimitLayer::new(250 * 1024 * 1024 /* 250mb */));

        let router = if let Some((username, password)) = basic_credential {
                info!("web basic auth is enabled with username {}", &username);
                router.layer(ValidateRequestHeaderLayer::basic(&username, password.as_deref().unwrap_or_default()))
            } else {
                router
            };
        #[cfg(feature = "frontend")]
        {
            router.fallback(routes::frontend::serve_frontend)
        }
        #[cfg(not(feature = "frontend"))]
        {
            router.fallback(routes::common::backend_only_info)
        }
    }

    async fn state(&self, config: &gotcha::ConfigWrapper<Self::Config>) -> Result<Self::State, Box<dyn std::error::Error>> {
        Ok(AppState {
            ledger: SharedLedger(self.ledger.clone()),
            broadcaster: SharedBroadcaster(self.broadcaster.clone()),
            reload_sender: SharedReloadSender(self.reload_sender.clone()),
        })
    }
}

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
    pub data_source: Arc<dyn DataSource>,
    pub auth_credential: Option<String>,
    pub is_local_fs: bool,
}

pub struct ReloadSender(pub Sender<i32>);

impl Deref for ReloadSender {
    type Target = Sender<i32>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl ReloadSender {
    fn reload(&self) {
        self.0.try_send(1).ok();
    }
}

pub async fn serve(opts: ServeConfig) -> ZhangResult<()> {
    info!("version: {}, build date: {}", env!("ZHANG_BUILD_VERSION"), env!("ZHANG_BUILD_DATE"));
    let ledger = Ledger::async_load(opts.path.clone(), opts.endpoint.clone(), opts.data_source.clone()).await?;
    let ledger_data = Arc::new(RwLock::new(ledger));
    let broadcaster = Broadcaster::create();
    let (tx, rx) = mpsc::channel::<i32>(1);
    let reload_sender = Arc::new(ReloadSender(tx));

    info!("start reload listener");
    start_reload_listener(ledger_data.clone(), broadcaster.clone(), rx);

    if opts.is_local_fs {
        info!("start fs event listener");
        start_fs_event_lisenter(ledger_data.clone(), reload_sender.clone());
    }

    info!("start version report tasker");
    start_version_check_tasker(broadcaster.clone());

    if !opts.no_report {
        start_report_tasker();
    }
    start_server(opts, ledger_data, broadcaster.clone(), reload_sender.clone()).await
}

fn start_report_tasker() {
    tokio::spawn(async move {
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

fn start_version_check_tasker(update_checker_broadcaster: Arc<Broadcaster>) {
    tokio::spawn(async move {
        let mut report_interval = tokio::time::interval(Duration::from_secs(60));
        loop {
            report_interval.tick().await;
            update_checker(update_checker_broadcaster.clone()).await.ok();
        }
    });
}

fn start_fs_event_lisenter(cloned_ledger: Arc<RwLock<Ledger>>, reload_sender_for_fs: Arc<ReloadSender>) {
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
                debug!("gotcha event, sending reload event...");
                reload_sender_for_fs.0.try_send(1).ok();
            }
        }
    });
}

fn start_reload_listener(ledger_for_reload: Arc<RwLock<Ledger>>, cloned_broadcaster: Arc<Broadcaster>, mut rx: Receiver<i32>) {
    tokio::spawn(async move {
        while rx.recv().await.is_some() {
            info!("start reloading...");
            let start_time = Instant::now();
            let mut guard = ledger_for_reload.write().await;
            match guard.async_reload().await {
                Ok(_) => {
                    let duration = start_time.elapsed();
                    info!("ledger is reloaded successfully in {:?}", duration);
                    // todo: add reload duration to reload event
                    cloned_broadcaster.broadcast(BroadcastEvent::Reload).await;
                }
                Err(err) => {
                    error!("error on reload: {}", err);
                    // todo: broadcast the error
                }
            }
            drop(guard);
        }
    });
}

pub async fn start_server(
    opts: ServeConfig, ledger_data: Arc<RwLock<Ledger>>, broadcaster: Arc<Broadcaster>, reload_sender: Arc<ReloadSender>,
) -> ZhangResult<()> {
    let addr = SocketAddrV4::new(opts.addr.parse()?, opts.port);
    info!("zhang is listening on http://{}:{}/", opts.addr, opts.port);

    let app = create_server_app(ledger_data, broadcaster, reload_sender, opts.auth_credential);
    app.run().await.unwrap();
    Ok(())
}

pub fn create_server_app(
    ledger: Arc<RwLock<Ledger>>, broadcaster: Arc<Broadcaster>, reload_sender: Arc<ReloadSender>, auth_credential: Option<String>,
) -> ServerApp {
    let app = ServerApp {
        ledger,
        broadcaster,
        reload_sender,
        auth_credential,
    };
      app  
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
        .post("https://zhang-cloud.kilerd.me/client_report")
        .json(&VersionReport {
            version: env!("ZHANG_BUILD_VERSION"),
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
            .current_version(env!("ZHANG_BUILD_VERSION"))
            .build()
            .unwrap()
            .get_latest_release()
    })
    .await
    .expect("cannot spawn update checker task");
    if let Ok(release) = latest_release {
        if bump_is_greater(env!("ZHANG_BUILD_VERSION"), &release.version).unwrap_or(false) {
            broadcast.broadcast(BroadcastEvent::NewVersionFound { version: release.version }).await;
        }
    }
    Ok(())
}
