use std::fmt::Debug;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use clap::{Args, Parser};
use env_logger::Env;
use log::info;
use self_update::{UpdateStrategy, VersionStatus};
use tokio::task::spawn_blocking;
use zhang_server::ServeConfig;

use crate::opendal::OpendalDataSource;

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
    /// relying party id and origin derived from the request, ZHANG_SESSION_SECRET keeps the sessions across restarts
    #[clap(long)]
    pub passkey: Option<String>,

    /// data source type, default is fs, or enable it via env ZHANG_DATA_SOURCE
    #[clap(long)]
    pub source: Option<FileSystem>,

    /// whether the server report version info for anonymous statistics
    #[clap(long)]
    pub no_report: bool,
}

impl Opts {
    /// run the command; the error when it failed, so `main` prints it and exits with a non-zero code: the user
    /// sees why, and supervisors (systemd, Docker, Railway, ...) see the failure instead of a clean exit
    pub async fn run(self) -> Result<(), Box<dyn std::error::Error>> {
        match self {
            Opts::Serve(mut opts) => {
                let file_system = opts.source.clone().or(FileSystem::from_env()).unwrap_or(FileSystem::Fs);
                info!("active file system is {:?}", file_system);
                let data_source = OpendalDataSource::from_env(file_system.clone(), &mut opts).await?;
                let auth_credential = opts.auth.or(std::env::var("ZHANG_AUTH").ok()).filter(|it| it.contains(':'));
                let passkey_secret = opts.passkey.or_else(|| env_value("ZHANG_PASSKEY")).filter(|it| !it.is_empty());
                let result = zhang_server::serve(ServeConfig {
                    path: opts.path,
                    endpoint: opts.endpoint,
                    addr: opts.addr,
                    port: opts.port,
                    auth_credential,
                    passkey_secret,
                    passkey_rp_id: env_value("ZHANG_PASSKEY_RP_ID"),
                    passkey_origin: env_value("ZHANG_PASSKEY_ORIGIN"),
                    session_secret: env_value("ZHANG_SESSION_SECRET"),
                    no_report: opts.no_report,
                    data_source: Arc::new(data_source),
                })
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

/// The log filter, in env_logger syntax: `ZHANG_LOG`, else `RUST_LOG`, else `info`, so that errors, warnings and
/// the server's start-up lines show without any configuration
fn log_filter(zhang_log: Option<&str>, rust_log: Option<&str>) -> String {
    zhang_log.or(rust_log).unwrap_or("info").to_owned()
}

fn init_logger() {
    let filter = log_filter(std::env::var("ZHANG_LOG").ok().as_deref(), std::env::var("RUST_LOG").ok().as_deref());
    // `Env::new()` keeps `RUST_LOG_STYLE` for the colours; `ZHANG_LOG`, when it is set, is `filter` already
    env_logger::Builder::from_env(Env::new().filter_or("ZHANG_LOG", filter)).init();
}

#[tokio::main]
async fn main() -> ExitCode {
    // console_subscriber::init();
    init_logger();
    let opts = Opts::parse();

    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            info!("receive ctrl+c, exit");
            ExitCode::SUCCESS
        }
        result = opts.run() => match result {
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
    }
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
    #[test]
    fn log_filter_reads_zhang_log_then_rust_log_then_defaults_to_info() {
        use crate::log_filter;
        assert_eq!(log_filter(Some("debug"), None), "debug");
        assert_eq!(log_filter(Some("zhang_core=trace"), Some("info")), "zhang_core=trace");
        assert_eq!(log_filter(None, Some("zhang_core=trace,warn")), "zhang_core=trace,warn");
        assert_eq!(log_filter(None, None), "info");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn integration_test() {
        env_logger::try_init().ok();
        type ValidationPoint = (String, Value);
        #[derive(Deserialize)]
        struct Validation {
            uri: String,
            validations: Vec<ValidationPoint>,
        }
        let paths = std::fs::read_dir("../integration-tests").unwrap();

        for path in paths {
            let path = path.unwrap();
            if !path.path().is_dir() {
                continue;
            }
            let original_test_source_folder = path.path();
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
                        },
                    )
                    .await
                    .expect("a known ledger format");
                    let data_source = Arc::new(data_source);
                    let ledger = Ledger::load(test_temp_folder.to_path_buf(), main_file.to_string(), data_source.clone()).expect("cannot load ledger");
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
                            no_report: false,
                            data_source: data_source.clone(),
                        },
                        ledger_data,
                        broadcaster,
                        reload_sender,
                    );

                    let config = app.config().await.unwrap();
                    let state = app.state(&config).await.unwrap();

                    let context = GotchaContext { config: config.clone(), state };

                    let router = app.build_router(context.clone()).await.unwrap();

                    let response = router
                        .oneshot(
                            Request::builder()
                                .method(http::Method::GET)
                                .uri(&validation.uri)
                                .header(http::header::CONTENT_TYPE, mime::APPLICATION_JSON.as_ref())
                                .body(Body::empty())
                                .unwrap(),
                        )
                        .await
                        .unwrap();

                    assert_eq!(response.status(), StatusCode::OK);

                    let body = response.into_body().collect().await.unwrap().to_bytes();
                    let res: Value = serde_json::from_slice(&body).unwrap();

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
                no_report: true,
                data_source,
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
