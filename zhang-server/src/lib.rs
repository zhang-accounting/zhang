use std::any::Any;
use std::ops::Deref;
use std::panic::AssertUnwindSafe;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::DefaultBodyLimit;
use axum::routing::{any, get};
use chrono::Utc;
use futures::FutureExt;
use gotcha::config::BasicConfig;
use gotcha::{ConfigWrapper, GotchaApp, GotchaContext, GotchaRouter};
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
use tokio::task::JoinHandle;
use tower_http::cors::CorsLayer;
use tower_http::limit::RequestBodyLimitLayer;
use zhang_core::data_source::DataSource;
use zhang_core::error::IoErrorIntoZhangError;
use zhang_core::inputs::ExtraInput;
use zhang_core::ledger::Ledger;
use zhang_core::{ZhangError, ZhangResult};

use crate::auth::{AuthConfig, AuthState, SharedAuth};
use crate::broadcast::{BroadcastEvent, Broadcaster};
use crate::error::ServerError;
use crate::response::ResponseWrapper;
use crate::state::AppState;

pub mod account_queries;
pub mod auth;
mod balance_writes;
pub mod broadcast;
pub mod builtin;
mod cells;
pub mod error;
pub mod journals;
pub mod report;
pub mod request;
pub mod response;
pub mod routes;
pub mod state;
pub mod tasks;
pub mod util;
mod validate;
mod watch;

pub type LedgerState = Arc<RwLock<Ledger>>;

pub type ServerResult<T> = Result<T, ServerError>;

pub type ApiResult<T> = ServerResult<ResponseWrapper<T>>;

pub struct ServerApp {
    opts: ServeConfig,
    ledger: Arc<RwLock<Ledger>>,
    broadcaster: Arc<Broadcaster>,
    reload_sender: Arc<ReloadSender>,
    auth: Arc<AuthState>,
}

impl GotchaApp for ServerApp {
    type State = AppState;

    type Config = ();

    async fn config(&self) -> Result<ConfigWrapper<Self::Config>, Box<dyn std::error::Error>> {
        Ok(ConfigWrapper {
            basic: BasicConfig {
                host: self.opts.addr.clone(),
                port: self.opts.port,
            },
            application: (),
        })
    }

    fn routes(&self, router: GotchaRouter<GotchaContext<Self::State, Self::Config>>) -> GotchaRouter<GotchaContext<Self::State, Self::Config>> {
        let router = router
            .get("/api/auth/status", auth::handlers::get_auth_status)
            .post("/api/auth/login", auth::handlers::auth_login)
            .post("/api/auth/logout", auth::handlers::auth_logout)
            .post("/api/auth/passkey/register/start", auth::handlers::passkey_register_start)
            .post("/api/auth/passkey/register/finish", auth::handlers::passkey_register_finish)
            .post("/api/auth/passkey/login/start", auth::handlers::passkey_login_start)
            .post("/api/auth/passkey/login/finish", auth::handlers::passkey_login_finish)
            .get("/api/auth/passkeys", auth::handlers::get_passkeys)
            .delete("/api/auth/passkeys/:passkey_id", auth::handlers::delete_passkey)
            .get("/api/sse", sse)
            .post("/api/reload", reload)
            .get("/api/info", get_basic_info)
            .get("/api/options", get_all_options)
            .get("/api/errors", get_errors)
            .get("/api/files", get_files)
            // the file path is standard base64, which can contain `/`, see `Base64Path`: the OpenAPI document cannot
            // describe a catch-all and the router cannot have one next to `:file_path`, so the two `:file_path`
            // routes are the documented ones, and the three after them take an empty path and a path of more segments
            .get("/api/files/:file_path", get_file_content)
            .put("/api/files/:file_path", update_file_content)
            .route("/api/files/", get(get_file_content).put(update_file_content))
            .route("/api/files/:file_path/", get(get_file_content).put(update_file_content))
            .route("/api/files/:file_path/*rest", get(get_file_content).put(update_file_content))
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
            .post("/api/accounts/:account_name/balances", create_account_balance)
            .post("/api/accounts/batch-balances", create_batch_account_balances)
            .get("/api/documents", get_documents)
            // the file path is standard base64, which can contain `/`, see `Base64Path`; an empty one is a 400
            .get("/api/documents/", download_document)
            .get("/api/documents/*file_path", download_document)
            .get("/api/commodities", get_all_commodities)
            .get("/api/commodities/:commodity_name", get_single_commodity)
            .get("/api/statistic/summary", get_statistic_summary)
            .get("/api/statistic/graph", get_statistic_graph)
            .get("/api/statistic/:account_type", get_statistic_rank_detail_by_account_type)
            .get("/api/budgets", get_budget_list)
            .get("/api/budgets/:budget_name", get_budget_info)
            .get("/api/budgets/:budget_name/interval/:year/:month", get_budget_interval_detail)
            .get("/api/plugins", routes::plugin::plugin_list)
            // router plugins: any method, behind the same layers (and authentication) as the rest of the API
            .route(routes::plugin_router::ROUTE, any(routes::plugin_router::route_to_plugin))
            .post("/api/query", routes::query::run_query)
            .post("/api/query/csv", routes::query::run_query_csv)
            .get("/api/query/schema", routes::query::get_query_schema)
            .get("/api/query/saved", routes::query::get_saved_queries)
            .get("/api/query/builtins", routes::query::get_builtin_queries)
            .post("/api/query/builtins/:name/text", routes::query::get_builtin_query_text)
            .layer(CorsLayer::permissive().expose_headers(cors_expose_headers()))
            .layer(DefaultBodyLimit::disable())
            .layer(RequestBodyLimitLayer::new(250 * 1024 * 1024 /* 250mb */));

        // the frontend (static assets and the SPA fallback below) stays reachable, it shows the login page
        let router = if self.auth.enabled() {
            router.layer(axum::middleware::from_fn_with_state(
                SharedAuth(self.auth.clone()),
                auth::require_authentication,
            ))
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

    async fn state(&self, _config: &gotcha::ConfigWrapper<Self::Config>) -> Result<Self::State, Box<dyn std::error::Error>> {
        let passkeys = self.auth.load_passkeys().await?;
        if self.auth.passkey_enabled() {
            info!("{} registered passkey(s) loaded from {}", passkeys, auth::PASSKEYS_PATH);
        }
        Ok(AppState {
            ledger: SharedLedger(self.ledger.clone()),
            broadcaster: SharedBroadcaster(self.broadcaster.clone()),
            reload_sender: SharedReloadSender(self.reload_sender.clone()),
            auth: SharedAuth(self.auth.clone()),
        })
    }
}

/// `Access-Control-Expose-Headers` for cross-origin clients such as the frontend dev server:
/// `*` exposes every response header where the wildcard is honoured, and `Content-Disposition`
/// is also listed by name so the `POST /api/query/csv` filename stays readable where it is not.
fn cors_expose_headers() -> [axum::http::HeaderName; 2] {
    [axum::http::HeaderName::from_static("*"), axum::http::header::CONTENT_DISPOSITION]
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
    /// `user:pass`, enables the password login (`--auth` / `ZHANG_AUTH`)
    pub auth_credential: Option<String>,
    /// enables the passkey login, the secret needed to register a passkey without a session (`--passkey` / `ZHANG_PASSKEY`)
    pub passkey_secret: Option<String>,
    /// the WebAuthn relying party id, by default the host of the request (`ZHANG_PASSKEY_RP_ID`)
    pub passkey_rp_id: Option<String>,
    /// the origin of the web UI, by default derived from the request (`ZHANG_PASSKEY_ORIGIN`)
    pub passkey_origin: Option<String>,
    /// the key that signs the sessions, random (sessions end on restart) when absent (`ZHANG_SESSION_SECRET`)
    pub session_secret: Option<String>,
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

pub async fn serve(mut opts: ServeConfig) -> ZhangResult<()> {
    info!("version: {}, build date: {}", env!("ZHANG_BUILD_VERSION"), env!("ZHANG_BUILD_DATE"));
    let ledger = load_served_ledger(&mut opts).await?;
    let ledger_data = Arc::new(RwLock::new(ledger));
    let broadcaster = Broadcaster::create();
    let (tx, rx) = mpsc::channel::<i32>(1);
    let reload_sender = Arc::new(ReloadSender(tx));

    info!("start reload listener");
    start_reload_listener(ledger_data.clone(), broadcaster.clone(), reload_sender.clone(), rx);

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

/// The ledger `opts` serves, loaded. The root of a ledger on the local file system is made canonical first, in
/// `opts.path` and so in the ledger's entry: the load lists the files it read by joining them onto the root, and the
/// watcher compares that list with the paths the filesystem reports, which are canonical (macOS) or joined onto the
/// watched root (other platforms). A root as typed, `.` or through a symlink such as `/tmp` on macOS, spelled the
/// files differently, and no edit ever reloaded the ledger (#492). A remote root is a path on the remote storage, and
/// stays as it is.
pub async fn load_served_ledger(opts: &mut ServeConfig) -> ZhangResult<Ledger> {
    if opts.is_local_fs {
        opts.path = opts.path.canonicalize().with_path(&opts.path)?;
    }
    Ledger::async_load(opts.path.clone(), opts.endpoint.clone(), opts.data_source.clone()).await
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
        let roots = watch::watch_roots(&entry_path);
        info!("watching {}", entry_path.to_str().unwrap_or(""));
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
            let is_stale = all
                .iter()
                .filter_map(|event| event.as_ref().ok())
                .any(|event| watch::should_reload(event, &roots, &guard.visited_files, &guard.extra_inputs));

            drop(guard);

            if is_stale {
                debug!("gotcha event, sending reload event...");
                reload_sender_for_fs.0.try_send(1).ok();
            }
        }
    });
}

fn start_reload_listener(
    ledger_for_reload: Arc<RwLock<Ledger>>, cloned_broadcaster: Arc<Broadcaster>, reload_sender: Arc<ReloadSender>, mut rx: Receiver<i32>,
) {
    tokio::spawn(async move {
        let mut midnight_reload = schedule_midnight_reload(&*ledger_for_reload.read().await, &reload_sender);
        while rx.recv().await.is_some() {
            info!("start reloading...");
            let start_time = Instant::now();
            let mut guard = ledger_for_reload.write().await;
            // a reload that panics must not end this task, which left the server on the ledger it had, never reloading
            // again (#492): the panic is caught and logged like a failed reload. The ledger served stays the previous
            // one, as a reload replaces it only once it loaded whole, so the guard is unwind safe
            match AssertUnwindSafe(guard.async_reload()).catch_unwind().await {
                Ok(Ok(_)) => {
                    let duration = start_time.elapsed();
                    info!("ledger is reloaded successfully in {:?}", duration);
                    // todo: add reload duration to reload event
                    cloned_broadcaster.broadcast(BroadcastEvent::Reload).await;
                }
                Ok(Err(err)) => {
                    error!("error on reload: {}", err);
                    // todo: broadcast the error
                }
                Err(panic) => {
                    error!("panic on reload, the previous ledger is kept: {}", panic_message(panic.as_ref()));
                }
            }
            // replaced on every reload, for the ledger now served: a failed reload keeps the previous one
            if let Some(task) = midnight_reload.take() {
                task.abort();
            }
            midnight_reload = schedule_midnight_reload(&guard, &reload_sender);
            drop(guard);
        }
    });
}

/// the message a caught panic was raised with, for the log
fn panic_message(panic: &(dyn Any + Send)) -> &str {
    panic
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| panic.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("a panic without a message")
}

/// reload `ledger` at the next local midnight in its timezone if it depends on the date
fn schedule_midnight_reload(ledger: &Ledger, reload_sender: &Arc<ReloadSender>) -> Option<JoinHandle<()>> {
    if !ledger.extra_inputs.contains(&ExtraInput::Clock) {
        return None;
    }
    let timezone = ledger.options.timezone;
    let at = watch::next_local_midnight(Utc::now(), timezone);
    info!("the ledger depends on the date, reloading it at {}", at.with_timezone(&timezone));
    let reload_sender = reload_sender.clone();
    Some(tokio::spawn(async move {
        // check the wall clock at least every minute instead of sleeping once: a sleep stops while the machine is
        // suspended, and the wall clock can be adjusted
        while let Ok(remaining) = (at - Utc::now()).to_std() {
            tokio::time::sleep(remaining.min(Duration::from_secs(60))).await;
        }
        reload_sender.reload();
    }))
}

pub async fn start_server(
    opts: ServeConfig, ledger_data: Arc<RwLock<Ledger>>, broadcaster: Arc<Broadcaster>, reload_sender: Arc<ReloadSender>,
) -> ZhangResult<()> {
    info!("zhang is listening on http://{}:{}/", opts.addr, opts.port);
    // read the query result limit once, at startup, so a bad value is reported right away
    routes::query::max_result_values();

    let app = create_server_app(opts, ledger_data, broadcaster, reload_sender);
    run_app(app).await.map_err(|e| ZhangError::CustomError(e.to_string()))
}

/// [`GotchaApp::run`], serving with the peer address of the connections, which the sign-in rate
/// limit falls back to without `X-Forwarded-For`.
async fn run_app(app: ServerApp) -> Result<(), Box<dyn std::error::Error>> {
    app.logger()?;
    let config = app.config().await?;
    let state = app.state(&config).await?;
    let router = app.build_router(GotchaContext { config: config.clone(), state }).await?;
    let listener = tokio::net::TcpListener::bind((config.basic.host.as_str(), config.basic.port)).await?;
    axum::serve(listener, router.into_make_service_with_connect_info::<std::net::SocketAddr>()).await?;
    Ok(())
}

fn log_auth_settings(auth: &AuthState, opts: &ServeConfig) {
    if !auth.enabled() {
        return;
    }
    let mut methods = vec![];
    if let Some(credential) = &opts.auth_credential {
        methods.push(format!("password (user {})", credential.split(':').next().unwrap_or_default()));
    }
    if auth.passkey_enabled() {
        methods.push("passkey".to_owned());
    }
    info!("authentication is enabled: {}", methods.join(", "));
    if !auth.has_session_secret() {
        info!("ZHANG_SESSION_SECRET is not set, sessions end when the server restarts");
    }
}

pub fn create_server_app(opts: ServeConfig, ledger: Arc<RwLock<Ledger>>, broadcaster: Arc<Broadcaster>, reload_sender: Arc<ReloadSender>) -> ServerApp {
    let auth = Arc::new(AuthState::new(AuthConfig::from_serve_config(&opts), ledger.clone()));
    log_auth_settings(&auth, &opts);
    ServerApp {
        opts,
        ledger,
        broadcaster,
        reload_sender,
        auth,
    }
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
    let latest_version = latest_release
        .ok()
        .and_then(|releases| releases.latest().map(|release| release.version().to_owned()));
    if let Some(version) = latest_version {
        if bump_is_greater(env!("ZHANG_BUILD_VERSION"), &version).unwrap_or(false) {
            broadcast.broadcast(BroadcastEvent::NewVersionFound { version }).await;
        }
    }
    Ok(())
}

#[cfg(test)]
mod reload_test {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use axum::extract::{Path, State};
    use axum::response::IntoResponse;
    use axum::Json;
    use bigdecimal::BigDecimal;
    use tokio::sync::{mpsc, RwLock};
    use zhang_ast::amount::Amount;
    use zhang_core::data_source::{DataSource, LoadResult, LocalFileSystemDataSource};
    use zhang_core::data_type::text::ZhangDataType;
    use zhang_core::data_type::DataType;
    use zhang_core::ledger::Ledger;
    use zhang_core::ZhangResult;

    use super::start_reload_listener;
    use crate::broadcast::Broadcaster;
    use crate::request::AccountBalanceRequest;
    use crate::routes::account::create_account_balance;
    use crate::state::{SharedLedger, SharedReloadSender};
    use crate::ReloadSender;

    /// A write asks for the reload that serves the readers: they read what it wrote, with no other write after it.
    #[tokio::test]
    async fn the_readers_read_what_a_write_wrote() {
        let dir = std::env::temp_dir().join(format!("zhang-reload-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let dir = dir.canonicalize().unwrap();
        std::fs::write(
            dir.join("main.bean"),
            "option \"operating_currency\" \"CNY\"\n2020-01-01 commodity CNY\n2024-01-01 open Assets:A\n2024-01-01 open Income:X\n\
             2024-06-01 * \"seed\"\n  Assets:A 50 CNY\n  Income:X\n",
        )
        .unwrap();
        let source = Arc::new(LocalFileSystemDataSource::new(beancount::Beancount {}));
        let loaded = Ledger::async_load(dir.clone(), "main.bean".to_owned(), source).await.expect("load ledger");
        let ledger = Arc::new(RwLock::new(loaded));
        let (sender, receiver) = mpsc::channel(1);
        let reload_sender = Arc::new(ReloadSender(sender));
        start_reload_listener(ledger.clone(), Broadcaster::create(), reload_sender.clone(), receiver);

        let check = AccountBalanceRequest::Check {
            amount: Amount::new(BigDecimal::from(50), "CNY"),
        };
        let response = create_account_balance(
            State(SharedLedger(ledger.clone())),
            State(SharedReloadSender(reload_sender)),
            Path(("Assets:A".to_owned(),)),
            Json(check),
        )
        .await
        .into_response();
        assert!(response.status().is_success(), "{}", response.status());

        let mut assertions = 0;
        for _ in 0..100 {
            let ledger = ledger.read().await;
            assertions = ledger.store.read().unwrap().balance_assertions.len();
            if assertions == 1 {
                assert!(!ledger.stale, "the ledger readers read is the one reloaded");
                break;
            }
            drop(ledger);
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert_eq!(assertions, 1, "the readers read the balance written");
        std::fs::remove_dir_all(dir).ok();
    }

    /// a source whose load panics while `panicking` is set, and otherwise reads `content` as the main file
    struct Panicking {
        panicking: AtomicBool,
        content: Mutex<String>,
        loads: AtomicUsize,
    }

    #[async_trait::async_trait]
    impl DataSource for Panicking {
        fn load(&self, entry: String, endpoint: String) -> ZhangResult<LoadResult> {
            self.loads.fetch_add(1, Ordering::SeqCst);
            if self.panicking.load(Ordering::SeqCst) {
                panic!("the load panicked");
            }
            let content = self.content.lock().unwrap().clone();
            let directives = ZhangDataType {}.transform(content, Some(endpoint.clone()))?;
            Ok(LoadResult {
                directives,
                visited_files: vec![PathBuf::from(entry).join(endpoint)],
            })
        }
    }

    /// the accounts of the ledger served, sorted
    async fn accounts(ledger: &RwLock<Ledger>) -> Vec<String> {
        let ledger = ledger.read().await;
        let store = ledger.store.read().unwrap();
        let mut accounts: Vec<String> = store.accounts.keys().cloned().collect();
        accounts.sort();
        accounts
    }

    /// A reload that panics, as an `include` outside the ledger made it (#492), does not end the reload task: the
    /// ledger served stays the previous one, and the next change is reloaded.
    #[tokio::test]
    async fn a_panic_in_a_reload_does_not_stop_the_next_reload() {
        let source = Arc::new(Panicking {
            panicking: AtomicBool::new(false),
            content: Mutex::new("1970-01-01 open Assets:A\n".to_owned()),
            loads: AtomicUsize::new(0),
        });
        let loaded = Ledger::async_load(PathBuf::from("/panicking"), "main.zhang".to_owned(), source.clone())
            .await
            .expect("load ledger");
        let ledger = Arc::new(RwLock::new(loaded));
        let (sender, receiver) = mpsc::channel(1);
        let reload_sender = Arc::new(ReloadSender(sender));
        start_reload_listener(ledger.clone(), Broadcaster::create(), reload_sender.clone(), receiver);

        source.panicking.store(true, Ordering::SeqCst);
        reload_sender.reload();
        for _ in 0..100 {
            if source.loads.load(Ordering::SeqCst) >= 2 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert_eq!(source.loads.load(Ordering::SeqCst), 2, "the panicking reload ran");
        // without a guard the task ends at the panic: leave it the time to, before checking it still listens
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(!reload_sender.is_closed(), "the reload task is still listening after a panic");
        assert_eq!(accounts(&ledger).await, vec!["Assets:A"], "the ledger served is the previous one");

        source.panicking.store(false, Ordering::SeqCst);
        *source.content.lock().unwrap() = "1970-01-01 open Assets:A\n1970-01-01 open Assets:B\n".to_owned();
        reload_sender.reload();
        let mut served = vec![];
        for _ in 0..100 {
            served = accounts(&ledger).await;
            if served.len() == 2 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert_eq!(served, vec!["Assets:A", "Assets:B"], "the next change is reloaded");
    }
}

/// The root `zhang serve` is given, as typed, spelled the files the load listed differently from the paths the
/// filesystem reports, and no edit reloaded a ledger served as `zhang serve .` or through a symlink (#492).
#[cfg(all(test, unix))]
mod served_root_test {
    use std::path::{Component, Path, PathBuf};
    use std::sync::Arc;

    use notify::event::{DataChange, ModifyKind};
    use notify::{Event, EventKind};
    use zhang_core::data_source::{DataSource, LoadResult};
    use zhang_core::data_type::text::ZhangDataType;
    use zhang_core::data_type::DataType;
    use zhang_core::ZhangResult;

    use super::{load_served_ledger, watch, ServeConfig};

    /// a source listing the files it loads as the CLI's does: the file joined onto the root as it was given
    struct Joined;

    #[async_trait::async_trait]
    impl DataSource for Joined {
        fn load(&self, entry: String, endpoint: String) -> ZhangResult<LoadResult> {
            let file = PathBuf::from(entry).join(endpoint);
            let content = std::fs::read_to_string(&file)?;
            let directives = ZhangDataType {}.transform(content, Some(file.to_string_lossy().into_owned()))?;
            Ok(LoadResult {
                directives,
                visited_files: vec![file],
            })
        }
    }

    /// what `zhang serve <root>` on the local file system hands to `serve`
    fn local_serve_config(root: PathBuf) -> ServeConfig {
        ServeConfig {
            path: root,
            endpoint: "main.zhang".to_owned(),
            addr: String::new(),
            port: 0,
            no_report: true,
            data_source: Arc::new(Joined),
            auth_credential: None,
            passkey_secret: None,
            passkey_rp_id: None,
            passkey_origin: None,
            session_secret: None,
            is_local_fs: true,
        }
    }

    /// a ledger directory `ledger` with a main file under `dir`, and a symlink `link` to it
    fn ledger_and_link(dir: &Path) -> (PathBuf, PathBuf) {
        let ledger = dir.join("ledger");
        std::fs::create_dir(&ledger).unwrap();
        std::fs::write(ledger.join("main.zhang"), "1970-01-01 open Assets:A\n").unwrap();
        let link = dir.join("link");
        std::os::unix::fs::symlink(&ledger, &link).unwrap();
        (ledger, link)
    }

    /// whether the ledger served from `root`, loaded as `serve` loads it, is stale after an edit of `main`, which
    /// the filesystem reports by the canonical path
    async fn edit_reloads_ledger_served_from(root: PathBuf, main: &Path) -> bool {
        let mut opts = local_serve_config(root);
        let ledger = load_served_ledger(&mut opts).await.expect("load ledger");
        assert_eq!(opts.path, ledger.entry.0, "the root served is the root loaded");
        let roots = watch::watch_roots(&ledger.entry.0);
        let edit = Event::new(EventKind::Modify(ModifyKind::Data(DataChange::Content))).add_path(main.canonicalize().unwrap());
        watch::should_reload(&edit, &roots, &ledger.visited_files, &ledger.extra_inputs)
    }

    /// `target`, absolute, relative to the working directory, as `zhang serve .` or `zhang serve ../ledger` name a root
    fn relative_to_cwd(target: &Path) -> PathBuf {
        let cwd = std::env::current_dir().unwrap().canonicalize().unwrap();
        let mut relative = PathBuf::new();
        for component in cwd.components() {
            if let Component::Normal(_) = component {
                relative.push("..");
            }
        }
        relative.join(target.strip_prefix("/").unwrap())
    }

    #[tokio::test]
    async fn should_reload_an_edit_of_a_ledger_served_through_a_symlinked_root() {
        let dir = tempfile::tempdir().unwrap();
        let (ledger, link) = ledger_and_link(dir.path());
        assert!(edit_reloads_ledger_served_from(link, &ledger.join("main.zhang")).await);
    }

    #[tokio::test]
    async fn should_reload_an_edit_of_a_ledger_served_through_a_relative_root() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = dir.path().canonicalize().unwrap();
        std::fs::write(ledger.join("main.zhang"), "1970-01-01 open Assets:A\n").unwrap();
        let relative = relative_to_cwd(&ledger);
        assert!(relative.is_relative(), "{}", relative.display());
        assert!(edit_reloads_ledger_served_from(relative, &ledger.join("main.zhang")).await);
    }
}
