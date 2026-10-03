//! Export of query results: beanquery-style *numberify* and CSV.
//!
//! [`numberify`] makes a result spreadsheet-friendly. It replaces every `amount`,
//! `position` and `inventory` column with one `decimal` column per currency, so each cell
//! is a plain number. [`to_csv`] renders the numberified result as RFC 4180 CSV.
//!
//! The module is pure: it does no I/O and reads no clock, so it also builds for
//! `wasm32-unknown-unknown`. Numbers stay exact [`BigDecimal`]s and are never rounded.
//!
//! # Numberify rules
//!
//! The rules follow beanquery 0.2.0 (`bean-query -m`), as observed on the shared ledger:
//!
//! - A column `name` of type `amount`, `position` or `inventory` becomes one `decimal`
//!   column `name (CUR)` per currency that occurs in it. Other columns pass through
//!   unchanged. A column in which no currency occurs disappears.
//! - Currencies are ordered by the number of cells they occur in, most frequent first.
//!   Ties are broken by currency name in **descending** order (`VBMPX`, `USD`, `RGAGX`).
//! - `amount`: the number when the currency matches, otherwise NULL. A zero amount is
//!   treated like NULL: it renders as NULL and does not count towards the currencies
//!   (beancount's zero `Amount` is falsy).
//! - `position`: the units number when the units currency matches, otherwise NULL. The cost
//!   is dropped. Zero units still render as `0`.
//! - `inventory`: the sum of the units of every lot in that currency, ignoring costs. A sum
//!   of zero renders as NULL. An inventory counts once for each currency it holds.
//! - NULL cells of these columns are NULL in every split column.
//!
//! # CSV rendering
//!
//! - The first record is the header of column names. Every record, the last one included,
//!   ends with CRLF.
//! - NULL is the empty field. Booleans are `TRUE`/`FALSE`, dates `YYYY-MM-DD`, sets
//!   their sorted elements joined by `,`, intervals like `1 year 2 months`, and metadata
//!   (`metas`) its `key: value` pairs joined by `; `. Decimals keep their exact digits and scale and
//!   never use exponent notation (`4.00`, `-0.03`, `1000`).
//! - A field that contains `,`, `"`, CR or LF is quoted, with `"` doubled. A record with a
//!   single empty field is written as `""`, so that it is not a blank line.
//!
//! # Differences from `bean-query -f csv -m`
//!
//! - beanquery reuses its text-table renderers for CSV, so numbers carry alignment padding
//!   (`" 600.00"`). Fields here are never padded.
//! - beanquery's command line quantizes numberified numbers to the ledger's display
//!   precision of each currency (`4.088 × 88.07` is written `360.03`). Here they stay
//!   exact (`360.03016`).
//! - beanquery prints decimals with a positive exponent in scientific notation (`1E+3`).
//!   Here they are written out (`1000`).

use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap};

use bigdecimal::{BigDecimal, Zero};

use crate::decimal::to_plain_string;
use crate::value::{DataType, Value};
use crate::{ColumnInfo, QueryResult};

/// A query result whose `amount`, `position` and `inventory` columns are split into one
/// `decimal` column per currency (see the [module docs](self)). It never holds amounts,
/// positions or inventories.
#[derive(Debug, Clone, PartialEq)]
pub struct NumberifiedResult {
    pub columns: Vec<ColumnInfo>,
    pub rows: Vec<Vec<Value>>,
}

impl NumberifiedResult {
    /// Render the result as RFC 4180 CSV with a header record (see the
    /// [module docs](self)).
    pub fn to_csv(&self) -> String {
        let mut out = String::new();
        write_record(&mut out, self.columns.iter().map(|column| Cow::Borrowed(column.name.as_str())));
        for row in &self.rows {
            write_record(&mut out, row.iter().map(csv_field));
        }
        out
    }
}

/// Numberify a result and render it as RFC 4180 CSV: `numberify(result).to_csv()`.
pub fn to_csv(result: &QueryResult) -> String {
    numberify(result).to_csv()
}

/// The numbers of one cell of a split column, per currency. A currency that is present
/// counts towards the column's currencies; `None` renders as NULL.
type CellNumbers<'a> = BTreeMap<&'a str, Option<BigDecimal>>;

/// Split every `amount`, `position` and `inventory` column into one `decimal` column per
/// currency, following beanquery's numberify rules (see the [module docs](self)).
pub fn numberify(result: &QueryResult) -> NumberifiedResult {
    let mut columns = Vec::with_capacity(result.columns.len());
    let mut rows: Vec<Vec<Value>> = result.rows.iter().map(|_| Vec::with_capacity(result.columns.len())).collect();
    for (index, column) in result.columns.iter().enumerate() {
        let cell = |row: &[Value]| row.get(index).cloned().unwrap_or(Value::Null);
        let Some(cells) = split_cells(column.ty, result.rows.iter().map(|row| row.get(index))) else {
            columns.push(column.clone());
            for (target, row) in rows.iter_mut().zip(&result.rows) {
                target.push(cell(row));
            }
            continue;
        };
        for currency in currency_order(&cells) {
            columns.push(ColumnInfo {
                name: format!("{} ({})", column.name, currency),
                ty: DataType::Decimal,
            });
            for (target, numbers) in rows.iter_mut().zip(&cells) {
                let number = numbers.get(currency).cloned().flatten();
                target.push(number.map(Value::Decimal).unwrap_or(Value::Null));
            }
        }
    }
    NumberifiedResult { columns, rows }
}

/// The per-currency numbers of every cell of a column, or `None` when the column is not
/// split.
fn split_cells<'a>(ty: DataType, cells: impl Iterator<Item = Option<&'a Value>>) -> Option<Vec<CellNumbers<'a>>> {
    if !matches!(ty, DataType::Amount | DataType::Position | DataType::Inventory) {
        return None;
    }
    Some(cells.map(|cell| cell.map(cell_numbers).unwrap_or_default()).collect())
}

fn cell_numbers(value: &Value) -> CellNumbers<'_> {
    let mut numbers = CellNumbers::new();
    match value {
        // a zero amount counts as absent, like beancount's falsy zero `Amount`
        Value::Amount(amount) if !amount.number.is_zero() => {
            numbers.insert(amount.commodity.as_str(), Some(amount.number.clone()));
        }
        Value::Position(position) => {
            numbers.insert(position.units.commodity.as_str(), Some(position.units.number.clone()));
        }
        Value::Inventory(inventory) => {
            let mut totals: BTreeMap<&str, BigDecimal> = BTreeMap::new();
            for (currency, number) in inventory.lot_units() {
                *totals.entry(currency).or_default() += number;
            }
            // the currency still counts when its lots cancel out, but the cell is NULL
            numbers.extend(totals.into_iter().map(|(currency, total)| (currency, Some(total).filter(|it| !it.is_zero()))));
        }
        _ => {}
    }
    numbers
}

/// The currencies of a split column: most frequent first, ties by name descending.
fn currency_order<'a>(cells: &[CellNumbers<'a>]) -> Vec<&'a str> {
    let mut counts: HashMap<&'a str, usize> = HashMap::new();
    for numbers in cells {
        for currency in numbers.keys() {
            *counts.entry(currency).or_default() += 1;
        }
    }
    let mut currencies = counts.into_iter().collect::<Vec<_>>();
    currencies.sort_unstable_by(|(a_currency, a_count), (b_currency, b_count)| b_count.cmp(a_count).then_with(|| b_currency.cmp(a_currency)));
    currencies.into_iter().map(|(currency, _)| currency).collect()
}

/// The CSV text of one cell, before quoting.
pub fn csv_field(value: &Value) -> Cow<'_, str> {
    match value {
        Value::Null => Cow::Borrowed(""),
        Value::Bool(it) => Cow::Borrowed(if *it { "TRUE" } else { "FALSE" }),
        Value::Int(it) => Cow::Owned(it.to_string()),
        Value::Decimal(it) => Cow::Owned(to_plain_string(it)),
        Value::Str(it) => Cow::Borrowed(it),
        Value::Date(it) => Cow::Owned(it.format("%Y-%m-%d").to_string()),
        Value::Set(it) => Cow::Owned(it.iter().map(String::as_str).collect::<Vec<_>>().join(",")),
        // `1 year 2 months`, and `key: value` pairs joined by `; `
        Value::Interval(_) | Value::Metas(_) => Cow::Owned(value.to_string()),
        // not present in a numberified result; rendered like `str()` for completeness
        Value::Amount(_) | Value::Position(_) | Value::Inventory(_) => Cow::Owned(value.to_string()),
    }
}

/// Append one CSV record (RFC 4180) terminated by CRLF.
fn write_record<'a>(out: &mut String, fields: impl ExactSizeIterator<Item = Cow<'a, str>>) {
    let single = fields.len() == 1;
    for (idx, field) in fields.enumerate() {
        if idx > 0 {
            out.push(',');
        }
        // a lone empty field is quoted so that the record is not a blank line
        if field.contains([',', '"', '\r', '\n']) || (single && field.is_empty()) {
            out.push('"');
            out.push_str(&field.replace('"', "\"\""));
            out.push('"');
        } else {
            out.push_str(&field);
        }
    }
    out.push_str("\r\n");
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::str::FromStr;

    use chrono::NaiveDate;

    use super::*;
    use crate::{Amount, Cost, Inventory, Position};

    fn d(number: &str) -> BigDecimal {
        BigDecimal::from_str(number).unwrap()
    }

    fn amount(number: &str, currency: &str) -> Amount {
        Amount::new(d(number), currency)
    }

    fn lot(number: &str, currency: &str, cost: &str) -> Position {
        Position::new(
            amount(number, currency),
            Some(Cost {
                number: d(cost),
                currency: "USD".to_owned(),
                date: NaiveDate::from_ymd_opt(2024, 1, 1),
                label: None,
            }),
        )
    }

    fn result(columns: &[(&str, DataType)], rows: Vec<Vec<Value>>) -> QueryResult {
        QueryResult {
            columns: columns
                .iter()
                .map(|(name, ty)| ColumnInfo {
                    name: (*name).to_owned(),
                    ty: *ty,
                })
                .collect(),
            rows,
            total: None,
        }
    }

    fn names(result: &NumberifiedResult) -> Vec<&str> {
        result.columns.iter().map(|column| column.name.as_str()).collect()
    }

    fn dec(number: &str) -> Value {
        Value::Decimal(d(number))
    }

    #[test]
    fn amount_columns_split_per_currency_by_frequency() {
        let input = result(
            &[("a", DataType::Amount)],
            vec![
                vec![Value::Amount(amount("1.50", "EUR"))],
                vec![Value::Amount(amount("2", "USD"))],
                vec![Value::Amount(amount("-3.25", "USD"))],
                vec![Value::Null],
            ],
        );
        let output = numberify(&input);
        assert_eq!(names(&output), ["a (USD)", "a (EUR)"]);
        assert!(output.columns.iter().all(|column| column.ty == DataType::Decimal));
        assert_eq!(
            output.rows,
            vec![
                vec![Value::Null, dec("1.50")],
                vec![dec("2"), Value::Null],
                vec![dec("-3.25"), Value::Null],
                vec![Value::Null, Value::Null],
            ]
        );
    }

    #[test]
    fn ties_are_ordered_by_currency_name_descending() {
        let mut inventory = Inventory::new();
        inventory.add_amount(&amount("1", "RGAGX"));
        inventory.add_amount(&amount("2", "USD"));
        inventory.add_amount(&amount("3", "VBMPX"));
        let output = numberify(&result(&[("p", DataType::Inventory)], vec![vec![Value::Inventory(inventory)]]));
        assert_eq!(names(&output), ["p (VBMPX)", "p (USD)", "p (RGAGX)"]);
        assert_eq!(output.rows, vec![vec![dec("3"), dec("2"), dec("1")]]);
    }

    #[test]
    fn zero_amounts_are_null_and_do_not_count() {
        let input = result(
            &[("a", DataType::Amount)],
            vec![vec![Value::Amount(amount("0.00", "USD"))], vec![Value::Amount(amount("5.0", "EUR"))]],
        );
        let output = numberify(&input);
        assert_eq!(names(&output), ["a (EUR)"]);
        assert_eq!(output.rows, vec![vec![Value::Null], vec![dec("5.0")]]);
    }

    #[test]
    fn positions_keep_units_and_drop_the_cost() {
        let input = result(
            &[("p", DataType::Position)],
            vec![
                vec![Value::Position(lot("4", "AAPL", "1"))],
                vec![Value::Position(Position::new(amount("0", "EUR"), None))],
                vec![Value::Null],
            ],
        );
        let output = numberify(&input);
        // the cost currency (USD) gets no column
        assert_eq!(names(&output), ["p (EUR)", "p (AAPL)"]);
        assert_eq!(
            output.rows,
            vec![vec![Value::Null, dec("4")], vec![dec("0"), Value::Null], vec![Value::Null, Value::Null]]
        );
    }

    #[test]
    fn inventories_sum_units_across_lots_and_null_a_zero_sum() {
        let mut held = Inventory::new();
        held.add_amount(&amount("1.50", "USD"));
        held.add_position(&lot("2", "AAPL", "10"));
        held.add_position(&lot("3", "AAPL", "12"));
        let mut cancelled = Inventory::new();
        cancelled.add_position(&lot("1", "X", "10"));
        cancelled.add_position(&lot("-1", "X", "20"));
        let input = result(
            &[("i", DataType::Inventory)],
            vec![
                vec![Value::Inventory(held)],
                vec![Value::Inventory(cancelled)],
                vec![Value::Inventory(Inventory::new())],
            ],
        );
        let output = numberify(&input);
        // X still counts: its lots exist, they only cancel out
        assert_eq!(names(&output), ["i (X)", "i (USD)", "i (AAPL)"]);
        assert_eq!(
            output.rows,
            vec![
                vec![Value::Null, dec("1.50"), dec("5")],
                vec![Value::Null, Value::Null, Value::Null],
                vec![Value::Null, Value::Null, Value::Null],
            ]
        );
    }

    #[test]
    fn columns_without_currencies_disappear_and_others_pass_through() {
        let input = result(
            &[("n", DataType::Int), ("p", DataType::Inventory), ("s", DataType::Str)],
            vec![vec![Value::Int(1), Value::Inventory(Inventory::new()), Value::from("x")]],
        );
        let output = numberify(&input);
        assert_eq!(names(&output), ["n", "s"]);
        assert_eq!(output.columns[0].ty, DataType::Int);
        assert_eq!(output.rows, vec![vec![Value::Int(1), Value::from("x")]]);
        // no rows: split columns have no currencies at all
        let empty = numberify(&result(&[("p", DataType::Inventory)], vec![]));
        assert!(empty.columns.is_empty());
        assert_eq!(empty.to_csv(), "\r\n");
    }

    #[test]
    fn csv_renders_scalars_like_beanquery() {
        let date = NaiveDate::from_ymd_opt(2016, 11, 18).unwrap();
        let tags = BTreeSet::from(["trip".to_owned(), "a-tag".to_owned()]);
        let input = result(
            &[
                ("date", DataType::Date),
                ("tags", DataType::Set),
                ("empty", DataType::Set),
                ("t", DataType::Bool),
                ("f", DataType::Bool),
                ("n", DataType::Null),
                ("i", DataType::Int),
                ("d", DataType::Decimal),
                ("big", DataType::Decimal),
            ],
            vec![vec![
                Value::Date(date),
                Value::Set(tags),
                Value::Set(BTreeSet::new()),
                Value::Bool(true),
                Value::Bool(false),
                Value::Null,
                Value::Int(-7),
                dec("-0.030"),
                dec("1E+3"),
            ]],
        );
        assert_eq!(
            to_csv(&input),
            "date,tags,empty,t,f,n,i,d,big\r\n2016-11-18,\"a-tag,trip\",,TRUE,FALSE,,-7,-0.030,1000\r\n"
        );
    }

    #[test]
    fn csv_quotes_only_fields_that_need_it() {
        let input = result(
            &[("s", DataType::Str), ("name, with comma", DataType::Str)],
            vec![
                vec![Value::from("a,b \"c\""), Value::from("line\nbreak")],
                vec![Value::from(" spaced "), Value::from("cr\rhere")],
                vec![Value::from("中文"), Value::Null],
            ],
        );
        assert_eq!(
            to_csv(&input),
            "s,\"name, with comma\"\r\n\"a,b \"\"c\"\"\",\"line\nbreak\"\r\n spaced ,\"cr\rhere\"\r\n中文,\r\n"
        );
    }

    #[test]
    fn csv_quotes_a_lone_empty_field() {
        let input = result(&[("n", DataType::Str)], vec![vec![Value::Null], vec![Value::from("")], vec![Value::from("x")]]);
        assert_eq!(to_csv(&input), "n\r\n\"\"\r\n\"\"\r\nx\r\n");
    }

    #[test]
    fn csv_of_a_numberified_inventory() {
        let mut inventory = Inventory::new();
        inventory.add_position(&lot("2", "AAPL", "10"));
        inventory.add_amount(&amount("-12.50", "USD"));
        let input = result(
            &[("account", DataType::Str), ("balance", DataType::Inventory)],
            vec![
                vec![Value::from("Assets:Broker"), Value::Inventory(inventory)],
                vec![Value::from("Assets:Cash"), Value::Null],
            ],
        );
        assert_eq!(
            to_csv(&input),
            "account,balance (USD),balance (AAPL)\r\nAssets:Broker,-12.50,2\r\nAssets:Cash,,\r\n"
        );
    }
}
