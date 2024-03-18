use std::path::PathBuf;
use std::sync::Arc;

use beancount::Beancount;
use wasm_bindgen::prelude::*;
use zhang_core::data_type::text::ZhangDataType;
use zhang_core::data_type::DataType;
use zhang_core::ledger::Ledger;

use crate::data_source::InMemoryDataSource;

mod data_source;
mod utils;

// use console_error_panic_hook::hook;
// When the `wee_alloc` feature is enabled, use `wee_alloc` as the global
// allocator.
#[cfg(feature = "wee_alloc")]
#[global_allocator]
static ALLOC: wee_alloc::WeeAlloc = wee_alloc::WeeAlloc::INIT;

#[wasm_bindgen(getter_with_clone)]
pub struct PlayGroundParse {
    pub zhang: ParseResult,
    pub beancount: ParseResult,
}
#[wasm_bindgen]
impl PlayGroundParse {
    pub fn zhang_parse_result(&self) -> ParseResult {
        self.zhang.clone()
    }

    pub fn beancount_parse_result(&self) -> ParseResult {
        self.beancount.clone()
    }
}

#[wasm_bindgen(getter_with_clone)]
#[derive(Clone)]
pub struct ParseResult {
    is_pass: bool,
    msg: Option<String>,
    store: Option<JsValue>,
}

#[wasm_bindgen]
impl ParseResult {
    pub fn pass(&self) -> bool {
        self.is_pass
    }

    pub fn msg(&self) -> Option<String> {
        self.msg.clone()
    }
    pub fn store(&self) -> JsValue {
        self.store.clone().unwrap_or_default()
    }
}

#[wasm_bindgen]
extern "C" {
    fn alert(s: &str);
    #[wasm_bindgen(js_namespace = console)]
    fn log(s: &str);
}

#[wasm_bindgen]
pub fn greet() {
    alert("Hello, zhang-js!");
}

#[wasm_bindgen]
pub fn parse(content: &str) -> PlayGroundParse {
    console_error_panic_hook::set_once();
    let zhang_data_type = ZhangDataType {};
    let beancount_data_type = Beancount::default();

    let source = Arc::new(InMemoryDataSource {
        data_type: Box::new(ZhangDataType {}),
    });
    let zhang_parse_result = zhang_data_type.transform(content.to_owned(), None);
    let beancount_parse_result = beancount_data_type.transform(content.to_owned(), None);
    let (is_zhang_pass, zhang_store, zhang_error_msg) = match zhang_parse_result {
        Ok(data) => {
            let result = Ledger::process(data, (PathBuf::from("/"), "".to_owned()), vec![], source.clone()).unwrap();
            let result1 = result.store.read().unwrap();
            let store_js_value = serde_wasm_bindgen::to_value(&*result1).unwrap();
            (true, Some(store_js_value), None)
        }
        Err(e) => (false, None, Some(e.to_string())),
    };

    let (is_beancount_pass, beancount_store, beancount_error_msg) = match beancount_parse_result {
        Ok(data) => {
            let result = Ledger::process(data, (PathBuf::from("/"), "".to_owned()), vec![], source.clone()).unwrap();
            let result1 = result.store.read().unwrap();
            let store_js_value = serde_wasm_bindgen::to_value(&*result1).unwrap();
            (true, Some(store_js_value), None)
        }
        Err(e) => (false, None, Some(e.to_string())),
    };
    PlayGroundParse {
        zhang: ParseResult {
            is_pass: is_zhang_pass,
            msg: zhang_error_msg,
            store: zhang_store,
        },
        beancount: ParseResult {
            is_pass: is_beancount_pass,
            msg: beancount_error_msg,
            store: beancount_store,
        },
    }
}
