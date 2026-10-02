//! `#entries` (one row per directive) and `#transactions` (one row per transaction), as in
//! beanquery.
//!
//! zhang materializes every balance assertion as a transaction with flag `C`; those are not
//! transactions here (the assertion is a `balance` entry), and neither are transactions the
//! ledger rejected (they never reach the store). The padding transactions of `balance ... with
//! pad` (flag `P`) are, as in beancount.

use chrono::Datelike;
use uuid::Uuid;
use zhang_ast::{Directive, Flag, Spanned, Transaction};
use zhang_core::ledger::Ledger;
use zhang_core::store::Store;
use zhang_core::utils::id::FromSpan;

use super::directives::{date_part, date_value, directive, ledger_order, meta_value, set_value, str_value, year};
use super::{ColumnDef, Record, Rows, Table};
use crate::projector::Projection;
use crate::value::{DataType, Value};

pub(super) static ENTRIES: Table = Table {
    name: "entries",
    description: "One row per directive of the ledger (transactions, open, close, balance, price, note, document, event, \
                  commodity, custom, query and budget directives), sorted by date as beancount sorts entries.",
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

/// Whether a transaction directive is one of the ledger's transactions: stored, and not the
/// correcting transaction of a balance assertion.
fn is_transaction(store: &Store, directive: &Spanned<Directive>) -> bool {
    match &directive.data {
        Directive::Transaction(txn) => txn.flag != Some(Flag::BalanceCheck) && store.transactions.contains_key(&Uuid::from_span(&directive.span)),
        _ => false,
    }
}

fn entry_rows<'a>(ledger: &'a Ledger, store: &'a Store, _projection: Projection) -> Vec<Record<'a>> {
    ledger_order(ledger)
        .into_iter()
        .filter(|directive| !matches!(directive.data, Directive::Transaction(_)) || is_transaction(store, directive))
        .map(Record::Directive)
        .collect()
}

fn transaction_rows<'a>(ledger: &'a Ledger, store: &'a Store, _projection: Projection) -> Vec<Record<'a>> {
    ledger
        .directives
        .iter()
        .filter(|directive| is_transaction(store, directive))
        .map(Record::Directive)
        .collect()
}

fn transaction<'r>(record: &'r Record<'_>) -> Option<&'r Transaction> {
    match &directive(record)?.data {
        Directive::Transaction(txn) => Some(txn),
        _ => None,
    }
}

/// The type of an entry: beancount's directive name, or zhang's keyword for the directives
/// beancount does not have. A `balance ... with pad` is a `balance`.
fn entry_type(directive: &Directive) -> &'static str {
    match directive {
        Directive::Open(_) => "open",
        Directive::Close(_) => "close",
        Directive::Commodity(_) => "commodity",
        Directive::Transaction(_) => "transaction",
        Directive::BalancePad(_) | Directive::BalanceCheck(_) => "balance",
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
        Directive::Note(it) => vec![it.account.name()],
        Directive::Document(it) => vec![it.account.name()],
        _ => vec![],
    };
    Value::Set(accounts.into_iter().map(str::to_owned).collect())
}

/// The id of an entry: a transaction has its transaction id (the `id` of its postings);
/// another directive an id derived from its source position, distinct from the id of the
/// padding transaction that shares the position of a `balance ... with pad`. The derivation
/// does not depend on the platform's `usize`.
fn entry_id(directive: &Spanned<Directive>) -> String {
    let id = Uuid::from_span(&directive.span);
    match directive.data {
        Directive::Transaction(_) => id.to_string(),
        _ => Uuid::from_txn_posting(&id, u32::MAX as usize).to_string(),
    }
}

fn flag(txn: &Transaction) -> Value {
    Value::Str(txn.flag.clone().unwrap_or(Flag::Okay).to_string())
}

fn payee(txn: &Transaction) -> Value {
    str_value(txn.payee.as_ref().map(|it| it.as_str()))
}

/// '' when absent, as in beancount
fn narration(txn: &Transaction) -> Value {
    Value::Str(txn.narration.as_ref().map(|it| it.as_str()).unwrap_or_default().to_owned())
}

fn description(txn: &Transaction) -> Value {
    let parts = [txn.payee.as_ref(), txn.narration.as_ref()]
        .into_iter()
        .flatten()
        .map(|it| it.as_str())
        .filter(|it| !it.is_empty())
        .collect::<Vec<_>>();
    Value::Str(parts.join(" | "))
}

/// The tags or links of a transaction, note or document; NULL for other directives.
fn tags_or_links(directive: &Directive, links: bool) -> Value {
    match directive {
        Directive::Transaction(txn) => set_value(if links { &txn.links } else { &txn.tags }),
        Directive::Note(note) => set_value((if links { &note.links } else { &note.tags }).iter().flatten()),
        Directive::Document(document) => set_value((if links { &document.links } else { &document.tags }).iter().flatten()),
        _ => Value::Null,
    }
}

fn of_transaction(record: &Record<'_>, get: fn(&Transaction) -> Value) -> Value {
    transaction(record).map_or(Value::Null, get)
}

fn of_directive(record: &Record<'_>, get: impl Fn(&Spanned<Directive>) -> Value) -> Value {
    directive(record).map_or(Value::Null, get)
}

static ENTRY_COLUMNS: &[ColumnDef] = &[
    ColumnDef::record(
        "id",
        DataType::Str,
        "Unique id of the entry: for a transaction its transaction id (the id column of its postings).",
        |_, record| of_directive(record, |it| Value::Str(entry_id(it))),
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
    ColumnDef::record("date", DataType::Date, "Date of the directive.", |_, record| date_value(record)),
    ColumnDef::record("year", DataType::Int, "Year of the date.", |_, record| date_part(record, year)),
    ColumnDef::record("month", DataType::Int, "Month (1-12) of the date.", |_, record| {
        date_part(record, |date| date.month())
    }),
    ColumnDef::record("day", DataType::Int, "Day of month of the date.", |_, record| {
        date_part(record, |date| date.day())
    }),
    ColumnDef::record("flag", DataType::Str, "Flag of a transaction; NULL for other directives.", |_, record| {
        of_transaction(record, flag)
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
        |_, record| of_transaction(record, description),
    ),
    ColumnDef::record(
        "tags",
        DataType::Set,
        "Tags of a transaction, note or document; NULL for other directives.",
        |_, record| of_directive(record, |it| tags_or_links(&it.data, false)),
    ),
    ColumnDef::record(
        "links",
        DataType::Set,
        "Links of a transaction, note or document; NULL for other directives.",
        |_, record| of_directive(record, |it| tags_or_links(&it.data, true)),
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
];

static TRANSACTION_COLUMNS: &[ColumnDef] = &[
    ColumnDef::record("date", DataType::Date, "Date of the transaction.", |_, record| date_value(record)),
    ColumnDef::record("flag", DataType::Str, "Flag of the transaction: '*', '!', or 'P' for padding.", |_, record| {
        of_transaction(record, flag)
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
];
