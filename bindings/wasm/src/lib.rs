use std::path::PathBuf;
use std::sync::Arc;

use beancount::Beancount;
use wasm_bindgen::prelude::*;
use zhang_core::ast::{Directive, Spanned};
use zhang_core::clock::Clock;
use zhang_core::data_type::text::ZhangDataType;
use zhang_core::data_type::DataType;
use zhang_core::ledger::{Ledger, LedgerProcessContext};
use zhang_core::ZhangResult;

use crate::data_source::InMemoryDataSource;

mod data_source;

#[global_allocator]
static ALLOCATOR: talc::wasm::WasmDynamicTalc = talc::wasm::new_wasm_dynamic_allocator();

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
pub fn parse(content: &str) -> PlayGroundParse {
    console_error_panic_hook::set_once();
    let source = Arc::new(InMemoryDataSource {
        data_type: Box::new(ZhangDataType {}),
    });
    PlayGroundParse {
        zhang: parse_result(ZhangDataType {}.transform(content.to_owned(), None), &source),
        beancount: parse_result(Beancount::default().transform(content.to_owned(), None), &source),
    }
}

/// the playground result of one data type: the store of the processed directives, or the parse error
fn parse_result(parsed: ZhangResult<Vec<Spanned<Directive>>>, source: &Arc<InMemoryDataSource>) -> ParseResult {
    match parsed {
        Ok(directives) => {
            let ledger = Ledger::process(LedgerProcessContext {
                directives,
                entry: (PathBuf::from("/"), "".to_owned()),
                visited_files: vec![],
                data_source: source.clone(),
                // never read: the playground runs no plugins, and nothing else asks for the time
                clock: Clock::System,
            })
            .unwrap();
            let store = ledger.store.read().unwrap();
            ParseResult {
                is_pass: true,
                msg: None,
                store: Some(serde_wasm_bindgen::to_value(&*store).unwrap()),
            }
        }
        Err(e) => ParseResult {
            is_pass: false,
            msg: Some(e.to_string()),
            store: None,
        },
    }
}
