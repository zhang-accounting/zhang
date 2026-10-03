//! The `errors` table: one row per ledger error, the problems the web UI lists on its errors
//! page and `GET /api/errors` returns.
//!
//! A user queries errors to clean a ledger up: how many errors of each kind there are, which
//! files and accounts they come from, which ones appeared since a date, and where exactly each
//! one is. The columns follow from that.
//!
//! - **`kind` is a stable name to filter and group on.** It is the error code, as
//!   `/api/errors` returns it in `error_type` and as the error code guide of the documentation
//!   names it, such as `UnbalancedTransaction` or `AccountDoesNotExist`.
//! - **`message` is what the UI says**: the English text the errors page shows for the kind.
//! - **Where it is.** `file` is the file of the directive that raised the error, relative to
//!   the ledger's directory as the UI's file list names it (the full path if the file is
//!   outside it), and `source` is the text of that directive, the snippet the UI opens for an
//!   error. `line` and `column` are reserved for its position and are `NULL` for now: zhang
//!   records where a directive starts as an offset into its file, not as a line, and does not
//!   keep the file's text once it is loaded. Rows still come in the order of the files: by
//!   `file`, then by position in the file, which is line order.
//! - **When and what it concerns.** `date` is the date of the directive (`NULL` for undated
//!   ones, such as options), and `account` the account the error is about, for the errors
//!   that name one (an account that does not exist or is closed, a failed balance check).
//!   `meta()` reads the rest of what zhang records about an error, such as `meta('txn_id')`
//!   for errors of a transaction, or `meta('budget_name')`.
//! - **Which error it is.** `id` is the id zhang gives the error, the `id` of `/api/errors`, and
//!   `span_start` and `span_end` are the byte offsets of the directive in its file, as the UI
//!   uses them to open and edit it. The id is derived from the directive's position, so the
//!   errors of one directive share it.
//!
//! An error without a file would come first, as `NULL` sorts first.
//! `SELECT *` gives `file`, `date`, `kind`, `account` and `message`.

use std::borrow::Cow;
use std::collections::HashMap;
use std::path::Path;

use chrono::NaiveDate;
use zhang_ast::error::ErrorKind;
use zhang_ast::SpanInfo;
use zhang_core::domains::schemas::ErrorDomain;
use zhang_core::ledger::Ledger;
use zhang_core::store::Store;

use super::{ledger_file, ColumnDef, Record, Rows, Table};
use crate::projector::Projection;
use crate::value::{DataType, Value};

pub(super) static ERRORS: Table = Table {
    name: "errors",
    description: "One row per ledger error, as the errors page lists them; ordered by file, then position in the file.",
    columns: COLUMNS,
    wildcard: &["file", "date", "kind", "account", "message"],
    rows: Rows::Records(rows),
};

/// A ledger error: a row of the `errors` table.
pub(crate) struct LedgerError<'a> {
    error: &'a ErrorDomain,
    /// the file of the directive, relative to the ledger's directory when it is inside it
    file: Option<&'a Path>,
    /// the date of the directive; only looked up when the `date` column is projected
    date: Option<NaiveDate>,
}

impl LedgerError<'_> {
    pub(super) fn meta(&self, key: &str) -> Option<String> {
        self.error.metas.get(key).cloned()
    }

    fn file(&self) -> Option<Cow<'_, str>> {
        self.file.map(Path::to_string_lossy)
    }
}

/// The text the UI shows for an error of `kind` (`ERROR.<kind>` of the English translation of
/// the frontend).
pub(crate) fn message(kind: &ErrorKind) -> &'static str {
    match kind {
        ErrorKind::UnbalancedTransaction => "Transaction is Unbalanced",
        ErrorKind::TransactionCannotInferTradeAmount => "Cannot infer the trade amount of the transaction",
        ErrorKind::TransactionHasMultipleImplicitPosting => "Transaction has more than one implicit posting unit",
        ErrorKind::TransactionExplicitPostingHaveMultipleCommodity => "Explicit postings of the transaction use multiple commodities",
        ErrorKind::AccountBalanceCheckError => "Account does not pass the balance check",
        ErrorKind::AccountDoesNotExist => "Account does not exist",
        ErrorKind::AccountClosed => "Try to operate a closed account",
        ErrorKind::CommodityDoesNotDefine => "Try to use a undefined commodity",
        ErrorKind::NoEnoughCommodityLot => "Not enough commodity lots to book this posting",
        ErrorKind::CloseNonZeroAccount => "Trying to close an account with non zero balance",
        ErrorKind::BudgetDoesNotExist => "Budget does not exist",
        ErrorKind::DefineDuplicatedBudget => "Trying to define duplicated budget name",
        ErrorKind::MultipleOperatingCurrencyDetect => "Ledger contains multiple operating currency options, which is not recommended in zhang",
        ErrorKind::ParseInvalidMeta => "Directive has an invalid meta value",
        ErrorKind::UnsupportedBookingMethod => "Booking method is not supported yet, the account uses the default booking method",
        ErrorKind::AmbiguousLotMatch => "Reduction matches several lots, which is ambiguous under the STRICT booking method",
        ErrorKind::PluginError => "Plugin {{meta.plugin}}: {{meta.message}}",
    }
}

/// [`message`] with its `{{meta.<key>}}` placeholders filled from `metas`, the way the UI renders it.
/// A missing meta renders as an empty string.
pub(crate) fn render_message(kind: &ErrorKind, metas: &HashMap<String, String>) -> String {
    let template = message(kind);
    let mut rendered = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find("{{meta.") {
        rendered.push_str(&rest[..start]);
        let after = &rest[start + "{{meta.".len()..];
        match after.find("}}") {
            Some(end) => {
                rendered.push_str(metas.get(&after[..end]).map(String::as_str).unwrap_or_default());
                rest = &after[end + "}}".len()..];
            }
            None => {
                rendered.push_str(&rest[start..]);
                rest = "";
            }
        }
    }
    rendered.push_str(rest);
    rendered
}

fn rows<'a>(ledger: &'a Ledger, store: &'a Store, projection: Projection) -> Vec<Record<'a>> {
    let mut errors = store
        .errors
        .iter()
        .map(|error| LedgerError {
            error,
            file: error
                .span
                .as_ref()
                .and_then(|span| span.filename.as_deref())
                .map(|path| ledger_file(ledger, path)),
            date: None,
        })
        .collect::<Vec<_>>();

    if projects(projection, "date") {
        // (file, start) of the erroneous directives -> their date
        let mut dates: HashMap<(Option<&Path>, usize), Option<NaiveDate>> = errors
            .iter()
            .filter_map(|it| it.error.span.as_ref())
            .map(|span| ((span.filename.as_deref(), span.start), None))
            .collect();
        for directive in &ledger.directives {
            if let Some(date) = dates.get_mut(&(directive.span.filename.as_deref(), directive.span.start)) {
                if date.is_none() {
                    *date = directive.datetime().map(|it| it.date());
                }
            }
        }
        for error in &mut errors {
            error.date = error
                .error
                .span
                .as_ref()
                .and_then(|span| dates.get(&(span.filename.as_deref(), span.start)).copied().flatten());
        }
    }

    // by file, then by position in the file; errors of the same directive keep their order
    errors.sort_by(|a, b| {
        let start = |it: &LedgerError<'_>| it.error.span.as_ref().map(|span| span.start);
        a.file().cmp(&b.file()).then_with(|| start(a).cmp(&start(b)))
    });
    errors.into_iter().map(Record::Error).collect()
}

/// Whether the column `name` of this table is projected.
fn projects(projection: Projection, name: &str) -> bool {
    ERRORS.column(name).is_some_and(|column| projection.contains(column))
}

fn ledger_error<'r, 'a>(record: &'r Record<'a>) -> Option<&'r LedgerError<'a>> {
    match record {
        Record::Error(error) => Some(error),
        _ => None,
    }
}

static COLUMNS: &[ColumnDef] = &[
    ColumnDef::record(
        "kind",
        DataType::Str,
        "Error code, such as 'UnbalancedTransaction'; the error_type of GET /api/errors.",
        |_, record| ledger_error(record).map_or(Value::Null, |it| Value::Str(it.error.error_type.to_string())),
    ),
    ColumnDef::record("message", DataType::Str, "What the errors page of the UI says about the error.", |_, record| {
        ledger_error(record).map_or(Value::Null, |it| Value::Str(render_message(&it.error.error_type, &it.error.metas)))
    }),
    ColumnDef::record(
        "file",
        DataType::Str,
        "File of the directive that raised the error, relative to the ledger's directory, or NULL if unknown.",
        |_, record| {
            ledger_error(record)
                .and_then(|it| it.file())
                .map_or(Value::Null, |it| Value::Str(it.into_owned()))
        },
    ),
    ColumnDef::record(
        "line",
        DataType::Int,
        "Line of the directive in its file; NULL for now, as zhang does not record line numbers yet.",
        |_, _| Value::Null,
    ),
    ColumnDef::record(
        "column",
        DataType::Int,
        "Column of the directive in its line; NULL for now, as zhang does not record line numbers yet.",
        |_, _| Value::Null,
    ),
    ColumnDef::record(
        "date",
        DataType::Date,
        "Date of the directive that raised the error, or NULL for an undated directive such as an option.",
        |_, record| ledger_error(record).and_then(|it| it.date).map_or(Value::Null, Value::Date),
    ),
    ColumnDef::record(
        "account",
        DataType::Str,
        "Account the error is about, or NULL if the error does not name one.",
        |_, record| {
            ledger_error(record)
                .and_then(|it| it.error.metas.get("account_name").cloned())
                .map_or(Value::Null, Value::Str)
        },
    ),
    ColumnDef::record(
        "source",
        DataType::Str,
        "Text of the directive that raised the error, or NULL if unknown.",
        |_, record| {
            ledger_error(record)
                .and_then(|it| it.error.span.as_ref())
                .map(|span| span.content.trim_end())
                .filter(|content| !content.is_empty())
                .map_or(Value::Null, |content| Value::Str(content.to_owned()))
        },
    ),
    ColumnDef::record(
        "id",
        DataType::Str,
        "Id of the error, the id of GET /api/errors; the errors of one directive share it.",
        |_, record| ledger_error(record).map_or(Value::Null, |it| Value::Str(it.error.id.clone())),
    ),
    ColumnDef::record(
        "span_start",
        DataType::Int,
        "Byte offset in its file where the directive that raised the error starts, or NULL if unknown.",
        |_, record| span_offset(record, |span| span.start),
    ),
    ColumnDef::record(
        "span_end",
        DataType::Int,
        "Byte offset in its file where the directive that raised the error ends, or NULL if unknown.",
        |_, record| span_offset(record, |span| span.end),
    ),
    ColumnDef::record(
        "metas",
        DataType::Metas,
        "What zhang records about the error, such as txn_id or account_name, as (key, value) pairs sorted by key: the metas of \
         GET /api/errors. A zhang extension.",
        |_, record| ledger_error(record).map_or(Value::Null, |it| Value::Metas(error_metas(it.error))),
    ),
];

/// The details zhang records about an error, as `(key, value)` pairs sorted by key.
fn error_metas(error: &ErrorDomain) -> Vec<(String, String)> {
    let mut metas = error.metas.iter().map(|(key, value)| (key.clone(), value.clone())).collect::<Vec<_>>();
    metas.sort();
    metas
}

/// An offset of the span of the error's directive; NULL for an error without one.
fn span_offset(record: &Record<'_>, offset: fn(&SpanInfo) -> usize) -> Value {
    ledger_error(record)
        .and_then(|it| it.error.span.as_ref())
        .and_then(|span| i64::try_from(offset(span)).ok())
        .map_or(Value::Null, Value::Int)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use zhang_ast::error::ErrorKind;

    use super::{message, render_message};

    /// The `message` of every kind is the text the UI shows: `ERROR.<kind>` of the frontend's
    /// English translation, for every kind it translates.
    #[test]
    fn messages_are_the_ui_texts() {
        let kinds = [
            ErrorKind::UnbalancedTransaction,
            ErrorKind::TransactionCannotInferTradeAmount,
            ErrorKind::TransactionHasMultipleImplicitPosting,
            ErrorKind::TransactionExplicitPostingHaveMultipleCommodity,
            ErrorKind::AccountBalanceCheckError,
            ErrorKind::AccountDoesNotExist,
            ErrorKind::AccountClosed,
            ErrorKind::CommodityDoesNotDefine,
            ErrorKind::NoEnoughCommodityLot,
            ErrorKind::CloseNonZeroAccount,
            ErrorKind::BudgetDoesNotExist,
            ErrorKind::DefineDuplicatedBudget,
            ErrorKind::MultipleOperatingCurrencyDetect,
            ErrorKind::ParseInvalidMeta,
            ErrorKind::UnsupportedBookingMethod,
            ErrorKind::AmbiguousLotMatch,
            ErrorKind::PluginError,
        ];
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../frontend/public/locales/en/translation.json");
        let translation: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let ui = translation["ERROR"].as_object().unwrap();
        assert!(ui.len() >= 12, "{:?}", ui);
        for (name, text) in ui {
            let kind = kinds
                .iter()
                .find(|kind| kind.to_string() == *name)
                .unwrap_or_else(|| panic!("the UI translates an unknown kind {}", name));
            assert_eq!(message(kind), text.as_str().unwrap().trim(), "{}", name);
        }
    }

    #[test]
    fn plugin_error_messages_are_filled_from_the_metas() {
        let metas = HashMap::from([
            ("plugin".to_owned(), "validator".to_owned()),
            ("message".to_owned(), "missing receipt".to_owned()),
        ]);
        assert_eq!(render_message(&ErrorKind::PluginError, &metas), "Plugin validator: missing receipt");
        assert_eq!(render_message(&ErrorKind::PluginError, &Default::default()), "Plugin : ");
        assert_eq!(render_message(&ErrorKind::AccountClosed, &metas), "Try to operate a closed account");
    }
}
