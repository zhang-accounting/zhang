use std::convert::TryFrom;
use std::fmt::Debug;
use std::future::Future;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use clap::{Args, Parser};
use log::{info, LevelFilter};
use self_update::{UpdateStrategy, VersionStatus};
use tokio::task::spawn_blocking;
use zhang_core::plugin_policy::{parse_hosts, parse_size, parse_timeout, PluginLevel, ALLOWED_HOSTS_ENV, LEVEL_ENV, MAX_MEMORY_ENV, MAX_TIMEOUT_ENV};
use zhang_server::routes::plugin_router::{PluginSettings, MAX_BODY_ENV, MAX_CONCURRENCY_ENV};
use zhang_server::ServeConfig;

use crate::opendal::OpendalDataSource;

mod allocator;
pub mod opendal;

#[derive(Parser, Debug)]
// the release version (`.build_version`, written by the release workflow; see build.rs), the one `zhang update` and
// the server report; the bare `version` would print the crate version, which stays 0.1.0
#[clap(about, version = env!("ZHANG_BUILD_VERSION"), author)]
pub enum Opts {
    /// start an internal server with frontend ui
    Serve(ServerOpts),

    /// self update
    Update {
        #[clap(short, long)]
        verbose: bool,
    },
}

#[derive(Debug, Clone, PartialEq, clap::ValueEnum)]
pub enum FileSystem {
    Fs,
    S3,
    WebDav,
    Github,
}

impl FileSystem {
    fn from_env() -> Option<FileSystem> {
        match std::env::var("ZHANG_DATA_SOURCE").as_deref() {
            Ok("fs") => Some(FileSystem::Fs),
            Ok("web-dav") => Some(FileSystem::WebDav),
            Ok("github") => Some(FileSystem::Github),
            Ok("s3") => Some(FileSystem::S3),
            _ => None,
        }
    }
}

#[derive(Args, Debug)]
pub struct ServerOpts {
    /// base path of zhang project
    pub path: PathBuf,

    /// the endpoint of main zhang file.
    #[clap(short, long, default_value = "main.zhang")]
    pub endpoint: String,

    /// serve addr
    #[clap(long, default_value = "0.0.0.0")]
    pub addr: String,

    /// serve port
    #[clap(short, long, default_value_t = 8000)]
    pub port: u16,

    /// `user:pass` credential to enable the password login, or enable it via env ZHANG_AUTH
    #[clap(long)]
    pub auth: Option<String>,

    /// enable the passkey login; the value is the secret needed to register a passkey without a session,
    /// or enable it via env ZHANG_PASSKEY. ZHANG_PASSKEY_RP_ID and ZHANG_PASSKEY_ORIGIN override the
    /// relying party id and origin derived from the request, ZHANG_SESSION_SECRET keeps the sessions across restarts,
    /// ZHANG_APP_RETURN_SCHEMES adds url schemes (comma separated) the mobile app sign-in may return to, next to zhang-app
    #[clap(long)]
    pub passkey: Option<String>,

    /// data source type, default is fs, or enable it via env ZHANG_DATA_SOURCE
    #[clap(long)]
    pub source: Option<FileSystem>,

    /// whether the server report version info for anonymous statistics
    #[clap(long)]
    pub no_report: bool,

    /// what plugins may do: `off` (none is loaded), `read` (they run and change nothing, the default) or `write` (they
    /// may write to the ledger where their `plugin` directive grants it), or set it via env ZHANG_PLUGINS.
    /// ZHANG_PLUGIN_ALLOWED_HOSTS (host patterns, comma separated, `*` for any; none by default), ZHANG_PLUGIN_MAX_TIMEOUT
    /// (`30s` by default) and ZHANG_PLUGIN_MAX_MEMORY bound what a `plugin` directive can grant
    #[clap(long, value_name = "off|read|write")]
    pub plugins: Option<PluginLevel>,
}

impl Opts {
    /// run the command; the error when it failed, so `main` prints it and exits with a non-zero code: the user
    /// sees why, and supervisors (systemd, Docker, Railway, ...) see the failure instead of a clean exit. `serve`
    /// stops gracefully once `stop` resolves; `update` does not listen to it, a signal ends it as it ends any process
    pub async fn run(self, stop: impl Future<Output = ()> + Send + 'static) -> Result<(), Box<dyn std::error::Error>> {
        match self {
            Opts::Serve(mut opts) => {
                let file_system = opts.source.clone().or(FileSystem::from_env()).unwrap_or(FileSystem::Fs);
                info!("active file system is {:?}, allocator {}", file_system, allocator::NAME);
                let data_source = OpendalDataSource::from_env(file_system.clone(), &mut opts).await?;
                let auth_credential = opts.auth.or(std::env::var("ZHANG_AUTH").ok()).filter(|it| it.contains(':'));
                let passkey_secret = opts.passkey.or_else(|| env_value("ZHANG_PASSKEY")).filter(|it| !it.is_empty());
                // a limit the operator set but mistyped stops the server, rather than running without it
                let plugins = plugin_settings(opts.plugins, env_value)?;
                let result = zhang_server::serve(
                    ServeConfig {
                        path: opts.path,
                        endpoint: opts.endpoint,
                        addr: opts.addr,
                        port: opts.port,
                        auth_credential,
                        passkey_secret,
                        passkey_rp_id: env_value("ZHANG_PASSKEY_RP_ID"),
                        passkey_origin: env_value("ZHANG_PASSKEY_ORIGIN"),
                        session_secret: env_value("ZHANG_SESSION_SECRET"),
                        app_return_schemes: env_value("ZHANG_APP_RETURN_SCHEMES"),
                        no_report: opts.no_report,
                        data_source: Arc::new(data_source),
                        plugins,
                    },
                    stop,
                )
                .await;
                // the initial load (and binding the port) can fail here; reload failures while
                // serving keep the previous ledger and never reach this point
                result?;
                Ok(())
            }
            Opts::Update { verbose } => {
                info!("performing self update");
                info!("current version is {}", env!("ZHANG_BUILD_VERSION"));
                let update_result = spawn_blocking(move || {
                    self_update::backends::github::Update::configure()
                        .repo_owner("zhang-accounting")
                        .repo_name("zhang")
                        .bin_name("zhang")
                        .show_download_progress(verbose)
                        .show_output(verbose)
                        .current_version(env!("ZHANG_BUILD_VERSION"))
                        // install the newest release, as self_update 0.x did; 1.x defaults to the newest semver-compatible one
                        .update_strategy(UpdateStrategy::Latest)
                        .build()
                        .unwrap()
                        .update()
                })
                .await
                .unwrap();
                match update_result {
                    Ok(VersionStatus::UpToDate(version)) => {
                        info!("zhang is already up to dated with version {}", version);
                        Ok(())
                    }
                    Ok(VersionStatus::Updated(version)) => {
                        info!("zhang is updated to version {}", version);
                        Ok(())
                    }
                    // `VersionStatus` is non-exhaustive
                    Ok(status) => {
                        info!("zhang self update finished: {}", status);
                        Ok(())
                    }
                    Err(e) => Err(format!("fail to update: {e}").into()),
                }
            }
        }
    }
}

/// The value of an environment variable, when it is set and not empty.
fn env_value(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|it| !it.trim().is_empty())
}

/// What plugins may use: the level `level` the `--plugins` flag gives, else `ZHANG_PLUGINS`, and the rest from the
/// environment `env` reads. `ZHANG_PLUGIN_ALLOWED_HOSTS` is host patterns separated by commas,
/// `ZHANG_PLUGIN_MAX_TIMEOUT` a timeout as the `timeout` meta writes it (`30s`), `ZHANG_PLUGIN_MAX_MEMORY` and
/// `ZHANG_PLUGIN_MAX_BODY` sizes (`256MiB`), `ZHANG_PLUGIN_MAX_CONCURRENCY` a whole number above zero. A variable that is
/// not set keeps the default; one that is set to something else is an error naming it.
fn plugin_settings(level: Option<PluginLevel>, env: impl Fn(&str) -> Option<String>) -> Result<PluginSettings, String> {
    let size = |name: &str| env(name).map(|value| parse_size(&value).map_err(|e| format!("{name}: {e}"))).transpose();
    let mut settings = PluginSettings::default();
    match level {
        Some(level) => settings.policy.level = level,
        None => {
            if let Some(value) = env(LEVEL_ENV) {
                settings.policy.level = value.parse().map_err(|e| format!("{LEVEL_ENV}: {e}"))?;
            }
        }
    }
    if let Some(value) = env(ALLOWED_HOSTS_ENV) {
        settings.policy.allowed_hosts = parse_hosts(&value);
    }
    if let Some(value) = env(MAX_TIMEOUT_ENV) {
        settings.policy.max_timeout = parse_timeout(&value).ok_or_else(|| {
            format!("{MAX_TIMEOUT_ENV}: {value:?} is not a timeout: write whole seconds, or a whole number with a unit ms, s, m or h, at most 24h")
        })?;
    }
    if let Some(max_memory) = size(MAX_MEMORY_ENV)? {
        settings.policy.max_memory = max_memory;
    }
    if let Some(max_body) = size(MAX_BODY_ENV)? {
        settings.routes.max_body = usize::try_from(max_body).map_err(|_| format!("{MAX_BODY_ENV}: {max_body} bytes is too large"))?;
    }
    if let Some(value) = env(MAX_CONCURRENCY_ENV) {
        settings.routes.max_concurrency = value
            .trim()
            .parse::<usize>()
            .ok()
            .filter(|it| *it > 0)
            .ok_or_else(|| format!("{MAX_CONCURRENCY_ENV}: {value:?} is not a whole number above zero"))?;
    }
    Ok(settings)
}

/// The log targets of dependencies that log at `info` what only their developers need, each with the level it shows at,
/// at most: wasmtime's WASI layer logs a span per system call of a plugin (`clock_time_get`, `random_get`: ten lines
/// per plugin load), gotcha the OpenAPI spec of each route as the server starts (some 45 lines), and `tracing::span`
/// carries the spans that have no fields.
const QUIET_TARGETS: [(&str, LevelFilter); 3] = [
    ("wasmtime_wasi", LevelFilter::Warn),
    ("gotcha", LevelFilter::Warn),
    ("tracing::span", LevelFilter::Warn),
];

/// The log filter, in env_logger syntax: `ZHANG_LOG`, else `RUST_LOG`, else `info`, so that errors, warnings and
/// the server's start-up lines show without any configuration (a variable set to nothing counts as not set), with the
/// targets of [`QUIET_TARGETS`] quieted ([`quiet_noisy_targets`])
fn log_filter(zhang_log: Option<&str>, rust_log: Option<&str>) -> String {
    let filter = [zhang_log, rust_log]
        .iter()
        .flatten()
        .map(|it| it.trim())
        .find(|it| !it.is_empty())
        .unwrap_or("info");
    quiet_noisy_targets(filter)
}

/// `filter` with the targets of [`QUIET_TARGETS`] shown at their level at most, and never above the level of the
/// targets `filter` names not (`RUST_LOG=error` quiets them to `error`). A target `filter` names, or a target before or
/// under it (env_logger matches a target by its start), is left as `filter` says: `RUST_LOG=info,wasmtime_wasi=trace`
/// shows the WASI spans again. A filter with no level for the targets it does not name shows none of them already.
fn quiet_noisy_targets(filter: &str) -> String {
    // `directives/regex`: the regex applies to the messages, after the directives
    let directives = filter.split_once('/').map_or(filter, |(directives, _)| directives);
    let mut others = None;
    let mut named = vec![];
    for directive in directives.split(',').map(str::trim).filter(|it| !it.is_empty()) {
        match directive.split_once('=') {
            Some((target, _)) => named.push(target.trim()),
            None => match directive.parse::<LevelFilter>() {
                Ok(level) => others = Some(level),
                // a target alone: the target at every level
                Err(_) => named.push(directive),
            },
        }
    }
    let Some(others) = others else {
        return filter.to_owned();
    };
    let quieted = QUIET_TARGETS
        .iter()
        .filter(|(target, _)| !named.iter().any(|name| target.starts_with(name) || name.starts_with(target)))
        .map(|(target, level)| format!("{target}={}", (*level).min(others).as_str().to_lowercase()))
        .collect::<Vec<_>>();
    if quieted.is_empty() {
        filter.to_owned()
    } else {
        format!("{},{filter}", quieted.join(","))
    }
}

fn init_logger() {
    let filter = log_filter(std::env::var("ZHANG_LOG").ok().as_deref(), std::env::var("RUST_LOG").ok().as_deref());
    let mut logger = env_logger::Builder::new();
    logger.parse_filters(&filter);
    // colours only when stderr is a terminal (never in a pipe or a container's log), unless `RUST_LOG_STYLE` says
    // `always` or `never`; `NO_COLOR` is honoured
    if let Ok(style) = std::env::var("RUST_LOG_STYLE") {
        logger.parse_write_style(&style);
    }
    logger.init();
}

/// The stop signals: SIGINT (Ctrl+C) and, on Unix, SIGTERM, the signal `docker stop`, Kubernetes and systemd send.
/// Each is listened for from [`StopSignals::listen`] on, so that none sent between two receptions is missed.
struct StopSignals {
    #[cfg(unix)]
    interrupt: Option<tokio::signal::unix::Signal>,
    #[cfg(unix)]
    terminate: Option<tokio::signal::unix::Signal>,
}

impl StopSignals {
    /// Listens for the stop signals; one that cannot be listened for is logged, and never received.
    fn listen() -> Self {
        #[cfg(unix)]
        {
            use tokio::signal::unix::{signal, SignalKind};
            let listen = |kind: SignalKind, name: &str| match signal(kind) {
                Ok(signal) => Some(signal),
                Err(e) => {
                    log::warn!("cannot listen for {name}: {e}");
                    None
                }
            };
            StopSignals {
                interrupt: listen(SignalKind::interrupt(), "SIGINT"),
                terminate: listen(SignalKind::terminate(), "SIGTERM"),
            }
        }
        #[cfg(not(unix))]
        {
            StopSignals {}
        }
    }

    /// The next stop signal: its name, and the exit code of a process it ends (128 and its number).
    async fn recv(&mut self) -> (&'static str, i32) {
        #[cfg(unix)]
        {
            async fn next(signal: &mut Option<tokio::signal::unix::Signal>) {
                match signal {
                    Some(signal) => {
                        signal.recv().await;
                    }
                    None => std::future::pending().await,
                }
            }
            tokio::select! {
                () = next(&mut self.interrupt) => ("SIGINT", 130),
                () = next(&mut self.terminate) => ("SIGTERM", 143),
            }
        }
        #[cfg(not(unix))]
        match tokio::signal::ctrl_c().await {
            Ok(()) => ("Ctrl+C", 130),
            Err(e) => {
                log::warn!("cannot listen for Ctrl+C: {e}");
                std::future::pending().await
            }
        }
    }
}

/// Resolves once the process is asked to stop ([`StopSignals`]). A second signal while the stop is in progress exits at
/// once, with the code of a process the signal ended (130 for SIGINT, 143 for SIGTERM).
async fn stop_signal() {
    let mut signals = StopSignals::listen();
    let (signal, _) = signals.recv().await;
    info!("received {signal}, stopping: no new connections, the requests in progress finish; send it again to exit at once");
    tokio::spawn(async move {
        let (signal, code) = signals.recv().await;
        log::warn!("received {signal} again, exiting at once");
        std::process::exit(code);
    });
}

fn main() -> ExitCode {
    // console_subscriber::init();
    allocator::install();
    init_logger();
    let opts = Opts::parse();

    // the runtime `#[tokio::main]` builds, shut down with a deadline: dropped, it would wait for the blocking work still
    // running however long it takes, past the grace of a stop
    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build().expect("the tokio runtime");
    let code = runtime.block_on(async {
        match opts.run(stop_signal()).await {
            Ok(()) => {
                println!("operation completed");
                ExitCode::SUCCESS
            }
            Err(e) => {
                // on stderr whatever the log filter is: a command that fails must say why (#491)
                eprintln!("error: {e}");
                ExitCode::FAILURE
            }
        }
    });
    runtime.shutdown_timeout(zhang_server::EXIT_GRACE);
    code
}

#[cfg(test)]
mod test {
    use std::io::{stdout, Write};
    use std::sync::Arc;

    use axum::body::Body;
    use axum::extract::Request;
    use gotcha::{GotchaApp, GotchaContext};
    use http::StatusCode;
    use http_body_util::BodyExt;
    use jsonpath_rust::JsonPathQuery;
    use serde::Deserialize;
    use serde_json::Value;
    use tempfile::tempdir;
    use tokio::sync::{mpsc, RwLock};
    use tower::util::ServiceExt;
    use zhang_core::ledger::Ledger;
    use zhang_server::broadcast::Broadcaster;
    use zhang_server::{create_server_app, ReloadSender, ServeConfig};

    use crate::opendal::OpendalDataSource;
    use crate::{FileSystem, ServerOpts};

    macro_rules! pprintln {

    ($($arg:tt)*) => {
        {
            let mut lock = stdout().lock();
            writeln!(lock, $($arg)*).unwrap();
        }

    };
}
    /// Loading a ledger with a Lua plugin compiles the Lua interpreter. With the binary's allocator, compiling it on a
    /// thread per CPU raised the peak of the process by about 10 MiB per CPU (840 MiB on 64), and kept it; compiled on
    /// a thread of its own (`zhang_core::plugin::runtime`), the peak rises by the same on any machine, and what the
    /// compilation took goes back once it is over. Ignored: the peak is the process's, which the tests running at the
    /// same time raise too. Run it alone, on a machine with many CPUs:
    /// `cargo test -p zhang --bin zhang -- --ignored --exact test::compiling_a_lua_plugin_takes_little_memory_and_gives_it_back`
    #[test]
    #[ignore]
    #[cfg(target_os = "linux")]
    fn compiling_a_lua_plugin_takes_little_memory_and_gives_it_back() {
        use zhang_core::data_source::LocalFileSystemDataSource;
        use zhang_core::data_type::text::ZhangDataType;
        use zhang_core::plugin::disk_cache::CACHE_DIR_ENV;

        fn status_mib(key: &str) -> u64 {
            let status = std::fs::read_to_string("/proc/self/status").unwrap();
            let line = status.lines().find_map(|line| line.strip_prefix(key)).unwrap();
            line.trim_start_matches(':').trim().trim_end_matches("kB").trim().parse::<u64>().unwrap() / 1024
        }
        crate::allocator::install();
        // no disk cache, so that the interpreter is compiled rather than read back
        std::env::set_var(CACHE_DIR_ENV, "");
        let tempdir = tempdir().unwrap();
        std::fs::write(tempdir.path().join("main.zhang"), "option \"features.plugin\" \"true\"\nplugin \"zhang:mcp\"\n").unwrap();
        let (peak, resident) = (status_mib("VmHWM"), status_mib("VmRSS"));
        let data_source = Arc::new(LocalFileSystemDataSource::new(ZhangDataType {}));
        let ledger = Ledger::load(tempdir.path().to_path_buf(), "main.zhang".to_string(), data_source).unwrap();
        assert!(ledger.errors.is_empty(), "{:?}", ledger.errors);
        let (raised, kept) = (status_mib("VmHWM") - peak, status_mib("VmRSS").saturating_sub(resident));
        pprintln!("compiling the Lua interpreter raised the peak by {} MiB and kept {} MiB", raised, kept);
        assert!(raised > 20, "the ledger's load compiled the interpreter: {} MiB", raised);
        assert!(raised < 120, "compiling the Lua interpreter raised the peak by {} MiB", raised);
        assert!(kept < 50, "compiling the Lua interpreter kept {} MiB", kept);
    }

    #[test]
    fn plugin_settings_read_the_limits_from_the_environment() {
        use zhang_core::plugin_policy::{PluginLevel, PluginPolicy, MIB};
        use zhang_server::routes::plugin_router::{PluginRouteLimits, PluginSettings};

        use crate::plugin_settings;

        let env =
            |pairs: &'static [(&'static str, &'static str)]| move |name: &str| pairs.iter().find(|(key, _)| *key == name).map(|(_, value)| value.to_string());
        assert_eq!(plugin_settings(None, env(&[])), Ok(PluginSettings::default()));
        assert_eq!(
            plugin_settings(
                None,
                env(&[
                    ("ZHANG_PLUGINS", "Write"),
                    ("ZHANG_PLUGIN_ALLOWED_HOSTS", "api.example.com, *.Rates.example"),
                    ("ZHANG_PLUGIN_MAX_TIMEOUT", "2m"),
                    ("ZHANG_PLUGIN_MAX_MEMORY", "64MiB"),
                    ("ZHANG_PLUGIN_MAX_BODY", "1 MiB"),
                    ("ZHANG_PLUGIN_MAX_CONCURRENCY", "2"),
                ])
            ),
            Ok(PluginSettings {
                policy: PluginPolicy {
                    level: PluginLevel::Write,
                    allowed_hosts: vec!["api.example.com".to_owned(), "*.rates.example".to_owned()],
                    max_timeout: std::time::Duration::from_secs(120),
                    max_memory: 64 * MIB,
                },
                routes: PluginRouteLimits {
                    max_body: MIB as usize,
                    max_concurrency: 2,
                    ..PluginRouteLimits::default()
                },
            })
        );
        // the flag wins over the environment
        let settings = plugin_settings(Some(PluginLevel::Off), env(&[("ZHANG_PLUGINS", "write")])).unwrap();
        assert_eq!(settings.policy.level, PluginLevel::Off);
        for (name, value) in [
            ("ZHANG_PLUGINS", "sometimes"),
            ("ZHANG_PLUGIN_MAX_TIMEOUT", "forever"),
            ("ZHANG_PLUGIN_MAX_TIMEOUT", "25h"),
            ("ZHANG_PLUGIN_MAX_MEMORY", "lots"),
            ("ZHANG_PLUGIN_MAX_BODY", "0"),
            ("ZHANG_PLUGIN_MAX_CONCURRENCY", "0"),
            ("ZHANG_PLUGIN_MAX_CONCURRENCY", "-1"),
        ] {
            let pairs: &'static [(&'static str, &'static str)] = Box::leak(Box::new([(name, value)]));
            let error = plugin_settings(None, env(pairs)).unwrap_err();
            assert!(error.starts_with(&format!("{name}: ")), "{}", error);
        }
    }

    #[test]
    fn log_filter_reads_zhang_log_then_rust_log_then_defaults_to_info() {
        use crate::log_filter;
        const QUIET: &str = "wasmtime_wasi=warn,gotcha=warn,tracing::span=warn";
        assert_eq!(log_filter(Some("debug"), None), format!("{QUIET},debug"));
        assert_eq!(log_filter(Some("zhang_core=trace"), Some("info")), "zhang_core=trace");
        assert_eq!(log_filter(None, Some("zhang_core=trace,warn")), format!("{QUIET},zhang_core=trace,warn"));
        assert_eq!(log_filter(None, None), format!("{QUIET},info"));
        // set to nothing is not set
        assert_eq!(log_filter(Some(" "), Some("")), format!("{QUIET},info"));
    }

    #[test]
    fn the_noisy_targets_are_quieted_unless_the_filter_names_them() {
        use crate::quiet_noisy_targets;
        assert_eq!(quiet_noisy_targets("info"), "wasmtime_wasi=warn,gotcha=warn,tracing::span=warn,info");
        // never louder than the other targets
        assert_eq!(quiet_noisy_targets("error"), "wasmtime_wasi=error,gotcha=error,tracing::span=error,error");
        assert_eq!(quiet_noisy_targets("OFF"), "wasmtime_wasi=off,gotcha=off,tracing::span=off,OFF");
        // a target the filter names, one before it or under it, is as the filter says
        assert_eq!(
            quiet_noisy_targets("info,wasmtime_wasi=trace"),
            "gotcha=warn,tracing::span=warn,info,wasmtime_wasi=trace"
        );
        assert_eq!(quiet_noisy_targets("info,wasmtime=debug"), "gotcha=warn,tracing::span=warn,info,wasmtime=debug");
        assert_eq!(
            quiet_noisy_targets("gotcha::router,info"),
            "wasmtime_wasi=warn,tracing::span=warn,gotcha::router,info"
        );
        assert_eq!(
            quiet_noisy_targets("debug,tracing=trace,gotcha=info,wasmtime_wasi=info"),
            "debug,tracing=trace,gotcha=info,wasmtime_wasi=info"
        );
        // no level for the targets it does not name: they show nothing already
        assert_eq!(quiet_noisy_targets("zhang_core=trace"), "zhang_core=trace");
        // the regex on the messages stays last
        assert_eq!(
            quiet_noisy_targets("info/reload"),
            "wasmtime_wasi=warn,gotcha=warn,tracing::span=warn,info/reload"
        );
    }

    /// The quieted filter, as env_logger reads it: the noisy targets show warnings and errors only, the others what
    /// the filter says
    #[test]
    fn env_logger_reads_the_quieted_filter() {
        use log::{Level, Metadata};

        use crate::log_filter;
        let enabled = |filter: &str, target: &str, level: Level| {
            let logger = env_logger::Builder::new().parse_filters(&log_filter(Some(filter), None)).build();
            log::Log::enabled(&logger, &Metadata::builder().target(target).level(level).build())
        };
        assert!(!enabled("info", "wasmtime_wasi::p1", Level::Info));
        assert!(enabled("info", "wasmtime_wasi::p1", Level::Warn));
        assert!(!enabled("info", "gotcha::router", Level::Info));
        assert!(!enabled("info", "tracing::span", Level::Info));
        assert!(enabled("info", "zhang_server", Level::Info));
        assert!(enabled("info,wasmtime_wasi=trace", "wasmtime_wasi::p1", Level::Info));
        assert!(enabled("info,gotcha=info", "gotcha::router", Level::Info));
        assert!(!enabled("error", "wasmtime_wasi::p1", Level::Warn));
    }

    type ValidationPoint = (String, Value);

    /// A request of `validations.json` and the JSONPath checks on its answer. `GET uri` by default; `method` and
    /// `body` (sent as JSON) for the others, such as the built-in queries, `POST /api/query/builtins/{name}`. A
    /// query result, `{"columns", "rows"}`, is read as its rows by column name, so a check names a cell as
    /// `$.data[0].assigned.number`.
    #[derive(Deserialize)]
    struct Validation {
        uri: String,
        #[serde(default)]
        method: Option<String>,
        #[serde(default)]
        body: Option<Value>,
        validations: Vec<ValidationPoint>,
    }

    /// A query result's `data` as its rows by column name; any other `data` as it is.
    fn rows_by_column(mut res: Value) -> Value {
        let (columns, rows) = match (res["data"]["columns"].as_array(), res["data"]["rows"].as_array()) {
            (Some(columns), Some(rows)) => (columns.clone(), rows.clone()),
            _ => return res,
        };
        let names = columns.iter().map(|it| it["name"].as_str().unwrap_or_default().to_owned()).collect::<Vec<_>>();
        let objects = rows
            .iter()
            .map(|row| {
                let cells = row.as_array().cloned().unwrap_or_default();
                Value::Object(names.iter().cloned().zip(cells).collect())
            })
            .collect::<Vec<_>>();
        res["data"] = Value::Array(objects);
        res
    }

    /// The end-to-end run of one directory of `integration-tests/`: a copy of the directory is served by the whole
    /// app as `zhang serve` builds it, once per main file it has (`main.zhang`, `main.bean`), and every request of its
    /// `validations.json` must be answered as the file says.
    async fn validate_fixture(name: &str) {
        env_logger::try_init().ok();
        let original_test_source_folder = std::path::Path::new("../integration-tests").join(name);
        pprintln!("    \x1b[0;32mIntegration Test\x1b[0;0m: {}", original_test_source_folder.display());
        let tempdir = tempdir().unwrap();
        let test_temp_folder = tempdir.path();

        for entry in walkdir::WalkDir::new(&original_test_source_folder).into_iter().filter_map(|e| e.ok()) {
            if entry.path().eq(&original_test_source_folder) {
                continue;
            }
            if entry.path().is_dir() {
                // create dir
                let target_folder = entry.path().strip_prefix(&original_test_source_folder).unwrap();
                tokio::fs::create_dir_all(test_temp_folder.join(target_folder))
                    .await
                    .expect("cannot create folder");
            } else {
                // copy file
                let target_file = entry.path().strip_prefix(&original_test_source_folder).unwrap();
                tokio::fs::copy(entry.path(), test_temp_folder.join(target_file))
                    .await
                    .expect("cannot create folder");
            }
        }
        let validations_content = std::fs::read_to_string(test_temp_folder.join("validations.json")).unwrap();
        let validations: Vec<Validation> = serde_json::from_str(&validations_content).unwrap();

        for validation in validations {
            pprintln!("      \x1b[0;32mTesting\x1b[0;0m: {}", &validation.uri);

            for main_file in ["main.zhang", "main.bean"] {
                let main_file_exists = test_temp_folder.join(main_file).exists();
                if !main_file_exists {
                    continue;
                }
                pprintln!("      \x1b[0;32mDetected main file\x1b[0;0m: {}", &main_file);
                let data_source = OpendalDataSource::from_env(
                    FileSystem::Fs,
                    &mut ServerOpts {
                        path: test_temp_folder.to_path_buf(),
                        endpoint: main_file.to_string(),
                        addr: "".to_string(),
                        port: 0,
                        auth: None,
                        passkey: None,
                        source: None,
                        no_report: false,
                        plugins: None,
                    },
                )
                .await
                .expect("a known ledger format");
                let data_source = Arc::new(data_source);
                // loaded as `zhang serve` loads it: its budgets computed by the bundled script zhang:budget
                let options = zhang_server::analysis::load_options(Arc::new(zhang_core::plugin_policy::PluginPolicy::default()));
                let ledger = Ledger::load_with_options(
                    test_temp_folder.to_path_buf(),
                    main_file.to_string(),
                    data_source.clone(),
                    zhang_core::clock::Clock::System,
                    options,
                )
                .expect("cannot load ledger");
                let ledger_data = Arc::new(RwLock::new(ledger));
                let broadcaster = Broadcaster::create();
                let (tx, _) = mpsc::channel(1);
                let reload_sender = Arc::new(ReloadSender::new(tx));
                let app = create_server_app(
                    ServeConfig {
                        path: test_temp_folder.to_path_buf(),
                        endpoint: main_file.to_string(),
                        addr: "".to_string(),
                        port: 0,
                        auth_credential: None,
                        passkey_secret: None,
                        passkey_rp_id: None,
                        passkey_origin: None,
                        session_secret: None,
                        app_return_schemes: None,
                        no_report: false,
                        data_source: data_source.clone(),
                        plugins: Default::default(),
                    },
                    ledger_data,
                    broadcaster,
                    reload_sender,
                );

                let config = app.config().await.unwrap();
                let state = app.state(&config).await.unwrap();

                let context = GotchaContext { config: config.clone(), state };

                let router = app.build_router(context.clone()).await.unwrap();

                let method = validation.method.as_deref().unwrap_or("GET").parse::<http::Method>().expect("a method");
                let body = match &validation.body {
                    Some(body) => Body::from(serde_json::to_vec(body).unwrap()),
                    None => Body::empty(),
                };
                let response = router
                    .oneshot(
                        Request::builder()
                            .method(method)
                            .uri(&validation.uri)
                            .header(http::header::CONTENT_TYPE, mime::APPLICATION_JSON.as_ref())
                            .body(body)
                            .unwrap(),
                    )
                    .await
                    .unwrap();

                assert_eq!(
                    response.status(),
                    StatusCode::OK,
                    "{} {}",
                    original_test_source_folder.display(),
                    validation.uri
                );

                let body = response.into_body().collect().await.unwrap().to_bytes();
                let res: Value = rows_by_column(serde_json::from_slice(&body).unwrap());

                for point in validation.validations.iter() {
                    pprintln!(
                        "        \x1b[0;32mValidating\x1b[0;0m: \x1b[0;34m{}\x1b[0;0m to be \x1b[0;34m{}\x1b[0;0m",
                        point.0,
                        &point.1
                    );

                    let value = res.clone().path(&point.0).unwrap();
                    let expected_value = Value::Array(vec![point.1.clone()]);
                    if !expected_value.eq(&value) {
                        panic!(
                            "Validation fail\n\
                     Test case: {} \n\
                     Test URL: {} \n\
                     Test rule: {} \n\
                     Excepted value: {} \n\
                     Get: {}",
                            original_test_source_folder.display(),
                            validation.uri,
                            point.0,
                            expected_value,
                            value
                        );
                    }
                }
            }
        }
    }

    /// One test per directory of `integration-tests/`, so that the directories run in parallel and a failure names its
    /// directory. `every_fixture_directory_is_listed` fails when a directory is added without its entry here, or an
    /// entry outlives its directory.
    macro_rules! fixture_tests {
        ($($test:ident = $dir:literal,)*) => {
            mod fixtures {
                $(
                    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
                    async fn $test() {
                        super::validate_fixture($dir).await
                    }
                )*

                #[test]
                fn every_fixture_directory_is_listed() {
                    let mut listed: Vec<&str> = vec![$($dir),*];
                    listed.sort_unstable();
                    let mut on_disk: Vec<String> = std::fs::read_dir("../integration-tests")
                        .unwrap()
                        .map(|entry| entry.unwrap())
                        .filter(|entry| entry.path().is_dir())
                        .map(|entry| entry.file_name().to_string_lossy().into_owned())
                        .collect();
                    on_disk.sort_unstable();
                    assert_eq!(
                        listed, on_disk,
                        "every directory of integration-tests/ needs an entry in `fixture_tests!`, and every entry its directory"
                    );
                }
            }
        };
    }

    fixture_tests! {
        account_booking_method = "account_booking_method",
        allow_balance_account_on_the_open_day = "allow_balance_account_on_the_open_day",
        allow_multiple_leading_zero_in_amount = "allow_multiple_leading_zero_in_amount",
        allow_multiple_leading_zero_in_amount_beancount = "allow_multiple_leading_zero_in_amount_beancount",
        budget_beancount_custom_syntax = "budget-beancount-custom-syntax",
        budget_close_and_currency = "budget-close-and-currency",
        budget_sytem_syntax_and_category = "budget-sytem-syntax-and-category",
        budget_sytem_syntax_and_category_multiple_file = "budget-sytem-syntax-and-category-multiple-file",
        comments_between_postings = "comments-between-postings",
        commodity_latest_price_ignores_future_prices = "commodity-latest-price-ignores-future-prices",
        commodity_latest_price_newer_inverse_quote = "commodity-latest-price-newer-inverse-quote",
        commodity_latest_price_round_inverse_quote = "commodity-latest-price-round-inverse-quote",
        commodity_group = "commodity_group",
        commodity_latest_price_should_return_operating_commodity = "commodity_latest_price_should_return_operating_commodity",
        cost_spec_forms = "cost-spec-forms",
        fava_demo_ledger = "fava-demo-ledger",
        implicit_posting_test = "implicit-posting-test",
        inconsistent_indent_for_postings = "inconsistent-indent-for-postings",
        inline_comment_and_trailling_space = "inline-comment-and-trailling-space",
        multiple_operating_currency_support = "multiple-operating-currency-support",
        multiple_commodity_balance = "multiple_commodity_balance",
        pad_info_should_be_used_once_beancount = "pad_info_should_be_used_once_beancount",
        posting_flags = "posting-flags",
        posting_flags_star_hash_beancount = "posting-flags-star-hash-beancount",
        posting_flags_star_hash_zhang = "posting-flags-star-hash-zhang",
        posting_lot_test_implicit_cost_lot = "posting-lot-test-implicit-cost-lot",
        posting_lot_test_label = "posting-lot-test-label",
        posting_lot_test_lifo = "posting-lot-test-lifo",
        posting_lot_test_no_enough_lot_amount = "posting-lot-test-no-enough-lot-amount",
        posting_lot_test_only_have_cost = "posting-lot-test-only-have-cost",
        posting_lot_test_only_have_cost_and_single_price = "posting-lot-test-only-have-cost-and-single-price",
        posting_lot_test_only_have_price = "posting-lot-test-only-have-price",
        posting_lot_test_posting_rows = "posting-lot-test-posting-rows",
        posting_lot_test_sub_amount_given_cost = "posting-lot-test-sub-amount-given-cost",
        posting_lot_test_sub_amount_given_cost_and_cost_date = "posting-lot-test-sub-amount-given-cost-and-cost-date",
        posting_lot_test_sub_amount_given_cost_and_cost_date_2 = "posting-lot-test-sub-amount-given-cost-and-cost-date-2",
        query_directive_saved_queries = "query-directive-saved-queries",
        query_zhang_tables = "query-zhang-tables",
        raise_error_if_posting_commodity_is_not_defined = "raise-error-if-posting-commodity-is-not-defined",
        random_order_of_links_and_tags = "random-order-of-links-and-tags",
        shold_handle_the_meta_for_operating_currency_in_commodity_directive = "shold_handle_the_meta_for_operating_currency_in_commodity_directive",
        should_raise_unbalance_error_for_unbalanced_txn = "should_raise_unbalance_error_for_unbalanced_txn",
        should_txn_support_filter_by_payee_narration = "should_txn_support_filter_by_payee_narration",
        should_txn_support_filter_by_tags_and_links = "should_txn_support_filter_by_tags_and_links",
        simple_parser = "simple-parser",
        space_of_posting_uint_can_be_empty = "space-of-posting-uint-can-be-empty",
        unquoted_meta_values_beancount = "unquoted-meta-values-beancount",
        value_holdings_through_inverse_prices = "value-holdings-through-inverse-prices",
        wildcard_include_directive_supportted = "wildcard-include-directive-supportted",
    }

    /// Sends a request to `router` as a browser on `http://localhost:8010` would; the status and the JSON body.
    async fn send(router: &axum::Router, method: http::Method, uri: &str, cookie: Option<&str>, body: Option<Value>) -> (StatusCode, Option<String>, Value) {
        let mut request = Request::builder().method(method).uri(uri).header(http::header::HOST, "localhost:8010");
        if let Some(cookie) = cookie {
            request = request.header(http::header::COOKIE, cookie);
        }
        let request = match body {
            Some(body) => request
                .header(http::header::CONTENT_TYPE, mime::APPLICATION_JSON.as_ref())
                .body(Body::from(serde_json::to_vec(&body).unwrap())),
            None => request.body(Body::empty()),
        }
        .unwrap();
        let response = router.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let cookie = response
            .headers()
            .get(http::header::SET_COOKIE)
            .map(|it| it.to_str().unwrap().split(';').next().unwrap().to_owned());
        let body = response.into_body().collect().await.unwrap().to_bytes();
        (status, cookie, serde_json::from_slice(&body).unwrap_or(Value::Null))
    }

    async fn passkey_server(path: &std::path::Path) -> axum::Router {
        let mut opts = ServerOpts {
            path: path.to_path_buf(),
            endpoint: "main.zhang".to_string(),
            addr: "".to_string(),
            port: 0,
            auth: None,
            passkey: Some("letmein".to_string()),
            source: None,
            no_report: true,
            plugins: None,
        };
        let data_source = Arc::new(OpendalDataSource::from_env(FileSystem::Fs, &mut opts).await.unwrap());
        let ledger = Ledger::load(path.to_path_buf(), "main.zhang".to_string(), data_source.clone()).expect("cannot load ledger");
        let (tx, _) = mpsc::channel(1);
        let app = create_server_app(
            ServeConfig {
                path: path.to_path_buf(),
                endpoint: "main.zhang".to_string(),
                addr: "".to_string(),
                port: 0,
                auth_credential: None,
                passkey_secret: opts.passkey.clone(),
                passkey_rp_id: None,
                passkey_origin: None,
                session_secret: Some("session-secret".to_string()),
                app_return_schemes: None,
                no_report: true,
                data_source,
                plugins: Default::default(),
            },
            Arc::new(RwLock::new(ledger)),
            Broadcaster::create(),
            Arc::new(ReloadSender::new(tx)),
        );
        let config = app.config().await.unwrap();
        let state = app.state(&config).await.unwrap();
        app.build_router(GotchaContext { config, state }).await.unwrap()
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn passkeys_persist_through_the_opendal_data_source() {
        use webauthn_authenticator_rs::softpasskey::SoftPasskey;
        use webauthn_authenticator_rs::WebauthnAuthenticator;
        use webauthn_rs::prelude::{CreationChallengeResponse, RequestChallengeResponse, Url};

        let tempdir = tempdir().unwrap();
        std::fs::write(tempdir.path().join("main.zhang"), "1970-01-01 commodity CNY\n").unwrap();
        let origin = Url::parse("http://localhost:8010").unwrap();
        let mut authenticator = WebauthnAuthenticator::new(SoftPasskey::new(true));

        let router = passkey_server(tempdir.path()).await;
        let (status, _, started) = send(
            &router,
            http::Method::POST,
            "/api/auth/passkey/register/start",
            None,
            Some(serde_json::json!({"secret": "letmein", "name": "Phone"})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{started}");
        let mut options = started["data"]["options"].clone();
        // the soft token cannot store discoverable credentials
        options["publicKey"]["authenticatorSelection"]["requireResidentKey"] = false.into();
        let options: CreationChallengeResponse = serde_json::from_value(options).unwrap();
        let credential = authenticator.do_registration(origin.clone(), options).unwrap();
        let (status, cookie, body) = send(
            &router,
            http::Method::POST,
            "/api/auth/passkey/register/finish",
            None,
            Some(serde_json::json!({"state_id": started["data"]["state_id"], "credential": credential})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let (status, _, _) = send(&router, http::Method::GET, "/api/info", cookie.as_deref(), None).await;
        assert_eq!(status, StatusCode::OK);

        // the opendal fs data source creates the folder, next to the ledger
        let stored: Vec<Value> = serde_json::from_slice(&std::fs::read(tempdir.path().join(".zhang/passkeys.json")).unwrap()).unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0]["name"], "Phone");

        // and reads it back on the next start
        let router = passkey_server(tempdir.path()).await;
        let (_, _, status) = send(&router, http::Method::GET, "/api/auth/status", None, None).await;
        assert_eq!(status["data"]["passkey_registered"], true);
        let (_, _, started) = send(&router, http::Method::POST, "/api/auth/passkey/login/start", None, None).await;
        let options: RequestChallengeResponse = serde_json::from_value(started["data"]["options"].clone()).unwrap();
        let credential = authenticator.do_authentication(origin, options).unwrap();
        let (status, cookie, body) = send(
            &router,
            http::Method::POST,
            "/api/auth/passkey/login/finish",
            None,
            Some(serde_json::json!({"state_id": started["data"]["state_id"], "credential": credential})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(cookie.is_some_and(|it| it.starts_with("zhang_session=")));
    }
}
