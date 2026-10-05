use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use beancount::Beancount;
use wasm_bindgen::prelude::*;
use zhang_core::ast::{Directive, Spanned};
use zhang_core::clock::Clock;
use zhang_core::data_type::text::ZhangDataType;
use zhang_core::data_type::{DataType, Dialect};
use zhang_core::ledger::{Ledger, LedgerProcessContext};
use zhang_core::ZhangResult;
use zhang_query::{Params, Query, Value};

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
    lots: Option<JsValue>,
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

    /// the lots every account holds, as the query engine lists them
    pub fn lots(&self) -> JsValue {
        self.lots.clone().unwrap_or_default()
    }
}

/// The lots every account holds, as the query engine lists them (`commodities.lots`): the booked postings by account,
/// commodity and lot, those with units left, each lot in the order it was opened.
const LOTS: &str = "SELECT account, sum(number) AS units, currency, cost_number, cost_currency, cost_date, cost_label \
                    GROUP BY account, currency, cost_number, cost_currency, cost_date, cost_label \
                    HAVING sum(number) != 0 \
                    ORDER BY account, first(seq), first(posting_index)";

/// the rows of [`LOTS`] on `ledger`, each by column name, without its NULL cells
fn lots(ledger: &Ledger) -> Vec<BTreeMap<String, String>> {
    let query = Query::compile(LOTS).expect("the lots query compiles");
    // the query reads no `today()`: a fixed date leaves the clock unread
    let result = query.execute_at(ledger, &Params::new(), Default::default()).expect("the lots query runs");
    result
        .rows
        .iter()
        .map(|row| {
            let cells = result.columns.iter().zip(row).filter(|(_, value)| !matches!(value, Value::Null));
            cells.map(|(column, value)| (column.name.clone(), value.to_string())).collect()
        })
        .collect()
}

#[wasm_bindgen]
pub fn parse(content: &str) -> PlayGroundParse {
    console_error_panic_hook::set_once();
    let source = Arc::new(InMemoryDataSource {
        data_type: Box::new(ZhangDataType {}),
    });
    PlayGroundParse {
        zhang: parse_result(ZhangDataType {}.transform(content.to_owned(), None), Dialect::Zhang, &source),
        beancount: parse_result(Beancount::default().transform(content.to_owned(), None), Dialect::Beancount, &source),
    }
}

/// the playground result of one data type: the store of the directives `parsed` in the format `dialect`, processed as a
/// ledger in that format, or the parse error
fn parse_result(parsed: ZhangResult<Vec<Spanned<Directive>>>, dialect: Dialect, source: &Arc<InMemoryDataSource>) -> ParseResult {
    match parsed {
        Ok(directives) => {
            let ledger = Ledger::process(LedgerProcessContext {
                directives,
                entry: (PathBuf::from("/"), "".to_owned()),
                dialect,
                visited_files: vec![],
                data_source: source.clone(),
                // never read: the playground runs no plugins, and nothing else asks for the time
                clock: Clock::System,
            })
            .unwrap();
            let lots = lots(&ledger);
            let store = ledger.store.read().unwrap();
            ParseResult {
                is_pass: true,
                msg: None,
                store: Some(serde_wasm_bindgen::to_value(&*store).unwrap()),
                lots: Some(serde_wasm_bindgen::to_value(&lots).unwrap()),
            }
        }
        Err(e) => ParseResult {
            is_pass: false,
            msg: Some(e.to_string()),
            store: None,
            lots: None,
        },
    }
}
