//! `#accounts`: one row per account with an `open` or `close` directive, in the order of its
//! first one, as in beanquery.
//!
//! `open` and `close` are the account's directives, as structured values: their fields are
//! read with attribute access (`open.date`, `open.currencies`, `close.date`, ...). Used on
//! their own they read as the directive's date, and like their fields they are NULL when the
//! account has no such directive.

use zhang_ast::{Close, Directive, Open, Spanned};
use zhang_core::ledger::Ledger;

use super::cache::LedgerCache;
use super::directives::{date_of, set_value, str_value};
use super::{render_meta, ColumnDef, Record, Rows, Table};
use crate::projector::Projection;
use crate::value::{DataType, Value};

pub(super) static ACCOUNTS: Table = Table {
    name: "accounts",
    description: "One row per account with an open or close directive, in the order of its first one. open and close are \
                  structured: read their fields with open.date, open.currencies, close.date, ...",
    columns: COLUMNS,
    wildcard: &["account", "open", "close"],
    rows: Rows::Records(rows),
};

fn rows<'a>(ledger: &'a Ledger, _projection: Projection) -> Vec<Record<'a>> {
    let booking = ledger.booking_methods();
    let directive = |idx: Option<usize>| idx.map(|idx| &ledger.directives[idx]);
    LedgerCache::of(ledger)
        .lookups(ledger)
        .accounts()
        .map(|(name, open, close)| Record::Account {
            name,
            open: directive(open),
            close: directive(close),
            booking: booking.get(name).copied(),
        })
        .collect()
}

fn open<'r>(record: &'r Record<'_>) -> Option<(&'r Spanned<Directive>, &'r Open)> {
    match record {
        Record::Account { open: Some(directive), .. } => match &directive.data {
            Directive::Open(open) => Some((directive, open)),
            _ => None,
        },
        _ => None,
    }
}

fn close<'r>(record: &'r Record<'_>) -> Option<(&'r Spanned<Directive>, &'r Close)> {
    match record {
        Record::Account { close: Some(directive), .. } => match &directive.data {
            Directive::Close(close) => Some((directive, close)),
            _ => None,
        },
        _ => None,
    }
}

fn date(directive: Option<(&Spanned<Directive>, impl Sized)>) -> Value {
    directive.and_then(|(it, _)| date_of(&it.data)).map_or(Value::Null, Value::Date)
}

fn meta(directive: Option<(&Spanned<Directive>, impl Sized)>) -> Value {
    directive.map_or(Value::Null, |(it, _)| render_meta(it.data.meta()))
}

static COLUMNS: &[ColumnDef] = &[
    ColumnDef::record("account", DataType::Str, "Name of the account.", |_, record| match record {
        Record::Account { name, .. } => Value::Str((*name).to_owned()),
        _ => Value::Null,
    }),
    ColumnDef::record(
        "open",
        DataType::Date,
        "The open directive of the account; on its own it reads as its date, and its fields are open.date, \
         open.account, open.currencies, open.booking and open.meta. NULL without an open directive.",
        |_, record| date(open(record)),
    ),
    ColumnDef::record(
        "close",
        DataType::Date,
        "The close directive of the account; on its own it reads as its date, and its fields are close.date, \
         close.account and close.meta. NULL while the account is not closed.",
        |_, record| date(close(record)),
    ),
    ColumnDef::record("open.date", DataType::Date, "Date the account is opened.", |_, record| date(open(record))),
    ColumnDef::record("open.account", DataType::Str, "The account of the open directive.", |_, record| {
        str_value(open(record).map(|(_, it)| it.account.name()))
    }),
    ColumnDef::record(
        "open.currencies",
        DataType::Set,
        "The currencies the account is restricted to; NULL when it accepts any.",
        |_, record| match open(record) {
            Some((_, open)) if !open.commodities.is_empty() => set_value(&open.commodities),
            _ => Value::Null,
        },
    ),
    ColumnDef::record(
        "open.booking",
        DataType::Str,
        "The booking method the account books with, as booking resolves its booking_method metadata (the last value of \
         the latest open that has one, e.g. 'FIFO'); NULL when it books with the ledger's default, also when the value is \
         not a booking method zhang implements.",
        |_, record| match record {
            Record::Account { booking: Some(method), .. } => Value::Str(method.to_string()),
            _ => Value::Null,
        },
    ),
    ColumnDef::record(
        "open.meta",
        DataType::Str,
        "Metadata of the open directive, as `key: \"value\"` pairs.",
        |_, record| meta(open(record)),
    ),
    ColumnDef::record("close.date", DataType::Date, "Date the account is closed.", |_, record| date(close(record))),
    ColumnDef::record("close.account", DataType::Str, "The account of the close directive.", |_, record| {
        str_value(close(record).map(|(_, it)| it.account.name()))
    }),
    ColumnDef::record(
        "close.meta",
        DataType::Str,
        "Metadata of the close directive, as `key: \"value\"` pairs.",
        |_, record| meta(close(record)),
    ),
];
