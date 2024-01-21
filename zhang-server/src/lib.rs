use std::net::SocketAddrV4;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::{DefaultBodyLimit, FromRef};
use axum::routing::{get, post, put};
use axum::Router;
use itertools::Itertools;
use log::{debug, error, info, trace};
use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};
use self_update::version::bump_is_greater;
use serde::Serialize;
use tokio::sync::mpsc::error::TryRecvError;
use tokio::sync::mpsc::{channel, Receiver, Sender};
use tokio::sync::{mpsc, RwLock};
use tower_http::cors::CorsLayer;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::validate_request::ValidateRequestHeaderLayer;
use uuid::Uuid;

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

use routes::account::*;
use routes::budget::*;
use routes::commodity::*;
use routes::common::*;
use routes::document::*;
use routes::file::*;
use routes::statistics::*;
use routes::transaction::*;
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
    pub transformer: Arc<dyn Transformer>,
    pub auth_credential: Option<String>,
    pub is_local_fs: bool,
}

pub struct ReloadSender(pub Sender<i32>);

impl ReloadSender {
    fn reload(&self) {
        self.0.try_send(1).ok();
    }
}

pub async fn serve(opts: ServeConfig) -> ZhangResult<()> {
    info!("version: {}, build date: {}", env!("ZHANG_BUILD_VERSION"), env!("ZHANG_BUILD_DATE"));
    let ledger = Ledger::async_load(opts.path.clone(), opts.endpoint.clone(), opts.transformer.clone()).await?;
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
    let uuid = Uuid::new_v4();
    tokio::spawn(async move {
        let mut report_interval = tokio::time::interval(Duration::from_secs(60 * 60));
        info!("start zhang's version report task, random uuid {} is generated", &uuid);
        loop {
            report_interval.tick().await;
            match version_report_task(uuid).await {
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
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
    Ok(())
}
pub fn create_server_app(
    ledger: Arc<RwLock<Ledger>>, broadcaster: Arc<Broadcaster>, reload_sender: Arc<ReloadSender>, auth_credential: Option<String>,
) -> Router {
    let basic_credential = auth_credential.map(|credential| {
        let token_part = credential.splitn(2, ':').map(|it| it.to_owned()).collect_vec();
        (
            token_part.first().cloned().expect("cannot retrieve credential user_id"),
            token_part.get(1).cloned(),
        )
    });
    #[derive(Clone)]
    struct AppState {
        ledger: Arc<RwLock<Ledger>>,
        broadcaster: Arc<Broadcaster>,
        reload_sender: Arc<ReloadSender>,
    }

    impl FromRef<AppState> for Arc<RwLock<Ledger>> {
        fn from_ref(input: &AppState) -> Self {
            input.ledger.clone()
        }
    }

    impl FromRef<AppState> for Arc<Broadcaster> {
        fn from_ref(input: &AppState) -> Self {
            input.broadcaster.clone()
        }
    }
    impl FromRef<AppState> for Arc<ReloadSender> {
        fn from_ref(input: &AppState) -> Self {
            input.reload_sender.clone()
        }
    }

    let app = Router::new()
        .route("/api/sse", get(sse))
        .route("/api/reload", post(reload))
        .route("/api/info", get(get_basic_info))
        .route("/api/store", get(get_store_data))
        .route("/api/options", get(get_all_options))
        .route("/api/errors", get(get_errors))
        .route("/api/files", get(get_files))
        .route("/api/files/:file_path", get(get_file_content))
        .route("/api/files/:file_path", put(update_file_content))
        .route("/api/for-new-transaction", get(get_info_for_new_transactions))
        .route("/api/journals", get(get_journals))
        .route("/api/transactions", post(create_new_transaction))
        .route("/api/transactions/:transaction_id/documents", post(upload_transaction_document))
        .route("/api/accounts", get(get_account_list))
        .route("/api/accounts/:account_name", get(get_account_info))
        .route("/api/accounts/:account_name/documents", post(upload_account_document))
        .route("/api/accounts/:account_name/documents", get(get_account_documents))
        .route("/api/accounts/:account_name/journals", get(get_account_journals))
        .route("/api/accounts/:account_name/balances", post(create_account_balance))
        .route("/api/accounts/batch-balances", post(create_batch_account_balances))
        .route("/api/documents", get(get_documents))
        .route("/api/documents/:file_path", get(download_document))
        .route("/api/commodities", get(get_all_commodities))
        .route("/api/commodities/:commodity_name", get(get_single_commodity))
        .route("/api/statistic/summary", get(get_statistic_summary))
        .route("/api/statistic/graph", get(get_statistic_graph))
        .route("/api/statistic/:account_type", get(get_statistic_rank_detail_by_account_type))
        .route("/api/budgets", get(get_budget_list))
        .route("/api/budgets/:budget_name", get(get_budget_info))
        .route("/api/budgets/:budget_name/interval/:year/:month", get(get_budget_interval_detail))
        .layer(CorsLayer::permissive())
        .layer(DefaultBodyLimit::disable())
        .layer(RequestBodyLimitLayer::new(250 * 1024 * 1024 /* 250mb */))
        .with_state(AppState {
            ledger,
            broadcaster,
            reload_sender,
        });

    let app = if let Some((username, password)) = basic_credential {
        info!("web basic auth is enabled with username {}", &username);
        app.layer(ValidateRequestHeaderLayer::basic(&username, password.as_deref().unwrap_or_default()))
    } else {
        app
    };

    #[cfg(feature = "frontend")]
    {
        app.fallback(routes::frontend::serve_frontend)
    }
    #[cfg(not(feature = "frontend"))]
    {
        app.fallback(routes::common::backend_only_info)
    }
}

async fn version_report_task(uuid: Uuid) -> ServerResult<()> {
    #[derive(Serialize)]
    struct VersionReport<'a> {
        version: &'a str,
        build_date: &'a str,
        uuid: &'a Uuid,
    }
    debug!("reporting zhang's version");
    let client = reqwest::Client::new();
    client
        .post("https://zhang-cloud.kilerd.me/client_report")
        .json(&VersionReport {
            version: env!("ZHANG_BUILD_VERSION"),
            build_date: env!("ZHANG_BUILD_DATE"),
            uuid: &uuid,
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

pub type ApiResult<T> = ServerResult<ResponseWrapper<T>>;
