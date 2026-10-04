//! Running balances for the requested accounts, written to each transaction's `balances` meta.
//!
//! ```zhang
//! option "features.plugin" "true"
//! plugin "plugins/balances.wasm"
//!   account: "Assets"
//!   account: "Income:Gains"
//!   scope: "subtree"
//! ```
//!
//! `scope: "exact"` counts only each requested account's own postings. The default, `subtree`,
//! includes descendants. An unbooked target posting is reported as a plugin error; none of
//! that transaction changes the running balances. The plugin also lists resolved `cost-dates`,
//! so a sale split across lots demonstrates the booked view. Built-in pads run after plugins:
//! these running balances describe this stream, before pads have been generated.

use std::collections::BTreeMap;

use zhang_plugin_sdk::ast::ZhangString;
use zhang_plugin_sdk::config::Config;
use zhang_plugin_sdk::realization::{AccountScope, SparseRealization};
use zhang_plugin_sdk::{errors, plugin, Directive, Error, Stream};

plugin! {
    name: "balances",
    version: env!("CARGO_PKG_VERSION"),
    processor: process,
}

fn process(mut stream: Stream) -> Result<Stream, Error> {
    let config = Config::load();
    let accounts = config
        .meta("account")
        .ok_or_else(|| Error::msg("set at least one account meta on the balances plugin"))?;
    let scope = match config.get("scope") {
        None => AccountScope::Subtree,
        Some(value) => match value.str(0)? {
            "exact" => AccountScope::Exact,
            "subtree" => AccountScope::Subtree,
            _ => return Err(Error::msg("balances scope must be exact or subtree")),
        },
    };
    let mut balances = SparseRealization::new(accounts.iter(), scope);
    for directive in &mut stream {
        if !matches!(directive.data, Directive::Transaction(_)) {
            continue;
        }
        if let Err(error) = balances.apply(&directive.data) {
            errors::emit_error_at(&directive.span, error.to_string(), [("rule", "unbooked")]);
            continue;
        }
        let Directive::Transaction(txn) = &mut directive.data else { unreachable!() };
        let totals = balances.accounts().collect::<BTreeMap<_, _>>();
        txn.meta
            .insert("balances".to_owned(), ZhangString::quote(zhang_plugin_sdk::serde_json::to_string(&totals)?));
        let dates = txn
            .postings
            .iter()
            .filter_map(|posting| posting.cost.as_ref()?.date.as_ref().map(|date| date.naive_date().to_string()))
            .collect::<Vec<_>>();
        if !dates.is_empty() {
            txn.meta.insert("cost-dates".to_owned(), ZhangString::quote(dates.join(", ")));
        }
    }
    Ok(stream)
}
