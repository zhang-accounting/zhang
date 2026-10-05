//! `lots`, an example processor plugin built with `zhang-plugin-sdk`: the view of a transaction a
//! plugin gets, written into the transaction's `lots` meta.
//!
//! Zhang books transactions before it calls a plugin, so for every transaction with a posting at
//! cost the plugin sees the booked postings: the units of a posting written without any are
//! filled in, a cost spec is the per-unit cost and acquisition date of the lot, and a sale across
//! several lots is one posting per lot. The plugin lists them, one entry per posting:
//!
//! ```text
//! lots: "Assets:Broker -10 AAPL {100 USD, 2024-01-10}; Assets:Broker -5 AAPL {110 USD, 2024-01-20}; Assets:Cash 1800 USD; Income:Gains -250 USD"
//! ```
//!
//! Declared with `stage: "raw"`, it runs before booking and lists the postings as written instead:
//!
//! ```text
//! lots: "Assets:Broker -15 AAPL {}; Assets:Cash 1800 USD; Income:Gains ?"
//! ```
//!
//! ```zhang
//! option "features.plugin" "true"
//! plugin "plugins/lots.wasm"
//! ```

use zhang_plugin_sdk::ast::{Posting, PostingCost, ZhangString};
use zhang_plugin_sdk::{plugin, Directive, Error, Stream};

/// the transaction meta the plugin writes
const META: &str = "lots";

plugin! {
    name: "lots",
    version: env!("CARGO_PKG_VERSION"),
    processor: process,
}

fn process(mut stream: Stream) -> Result<Stream, Error> {
    for directive in stream.iter_mut() {
        let Directive::Transaction(txn) = &mut directive.data else {
            continue;
        };
        if txn.postings.iter().any(|posting| posting.cost.is_some()) {
            let postings = txn.postings.iter().map(describe).collect::<Vec<_>>().join("; ");
            txn.meta.insert(META.to_owned(), ZhangString::quote(postings));
        }
    }
    Ok(stream)
}

/// `account units {cost, date, "label"}`; `?` for the units of a posting not booked yet
fn describe(posting: &Posting) -> String {
    let units = posting
        .units
        .as_ref()
        .map_or("?".to_owned(), |units| format!("{} {}", units.number, units.commodity));
    match &posting.cost {
        Some(cost) => format!("{} {units} {}", posting.account.name(), describe_cost(cost)),
        None => format!("{} {units}", posting.account.name()),
    }
}

fn describe_cost(cost: &PostingCost) -> String {
    let mut parts = vec![];
    if let Some(base) = &cost.base {
        parts.push(format!("{} {}", base.number, base.commodity));
    }
    if let Some(date) = &cost.date {
        parts.push(date.naive_date().to_string());
    }
    if let Some(label) = &cost.label {
        parts.push(format!("{label:?}"));
    }
    let (open, close) = if cost.total { ("{{", "}}") } else { ("{", "}") };
    format!("{open}{}{close}", parts.join(", "))
}

#[cfg(test)]
mod test {
    use std::str::FromStr;

    use zhang_plugin_sdk::ast::amount::Amount;
    use zhang_plugin_sdk::ast::{Account, Date, Posting, PostingCost, SpanInfo, Transaction};
    use zhang_plugin_sdk::bigdecimal::BigDecimal;
    use zhang_plugin_sdk::{Directive, Spanned};

    fn posting(account: &str, units: Option<(i64, &str)>, cost: Option<PostingCost>) -> Posting {
        Posting {
            flag: None,
            account: Account::from_str(account).unwrap(),
            units: units.map(|(number, commodity)| Amount::new(BigDecimal::from(number), commodity)),
            cost,
            price: None,
            comment: None,
            meta: Default::default(),
            written: None,
        }
    }

    #[test]
    fn should_list_the_postings_of_a_transaction_at_cost_as_it_sees_them() {
        let cost = PostingCost {
            base: Some(Amount::new(BigDecimal::from(100), "USD")),
            date: Some(Date::Date(chrono_date("2024-01-10"))),
            label: Some("a".to_owned()),
            total: false,
            ..PostingCost::default()
        };
        let at_cost = Transaction {
            date: Date::Date(chrono_date("2024-01-10")),
            flag: None,
            payee: None,
            narration: None,
            tags: Default::default(),
            links: Default::default(),
            postings: vec![posting("Assets:Broker", Some((10, "AAPL")), Some(cost)), posting("Assets:Cash", None, None)],
            meta: Default::default(),
        };
        let plain = Transaction {
            postings: vec![posting("Assets:Cash", Some((-1, "USD")), None), posting("Expenses:Fee", Some((1, "USD")), None)],
            ..at_cost.clone()
        };
        let stream = vec![
            Spanned::new(Directive::Transaction(at_cost), SpanInfo::default()),
            Spanned::new(Directive::Transaction(plain), SpanInfo::default()),
        ];

        let out = super::process(stream).unwrap();

        let metas: Vec<Option<String>> = out
            .iter()
            .map(|it| match &it.data {
                Directive::Transaction(txn) => txn.meta.get_one("lots").map(|it| it.as_str().to_owned()),
                _ => None,
            })
            .collect();
        assert_eq!(
            metas,
            vec![Some("Assets:Broker 10 AAPL {100 USD, 2024-01-10, \"a\"}; Assets:Cash ?".to_owned()), None]
        );
    }

    fn chrono_date(date: &str) -> zhang_plugin_sdk::chrono::NaiveDate {
        zhang_plugin_sdk::chrono::NaiveDate::from_str(date).unwrap()
    }
}
