use clap::Parser;
use zhang::cli::Opts;

#[tokio::main]
async fn main() {
    // console_subscriber::init();
    env_logger::init();
    let opts = Opts::parse();
    opts.run().await;
}
