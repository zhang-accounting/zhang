#[macro_use] extern crate log;

use clap::Parser;
use avaro::parse_avaro;
use avaro::cli::Opts;


fn main() {
    env_logger::init();
    let opts = Opts::parse();
    opts.run();
}