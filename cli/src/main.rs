fn main() {
    let args: Vec<String> = std::env::args().collect();
    let option = args.get(1).unwrap();
    let content = std::fs::read_to_string(option).unwrap();
    let parser = avaro::EntryParser::new();
    let c = match parser.parse(&content) {
        Ok(entry) => { serde_json::to_string(&entry).unwrap() }
        Err(e) => format!("{{\"error\": {}}}", e.to_string())
    };
    println!("{}", c);
}
