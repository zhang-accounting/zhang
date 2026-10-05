//! `#entries` (one row per directive) and `#transactions` (one row per transaction), as in
//! beanquery.
//!
//! A balance assertion is a `balance` entry, not a transaction: it books nothing. Transactions
//! the ledger rejected are not rows either (they never reach the store). The padding
//! transactions of `balance ... with pad` (flag `P`) are transactions, as in beancount.
//!
//! Both tables read their rows from the cache of the ledger ([`super::cache::Entries`]), which
//! also gives zhang's own columns: `seq` (the position in `#entries`), and on `#transactions`
//! the stored `id` and the errors recorded for the transaction (`balanced`, `errors`).

use chrono::{Datelike, NaiveDate};
use zhang_ast::{Directive, Spanned};
use zhang_core::ledger::Ledger;
use zhang_core::store::{Store, TransactionDomain};

use super::cache::{EntryInfo, LedgerCache};
use super::directives::{date_value, directive, directive_time, directive_timestamp, meta_value, set_value, str_value, year};
use super::postings::{balanced, error_kinds, txn_date, txn_description, txn_flag, txn_narration, txn_payee, txn_time, txn_timestamp};
use super::{directive_meta, meta_pairs, ColumnDef, Dataset, Record, Rows, Table};
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
fn entry_rows<'a>(ledger: &'a Ledger, store: &'a Store, _projection: Projection) -> Vec<Record<'a>> {
    let table = LedgerCache::of(ledger, store).entries(ledger, store);
    table.rows.iter().map(|info| entry_record(ledger, store, info)).collect()
}

/// The transaction entries, in the order of the ledger's directives.
fn transaction_rows<'a>(ledger: &'a Ledger, store: &'a Store, _projection: Projection) -> Vec<Record<'a>> {
    let table = LedgerCache::of(ledger, store).entries(ledger, store);
    table
        .transactions
        .iter()
        .map(|idx| entry_record(ledger, store, &table.rows[*idx as usize]))
        .collect()
}

fn entry_record<'a>(ledger: &'a Ledger, store: &'a Store, info: &'a EntryInfo) -> Record<'a> {
    Record::Entry {
        directive: &ledger.directives[info.directive as usize],
        info,
        txn: info.txn.and_then(|id| store.transactions.get(&id)),
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

/// The time of day of the row's directive in the ledger's timezone, as zhang stores the date and
/// time of a transaction; for a transaction, the time it is stored at.
pub(super) fn time(data: &Dataset<'_>, record: &Record<'_>) -> Value {
    match stored(record) {
        Some(txn) => txn_time(txn),
        None => directive(record).map_or(Value::Null, |it| directive_time(data, it)),
    }
}

/// The Unix time of the row's directive, read like [`time`].
pub(super) fn timestamp(data: &Dataset<'_>, record: &Record<'_>) -> Value {
    match stored(record) {
        Some(txn) => txn_timestamp(txn),
        None => directive(record).map_or(Value::Null, |it| directive_timestamp(data, it)),
    }
}

/// What zhang stored of the transaction of an `#entries` or `#transactions` row, which its
/// columns read, as the postings' do.
fn stored<'r>(record: &'r Record<'_>) -> Option<&'r TransactionDomain> {
    match record {
        Record::Entry { txn, .. } => *txn,
        _ => None,
    }
}

/// The date of the row: a transaction's as zhang stores it, another directive's as written.
fn date(record: &Record<'_>) -> Value {
    match stored(record) {
        Some(txn) => Value::Date(txn_date(txn)),
        None => date_value(record),
    }
}

/// The `year`, `month` or `day` of the row's [`date`].
fn date_part(record: &Record<'_>, part: fn(NaiveDate) -> u32) -> Value {
    match date(record) {
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

fn payee(txn: &TransactionDomain) -> Value {
    str_value(txn_payee(txn))
}

fn narration(txn: &TransactionDomain) -> Value {
    Value::Str(txn_narration(txn).to_owned())
}

/// The tags or links of a transaction, note or document; NULL for other directives. A
/// transaction's are those zhang stored.
fn tags_or_links(record: &Record<'_>, directive: &Directive, links: bool) -> Value {
    match directive {
        Directive::Transaction(_) => stored(record).map_or(Value::Null, |txn| set_value(if links { &txn.links } else { &txn.tags })),
        Directive::Note(note) => set_value((if links { &note.links } else { &note.tags }).iter().flatten()),
        Directive::Document(document) => set_value((if links { &document.links } else { &document.tags }).iter().flatten()),
        _ => Value::Null,
    }
}

fn of_transaction(record: &Record<'_>, get: fn(&TransactionDomain) -> Value) -> Value {
    stored(record).map_or(Value::Null, get)
}

fn of_directive(record: &Record<'_>, get: impl Fn(&Spanned<Directive>) -> Value) -> Value {
    directive(record).map_or(Value::Null, get)
}

static ENTRY_COLUMNS: &[ColumnDef] = &[
    ColumnDef::record(
        "id",
        DataType::Str,
        "Unique id of the entry: for a transaction its transaction id (the id column of its postings), for a balance \
         assertion the id zhang stored its check with.",
        |data, record| entry_info(record).map_or(Value::Null, |info| Value::Str(data.entry_id(info.seq).to_owned())),
    ),
    ColumnDef::record(
        "type",
        DataType::Str,
        "Kind of the directive, lower-case: 'transaction', 'open', 'close', 'balance', 'price', 'note', 'document', \
         'event', 'commodity', 'custom', 'query', 'budget', 'budget-add', 'budget-transfer' or 'budget-close'.",
        |_, record| of_directive(record, |it| Value::Str(entry_type(&it.data).to_owned())),
    ),
    ColumnDef::record("filename", DataType::Str, "The ledger file that holds the directive.", |_, record| {
        of_directive(record, |it| str_value(it.span.filename.as_ref().map(|path| path.to_string_lossy()).as_deref()))
    }),
    ColumnDef::record("date", DataType::Date, "Date of the directive; of a transaction, as zhang stores it.", |_, record| {
        date(record)
    }),
    ColumnDef::record("year", DataType::Int, "Year of the date.", |_, record| date_part(record, year)),
    ColumnDef::record("month", DataType::Int, "Month (1-12) of the date.", |_, record| {
        date_part(record, |date| date.month())
    }),
    ColumnDef::record("day", DataType::Int, "Day of month of the date.", |_, record| {
        date_part(record, |date| date.day())
    }),
    ColumnDef::record("flag", DataType::Str, "Flag of a transaction; NULL for other directives.", |_, record| {
        of_transaction(record, txn_flag)
    }),
    ColumnDef::record("payee", DataType::Str, "Payee of a transaction; NULL for other directives.", |_, record| {
        of_transaction(record, payee)
    }),
    ColumnDef::record(
        "narration",
        DataType::Str,
        "Narration of a transaction ('' when absent); NULL for other directives.",
        |_, record| of_transaction(record, narration),
    ),
    ColumnDef::record(
        "description",
        DataType::Str,
        "Payee and narration of a transaction joined with ' | '; NULL for other directives.",
        |_, record| of_transaction(record, txn_description),
    ),
    ColumnDef::record(
        "tags",
        DataType::Set,
        "Tags of a transaction, note or document; NULL for other directives.",
        |_, record| of_directive(record, |it| tags_or_links(record, &it.data, false)),
    ),
    ColumnDef::record(
        "links",
        DataType::Set,
        "Links of a transaction, note or document; NULL for other directives.",
        |_, record| of_directive(record, |it| tags_or_links(record, &it.data, true)),
    ),
    ColumnDef::record("meta", DataType::Str, "Metadata of the directive, as `key: \"value\"` pairs.", |_, record| {
        meta_value(record)
    }),
    ColumnDef::record(
        "accounts",
        DataType::Set,
        "The accounts the directive refers to: the posting accounts of a transaction, the account of open, close, \
         balance, note and document, and the pad account of balance ... with pad; empty for other directives.",
        |_, record| of_directive(record, |it| entry_accounts(&it.data)),
    ),
    ColumnDef::record(
        "seq",
        DataType::Int,
        "Position of the entry, from 0, in the order zhang processes the ledger; ORDER BY seq DESC lists the newest first. A \
         zhang extension.",
        |_, record| seq(record),
    ),
    ColumnDef::record(
        "time",
        DataType::Str,
        "Time of day of the directive in the ledger's timezone, as `HH:MM:SS`: the time written, or midnight without one, moved past the gap on a day daylight saving skips it, as zhang stores it. A zhang \
         extension.",
        time,
    ),
    ColumnDef::record(
        "timestamp",
        DataType::Int,
        "Unix time, in seconds, of the directive's date and time. A zhang extension.",
        timestamp,
    ),
    ColumnDef::record(
        "metas",
        DataType::Metas,
        "Metadata of the directive as (key, value) pairs: sorted by key, every value of a repeated key in written order. A zhang \
         extension.",
        |_, record| metas_value(record),
    ),
];

static TRANSACTION_COLUMNS: &[ColumnDef] = &[
    ColumnDef::record("date", DataType::Date, "Date of the transaction, as zhang stores it.", |_, record| {
        date(record)
    }),
    ColumnDef::record("flag", DataType::Str, "Flag of the transaction: '*', '!', or 'P' for padding.", |_, record| {
        of_transaction(record, txn_flag)
    }),
    ColumnDef::record("payee", DataType::Str, "Payee of the transaction.", |_, record| of_transaction(record, payee)),
    ColumnDef::record("narration", DataType::Str, "Narration of the transaction; '' when absent.", |_, record| {
        of_transaction(record, narration)
    }),
    ColumnDef::record("tags", DataType::Set, "Tags of the transaction.", |_, record| {
        of_transaction(record, |txn| set_value(&txn.tags))
    }),
    ColumnDef::record("links", DataType::Set, "Links of the transaction.", |_, record| {
        of_transaction(record, |txn| set_value(&txn.links))
    }),
    ColumnDef::record("accounts", DataType::Set, "The accounts of the postings of the transaction.", |_, record| {
        of_directive(record, |it| entry_accounts(&it.data))
    }),
    ColumnDef::record("meta", DataType::Str, "Metadata of the transaction, as `key: \"value\"` pairs.", |_, record| {
        meta_value(record)
    }),
    ColumnDef::record(
        "id",
        DataType::Str,
        "Unique id of the transaction: the id column of its postings and of its #entries row. A zhang extension.",
        |data, record| entry_info(record).map_or(Value::Null, |info| Value::Str(data.entry_id(info.seq).to_owned())),
    ),
    ColumnDef::record(
        "seq",
        DataType::Int,
        "Position of the transaction, from 0, in the order zhang processes the ledger, as in #entries; ORDER BY seq DESC \
         lists the newest first. A zhang extension.",
        |_, record| seq(record),
    ),
    ColumnDef::record(
        "time",
        DataType::Str,
        "Time of day of the transaction in the ledger's timezone, as `HH:MM:SS`: the time written, or midnight without one, moved past the gap on a day daylight saving skips it, as zhang stores it. A zhang \
         extension.",
        time,
    ),
    ColumnDef::record(
        "timestamp",
        DataType::Int,
        "Unix time, in seconds, of the transaction's date and time. A zhang extension.",
        timestamp,
    ),
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
    ColumnDef::record(
        "metas",
        DataType::Metas,
        "Metadata of the transaction as (key, value) pairs: sorted by key, every value of a repeated key in written order. A zhang \
         extension.",
        |_, record| metas_value(record),
    ),
];

/// The `metas` column of an entry.
fn metas_value(record: &Record<'_>) -> Value {
    of_directive(record, |it| Value::Metas(meta_pairs(directive_meta(&it.data))))
}
