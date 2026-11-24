//! Analyzers as providers: their manifests and outputs (`zhang_shared::plugin_abi::analysis`), as a ledger's catalog
//! and snapshots take them (RFC #698 §3.2 to §3.4).
//!
//! **Names.** An analyzer's tables and columns are named after its namespace: a table `items` of the namespace `shop` is
//! `#shop_items`, a label column `tags` is `postings.shop_tags`, a named query `item` is `shop.item`. The namespace is
//! the manifest's, or the plugin's name with every character other than a lower-case letter, a digit or `_` written
//! `_`. The names the budget system has always had (`#budgets`, `#budget_events`, `#budget_definitions`,
//! `postings.budgets` and the `budgets.*` queries) are kept for `zhang:budget`: the analyzer granted them names them as
//! they are, and it serves them instead of the ones zhang declares for it ([`super::compat`]), whose other tables and
//! queries stay; no other analyzer can have them, whatever its namespace. Nor can an analyzer name a query in the
//! namespace of zhang's own built-in queries ([`BUILTIN_QUERY_NAMESPACES`]).
//!
//! **Outputs.** A cell is JSON as the query API writes one ([`cell`]), checked against its column's type; a row of a
//! record table has a cell per column and maybe its metadata, a fact of a series a cell per column but the partition
//! columns (its partition's key) and the period's parts (generated from the period). The output as a whole holds at
//! most [`MAX_OUTPUT_VALUES`] values, counted as a query's result counts them. Anything else that does not hold, the
//! checks of every snapshot included ([`super::snapshot::publish`]), is an error naming it, and nothing of the output is
//! published.

use std::collections::BTreeMap;
use std::sync::Arc;

use bigdecimal::BigDecimal;
use chrono::Datelike;
use serde_json::{Map, Value as Json};
use zhang_ast::amount::Amount;
use zhang_ast::{Meta, ZhangString};
use zhang_shared::plugin_abi::analysis as abi;

/// The provider the legacy budget names are granted to (§3.4 of the RFC).
pub(crate) use super::compat::BUDGET;
use super::snapshot::{publish, Cause, FactRow, Label, Partition, RecordRow, Snapshot, TableData};
use super::{ColumnDecl, DatePart, Fill, LabelDecl, Manifest, NamedQuery, PeriodUnit, Provider, RecordOrder, SeriesDecl, TableDecl, TableKind};
use crate::executor::weight;
use crate::params::{from_json, JsonError, Params};
use crate::value::{DataType, Value};

/// The tables the budget system has always had, kept for `zhang:budget`.
pub(crate) const LEGACY_TABLES: [&str; 3] = ["budgets", "budget_events", "budget_definitions"];

/// The columns of `postings` the budget system has always had, kept for `zhang:budget`.
pub(crate) const LEGACY_COLUMNS: [&str; 1] = ["budgets"];

/// The named queries the budget pages have always run, kept for `zhang:budget`.
pub(crate) const LEGACY_QUERIES: [&str; 5] = ["budgets.month", "budgets.budget", "budgets.budget_month", "budgets.events", "budgets.postings"];

/// The namespaces of zhang's own built-in queries, `accounts.list` or `report.summary`, and of the budget pages'
/// (`budgets.month`): an analyzer whose namespace is one of them declares no named query.
pub const BUILTIN_QUERY_NAMESPACES: [&str; 7] = ["accounts", "budgets", "commodities", "journals", "ledger", "postings", "report"];

/// The most values an analysis may publish, counted as a query's result counts them (`ExecuteOptions::max_result_values`).
pub const MAX_OUTPUT_VALUES: u64 = 1_000_000;

/// The types a column of an analyzer's table may have.
const COLUMN_TYPES: [DataType; 8] = [
    DataType::Str,
    DataType::Int,
    DataType::Decimal,
    DataType::Date,
    DataType::Bool,
    DataType::Amount,
    DataType::Set,
    DataType::Metas,
];

/// The namespace of the analyzer `plugin`: `declared`, or the plugin's name with every character other than a lower-case
/// letter, a digit or `_` written `_` (`fx-rate` is `fx_rate`). It is a name, `[a-z][a-z0-9_]*`.
pub(crate) fn namespace(plugin: &str, declared: Option<&str>) -> Result<String, String> {
    let namespace = match declared {
        Some(declared) => declared.to_owned(),
        None => plugin
            .chars()
            .map(|it| {
                if it.is_ascii_lowercase() || it.is_ascii_digit() || it == '_' {
                    it
                } else {
                    '_'
                }
            })
            .collect(),
    };
    super::check_name(&namespace).map_err(|err| match declared {
        Some(_) => format!("its namespace '{}' is no name: {}", namespace, err),
        None => format!("its name gives the namespace '{}', which is no name ({}): declare a namespace", namespace, err),
    })?;
    Ok(namespace)
}

/// The manifest of the analyzer `provider` (the plugin's name), whose tables and columns are named after its namespace:
/// `legacy` grants it the legacy budget names, which it then names as they are. An error says what does not hold; the
/// checks every manifest gets follow when the catalog is built ([`super::build`]).
pub(crate) fn manifest(provider: &str, declared: &abi::Manifest, legacy: bool) -> Result<Manifest, String> {
    if declared.analysis_abi != abi::ANALYSIS_ABI {
        return Err(format!(
            "its manifest is written for the analysis ABI {}, and this zhang reads {}",
            declared.analysis_abi,
            abi::ANALYSIS_ABI
        ));
    }
    let namespace = namespace(provider, declared.namespace.as_deref())?;
    let named = |name: &str, legacy_names: &[&str], what: &str| -> Result<String, String> {
        if legacy && legacy_names.contains(&name) {
            return Ok(name.to_owned());
        }
        let full = format!("{}_{}", namespace, name);
        if legacy_names.contains(&full.as_str()) {
            return Err(format!("{} {} would be named {}, which is kept for {}", what, name, full, BUDGET));
        }
        Ok(full)
    };
    let tables = declared
        .tables
        .iter()
        .map(|table| table_decl(table, named(&table.name, &LEGACY_TABLES, "table")?).map_err(|err| format!("table '{}': {}", table.name, err)))
        .collect::<Result<Vec<_>, _>>()?;
    let labels = declared
        .columns
        .iter()
        .map(|column| {
            let what = format!("column {}.{}", column.table, column.name);
            let name = named(&column.name, &LEGACY_COLUMNS, "column")?;
            let params = column
                .params
                .iter()
                .map(|(name, ty)| Ok((name.clone(), param_type(ty).map_err(|err| format!("{}: parameter :{}: {}", what, name, err))?)))
                .collect::<Result<Vec<_>, String>>()?;
            Ok(LabelDecl {
                table: column.table.clone(),
                name,
                published_as: column.name.clone(),
                description: column.description.clone(),
                predicate: column.predicate.clone(),
                params,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let queries = declared
        .queries
        .iter()
        .map(|query| {
            let what = format!("query {}", query.name);
            let name = query_name(&namespace, &query.name, legacy).map_err(|err| format!("{}: {}", what, err))?;
            let params = query
                .params
                .iter()
                .map(|param| {
                    Ok((
                        param.name.clone(),
                        param_type(&param.ty).map_err(|err| format!("{}: parameter :{}: {}", what, param.name, err))?,
                    ))
                })
                .collect::<Result<Vec<_>, String>>()?;
            Ok(NamedQuery {
                name,
                description: query.description.clone(),
                bql: query.bql.clone(),
                params,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(Manifest {
        provider: provider.to_owned(),
        tables,
        labels,
        queries,
    })
}

/// The name of the named query `name` of an analyzer of the namespace `namespace` (`legacy` for `zhang:budget`):
/// `<namespace>.<name>`, or a legacy query's name as it is.
fn query_name(namespace: &str, name: &str, legacy: bool) -> Result<String, String> {
    if legacy && LEGACY_QUERIES.contains(&name) {
        return Ok(name.to_owned());
    }
    super::check_name(name)?;
    if BUILTIN_QUERY_NAMESPACES.contains(&namespace) {
        return Err(format!(
            "it would be named {}.{}, in the namespace of zhang's built-in queries: declare another namespace",
            namespace, name
        ));
    }
    Ok(format!("{}.{}", namespace, name))
}

/// The declaration of the table `table`, named `name` in the catalog.
fn table_decl(table: &abi::TableManifest, name: String) -> Result<TableDecl, String> {
    let columns = table
        .columns
        .iter()
        .map(|column| {
            let ty = column_type(&column.ty).map_err(|err| format!("column '{}': {}", column.name, err))?;
            let fill = match &column.fill {
                None => None,
                Some(abi::Fill::Named(fill)) if fill == "carry" => Some(Fill::Carry),
                Some(abi::Fill::Named(fill)) if fill == "zero" => Some(Fill::Zero),
                Some(abi::Fill::Previous { previous }) => Some(Fill::Previous(previous.clone())),
                Some(abi::Fill::Named(other)) => {
                    return Err(format!(
                        "column '{}' is filled with '{}': a fill is carry, zero or {{\"previous\": column}}",
                        column.name, other
                    ))
                }
            };
            Ok(ColumnDecl {
                name: column.name.clone(),
                ty,
                description: column.description.clone(),
                fill,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let kind = match table.kind {
        abi::TableKind::Records => {
            if table.period.is_some() || !table.partition.is_empty() {
                return Err("a record table has no period and no partition".to_owned());
            }
            TableKind::Records(match &table.order {
                None | Some(abi::RecordOrder::Published(_)) => RecordOrder::Published,
                Some(abi::RecordOrder::Columns(columns)) => RecordOrder::Columns(columns.clone()),
            })
        }
        abi::TableKind::Series => {
            if table.order.is_some() {
                return Err("a series table has the order of its periods".to_owned());
            }
            let period = table.period.as_ref().ok_or("a series table has a period")?;
            let unit = match period.unit.as_str() {
                "month" => PeriodUnit::Month,
                other => return Err(format!("its period is a '{}': the only unit is month", other)),
            };
            let derive = period
                .derive
                .iter()
                .map(|(column, part)| match part.as_str() {
                    "year" => Ok((column.clone(), DatePart::Year)),
                    "month" => Ok((column.clone(), DatePart::Month)),
                    other => Err(format!("column '{}' is the '{}' of the period: a part is year or month", column, other)),
                })
                .collect::<Result<Vec<_>, _>>()?;
            TableKind::Series(SeriesDecl {
                period: period.column.clone(),
                unit,
                derive,
                partition: table.partition.clone(),
                extend_to_today: table.extend_to_today,
            })
        }
    };
    let wildcard = table
        .wildcard
        .clone()
        .unwrap_or_else(|| columns.iter().map(|column| column.name.clone()).collect());
    Ok(TableDecl {
        name,
        published_as: table.name.clone(),
        description: table.description.clone(),
        kind,
        columns,
        wildcard,
    })
}

/// The type of a column named `name` in a manifest.
fn column_type(name: &str) -> Result<DataType, String> {
    COLUMN_TYPES.into_iter().find(|ty| ty.name() == name).ok_or_else(|| {
        let names = COLUMN_TYPES.map(|ty| ty.name()).join(", ");
        format!("its type is '{}', which is not one of {}", name, names)
    })
}

/// The type of a parameter of a label column named `name` in a manifest: one a query's parameter can have.
fn param_type(name: &str) -> Result<DataType, String> {
    crate::params::JSON_TYPES.into_iter().find(|ty| ty.name() == name).ok_or_else(|| {
        let names = crate::params::JSON_TYPES.map(|ty| ty.name()).join(", ");
        format!("its type is '{}', which is not one of {}", name, names)
    })
}

/// The snapshot `output`, the output of the analyzer `provider`, checked against its manifest, on a ledger of
/// `entries` entries, which the causes of a series name by their `seq`: see the module docs. The error names what does
/// not hold.
pub(crate) fn snapshot(provider: &Provider, output: &abi::Output, entries: usize) -> Result<Snapshot, String> {
    let manifest = &provider.manifest;
    let mut values = Values::default();
    let table = |name: &str, kind: &str| {
        manifest
            .tables
            .iter()
            .find(|decl| decl.published_as == name && matches!((&decl.kind, kind), (TableKind::Records(_), "record") | (TableKind::Series(_), "series")))
            .ok_or_else(|| format!("its manifest declares no {} table {}", kind, name))
    };
    let mut tables = vec![];
    for (name, rows) in &output.tables {
        let decl = table(name, "record")?;
        let rows = rows
            .iter()
            .enumerate()
            .map(|(at, row)| record_row(decl, row, &mut values).map_err(|err| format!("table {}, row {}: {}", name, at + 1, err)))
            .collect::<Result<Vec<_>, _>>()?;
        tables.push((decl.name.clone(), TableData::Records(rows)));
    }
    for (name, series) in &output.series {
        let decl = table(name, "series")?;
        let partitions = series
            .partitions
            .iter()
            .enumerate()
            .map(|(at, partition)| {
                series_partition(decl, partition, entries, &mut values).map_err(|err| format!("table {}, partition {}: {}", name, at + 1, err))
            })
            .collect::<Result<Vec<_>, _>>()?;
        tables.push((decl.name.clone(), TableData::Series(partitions)));
    }
    let mut labels = vec![];
    for (name, published) in &output.labels {
        let decl = manifest
            .labels
            .iter()
            .find(|decl| decl.published_as == *name)
            .ok_or_else(|| format!("its manifest declares no label column {}", name))?;
        let published = published
            .iter()
            .map(|label| label_of(decl, label, &mut values).map_err(|err| format!("label column {}, label {:?}: {}", name, label.label, err)))
            .collect::<Result<Vec<_>, _>>()?;
        labels.push((decl.name.clone(), published));
    }
    publish(provider, tables, labels)
}

/// The values of an output so far, which it may hold at most [`MAX_OUTPUT_VALUES`] of.
#[derive(Default)]
pub(crate) struct Values(u64);

impl Values {
    pub(crate) fn count(&mut self, value: &Value) -> Result<(), String> {
        self.0 += weight(value);
        match self.0 > MAX_OUTPUT_VALUES {
            true => Err(format!("the output holds more than {} values", MAX_OUTPUT_VALUES)),
            false => Ok(()),
        }
    }
}

/// The most digits a decimal of an analyzer's output has, written out in full, before and after its point together:
/// what `zhang_convert` reads and answers ([`abi::MAX_CONVERTED_DIGITS`]). A longer one is refused before it is parsed,
/// which takes the square of its length: a decimal of the 999,999 digits a ledger's number may have costs most of a
/// second, and an output holds a million values.
pub const MAX_OUTPUT_DIGITS: u64 = abi::MAX_CONVERTED_DIGITS;

/// The decimal written `text` in an analyzer's output: a decimal as a query's parameter reads one, of at most
/// [`MAX_OUTPUT_DIGITS`] digits written out in full. Its text is bounded before it is parsed (its digits, its sign, its
/// point and its exponent together), then the number, so that `1e5000` is refused as `1` with 5,000 zeros is.
pub(crate) fn output_decimal(text: &str) -> Result<BigDecimal, String> {
    let too_long = || format!("it has more than {} digits written out in full, the most an analyzer writes", MAX_OUTPUT_DIGITS);
    let trimmed = text.trim();
    if trimmed.len() as u64 > MAX_OUTPUT_DIGITS + 32 || trimmed.bytes().filter(u8::is_ascii_digit).count() as u64 > MAX_OUTPUT_DIGITS {
        return Err(too_long());
    }
    let number = match crate::params::decimal(trimmed).map_err(json_error)? {
        Value::Decimal(number) => number,
        _ => return Err("a decimal is a string, such as \"12.50\"".to_owned()),
    };
    match written_digits(&number) > MAX_OUTPUT_DIGITS {
        true => Err(too_long()),
        false => Ok(number),
    }
}

/// The digits of `number` written out in full, before and after its point together (`1e3` is `1000`: 4).
fn written_digits(number: &BigDecimal) -> u64 {
    let (digits, scale) = (number.digits(), number.fractional_digit_count());
    match scale > 0 {
        // the digits, with a zero before the point when they are all after it
        true => digits.max(scale.unsigned_abs() + 1),
        false if bigdecimal::Zero::is_zero(number) => 1,
        false => digits.saturating_add(scale.unsigned_abs()),
    }
}

/// The cell `json` of a column of the type `ty`, as the query API writes one: see [`abi::Output`].
pub(crate) fn cell(ty: DataType, json: &Json) -> Result<Value, String> {
    let decimal = |json: &Json| match json {
        Json::String(text) => output_decimal(text).map(Value::Decimal),
        _ => Err("a decimal is a string, such as \"12.50\"".to_owned()),
    };
    match (ty, json) {
        (_, Json::Null) => Ok(Value::Null),
        (DataType::Decimal, json) => decimal(json),
        (DataType::Amount, Json::Object(amount)) => {
            let number = amount.get("number").ok_or("an amount has a number")?;
            let Value::Decimal(number) = decimal(number)? else {
                return Err("an amount has a number".to_owned());
            };
            let currency = amount.get("currency").and_then(Json::as_str).ok_or("an amount has a currency, a string")?;
            Ok(Value::Amount(Amount::new(number, currency)))
        }
        // the shorter form, `["12.50", "CNY"]`
        (DataType::Amount, Json::Array(pair)) if pair.len() == 2 => {
            let Value::Decimal(number) = decimal(&pair[0])? else {
                return Err("an amount has a number".to_owned());
            };
            let currency = pair[1].as_str().ok_or("an amount has a currency, a string")?;
            Ok(Value::Amount(Amount::new(number, currency)))
        }
        (DataType::Metas, Json::Array(pairs)) => pairs
            .iter()
            .map(|pair| {
                let text = |key: &str| pair.get(key).and_then(Json::as_str).map(str::to_owned);
                text("key")
                    .zip(text("value"))
                    .ok_or("metadata is a list of {\"key\", \"value\"} strings".to_owned())
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Metas),
        (DataType::Amount, _) => Err("an amount is {\"number\": \"12.50\", \"currency\": \"CNY\"} or [\"12.50\", \"CNY\"]".to_owned()),
        (DataType::Metas, _) => Err("metadata is a list of {\"key\", \"value\"} strings".to_owned()),
        (ty, json) => from_json(ty, json).map_err(json_error),
    }
}

/// What a JSON value that is no cell of its type is.
fn json_error(error: JsonError) -> String {
    match error {
        JsonError::Expected(expected) => format!("it must be {}", expected),
        JsonError::OutOfRange => "it is out of range".to_owned(),
    }
}

/// The metadata `$meta` of a row: `{"key": ["value", ...]}`.
fn row_meta(row: &Map<String, Json>) -> Result<Option<Arc<Meta>>, String> {
    let Some(meta) = row.get(abi::ROW_META) else {
        return Ok(None);
    };
    let entries = meta.as_object().ok_or("its $meta is {\"key\": [\"value\"]}")?;
    let mut parsed = Meta::default();
    for (key, values) in entries {
        let values = values.as_array().ok_or("its $meta is {\"key\": [\"value\"]}")?;
        for value in values {
            let value = value.as_str().ok_or("its $meta is {\"key\": [\"value\"]}")?;
            parsed.insert(key.clone(), ZhangString::quote(value));
        }
    }
    Ok(Some(Arc::new(parsed)))
}

/// The cell of the column `column` of `row`, which must have it.
fn cell_of(row: &Map<String, Json>, column: &ColumnDecl, values: &mut Values) -> Result<Value, String> {
    let json = row.get(&column.name).ok_or_else(|| format!("it has no {}", column.name))?;
    let value = cell(column.ty, json).map_err(|err| format!("{}: {}", column.name, err))?;
    values.count(&value)?;
    Ok(value)
}

/// `row` has no cell but those of `expected` columns and its metadata.
fn no_other_cells(row: &Map<String, Json>, expected: impl Fn(&str) -> bool) -> Result<(), String> {
    match row.keys().find(|key| *key != abi::ROW_META && !expected(key)) {
        Some(key) => Err(format!("it has a cell {}, which is no column it has", key)),
        None => Ok(()),
    }
}

/// A row of the record table `decl`.
fn record_row(decl: &TableDecl, row: &Json, values: &mut Values) -> Result<RecordRow, String> {
    let row = match row {
        Json::Object(row) => row,
        // a list of its cells, in the order of the columns, maybe followed by its metadata
        Json::Array(cells) => return listed_row(decl, cells, values),
        _ => return Err("a row is an object of its cells, or a list of them".to_owned()),
    };
    no_other_cells(row, |key| decl.columns.iter().any(|column| column.name == key))?;
    let cells = decl.columns.iter().map(|column| cell_of(row, column, values)).collect::<Result<Vec<_>, _>>()?;
    Ok(RecordRow {
        cells: cells.into_boxed_slice(),
        meta: row_meta(row)?,
    })
}

/// A row of the record table `decl` written as the list `listed`: its cells in the order of the columns, maybe
/// followed by its metadata, `{"key": ["value"]}`.
fn listed_row(decl: &TableDecl, listed: &[Json], values: &mut Values) -> Result<RecordRow, String> {
    let columns = decl.columns.len();
    if listed.len() != columns && listed.len() != columns + 1 {
        return Err(format!(
            "it lists {} cells, where the table has {} columns (and maybe the row's metadata after them)",
            listed.len(),
            columns
        ));
    }
    let cells = decl
        .columns
        .iter()
        .zip(listed)
        .map(|(column, json)| {
            let value = cell(column.ty, json).map_err(|err| format!("{}: {}", column.name, err))?;
            values.count(&value)?;
            Ok(value)
        })
        .collect::<Result<Vec<_>, String>>()?;
    let meta = match listed.get(columns) {
        None => None,
        Some(meta) => row_meta(&Map::from_iter([(abi::ROW_META.to_owned(), meta.clone())]))?,
    };
    Ok(RecordRow {
        cells: cells.into_boxed_slice(),
        meta,
    })
}

/// A partition of the series table `decl`, on a ledger of `entries` entries.
fn series_partition(decl: &TableDecl, partition: &abi::PartitionOutput, entries: usize, values: &mut Values) -> Result<Partition, String> {
    let TableKind::Series(series) = &decl.kind else {
        unreachable!("a series table")
    };
    let date = |text: &str, what: &str| match cell(DataType::Date, &Json::String(text.to_owned())) {
        Ok(Value::Date(date)) => Ok(date),
        _ => Err(format!("its {} is a date YYYY-MM-DD, not {:?}", what, text)),
    };
    let cause = |cause: &Option<abi::Cause>, what: &str| match cause {
        Some(cause) if cause.seq as usize >= entries => Err(format!("its {} names the entry {}, which the ledger does not have", what, cause.seq)),
        cause => Ok(cause.map(|cause| Cause { seq: cause.seq })),
    };
    let part = |name: &str| series.derive.iter().find(|(column, _)| column == name).map(|(_, part)| *part);
    let in_key = |name: &str| series.partition.iter().any(|column| column == name);
    if let Some(key) = partition.key.keys().find(|key| !in_key(key)) {
        return Err(format!("its key has {}, which is no partition column", key));
    }
    let mut key = BTreeMap::new();
    for column in decl.columns.iter().filter(|column| in_key(&column.name)) {
        let value = cell_of(&partition.key, column, values).map_err(|err| format!("its key: {}", err))?;
        key.insert(column.name.as_str(), value);
    }
    let mut facts: Vec<FactRow> = Vec::with_capacity(partition.rows.len());
    for (at, row) in partition.rows.iter().enumerate() {
        let previous = facts.last();
        let fact = (|| {
            no_other_cells(row, |name| {
                !in_key(name) && part(name).is_none() && decl.columns.iter().any(|column| column.name == name)
            })?;
            let period = match row.get(&series.period) {
                Some(Json::String(text)) => date(text, "period")?,
                _ => return Err(format!("it has no {}, the period", series.period)),
            };
            let cells = decl
                .columns
                .iter()
                .enumerate()
                .map(
                    |(index, column)| match (column.name == series.period, part(&column.name), key.get(column.name.as_str())) {
                        (true, _, _) => Ok(Value::Date(period)),
                        (_, Some(DatePart::Year), _) => Ok(Value::Int(i64::from(period.year()))),
                        (_, Some(DatePart::Month), _) => Ok(Value::Int(i64::from(period.month()))),
                        (_, None, Some(value)) => Ok(value.clone()),
                        // a fact after the first may leave out a cell filled with `carry`: it is the fact's before
                        (_, None, None) => match (row.contains_key(&column.name), &column.fill, previous) {
                            (false, Some(Fill::Carry), Some(previous)) => {
                                let value = previous.cells[index].clone();
                                values.count(&value)?;
                                Ok(value)
                            }
                            _ => cell_of(row, column, values),
                        },
                    },
                )
                .collect::<Result<Vec<_>, _>>()?;
            // and its metadata: then it has the fact's before, as a generated period does
            let meta = match (row.contains_key(abi::ROW_META), previous) {
                (false, Some(previous)) => previous.meta.clone(),
                _ => row_meta(row)?,
            };
            Ok(FactRow {
                period,
                cells: cells.into_boxed_slice(),
                meta,
            })
        })()
        .map_err(|err: String| format!("row {}: {}", at + 1, err))?;
        facts.push(fact);
    }
    Ok(Partition {
        facts,
        until: date(&partition.until, "until")?,
        until_cause: cause(&partition.until_cause, "until_cause")?,
        first_cause: cause(&partition.first_cause, "first_cause")?,
    })
}

/// A label of the label column `decl`.
fn label_of(decl: &LabelDecl, label: &abi::LabelOutput, values: &mut Values) -> Result<Label, String> {
    if let Some(name) = label.params.keys().find(|name| !decl.params.iter().any(|(param, _)| param == *name)) {
        return Err(format!("it binds :{}, which is no parameter of the column", name));
    }
    let mut params = Params::new();
    for (name, ty) in &decl.params {
        let json = label.params.get(name).ok_or_else(|| format!("it does not bind :{}", name))?;
        let value = cell(*ty, json).map_err(|err| format!(":{}: {}", name, err))?;
        values.count(&value)?;
        params = params.bind(name.clone(), value);
    }
    values.count(&Value::Str(label.label.clone()))?;
    Ok(Label {
        label: label.label.clone(),
        params,
    })
}
