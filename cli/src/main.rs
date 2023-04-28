use std::path::PathBuf;
use std::sync::Arc;

use beancount_transformer::BeancountTransformer;
use clap::{Args, Parser};
use env_logger::Env;
use log::LevelFilter;
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
    pub exporer: Exporter,
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
    fn transformer(&self) -> Arc<dyn Transformer + 'static> {
        match self {
            SupportedFormat::Zhang => Arc::new(TextTransformer::default()),
            SupportedFormat::Beancount => Arc::new(BeancountTransformer::default()),
        }
    }
    fn exporter(&self) -> Arc<dyn AppendableExporter> {
        Arc::new(TextExporter {})
    }
}

impl Opts {
    pub async fn run(self) {
        match self {
            Opts::Parse(parse_opts) => {
                let format = match parse_opts.endpoint.rsplit_once(".") {
                    Some((_, "bc")) | Some((_, "bean")) => SupportedFormat::Beancount,
                    _ => SupportedFormat::Zhang,
                };

                Ledger::load_with_database(
                    parse_opts.path,
                    parse_opts.endpoint,
                    parse_opts.database,
                    format.transformer()
                )
                .await
                .expect("Cannot load ledger");
            }
            Opts::Export(_) => todo!(),
            Opts::Serve(opts) => {
                let format = match opts.endpoint.rsplit_once(".") {
                    Some((_, "bc")) | Some((_, "bean")) => SupportedFormat::Beancount,
                    _ => SupportedFormat::Zhang,
                };

                zhang_server::serve(
                    ServeConfig {
                        path: opts.path,
                        endpoint: opts.endpoint,
                        port: opts.port,
                        database: opts.database,
                        no_report: opts.no_report,
                        exporter: format.exporter(),
                        transformer: format.transformer()
                    },
                )
                .await
                .expect("cannot serve")
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
