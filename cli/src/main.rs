use avaro::parse_avaro;

use structopt::StructOpt;

#[derive(Debug, StructOpt)]
struct Opt {
    #[structopt(name = "FILE")]
    file_name: String,
}

fn main() {
    let opt: Opt = Opt::from_args();

    let content = std::fs::read_to_string(opt.file_name).unwrap();
    let result = parse_avaro(&content);
    match result {
        Ok(entities) => {
            dbg!(entities);
        }
        Err(e) => {
            eprintln!("{}", e);
        }
    };
}
