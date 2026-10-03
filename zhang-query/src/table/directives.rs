//! The tables with one row per directive of a kind, as in beanquery: `#balances`, `#notes`,
//! `#events`, `#documents` and `#commodities`, plus the helpers the directive tables share.
//!
//! Rows come in ledger order: by date, then as beancount orders the directives of a day
//! ([`ledger_order`]). `#documents` adds, after its directives, the documents that
//! transactions and postings name in their metadata.

use std::collections::{BTreeSet, HashMap};
use std::path::{Component, Path, PathBuf};

use bigdecimal::{BigDecimal, Zero};
use chrono::{Datelike, NaiveDate};
use uuid::Uuid;
use zhang_ast::amount::Amount;
use zhang_ast::{Account, Directive, Meta, Posting, SpanInfo, Spanned, Transaction};
use zhang_core::ledger::Ledger;
use zhang_core::store::Store;

use super::{directive_meta, ledger_file, render_meta, ColumnDef, LedgerCache, Record, Rows, Table};
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
/// are no entries: the correcting transactions of balance assertions, and those zhang rejected.
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

/// Whether two spans start at the same position of the same file.
fn same_position(a: &SpanInfo, b: &SpanInfo) -> bool {
    a.start == b.start && a.filename == b.filename
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
/// `discrepancy` are the balances computed ([`actual_balances`]).
fn balance_rows<'a>(ledger: &'a Ledger, store: &'a Store, projection: Projection) -> Vec<Record<'a>> {
    let wanted = ["actual", "passed", "discrepancy"]
        .into_iter()
        .any(|name| BALANCES.column(name).is_some_and(|column| projection.contains(column)));
    let mut actuals = if wanted { actual_balances(ledger, store) } else { HashMap::new() };
    ledger_order(ledger, store)
        .filter(|directive| assertion(&directive.data).is_some())
        .map(|directive| Record::Balance {
            directive,
            actual: actuals.remove(&std::ptr::from_ref(directive)),
        })
        .collect()
}

/// A balance assertion of [`actual_balances`]: the bound of the directives whose transactions
/// it sees (their index in the ledger's directives is below it), the assertion, and the asserted
/// amount.
type Assertion<'a> = (usize, &'a Spanned<Directive>, &'a Amount);

/// The true balance of the account of every assertion, in the assertion's currency, keyed by
/// the assertion: the units of every posting to that very account (not its sub-accounts, as
/// zhang checks a balance) that the store folded before the assertion. The correcting
/// transactions (flag `C`) zhang inserts after its checks are not postings, so a failed
/// assertion does not move the balance. A `balance ... with pad` is checked after its own
/// padding transaction (flag `P`), which follows it in the stream.
///
/// The postings are the cached rows of the asserted accounts ([`LedgerCache`]), in the order
/// the store folded them, which is the order of the ledger's directives.
fn actual_balances<'a>(ledger: &'a Ledger, store: &'a Store) -> HashMap<*const Spanned<Directive>, Amount> {
    let cache = LedgerCache::of(ledger, store);
    let (postings, entries) = (cache.postings(ledger, store), cache.entries(ledger, store));
    // the assertions of every account in fold order, each with the bound of the transactions it
    // sees: those folded before it, and its own padding for a balance with pad
    let mut assertions: HashMap<&str, Vec<Assertion<'_>>> = HashMap::new();
    for (idx, directive) in ledger.directives.iter().enumerate() {
        let Some((account, amount, _)) = assertion(&directive.data) else {
            continue;
        };
        let padding = matches!(directive.data, Directive::BalancePad(_))
            && ledger.directives.get(idx + 1).is_some_and(|next| same_position(&next.span, &directive.span))
            && entries.of_directive(idx + 1).is_some_and(|entry| entry.txn.is_some());
        let bound = if padding { idx + 2 } else { idx };
        assertions.entry(account.name()).or_default().push((bound, directive, amount));
    }

    let mut actuals = HashMap::new();
    for (account, checks) in assertions {
        let rows = postings.account_rows(account);
        // currency -> units of the postings so far
        let mut sums: HashMap<&str, BigDecimal> = HashMap::new();
        let mut next = 0;
        // the last posting added up: the rows booking splits a posting into share it
        let mut last = None;
        for (bound, directive, amount) in checks {
            while let Some(row) = rows.get(next).map(|idx| &postings.rows[*idx as usize]) {
                let entry = &postings.entries[row.entry as usize];
                let folded_at = entry.entry.map(|seq| entries.rows[seq as usize].directive as usize);
                if folded_at.is_some_and(|at| at >= bound) {
                    break;
                }
                next += 1;
                if last == Some((row.entry, row.posting_index)) {
                    continue;
                }
                last = Some((row.entry, row.posting_index));
                // a row booked against a lot may hold a part of the posting, or its units
                // written with the lot's scale: those are read from the stored posting
                let units = match row.lot.as_ref().and_then(|lot| lot.cost.as_ref()) {
                    None => Some(&row.units),
                    Some(_) => store
                        .transactions
                        .get(&entry.id)
                        .and_then(|txn| txn.postings.get(row.posting_index as usize))
                        .map(|posting| &posting.inferred_amount),
                };
                if let Some(units) = units {
                    *sums.entry(units.commodity.as_str()).or_insert_with(BigDecimal::zero) += &units.number;
                }
            }
            let units = sums.get(amount.commodity.as_str()).cloned().unwrap_or_else(BigDecimal::zero);
            actuals.insert(std::ptr::from_ref(directive), Amount::new(units, amount.commodity.clone()));
        }
    }
    actuals
}

/// Whether an assertion holds: `|actual - asserted| <= tolerance`, without a tolerance
/// exactly, as zhang's balance check decides it.
fn holds(actual: &Amount, asserted: &Amount, tolerance: Option<&BigDecimal>) -> bool {
    let distance = (&actual.number - &asserted.number).abs();
    tolerance.map_or(distance.is_zero(), |tolerance| distance <= *tolerance)
}

fn balance_field(record: &Record<'_>, get: impl Fn(&Account, &Amount, Option<&BigDecimal>) -> Value) -> Value {
    directive(record)
        .and_then(|it| assertion(&it.data))
        .map_or(Value::Null, |(account, amount, tolerance)| get(account, amount, tolerance))
}

/// The asserted amount, tolerance and true balance of an assertion row, if computed.
fn balance_check<'r>(record: &'r Record<'_>) -> Option<(&'r Amount, Option<&'r BigDecimal>, &'r Amount)> {
    let Record::Balance {
        directive,
        actual: Some(actual),
    } = record
    else {
        return None;
    };
    assertion(&directive.data).map(|(_, amount, tolerance)| (amount, tolerance, actual))
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
            Some((amount, tolerance, actual)) if !holds(actual, amount, tolerance) => {
                Value::Amount(Amount::new(&actual.number - &amount.number, amount.commodity.clone()))
            }
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
         account, not counting its sub-accounts; a balance with pad includes its padding (zhang extension).",
        |_, record| balance_check(record).map_or(Value::Null, |(_, _, actual)| Value::Amount(actual.clone())),
    ),
    ColumnDef::record(
        "passed",
        DataType::Bool,
        "Whether the assertion holds: actual is within the tolerance of the asserted amount, or equal to it without a \
         tolerance (zhang extension).",
        |_, record| balance_check(record).map_or(Value::Null, |(amount, tolerance, actual)| Value::Bool(holds(actual, amount, tolerance))),
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
    path: &'a Path,
    /// the id of the transaction the store keeps, for a document named in metadata
    transaction_id: Option<Uuid>,
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
fn document_rows<'a>(ledger: &'a Ledger, store: &'a Store, _projection: Projection) -> Vec<Record<'a>> {
    let row = |directive, source, filename: &'a str, transaction_id| {
        Record::Document(DocumentRow {
            directive,
            source,
            filename,
            path: ledger_file(ledger, Path::new(filename)),
            transaction_id,
        })
    };
    let mut rows = ledger_order(ledger, store)
        .filter_map(|directive| match &directive.data {
            Directive::Document(document) => Some(row(directive, DocumentSource::Directive(document), document.filename.as_str(), None)),
            _ => None,
        })
        .collect::<Vec<_>>();
    let cache = LedgerCache::of(ledger, store);
    let entries = cache.entries(ledger, store);
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
                rows.push(row(directive, source, filename.as_str(), entry.txn));
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
         metadata of a transaction or of one of its postings (zhang extension).",
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
        "Path of the document as written, relative to the ledger's directory: the path the web UI downloads it with \
         (zhang extension).",
        |_, record| document(record).map_or(Value::Null, |it| Value::Str(it.path.to_string_lossy().into_owned())),
    ),
    ColumnDef::record(
        "transaction_id",
        DataType::Str,
        "Id of the transaction whose metadata names the document, its id in the postings table; NULL for a document \
         directive (zhang extension).",
        |_, record| {
            document(record)
                .and_then(|it| it.transaction_id)
                .map_or(Value::Null, |id| Value::Str(id.to_string()))
        },
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
