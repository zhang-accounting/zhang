use std::fmt::Debug;
use std::path::PathBuf;
use std::sync::Arc;

use clap::{Args, Parser};
use env_logger::Env;
use log::{error, info, LevelFilter};
use self_update::Status;
use tokio::task::spawn_blocking;
use zhang_server::ServeConfig;

use crate::opendal::OpendalDataSource;

pub mod github;
pub mod opendal;

#[derive(Parser, Debug)]
#[clap(about, version, author)]
pub enum Opts {
    /// zhang parser
    Parse(ParseOpts),

    /// export to target file
    Export(ExportOpts),

    /// start an internal server with frontend ui
    Serve(ServerOpts),

    /// self update
    Update {
        #[clap(short, long)]
        verbose: bool,
    },
}

#[derive(Args, Debug)]
pub struct ParseOpts {
    /// base path of zhang project
    pub path: PathBuf,

    /// the endpoint of main zhang file.
    #[clap(short, long, default_value = "main.zhang")]
    pub endpoint: String,

    /// indicate cache database file path, using tempfile if not present
    #[clap(long)]
    pub database: Option<PathBuf>,
}
#[derive(Args, Debug)]
pub struct ExportOpts {
    /// base path of zhang project
    pub path: PathBuf,

    /// the endpoint of main zhang file.
    #[clap(short, long, default_value = "main.zhang")]
    pub endpoint: String,

    /// the endpoint of main zhang file.
    #[clap(short, long, default_value = "Text")]
    pub exporter: Exporter,
}

#[derive(Debug, Clone, clap::ValueEnum)]
pub enum Exporter {
    Text,
    Beancount,
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
            Ok("s3") => Some(FileSystem::S3),
            Ok("web-dav") => Some(FileSystem::WebDav),
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

    /// web basic auth credential to enable basic auth. or enable it via env ZHANG_AUTH
    #[clap(long)]
    pub auth: Option<String>,

    /// data source type, default is fs, or enable it via env ZHANG_AUTH
    #[clap(long)]
    pub source: Option<FileSystem>,

    /// whether the server report version info for anonymous statistics
    #[clap(long)]
    pub no_report: bool,
}

impl Opts {
    pub async fn run(self) {
        match self {
            Opts::Parse(_parse_opts) => {
                // let format = SupportedFormat::from_path(&parse_opts.endpoint).expect("unsupported file type");
                // todo: fix parse
                // Ledger::load_with_database(parse_opts.path, parse_opts.endpoint, format.transformer()).expect("Cannot load ledger");
            }
            Opts::Export(_) => todo!(),
            Opts::Serve(mut opts) => {
                let file_system = opts.source.clone().or(FileSystem::from_env()).unwrap_or(FileSystem::Fs);
                let data_source = OpendalDataSource::from_env(file_system.clone(), &mut opts).await;
                let auth_credential = opts.auth.or(std::env::var("ZHANG_AUTH").ok()).filter(|it| it.contains(':'));
                let result = zhang_server::serve(ServeConfig {
                    path: opts.path,
                    endpoint: opts.endpoint,
                    addr: opts.addr,
                    port: opts.port,
                    auth_credential,
                    is_local_fs: file_system == FileSystem::Fs,
                    no_report: opts.no_report,
                    data_source: Arc::new(data_source),
                })
                .await;
                match result {
                    Ok(_) => {}
                    Err(e) => {
                        error!("An error occur when serving zhang server: {}", e)
                    }
                }
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
                        .build()
                        .unwrap()
                        .update()
                })
                .await
                .unwrap();
                match update_result {
                    Ok(Status::UpToDate(version)) => info!("zhang is already up to dated with version {}", version),
                    Ok(Status::Updated(version)) => info!("zhang is updated to version {}", version),
                    Err(e) => error!("fail to update: {}", e),
                }
            }
        }
    }
}

#[tokio::main]
async fn main() {
    // console_subscriber::init();
    let env = Env::new().filter("ZHANG_LOG").default_filter_or("RUST_LOG");
    env_logger::Builder::default()
        .filter_level(LevelFilter::Error)
        .filter_module("zhang", LevelFilter::Info)
        .parse_env(env)
        .init();
    let opts = Opts::parse();

    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            info!("receive ctrl+c, exit");
        }
        _ = opts.run() => {
            println!("operation completed");
        }
    }
}

#[cfg(test)]
mod test {
    use std::io::{stdout, Write};
    use std::sync::Arc;

    use axum::body::Body;
    use axum::extract::Request;
    use http::StatusCode;
    use http_body_util::BodyExt;
    use jsonpath_rust::JsonPathQuery;
    use serde::Deserialize;
    use serde_json::Value;
    use tokio::sync::{mpsc, RwLock};
    use tower::util::ServiceExt;
    use zhang_core::ledger::Ledger;
    use zhang_server::broadcast::Broadcaster;
    use zhang_server::{create_server_app, ReloadSender};

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
    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn integration_test() {
        env_logger::try_init().ok();
        type ValidationPoint = (String, serde_json::Value);
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
            pprintln!("    \x1b[0;32mIntegration Test\x1b[0;0m: {}", path.path().display());

            let pathbuf = path.path();
            let validations_content = std::fs::read_to_string(path.path().join("validations.json")).unwrap();
            let validations: Vec<Validation> = serde_json::from_str(&validations_content).unwrap();

            for validation in validations {
                pprintln!("      \x1b[0;32mTesting\x1b[0;0m: {}", &validation.uri);

                let data_source = OpendalDataSource::from_env(
                    FileSystem::Fs,
                    &mut ServerOpts {
                        path: pathbuf.clone(),
                        endpoint: "main.zhang".to_owned(),
                        addr: "".to_string(),
                        port: 0,
                        auth: None,
                        source: None,
                        no_report: false,
                    },
                )
                .await;
                let data_source = Arc::new(data_source);
                let ledger = Ledger::async_load(pathbuf.clone(), "main.zhang".to_owned(), data_source.clone())
                    .await
                    .expect("cannot load ledger");
                let ledger_data = Arc::new(RwLock::new(ledger));
                let broadcaster = Broadcaster::create();
                let (tx, _) = mpsc::channel(1);
                let reload_sender = Arc::new(ReloadSender(tx));
                let app = create_server_app(ledger_data, broadcaster, reload_sender, None);

                let response = app
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

                for point in validation.validations {
                    pprintln!(
                        "        \x1b[0;32mValidating\x1b[0;0m: \x1b[0;34m{}\x1b[0;0m to be \x1b[0;34m{}\x1b[0;0m",
                        point.0,
                        &point.1
                    );

                    let value = res.clone().path(&point.0).unwrap();
                    let expected_value = Value::Array(vec![point.1]);
                    if !expected_value.eq(&value) {
                        panic!("Validation fail: {} != {}", &expected_value, &value);
                    }
                }
            }
        }
    }
}
