use wasm_bindgen::prelude::*;

use zhang_core::data_type::text::ZhangDataType;
use zhang_core::data_type::DataType;

mod utils;

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
    let zhang_data_type = ZhangDataType {};
    let _result = zhang_data_type.transform(content.to_owned(), Some("main.zhang".to_owned()));
    "OK".to_owned()
}
