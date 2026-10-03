//! `guard`, an example processor plugin built with `zhang-plugin-sdk`. It
//!
//! - reports an expense posting larger than the `threshold` setting, an amount resolved per transaction from the
//!   transaction's `threshold` meta, the latest `custom "guard" "threshold" …` directive, or the plugin's
//!   `threshold` meta, except for the payees listed in the file its `allowlist` meta names;
//! - reports a transaction dated after today;
//! - stamps every transaction with a `guard-id` meta that stays the same on every load.
//!
//! ```zhang
//! option "features.plugin" "true"
//! plugin "plugins/guard.wasm"
//!   threshold: "500 CNY"
//!   allowlist: "guard/allow.txt"
//!   allowed_paths: "guard"
//!
//! 2024-01-01 custom "guard" "threshold" 100 CNY
//! ```

use std::collections::HashSet;

use zhang_plugin_sdk::ast::ZhangString;
use zhang_plugin_sdk::config::Config;
use zhang_plugin_sdk::{clock, custom, errors, fs, plugin, Directive, Error, Stream};

/// the plugin's name: what `/api/plugins` lists, and the first value of its `custom` directives
const NAME: &str = "guard";

/// the transaction meta holding the id the plugin stamps
const ID_META: &str = "guard-id";

plugin! {
    name: NAME,
    version: env!("CARGO_PKG_VERSION"),
    processor: process,
}

fn process(mut stream: Stream) -> Result<Stream, Error> {
    let config = Config::load().with_custom(custom::entries(NAME, &stream));
    // the reserved config keys and the host functions below need plugin ABI v1; say so on an older zhang
    config.abi()?;
    let today = clock::today()?;
    let allowlist = match config.get("allowlist") {
        Some(path) => read_allowlist(path.str(0)?)?,
        None => HashSet::new(),
    };

    for directive in stream.iter_mut() {
        if !matches!(directive.data, Directive::Transaction(_)) {
            continue;
        }
        // from the directive's text and the plugin's seed: the same on every load
        let id = format!("{:016x}", clock::rng_for(directive)?.next_u64());
        let span = directive.span.clone();
        let Directive::Transaction(txn) = &mut directive.data else {
            continue;
        };
        let date = txn.date.naive_date();

        if date > today {
            errors::emit_error_at(&span, format!("the transaction is dated {date}, after today ({today})"), [("rule", "future")]);
        }

        let payee = txn.payee.as_ref().map(ZhangString::as_str).unwrap_or_default();
        if !allowlist.contains(payee) {
            if let Some(threshold) = config.resolve("threshold", date, Some(&txn.meta)) {
                let threshold = threshold.amount(0)?;
                for posting in &txn.postings {
                    let Some(units) = &posting.units else {
                        continue;
                    };
                    if posting.account.name().starts_with("Expenses:") && units.commodity == threshold.commodity && units.number > threshold.number {
                        errors::emit_error_at(
                            &span,
                            format!(
                                "{} {} {} is over the threshold of {} {}",
                                posting.account.name(),
                                units.number,
                                units.commodity,
                                threshold.number,
                                threshold.commodity
                            ),
                            [
                                ("rule", "threshold".to_owned()),
                                ("threshold", format!("{} {}", threshold.number, threshold.commodity)),
                            ],
                        );
                    }
                }
            }
        }

        if txn.meta.get_one(ID_META).is_none() {
            txn.meta.insert(ID_META.to_owned(), ZhangString::quote(id));
        }
    }
    Ok(stream)
}

/// the payees listed in the file at `path`, one per line
fn read_allowlist(path: &str) -> Result<HashSet<String>, Error> {
    let text = fs::read_to_string(path)?;
    Ok(text.lines().map(str::trim).filter(|line| !line.is_empty()).map(str::to_owned).collect())
}

#[cfg(test)]
mod test {
    #[test]
    fn should_need_zhang_to_run() {
        // a native build links nothing from zhang: the host functions answer `unavailable`
        let error = super::process(vec![]).unwrap_err();
        assert!(error.message().contains("older than plugin ABI v1"), "{error}");
    }
}
