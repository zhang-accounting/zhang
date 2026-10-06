//! Metadata functions: `meta` (posting), `entry_meta` (transaction) and `any_meta` (posting,
//! then transaction), as in beanquery, and the zhang extensions `meta_values` and
//! `entry_meta_values`, which return every value of a repeated key. Values are strings.

use crate::table::{Dataset, RowRef};
use crate::value::Value;

fn key_arg<'a>(args: &'a [Value], function: &str) -> Result<&'a str, String> {
    args[0].as_str().ok_or_else(|| format!("{}() expects a metadata key", function))
}

pub(super) fn meta(args: &[Value], data: &Dataset<'_>, row: Option<RowRef<'_, '_>>) -> Result<Value, String> {
    let key = key_arg(args, "meta")?;
    Ok(row.and_then(|row| data.row_meta(row, key)).into())
}

pub(super) fn entry_meta(args: &[Value], data: &Dataset<'_>, row: Option<RowRef<'_, '_>>) -> Result<Value, String> {
    let key = key_arg(args, "entry_meta")?;
    Ok(row.and_then(|row| data.row_entry_meta(row, key)).into())
}

pub(super) fn any_meta(args: &[Value], data: &Dataset<'_>, row: Option<RowRef<'_, '_>>) -> Result<Value, String> {
    let key = key_arg(args, "any_meta")?;
    Ok(row.and_then(|row| data.row_meta(row, key).or_else(|| data.row_entry_meta(row, key))).into())
}

/// `meta_values(key)`: the set of every value of the posting's metadata `key`.
pub(super) fn meta_values(args: &[Value], data: &Dataset<'_>, row: Option<RowRef<'_, '_>>) -> Result<Value, String> {
    let key = key_arg(args, "meta_values")?;
    Ok(Value::Set(
        row.map(|row| data.row_meta_values(row, key)).unwrap_or_default().into_iter().collect(),
    ))
}

/// `entry_meta_values(key)`: the set of every value of the transaction's metadata `key`.
pub(super) fn entry_meta_values(args: &[Value], data: &Dataset<'_>, row: Option<RowRef<'_, '_>>) -> Result<Value, String> {
    let key = key_arg(args, "entry_meta_values")?;
    Ok(Value::Set(
        row.map(|row| data.row_entry_meta_values(row, key)).unwrap_or_default().into_iter().collect(),
    ))
}

#[cfg(test)]
mod tests {
    use super::super::testing::*;
    use super::*;
    use crate::functions::TestContext;

    fn ctx() -> MetaContext {
        let map = |pairs: &[(&str, &str)]| pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        MetaContext {
            inner: TestContext::default(),
            posting: map(&[("shared", "posting"), ("only-posting", "p")]),
            entry: map(&[("shared", "entry"), ("only-entry", "e")]),
        }
    }

    #[test]
    fn meta_reads_posting_metadata() {
        let ctx = ctx();
        assert_eq!(call_with(&ctx, "meta", vec!["shared".into()]), Value::from("posting"));
        assert_eq!(call_with(&ctx, "meta", vec!["only-posting".into()]), Value::from("p"));
        assert_eq!(call_with(&ctx, "meta", vec!["only-entry".into()]), Value::Null);
    }

    #[test]
    fn entry_meta_reads_transaction_metadata() {
        let ctx = ctx();
        assert_eq!(call_with(&ctx, "entry_meta", vec!["shared".into()]), Value::from("entry"));
        assert_eq!(call_with(&ctx, "entry_meta", vec!["only-posting".into()]), Value::Null);
        assert_eq!(call_with(&ctx, "entry_meta", vec!["only-entry".into()]), Value::from("e"));
    }

    #[test]
    fn meta_values_default_to_the_single_value() {
        let ctx = ctx();
        let set = |items: &[&str]| Value::Set(items.iter().map(|it| it.to_string()).collect());
        assert_eq!(call_with(&ctx, "meta_values", vec!["shared".into()]), set(&["posting"]));
        assert_eq!(call_with(&ctx, "entry_meta_values", vec!["only-entry".into()]), set(&["e"]));
        assert_eq!(call_with(&ctx, "meta_values", vec!["missing".into()]), set(&[]));
        assert_eq!(call_with(&ctx, "meta_values", vec![Value::Null]), Value::Null);
    }

    #[test]
    fn any_meta_prefers_the_posting() {
        let ctx = ctx();
        assert_eq!(call_with(&ctx, "any_meta", vec!["shared".into()]), Value::from("posting"));
        assert_eq!(call_with(&ctx, "any_meta", vec!["only-posting".into()]), Value::from("p"));
        assert_eq!(call_with(&ctx, "any_meta", vec!["only-entry".into()]), Value::from("e"));
        assert_eq!(call_with(&ctx, "any_meta", vec!["missing".into()]), Value::Null);
    }
}
