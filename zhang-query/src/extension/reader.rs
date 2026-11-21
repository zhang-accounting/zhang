//! Reading an analyzer's output straight into cells: the quick way of [`super::abi::snapshot`].
//!
//! The output of an analysis is read at every load, so at every write. [`super::abi::snapshot`] reads it as a tree of
//! JSON values first and then checks every cell of it against its column; [`snapshot`] here reads the text of each
//! table, series and label column ([`abi::RawOutput`]) once, straight into the cells of its declaration, with no tree
//! in between. It takes an output the long way takes, and makes the same snapshot of it, but it says nothing about
//! an output that does not hold: anything it does not take as it is (a cell of another type, a missing or a repeated
//! one, a struct written as a list, the limit of values) gives `None`, and the caller reads the output the long way,
//! which says what is wrong. So the messages are the long way's, and an unusual output only costs a second reading.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use bigdecimal::BigDecimal;
use chrono::Datelike;
use serde::de::{DeserializeSeed, Deserializer, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde_json::value::RawValue;
use zhang_ast::amount::Amount;
use zhang_ast::{Meta, ZhangString};
use zhang_shared::plugin_abi::analysis as abi;

use super::abi::Values;
use super::snapshot::{publish, Cause, FactRow, Label, Partition, RecordRow, Snapshot, TableData};
use super::{ColumnDecl, DatePart, LabelDecl, Provider, TableDecl, TableKind};
use crate::params::Params;
use crate::value::{parse_date, DataType, Value};

/// The snapshot of `output`, the output of the analyzer `provider` on a ledger of `entries` entries, read straight into
/// cells: the snapshot [`super::abi::snapshot`] makes of it, or `None` when it does not take the output as it is (see
/// the module docs).
pub(crate) fn snapshot(provider: &Provider, output: &abi::RawOutput<'_>, entries: usize) -> Option<Snapshot> {
    let manifest = &provider.manifest;
    // a table or a label column written twice: the long way reads its earlier values too
    if output.tables.repeats_a_key() || output.series.repeats_a_key() || output.labels.repeats_a_key() {
        return None;
    }
    let mut values = Values::default();
    let declared = |name: &str, series: bool| {
        manifest
            .tables
            .iter()
            .find(|decl| decl.published_as == name && matches!(decl.kind, TableKind::Series(_)) == series)
    };
    let mut tables = vec![];
    for (name, rows) in &output.tables.0 {
        let decl = declared(name, false)?;
        let rows = read(rows, Rows { decl, values: &mut values })?;
        tables.push((decl.name.clone(), TableData::Records(rows)));
    }
    for (name, series) in &output.series.0 {
        let decl = declared(name, true)?;
        let partitions = read(
            series,
            Series {
                decl,
                entries,
                values: &mut values,
            },
        )?;
        tables.push((decl.name.clone(), TableData::Series(partitions)));
    }
    let mut labels = vec![];
    for (name, published) in &output.labels.0 {
        let decl = manifest.labels.iter().find(|decl| decl.published_as == *name)?;
        let published = read(published, Labels { decl, values: &mut values })?;
        labels.push((decl.name.clone(), published));
    }
    publish(provider, tables, labels).ok()
}

/// What `seed` reads of the JSON text `raw`, all of it.
fn read<'de, T: DeserializeSeed<'de>>(raw: &'de RawValue, seed: T) -> Option<T::Value> {
    let mut deserializer = serde_json::Deserializer::from_str(raw.get());
    let value = seed.deserialize(&mut deserializer).ok()?;
    deserializer.end().ok()?;
    Some(value)
}

/// Stop reading: the long way says why.
fn stop<T, E: serde::de::Error>() -> Result<T, E> {
    Err(E::custom("not read the quick way"))
}

/// `value` counted against the limit of the output's values.
fn count<E: serde::de::Error>(values: &mut Values, value: &Value) -> Result<(), E> {
    match values.count(value) {
        Ok(()) => Ok(()),
        Err(_) => stop(),
    }
}

/// The most columns a row read the quick way has: which of its cells were read is a bit each of a `u128`.
const MAX_COLUMNS: usize = 128;

/// A row of `columns` cells, each NULL until it is read.
fn nulls(columns: usize) -> Box<[Value]> {
    std::iter::repeat_with(|| Value::Null).take(columns).collect()
}

/// Reads the keys of an object, each matched by `pick` without being copied.
struct Key<F>(F);

impl<'de, F: FnOnce(&str) -> T, T> DeserializeSeed<'de> for Key<F> {
    type Value = T;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<T, D::Error> {
        struct KeyVisitor<F>(F);
        impl<F: FnOnce(&str) -> T, T> Visitor<'_> for KeyVisitor<F> {
            type Value = T;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a key")
            }
            fn visit_str<E: serde::de::Error>(self, key: &str) -> Result<T, E> {
                Ok((self.0)(key))
            }
        }
        deserializer.deserialize_str(KeyVisitor(self.0))
    }
}

/// A cell of a column of the type `0`, as [`super::abi::cell`] reads one.
struct Cell(DataType);

impl<'de> DeserializeSeed<'de> for Cell {
    type Value = Value;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Value, D::Error> {
        deserializer.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for Cell {
    type Value = Value;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "a cell of the type {}", self.0)
    }

    fn visit_unit<E: serde::de::Error>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }

    fn visit_bool<E: serde::de::Error>(self, it: bool) -> Result<Value, E> {
        match self.0 {
            DataType::Bool => Ok(Value::Bool(it)),
            _ => stop(),
        }
    }

    fn visit_i64<E: serde::de::Error>(self, it: i64) -> Result<Value, E> {
        match self.0 {
            DataType::Int => Ok(Value::Int(it)),
            _ => stop(),
        }
    }

    fn visit_u64<E: serde::de::Error>(self, it: u64) -> Result<Value, E> {
        match (self.0, i64::try_from(it)) {
            (DataType::Int, Ok(it)) => Ok(Value::Int(it)),
            _ => stop(),
        }
    }

    fn visit_str<E: serde::de::Error>(self, text: &str) -> Result<Value, E> {
        match self.0 {
            DataType::Str => Ok(Value::Str(text.to_owned())),
            _ => text_cell(self.0, text),
        }
    }

    fn visit_string<E: serde::de::Error>(self, text: String) -> Result<Value, E> {
        match self.0 {
            DataType::Str => Ok(Value::Str(text)),
            _ => text_cell(self.0, &text),
        }
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut items: A) -> Result<Value, A::Error> {
        match self.0 {
            DataType::Set => {
                let mut set = std::collections::BTreeSet::new();
                while let Some(item) = items.next_element::<String>()? {
                    set.insert(item);
                }
                Ok(Value::Set(set))
            }
            DataType::Metas => {
                let mut pairs = vec![];
                while let Some(pair) = items.next_element_seed(MetaPair)? {
                    pairs.push(pair);
                }
                Ok(Value::Metas(pairs))
            }
            _ => stop(),
        }
    }

    fn visit_map<A: MapAccess<'de>>(self, mut fields: A) -> Result<Value, A::Error> {
        if self.0 != DataType::Amount {
            return stop();
        }
        // {"number": "12.50", "currency": "CNY"}; another key is not read
        let (mut number, mut currency) = (None, None);
        while let Some(key) = fields.next_key_seed(Key(|key: &str| match key {
            "number" => 0,
            "currency" => 1,
            _ => 2,
        }))? {
            match key {
                0 if number.is_none() => number = Some(fields.next_value_seed(Number)?),
                1 if currency.is_none() => currency = Some(fields.next_value::<String>()?),
                2 => {
                    fields.next_value::<IgnoredAny>()?;
                }
                _ => return stop(),
            }
        }
        match (number, currency) {
            (Some(number), Some(currency)) => Ok(Value::Amount(Amount::new(number, currency))),
            _ => stop(),
        }
    }
}

/// The number of an amount: a decimal written as text, read as `cell` reads one, without a copy of the text.
struct Number;

impl<'de> DeserializeSeed<'de> for Number {
    type Value = BigDecimal;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<BigDecimal, D::Error> {
        deserializer.deserialize_str(self)
    }
}

impl Visitor<'_> for Number {
    type Value = BigDecimal;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("a decimal as text")
    }

    fn visit_str<E: serde::de::Error>(self, text: &str) -> Result<BigDecimal, E> {
        match crate::params::decimal(text) {
            Ok(Value::Decimal(number)) => Ok(number),
            _ => stop(),
        }
    }
}

/// A cell of a type other than `str` written as the text `text`: a decimal or a date.
fn text_cell<E: serde::de::Error>(ty: DataType, text: &str) -> Result<Value, E> {
    match ty {
        DataType::Decimal => crate::params::decimal(text).or_else(|_| stop()),
        DataType::Date => parse_date(text).map(Value::Date).map_or_else(stop, Ok),
        _ => stop(),
    }
}

/// A pair `{"key": "...", "value": "..."}` of a `metas` cell; another key is not read.
struct MetaPair;

impl<'de> DeserializeSeed<'de> for MetaPair {
    type Value = (String, String);

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<(String, String), D::Error> {
        deserializer.deserialize_map(self)
    }
}

impl<'de> Visitor<'de> for MetaPair {
    type Value = (String, String);

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("a {\"key\", \"value\"} pair")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut fields: A) -> Result<(String, String), A::Error> {
        let (mut key, mut value) = (None, None);
        while let Some(at) = fields.next_key_seed(Key(|key: &str| match key {
            "key" => 0,
            "value" => 1,
            _ => 2,
        }))? {
            match at {
                0 if key.is_none() => key = Some(fields.next_value::<String>()?),
                1 if value.is_none() => value = Some(fields.next_value::<String>()?),
                2 => {
                    fields.next_value::<IgnoredAny>()?;
                }
                _ => return stop(),
            }
        }
        key.zip(value).map_or_else(stop, Ok)
    }
}

/// The metadata `$meta` of a row, `{"key": ["value", ...]}`, as `row_meta` reads it: its keys in order.
struct RowMeta;

impl<'de> DeserializeSeed<'de> for RowMeta {
    type Value = Arc<Meta>;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Arc<Meta>, D::Error> {
        deserializer.deserialize_map(self)
    }
}

impl<'de> Visitor<'de> for RowMeta {
    type Value = Arc<Meta>;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("metadata")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut entries: A) -> Result<Arc<Meta>, A::Error> {
        let mut read: BTreeMap<String, Vec<String>> = BTreeMap::new();
        while let Some(key) = entries.next_key::<String>()? {
            let values = entries.next_value::<Vec<String>>()?;
            if read.insert(key, values).is_some() {
                return stop();
            }
        }
        let mut meta = Meta::default();
        for (key, values) in read {
            for value in values {
                meta.insert(key.clone(), ZhangString::quote(value));
            }
        }
        Ok(Arc::new(meta))
    }
}

/// The rows of a record table, each an object of its cells and maybe its `$meta`.
struct Rows<'d, 'v> {
    decl: &'d TableDecl,
    values: &'v mut Values,
}

impl<'de> DeserializeSeed<'de> for Rows<'_, '_> {
    type Value = Vec<RecordRow>;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Vec<RecordRow>, D::Error> {
        deserializer.deserialize_seq(self)
    }
}

impl<'de> Visitor<'de> for Rows<'_, '_> {
    type Value = Vec<RecordRow>;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("rows")
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut rows: A) -> Result<Vec<RecordRow>, A::Error> {
        let mut read = Vec::with_capacity(rows.size_hint().unwrap_or(0));
        while let Some(row) = rows.next_element_seed(Row {
            columns: &self.decl.columns,
            values: &mut *self.values,
        })? {
            read.push(row);
        }
        // the snapshot keeps them: no room to spare
        read.shrink_to_fit();
        Ok(read)
    }
}

/// A row of a record table: a cell for each of `columns`, and maybe its `$meta`.
struct Row<'d, 'v> {
    columns: &'d [ColumnDecl],
    values: &'v mut Values,
}

impl<'de> DeserializeSeed<'de> for Row<'_, '_> {
    type Value = RecordRow;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<RecordRow, D::Error> {
        deserializer.deserialize_map(self)
    }
}

impl<'de> Visitor<'de> for Row<'_, '_> {
    type Value = RecordRow;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("a row")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut entries: A) -> Result<RecordRow, A::Error> {
        let columns = self.columns;
        if columns.len() > MAX_COLUMNS {
            return stop();
        }
        // the cells are read in place, each once
        let (mut cells, mut read) = (nulls(columns.len()), 0u128);
        let mut meta = None;
        while let Some(key) = entries.next_key_seed(Key(|key: &str| column_key(columns, key)))? {
            match key {
                ColumnKey::Meta if meta.is_none() => meta = Some(entries.next_value_seed(RowMeta)?),
                ColumnKey::Column(at) if read & (1 << at) == 0 => {
                    cells[at] = entries.next_value_seed(Cell(columns[at].ty))?;
                    count(self.values, &cells[at])?;
                    read |= 1 << at;
                }
                _ => return stop(),
            }
        }
        if read.count_ones() as usize != columns.len() {
            return stop();
        }
        Ok(RecordRow { cells, meta })
    }
}

/// What a key of a row is.
enum ColumnKey {
    Meta,
    Column(usize),
    Other,
}

fn column_key(columns: &[ColumnDecl], key: &str) -> ColumnKey {
    if key == abi::ROW_META {
        return ColumnKey::Meta;
    }
    columns.iter().position(|column| column.name == key).map_or(ColumnKey::Other, ColumnKey::Column)
}

/// The partitions of a series table: `{"partitions": [...]}`.
struct Series<'d, 'v> {
    decl: &'d TableDecl,
    entries: usize,
    values: &'v mut Values,
}

impl<'de> DeserializeSeed<'de> for Series<'_, '_> {
    type Value = Vec<Partition>;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Vec<Partition>, D::Error> {
        deserializer.deserialize_map(self)
    }
}

impl<'de> Visitor<'de> for Series<'_, '_> {
    type Value = Vec<Partition>;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("a series")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut fields: A) -> Result<Vec<Partition>, A::Error> {
        let mut partitions = None;
        while let Some(is_partitions) = fields.next_key_seed(Key(|key: &str| key == "partitions"))? {
            match is_partitions {
                true if partitions.is_none() => {
                    partitions = Some(fields.next_value_seed(Partitions {
                        decl: self.decl,
                        entries: self.entries,
                        values: &mut *self.values,
                    })?)
                }
                true => return stop(),
                false => {
                    fields.next_value::<IgnoredAny>()?;
                }
            }
        }
        Ok(partitions.unwrap_or_default())
    }
}

struct Partitions<'d, 'v> {
    decl: &'d TableDecl,
    entries: usize,
    values: &'v mut Values,
}

impl<'de> DeserializeSeed<'de> for Partitions<'_, '_> {
    type Value = Vec<Partition>;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Vec<Partition>, D::Error> {
        deserializer.deserialize_seq(self)
    }
}

impl<'de> Visitor<'de> for Partitions<'_, '_> {
    type Value = Vec<Partition>;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("partitions")
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut partitions: A) -> Result<Vec<Partition>, A::Error> {
        let mut read = vec![];
        while let Some(partition) = partitions.next_element_seed(PartitionSeed {
            decl: self.decl,
            entries: self.entries,
            values: &mut *self.values,
        })? {
            read.push(partition);
        }
        Ok(read)
    }
}

/// A partition: its key, its facts, `until` and the causes.
struct PartitionSeed<'d, 'v> {
    decl: &'d TableDecl,
    entries: usize,
    values: &'v mut Values,
}

impl<'de> DeserializeSeed<'de> for PartitionSeed<'_, '_> {
    type Value = Partition;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Partition, D::Error> {
        deserializer.deserialize_map(self)
    }
}

/// The fields of a partition.
enum PartitionField {
    Key,
    Until,
    UntilCause,
    FirstCause,
    Rows,
    Other,
}

impl<'de> Visitor<'de> for PartitionSeed<'_, '_> {
    type Value = Partition;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("a partition")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut fields: A) -> Result<Partition, A::Error> {
        let TableKind::Series(series) = &self.decl.kind else {
            return stop();
        };
        let columns = &self.decl.columns;
        let kinds = fact_columns(self.decl).map_or_else(stop, Ok)?;
        let (mut key, mut until, mut until_cause, mut first_cause, mut facts) = (None, None, None, None, None);
        while let Some(field) = fields.next_key_seed(Key(|key: &str| match key {
            "key" => PartitionField::Key,
            "until" => PartitionField::Until,
            "until_cause" => PartitionField::UntilCause,
            "first_cause" => PartitionField::FirstCause,
            "rows" => PartitionField::Rows,
            _ => PartitionField::Other,
        }))? {
            match field {
                PartitionField::Key if key.is_none() => {
                    key = Some(fields.next_value_seed(PartitionKey {
                        columns,
                        partition: &series.partition,
                        values: &mut *self.values,
                    })?)
                }
                PartitionField::Until if until.is_none() => until = Some(fields.next_value::<String>()?),
                PartitionField::UntilCause if until_cause.is_none() => until_cause = Some(fields.next_value_seed(CauseSeed(self.entries))?),
                PartitionField::FirstCause if first_cause.is_none() => first_cause = Some(fields.next_value_seed(CauseSeed(self.entries))?),
                PartitionField::Rows if facts.is_none() => {
                    facts = Some(fields.next_value_seed(Facts {
                        columns,
                        kinds: &kinds,
                        values: &mut *self.values,
                    })?)
                }
                PartitionField::Other => {
                    fields.next_value::<IgnoredAny>()?;
                }
                _ => return stop(),
            }
        }
        let (Some(key), Some(until), Some(mut facts)) = (key, until, facts) else {
            return stop();
        };
        // the cells of the partition's key, which the facts were read without
        for fact in &mut facts {
            for (at, value) in &key {
                if matches!(kinds[*at], FactColumn::Key) {
                    fact.cells[*at] = value.clone();
                }
            }
        }
        Ok(Partition {
            facts,
            until: parse_date(&until).map_or_else(stop, Ok)?,
            until_cause: until_cause.flatten(),
            first_cause: first_cause.flatten(),
        })
    }
}

/// The key of a partition: a cell for each of the partition's columns, by its index among `columns`.
struct PartitionKey<'d, 'v> {
    columns: &'d [ColumnDecl],
    partition: &'d [String],
    values: &'v mut Values,
}

impl<'de> DeserializeSeed<'de> for PartitionKey<'_, '_> {
    type Value = Vec<(usize, Value)>;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        deserializer.deserialize_map(self)
    }
}

impl<'de> Visitor<'de> for PartitionKey<'_, '_> {
    type Value = Vec<(usize, Value)>;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("a partition's key")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut cells: A) -> Result<Self::Value, A::Error> {
        let (columns, partition) = (self.columns, self.partition);
        let mut key: Vec<(usize, Value)> = vec![];
        while let Some(at) = cells.next_key_seed(Key(|key: &str| {
            partition
                .iter()
                .any(|column| column == key)
                .then(|| columns.iter().position(|column| column.name == key))
                .flatten()
        }))? {
            match at {
                Some(at) if !key.iter().any(|(read, _)| *read == at) => {
                    let value = cells.next_value_seed(Cell(columns[at].ty))?;
                    count(self.values, &value)?;
                    key.push((at, value));
                }
                _ => return stop(),
            }
        }
        // a cell for every partition column
        let declared = columns.iter().filter(|column| partition.contains(&column.name)).count();
        if key.len() != declared {
            return stop();
        }
        Ok(key)
    }
}

/// A cause, `{"seq": 41}` or `null`, of an entry the ledger has.
struct CauseSeed(usize);

impl<'de> DeserializeSeed<'de> for CauseSeed {
    type Value = Option<Cause>;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Option<Cause>, D::Error> {
        deserializer.deserialize_option(self)
    }
}

impl<'de> Visitor<'de> for CauseSeed {
    type Value = Option<Cause>;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("a cause")
    }

    fn visit_none<E: serde::de::Error>(self) -> Result<Option<Cause>, E> {
        Ok(None)
    }

    fn visit_unit<E: serde::de::Error>(self) -> Result<Option<Cause>, E> {
        Ok(None)
    }

    fn visit_some<D: Deserializer<'de>>(self, deserializer: D) -> Result<Option<Cause>, D::Error> {
        deserializer.deserialize_map(self)
    }

    fn visit_map<A: MapAccess<'de>>(self, mut fields: A) -> Result<Option<Cause>, A::Error> {
        let mut seq = None;
        while let Some(is_seq) = fields.next_key_seed(Key(|key: &str| key == "seq"))? {
            match is_seq {
                true if seq.is_none() => seq = Some(fields.next_value::<u32>()?),
                true => return stop(),
                false => {
                    fields.next_value::<IgnoredAny>()?;
                }
            }
        }
        match seq {
            Some(seq) if (seq as usize) < self.0 => Ok(Some(Cause { seq })),
            _ => stop(),
        }
    }
}

/// What each column of the series `decl` is to a fact, as `series_partition` reads one: the period, then a part of it,
/// then a column of the partition's key. `None` for a period that is a part or a key column, which no fact can have.
fn fact_columns(decl: &TableDecl) -> Option<Vec<FactColumn>> {
    let TableKind::Series(series) = &decl.kind else {
        return None;
    };
    let part = |name: &str| series.derive.iter().find(|(column, _)| column == name).map(|(_, part)| *part);
    if part(&series.period).is_some() || series.partition.contains(&series.period) {
        return None;
    }
    Some(
        decl.columns
            .iter()
            .map(|column| match (column.name == series.period, part(&column.name)) {
                (true, _) => FactColumn::Period,
                (_, Some(part)) => FactColumn::Part(part),
                _ if series.partition.contains(&column.name) => FactColumn::Key,
                _ => FactColumn::Cell,
            })
            .collect(),
    )
}

/// The facts of a partition, each read without the cells of the partition's key.
struct Facts<'d, 'v> {
    columns: &'d [ColumnDecl],
    kinds: &'d [FactColumn],
    values: &'v mut Values,
}

impl<'de> DeserializeSeed<'de> for Facts<'_, '_> {
    type Value = Vec<FactRow>;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Vec<FactRow>, D::Error> {
        deserializer.deserialize_seq(self)
    }
}

impl<'de> Visitor<'de> for Facts<'_, '_> {
    type Value = Vec<FactRow>;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("facts")
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut rows: A) -> Result<Vec<FactRow>, A::Error> {
        let mut facts = Vec::with_capacity(rows.size_hint().unwrap_or(0));
        while let Some(fact) = rows.next_element_seed(Fact {
            columns: self.columns,
            kinds: self.kinds,
            values: &mut *self.values,
        })? {
            facts.push(fact);
        }
        facts.shrink_to_fit();
        Ok(facts)
    }
}

/// What a column of a series is to a fact.
#[derive(Clone, Copy)]
enum FactColumn {
    /// the period: the fact's date
    Period,
    /// a part of the period, derived from it
    Part(DatePart),
    /// a column of the partition's key
    Key,
    /// a cell of the fact
    Cell,
}

/// A fact: the period, a cell for each other column but the key's and the derived ones, and maybe its `$meta`.
struct Fact<'d, 'v> {
    columns: &'d [ColumnDecl],
    kinds: &'d [FactColumn],
    values: &'v mut Values,
}

impl<'de> DeserializeSeed<'de> for Fact<'_, '_> {
    type Value = FactRow;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<FactRow, D::Error> {
        deserializer.deserialize_map(self)
    }
}

impl<'de> Visitor<'de> for Fact<'_, '_> {
    type Value = FactRow;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("a fact")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut entries: A) -> Result<FactRow, A::Error> {
        let (columns, kinds) = (self.columns, self.kinds);
        if columns.len() > MAX_COLUMNS {
            return stop();
        }
        // the cells are read in place, each once; those of the key stay NULL until the partition's key is read
        let (mut cells, mut read) = (nulls(columns.len()), 0u128);
        let (mut period, mut meta) = (None, None);
        while let Some(key) = entries.next_key_seed(Key(|key: &str| column_key(columns, key)))? {
            match key {
                ColumnKey::Meta if meta.is_none() => meta = Some(entries.next_value_seed(RowMeta)?),
                ColumnKey::Column(at) => match kinds[at] {
                    FactColumn::Period if period.is_none() => {
                        let text = entries.next_value::<String>()?;
                        period = Some(parse_date(&text).map_or_else(stop, Ok)?);
                    }
                    FactColumn::Cell if read & (1 << at) == 0 => {
                        cells[at] = entries.next_value_seed(Cell(columns[at].ty))?;
                        count(self.values, &cells[at])?;
                        read |= 1 << at;
                    }
                    _ => return stop(),
                },
                _ => return stop(),
            }
        }
        let Some(period) = period else { return stop() };
        for (at, kind) in kinds.iter().enumerate() {
            match kind {
                FactColumn::Period => cells[at] = Value::Date(period),
                FactColumn::Part(DatePart::Year) => cells[at] = Value::Int(i64::from(period.year())),
                FactColumn::Part(DatePart::Month) => cells[at] = Value::Int(i64::from(period.month())),
                FactColumn::Key => {}
                FactColumn::Cell if read & (1 << at) == 0 => return stop(),
                FactColumn::Cell => {}
            }
        }
        Ok(FactRow { period, cells, meta })
    }
}

/// The labels of a label column, each `{"label": "...", "params": {...}}`.
struct Labels<'d, 'v> {
    decl: &'d LabelDecl,
    values: &'v mut Values,
}

impl<'de> DeserializeSeed<'de> for Labels<'_, '_> {
    type Value = Vec<Label>;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Vec<Label>, D::Error> {
        deserializer.deserialize_seq(self)
    }
}

impl<'de> Visitor<'de> for Labels<'_, '_> {
    type Value = Vec<Label>;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("labels")
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut labels: A) -> Result<Vec<Label>, A::Error> {
        let mut read = vec![];
        while let Some(label) = labels.next_element_seed(LabelSeed {
            decl: self.decl,
            values: &mut *self.values,
        })? {
            read.push(label);
        }
        Ok(read)
    }
}

struct LabelSeed<'d, 'v> {
    decl: &'d LabelDecl,
    values: &'v mut Values,
}

impl<'de> DeserializeSeed<'de> for LabelSeed<'_, '_> {
    type Value = Label;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Label, D::Error> {
        deserializer.deserialize_map(self)
    }
}

impl<'de> Visitor<'de> for LabelSeed<'_, '_> {
    type Value = Label;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("a label")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut fields: A) -> Result<Label, A::Error> {
        let declared = &self.decl.params;
        let (mut label, mut params) = (None, None);
        while let Some(field) = fields.next_key_seed(Key(|key: &str| match key {
            "label" => 0,
            "params" => 1,
            _ => 2,
        }))? {
            match field {
                0 if label.is_none() => label = Some(fields.next_value::<String>()?),
                1 if params.is_none() => params = Some(fields.next_value_seed(LabelParams { declared })?),
                2 => {
                    fields.next_value::<IgnoredAny>()?;
                }
                _ => return stop(),
            }
        }
        let Some(label) = label else { return stop() };
        // every declared parameter, bound in order
        let mut bound = params.unwrap_or_default();
        if bound.len() != declared.len() {
            return stop();
        }
        bound.sort_by_key(|(at, _)| *at);
        let mut params = Params::new();
        for ((name, _), (_, value)) in declared.iter().zip(bound) {
            count(self.values, &value)?;
            params = params.bind(name.clone(), value);
        }
        count(self.values, &Value::Str(label.clone()))?;
        Ok(Label { label, params })
    }
}

/// The parameters a label binds, by their index among the declared ones.
struct LabelParams<'d> {
    declared: &'d [(String, DataType)],
}

impl<'de> DeserializeSeed<'de> for LabelParams<'_> {
    type Value = Vec<(usize, Value)>;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        deserializer.deserialize_map(self)
    }
}

impl<'de> Visitor<'de> for LabelParams<'_> {
    type Value = Vec<(usize, Value)>;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("a label's parameters")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut params: A) -> Result<Self::Value, A::Error> {
        let declared = self.declared;
        let mut bound: Vec<(usize, Value)> = vec![];
        while let Some(at) = params.next_key_seed(Key(|key: &str| declared.iter().position(|(name, _)| name == key)))? {
            match at {
                Some(at) if !bound.iter().any(|(read, _)| *read == at) => {
                    let value = params.next_value_seed(Cell(declared[at].1))?;
                    bound.push((at, value));
                }
                _ => return stop(),
            }
        }
        Ok(bound)
    }
}
