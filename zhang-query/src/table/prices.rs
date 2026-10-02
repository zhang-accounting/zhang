//! The `prices` table: one row per `price` directive, in ledger order, as in beanquery.

use zhang_ast::{Directive, Price, Spanned};
use zhang_core::ledger::Ledger;
use zhang_core::store::Store;

use super::{ColumnDef, Record, Rows, Table};
use crate::projector::Projection;
use crate::value::{DataType, Value};

pub(super) static PRICES: Table = Table {
    name: "prices",
    description: "One row per price directive, in ledger order.",
    columns: COLUMNS,
    wildcard: &["date", "currency", "amount"],
    rows: Rows::Records(rows),
};

fn rows<'a>(ledger: &'a Ledger, _store: &'a Store, _projection: Projection) -> Vec<Record<'a>> {
    ledger
        .directives
        .iter()
        .filter(|directive| matches!(directive.data, Directive::Price(_)))
        .map(Record::Directive)
        .collect()
}

fn price<'r>(record: &'r Record<'_>) -> Option<&'r Price> {
    match record {
        Record::Directive(Spanned {
            data: Directive::Price(price), ..
        }) => Some(price),
        _ => None,
    }
}

static COLUMNS: &[ColumnDef] = &[
    ColumnDef::record("date", DataType::Date, "Date of the price.", |_, record| {
        price(record).map_or(Value::Null, |it| Value::Date(it.date.naive_date()))
    }),
    ColumnDef::record("currency", DataType::Str, "The commodity being priced.", |_, record| {
        price(record).map_or(Value::Null, |it| Value::Str(it.currency.clone()))
    }),
    ColumnDef::record("amount", DataType::Amount, "The price of one unit of the commodity.", |_, record| {
        price(record).map_or(Value::Null, |it| Value::Amount(it.amount.clone()))
    }),
];
