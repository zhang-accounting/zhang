//! The tables with one row per directive of a kind, as in beanquery: `#balances`, `#notes`,
//! `#events`, `#documents` and `#commodities`, plus the helpers the directive tables share.
//!
//! Rows come in ledger order, the order zhang processed the ledger in. The rows of `#balances`,
//! `#notes`, `#events` and `#commodities` are the `#entries` rows of their kind of directive
//! ([`directives_where`]), so these tables also have the columns of every directive as `#entries`
//! has them (`id`, `type`, `filename`, `year`, `month`, `day`, `time`, `timestamp`, `seq`, `metas`).
//! `#documents` adds, after its directives, the documents that transactions and postings name in
//! their metadata.

use std::borrow::Cow;
use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

use bigdecimal::BigDecimal;
use chrono::{Datelike, NaiveDate};
use uuid::Uuid;
use zhang_ast::amount::Amount;
use zhang_ast::{resolve_local_datetime, written_groups, Account, Directive, Meta, Posting, Spanned, Transaction};
use zhang_core::data_type::Dialect;
use zhang_core::ledger::Ledger;
use zhang_core::outcome::Detail;

use super::entries::{DATE, DAY, FILENAME, ID, LINKS, META, METAS, MONTH, SEQ, TAGS, TIME, TIMESTAMP, TYPE, YEAR};
use super::postings::time_value;
use super::{ledger_file, render_meta, ColumnDef, Dataset, LedgerCache, Record, Rows, Table};
use crate::projector::Projection;
use crate::value::{DataType, Value};

/// The date of a dated directive.
pub(super) fn date_of(directive: &Directive) -> Option<NaiveDate> {
    directive.datetime().map(|datetime| datetime.date())
}

/// One [`Record::Entry`] per `#entries` row whose directive `keep` selects, in the order zhang processed the ledger (the
/// `seq` order); `keep` only selects directives that are not transactions.
pub(super) fn directives_where<'a>(ledger: &'a Ledger, keep: impl Fn(&Directive) -> bool) -> Vec<Record<'a>> {
    let entries = LedgerCache::of(ledger).entries(ledger);
    entries
        .rows
        .iter()
        .filter(|info| keep(&ledger.directives[info.directive as usize].data))
        .map(|info| Record::Entry {
            directive: &ledger.directives[info.directive as usize],
            info,
        })
        .collect()
}

/// The directive of a [`Record::Entry`] row.
pub(super) fn directive<'r>(record: &'r Record<'_>) -> Option<&'r Spanned<Directive>> {
    match record {
        Record::Entry { directive, .. } => Some(directive),
        _ => None,
    }
}

pub(super) fn date_value(record: &Record<'_>) -> Value {
    directive(record).and_then(|it| date_of(&it.data)).map_or(Value::Null, Value::Date)
}

pub(super) fn str_value(value: Option<&str>) -> Value {
    value.map_or(Value::Null, |it| Value::Str(it.to_owned()))
}

pub(super) fn set_value<'s>(items: impl IntoIterator<Item = &'s String>) -> Value {
    Value::Set(items.into_iter().cloned().collect::<BTreeSet<_>>())
}

/// The `meta` column of a directive row.
pub(super) fn meta_value(record: &Record<'_>) -> Value {
    render_meta(directive(record).and_then(|it| it.data.meta()))
}

pub(super) fn year(date: NaiveDate) -> u32 {
    date.year() as u32
}

/// The `time` column of a directive: its time of day in the ledger's timezone, as zhang stores
/// the date and time of a directive.
pub(super) fn directive_time(data: &Dataset<'_>, directive: &Spanned<Directive>) -> Value {
    directive
        .data
        .datetime()
        .map_or(Value::Null, |it| time_value(resolve_local_datetime(&data.ledger.options.timezone, &it).time()))
}

/// The `timestamp` column of a directive: the Unix time of its date and time, read like
/// [`directive_time`].
pub(super) fn directive_timestamp(data: &Dataset<'_>, directive: &Spanned<Directive>) -> Value {
    directive.data.datetime().map_or(Value::Null, |it| {
        Value::Int(resolve_local_datetime(&data.ledger.options.timezone, &it).timestamp())
    })
}

// ---------------------------------------------------------------------------------------
// #balances

pub(super) static BALANCES: Table = Table {
    name: "balances",
    description: "One row per balance assertion (balance, and balance with pad), in ledger order, with the account's \
                  true balance at the assertion and whether the assertion holds.",
    columns: BALANCE_COLUMNS,
    wildcard: &["date", "account", "amount", "tolerance", "discrepancy"],
    rows: Rows::Records(|ledger, _| directives_where(ledger, |it| assertion(it).is_some())),
};

/// The account, asserted amount and tolerance of a balance assertion.
fn assertion(directive: &Directive) -> Option<(&Account, &Amount, Option<&BigDecimal>)> {
    match directive {
        Directive::BalanceCheck(check) => Some((&check.account, &check.amount, check.tolerance.as_ref())),
        Directive::BalancePad(pad) => Some((&pad.account, &pad.amount, None)),
        _ => None,
    }
}

fn balance_field(record: &Record<'_>, get: impl Fn(&Account, &Amount, Option<&BigDecimal>) -> Value) -> Value {
    directive(record)
        .and_then(|it| assertion(&it.data))
        .map_or(Value::Null, |(account, amount, tolerance)| get(account, amount, tolerance))
}

/// The asserted amount of an assertion row and what zhang's balance check of it found while loading the ledger
/// ([`Detail::Assertion`]): the balance of the asserted account and its sub-accounts in the asserted currency where the
/// assertion stands, and whether it holds (a failed one is an `AccountBalanceCheckError`).
fn balance_check<'r>(data: &'r Dataset<'_>, record: &'r Record<'_>) -> Option<(&'r Amount, &'r Amount, bool)> {
    let Record::Entry { directive, info } = record else {
        return None;
    };
    let Detail::Assertion { balance, passed, .. } = &data.ledger.outcomes[info.directive as usize].detail else {
        return None;
    };
    assertion(&directive.data).map(|(_, amount, _)| (amount, balance, *passed))
}

static BALANCE_COLUMNS: &[ColumnDef] = &[
    DATE,
    ColumnDef::record("account", DataType::Str, "The account whose balance is asserted.", |_, record| {
        balance_field(record, |account, _, _| Value::Str(account.name().to_owned()))
    }),
    ColumnDef::record(
        "amount",
        DataType::Amount,
        "The asserted balance of the account in one currency.",
        |_, record| balance_field(record, |_, amount, _| Value::Amount(amount.clone())),
    ),
    ColumnDef::record(
        "tolerance",
        DataType::Decimal,
        "The explicit tolerance of the assertion (`~ 0.01`); NULL when it has none.",
        |_, record| balance_field(record, |_, _, tolerance| tolerance.map_or(Value::Null, |it| Value::Decimal(it.clone()))),
    ),
    ColumnDef::record(
        "discrepancy",
        DataType::Amount,
        "When the assertion fails, the true balance minus the asserted amount (actual - amount); NULL when it holds.",
        |data, record| match balance_check(data, record) {
            Some((amount, balance, false)) => Value::Amount(Amount::new(&balance.number - &amount.number, amount.commodity.clone())),
            _ => Value::Null,
        },
    ),
    META,
    ColumnDef::record(
        "actual",
        DataType::Amount,
        "The account's true balance in the asserted currency at the assertion: the units of every earlier posting to the \
         account and its sub-accounts, as zhang checks it; a balance with pad is checked once the pads of its time are \
         booked. A zhang extension.",
        |data, record| balance_check(data, record).map_or(Value::Null, |(_, balance, _)| Value::Amount(balance.clone())),
    ),
    ColumnDef::record(
        "passed",
        DataType::Bool,
        "Whether the assertion holds, as zhang's balance check decided it (a failing one is an AccountBalanceCheckError): \
         actual is within the tolerance of the asserted amount, or equal to it without a tolerance. A zhang extension.",
        |data, record| balance_check(data, record).map_or(Value::Null, |(_, _, passed)| Value::Bool(passed)),
    ),
    ColumnDef::record(
        "pad",
        DataType::Str,
        "The account a `balance ... with pad` pads from; NULL for a balance without a pad. A zhang extension.",
        |_, record| match directive(record).map(|it| &it.data) {
            Some(Directive::BalancePad(pad)) => Value::Str(pad.pad.name().to_owned()),
            _ => Value::Null,
        },
    ),
    ID,
    SEQ,
    TIME,
    TIMESTAMP,
    TYPE,
    FILENAME,
    YEAR,
    MONTH,
    DAY,
    METAS,
];

// ---------------------------------------------------------------------------------------
// #notes

pub(super) static NOTES: Table = Table {
    name: "notes",
    description: "One row per note directive, in ledger order.",
    columns: NOTE_COLUMNS,
    wildcard: &["date", "account", "comment", "tags", "links"],
    rows: Rows::Records(|ledger, _| directives_where(ledger, |it| matches!(it, Directive::Note(_)))),
};

fn note<'r>(record: &'r Record<'_>) -> Option<&'r zhang_ast::Note> {
    match &directive(record)?.data {
        Directive::Note(note) => Some(note),
        _ => None,
    }
}

static NOTE_COLUMNS: &[ColumnDef] = &[
    DATE,
    ColumnDef::record("account", DataType::Str, "The account the note is about.", |_, record| {
        str_value(note(record).map(|it| it.account.name()))
    }),
    ColumnDef::record("comment", DataType::Str, "The text of the note.", |_, record| {
        str_value(note(record).map(|it| it.comment.as_str()))
    }),
    TAGS,
    LINKS,
    META,
    ID,
    TYPE,
    FILENAME,
    YEAR,
    MONTH,
    DAY,
    TIME,
    TIMESTAMP,
    SEQ,
    METAS,
];

// ---------------------------------------------------------------------------------------
// #events

pub(super) static EVENTS: Table = Table {
    name: "events",
    description: "One row per event directive, in ledger order.",
    columns: EVENT_COLUMNS,
    wildcard: &["date", "type", "description"],
    rows: Rows::Records(|ledger, _| directives_where(ledger, |it| matches!(it, Directive::Event(_)))),
};

fn event<'r>(record: &'r Record<'_>) -> Option<&'r zhang_ast::Event> {
    match &directive(record)?.data {
        Directive::Event(event) => Some(event),
        _ => None,
    }
}

static EVENT_COLUMNS: &[ColumnDef] = &[
    DATE,
    ColumnDef::record("type", DataType::Str, "The kind of event, e.g. 'location'.", |_, record| {
        str_value(event(record).map(|it| it.event_type.as_str()))
    }),
    ColumnDef::record("description", DataType::Str, "The value of the event, e.g. a city.", |_, record| {
        str_value(event(record).map(|it| it.description.as_str()))
    }),
    META,
    ID,
    FILENAME,
    YEAR,
    MONTH,
    DAY,
    TIME,
    TIMESTAMP,
    SEQ,
    METAS,
];

// ---------------------------------------------------------------------------------------
// #documents

pub(super) static DOCUMENTS: Table = Table {
    name: "documents",
    description: "One row per document directive, in ledger order, then one row per document a transaction or one of its \
                  postings names in its `document` metadata, in ledger order.",
    columns: DOCUMENT_COLUMNS,
    wildcard: &["date", "account", "filename", "tags", "links"],
    rows: Rows::Records(document_rows),
};

/// What holds a document of the `#documents` table.
#[derive(Clone, Copy)]
enum DocumentSource<'a> {
    /// a `document` directive
    Directive(&'a zhang_ast::Document),
    /// a `document` metadata value of a transaction
    Transaction(&'a Transaction),
    /// a `document` metadata value of a posting of a transaction
    Posting(&'a Transaction, &'a Posting),
}

/// A document: a row of the `#documents` table.
pub(crate) struct DocumentRow<'a> {
    /// the `document` directive, or the transaction whose metadata names the document
    directive: &'a Spanned<Directive>,
    source: DocumentSource<'a>,
    /// the path as written in the ledger
    filename: &'a str,
    /// the path relative to the ledger's directory
    path: Cow<'a, Path>,
    /// the id of the transaction the load accepted, for a document named in metadata
    transaction_id: Option<Uuid>,
    /// the position in `#entries` of the document directive, or of the transaction
    seq: u32,
}

impl<'a> DocumentRow<'a> {
    /// The metadata of the document directive, or of the transaction or posting that names
    /// the document.
    pub(super) fn metadata(&self) -> &'a Meta {
        match self.source {
            DocumentSource::Directive(document) => &document.meta,
            DocumentSource::Transaction(transaction) => &transaction.meta,
            DocumentSource::Posting(_, posting) => &posting.meta,
        }
    }
}

/// The document directives in ledger order, then the `document` metadata values of the
/// transactions the load accepted (those of `#transactions`), in ledger order: a transaction's own
/// first, then those of its postings as written, in order. A repeated key gives one row per value.
/// This is the one list of the ledger's documents.
///
/// The path of a document directive of a beancount ledger is the one zhang resolved it to while
/// loading the ledger ([`Detail::Document`]): relative to the file of the directive, as beancount
/// reads it, or relative to the ledger's root where only that names a file.
fn document_rows<'a>(ledger: &'a Ledger, _projection: Projection) -> Vec<Record<'a>> {
    let row = |directive, source, filename: &'a str, path: Cow<'a, Path>, transaction_id, seq| {
        Record::Document(DocumentRow {
            directive,
            source,
            filename,
            path,
            transaction_id,
            seq,
        })
    };
    let cache = LedgerCache::of(ledger);
    let entries = cache.entries(ledger);
    let mut rows = vec![];
    for entry in &entries.rows {
        let directive = &ledger.directives[entry.directive as usize];
        let Directive::Document(document) = &directive.data else {
            continue;
        };
        let path = match (&ledger.outcomes[entry.directive as usize].detail, ledger.dialect) {
            (Detail::Document { path, .. }, Dialect::Beancount) => Cow::Borrowed(Path::new(path.as_str())),
            _ => ledger_file(ledger, Path::new(document.filename.as_str())),
        };
        rows.push(row(
            directive,
            DocumentSource::Directive(document),
            document.filename.as_str(),
            path,
            None,
            entry.seq,
        ));
    }
    for entry in cache.documented(ledger).iter().map(|seq| &entries.rows[*seq as usize]) {
        let directive = &ledger.directives[entry.directive as usize];
        let Directive::Transaction(transaction) = &directive.data else {
            continue;
        };
        // the postings as written, not the booked legs: booking copies the metadata of a posting
        // it splits across lots onto every leg, and the posting names its documents once
        let holders = std::iter::once((DocumentSource::Transaction(transaction), &transaction.meta)).chain(
            written_groups(&transaction.postings)
                .into_iter()
                .map(|group| &group.legs[0])
                .map(|posting| (DocumentSource::Posting(transaction, posting), &posting.meta)),
        );
        for (source, meta) in holders {
            for filename in meta.get_all("document") {
                let path = ledger_file(ledger, Path::new(filename.as_str()));
                rows.push(row(directive, source, filename.as_str(), path, entry.txn, entry.seq));
            }
        }
    }
    rows
}

fn document<'r, 'a>(record: &'r Record<'a>) -> Option<&'r DocumentRow<'a>> {
    match record {
        Record::Document(document) => Some(document),
        _ => None,
    }
}

/// The path of a document, as beancount resolves it: a relative path is relative to the
/// directory of the file that declares it.
fn document_path(document: &DocumentRow<'_>) -> Value {
    let path = Path::new(document.filename);
    let resolved = match document.directive.span.filename.as_deref().and_then(Path::parent) {
        Some(dir) if path.is_relative() => normalize(&dir.join(path)),
        _ => path.to_path_buf(),
    };
    Value::Str(resolved.to_string_lossy().into_owned())
}

/// `path` with `.` and `..` components resolved lexically.
fn normalize(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir if matches!(normalized.components().next_back(), Some(Component::Normal(_))) => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

/// The tags or links of a document: those of its directive, or of the transaction that names it.
fn document_marks(record: &Record<'_>, of_directive: fn(&zhang_ast::Document) -> Value, of_transaction: fn(&Transaction) -> Value) -> Value {
    document(record).map_or(Value::Null, |document| match document.source {
        DocumentSource::Directive(directive) => of_directive(directive),
        DocumentSource::Transaction(transaction) | DocumentSource::Posting(transaction, _) => of_transaction(transaction),
    })
}

static DOCUMENT_COLUMNS: &[ColumnDef] = &[
    ColumnDef::record(
        "date",
        DataType::Date,
        "Date of the document directive, or of the transaction that names the document.",
        |_, record| document(record).and_then(|it| date_of(&it.directive.data)).map_or(Value::Null, Value::Date),
    ),
    ColumnDef::record(
        "account",
        DataType::Str,
        "The account the document belongs to: that of the directive, or of the posting that names it; NULL for a document of a transaction.",
        |_, record| {
            str_value(document(record).and_then(|it| match it.source {
                DocumentSource::Directive(directive) => Some(directive.account.name()),
                DocumentSource::Posting(_, posting) => Some(posting.account.name()),
                DocumentSource::Transaction(_) => None,
            }))
        },
    ),
    ColumnDef::record(
        "filename",
        DataType::Str,
        "Path of the document file; a relative path is resolved against the directory of the ledger file that declares it.",
        |_, record| document(record).map_or(Value::Null, document_path),
    ),
    ColumnDef::record(
        "tags",
        DataType::Set,
        "Tags of the document directive, or of the transaction that names the document.",
        |_, record| document_marks(record, |it| set_value(it.tags.iter().flatten()), |it| set_value(&it.tags)),
    ),
    ColumnDef::record(
        "links",
        DataType::Set,
        "Links of the document directive, or of the transaction that names the document.",
        |_, record| document_marks(record, |it| set_value(it.links.iter().flatten()), |it| set_value(&it.links)),
    ),
    ColumnDef::record(
        "meta",
        DataType::Str,
        "Metadata of the document directive, or of the transaction or posting that names the document, as `key: \"value\"` pairs.",
        |_, record| document(record).map_or(Value::Null, |it| render_meta(Some(it.metadata()))),
    ),
    ColumnDef::record(
        "source",
        DataType::Str,
        "What declares the document: 'directive' for a document directive, 'transaction' or 'posting' for the document \
         metadata of a transaction or of one of its postings. A zhang extension.",
        |_, record| {
            document(record).map_or(Value::Null, |it| {
                Value::Str(
                    match it.source {
                        DocumentSource::Directive(_) => "directive",
                        DocumentSource::Transaction(_) => "transaction",
                        DocumentSource::Posting(..) => "posting",
                    }
                    .to_owned(),
                )
            })
        },
    ),
    ColumnDef::record(
        "path",
        DataType::Str,
        "Path of the document within the ledger, the path the web UI downloads it with: as written, relative to the ledger's \
         directory; for a document directive of a beancount ledger, as zhang resolved it on load, relative to its file or to the \
         ledger's root. A zhang extension.",
        |_, record| document(record).map_or(Value::Null, |it| Value::Str(it.path.to_string_lossy().into_owned())),
    ),
    ColumnDef::record(
        "transaction_id",
        DataType::Str,
        "Id of the transaction whose metadata names the document, its id in the postings table; NULL for a document \
         directive. A zhang extension.",
        |_, record| {
            document(record)
                .and_then(|it| it.transaction_id)
                .map_or(Value::Null, |id| Value::Str(id.to_string()))
        },
    ),
    ColumnDef::record(
        "seq",
        DataType::Int,
        "seq of the document directive, or of the transaction that names the document, as in #entries. A zhang extension.",
        |_, record| document(record).map_or(Value::Null, |it| Value::Int(it.seq.into())),
    ),
    ColumnDef::record(
        "time",
        DataType::Str,
        "Time of day of the document directive, or of the transaction that names the document, in the ledger's timezone, \
         as `HH:MM:SS`: the time written, or midnight without one, moved past the gap on a day daylight saving skips it, \
         as zhang stores it. A zhang extension.",
        |data, record| document(record).map_or(Value::Null, |it| directive_time(data, it.directive)),
    ),
    ColumnDef::record(
        "timestamp",
        DataType::Int,
        "Unix time, in seconds, of the date and time of the document directive, or of the transaction that names the \
         document. A zhang extension.",
        |data, record| document(record).map_or(Value::Null, |it| directive_timestamp(data, it.directive)),
    ),
];

// ---------------------------------------------------------------------------------------
// #commodities

pub(super) static COMMODITIES: Table = Table {
    name: "commodities",
    description: "One row per commodity directive, in ledger order.",
    columns: COMMODITY_COLUMNS,
    wildcard: &["meta", "date", "name"],
    rows: Rows::Records(|ledger, _| directives_where(ledger, |it| matches!(it, Directive::Commodity(_)))),
};

fn commodity<'r>(record: &'r Record<'_>) -> Option<&'r zhang_ast::Commodity> {
    match &directive(record)?.data {
        Directive::Commodity(commodity) => Some(commodity),
        _ => None,
    }
}

static COMMODITY_COLUMNS: &[ColumnDef] = &[
    META,
    DATE,
    ColumnDef::record("name", DataType::Str, "The currency or commodity, e.g. 'USD'.", |_, record| {
        str_value(commodity(record).map(|it| it.currency.as_str()))
    }),
    ID,
    TYPE,
    FILENAME,
    YEAR,
    MONTH,
    DAY,
    TIME,
    TIMESTAMP,
    SEQ,
    METAS,
];

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::normalize;

    #[test]
    fn paths_are_normalized_lexically() {
        assert_eq!(
            normalize(Path::new("/ledger/./docs/../statements/a.pdf")),
            Path::new("/ledger/statements/a.pdf")
        );
        assert_eq!(normalize(Path::new("/ledger/a.pdf")), Path::new("/ledger/a.pdf"));
        assert_eq!(normalize(Path::new("../a.pdf")), Path::new("../a.pdf"));
    }
}
