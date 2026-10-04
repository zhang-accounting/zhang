//! The `prices` table: one row per `price` directive, in ledger order, as in beanquery.
//!
//! zhang directives can carry a time of day, and a ledger may quote a price several times a day:
//! `time` and `timestamp` (zhang extensions, like those of `#entries`) tell those quotes apart. They
//! are not part of `SELECT *`, which stays beanquery's.

use zhang_ast::{Directive, Price, Spanned};
use zhang_core::ledger::Ledger;
use zhang_core::store::Store;

use super::directives::{directives_where, meta_value};
use super::entries::{time, timestamp};
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

fn rows<'a>(ledger: &'a Ledger, store: &'a Store, _projection: Projection) -> Vec<Record<'a>> {
    directives_where(ledger, store, |it| matches!(it, Directive::Price(_)))
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
    ColumnDef::record("meta", DataType::Str, "Metadata of the price, as `key: \"value\"` pairs.", |_, record| {
        meta_value(record)
    }),
    ColumnDef::record(
        "time",
        DataType::Str,
        "Time of day of the price in the ledger's timezone, as `HH:MM:SS`; '00:00:00' when it has none. A zhang extension.",
        time,
    ),
    ColumnDef::record(
        "timestamp",
        DataType::Int,
        "Unix time, in seconds, of the price's date and time. A zhang extension.",
        timestamp,
    ),
];
