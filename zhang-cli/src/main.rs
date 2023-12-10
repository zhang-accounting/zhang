use std::fmt::Debug;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use clap::{Args, Parser};
use env_logger::Env;
use log::{error, info, LevelFilter};
use self_update::Status;
use tokio::task::spawn_blocking;

use beancount::Beancount;
use zhang_core::exporter::AppendableExporter;
use zhang_core::ledger::Ledger;
use zhang_core::text::exporter::TextExporter;
use zhang_core::text::transformer::TextTransformer;
use zhang_core::transform::Transformer;
use zhang_server::ServeConfig;

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

    /// web basic auth credential to enable basic auth. or enable it via environment variable ZHANG_AUTH
    #[clap(long)]
    pub auth: Option<String>,

    /// whether the server report version info for anonymous statistics
    #[clap(long)]
    pub no_report: bool,
}

enum SupportedFormat {
    Zhang,
    Beancount,
}

impl SupportedFormat {
    fn from_path(path: impl AsRef<Path>) -> Option<SupportedFormat> {
        path.as_ref().extension().and_then(|it| it.to_str()).and_then(|ext| match ext {
            "bc" | "bean" => Some(SupportedFormat::Beancount),
            "zhang" => Some(SupportedFormat::Zhang),
            _ => None,
        })
    }
    fn transformer(&self) -> Arc<dyn Transformer + 'static> {
        match self {
            SupportedFormat::Zhang => Arc::new(TextTransformer::default()),
            SupportedFormat::Beancount => Arc::new(Beancount::default()),
        }
    }
    fn exporter(&self) -> Arc<dyn AppendableExporter> {
        match self {
            SupportedFormat::Zhang => Arc::new(TextExporter {}),
            SupportedFormat::Beancount => Arc::new(Beancount {}),
        }
    }
}

impl Opts {
    pub async fn run(self) {
        match self {
            Opts::Parse(parse_opts) => {
                let format = SupportedFormat::from_path(&parse_opts.endpoint).expect("unsupported file type");
                Ledger::load_with_database(parse_opts.path, parse_opts.endpoint, format.transformer()).expect("Cannot load ledger");
            }
            Opts::Export(_) => todo!(),
            Opts::Serve(opts) => {
                let format = SupportedFormat::from_path(&opts.endpoint).expect("unsupported file type");
                let auth_credential = opts.auth.or(std::env::var("ZHANG_AUTH").ok());
                zhang_server::serve(ServeConfig {
                    path: opts.path,
                    endpoint: opts.endpoint,
                    addr: opts.addr,
                    port: opts.port,
                    auth_credential,
                    no_report: opts.no_report,
                    exporter: format.exporter(),
                    transformer: format.transformer(),
                })
                .await
                .expect("cannot serve")
            }
            Opts::Update { verbose } => {
                info!("performing self update");
                info!("current version is {}", env!("CARGO_PKG_VERSION"));
                let update_result = spawn_blocking(move || {
                    self_update::backends::github::Update::configure()
                        .repo_owner("zhang-accounting")
                        .repo_name("zhang")
                        .bin_name("zhang")
                        .show_download_progress(verbose)
                        .show_output(verbose)
                        .current_version(env!("CARGO_PKG_VERSION"))
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

// impl ExportOpts {
//     pub async fn run(self) {
//         let result = match self {
//             ExportOpts::Beancount { file, output } => exporter::beancount::run(file, output).await,
//         };
//         match result {
//             Ok(_) => {}
//             Err(error) => {
//                 eprintln!("{}", error)
//             }
//         }
//     }
// }

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
    opts.run().await;
}

#[cfg(test)]
mod test {
    use std::io::{stdout, Write};
    use std::net::TcpStream;
    use std::sync::atomic::{AtomicU16, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    use jsonpath_rust::JsonPathQuery;
    use serde::Deserialize;
    use serde_json::Value;
    use tokio::sync::RwLock;

    use zhang_core::ledger::Ledger;
    use zhang_server::broadcast::Broadcaster;
    use zhang_server::ServeConfig;

    use crate::SupportedFormat;

    #[tokio::test]
    async fn integration_test() {
        env_logger::try_init().ok();
        type ValidationPoint = (String, serde_json::Value);
        #[derive(Deserialize)]
        struct Validation {
            uri: String,
            validations: Vec<ValidationPoint>,
        }
        let port = Arc::new(AtomicU16::new(19876));
        let paths = std::fs::read_dir("../integration-tests").unwrap();

        for path in paths {
            let path = path.unwrap();
            if path.path().is_dir() {
                {
                    let mut lock = stdout().lock();
                    writeln!(lock, "    \x1b[0;32mIntegration Test\x1b[0;0m: {}", path.path().display()).unwrap();
                }

                let pathbuf = path.path();
                port.clone().fetch_add(1, Ordering::SeqCst);

                loop {
                    if TcpStream::connect(format!("127.0.0.1:{}", port.clone().load(Ordering::SeqCst))).is_ok() {
                        port.fetch_add(1, Ordering::SeqCst);
                    } else {
                        break;
                    }
                }
                let cloned_port = port.clone();
                let local = tokio::task::LocalSet::new();
                local
                    .run_until(async move {
                        let format = SupportedFormat::Zhang;

                        let cc_port = cloned_port.clone();
                        let server_handler = tokio::task::spawn_local(async move {
                            let ledger =
                                Ledger::load_with_database(pathbuf.clone(), "main.zhang".to_owned(), format.transformer()).expect("cannot load ledger");
                            let ledger_data = Arc::new(RwLock::new(ledger));
                            let broadcaster = Broadcaster::create();

                            zhang_server::start_server(
                                ServeConfig {
                                    path: pathbuf,
                                    endpoint: "main.zhang".to_owned(),
                                    addr: "127.0.0.1".to_string(),
                                    port: cc_port.load(Ordering::SeqCst),
                                    no_report: true,
                                    exporter: format.exporter(),
                                    transformer: format.transformer(),
                                },
                                ledger_data,
                                broadcaster,
                            )
                            .await
                            .expect("cannot start server")
                        });

                        let mut times = 0;
                        while times < 100 {
                            if TcpStream::connect(format!("127.0.0.1:{}", cloned_port.clone().load(Ordering::SeqCst))).is_ok() {
                                break;
                            } else {
                                times += 1;
                                tokio::time::sleep(Duration::from_millis(10)).await;
                            }
                        }

                        let validations_content = std::fs::read_to_string(path.path().join("validations.json")).unwrap();
                        let validations: Vec<Validation> = serde_json::from_str(&validations_content).unwrap();

                        for validation in validations {
                            {
                                let mut lock = stdout().lock();
                                writeln!(lock, "      \x1b[0;32mTesting\x1b[0;0m: {}", &validation.uri).unwrap();
                            }

                            let client = reqwest::Client::new();
                            let res: serde_json::Value = client
                                .get(format!("http://127.0.0.1:{}{}", cloned_port.clone().load(Ordering::SeqCst), &validation.uri))
                                .timeout(Duration::from_secs(10))
                                .send()
                                .await
                                .expect("cannot connect to server")
                                .json()
                                .await
                                .expect("cannot serde to json");
                            for point in validation.validations {
                                {
                                    let mut lock = stdout().lock();
                                    writeln!(
                                        lock,
                                        "        \x1b[0;32mValidating\x1b[0;0m: \x1b[0;34m{}\x1b[0;0m to be \x1b[0;34m{}\x1b[0;0m",
                                        point.0, &point.1
                                    )
                                    .unwrap();
                                }

                                let value = res.clone().path(&point.0).unwrap();
                                let expected_value = Value::Array(vec![point.1]);
                                if !expected_value.eq(&value) {
                                    panic!("Validation fail: {} != {}", &expected_value, &value);
                                }
                            }
                        }
                        server_handler.abort();
                    })
                    .await;
            }
        }
    }
}
