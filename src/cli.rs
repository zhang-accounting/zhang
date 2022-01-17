use crate::error::AvaroResult;
use crate::importer;
use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[clap(about, version, author)]
pub enum Opts {
    #[clap(subcommand)]
    Importer(ImportOpts),

    Parse(ParseOpts),
}

#[derive(Args, Debug)]
pub struct ParseOpts {
    file: PathBuf,
}

#[derive(Subcommand, Debug)]
pub enum ImportOpts {
    Wechat { file: PathBuf, config: PathBuf },
}

impl Opts {
    pub fn run(self) {
        match self {
            Opts::Importer(importer) => importer.run(),
            Opts::Parse(_) => {}
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
