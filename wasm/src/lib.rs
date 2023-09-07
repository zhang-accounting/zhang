mod utils;

use std::path::PathBuf;
use std::str::FromStr;
use wasm_bindgen::prelude::*;
use zhang_core::transform::{TextFileBasedTransformer, TextTransformer};

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
    alert("Hello, zhang-js!");
}

#[wasm_bindgen]
pub fn parse(content: &str) -> String {
    let transformer = TextTransformer::default();
    let result = transformer.parse(content, PathBuf::from_str("hello").unwrap());
    "OK".to_owned()
}
