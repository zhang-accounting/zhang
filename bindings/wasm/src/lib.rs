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

// the playground runs as single-threaded WebAssembly, the one target talc's allocator is safe on; a native build of the
// crate, for its tests, keeps the system's
#[cfg(target_arch = "wasm32")]
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

/// The directives `directives` in the format `dialect`, processed as a ledger in that format, as zhang-core processes
/// a ledger without plugins: the playground's build has no plugin runtime, so the bundled script `zhang:budget`, the
/// only implementation of budgets, does not run, and a ledger with budgets says once that they are not interpreted
/// (`ExtensionUnavailable`, RFC #698 P3.3).
fn process(directives: Vec<Spanned<Directive>>, dialect: Dialect, source: &Arc<InMemoryDataSource>) -> ZhangResult<Ledger> {
    Ledger::process(LedgerProcessContext {
        directives,
        entry: (PathBuf::from("/"), "".to_owned()),
        dialect,
        visited_files: vec![],
        data_source: source.clone(),
        // never read: the playground runs no plugins, and nothing else asks for the time
        clock: Clock::System,
    })
}

/// the playground result of one data type: the lots of the directives `parsed` in the format `dialect`, processed as a
/// ledger in that format, or the parse error
fn parse_result(parsed: ZhangResult<Vec<Spanned<Directive>>>, dialect: Dialect, source: &Arc<InMemoryDataSource>) -> ParseResult {
    match parsed {
        Ok(directives) => {
            let ledger = process(directives, dialect, source).unwrap();
            ParseResult {
                is_pass: true,
                msg: None,
                lots: Some(serde_wasm_bindgen::to_value(&lots(&ledger)).unwrap()),
            }
        }
        Err(e) => ParseResult {
            is_pass: false,
            msg: Some(e.to_string()),
            lots: None,
        },
    }
}

#[cfg(test)]
mod test {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use beancount::Beancount;
    use zhang_core::ast::error::ErrorKind;
    use zhang_core::data_type::text::ZhangDataType;
    use zhang_core::data_type::{DataType, Dialect};
    use zhang_core::ledger::Ledger;

    use super::{lots, process, InMemoryDataSource};

    /// A ledger with lots, and no budget.
    const LEDGER: &str = "option \"operating_currency\" \"CNY\"
1970-01-01 open Assets:Bank
1970-01-01 open Assets:Broker
1970-01-01 open Expenses:Food
2024-01-02 * \"Broker\" \"buy\"
  Assets:Broker 10 AAPL {100 CNY}
  Assets:Bank -1000 CNY
2024-01-05 * \"Cafe\" \"lunch\"
  Expenses:Food 30 CNY
  Assets:Bank -30 CNY
";

    /// What makes [`LEDGER`] a ledger with budgets, in the format `dialect`: an account linked to a budget, the budget,
    /// an add to it and a posting to it, then an add to a budget never defined.
    fn budgets(dialect: Dialect) -> &'static str {
        match dialect {
            Dialect::Zhang => {
                "1970-01-01 open Expenses:Groceries
  budget: \"Food\"
2024-01-01 budget Food CNY
2024-01-01 budget-add Food 100 CNY
2024-01-06 * \"Market\" \"groceries\"
  Expenses:Groceries 20 CNY
  Assets:Bank -20 CNY
2024-01-07 budget-add Nope 1 CNY
"
            }
            Dialect::Beancount => {
                "1970-01-01 open Expenses:Groceries
  budget: \"Food\"
2024-01-01 custom \"budget\" \"Food\" \"CNY\"
2024-01-01 custom \"budget-add\" \"Food\" 100 CNY
2024-01-06 * \"Market\" \"groceries\"
  Expenses:Groceries 20 CNY
  Assets:Bank -20 CNY
2024-01-07 custom \"budget-add\" \"Nope\" 1 CNY
"
            }
        }
    }

    /// [`budgets`] without the budget directives and the `budget` metadata: the account and the posting only.
    fn without_budgets(dialect: Dialect) -> String {
        budgets(dialect)
            .lines()
            .filter(|line| !line.contains("budget"))
            .map(|line| format!("{line}\n"))
            .collect()
    }

    /// `content` processed as the playground processes it, in the format `dialect`.
    fn played(content: &str, dialect: Dialect) -> Ledger {
        let source = Arc::new(InMemoryDataSource {
            data_type: Box::new(ZhangDataType {}),
        });
        let parsed = match dialect {
            Dialect::Zhang => ZhangDataType {}.transform(content.to_owned(), None),
            Dialect::Beancount => Beancount::default().transform(content.to_owned(), None),
        };
        process(parsed.expect("the ledger parses"), dialect, &source).expect("the ledger processes")
    }

    /// The playground has no plugin runtime, so the bundled script `zhang:budget`, the only implementation of budgets
    /// (RFC #698 P3.3), does not run in it: a ledger with budgets says so once, on its first use of budgets, and
    /// reports no budget error; everything else, its lots included, is as without its budgets. A ledger without budgets
    /// is unchanged.
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn a_ledger_with_budgets_says_they_are_not_interpreted_and_is_unchanged_otherwise() {
        for dialect in [Dialect::Zhang, Dialect::Beancount] {
            let without = played(LEDGER, dialect);
            assert!(without.errors.is_empty(), "{dialect:?}: {:?}", without.errors);
            assert!(without.extensions.unavailable("zhang:budget").is_some(), "{dialect:?}");

            // with an add to a budget never defined, too: no budget error
            let with = played(&format!("{LEDGER}{}", budgets(dialect)), dialect);
            let errors = with.errors.iter().map(|error| error.error_type.clone()).collect::<Vec<_>>();
            assert_eq!(errors, [ErrorKind::ExtensionUnavailable], "{dialect:?}");
            let unavailable = &with.errors[0];
            assert_eq!(unavailable.metas["plugin"], "zhang:budget");
            #[cfg(target_arch = "wasm32")]
            assert_eq!(unavailable.metas["reason"], zhang_core::extensions::NO_PLUGIN_RUNTIME);
            // a native build of this crate gets the plugin runtime when the workspace's other crates turn it on
            #[cfg(not(target_arch = "wasm32"))]
            assert!(
                [zhang_core::extensions::NO_PLUGIN_RUNTIME, zhang_core::extensions::NOT_RUN_BY_EMBEDDER].contains(&unavailable.metas["reason"].as_str()),
                "{:?}",
                unavailable.metas
            );
            let span = unavailable.span.as_ref().expect("on a directive");
            assert_eq!(span.line, Some(LEDGER.lines().count() + 1), "{dialect:?}: on the first open linked to a budget");

            // everything else as without the budgets: the lots of the same ledger without its budget directives and
            // metadata
            let plain = played(&format!("{LEDGER}{}", without_budgets(dialect)), dialect);
            assert!(plain.errors.is_empty(), "{dialect:?}: {:?}", plain.errors);
            assert_eq!(lots(&with), lots(&plain), "{dialect:?}");
            assert_eq!(lots(&with).len(), 4, "{dialect:?}: {:?}", lots(&with));
            let lot = |cells: &[(&str, &str)]| {
                cells
                    .iter()
                    .map(|(column, value)| (column.to_string(), value.to_string()))
                    .collect::<BTreeMap<_, _>>()
            };
            let expected = vec![
                lot(&[("account", "Assets:Bank"), ("cost_label", ""), ("currency", "CNY"), ("units", "-1030")]),
                lot(&[
                    ("account", "Assets:Broker"),
                    ("cost_currency", "CNY"),
                    ("cost_date", "2024-01-02"),
                    ("cost_number", "100"),
                    ("currency", "AAPL"),
                    ("units", "10"),
                ]),
                lot(&[("account", "Expenses:Food"), ("cost_label", ""), ("currency", "CNY"), ("units", "30")]),
            ];
            assert_eq!(lots(&without), expected, "{dialect:?}");
        }
    }

    /// The playground's entry point answers as before for a ledger with budgets: it parses, and lists the lots.
    #[cfg(target_arch = "wasm32")]
    #[wasm_bindgen_test::wasm_bindgen_test]
    fn the_playground_answers_a_ledger_with_budgets() {
        for dialect in [Dialect::Zhang, Dialect::Beancount] {
            let parsed = super::parse(&format!("{LEDGER}{}", budgets(dialect)));
            let result = match dialect {
                Dialect::Zhang => parsed.zhang_parse_result(),
                Dialect::Beancount => parsed.beancount_parse_result(),
            };
            assert!(result.pass(), "{dialect:?}: {:?}", result.msg());
            let lots: Vec<BTreeMap<String, String>> = serde_wasm_bindgen::from_value(result.lots()).expect("the lots");
            assert_eq!(lots, super::lots(&played(&format!("{LEDGER}{}", budgets(dialect)), dialect)), "{dialect:?}");
            assert_eq!(lots.len(), 4, "{dialect:?}: {lots:?}");
        }
    }
}
