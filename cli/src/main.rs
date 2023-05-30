use std::fmt::Debug;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use clap::{Args, Parser};
use env_logger::Env;
use log::{error, info, LevelFilter};
use self_update::Status;
use tokio::task::spawn_blocking;

use beancount_exporter::BeancountExporter;
use beancount_transformer::BeancountTransformer;
use text_exporter::TextExporter;
use text_transformer::TextTransformer;
use zhang_core::exporter::AppendableExporter;
use zhang_core::ledger::Ledger;
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

    /// serve port
    #[clap(short, long, default_value_t = 8000)]
    pub port: u16,

    /// indicate cache database file path, use memory database if not present
    #[clap(long)]
    pub database: Option<PathBuf>,

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
            SupportedFormat::Beancount => Arc::new(BeancountTransformer::default()),
        }
    }
    fn exporter(&self) -> Arc<dyn AppendableExporter> {
        match self {
            SupportedFormat::Zhang => Arc::new(TextExporter {}),
            SupportedFormat::Beancount => Arc::new(BeancountExporter {}),
        }
    }
}

impl Opts {
    pub async fn run(self) {
        match self {
            Opts::Parse(parse_opts) => {
                let format = SupportedFormat::from_path(&parse_opts.endpoint).expect("unsupported file type");
                Ledger::load_with_database(parse_opts.path, parse_opts.endpoint, parse_opts.database, format.transformer())
                    .await
                    .expect("Cannot load ledger");
            }
            Opts::Export(_) => todo!(),
            Opts::Serve(opts) => {
                let format = SupportedFormat::from_path(&opts.endpoint).expect("unsupported file type");
                zhang_server::serve(ServeConfig {
                    path: opts.path,
                    endpoint: opts.endpoint,
                    port: opts.port,
                    database: opts.database,
                    no_report: opts.no_report,
                    exporter: format.exporter(),
                    transformer: format.transformer(),
                })
                .await
                .expect("cannot serve")
            }
            Opts::Update { verbose } => {
                dbg!(verbose);
                info!("performing self update");
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
