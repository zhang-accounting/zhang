//! The `prices` table: one row per `price` directive, in ledger order, as in beanquery.
//!
//! Its rows are those of `#entries` whose directive is a price, so it has the columns of every directive as
//! `#entries` has them (`id`, `type`, `filename`, `year`, `month`, `day`, `seq`, `metas`). zhang directives can also
//! carry a time of day, and a ledger may quote a price several times a day: `time` and `timestamp` (zhang extensions)
//! tell those quotes apart. None of these are part of `SELECT *`, which stays beanquery's.

use zhang_ast::{Directive, Price};
use zhang_core::ledger::Ledger;
use zhang_core::store::Store;

use super::directives::{directive, directives_where};
use super::entries::{DATE, DAY, FILENAME, ID, META, METAS, MONTH, SEQ, TIME, TIMESTAMP, TYPE, YEAR};
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
    match &directive(record)?.data {
        Directive::Price(price) => Some(price),
        _ => None,
    }
}

static COLUMNS: &[ColumnDef] = &[
    DATE,
    ColumnDef::record("currency", DataType::Str, "The commodity being priced.", |_, record| {
        price(record).map_or(Value::Null, |it| Value::Str(it.currency.clone()))
    }),
    ColumnDef::record("amount", DataType::Amount, "The price of one unit of the commodity.", |_, record| {
        price(record).map_or(Value::Null, |it| Value::Amount(it.amount.clone()))
    }),
    META,
    TIME,
    TIMESTAMP,
    ID,
    TYPE,
    FILENAME,
    YEAR,
    MONTH,
    DAY,
    SEQ,
    METAS,
];
