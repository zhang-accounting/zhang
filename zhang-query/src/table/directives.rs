//! The tables with one row per directive of a kind, as in beanquery: `#balances`, `#notes`,
//! `#events`, `#documents` and `#commodities`, plus the helpers the directive tables share.
//!
//! Rows come in ledger order: by date, then as beancount orders the directives of a day
//! ([`ledger_order`]). `#documents` adds, after its directives, the documents that
//! transactions and postings name in their metadata.

use std::borrow::Cow;
use std::collections::{BTreeSet, HashMap, VecDeque};
use std::path::{Component, Path, PathBuf};

use bigdecimal::BigDecimal;
use chrono::{Datelike, NaiveDate};
use uuid::Uuid;
use zhang_ast::amount::Amount;
use zhang_ast::{resolve_local_datetime, Account, Directive, Meta, Posting, Spanned, Transaction};
use zhang_core::data_type::is_beancount_endpoint;
use zhang_core::ledger::Ledger;
use zhang_core::store::{BalanceAssertionDomain, DocumentType, Store};

use super::postings::time_value;
use super::{directive_meta, ledger_file, render_meta, ColumnDef, Dataset, LedgerCache, Record, Rows, Table};
use crate::projector::Projection;
use crate::value::{DataType, Value};

/// The date of a dated directive.
pub(super) fn date_of(directive: &Directive) -> Option<NaiveDate> {
    directive.datetime().map(|datetime| datetime.date())
}

/// Where beancount sorts a directive among the directives of its day: `open` first, then
/// balance assertions, the other kinds, `document` and `close` last.
pub(super) fn day_rank(directive: &Directive) -> i8 {
    match directive {
        Directive::Open(_) => -2,
        Directive::BalanceCheck(_) | Directive::BalancePad(_) => -1,
        Directive::Document(_) => 1,
        Directive::Close(_) => 2,
        _ => 0,
    }
}

/// The dated directives of the ledger in beancount's order: by date, then by [`day_rank`],
/// then in ledger order (zhang's order of the day, which follows the source for directives
/// without a time). These are the rows of `#entries`, in the order the cache of the ledger keeps
/// them ([`super::cache::Entries`]), so every directive is there except the transactions that
/// are no entries: those zhang rejected.
pub(super) fn ledger_order<'a>(ledger: &'a Ledger, store: &Store) -> impl Iterator<Item = &'a Spanned<Directive>> {
    let entries = LedgerCache::of(ledger, store).entries(ledger, store);
    entries.rows.iter().map(|entry| &ledger.directives[entry.directive as usize])
}

/// One [`Record::Directive`] per directive that `keep` selects, in [`ledger_order`]; `keep`
/// only selects directives that are not transactions.
pub(super) fn directives_where<'a>(ledger: &'a Ledger, store: &Store, keep: impl Fn(&Directive) -> bool) -> Vec<Record<'a>> {
    ledger_order(ledger, store)
        .filter(|directive| keep(&directive.data))
        .map(Record::Directive)
        .collect()
}

/// The directive of a [`Record::Directive`] or [`Record::Balance`] row.
pub(super) fn directive<'r>(record: &'r Record<'_>) -> Option<&'r Spanned<Directive>> {
    match record {
        Record::Directive(directive) | Record::Balance { directive, .. } | Record::Entry { directive, .. } => Some(directive),
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
    render_meta(directive(record).and_then(|it| directive_meta(&it.data)))
}

/// The `year`, `month` or `day` of the row's date.
pub(super) fn date_part(record: &Record<'_>, part: fn(NaiveDate) -> u32) -> Value {
    directive(record)
        .and_then(|it| date_of(&it.data))
        .map_or(Value::Null, |date| Value::Int(part(date) as i64))
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
    rows: Rows::Records(balance_rows),
};

/// The account, asserted amount and tolerance of a balance assertion.
fn assertion(directive: &Directive) -> Option<(&Account, &Amount, Option<&BigDecimal>)> {
    match directive {
        Directive::BalanceCheck(check) => Some((&check.account, &check.amount, check.tolerance.as_ref())),
        Directive::BalancePad(pad) => Some((&pad.account, &pad.amount, None)),
        _ => None,
    }
}

/// The balance assertions. Only when the projection reads `actual`, `passed` or
/// `discrepancy` are the checks zhang made looked up ([`AssertionCheck`]).
fn balance_rows<'a>(ledger: &'a Ledger, store: &'a Store, projection: Projection) -> Vec<Record<'a>> {
    let wanted = ["actual", "passed", "discrepancy"]
        .into_iter()
        .any(|name| BALANCES.column(name).is_some_and(|column| projection.contains(column)));
    let entries = LedgerCache::of(ledger, store).entries(ledger, store);
    entries
        .rows
        .iter()
        .map(|entry| (&ledger.directives[entry.directive as usize], entry))
        .filter(|(directive, _)| assertion(&directive.data).is_some())
        .map(|(directive, entry)| Record::Balance {
            directive,
            seq: entry.seq,
            check: entry
                .assertion
                .filter(|_| wanted)
                .map(|assertion| AssertionCheck::of(&store.balance_assertions[assertion as usize])),
        })
        .collect()
}

/// What zhang's balance check found for an assertion while loading the ledger.
pub(crate) struct AssertionCheck {
    /// the account's balance in the asserted currency where the assertion stands
    actual: Amount,
    /// whether the assertion holds; a failed one is an `AccountBalanceCheckError`
    passed: bool,
}

impl AssertionCheck {
    /// The check zhang made of an assertion while loading the ledger (`Store::balance_assertions`),
    /// which the cache matched with its directive: the balance of the asserted account and its
    /// sub-accounts in the asserted currency, summed from the postings before the assertion (for a
    /// `balance ... with pad`, once the balance entries of its time are booked), and whether it is
    /// within the tolerance of the asserted amount. The same check reports an
    /// `AccountBalanceCheckError` when it fails. Assertions never move a balance.
    fn of(check: &BalanceAssertionDomain) -> AssertionCheck {
        AssertionCheck {
            actual: check.balance.clone(),
            passed: check.passed,
        }
    }
}

fn balance_field(record: &Record<'_>, get: impl Fn(&Account, &Amount, Option<&BigDecimal>) -> Value) -> Value {
    directive(record)
        .and_then(|it| assertion(&it.data))
        .map_or(Value::Null, |(account, amount, tolerance)| get(account, amount, tolerance))
}

/// The asserted amount of an assertion row and what zhang's check of it found, if looked up.
fn balance_check<'r>(record: &'r Record<'_>) -> Option<(&'r Amount, &'r AssertionCheck)> {
    let Record::Balance {
        directive, check: Some(check), ..
    } = record
    else {
        return None;
    };
    assertion(&directive.data).map(|(_, amount, _)| (amount, check))
}

static BALANCE_COLUMNS: &[ColumnDef] = &[
    ColumnDef::record("date", DataType::Date, "Date of the assertion.", |_, record| date_value(record)),
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
        |_, record| match balance_check(record) {
            Some((amount, check)) if !check.passed => Value::Amount(Amount::new(&check.actual.number - &amount.number, amount.commodity.clone())),
            _ => Value::Null,
        },
    ),
    ColumnDef::record("meta", DataType::Str, "Metadata of the assertion, as `key: \"value\"` pairs.", |_, record| {
        meta_value(record)
    }),
    ColumnDef::record(
        "actual",
        DataType::Amount,
        "The account's true balance in the asserted currency at the assertion: the units of every earlier posting to the \
         account and its sub-accounts, as zhang checks it; a balance with pad is checked once the pads of its time are \
         booked. A zhang extension.",
        |_, record| balance_check(record).map_or(Value::Null, |(_, check)| Value::Amount(check.actual.clone())),
    ),
    ColumnDef::record(
        "passed",
        DataType::Bool,
        "Whether the assertion holds, as zhang's balance check decided it (a failing one is an AccountBalanceCheckError): \
         actual is within the tolerance of the asserted amount, or equal to it without a tolerance. A zhang extension.",
        |_, record| balance_check(record).map_or(Value::Null, |(_, check)| Value::Bool(check.passed)),
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
    ColumnDef::record(
        "id",
        DataType::Str,
        "The id zhang stored the check of the assertion with, which /api/journals lists it with: the id of its row in \
         #entries. A zhang extension.",
        |data, record| match record {
            Record::Balance { seq, .. } => Value::Str(data.entry_id(*seq).to_owned()),
            _ => Value::Null,
        },
    ),
    ColumnDef::record(
        "seq",
        DataType::Int,
        "Position of the assertion in the order zhang processes the ledger, as seq in #entries: where zhang checks it, so \
         it comes right after the postings its actual balance includes. A zhang extension.",
        |data, record| match record {
            Record::Balance { seq, .. } => Value::Int(data.entry_order(*seq).into()),
            _ => Value::Null,
        },
    ),
    ColumnDef::record(
        "time",
        DataType::Str,
        "Time of day of the assertion in the ledger's timezone, as `HH:MM:SS`: the time written, or midnight without one, moved past the gap on a day daylight saving skips it, as zhang stores it. A zhang \
         extension.",
        |data, record| directive(record).map_or(Value::Null, |it| directive_time(data, it)),
    ),
    ColumnDef::record(
        "timestamp",
        DataType::Int,
        "Unix time, in seconds, of the assertion's date and time. A zhang extension.",
        |data, record| directive(record).map_or(Value::Null, |it| directive_timestamp(data, it)),
    ),
];

// ---------------------------------------------------------------------------------------
// #notes

pub(super) static NOTES: Table = Table {
    name: "notes",
    description: "One row per note directive, in ledger order.",
    columns: NOTE_COLUMNS,
    wildcard: &["date", "account", "comment", "tags", "links"],
    rows: Rows::Records(|ledger, store, _| directives_where(ledger, store, |it| matches!(it, Directive::Note(_)))),
};

fn note<'r>(record: &'r Record<'_>) -> Option<&'r zhang_ast::Note> {
    match &directive(record)?.data {
        Directive::Note(note) => Some(note),
        _ => None,
    }
}

static NOTE_COLUMNS: &[ColumnDef] = &[
    ColumnDef::record("date", DataType::Date, "Date of the note.", |_, record| date_value(record)),
    ColumnDef::record("account", DataType::Str, "The account the note is about.", |_, record| {
        str_value(note(record).map(|it| it.account.name()))
    }),
    ColumnDef::record("comment", DataType::Str, "The text of the note.", |_, record| {
        str_value(note(record).map(|it| it.comment.as_str()))
    }),
    ColumnDef::record("tags", DataType::Set, "Tags of the note.", |_, record| {
        note(record).map_or(Value::Null, |it| set_value(it.tags.iter().flatten()))
    }),
    ColumnDef::record("links", DataType::Set, "Links of the note.", |_, record| {
        note(record).map_or(Value::Null, |it| set_value(it.links.iter().flatten()))
    }),
    ColumnDef::record("meta", DataType::Str, "Metadata of the note, as `key: \"value\"` pairs.", |_, record| {
        meta_value(record)
    }),
];

// ---------------------------------------------------------------------------------------
// #events

pub(super) static EVENTS: Table = Table {
    name: "events",
    description: "One row per event directive, in ledger order.",
    columns: EVENT_COLUMNS,
    wildcard: &["date", "type", "description"],
    rows: Rows::Records(|ledger, store, _| directives_where(ledger, store, |it| matches!(it, Directive::Event(_)))),
};

fn event<'r>(record: &'r Record<'_>) -> Option<&'r zhang_ast::Event> {
    match &directive(record)?.data {
        Directive::Event(event) => Some(event),
        _ => None,
    }
}

static EVENT_COLUMNS: &[ColumnDef] = &[
    ColumnDef::record("date", DataType::Date, "Date of the event.", |_, record| date_value(record)),
    ColumnDef::record("type", DataType::Str, "The kind of event, e.g. 'location'.", |_, record| {
        str_value(event(record).map(|it| it.event_type.as_str()))
    }),
    ColumnDef::record("description", DataType::Str, "The value of the event, e.g. a city.", |_, record| {
        str_value(event(record).map(|it| it.description.as_str()))
    }),
    ColumnDef::record("meta", DataType::Str, "Metadata of the event, as `key: \"value\"` pairs.", |_, record| {
        meta_value(record)
    }),
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
    /// the id of the transaction the store keeps, for a document named in metadata
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
/// transactions the store keeps (those of `#transactions`), in ledger order: a transaction's own
/// first, then those of its postings in order. A repeated key gives one row per value.
///
/// The path of a document directive of a beancount ledger is the one zhang resolved it to while
/// loading the ledger (relative to the file of the directive, as beancount reads it, or relative to
/// the ledger's root where only that names a file): the store keeps it with the directive's account
/// and date, in the order it read the directives, which is the order of the rows of one account and
/// day too.
fn document_rows<'a>(ledger: &'a Ledger, store: &'a Store, _projection: Projection) -> Vec<Record<'a>> {
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
    let mut resolved: HashMap<(&str, NaiveDate), VecDeque<&'a str>> = HashMap::new();
    if is_beancount_endpoint(&ledger.entry.1) {
        for document in &store.documents {
            if let DocumentType::Account(account) = &document.document_type {
                resolved
                    .entry((account.name(), document.datetime.date_naive()))
                    .or_default()
                    .push_back(document.path.as_str());
            }
        }
    }
    let cache = LedgerCache::of(ledger, store);
    let entries = cache.entries(ledger, store);
    let mut directives = entries
        .rows
        .iter()
        .filter_map(|entry| {
            let directive = &ledger.directives[entry.directive as usize];
            match &directive.data {
                Directive::Document(document) => Some((entry, directive, document)),
                _ => None,
            }
        })
        .collect::<Vec<_>>();
    // the store keeps the directives of a day in the order it read them: by time, then as written
    let mut by_day = directives.clone();
    by_day.sort_by_key(|(entry, _, _)| entry.directive);
    let mut paths: HashMap<u32, Cow<'a, Path>> = HashMap::new();
    for (entry, directive, document) in by_day {
        let date = date_of(&directive.data).unwrap_or_default();
        let path = resolved
            .get_mut(&(document.account.name(), date))
            .and_then(VecDeque::pop_front)
            .map(|path| Cow::Borrowed(Path::new(path)))
            .unwrap_or_else(|| ledger_file(ledger, Path::new(document.filename.as_str())));
        paths.insert(entry.seq, path);
    }
    let mut rows = directives
        .drain(..)
        .map(|(entry, directive, document)| {
            let path = paths.remove(&entry.seq).expect("every document directive has its path");
            row(
                directive,
                DocumentSource::Directive(document),
                document.filename.as_str(),
                path,
                None,
                entry.seq,
            )
        })
        .collect::<Vec<_>>();
    for entry in cache.documented(ledger, store).iter().map(|seq| &entries.rows[*seq as usize]) {
        let directive = &ledger.directives[entry.directive as usize];
        let Directive::Transaction(transaction) = &directive.data else {
            continue;
        };
        let holders = std::iter::once((DocumentSource::Transaction(transaction), &transaction.meta)).chain(
            transaction
                .postings
                .iter()
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
        |data, record| document(record).map_or(Value::Null, |it| Value::Int(data.entry_order(it.seq).into())),
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
    rows: Rows::Records(|ledger, store, _| directives_where(ledger, store, |it| matches!(it, Directive::Commodity(_)))),
};

fn commodity<'r>(record: &'r Record<'_>) -> Option<&'r zhang_ast::Commodity> {
    match &directive(record)?.data {
        Directive::Commodity(commodity) => Some(commodity),
        _ => None,
    }
}

static COMMODITY_COLUMNS: &[ColumnDef] = &[
    ColumnDef::record("meta", DataType::Str, "Metadata of the commodity, as `key: \"value\"` pairs.", |_, record| {
        meta_value(record)
    }),
    ColumnDef::record("date", DataType::Date, "Date of the commodity directive.", |_, record| date_value(record)),
    ColumnDef::record("name", DataType::Str, "The currency or commodity, e.g. 'USD'.", |_, record| {
        str_value(commodity(record).map(|it| it.currency.as_str()))
    }),
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
