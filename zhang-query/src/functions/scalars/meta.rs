//! Metadata functions: `meta` (posting), `entry_meta` (transaction) and `any_meta` (posting,
//! then transaction), as in beanquery. Values are returned as strings.

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
    fn any_meta_prefers_the_posting() {
        let ctx = ctx();
        assert_eq!(call_with(&ctx, "any_meta", vec!["shared".into()]), Value::from("posting"));
        assert_eq!(call_with(&ctx, "any_meta", vec!["only-posting".into()]), Value::from("p"));
        assert_eq!(call_with(&ctx, "any_meta", vec!["only-entry".into()]), Value::from("e"));
        assert_eq!(call_with(&ctx, "any_meta", vec!["missing".into()]), Value::Null);
    }
}
