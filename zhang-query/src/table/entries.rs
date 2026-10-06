//! `#entries` (one row per directive) and `#transactions` (one row per transaction), as in
//! beanquery.
//!
//! A balance assertion is a `balance` entry, not a transaction: it books nothing. Transactions
//! the ledger rejected are not rows either (the load gives them no place). The padding
//! transactions of `balance ... with pad` (flag `P`) are transactions, as in beancount.
//!
//! Both tables read their rows from the cache of the ledger ([`super::cache::Entries`]), which
//! also gives zhang's own columns: `seq` (the position in `#entries`), and on `#transactions`
//! the transaction's `id` and the errors recorded for it (`balanced`, `errors`).

use chrono::{DateTime, Datelike, FixedOffset, NaiveDate};
use zhang_ast::{Directive, Spanned, Transaction};
use zhang_core::ledger::Ledger;

use super::cache::{EntryInfo, LedgerCache};
use super::directives::{date_value, directive, directive_time, directive_timestamp, meta_value, set_value, str_value, year};
use super::postings::{balanced, error_kinds, txn_date, txn_description, txn_flag, txn_narration, txn_payee, txn_time, txn_timestamp};
use super::{meta_pairs, ColumnDef, Dataset, Record, Rows, Table};
use crate::projector::Projection;
use crate::value::{DataType, Value};

pub(super) static ENTRIES: Table = Table {
    name: "entries",
    description: "One row per directive of the ledger (transactions, open, close, balance, price, note, document, event, \
                  commodity, custom, query and budget directives), in the order zhang processes the ledger (seq).",
    columns: ENTRY_COLUMNS,
    wildcard: &[
        "id",
        "type",
        "filename",
        "date",
        "year",
        "month",
        "day",
        "flag",
        "payee",
        "narration",
        "description",
        "tags",
        "links",
        "meta",
        "accounts",
    ],
    rows: Rows::Records(entry_rows),
};

pub(super) static TRANSACTIONS: Table = Table {
    name: "transactions",
    description: "One row per transaction, including the padding transactions of balance ... with pad, in ledger order.",
    columns: TRANSACTION_COLUMNS,
    wildcard: &["date", "flag", "payee", "narration", "tags", "links", "accounts"],
    rows: Rows::Records(transaction_rows),
};

/// The entries, in ledger order: the dated directives without the transactions the ledger
/// rejected (see [`super::cache::Entries`]).
fn entry_rows<'a>(ledger: &'a Ledger, _projection: Projection) -> Vec<Record<'a>> {
    let table = LedgerCache::of(ledger).entries(ledger);
    table.rows.iter().map(|info| entry_record(ledger, info)).collect()
}

/// The transaction entries, in the order of the ledger's directives.
fn transaction_rows<'a>(ledger: &'a Ledger, _projection: Projection) -> Vec<Record<'a>> {
    let table = LedgerCache::of(ledger).entries(ledger);
    table.transactions.iter().map(|idx| entry_record(ledger, &table.rows[*idx as usize])).collect()
}

fn entry_record<'a>(ledger: &'a Ledger, info: &'a EntryInfo) -> Record<'a> {
    Record::Entry {
        directive: &ledger.directives[info.directive as usize],
        info,
    }
}

fn entry_info<'r>(record: &'r Record<'_>) -> Option<&'r EntryInfo> {
    match record {
        Record::Entry { info, .. } => Some(info),
        _ => None,
    }
}

fn seq(record: &Record<'_>) -> Value {
    entry_info(record).map_or(Value::Null, |info| Value::Int(info.seq.into()))
}

/// The time of day of the row's directive in the ledger's timezone, as zhang books the date and
/// time of a transaction; for a transaction, the time it is booked at.
pub(super) fn time(data: &Dataset<'_>, record: &Record<'_>) -> Value {
    match booked_at(data, record) {
        Some(at) => txn_time(&at),
        None => directive(record).map_or(Value::Null, |it| directive_time(data, it)),
    }
}

/// The Unix time of the row's directive, read like [`time`].
pub(super) fn timestamp(data: &Dataset<'_>, record: &Record<'_>) -> Value {
    match booked_at(data, record) {
        Some(at) => txn_timestamp(&at),
        None => directive(record).map_or(Value::Null, |it| directive_timestamp(data, it)),
    }
}

/// Whether the row is that of a transaction the load accepted, whose columns read its directive, as the postings' do.
fn stored(record: &Record<'_>) -> bool {
    entry_info(record).is_some_and(|info| info.txn.is_some())
}

/// The date and time zhang books the transaction of an `#entries` or `#transactions` row at, in the ledger's
/// timezone; `None` for another row.
fn booked_at(data: &Dataset<'_>, record: &Record<'_>) -> Option<DateTime<FixedOffset>> {
    match directive(record).map(|it| &it.data) {
        Some(Directive::Transaction(txn)) if stored(record) => Some(txn.date.to_timezone_datetime(&data.ledger.options.timezone).fixed_offset()),
        _ => None,
    }
}

/// The date of the row: a transaction's as zhang books it, another directive's as written.
fn date(data: &Dataset<'_>, record: &Record<'_>) -> Value {
    match booked_at(data, record) {
        Some(at) => Value::Date(txn_date(&at)),
        None => date_value(record),
    }
}

/// The `year`, `month` or `day` of the row's [`date`].
fn date_part(data: &Dataset<'_>, record: &Record<'_>, part: fn(NaiveDate) -> u32) -> Value {
    match date(data, record) {
        Value::Date(date) => Value::Int(part(date) as i64),
        _ => Value::Null,
    }
}

/// The type of an entry: beancount's directive name, or zhang's keyword for the directives
/// beancount does not have. A `balance ... with pad` is a `balance`, a beancount `pad` a `pad`.
fn entry_type(directive: &Directive) -> &'static str {
    match directive {
        Directive::Open(_) => "open",
        Directive::Close(_) => "close",
        Directive::Commodity(_) => "commodity",
        Directive::Transaction(_) => "transaction",
        Directive::BalancePad(_) | Directive::BalanceCheck(_) => "balance",
        Directive::Pad(_) => "pad",
        Directive::Note(_) => "note",
        Directive::Document(_) => "document",
        Directive::Price(_) => "price",
        Directive::Event(_) => "event",
        Directive::Custom(_) => "custom",
        Directive::Query(_) => "query",
        Directive::Option(_) => "option",
        Directive::Plugin(_) => "plugin",
        Directive::Include(_) => "include",
        Directive::Comment(_) => "comment",
        Directive::Budget(_) => "budget",
        Directive::BudgetAdd(_) => "budget-add",
        Directive::BudgetTransfer(_) => "budget-transfer",
        Directive::BudgetClose(_) => "budget-close",
    }
}

/// The accounts a directive refers to.
fn entry_accounts(directive: &Directive) -> Value {
    let accounts: Vec<&str> = match directive {
        Directive::Transaction(txn) => txn.postings.iter().map(|posting| posting.account.name()).collect(),
        Directive::Open(it) => vec![it.account.name()],
        Directive::Close(it) => vec![it.account.name()],
        Directive::BalanceCheck(it) => vec![it.account.name()],
        Directive::BalancePad(it) => vec![it.account.name(), it.pad.name()],
        Directive::Pad(it) => vec![it.account.name(), it.pad.name()],
        Directive::Note(it) => vec![it.account.name()],
        Directive::Document(it) => vec![it.account.name()],
        _ => vec![],
    };
    Value::Set(accounts.into_iter().map(str::to_owned).collect())
}

fn payee(txn: &Transaction) -> Value {
    str_value(txn_payee(txn))
}

fn narration(txn: &Transaction) -> Value {
    Value::Str(txn_narration(txn).to_owned())
}

/// The tags or links of a transaction, note or document; NULL for other directives. A
/// transaction's are those of a transaction the load accepted.
fn tags_or_links(record: &Record<'_>, directive: &Directive, links: bool) -> Value {
    match directive {
        Directive::Transaction(txn) if stored(record) => set_value(if links { &txn.links } else { &txn.tags }),
        Directive::Transaction(_) => Value::Null,
        Directive::Note(note) => set_value((if links { &note.links } else { &note.tags }).iter().flatten()),
        Directive::Document(document) => set_value((if links { &document.links } else { &document.tags }).iter().flatten()),
        _ => Value::Null,
    }
}

/// `get` of the directive of a transaction the load accepted; NULL for other rows.
fn of_transaction(record: &Record<'_>, get: fn(&Transaction) -> Value) -> Value {
    match (stored(record), directive(record).map(|it| &it.data)) {
        (true, Some(Directive::Transaction(txn))) => get(txn),
        _ => Value::Null,
    }
}

fn of_directive(record: &Record<'_>, get: impl Fn(&Spanned<Directive>) -> Value) -> Value {
    directive(record).map_or(Value::Null, get)
}

// The columns of `#entries`. The tables of one kind of directive (`#transactions`, `#prices`, `#balances`, `#notes`,
// `#events` and `#commodities`) read the same rows and list these columns too: every one of them has `id`, `type`
// (except `#events`, whose `type` is the event's), `filename`, `date`, `year`, `month`, `day`, `time`, `timestamp`,
// `seq`, `meta` and `metas`.
pub(super) const ID: ColumnDef = ColumnDef::record(
    "id",
    DataType::Str,
    "Unique id of the entry: for a transaction its transaction id (the id column of its postings), for a balance \
     assertion the id zhang stored its check with, which /api/journals lists it with.",
    |data, record| entry_info(record).map_or(Value::Null, |info| Value::Str(data.entry_id(info.seq).to_owned())),
);
pub(super) const TYPE: ColumnDef = ColumnDef::record(
    "type",
    DataType::Str,
    "Kind of the directive, lower-case: 'transaction', 'open', 'close', 'balance', 'price', 'note', 'document', \
     'event', 'commodity', 'custom', 'query', 'budget', 'budget-add', 'budget-transfer' or 'budget-close'.",
    |_, record| of_directive(record, |it| Value::Str(entry_type(&it.data).to_owned())),
);
pub(super) const FILENAME: ColumnDef = ColumnDef::record("filename", DataType::Str, "The ledger file that holds the directive.", |_, record| {
    of_directive(record, |it| str_value(it.span.filename.as_ref().map(|path| path.to_string_lossy()).as_deref()))
});
pub(super) const DATE: ColumnDef = ColumnDef::record("date", DataType::Date, "Date of the directive; of a transaction, as zhang stores it.", date);
pub(super) const YEAR: ColumnDef = ColumnDef::record("year", DataType::Int, "Year of the date.", |data, record| date_part(data, record, year));
pub(super) const MONTH: ColumnDef = ColumnDef::record("month", DataType::Int, "Month (1-12) of the date.", |data, record| {
    date_part(data, record, |date| date.month())
});
pub(super) const DAY: ColumnDef = ColumnDef::record("day", DataType::Int, "Day of month of the date.", |data, record| {
    date_part(data, record, |date| date.day())
});
pub(super) const FLAG: ColumnDef = ColumnDef::record(
    "flag",
    DataType::Str,
    "Flag of a transaction: '*', '!', or 'P' for padding; NULL for other directives.",
    |_, record| of_transaction(record, txn_flag),
);
pub(super) const PAYEE: ColumnDef = ColumnDef::record("payee", DataType::Str, "Payee of a transaction; NULL for other directives.", |_, record| {
    of_transaction(record, payee)
});
pub(super) const NARRATION: ColumnDef = ColumnDef::record(
    "narration",
    DataType::Str,
    "Narration of a transaction ('' when absent); NULL for other directives.",
    |_, record| of_transaction(record, narration),
);
pub(super) const DESCRIPTION: ColumnDef = ColumnDef::record(
    "description",
    DataType::Str,
    "Payee and narration of a transaction joined with ' | '; NULL for other directives.",
    |_, record| of_transaction(record, txn_description),
);
pub(super) const TAGS: ColumnDef = ColumnDef::record(
    "tags",
    DataType::Set,
    "Tags of a transaction, note or document; NULL for other directives.",
    |_, record| of_directive(record, |it| tags_or_links(record, &it.data, false)),
);
pub(super) const LINKS: ColumnDef = ColumnDef::record(
    "links",
    DataType::Set,
    "Links of a transaction, note or document; NULL for other directives.",
    |_, record| of_directive(record, |it| tags_or_links(record, &it.data, true)),
);
pub(super) const META: ColumnDef = ColumnDef::record("meta", DataType::Str, "Metadata of the directive, as `key: \"value\"` pairs.", |_, record| {
    meta_value(record)
});
pub(super) const ACCOUNTS: ColumnDef = ColumnDef::record(
    "accounts",
    DataType::Set,
    "The accounts the directive refers to: the posting accounts of a transaction, the account of open, close, \
     balance, note and document, and the pad account of balance ... with pad; empty for other directives.",
    |_, record| of_directive(record, |it| entry_accounts(&it.data)),
);
pub(super) const SEQ: ColumnDef = ColumnDef::record(
    "seq",
    DataType::Int,
    "Position of the entry, from 0, in the order zhang processes the ledger; ORDER BY seq DESC lists the newest first. A \
     zhang extension.",
    |_, record| seq(record),
);
pub(super) const TIME: ColumnDef = ColumnDef::record(
    "time",
    DataType::Str,
    "Time of day of the directive in the ledger's timezone, as `HH:MM:SS`: the time written, or midnight without one, \
     moved past the gap on a day daylight saving skips it, as zhang stores it. A zhang extension.",
    time,
);
pub(super) const TIMESTAMP: ColumnDef = ColumnDef::record(
    "timestamp",
    DataType::Int,
    "Unix time, in seconds, of the directive's date and time. A zhang extension.",
    timestamp,
);
pub(super) const METAS: ColumnDef = ColumnDef::record(
    "metas",
    DataType::Metas,
    "Metadata of the directive as (key, value) pairs: sorted by key, every value of a repeated key in written order. A zhang \
     extension.",
    |_, record| metas_value(record),
);

static ENTRY_COLUMNS: &[ColumnDef] = &[
    ID,
    TYPE,
    FILENAME,
    DATE,
    YEAR,
    MONTH,
    DAY,
    FLAG,
    PAYEE,
    NARRATION,
    DESCRIPTION,
    TAGS,
    LINKS,
    META,
    ACCOUNTS,
    SEQ,
    TIME,
    TIMESTAMP,
    METAS,
];

static TRANSACTION_COLUMNS: &[ColumnDef] = &[
    DATE,
    FLAG,
    PAYEE,
    NARRATION,
    TAGS,
    LINKS,
    ACCOUNTS,
    META,
    ID,
    SEQ,
    TIME,
    TIMESTAMP,
    ColumnDef::record(
        "balanced",
        DataType::Bool,
        "FALSE when zhang recorded that the transaction does not balance (an UnbalancedTransaction error), else TRUE. A \
         zhang extension.",
        |_, record| entry_info(record).map_or(Value::Null, |info| balanced(info.errors.as_ref())),
    ),
    ColumnDef::record(
        "errors",
        DataType::Set,
        "Kinds of the errors zhang recorded for the transaction, as #errors names them in kind; empty when there are none. A \
         zhang extension.",
        |_, record| entry_info(record).map_or(Value::Null, |info| error_kinds(info.errors.as_ref())),
    ),
    METAS,
];

/// The `metas` column of an entry.
fn metas_value(record: &Record<'_>) -> Value {
    of_directive(record, |it| Value::Metas(meta_pairs(it.data.meta())))
}
