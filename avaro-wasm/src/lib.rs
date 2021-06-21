mod utils;

use wasm_bindgen::prelude::*;
// use console_error_panic_hook::hook;
// When the `wee_alloc` feature is enabled, use `wee_alloc` as the global
// allocator.
#[cfg(feature = "wee_alloc")]
#[global_allocator]
static ALLOC: wee_alloc::WeeAlloc = wee_alloc::WeeAlloc::INIT;

#[wasm_bindgen]
extern "C" {
    fn alert(s: &str);
}

#[wasm_bindgen]
pub fn greet() {
    alert("Hello, avaro-js!");
}

#[wasm_bindgen]
pub fn parse(content: &str) -> String {
    let parser = avaro::EntryParser::new();
    match parser.parse(content) {
        Ok(entry) => serde_json::to_string(&entry).unwrap(),
        Err(e) => format!("{}", e.to_string()),
    }
}
