//! `PIVOT BY a, b`: reshape a finished result into a table, as beanquery does.
//!
//! It runs last, on the rows LIMIT leaves (after HAVING, ORDER BY and DISTINCT). With the
//! visible targets `a`, `b` and the others `o1, o2, ...`:
//!
//! - There is one row per distinct value of `a`, sorted ascending ([`Value::sort_cmp`], so
//!   NULL first), whatever the ORDER BY. Its first cell is that value.
//! - There is one group of columns per distinct value of `b`, sorted ascending, holding the
//!   other targets in their SELECT order. A column is named after the value of `b` when
//!   there is a single other target, else `<value>/<target>`. The first column is named
//!   `<a>/<b>`. Each column keeps the type of its target.
//! - A cell whose `(a, b)` pair is missing from the result is NULL. When several rows share a
//!   pair (`b` is not the only other GROUP BY key), the last one in result order wins.
//! - A value is named like Python's `str()`, as beanquery does: `2016`, `2016-01-31`,
//!   `12.50`, `True`, `100 USD`. NULL, which beanquery cannot sort next to other values, is
//!   `NULL`.

use indexmap::IndexSet;

use crate::compiler::{Pivot, Plan};
use crate::error::LocatedError;
use crate::executor::{row_weight, weight, Budget};
use crate::value::Value;
use crate::ColumnInfo;

/// Pivot the visible rows of `plan`'s result, within the `budget`: the pivoted cells are
/// charged before they are allocated (a pivot holds rows × columns cells, most of them
/// possibly NULL), then the rows they replace are released.
pub(crate) fn pivot(plan: &Plan, spec: Pivot, rows: Vec<Vec<Value>>, budget: &mut Budget) -> Result<(Vec<ColumnInfo>, Vec<Vec<Value>>), LocatedError> {
    let targets = &plan.targets[..plan.visible];
    let others = (0..targets.len()).filter(|idx| *idx != spec.rows && *idx != spec.columns).collect::<Vec<_>>();

    let mut row_keys = rows.iter().map(|row| row[spec.rows].clone()).collect::<IndexSet<_>>();
    let mut column_keys = rows.iter().map(|row| row[spec.columns].clone()).collect::<IndexSet<_>>();
    row_keys.sort_by(Value::sort_cmp);
    column_keys.sort_by(Value::sort_cmp);

    let width = 1 + column_keys.len() * others.len();
    // every cell weighs at least one value: refuse a pivot too large before building it
    budget.charge((row_keys.len() as u64).saturating_mul(width as u64))?;
    let mut table = Vec::with_capacity(row_keys.len());
    for key in &row_keys {
        budget.change(1, weight(key))?;
        let mut row = vec![Value::Null; width];
        row[0] = key.clone();
        table.push(row);
    }
    let released = rows.iter().map(|row| row_weight(row)).sum::<u64>();
    for mut row in rows {
        let r = row_keys.get_index_of(&row[spec.rows]).expect("a row key");
        let c = column_keys.get_index_of(&row[spec.columns]).expect("a column key");
        for (k, other) in others.iter().enumerate() {
            let cell = &mut table[r][1 + c * others.len() + k];
            let value = std::mem::replace(&mut row[*other], Value::Null);
            budget.change(weight(cell), weight(&value))?;
            *cell = value;
        }
    }
    budget.release(released);

    let mut columns = Vec::with_capacity(width);
    columns.push(ColumnInfo {
        name: format!("{}/{}", targets[spec.rows].name, targets[spec.columns].name),
        ty: targets[spec.rows].ty,
    });
    for key in &column_keys {
        let label = label(key);
        for other in &others {
            columns.push(ColumnInfo {
                name: if others.len() == 1 {
                    label.clone()
                } else {
                    format!("{}/{}", label, targets[*other].name)
                },
                ty: targets[*other].ty,
            });
        }
    }
    Ok((columns, table))
}

/// The name of the columns of a value of the second PIVOT BY column: Python's `str()` of the
/// value, as in beanquery (`True`, not `TRUE`), and `NULL` for NULL.
fn label(value: &Value) -> String {
    match value {
        Value::Bool(true) => "True".to_owned(),
        Value::Bool(false) => "False".to_owned(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Query;

    /// (name, type) of the columns, and the rows
    type Pivoted = (Vec<(String, String)>, Vec<Vec<Value>>);

    fn run(sql: &str, rows: Vec<Vec<Value>>, limit: Option<u64>) -> Result<Pivoted, LocatedError> {
        let query = Query::compile(sql).unwrap();
        let spec = query.plan.pivot.expect("a pivot");
        let mut budget = Budget::new(limit);
        let (columns, rows) = pivot(&query.plan, spec, rows, &mut budget)?;
        Ok((columns.into_iter().map(|it| (it.name, it.ty.to_string())).collect(), rows))
    }

    fn v(value: impl Into<Value>) -> Value {
        value.into()
    }

    const SQL: &str = "SELECT year, account, count(*) AS n GROUP BY 1, 2 PIVOT BY year, account";

    #[test]
    fn rows_and_columns_are_sorted_and_missing_cells_are_null() {
        let rows = vec![vec![v(2017), v("B"), v(1)], vec![v(2016), v("B"), v(2)], vec![v(2016), v("A"), v(3)]];
        let (columns, rows) = run(SQL, rows, None).unwrap();
        assert_eq!(
            columns,
            vec![
                ("year/account".to_owned(), "int".to_owned()),
                ("A".to_owned(), "int".to_owned()),
                ("B".to_owned(), "int".to_owned())
            ]
        );
        assert_eq!(rows, vec![vec![v(2016), v(3), v(2)], vec![v(2017), Value::Null, v(1)]]);
    }

    #[test]
    fn several_other_targets_are_named_after_the_value_and_the_target() {
        let sql = "SELECT account, year, count(*) AS n, sum(number) AS total GROUP BY 1, 2 PIVOT BY 1, 2";
        let rows = vec![vec![v("A"), v(2016), v(1), Value::Null], vec![v("A"), v(2015), v(2), v(5)]];
        let (columns, rows) = run(sql, rows, None).unwrap();
        let names = columns.iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>();
        assert_eq!(names, vec!["account/year", "2015/n", "2015/total", "2016/n", "2016/total"]);
        assert_eq!(columns[2].1, "decimal");
        assert_eq!(rows, vec![vec![v("A"), v(2), v(5), v(1), Value::Null]]);
    }

    #[test]
    fn the_last_row_of_a_pair_wins_and_null_sorts_first() {
        let sql = "SELECT payee, payee IS NULL AS np, month, count(*) AS n GROUP BY 1, 2, 3 PIVOT BY payee, np";
        let rows = vec![
            vec![v("x"), v(false), v(1), v(10)],
            vec![Value::Null, v(true), v(1), v(20)],
            vec![v("x"), v(false), v(2), v(30)],
        ];
        let (columns, rows) = run(sql, rows, None).unwrap();
        let names = columns.iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>();
        assert_eq!(names, vec!["payee/np", "False/month", "False/n", "True/month", "True/n"]);
        assert_eq!(
            rows,
            vec![
                vec![Value::Null, Value::Null, Value::Null, v(1), v(20)],
                vec![v("x"), v(2), v(30), Value::Null, Value::Null],
            ]
        );
        assert_eq!(label(&Value::Null), "NULL");
    }

    #[test]
    fn no_other_target_leaves_only_the_first_column() {
        let sql = "SELECT year, account GROUP BY 1, 2 PIVOT BY year, account";
        let (columns, rows) = run(sql, vec![vec![v(2016), v("A")], vec![v(2016), v("B")]], None).unwrap();
        assert_eq!(columns, vec![("year/account".to_owned(), "int".to_owned())]);
        assert_eq!(rows, vec![vec![v(2016)]]);
        let (columns, rows) = run(SQL, vec![], None).unwrap();
        assert_eq!(columns.len(), 1);
        assert!(rows.is_empty());
    }

    #[test]
    fn a_pivot_too_large_for_the_budget_fails_before_it_is_built() {
        // 300 distinct years × 300 distinct accounts, one row each: 300 rows, 90,301 cells
        let rows = (0..300).map(|idx| vec![v(idx as i64), v(format!("A{}", idx)), v(1)]).collect::<Vec<_>>();
        let err = run(SQL, rows.clone(), Some(50_000)).err().unwrap();
        assert_eq!(err.kind, crate::QueryErrorKind::TooLarge);
        let (_, pivoted) = run(SQL, rows, Some(200_000)).unwrap();
        assert_eq!(pivoted.len(), 300);
        assert_eq!(pivoted[0].len(), 301);
    }
}
