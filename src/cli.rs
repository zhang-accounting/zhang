use crate::core::ledger::Ledger;
use crate::{exporter, importer};
use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[clap(about, version, author)]
pub enum Opts {
    /// import sources accounting file as zhang data
    #[clap(subcommand)]
    Importer(ImportOpts),

    /// zhang parser
    Parse(ParseOpts),

    /// export to target file
    #[clap(subcommand)]
    Exporter(ExportOpts),

    /// start an internal server with frontend ui
    Server(ServerOpts),
}

#[derive(Subcommand, Debug)]
pub enum ImportOpts {
    Wechat { file: PathBuf, config: PathBuf },
}

#[derive(Args, Debug)]
pub struct ParseOpts {
    /// base path of zhang project
    pub path: PathBuf,

    /// the endpoint of main zhang file.
    #[clap(short, long, default_value = "main.zhang")]
    pub endpoint: String,
}

#[derive(Subcommand, Debug)]
pub enum ExportOpts {
    Beancount {
        file: PathBuf,
        #[clap(short, long)]
        output: Option<PathBuf>,
    },
}

#[derive(Args, Debug)]
pub struct ServerOpts {
    /// base path of zhang project
    pub path: PathBuf,

    /// the endpoint of main zhang file.
    #[clap(short, long, default_value = "main.zhang")]
    pub endpoint: String,

    /// serve port
    #[clap(short, long, default_value_t = 6666)]
    pub port: u16,
}

impl Opts {
    pub fn run(self) {
        match self {
            Opts::Importer(importer) => importer.run(),
            Opts::Parse(parse_opts) => {
                Ledger::load(parse_opts.path, parse_opts.endpoint).expect("Cannot load ledger");
            }
            Opts::Exporter(opts) => opts.run(),
            Opts::Server(opts) => crate::server::serve(opts).expect("cannot serve"),
        }
    }
}

impl ImportOpts {
    pub fn run(self) {
        let result = match self {
            ImportOpts::Wechat { file, config } => importer::wechat::run(file, config),
        };
        match result {
            Ok(_) => {}
            Err(error) => {
                eprintln!("{}", error)
            }
        }
        // dbg!(result);
    }
}

impl ExportOpts {
    pub fn run(self) {
        let result = match self {
            ExportOpts::Beancount { file, output } => exporter::beancount::run(file, output),
        };
        match result {
            Ok(_) => {}
            Err(error) => {
                eprintln!("{}", error)
            }
        }
        // dbg!(result);
    }
}
