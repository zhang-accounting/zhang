//! Metadata functions: `meta` (posting), `entry_meta` (transaction) and `any_meta` (posting,
//! then transaction), as in beanquery, and the zhang extensions `meta_values` and
//! `entry_meta_values`, which return every value of a repeated key. Values are strings.

use crate::functions::FunctionContext;
use crate::value::Value;

fn key_arg<'a>(args: &'a [Value], function: &str) -> Result<&'a str, String> {
    args[0].as_str().ok_or_else(|| format!("{}() expects a metadata key", function))
}

pub(super) fn meta(args: &[Value], ctx: &dyn FunctionContext) -> Result<Value, String> {
    Ok(ctx.posting_meta(key_arg(args, "meta")?).into())
}

pub(super) fn entry_meta(args: &[Value], ctx: &dyn FunctionContext) -> Result<Value, String> {
    Ok(ctx.entry_meta(key_arg(args, "entry_meta")?).into())
}

pub(super) fn any_meta(args: &[Value], ctx: &dyn FunctionContext) -> Result<Value, String> {
    let key = key_arg(args, "any_meta")?;
    Ok(ctx.posting_meta(key).or_else(|| ctx.entry_meta(key)).into())
}

/// `meta_values(key)`: the set of every value of the posting's metadata `key`.
pub(super) fn meta_values(args: &[Value], ctx: &dyn FunctionContext) -> Result<Value, String> {
    Ok(Value::Set(ctx.posting_meta_values(key_arg(args, "meta_values")?).into_iter().collect()))
}

/// `entry_meta_values(key)`: the set of every value of the transaction's metadata `key`.
pub(super) fn entry_meta_values(args: &[Value], ctx: &dyn FunctionContext) -> Result<Value, String> {
    Ok(Value::Set(ctx.entry_meta_values(key_arg(args, "entry_meta_values")?).into_iter().collect()))
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
