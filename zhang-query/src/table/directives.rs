//! The tables with one row per directive of a kind, as in beanquery: `#balances`, `#notes`,
//! `#events`, `#documents` and `#commodities`, plus the helpers the directive tables share.
//!
//! Rows come in ledger order: by date, then as beancount orders the directives of a day
//! ([`ledger_order`]).

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Component, Path, PathBuf};

use chrono::{Datelike, NaiveDate};
use zhang_ast::amount::Amount;
use zhang_ast::error::ErrorKind;
use zhang_ast::{Account, Directive, Flag, Spanned};
use zhang_core::ledger::Ledger;
use zhang_core::store::Store;

use super::{directive_meta, render_meta, ColumnDef, Record, Rows, Table};
use crate::projector::Projection;
use crate::value::{DataType, Value};

/// The date of a dated directive.
pub(super) fn date_of(directive: &Directive) -> Option<NaiveDate> {
    directive.datetime().map(|datetime| datetime.date())
}

/// Where beancount sorts a directive among the directives of its day: `open` first, then
/// balance assertions, the other kinds, `document` and `close` last.
fn day_rank(directive: &Directive) -> i8 {
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
/// without a time).
pub(super) fn ledger_order(ledger: &Ledger) -> Vec<&Spanned<Directive>> {
    let mut directives = ledger.directives.iter().collect::<Vec<_>>();
    directives.sort_by_key(|directive| (date_of(&directive.data), day_rank(&directive.data)));
    directives
}

/// One [`Record::Directive`] per directive that `keep` selects, in [`ledger_order`].
pub(super) fn directives_where<'a>(ledger: &'a Ledger, keep: impl Fn(&Directive) -> bool) -> Vec<Record<'a>> {
    ledger_order(ledger)
        .into_iter()
        .filter(|directive| keep(&directive.data))
        .map(Record::Directive)
        .collect()
}

/// The directive of a [`Record::Directive`] or [`Record::Balance`] row.
pub(super) fn directive<'r>(record: &'r Record<'_>) -> Option<&'r Spanned<Directive>> {
    match record {
        Record::Directive(directive) | Record::Balance { directive, .. } => Some(directive),
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

// ---------------------------------------------------------------------------------------
// #balances

pub(super) static BALANCES: Table = Table {
    name: "balances",
    description: "One row per balance assertion (balance, and balance with pad), in ledger order.",
    columns: BALANCE_COLUMNS,
    wildcard: &["date", "account", "amount", "tolerance", "discrepancy"],
    rows: Rows::Records(balance_rows),
};

/// The account, asserted amount and tolerance of a balance assertion.
fn assertion(directive: &Directive) -> Option<(&Account, &Amount, Option<&bigdecimal::BigDecimal>)> {
    match directive {
        Directive::BalanceCheck(check) => Some((&check.account, &check.amount, check.tolerance.as_ref())),
        Directive::BalancePad(pad) => Some((&pad.account, &pad.amount, None)),
        _ => None,
    }
}

/// The balance assertions. Only when the projection reads `discrepancy` are the failed checks
/// looked up: a check zhang reported as failed has a discrepancy of the balance minus the
/// asserted amount, which is minus the correcting transaction (flag `C`) the check inserted
/// right after itself. A `balance ... with pad` always holds.
fn balance_rows<'a>(ledger: &'a Ledger, store: &'a Store, projection: Projection) -> Vec<Record<'a>> {
    let wanted = BALANCES.column("discrepancy").is_some_and(|column| projection.contains(column));
    // the correcting transaction of a check has the check's span
    let mut corrections = HashMap::new();
    if wanted {
        let failed = store
            .errors
            .iter()
            .filter(|error| error.error_type == ErrorKind::AccountBalanceCheckError)
            .filter_map(|error| error.span.as_ref())
            .map(|span| (span.filename.as_deref(), span.start))
            .collect::<HashSet<_>>();
        for directive in &ledger.directives {
            let key = (directive.span.filename.as_deref(), directive.span.start);
            if let Directive::Transaction(txn) = &directive.data {
                if txn.flag == Some(Flag::BalanceCheck) && failed.contains(&key) {
                    if let Some(distance) = txn.postings.first().and_then(|posting| posting.units.as_ref()) {
                        corrections.insert(key, distance);
                    }
                }
            }
        }
    }
    let discrepancy_of = |check: &Spanned<Directive>| -> Option<Amount> {
        corrections
            .get(&(check.span.filename.as_deref(), check.span.start))
            .map(|distance| -(*distance).clone())
    };
    ledger_order(ledger)
        .into_iter()
        .filter(|directive| assertion(&directive.data).is_some())
        .map(|directive| Record::Balance {
            directive,
            discrepancy: if wanted && matches!(directive.data, Directive::BalanceCheck(_)) {
                discrepancy_of(directive)
            } else {
                None
            },
        })
        .collect()
}

fn balance_field(record: &Record<'_>, get: impl Fn(&Account, &Amount, Option<&bigdecimal::BigDecimal>) -> Value) -> Value {
    directive(record)
        .and_then(|it| assertion(&it.data))
        .map_or(Value::Null, |(account, amount, tolerance)| get(account, amount, tolerance))
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
        "When the assertion fails, the balance minus the asserted amount; NULL when it holds.",
        |_, record| match record {
            Record::Balance {
                discrepancy: Some(discrepancy),
                ..
            } => Value::Amount(discrepancy.clone()),
            _ => Value::Null,
        },
    ),
    ColumnDef::record("meta", DataType::Str, "Metadata of the assertion, as `key: \"value\"` pairs.", |_, record| {
        meta_value(record)
    }),
];

// ---------------------------------------------------------------------------------------
// #notes

pub(super) static NOTES: Table = Table {
    name: "notes",
    description: "One row per note directive, in ledger order.",
    columns: NOTE_COLUMNS,
    wildcard: &["date", "account", "comment", "tags", "links"],
    rows: Rows::Records(|ledger, _, _| directives_where(ledger, |it| matches!(it, Directive::Note(_)))),
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
    rows: Rows::Records(|ledger, _, _| directives_where(ledger, |it| matches!(it, Directive::Event(_)))),
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
    description: "One row per document directive, in ledger order.",
    columns: DOCUMENT_COLUMNS,
    wildcard: &["date", "account", "filename", "tags", "links"],
    rows: Rows::Records(|ledger, _, _| directives_where(ledger, |it| matches!(it, Directive::Document(_)))),
};

fn document<'r>(record: &'r Record<'_>) -> Option<&'r zhang_ast::Document> {
    match &directive(record)?.data {
        Directive::Document(document) => Some(document),
        _ => None,
    }
}

/// The path of a document, as beancount resolves it: a relative path is relative to the
/// directory of the file that holds the directive.
fn document_path(record: &Record<'_>) -> Value {
    let (Some(document), Some(directive)) = (document(record), directive(record)) else {
        return Value::Null;
    };
    let path = Path::new(document.filename.as_str());
    let resolved = match directive.span.filename.as_deref().and_then(Path::parent) {
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

static DOCUMENT_COLUMNS: &[ColumnDef] = &[
    ColumnDef::record("date", DataType::Date, "Date of the document.", |_, record| date_value(record)),
    ColumnDef::record("account", DataType::Str, "The account the document belongs to.", |_, record| {
        str_value(document(record).map(|it| it.account.name()))
    }),
    ColumnDef::record(
        "filename",
        DataType::Str,
        "Path of the document file; a relative path is resolved against the directory of the ledger file that declares it.",
        |_, record| document_path(record),
    ),
    ColumnDef::record("tags", DataType::Set, "Tags of the document.", |_, record| {
        document(record).map_or(Value::Null, |it| set_value(it.tags.iter().flatten()))
    }),
    ColumnDef::record("links", DataType::Set, "Links of the document.", |_, record| {
        document(record).map_or(Value::Null, |it| set_value(it.links.iter().flatten()))
    }),
    ColumnDef::record("meta", DataType::Str, "Metadata of the document, as `key: \"value\"` pairs.", |_, record| {
        meta_value(record)
    }),
];

// ---------------------------------------------------------------------------------------
// #commodities

pub(super) static COMMODITIES: Table = Table {
    name: "commodities",
    description: "One row per commodity directive, in ledger order.",
    columns: COMMODITY_COLUMNS,
    wildcard: &["meta", "date", "name"],
    rows: Rows::Records(|ledger, _, _| directives_where(ledger, |it| matches!(it, Directive::Commodity(_)))),
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
