use std::fmt::Debug;
use std::path::PathBuf;
use std::sync::Arc;

use clap::{Args, Parser};
use env_logger::Env;
use log::{error, info};
use self_update::Status;
use tokio::task::spawn_blocking;
use zhang_server::ServeConfig;

use crate::opendal::OpendalDataSource;

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
    // S3,
    WebDav,
    Github,
}

impl FileSystem {
    fn from_env() -> Option<FileSystem> {
        match std::env::var("ZHANG_DATA_SOURCE").as_deref() {
            Ok("fs") => Some(FileSystem::Fs),
            Ok("web-dav") => Some(FileSystem::WebDav),
            Ok("github") => Some(FileSystem::Github),
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
                info!("active file system is {:?}", &file_system);
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
    env_logger::Builder::default().parse_env(env).init();
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
                            source: None,
                            no_report: false,
                        },
                    )
                    .await;
                    let data_source = Arc::new(data_source);
                    let ledger = Ledger::async_load(test_temp_folder.to_path_buf(), main_file.to_string(), data_source.clone())
                        .await
                        .expect("cannot load ledger");
                    let ledger_data = Arc::new(RwLock::new(ledger));
                    let broadcaster = Broadcaster::create();
                    let (tx, _) = mpsc::channel(1);
                    let reload_sender = Arc::new(ReloadSender(tx));
                    let app = create_server_app(
                        ServeConfig {
                            path: test_temp_folder.to_path_buf(),
                            endpoint: main_file.to_string(),
                            addr: "".to_string(),
                            port: 0,
                            auth_credential: None,
                            is_local_fs: true,
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
                                &validation.uri,
                                point.0,
                                &expected_value,
                                &value
                            );
                        }
                    }
                }
            }
        }
    }
}
