//! The analysis ABI: what an `Analyzer` plugin declares about the tables it provides (its [`Manifest`]) and what it
//! publishes for one loaded ledger (its [`Output`]), as JSON. The host checks both against each other before anything
//! is published (RFC #698 §2.2, §3.2, §3.3).
//!
//! Like the rest of the plugin ABI, a field may be added and a reader ignores fields it does not know; a field is never
//! removed, renamed or made required. [`ANALYSIS_ABI`] versions the shapes on their own.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use serde_json::{Map, Value};

/// The version of the analysis ABI a [`Manifest`] is written for.
pub const ANALYSIS_ABI: u32 = 1;

/// The most bytes of JSON an analyzer's manifest has.
pub const MAX_MANIFEST_BYTES: usize = 1024 * 1024;

/// The most bytes of JSON one analysis answers.
pub const MAX_OUTPUT_BYTES: usize = 32 * 1024 * 1024;

/// The most JSON values (objects, arrays, strings, numbers, booleans and nulls, keys aside) one analysis answers.
pub const MAX_OUTPUT_NODES: usize = 4_000_000;

/// The most JSON objects one analysis answers: rows, amounts, targets, metas. An object costs the host far more once
/// read than its bytes of JSON, so they are counted on their own.
pub const MAX_OUTPUT_OBJECTS: usize = 400_000;

/// Check that `json` is JSON within `max_bytes` bytes, `max_nodes` values and `max_objects` objects, without reading
/// it into memory: what the host checks before it reads an analyzer's output ([`Output`]) or manifest ([`Manifest`]),
/// so that a small text of many tiny objects cannot make it hold gigabytes. The error says which bound it passes.
pub fn check_size(json: &[u8], max_bytes: usize, max_nodes: usize, max_objects: usize) -> Result<(), String> {
    use serde::de::{DeserializeSeed, IgnoredAny, MapAccess, SeqAccess, Visitor};

    if json.len() > max_bytes {
        return Err(format!("it is {} bytes of JSON, more than {}", json.len(), max_bytes));
    }
    struct Counter {
        nodes: usize,
        objects: usize,
        max_nodes: usize,
        max_objects: usize,
    }
    impl Counter {
        fn node<E: serde::de::Error>(&mut self, object: bool) -> Result<(), E> {
            self.nodes += 1;
            self.objects += usize::from(object);
            if self.nodes > self.max_nodes {
                return Err(E::custom(format!("it holds more than {} JSON values", self.max_nodes)));
            }
            if self.objects > self.max_objects {
                return Err(E::custom(format!("it holds more than {} JSON objects", self.max_objects)));
            }
            Ok(())
        }
    }
    struct Node<'c>(&'c mut Counter);
    impl<'de> DeserializeSeed<'de> for Node<'_> {
        type Value = ();
        fn deserialize<D: serde::Deserializer<'de>>(self, deserializer: D) -> Result<(), D::Error> {
            deserializer.deserialize_any(self)
        }
    }
    impl<'de> Visitor<'de> for Node<'_> {
        type Value = ();
        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("JSON")
        }
        fn visit_bool<E: serde::de::Error>(self, _: bool) -> Result<(), E> {
            self.0.node(false)
        }
        fn visit_i64<E: serde::de::Error>(self, _: i64) -> Result<(), E> {
            self.0.node(false)
        }
        fn visit_u64<E: serde::de::Error>(self, _: u64) -> Result<(), E> {
            self.0.node(false)
        }
        fn visit_f64<E: serde::de::Error>(self, _: f64) -> Result<(), E> {
            self.0.node(false)
        }
        fn visit_str<E: serde::de::Error>(self, _: &str) -> Result<(), E> {
            self.0.node(false)
        }
        fn visit_unit<E: serde::de::Error>(self) -> Result<(), E> {
            self.0.node(false)
        }
        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<(), A::Error> {
            self.0.node(false)?;
            while seq.next_element_seed(Node(&mut *self.0))?.is_some() {}
            Ok(())
        }
        fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<(), A::Error> {
            self.0.node(true)?;
            while map.next_key::<IgnoredAny>()?.is_some() {
                map.next_value_seed(Node(&mut *self.0))?;
            }
            Ok(())
        }
    }
    let mut counter = Counter {
        nodes: 0,
        objects: 0,
        max_nodes,
        max_objects,
    };
    let mut deserializer = serde_json::Deserializer::from_slice(json);
    Node(&mut counter)
        .deserialize(&mut deserializer)
        .and_then(|()| deserializer.end())
        .map_err(|e| match e.is_data() {
            // the bound it passes, as the counter said it, without where the reader stopped
            true => e.to_string().split(" at line").next().unwrap_or_default().to_owned(),
            false => e.to_string(),
        })
}

/// The export an analyzer declares its [`Manifest`] in, as JSON; read once while it registers.
pub const MANIFEST: &str = "manifest";

/// The export an analyzer analyses a ledger with: an [`Input`] as JSON in, an [`Output`] as JSON out.
pub const ANALYZE: &str = "analyze";

/// The host function that converts amounts with the ledger's prices, as the query engine converts them
/// (`convert(position, currency, date)`). `zhang_convert(input: i64) -> i64` takes the offset of a block holding a JSON
/// list of [`Conversion`]s and answers `{"Ok": [...]}`, a [`ConvertedAmount`] or `null` (no price converts it) for each,
/// in order. Exact: the numbers are text, and the host computes them as it computes a query's.
pub const CONVERT: &str = "zhang_convert";

/// The most conversions one `zhang_convert` call converts.
pub const MAX_CONVERSIONS: usize = 10_000;

/// The most digits a number `zhang_convert` reads or answers has, written out in full, before and after its point
/// together, as a number a write writes has (`MAX_WRITTEN_DIGITS` of the server); an answer has at most twice as many.
pub const MAX_CONVERTED_DIGITS: u64 = 1000;

/// What `analyze` reads: the records of the kinds the analyzer consumes, in the order zhang processed the ledger, and
/// what the ledger is kept in.
///
/// ```json
/// {"records": [{"kind": "budget-add", "seq": 41, "date": "2024-03-01", "time": "00:00:00", "has_time": false,
///               "timestamp": 1709251200, "fields": {"name": "Food", "amount": {"number": "500.00", "currency": "CNY"}},
///               "meta": {"note": ["groceries"]}, "file": "budget.zhang", "line": 12}],
///  "ledger": {"title": null, "operating_currency": "CNY", "timezone": "Asia/Shanghai"}}
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Input {
    pub records: Vec<Record>,
    pub ledger: super::LedgerInfo,
}

/// A directive of a kind an analyzer consumes, as a record: its place, its date and time, and its fields by name.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Record {
    /// `budget`, `budget-add`, `budget-transfer`, `budget-close`, or `custom:<type>` for a `custom "<type>"` directive
    pub kind: String,
    /// its row of `#entries`: its position in the order zhang processed the ledger, which a posting's `seq` compares
    /// with, and what a diagnostic names it by
    pub seq: u32,
    /// its date, `YYYY-MM-DD`, as written
    pub date: String,
    /// its time of day in the ledger's timezone, `HH:MM:SS`; `00:00:00` without one
    pub time: String,
    /// whether it is written with a time
    pub has_time: bool,
    /// the Unix time in seconds of its date and time in the ledger's timezone
    pub timestamp: i64,
    /// its fields: `name` and `commodity` (`budget`); `name` and `amount` (`budget-add`); `from`, `to` and `amount`
    /// (`budget-transfer`); `name` (`budget-close`); `values`, a list of strings (`custom`). An amount is
    /// `{"number": "500.00", "currency": "CNY"}`
    pub fields: Map<String, Value>,
    /// its metadata, every value of each key in order
    pub meta: BTreeMap<String, Vec<String>>,
    /// the file it is written in, within the ledger, with `/`
    pub file: Option<String>,
    /// its line there, from 1
    pub line: Option<usize>,
}

/// One amount for `zhang_convert` to convert: into `target` with the prices of `date`, and through `via`, a cost
/// currency, when no price converts it directly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Conversion {
    pub amount: ConvertedAmount,
    pub target: String,
    /// `YYYY-MM-DD`
    pub date: String,
    #[serde(default)]
    pub via: Option<String>,
}

/// An amount on the wire: its number as text, exact, and its currency.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConvertedAmount {
    pub number: String,
    pub currency: String,
}

/// What an analyzer declares about itself, read once while it registers: what it reads, the diagnostics it may report,
/// and the tables and columns it provides. It is static, so a ledger's catalog is known before any analysis runs, even
/// when one fails. On the wire (wrapped here for reading):
///
/// ```json
/// {"analysis_abi": 1, "namespace": "shop", "consumes": ["custom:shop"], "open_meta": ["shop"],
///  "diagnostics": [{"kind": "UnknownItem"}, {"kind": "Unpriced", "phase": "after_fold"}],
///  "tables": [{"name": "items", "kind": "records", "columns": [{"name": "item", "type": "str"}], "order": ["item"]}],
///  "columns": [{"table": "postings", "name": "items", "type": "set", "kind": "labels",
///               "predicate": "account IN :accounts", "params": {"accounts": "set"}}],
///  "queries": [{"name": "item", "description": "One item.", "bql": "SELECT * FROM #shop_items WHERE item = :item",
///               "params": [{"name": "item", "type": "str"}]}]}
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
    /// the named queries the plugin provides, which the server lists and runs with zhang's built-in queries
    /// (`GET /api/query/builtins`, `POST /api/query/builtins/{name}`)
    #[serde(default)]
    pub queries: Vec<QueryManifest>,
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

/// A named query an analyzer provides: BQL over the ledger's tables, its own included, with typed parameters. The host
/// serves it as one of the built-in queries, under its name after the namespace: a query `item` of the namespace `shop`
/// is `shop.item`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueryManifest {
    /// its name, after the namespace
    pub name: String,
    /// one sentence on what it returns
    #[serde(default)]
    pub description: String,
    /// the query, with its parameters written `:name`
    pub bql: String,
    /// every parameter the query uses, in the order the server lists them
    #[serde(default)]
    pub params: Vec<QueryParamManifest>,
}

/// A parameter of a named query.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueryParamManifest {
    /// `month` for `:month`
    pub name: String,
    /// `bool`, `int`, `decimal`, `str`, `date` or `set`
    #[serde(rename = "type")]
    pub ty: String,
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

/// An [`Output`] as the host reads it first: its diagnostics, and the JSON text of each table, series and label
/// column, by its name, which the host then reads straight into the cells their declarations say, with no tree of
/// JSON values in between. It reads the same JSON as [`Output`].
#[derive(Debug, Default, Deserialize)]
pub struct RawOutput<'a> {
    #[serde(default)]
    pub diagnostics: Vec<Diagnostic>,
    /// the rows of each record table, as in [`Output::tables`]
    #[serde(default, borrow)]
    pub tables: RawEntries<'a>,
    /// each series table, as in [`Output::series`]
    #[serde(default, borrow)]
    pub series: RawEntries<'a>,
    /// the labels of each label column, as in [`Output::labels`]
    #[serde(default, borrow)]
    pub labels: RawEntries<'a>,
}

/// The entries of a JSON object as the host reads them first: each key with the JSON text of its value, in the order
/// written. A key may be written twice, which [`Output`] reads as the last one only, its earlier values read whole
/// nonetheless: a reader of these entries that does not check its earlier values takes no such object.
#[derive(Debug, Default)]
pub struct RawEntries<'a>(pub Vec<(String, &'a RawValue)>);

impl<'de: 'a, 'a> Deserialize<'de> for RawEntries<'a> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Entries<'a>(std::marker::PhantomData<&'a ()>);
        impl<'de: 'a, 'a> serde::de::Visitor<'de> for Entries<'a> {
            type Value = RawEntries<'a>;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("an object")
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> Result<RawEntries<'a>, A::Error> {
                let mut entries = vec![];
                while let Some((key, value)) = map.next_entry::<String, &'de RawValue>()? {
                    entries.push((key, value));
                }
                Ok(RawEntries(entries))
            }
        }
        deserializer.deserialize_map(Entries(std::marker::PhantomData))
    }
}

impl<'a> RawEntries<'a> {
    /// Whether a key is written twice.
    pub fn repeats_a_key(&self) -> bool {
        let mut keys = self.0.iter().map(|(key, _)| key.as_str()).collect::<Vec<_>>();
        keys.sort_unstable();
        keys.windows(2).any(|pair| pair[0] == pair[1])
    }
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
            "queries": [{"name": "q", "bql": "SELECT 1", "later": true}],
            "pages": [{"name": "later"}]
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
        assert_eq!(
            manifest.queries,
            [QueryManifest {
                name: "q".to_owned(),
                description: String::new(),
                bql: "SELECT 1".to_owned(),
                params: vec![],
            }]
        );
        // a kind of table this version does not know is no manifest
        assert!(serde_json::from_value::<Manifest>(json!({"tables": [{"name": "t", "kind": "cube", "columns": []}]})).is_err());
    }

    #[test]
    fn should_check_the_size_of_a_json_text_without_reading_it() {
        assert_eq!(check_size(br#"{"a": [1, "x", null, {"b": true}]}"#, 100, 7, 2), Ok(()));
        assert_eq!(
            check_size(br#"{"a": [1, "x", null, {"b": true}]}"#, 100, 6, 2),
            Err("it holds more than 6 JSON values".to_owned())
        );
        assert_eq!(check_size(br#"[{}, {}, {}]"#, 100, 10, 2), Err("it holds more than 2 JSON objects".to_owned()));
        assert_eq!(check_size(b"[1, 2]", 3, 10, 2), Err("it is 6 bytes of JSON, more than 3".to_owned()));
        assert!(check_size(b"{\"a\": ", 100, 10, 2).unwrap_err().starts_with("EOF while parsing"));
        assert!(check_size(b"[1] [2]", 100, 10, 2).unwrap_err().starts_with("trailing characters"));
        // a text of tiny objects stops at the bound, whatever its length
        let tiny = format!("[{}{{}}]", "{},".repeat(1_000_000));
        assert_eq!(
            check_size(tiny.as_bytes(), usize::MAX, usize::MAX, 1000),
            Err("it holds more than 1000 JSON objects".to_owned())
        );
        // deeply nested arrays are refused by the reader before they exhaust the stack
        let deep = format!("{}{}", "[".repeat(100_000), "]".repeat(100_000));
        assert!(check_size(deep.as_bytes(), usize::MAX, usize::MAX, usize::MAX).is_err());
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
