//! The analysis ABI: what an `Analyzer` plugin declares about the tables it provides (its [`Manifest`]) and what it
//! publishes for one loaded ledger (its [`Output`]), as JSON. The host checks both against each other before anything
//! is published (RFC #698 §2.2, §3.2, §3.3).
//!
//! Like the rest of the plugin ABI, a field may be added and a reader ignores fields it does not know; a field is never
//! removed, renamed or made required. [`ANALYSIS_ABI`] versions the shapes on their own.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// The version of the analysis ABI a [`Manifest`] is written for.
pub const ANALYSIS_ABI: u32 = 1;

/// What an analyzer declares about itself, read once while it registers: what it reads, the diagnostics it may report,
/// and the tables and columns it provides. It is static, so a ledger's catalog is known before any analysis runs, even
/// when one fails. On the wire (wrapped here for reading):
///
/// ```json
/// {"analysis_abi": 1, "namespace": "shop", "consumes": ["custom:shop"], "open_meta": ["shop"],
///  "diagnostics": [{"kind": "UnknownItem"}, {"kind": "Unpriced", "phase": "after_fold"}],
///  "tables": [{"name": "items", "kind": "records", "columns": [{"name": "item", "type": "str"}], "order": ["item"]}],
///  "columns": [{"table": "postings", "name": "items", "type": "set", "kind": "labels",
///               "predicate": "account IN :accounts", "params": {"accounts": "set"}}]}
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    /// the version of the analysis ABI the manifest is written for, [`ANALYSIS_ABI`]
    #[serde(default)]
    pub analysis_abi: u32,
    /// What the names of the plugin's tables and columns start with, `[a-z][a-z0-9_]*`: a table `items` of the
    /// namespace `shop` is `#shop_items`. `None` for the plugin's name, lower case, with every character other than a
    /// letter, a digit or `_` written `_`
    #[serde(default)]
    pub namespace: Option<String>,
    /// the kinds of the records the plugin reads, such as `budget-add` or `custom:shop` (the `custom "shop"`
    /// directives)
    #[serde(default)]
    pub consumes: Vec<String>,
    /// the metadata keys of `open` directives the plugin reads; with no record to read and no `open` with one of
    /// these keys, the plugin is not called
    #[serde(default)]
    pub open_meta: Vec<String>,
    /// the kinds of the diagnostics the plugin may report; any other kind fails its analysis
    #[serde(default)]
    pub diagnostics: Vec<DiagnosticDecl>,
    /// the tables the plugin provides
    #[serde(default)]
    pub tables: Vec<TableManifest>,
    /// the columns the plugin adds to a core table: label columns of `postings`
    #[serde(default)]
    pub columns: Vec<LabelManifest>,
}

/// A kind of diagnostic an analyzer may report, and where the host places it in the ledger's errors.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiagnosticDecl {
    pub kind: String,
    #[serde(default)]
    pub phase: Phase,
}

/// Where a diagnostic goes in the ledger's errors, which decides the ids of the errors (RFC #698 §2.6).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// with the errors of the directive it names, after those of the directives before it and before those of the
    /// directives after it, as the core's own checks report theirs
    #[default]
    Fold,
    /// after the errors of every directive, as a check that needs every price of the ledger reports
    AfterFold,
}

/// A table an analyzer provides.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TableManifest {
    /// its name, after the namespace: `[a-z][a-z0-9_]*`
    pub name: String,
    pub kind: TableKind,
    #[serde(default)]
    pub description: String,
    /// at most 64
    pub columns: Vec<ColumnManifest>,
    /// the columns `SELECT *` expands to, in order; `None` for every column
    #[serde(default)]
    pub wildcard: Option<Vec<String>>,
    /// a record table's row order: `"published"` (the order of the output, the default) or the columns the rows are
    /// sorted by, which the host checks
    #[serde(default)]
    pub order: Option<RecordOrder>,
    /// a series table's period
    #[serde(default)]
    pub period: Option<PeriodManifest>,
    /// a series table's partition columns, each filled with `carry`
    #[serde(default)]
    pub partition: Vec<String>,
    /// whether a series that ends before the current period runs up to it, in every query
    #[serde(default)]
    pub extend_to_today: bool,
}

/// How a table has its rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TableKind {
    /// every row is in the output
    Records,
    /// periods per partition, the missing ones generated per query by the columns' fill
    Series,
}

/// The order of a record table's rows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RecordOrder {
    /// `"published"`: as the output lists them
    Published(PublishedOrder),
    /// sorted by these columns, ascending, as `ORDER BY` sorts
    Columns(Vec<String>),
}

/// The one word of [`RecordOrder::Published`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PublishedOrder {
    Published,
}

/// A column of an analyzer's table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColumnManifest {
    pub name: String,
    /// a type of the query language: `str`, `int`, `decimal`, `date`, `bool`, `amount`, `set` or `metas`
    #[serde(rename = "type")]
    pub ty: String,
    #[serde(default)]
    pub description: String,
    /// in a series table, how a generated period fills the column: none for the period and the columns derived
    /// from it
    #[serde(default)]
    pub fill: Option<Fill>,
}

/// How a generated period of a series fills a column: `"carry"`, `"zero"` or `{"previous": "<column>"}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Fill {
    /// `"carry"` or `"zero"`
    Named(String),
    /// the previous row's value of another column of the same type, filled with `carry`
    Previous { previous: String },
}

/// The period of a series table: a date column holding the first day of each period, and the columns derived from it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeriodManifest {
    pub column: String,
    /// `month`, the only unit of this version
    pub unit: String,
    /// the columns of a part of the period's date, by column: `{"year": "year", "month": "month"}`
    #[serde(default)]
    pub derive: BTreeMap<String, String>,
}

/// A label column an analyzer adds to a core table: a set column whose value, for a row, is the labels whose
/// predicate holds for it. The predicate is BQL over a row of the table, reading `:label` (the label) and the
/// declared parameters, which each label binds in the output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabelManifest {
    /// `postings`, the only table of this version
    pub table: String,
    /// its name, after the namespace
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub predicate: String,
    /// the parameters every label binds, by name, with their types
    #[serde(default)]
    pub params: BTreeMap<String, String>,
}

/// What an analyzer's `analyze` export answers for one loaded ledger: the diagnostics it reports and the rows of its
/// tables. The host checks all of it against the plugin's [`Manifest`] and publishes it whole, or nothing of it. A cell
/// is JSON as the query API writes one: a string, an integer, a boolean, a date `YYYY-MM-DD`, an amount
/// `{"number": "12.50", "currency": "CNY"}` with its number as text, a set as a list of strings, metadata as
/// `{"key": ["value"]}`, or `null`. A decimal is text too: a JSON number in a `decimal` or `amount` cell is refused.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Output {
    #[serde(default)]
    pub diagnostics: Vec<Diagnostic>,
    /// the rows of every record table, by its name in the manifest: each row has a cell per column, by name, and may
    /// have its metadata under `$meta`, which `meta()` reads
    #[serde(default)]
    pub tables: BTreeMap<String, Vec<Map<String, Value>>>,
    /// the facts of every series table, by its name in the manifest
    #[serde(default)]
    pub series: BTreeMap<String, SeriesOutput>,
    /// the labels of every label column, by its name in the manifest
    #[serde(default)]
    pub labels: BTreeMap<String, Vec<LabelOutput>>,
}

/// The key of the metadata of a row of [`Output`].
pub const ROW_META: &str = "$meta";

/// A problem an analyzer reports in the ledger.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    /// a kind its manifest declares
    pub kind: String,
    pub target: Target,
    #[serde(default)]
    pub metas: BTreeMap<String, String>,
    /// what the problem is, for people
    #[serde(default)]
    pub message: Option<String>,
}

/// What a diagnostic is about: exactly one of an entry by its `seq` (its row of `#entries`), a transaction by its id,
/// or the plugin itself: `{"seq": 41}`, `{"txn": "…"}` or `{"plugin": true}`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Target {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seq: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub txn: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin: Option<bool>,
}

/// The facts of one series table.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SeriesOutput {
    #[serde(default)]
    pub partitions: Vec<PartitionOutput>,
}

/// One partition of a series: its key, its facts, and how far it runs.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PartitionOutput {
    /// a cell per partition column
    pub key: Map<String, Value>,
    /// the first day of the last period of the series, as the plugin sees it (a query may extend it to the current
    /// period, or end it earlier)
    pub until: String,
    /// the entry whose date sets `until`
    #[serde(default)]
    pub until_cause: Option<Cause>,
    /// the entry the series starts with
    #[serde(default)]
    pub first_cause: Option<Cause>,
    /// the periods that have something, in order: a cell per column but the partition columns and those derived from
    /// the period, and the metadata under `$meta`
    pub rows: Vec<Map<String, Value>>,
}

/// An entry of the ledger, by its `seq`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cause {
    pub seq: u32,
}

/// A label of a label column and the parameters its predicate binds.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LabelOutput {
    pub label: String,
    #[serde(default)]
    pub params: Map<String, Value>,
}

#[cfg(test)]
mod test {
    use serde_json::json;

    use super::*;

    #[test]
    fn should_read_a_manifest_with_defaults_and_ignore_what_it_does_not_know() {
        let manifest: Manifest = serde_json::from_value(json!({
            "analysis_abi": 1,
            "consumes": ["budget"],
            "diagnostics": [{"kind": "A"}, {"kind": "B", "phase": "after_fold"}],
            "tables": [
                {"name": "t", "kind": "records", "columns": [{"name": "c", "type": "str"}], "order": "published"},
                {"name": "s", "kind": "series", "columns": [{"name": "d", "type": "date"}, {"name": "n", "type": "int", "fill": "zero"},
                 {"name": "m", "type": "int", "fill": {"previous": "n"}}],
                 "period": {"column": "d", "unit": "month", "derive": {"y": "year"}}, "partition": [], "row_meta": "carry"},
                {"name": "u", "kind": "records", "columns": [], "order": ["c"]}
            ],
            "queries": [{"name": "later"}]
        }))
        .unwrap();
        assert_eq!(manifest.namespace, None);
        assert_eq!(manifest.diagnostics[0].phase, Phase::Fold);
        assert_eq!(manifest.diagnostics[1].phase, Phase::AfterFold);
        assert_eq!(manifest.tables[0].order, Some(RecordOrder::Published(PublishedOrder::Published)));
        assert_eq!(manifest.tables[2].order, Some(RecordOrder::Columns(vec!["c".to_owned()])));
        assert_eq!(manifest.tables[1].columns[1].fill, Some(Fill::Named("zero".to_owned())));
        assert_eq!(manifest.tables[1].columns[2].fill, Some(Fill::Previous { previous: "n".to_owned() }));
        assert_eq!(manifest.tables[1].period.as_ref().unwrap().derive["y"], "year");
        assert!(manifest.columns.is_empty());
        // a kind of table this version does not know is no manifest
        assert!(serde_json::from_value::<Manifest>(json!({"tables": [{"name": "t", "kind": "cube", "columns": []}]})).is_err());
    }

    #[test]
    fn should_write_a_target_as_its_one_field() {
        let seq = Target {
            seq: Some(41),
            ..Target::default()
        };
        assert_eq!(serde_json::to_value(&seq).unwrap(), json!({"seq": 41}));
        let output: Output = serde_json::from_value(json!({
            "diagnostics": [{"kind": "A", "target": {"plugin": true}}],
            "series": {"s": {"partitions": [{"key": {"name": "x"}, "until": "2024-03-01", "rows": [{"n": 1, "$meta": {"k": ["v"]}}]}]}}
        }))
        .unwrap();
        assert_eq!(output.diagnostics[0].target.plugin, Some(true));
        assert_eq!(output.series["s"].partitions[0].rows[0][ROW_META], json!({"k": ["v"]}));
        assert!(output.tables.is_empty() && output.labels.is_empty());
    }
}
