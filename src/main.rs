#[macro_use] extern crate log;

use clap::Parser;
use zhang::cli::Opts;


fn main() {
    env_logger::init();
    let opts = Opts::parse();
    opts.run();
}