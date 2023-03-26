use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;

use clap::{Args, Parser, Subcommand};
use env_logger::Env;
use log::LevelFilter;
use text_transformer::TextTransformer;
use zhang_core::exporter::DebugExporter;
use zhang_core::ledger::Ledger;
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

impl Opts {
    pub async fn run(self) {
        match self {
            Opts::Parse(parse_opts) => {
                Ledger::load_with_database::<TextTransformer>(
                    parse_opts.path,
                    parse_opts.endpoint,
                    parse_opts.database,
                )
                .await
                .expect("Cannot load ledger");
            }
            Opts::Export(opts) => todo!(),
            Opts::Serve(opts) => {
                // todo(feat): detect transformer and exporter based on file extension
                let exporter = Arc::new(DebugExporter {});
                zhang_server::serve::<TextTransformer,_>(ServeConfig {
                    path: opts.path,
                    endpoint: opts.endpoint,
                    port: opts.port,
                    database: opts.database,
                    no_report: opts.no_report,
                    exporter
                })
                    .await
                    .expect("cannot serve")
            },
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
