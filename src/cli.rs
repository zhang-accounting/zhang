use crate::error::AvaroResult;
use crate::{exporter, importer};
use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;
use crate::core::load;

#[derive(Parser, Debug)]
#[clap(about, version, author)]
pub enum Opts {
    #[clap(subcommand)]
    Importer(ImportOpts),

    Parse(ParseOpts),

    /// export to target file
    #[clap(subcommand)]
    Exporter(ExportOpts),
}


#[derive(Subcommand, Debug)]
pub enum ImportOpts {
    Wechat { file: PathBuf, config: PathBuf },
}


#[derive(Args, Debug)]
pub struct ParseOpts {
    file: PathBuf,
}

#[derive(Subcommand, Debug)]
pub enum ExportOpts {
    Beancount{
        file: PathBuf,
        output: Option<PathBuf>,
    }

}

impl Opts {
    pub fn run(self) {
        match self {
            Opts::Importer(importer) => importer.run(),
            Opts::Parse(file) => {
                let result = std::fs::read_to_string(file.file).expect("cannot open file");
                load(&result);
            }
            Opts::Exporter(opts)=> {
                opts.run()
            }
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