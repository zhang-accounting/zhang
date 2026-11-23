use std::any::Any;
use std::ops::Deref;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::DefaultBodyLimit;
use axum::routing::{any, get};
use chrono::Utc;
use gotcha::config::BasicConfig;
use gotcha::{ConfigWrapper, GotchaApp, GotchaContext, GotchaRouter};
use log::{debug, error, info, trace, warn};
use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};
use routes::account::*;
use routes::commodity::*;
use routes::common::*;
use routes::document::*;
use routes::file::*;
use routes::statistics::*;
use routes::transaction::*;
use self_update::version::bump_is_greater;
use serde::Serialize;
use state::{SharedBroadcaster, SharedLedger, SharedReloadSender};
use tokio::sync::mpsc::{channel, Receiver, Sender};
use tokio::sync::{mpsc, oneshot, RwLock};
use tokio::task::JoinHandle;
use tower_http::cors::CorsLayer;
use tower_http::limit::RequestBodyLimitLayer;
use zhang_core::clock::Clock;
use zhang_core::data_source::DataSource;
use zhang_core::error::IoErrorIntoZhangError;
use zhang_core::extensions::SnapshotStatus;
use zhang_core::inputs::ExtraInput;
use zhang_core::ledger::Ledger;
use zhang_core::plugin_policy::PluginLevel;
use zhang_core::{ZhangError, ZhangResult};

use crate::auth::{AuthConfig, AuthState, SharedAuth};
use crate::broadcast::{BroadcastEvent, Broadcaster};
use crate::error::ServerError;
use crate::plugin_writes::PluginWrites;
use crate::response::ResponseWrapper;
use crate::routes::plugin_router::{PluginRoutes, PluginSettings};
use crate::state::{AppState, LedgerWriter, ReloadFailure, SharedPluginWrites};

pub mod account_queries;
pub mod analysis;
pub mod auth;
mod balance_writes;
pub mod broadcast;
pub mod builtin;
mod cells;
pub mod error;
pub mod journals;
pub mod plugin_writes;
pub mod report;
pub mod request;
pub mod response;
pub mod routes;
pub mod state;
pub mod tombstones;
pub mod util;
mod validate;
pub mod watch;

pub type LedgerState = Arc<RwLock<Ledger>>;

pub type ServerResult<T> = Result<T, ServerError>;

pub type ApiResult<T> = ServerResult<ResponseWrapper<T>>;

pub struct ServerApp {
    opts: ServeConfig,
    ledger: Arc<RwLock<Ledger>>,
    broadcaster: Arc<Broadcaster>,
    reload_sender: Arc<ReloadSender>,
    auth: Arc<AuthState>,
    plugin_routes: Arc<PluginRoutes>,
    /// what the writes of router plugins share: the key of their preview tokens and the lock of their commits
    plugin_writes: Arc<PluginWrites>,
}

impl GotchaApp for ServerApp {
    type State = AppState;

    type Config = ();

    /// Sets up no logger: the binary has one (env_logger, `zhang-cli`), with its filter (`ZHANG_LOG`, the targets it
    /// quiets) and colours only on a terminal. Gotcha's default installs a second one, a tracing subscriber that reads
    /// `RUST_LOG` only and colours its lines in a container's log too, and that took the tracing events of the
    /// dependencies (gotcha, wasmtime) from the binary's once the server started. Without it, those events reach the
    /// binary's logger through the `log` facade (tracing's `log` feature), before and after the start alike.
    fn logger(&self) -> Result<(), Box<dyn std::error::Error>> {
        Ok(())
    }

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
            .post("/api/auth/app/code", auth::handlers::app_login_code)
            .post("/api/auth/app/exchange", auth::handlers::app_login_exchange)
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
            .get("/api/files", get_files)
            // the file path is standard base64, which can contain `/`, see `Base64Path`: the OpenAPI document cannot
            // describe a catch-all and the router cannot have one next to `:file_path`, so the two `:file_path`
            // routes are the documented ones, and the three after them take an empty path and a path of more segments
            .get("/api/files/:file_path", get_file_content)
            .put("/api/files/:file_path", update_file_content)
            .route("/api/files/", get(get_file_content).put(update_file_content))
            .route("/api/files/:file_path/", get(get_file_content).put(update_file_content))
            .route("/api/files/:file_path/*rest", get(get_file_content).put(update_file_content))
            .get("/api/journals", get_journals)
            .post("/api/transactions", create_new_transaction)
            .post("/api/transactions/preview", preview_new_transaction)
            .put("/api/transactions/:transaction_id", update_single_transaction)
            .delete("/api/transactions/:transaction_id", delete_single_transaction)
            .post("/api/transactions/:transaction_id/preview", preview_transaction_update)
            .post("/api/transactions/:transaction_id/documents", upload_transaction_document)
            .get("/api/accounts", get_account_list)
            .get("/api/accounts/:account_name", get_account_info)
            .post("/api/accounts/:account_name/documents", upload_account_document)
            .get("/api/accounts/:account_name/journals", get_account_journals)
            .post("/api/accounts/:account_name/balances", create_account_balance)
            .post("/api/accounts/batch-balances", create_batch_account_balances)
            // the file path is standard base64, which can contain `/`, see `Base64Path`; an empty one is a 400
            .get("/api/documents/", download_document)
            .get("/api/documents/*file_path", download_document)
            .get("/api/commodities", get_all_commodities)
            .get("/api/commodities/:commodity_name", get_single_commodity)
            .get("/api/statistic/graph", get_statistic_graph)
            .get("/api/plugins", routes::plugin::plugin_list)
            // router plugins: any method, behind the same layers (and authentication) as the rest of the API
            .route(routes::plugin_router::ROUTE, any(routes::plugin_router::route_to_plugin))
            .post("/api/query", routes::query::run_query)
            .post("/api/query/csv", routes::query::run_query_csv)
            .get("/api/query/schema", routes::query::get_query_schema)
            .get("/api/query/saved", routes::query::get_saved_queries)
            .get("/api/query/builtins", routes::query::get_builtin_queries)
            .post("/api/query/builtins", routes::query::run_builtin_queries)
            .post("/api/query/builtins/:name", routes::query::run_builtin_query)
            .post("/api/query/builtins/:name/text", routes::query::get_builtin_query_text)
            // every error has the one JSON body, the rejections of the extractors too
            .layer(axum::middleware::from_fn(error::json_rejections))
            .layer(CorsLayer::permissive().expose_headers(cors_expose_headers()))
            .layer(DefaultBodyLimit::disable())
            .layer(RequestBodyLimitLayer::new(250 * 1024 * 1024 /* 250mb */));

        // the frontend (static assets and the SPA fallback below) stays reachable, it shows the login page; an `/api`
        // path no route takes is a JSON 404 there too
        let router = if self.auth.enabled() {
            router.layer(axum::middleware::from_fn_with_state(
                SharedAuth(self.auth.clone()),
                auth::require_authentication,
            ))
        } else {
            router
        };
        router.fallback(routes::common::fallback)
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
            plugin_routes: self.plugin_routes.clone(),
            plugin_writes: SharedPluginWrites(self.plugin_writes.clone()),
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
    let (tx, rx) = channel(64);

    // Automatically select the best implementation for your platform.
    // You can also access each implementation directly e.g. INotifyWatcher.
    let watcher = RecommendedWatcher::new(
        move |res| {
            // a read is no change: inotify reports one for every file a load opens, a hundred and more a write
            if !watch::is_change(&res) {
                return;
            }
            futures::executor::block_on(async {
                tx.send(res).await.ok();
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
    /// more url schemes the app login handoff may return to, comma separated, next to `zhang-app` (`ZHANG_APP_RETURN_SCHEMES`)
    pub app_return_schemes: Option<String>,
    /// what the operator lets plugins use: the policy every plugin of the ledger runs under, and the limits of the
    /// plugin routes (`ZHANG_PLUGIN_*`)
    pub plugins: PluginSettings,
}

impl ServeConfig {
    /// whether the ledger is on the local file system, which its data source tells: such a ledger is watched for
    /// edits, and served from its canonical root
    pub fn is_local(&self) -> bool {
        self.data_source.local_root(&self.path).is_some()
    }
}

/// A reload asked for. An asker that waits for it is told what it came to.
pub struct ReloadRequest {
    reply: Option<oneshot::Sender<Result<(), ReloadFailure>>>,
}

/// Asks for the reload of the served ledger, and keeps the failure of the last reload while the ledger served is the
/// one loaded before it (#492), for `/api/info` to show.
pub struct ReloadSender {
    sender: Sender<ReloadRequest>,
    last_failure: std::sync::RwLock<Option<ReloadFailure>>,
    /// the readers told of every reload, set once the reload listener runs
    broadcaster: std::sync::OnceLock<Arc<Broadcaster>>,
    /// the reload at the next local midnight of a ledger that depends on the date, replaced on every reload
    midnight_reload: std::sync::Mutex<Option<JoinHandle<()>>>,
    /// the transactions the writes deleted or replaced lately ([`tombstones`]), which outlive the reloads: read and
    /// changed by a writer, which holds the write cycle ([`LedgerWriter`])
    tombstones: std::sync::Mutex<tombstones::Tombstones>,
    /// The generation of the ledger served: a counter, starting at 1, that only moves on, by one whenever a reload
    /// loaded other content than the ledger served before ([`ledger_fingerprint`]). Whatever was computed from the
    /// ledger can be stamped with it and recognized as outdated once the ledger changed; the preview tokens of the
    /// writes of router plugins carry it, and a commit of a plugin moves it on too ([`plugin_writes`]). A reload of
    /// unchanged files, as the file watcher's after a write's own reload, keeps it
    generation: std::sync::atomic::AtomicU64,
    /// Whether a write landed in the files that the ledger served does not show yet: set by a plugin's commit under
    /// the ledger read lock, where it cannot mark the ledger stale itself, and by a write of the API until its own
    /// reload succeeded, and cleared by the next successful reload. A write of the API reloads first while it is set
    /// ([`SharedLedger::for_writing`]), so a write queued behind a plugin's commit never plans against the ledger the
    /// commit left behind (a second `include` of its file)
    written: std::sync::atomic::AtomicBool,
    /// the fingerprint of the ledger served ([`ledger_fingerprint`]), `None` before the first is recorded
    fingerprint: std::sync::Mutex<Option<u64>>,
    /// the freeing of the ledger the last reload replaced ([`ReloadSender::retire`])
    retired: std::sync::Mutex<Option<JoinHandle<()>>>,
    /// Whether a reload was asked for that must load whatever the files hold ([`ReloadSender::reload`]), since the
    /// listener last started one: taken by the listener as it starts the next. A reload asked for by
    /// [`ReloadSender::reload_if_files_changed`] alone loads nothing while the ledger's files hold what the ledger
    /// served was loaded from
    forced: std::sync::atomic::AtomicBool,
}

impl Deref for ReloadSender {
    type Target = Sender<ReloadRequest>;

    fn deref(&self) -> &Self::Target {
        &self.sender
    }
}

impl ReloadSender {
    pub fn new(sender: Sender<ReloadRequest>) -> Self {
        ReloadSender {
            sender,
            last_failure: std::sync::RwLock::new(None),
            broadcaster: std::sync::OnceLock::new(),
            midnight_reload: std::sync::Mutex::new(None),
            tombstones: std::sync::Mutex::new(tombstones::Tombstones::default()),
            generation: std::sync::atomic::AtomicU64::new(1),
            written: std::sync::atomic::AtomicBool::new(false),
            fingerprint: std::sync::Mutex::new(None),
            retired: std::sync::Mutex::new(None),
            forced: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// the transactions the writes deleted or replaced lately ([`tombstones`])
    pub fn tombstones(&self) -> std::sync::MutexGuard<'_, tombstones::Tombstones> {
        self.tombstones.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// remember the content of `ledger`, the one served now, so the next reload can tell whether it changed. This
    /// computes its fingerprint where it is called: from async code, prefer the listener's way, on a blocking thread
    pub fn record_content(&self, ledger: &Ledger) {
        self.record_fingerprint(ledger_fingerprint(ledger));
    }

    /// remember `fingerprint` ([`ledger_fingerprint`]) as that of the ledger served now
    fn record_fingerprint(&self, fingerprint: u64) {
        *self.fingerprint.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(fingerprint);
    }

    /// whether the ledger just loaded, whose fingerprint is `fingerprint`, differs in content from the one served
    /// before it (always, before the first is recorded), remembering it either way
    fn content_changed(&self, fingerprint: u64) -> bool {
        let mut previous = self.fingerprint.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let changed = *previous != Some(fingerprint);
        *previous = Some(fingerprint);
        changed
    }

    /// a write landed in the files: the ledger served is behind them until the reload asked for here runs
    pub fn mark_written(&self) {
        self.mark_pending();
        self.reload();
    }

    /// a write landed in the files, which its writer reloads itself: the ledger served is behind them until a reload
    /// succeeds
    pub(crate) fn mark_pending(&self) {
        self.written.store(true, std::sync::atomic::Ordering::SeqCst);
    }

    /// whether a write landed in the files since the ledger served was loaded, see [`ReloadSender::mark_written`]
    pub fn write_pending(&self) -> bool {
        self.written.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// the generation of the ledger served: 1 for the ledger served first, moved on by one whenever a reload loaded
    /// other content, and by a commit of a plugin
    pub fn generation(&self) -> u64 {
        self.generation.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// move the generation on, as a reload that loaded other content does, and a commit of a plugin once its text is
    /// in the file: the new generation
    pub(crate) fn next_generation(&self) -> u64 {
        self.generation.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1
    }

    /// ask for a reload, without waiting for it: a reload already asked for and not started reads the files as
    /// they are now too
    pub fn reload(&self) {
        self.forced.store(true, std::sync::atomic::Ordering::SeqCst);
        self.sender.try_send(ReloadRequest { reply: None }).ok();
    }

    /// Ask for a reload, as [`ReloadSender::reload`] does, that loads nothing when every one of the ledger's own files
    /// still holds what the ledger served was loaded from ([`Ledger::files_unchanged`]), checked once the writes and
    /// reloads before it are done: the file watcher's after a modification of the ledger's files, which the server's
    /// own writes make, each reloaded before its answer. A reload asked for otherwise meanwhile, a write pending, a
    /// stale ledger or a failed last reload load all the same
    pub fn reload_if_files_changed(&self) {
        self.sender.try_send(ReloadRequest { reply: None }).ok();
    }

    /// ask for a reload and wait for it: what the reload run for this request came to
    pub async fn reload_and_wait(&self) -> Result<(), ReloadFailure> {
        let (reply, outcome) = oneshot::channel();
        let no_reload_task = || ReloadFailure {
            file: None,
            message: "the server's reload task is not running, so the ledger cannot be reloaded: restart the server".to_owned(),
        };
        if self.sender.send(ReloadRequest { reply: Some(reply) }).await.is_err() {
            return Err(no_reload_task());
        }
        outcome.await.unwrap_or_else(|_| Err(no_reload_task()))
    }

    /// the failure of the last reload, while the ledger served is the one loaded before it
    pub fn last_failure(&self) -> Option<ReloadFailure> {
        self.last_failure.read().unwrap_or_else(|poisoned| poisoned.into_inner()).clone()
    }

    /// keep what the last reload came to
    fn record(&self, outcome: &Result<(), ReloadFailure>) {
        *self.last_failure.write().unwrap_or_else(|poisoned| poisoned.into_inner()) = outcome.clone().err();
    }

    /// Free `replaced`, the ledger a reload replaced, on a blocking thread: a ledger is many small allocations, and
    /// freeing a large one takes a while (about 230 ms for 100 000 transactions), which neither the readers nor the
    /// writer wait for. The next reload waits for it before it loads ([`ReloadSender::retired`]): the two at once
    /// slowed every load
    pub(crate) fn retire(&self, replaced: Ledger) {
        let freeing = tokio::task::spawn_blocking(move || drop(replaced));
        *self.retired.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(freeing);
    }

    /// once the ledger the last reload replaced is freed ([`ReloadSender::retire`])
    pub(crate) async fn retired(&self) {
        let freeing = self.retired.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).take();
        if let Some(freeing) = freeing {
            freeing.await.ok();
        }
    }

    /// What a reload that loaded the ledger whose fingerprint is `fingerprint` records once it swapped it in, under the
    /// ledger's write lock with the swap ([`LedgerWriter::reload`]): the generation moves on when the content changed,
    /// and the files are no longer ahead of the ledger served
    pub(crate) fn swapped(&self, fingerprint: u64) {
        // a reload that loaded the same content (the file watcher's, after a write's own) keeps the generation
        if self.content_changed(fingerprint) {
            self.next_generation();
        }
        // the files the ledger shows now are the ones written
        self.written.store(false, std::sync::atomic::Ordering::SeqCst);
    }

    /// What a reload came to, told: a failed reload is no longer only logged (#492), it is kept for `/api/info` while
    /// the ledger served is the one loaded before, and the readers are told; and the midnight reload is scheduled
    /// again for `served`, the ledger served now
    pub(crate) async fn reloaded(self: &Arc<Self>, outcome: &Result<(), ReloadFailure>, served: &Ledger) {
        self.record(outcome);
        if let Some(broadcaster) = self.broadcaster.get() {
            match outcome {
                // todo: add reload duration to reload event
                Ok(()) => broadcaster.broadcast(BroadcastEvent::Reload).await,
                Err(failure) => broadcaster.broadcast(BroadcastEvent::ReloadFailed(failure.clone())).await,
            }
        }
        // replaced on every reload, for the ledger now served: a failed reload keeps the previous one
        self.schedule_midnight_reload(served);
    }

    /// reload `ledger` at the next local midnight in its timezone if it depends on the date, instead of the reload
    /// scheduled before
    fn schedule_midnight_reload(self: &Arc<Self>, ledger: &Ledger) {
        let mut scheduled = self.midnight_reload.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(task) = scheduled.take() {
            task.abort();
        }
        *scheduled = schedule_midnight_reload(ledger, self);
    }
}

pub async fn serve(mut opts: ServeConfig) -> ZhangResult<()> {
    info!("version: {}, build date: {}", env!("ZHANG_BUILD_VERSION"), env!("ZHANG_BUILD_DATE"));
    info!("{}", opts.plugins);
    // the disk cache of compiled plugin code is set up now, before anything compiles, so that it is refused in the
    // folder of a ledger on the local disk, and that where it is, or why there is none, shows with the settings above
    if opts.plugins.policy.level != PluginLevel::Off {
        zhang_core::plugin::disk_cache::set_up_serving(opts.is_local().then_some(opts.path.as_path()));
    }
    // the official default plugins stop with the others: budgets are off, and the ledger says so once
    for stopped in zhang_core::extensions::stopped_by(&opts.plugins.policy) {
        warn!(
            "plugin {} does not run, as {}: budgets are off, budget directives are not interpreted and the budget pages answer that they are unavailable",
            stopped.provider, stopped.reason
        );
    }
    let ledger = load_served_ledger(&mut opts).await?;
    let ledger_data = Arc::new(RwLock::new(ledger));
    let broadcaster = Broadcaster::create();
    let (tx, rx) = mpsc::channel(1);
    let reload_sender = Arc::new(ReloadSender::new(tx));

    info!("start reload listener");
    start_reload_listener(ledger_data.clone(), broadcaster.clone(), reload_sender.clone(), rx);

    if opts.is_local() {
        info!("start fs event listener");
        start_fs_event_listener(ledger_data.clone(), reload_sender.clone());
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
/// stays as it is. The load runs on a blocking thread, as it reads files and runs plugins.
pub async fn load_served_ledger(opts: &mut ServeConfig) -> ZhangResult<Ledger> {
    if opts.is_local() {
        opts.path = opts.path.canonicalize().with_path(&opts.path)?;
    }
    let (path, endpoint, data_source) = (opts.path.clone(), opts.endpoint.clone(), opts.data_source.clone());
    let plugin_policy = Arc::new(opts.plugins.policy.clone());
    let options = analysis::load_options(plugin_policy);
    tokio::task::spawn_blocking(move || Ledger::load_with_options(path, endpoint, data_source, Clock::System, options))
        .await
        .unwrap_or_else(|failure| std::panic::resume_unwind(failure.into_panic()))
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

/// Watch the files of the local ledger `cloned_ledger` serves, and ask `reload_sender_for_fs` for a reload when one it
/// read changed ([`on_file_events`]): once no change came for [`watch::QUIET`], or [`watch::MAX_WAIT`] after the first,
/// so that a write of several files is one reload. The task the server starts for a local ledger, which the budget
/// baseline (`zhang-cli/examples/budget_baseline.rs`) starts too
pub fn start_fs_event_listener(cloned_ledger: Arc<RwLock<Ledger>>, reload_sender_for_fs: Arc<ReloadSender>) {
    tokio::spawn(async move {
        let (mut watcher, mut rx) = async_watcher().unwrap();

        let entry_path = {
            let guard1 = cloned_ledger.read().await;
            guard1.entry.0.clone()
        };
        let roots = watch::watch_roots(&entry_path);
        info!("watching {}", entry_path.to_str().unwrap_or(""));
        watcher.watch(entry_path.as_path(), RecursiveMode::Recursive).expect("cannot watch entry path");
        // the changes of a burst of writes, of an editor saving a file, at once
        while let Some(all) = watch::gather(&mut rx, watch::QUIET, watch::MAX_WAIT).await {
            trace!("receive all file changes: {:?}", all);
            on_file_events(&cloned_ledger, &reload_sender_for_fs, &roots, &all).await;
        }
    });
}

/// What the file watcher makes of `events`, what it reported under the ledger's root, whose spellings are `roots`
/// ([`watch::watch_roots`]): a reload when one of them changed something the load of the ledger served read
/// ([`watch::change`]), one that loads nothing while the ledger's own files hold what it was loaded from when only
/// those were modified ([`ReloadSender::reload_if_files_changed`]). The ledger is looked at once the writes and reloads
/// in progress are done, holding the write cycle shared ([`LedgerWriter`]): a load in progress reads files the ledger
/// served did not, the file an `include` it newly follows names, and a change to one of those while it loads is a
/// change to the ledger it swaps in. An error of the watcher, or a report that it lost events (its queue overflowed,
/// a rescan), may hide any change: the ledger is reloaded
pub async fn on_file_events(ledger: &Arc<RwLock<Ledger>>, reload_sender: &ReloadSender, roots: &[PathBuf], events: &[notify::Result<Event>]) {
    if let Some(lost) = events.iter().find(|event| event.as_ref().map_or(true, Event::need_rescan)) {
        warn!("the file watcher may have missed changes ({lost:?}), reloading the ledger");
        reload_sender.reload();
        return;
    }
    let _cycle = state::write_cycle(ledger).read_owned().await;
    let served = ledger.read().await;
    let change = events
        .iter()
        .filter_map(|event| event.as_ref().ok())
        .filter_map(|event| watch::change(event, roots, &served.visited_files, &served.extra_inputs))
        .max();
    drop(served);
    match change {
        Some(watch::Change::Inputs) => {
            debug!("gotcha event, sending reload event...");
            reload_sender.reload();
        }
        // the server's own writes modify the ledger's files, and reload it before they answer: a reload of the files
        // they left would load the ledger served again
        Some(watch::Change::LedgerFiles) => {
            debug!("gotcha event on the ledger's files, sending reload event unless they are as loaded...");
            reload_sender.reload_if_files_changed();
        }
        None => {}
    }
}

/// Run the reloads `reload_sender` asks for, one at a time, on `ledger_for_reload`, telling `cloned_broadcaster`'s
/// readers; the task the server starts, which a test of a write path starts too. Each runs as a writer of the ledger
/// ([`LedgerWriter`]): after the writes and reloads before it, and with the readers reading the ledger served until
/// the one it loads replaces it. The content of the ledger served first is recorded, so a reload of the same files
/// keeps the generation ([`ReloadSender::generation`])
pub fn start_reload_listener(
    ledger_for_reload: Arc<RwLock<Ledger>>, cloned_broadcaster: Arc<Broadcaster>, reload_sender: Arc<ReloadSender>, mut rx: Receiver<ReloadRequest>,
) {
    reload_sender.broadcaster.set(cloned_broadcaster).ok();
    tokio::spawn(async move {
        {
            // the fingerprint of the ledger served first, on a blocking thread as every other one is, before any writer
            // can reload it
            let _cycle = state::write_cycle(&ledger_for_reload).write_owned().await;
            let served = ledger_for_reload.clone().read_owned().await;
            let fingerprinted = tokio::task::spawn_blocking(move || (ledger_fingerprint(&served), served)).await;
            match fingerprinted {
                Ok((fingerprint, served)) => {
                    reload_sender.record_fingerprint(fingerprint);
                    reload_sender.schedule_midnight_reload(&served);
                }
                // nothing recorded: the first reload then counts as a change, which only voids tokens early
                Err(failure) => error!("the fingerprint of the ledger served could not be computed: {failure}"),
            }
        }
        while let Some(request) = rx.recv().await {
            let mut writer = LedgerWriter::new(ledger_for_reload.clone(), reload_sender.clone()).await;
            // a request that waits for the reload, or a reload asked for anew, loads; the file watcher's alone loads
            // only what the ledger served was not loaded from ([`ReloadSender::reload_if_files_changed`])
            let forced = reload_sender.forced.swap(false, std::sync::atomic::Ordering::SeqCst) || request.reply.is_some();
            let settled = !writer.stale && !reload_sender.write_pending() && reload_sender.last_failure().is_none();
            if !forced && settled && files_unchanged(&writer).await {
                debug!("the ledger's files hold what the ledger served was loaded from: no reload");
                continue;
            }
            let outcome = writer.reload().await;
            // the readers wait for the swap only, and no longer once it is done
            drop(writer);
            // the request that waits for it is answered
            if let Some(reply) = request.reply {
                reply.send(outcome).ok();
            }
        }
    });
}

/// whether the files of the ledger served still hold what it was loaded from ([`Ledger::files_unchanged`]), read on a
/// blocking thread; `false` when the check panics
async fn files_unchanged(ledger: &Ledger) -> bool {
    tokio::task::spawn_blocking(ledger.files_unchanged()).await.unwrap_or(false)
}

/// Load the served ledger again with `reload` ([`Ledger::reloader`]), as every reload of it does: on a blocking thread,
/// as it reads files and runs plugins, and with no lock held, so the readers read the ledger served meanwhile. The
/// fingerprint ([`ledger_fingerprint`]) is computed there too, with the load: with a processor plugin it is a pass over
/// every directive, which must not run on an async worker. A load that panics does not unwind into the caller, which
/// left a write without an answer and ended the listener (#492): the blocking task catches the panic, and it is a
/// failure like an error.
pub(crate) async fn load_again(reload: impl FnOnce() -> ZhangResult<Ledger> + Send + 'static) -> Result<(Ledger, u64), ReloadFailure> {
    let loaded = tokio::task::spawn_blocking(move || {
        reload().map(|reloaded| {
            let fingerprint = ledger_fingerprint(&reloaded);
            (reloaded, fingerprint)
        })
    });
    match loaded.await {
        Ok(Ok(loaded)) => Ok(loaded),
        Ok(Err(err)) => {
            error!("error on reload: {}", err);
            Err(ReloadFailure::from(&err))
        }
        Err(failure) => {
            let message = match failure.try_into_panic() {
                Ok(panic) => panic_message(panic.as_ref()).to_owned(),
                Err(failure) => failure.to_string(),
            };
            error!("panic on reload, the previous ledger is kept: {}", message);
            Err(ReloadFailure {
                file: None,
                message: format!("panic on reload: {message}"),
            })
        }
    }
}

/// The fingerprint of what a load produced, the content the generation counts ([`ReloadSender::generation`]):
///
/// - every directive's place and text in its file: the span the loader keeps on each one (file, offsets and the text
///   as written), so an edit that changes a directive, or a byte before one (its offsets move), gives another;
/// - the directives without a file, which plugins made: their canonical JSON;
/// - the options, by key;
/// - the code of every plugin, as the load read it ([`zhang_core::plugin::store::RegisteredPlugin::hash_code`]): a
///   wasm module's bytes, a Lua script's source (its module is the interpreter zhang embeds, the same for every
///   script), a bundled plugin's embedded file. So a module or a script updated in place counts, whatever the plugin
///   is, and voids the write tokens minted before; what its directive grants and configures is the directive's text;
/// - when a processor or a mapper plugin ran, everything it may have changed: the canonical JSON of every directive
///   and every error after the stages. What such a plugin makes depends on more than the spans: a file it read through
///   `allowed_paths`, the date, what it fetched, its module. Its output is hashed rather than what it read, so the
///   fingerprint is that of the ledger served, with no file read again and no race with an edit after the load.
///
/// Two loads of the same input give the same fingerprint. Without such plugins nothing is serialized or read again:
/// it costs a hash of the text (3.5 ms against a load of 426 ms for 50 000 transactions, where serializing them
/// through JSON cost 364 ms). With them, the JSON pass is the price of the guarantee, as each of their stages already
/// passes every directive through JSON; it is streamed into the hasher wherever the order of a directive's maps is
/// fixed (about a quarter of the cost of the sorted form). A reload computes it on its blocking thread, with the load.
/// Not a cryptographic hash: it tells a reload of the same input from a change, nothing more.
///
/// The file watcher's reload after a write loads nothing while the ledger's own files hold what the ledger served was
/// loaded from ([`ReloadSender::reload_if_files_changed`]), and keeps the generation as a load of them would: what
/// else the fingerprint sees of the input (a plugin's module or script in the ledger's directory, a file a plugin
/// read, the date) is another input of the load ([`ExtraInput`]), whose change always loads
fn ledger_fingerprint(ledger: &Ledger) -> u64 {
    use std::hash::{Hash, Hasher};

    use zhang_core::plugin::PluginType;

    // through `serde_json::Value`, whose objects are sorted, so a hash map or set hashes the same whatever order it
    // iterates in: an error's metas, the tags and links of a note or a document
    fn canonical(value: impl serde::Serialize) -> String {
        serde_json::to_value(value).map(|value| value.to_string()).unwrap_or_default()
    }

    /// the bytes a writer gets, fed to a hasher as they come
    struct HashWriter<'a, H: Hasher>(&'a mut H);

    impl<H: Hasher> std::io::Write for HashWriter<'_, H> {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.write(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    // the tags and links of a note or a document, hash sets, sorted: serialized, a set is an array in the order the
    // set iterates in, which every load picks anew
    fn sort_sets(value: &mut serde_json::Value) {
        match value {
            serde_json::Value::Object(fields) => {
                for (key, field) in fields.iter_mut() {
                    if let (true, serde_json::Value::Array(items)) = (key == "tags" || key == "links", &mut *field) {
                        items.sort_by_key(|item| item.to_string());
                    }
                    sort_sets(field);
                }
            }
            serde_json::Value::Array(items) => items.iter_mut().for_each(sort_sets),
            _ => {}
        }
    }

    // a directive as JSON, streamed into the hasher where its maps keep their order (metadata is an ordered map), a
    // quarter of the cost of the sorted form; a note or a document holds hash sets, so it takes the sorted form with
    // its sets sorted
    fn hash_directive<H: Hasher>(directive: &zhang_ast::Spanned<zhang_ast::Directive>, hasher: &mut H) {
        match &directive.data {
            zhang_ast::Directive::Note(_) | zhang_ast::Directive::Document(_) => {
                let mut value = serde_json::to_value(directive).unwrap_or_default();
                sort_sets(&mut value);
                value.to_string().hash(hasher)
            }
            _ => {
                serde_json::to_writer(HashWriter(&mut *hasher), directive).ok();
                // no JSON text holds this byte: it ends one directive before the next begins
                hasher.write_u8(0xff);
            }
        }
    }

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    let mut transformed = false;
    let mut analysed = false;
    for (plugin, types) in &ledger.plugins.ordered {
        plugin.hash_code(&mut hasher);
        transformed |= types.iter().any(|it| matches!(it, PluginType::Processor | PluginType::Mapper));
        analysed |= types.contains(&PluginType::Analyzer);
    }
    // what an analyzer published is not hashed: it is a function of what is (the ledger, its options, the analyzer's
    // code), but for the date when it read the time, which is hashed then, and for how its analysis ended, which the
    // load decides too (a timeout, the memory, the time left of the load's analyses): each analyzer's status is hashed,
    // so a load that failed where the one before published moves the generation, and a write token minted against
    // one does not commit against the other
    if analysed && ledger.extra_inputs.contains(&ExtraInput::Clock) {
        if let Some(reading) = ledger.clock_reading() {
            reading.date_naive().hash(&mut hasher);
        }
    }
    for slot in &ledger.extensions.analyzers {
        slot.provider.hash(&mut hasher);
        match slot.snapshot.get().map(|snapshot| &snapshot.status) {
            None => hasher.write_u8(0),
            Some(SnapshotStatus::Ok) => hasher.write_u8(1),
            Some(SnapshotStatus::Skipped) => hasher.write_u8(2),
            Some(SnapshotStatus::Failed { reason }) => {
                hasher.write_u8(3);
                reason.hash(&mut hasher);
            }
        }
    }
    for directive in ledger.directives.iter().chain(&ledger.metas) {
        let span = &directive.span;
        match &span.filename {
            Some(file) if !transformed => {
                file.hash(&mut hasher);
                span.start.hash(&mut hasher);
                span.end.hash(&mut hasher);
                span.content.hash(&mut hasher);
            }
            // a directive a plugin made, or any directive once a plugin could have changed it
            _ => hash_directive(directive, &mut hasher),
        }
    }
    if transformed {
        for error in &ledger.errors {
            canonical(error).hash(&mut hasher);
        }
    }
    // the options come out of a hash map in no fixed order: by key, so two loads list them alike
    let mut options = ledger.options.all();
    options.sort_by(|a, b| a.key.cmp(&b.key));
    for option in options {
        option.key.hash(&mut hasher);
        option.value.hash(&mut hasher);
    }
    hasher.finish()
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
    let plugin_routes = Arc::new(PluginRoutes::new(opts.plugins.routes));
    ServerApp {
        opts,
        ledger,
        broadcaster,
        reload_sender,
        auth,
        plugin_routes,
        plugin_writes: Arc::new(PluginWrites::new()),
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
    use axum::http::StatusCode;
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
    use crate::routes::common::{get_basic_info, reload};
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
        let loaded = Ledger::load(dir.clone(), "main.bean".to_owned(), source).expect("load ledger");
        let ledger = Arc::new(RwLock::new(loaded));
        let (sender, receiver) = mpsc::channel(1);
        let reload_sender = Arc::new(ReloadSender::new(sender));
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
            assertions = ledger
                .outcomes
                .iter()
                .filter(|it| matches!(it.detail, zhang_core::outcome::Detail::Assertion { .. }))
                .count();
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
                ..Default::default()
            })
        }
    }

    /// the accounts of the ledger served, sorted
    async fn accounts(ledger: &RwLock<Ledger>) -> Vec<String> {
        let ledger = ledger.read().await;
        let opens = ledger.directives.iter().filter_map(|it| match &it.data {
            zhang_ast::Directive::Open(open) => Some(open.account.name().to_owned()),
            _ => None,
        });
        let mut accounts: Vec<String> = opens.collect();
        accounts.sort();
        accounts.dedup();
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
        let loaded = Ledger::load(PathBuf::from("/panicking"), "main.zhang".to_owned(), source.clone()).expect("load ledger");
        let ledger = Arc::new(RwLock::new(loaded));
        let (sender, receiver) = mpsc::channel(1);
        let reload_sender = Arc::new(ReloadSender::new(sender));
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

    /// A write on a ledger an earlier write left stale reloads it first, by the reload the listener runs: a reload that
    /// panics is answered with a 409 that says nothing was written, kept for `/api/info` and told to the readers. The
    /// write's own reload had no guard, and the panic dropped the connection without an answer.
    #[tokio::test]
    async fn a_write_whose_reload_panics_is_answered_and_recorded() {
        let source = Arc::new(Panicking {
            panicking: AtomicBool::new(false),
            content: Mutex::new("1970-01-01 commodity CNY\n1970-01-01 open Assets:A\n".to_owned()),
            loads: AtomicUsize::new(0),
        });
        let mut loaded = Ledger::load(PathBuf::from("/panicking-write"), "main.zhang".to_owned(), source.clone()).expect("load ledger");
        // as an earlier write leaves it
        loaded.stale = true;
        let ledger = Arc::new(RwLock::new(loaded));
        let (sender, receiver) = mpsc::channel(1);
        let reload_sender = Arc::new(ReloadSender::new(sender));
        let broadcaster = Broadcaster::create();
        start_reload_listener(ledger.clone(), broadcaster.clone(), reload_sender.clone(), receiver);
        let mut events = broadcaster.new_client().await;

        source.panicking.store(true, Ordering::SeqCst);
        let check = AccountBalanceRequest::Check {
            amount: Amount::new(BigDecimal::from(0), "CNY"),
        };
        let answer = create_account_balance(
            State(SharedLedger(ledger.clone())),
            State(SharedReloadSender(reload_sender.clone())),
            Path(("Assets:A".to_owned(),)),
            Json(check),
        )
        .await;
        let (status, message) = refused(answer.err().expect("the write is answered with the failed reload")).await;
        assert_eq!(status, StatusCode::CONFLICT, "{message}");
        assert!(
            message.contains("nothing was written") && message.contains("panic on reload: the load panicked"),
            "{message}"
        );
        let failure = reload_sender.last_failure().expect("the failure is kept for /api/info");
        assert_eq!(failure.message, "panic on reload: the load panicked");
        let mut told = false;
        while let Ok(Some(event)) = tokio::time::timeout(Duration::from_secs(5), events.recv()).await {
            if format!("{event:?}").contains("ReloadFailed") {
                told = true;
                break;
            }
        }
        assert!(told, "the readers are told the reload failed");
        assert!(ledger.read().await.stale, "the ledger is still to reload before the next write");
        assert_eq!(accounts(&ledger).await, vec!["Assets:A"], "the ledger served is the previous one");
    }

    /// the body of the error `response` is answered with: its status, and the `message`
    async fn refused(response: impl IntoResponse) -> (StatusCode, String) {
        let response = response.into_response();
        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
        (status, body["message"].as_str().unwrap_or_default().to_owned())
    }

    /// A reload that fails, as a syntax error makes it, was only logged (#492): now `POST /api/reload` answers the
    /// failure, `/api/info` shows it with the file, the ledger served stays the one loaded before it, and a reload
    /// that succeeds, once the file is fixed, clears it.
    #[tokio::test]
    async fn a_failed_reload_is_answered_and_shown_until_a_reload_succeeds() {
        let dir = std::env::temp_dir().join(format!("zhang-reload-failed-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let dir = dir.canonicalize().unwrap();
        let main = dir.join("main.zhang");
        std::fs::write(&main, "1970-01-01 open Assets:A\n").unwrap();
        let source = Arc::new(LocalFileSystemDataSource::new(ZhangDataType {}));
        let loaded = Ledger::load(dir.clone(), "main.zhang".to_owned(), source).expect("load ledger");
        let ledger = Arc::new(RwLock::new(loaded));
        let (sender, receiver) = mpsc::channel(1);
        let reload_sender = Arc::new(ReloadSender::new(sender));
        start_reload_listener(ledger.clone(), Broadcaster::create(), reload_sender.clone(), receiver);
        let info = || async {
            get_basic_info(State(SharedLedger(ledger.clone())), State(SharedReloadSender(reload_sender.clone())))
                .await
                .expect("the basic info")
                .data
        };
        assert!(info().await.reload_failure.is_none(), "nothing failed yet");

        std::fs::write(&main, "1970-01-01 open Assets:A\n1970-01-02 this is not valid\n").unwrap();
        let answer = reload(State(SharedReloadSender(reload_sender.clone()))).await;
        let (status, message) = refused(answer.err().expect("the failed reload is answered with its failure")).await;
        assert_eq!(status, StatusCode::CONFLICT, "{message}");
        assert!(message.contains("the ledger failed to reload") && message.contains("main.zhang"), "{message}");

        let failure = info().await.reload_failure.expect("the failure shows while the previous ledger is served");
        assert!(failure.file.as_deref().is_some_and(|file| file.ends_with("main.zhang")), "{failure:?}");
        assert!(failure.message.contains("main.zhang") && failure.message.contains("line 2"), "{failure:?}");
        assert_eq!(accounts(&ledger).await, vec!["Assets:A"], "the ledger served is the one loaded before");

        std::fs::write(&main, "1970-01-01 open Assets:A\n1970-01-01 open Assets:B\n").unwrap();
        reload(State(SharedReloadSender(reload_sender.clone()))).await.expect("the fixed file reloads");
        assert!(info().await.reload_failure.is_none(), "a reload that succeeds clears the failure");
        assert_eq!(accounts(&ledger).await, vec!["Assets:A", "Assets:B"], "the fixed file is served");
        std::fs::remove_dir_all(dir).ok();
    }
}

/// The generation of the ledger served ([`ReloadSender::generation`]): a counter of its content, which a reload moves
/// on only when it loaded other content than the ledger served before
#[cfg(test)]
mod generation_test {
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use tokio::sync::{mpsc, RwLock};
    use zhang_ast::SpanInfo;
    use zhang_core::data_source::{DataSource, LoadResult, LocalFileSystemDataSource};
    use zhang_core::data_type::text::ZhangDataType;
    use zhang_core::data_type::DataType;
    use zhang_core::ledger::Ledger;
    use zhang_core::ZhangResult;

    use super::{ledger_fingerprint, start_reload_listener};
    use crate::broadcast::Broadcaster;
    use crate::ReloadSender;

    const LEDGER: &str = "option \"operating_currency\" \"CNY\"\n1970-01-01 open Assets:A\n1970-01-01 open Income:X\n\
                          2024-06-01 * \"seed\"\n  Assets:A 50 CNY\n  Income:X -50 CNY\n";

    /// A reload of the same files keeps the generation, and one that loaded other content moves it on by one: a
    /// directive changed, the same directives at other offsets (a byte before them), an option changed. A reload that
    /// fails keeps it, as the ledger served stays the one loaded before, and so does the reload of the files as they
    /// were once fixed
    #[tokio::test]
    async fn a_reload_moves_the_generation_on_only_when_the_content_changed() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let main = root.join("main.zhang");
        std::fs::write(&main, LEDGER).unwrap();
        let source = Arc::new(LocalFileSystemDataSource::new(ZhangDataType {}));
        let loaded = Ledger::load(root.clone(), "main.zhang".to_owned(), source).expect("load ledger");
        let ledger = Arc::new(RwLock::new(loaded));
        let (sender, receiver) = mpsc::channel(1);
        let reload_sender = Arc::new(ReloadSender::new(sender));
        start_reload_listener(ledger.clone(), Broadcaster::create(), reload_sender.clone(), receiver);
        let reloaded = || async {
            reload_sender.reload_and_wait().await.expect("the reload succeeds");
            reload_sender.generation()
        };
        assert_eq!(reload_sender.generation(), 1, "the ledger served first");

        assert_eq!(reloaded().await, 1, "the same files keep the generation");
        assert_eq!(reloaded().await, 1, "again");

        std::fs::write(&main, LEDGER.replace("50 CNY", "60 CNY")).unwrap();
        assert_eq!(reloaded().await, 2, "a directive changed");
        assert_eq!(reloaded().await, 2, "and reloaded as it is now");

        std::fs::write(&main, format!("\n{}", LEDGER.replace("50 CNY", "60 CNY"))).unwrap();
        assert_eq!(reloaded().await, 3, "the same directives at other offsets");

        std::fs::write(&main, format!("\n{}", LEDGER.replace("50 CNY", "60 CNY").replace("\"CNY\"\n", "\"USD\"\n"))).unwrap();
        assert_eq!(reloaded().await, 4, "an option changed");

        let served = std::fs::read_to_string(&main).unwrap();
        std::fs::write(&main, format!("{served}2024-06-02 this is not valid\n")).unwrap();
        reload_sender.reload_and_wait().await.expect_err("the reload fails");
        assert_eq!(reload_sender.generation(), 4, "a failed reload keeps the generation");
        std::fs::write(&main, served).unwrap();
        assert_eq!(reloaded().await, 4, "the files as they were when the ledger served was loaded");
    }

    /// the content of the comments of the ledger served
    async fn comments(ledger: &RwLock<Ledger>) -> Vec<String> {
        let ledger = ledger.read().await;
        let comments = ledger.directives.iter().chain(&ledger.metas).filter_map(|it| match &it.data {
            zhang_ast::Directive::Comment(comment) => Some(comment.content.clone()),
            _ => None,
        });
        comments.collect()
    }

    /// What a processor plugin makes counts, whatever it read to make it: here a rules file it reads through
    /// `allowed_paths`, whose content becomes a comment at a fixed place in `main.zhang`, so that no span changes with
    /// it. A change to the rules file moves the generation on, a reload of the same files keeps it, and an update of
    /// the plugin's module in place moves it on too, though the plugin makes the same
    #[tokio::test]
    async fn what_a_processor_plugin_read_counts() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/plugins/read_rules.wat");
        std::fs::copy(fixture, root.join("read_rules.wat")).unwrap();
        std::fs::create_dir_all(root.join("rules")).unwrap();
        std::fs::write(root.join("rules/rules.txt"), "coffee: Expenses:Food").unwrap();
        std::fs::write(
            root.join("main.zhang"),
            "option \"features.plugin\" \"true\"\nplugin \"read_rules.wat\"\n  allowed_paths: \"rules\"\n  read: \"rules/rules.txt\"\n",
        )
        .unwrap();
        let source = Arc::new(LocalFileSystemDataSource::new(ZhangDataType {}));
        let loaded = Ledger::load(root.clone(), "main.zhang".to_owned(), source).expect("load ledger");
        let ledger = Arc::new(RwLock::new(loaded));
        let (sender, receiver) = mpsc::channel(1);
        let reload_sender = Arc::new(ReloadSender::new(sender));
        start_reload_listener(ledger.clone(), Broadcaster::create(), reload_sender.clone(), receiver);
        let reloaded = || async {
            reload_sender.reload_and_wait().await.expect("the reload succeeds");
            reload_sender.generation()
        };
        let served = comments(&ledger).await;
        assert!(
            served.len() == 1 && served[0].contains("Expenses:Food"),
            "the plugin read the rules: {served:?}"
        );

        assert_eq!(reloaded().await, 1, "the same rules");
        std::fs::write(root.join("rules/rules.txt"), "coffee: Expenses:Drinks").unwrap();
        assert_eq!(reloaded().await, 2, "the rules changed");
        assert!(comments(&ledger).await[0].contains("Expenses:Drinks"), "the plugin read the new rules");
        assert_eq!(reloaded().await, 2, "and reloaded as they are now");

        // the version it answers, `0.1.0` in its text, which nothing it makes holds
        let module = std::fs::read_to_string(root.join("read_rules.wat")).unwrap();
        assert_eq!(module.matches("0.1.0").count(), 1);
        std::fs::write(root.join("read_rules.wat"), module.replace("0.1.0", "0.1.1")).unwrap();
        assert_eq!(reloaded().await, 3, "the module changed");
        assert_eq!(reloaded().await, 3, "and reloaded as it is now");
    }

    /// The code of a router plugin counts, whatever runs it: an edit of a Lua script moves the generation on as an edit
    /// of a wasm module does, though the module a script runs is always the interpreter zhang embeds (only the
    /// interpreter was hashed of a script, so an edit kept the generation); a reload of the same code keeps it. A
    /// bundled plugin's code is the script built into zhang, the same on every load
    #[tokio::test]
    async fn a_plugins_code_counts_whatever_runs_it() {
        const SCRIPT: &str = "return { name = \"echo\", version = \"0.1.0\", types = { \"Router\" },\n  \
                              router = function(request) return { json = request } end }\n";
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let wasm = Path::new(env!("CARGO_MANIFEST_DIR")).join("../zhang-core/tests/plugins/router.wat");
        std::fs::copy(wasm, root.join("router.wat")).unwrap();
        std::fs::write(root.join("echo.lua"), SCRIPT).unwrap();
        std::fs::write(
            root.join("main.zhang"),
            "option \"features.plugin\" \"true\"\nplugin \"echo.lua\"\nplugin \"router.wat\"\nplugin \"zhang:mcp\"\n",
        )
        .unwrap();
        let source = Arc::new(LocalFileSystemDataSource::new(ZhangDataType {}));
        let loaded = Ledger::load(root.clone(), "main.zhang".to_owned(), source).expect("load ledger");
        assert_eq!(loaded.plugins.ordered.len(), 3, "every plugin registered");
        let ledger = Arc::new(RwLock::new(loaded));
        let (sender, receiver) = mpsc::channel(1);
        let reload_sender = Arc::new(ReloadSender::new(sender));
        start_reload_listener(ledger.clone(), Broadcaster::create(), reload_sender.clone(), receiver);
        let reloaded = || async {
            reload_sender.reload_and_wait().await.expect("the reload succeeds");
            reload_sender.generation()
        };

        assert_eq!(reloaded().await, 1, "the same code");
        assert_eq!(reloaded().await, 1, "again");

        std::fs::write(root.join("echo.lua"), SCRIPT.replace("0.1.0", "0.1.1")).unwrap();
        assert_eq!(reloaded().await, 2, "the script changed");
        assert_eq!(reloaded().await, 2, "and reloaded as it is now");
        std::fs::write(root.join("echo.lua"), format!("{}-- a comment\n", SCRIPT.replace("0.1.0", "0.1.1"))).unwrap();
        assert_eq!(reloaded().await, 3, "a comment added to the script");

        let module = std::fs::read_to_string(root.join("router.wat")).unwrap();
        std::fs::write(root.join("router.wat"), format!("{module};; a comment\n")).unwrap();
        assert_eq!(reloaded().await, 4, "the module changed");
        assert_eq!(reloaded().await, 4, "and reloaded as it is now");
    }

    /// The options count in the fingerprint apart from the directives that set them: an option set otherwise, with
    /// every directive as it was, gives another fingerprint; the same ledger always gives the same one
    #[test]
    fn an_option_changes_the_fingerprint() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::write(root.join("main.zhang"), LEDGER).unwrap();
        let source = Arc::new(LocalFileSystemDataSource::new(ZhangDataType {}));
        let mut ledger = Ledger::load(root, "main.zhang".to_owned(), source).expect("load ledger");
        let before = ledger_fingerprint(&ledger);
        assert_eq!(ledger_fingerprint(&ledger), before, "the same ledger, the same fingerprint");

        ledger.options.parse("title", "Another title", &mut vec![], &SpanInfo::default()).unwrap();

        assert_ne!(ledger_fingerprint(&ledger), before, "an option set otherwise");
    }

    /// How an analysis ended counts in the fingerprint: a load whose analyzer failed, where the same files published
    /// before, moves the generation, and so does another reason of failure
    #[test]
    fn an_analyzers_status_changes_the_fingerprint() {
        use zhang_core::extensions::{AnalyzerSlot, Snapshot};

        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::write(root.join("main.zhang"), LEDGER).unwrap();
        let source = Arc::new(LocalFileSystemDataSource::new(ZhangDataType {}));
        let mut ledger = Ledger::load(root, "main.zhang".to_owned(), source).expect("load ledger");
        let fingerprint = |ledger: &mut Ledger, snapshot: Option<Snapshot>| {
            let slot = AnalyzerSlot::new("shop");
            if let Some(snapshot) = snapshot {
                slot.snapshot.set(snapshot).unwrap();
            }
            ledger.extensions.analyzers = vec![slot];
            ledger_fingerprint(ledger)
        };
        let published = fingerprint(&mut ledger, Some(Snapshot::ok(Arc::new(()))));
        assert_eq!(fingerprint(&mut ledger, Some(Snapshot::ok(Arc::new(())))), published);
        let timed_out = fingerprint(&mut ledger, Some(Snapshot::failed("timeout")));
        assert_ne!(timed_out, published, "a failed analysis");
        assert_ne!(fingerprint(&mut ledger, Some(Snapshot::failed("memory"))), timed_out, "another reason");
        assert_ne!(fingerprint(&mut ledger, Some(Snapshot::skipped())), published, "a skipped analyzer");
    }

    /// a source that makes its directives itself, as a plugin does: none has a file or a place in one
    struct Unfiled {
        content: String,
    }

    impl DataSource for Unfiled {
        fn load(&self, entry: String, endpoint: String) -> ZhangResult<LoadResult> {
            let directives = ZhangDataType {}
                .transform(self.content.clone(), None)?
                .into_iter()
                .map(|mut directive| {
                    directive.span = SpanInfo::default();
                    directive
                })
                .collect();
            Ok(LoadResult {
                directives,
                visited_files: vec![PathBuf::from(entry).join(endpoint)],
                ..Default::default()
            })
        }
    }

    /// Directives without a file, as plugins make them, hash by their content and stably: two loads of the same
    /// directives give one fingerprint, a note's tags and links included, which are hash sets every load fills in
    /// another order; a value changed in them gives another
    #[test]
    fn directives_without_a_file_hash_stably() {
        let keys = ["alpha", "bravo", "charlie", "delta", "echo", "foxtrot", "golf", "hotel"];
        let metas: String = keys.iter().map(|key| format!("  {key}: \"{key}\"\n")).collect();
        let tags: String = keys.iter().map(|key| format!(" #{key} ^{key}")).collect();
        let content = format!(
            "1970-01-01 open Assets:A\n1970-01-01 open Income:X\n2024-06-01 * \"seed\"\n{metas}  Assets:A 50 CNY\n  Income:X -50 CNY\n\
             2024-06-02 note Assets:A \"tagged\"{tags}\n"
        );
        let fingerprint = |content: &str| {
            let source = Arc::new(Unfiled { content: content.to_owned() });
            let ledger = Ledger::load(PathBuf::from("/unfiled"), "main.zhang".to_owned(), source).expect("load ledger");
            assert!(ledger.directives.iter().all(|it| it.span.filename.is_none()), "no directive has a file");
            ledger_fingerprint(&ledger)
        };

        let first = fingerprint(&content);
        for _ in 0..16 {
            assert_eq!(fingerprint(&content), first, "the same directives, the same fingerprint");
        }
        assert_ne!(
            fingerprint(&content.replace("\"golf\"", "\"golf!\"")),
            first,
            "a value changed in their metadata"
        );
        assert_ne!(fingerprint(&content.replace("#golf", "#golfing")), first, "a tag of the note changed");
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

    impl DataSource for Joined {
        /// a source on the local file system, as `zhang serve <root>` reads one
        fn local_root(&self, entry: &Path) -> Option<PathBuf> {
            Some(entry.to_path_buf())
        }

        fn load(&self, entry: String, endpoint: String) -> ZhangResult<LoadResult> {
            let file = PathBuf::from(entry).join(endpoint);
            let content = std::fs::read_to_string(&file)?;
            let directives = ZhangDataType {}.transform(content, Some(file.to_string_lossy().into_owned()))?;
            Ok(LoadResult {
                directives,
                visited_files: vec![file],
                ..Default::default()
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
            app_return_schemes: None,
            plugins: Default::default(),
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
