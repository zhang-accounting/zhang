//! Account name functions: `parent` and `leaf` (`root` lives with the registry examples).

use crate::functions::FunctionContext;
use crate::value::Value;

/// beancount `account.parent`: drop the last component. A top-level account has the empty
/// string as its parent; the empty account name has no parent (NULL).
pub(super) fn parent(args: &[Value], _ctx: &dyn FunctionContext) -> Result<Value, String> {
    let account = args[0].as_str().ok_or("parent() expects an account name")?;
    if account.is_empty() {
        return Ok(Value::Null);
    }
    let parent = account.rsplit_once(':').map(|(parent, _)| parent).unwrap_or("");
    Ok(Value::Str(parent.to_owned()))
}

/// beancount `account.leaf`: the last component; NULL for the empty account name.
pub(super) fn leaf(args: &[Value], _ctx: &dyn FunctionContext) -> Result<Value, String> {
    let account = args[0].as_str().ok_or("leaf() expects an account name")?;
    if account.is_empty() {
        return Ok(Value::Null);
    }
    let leaf = account.rsplit(':').next().unwrap_or(account);
    Ok(Value::Str(leaf.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::super::testing::*;
    use super::*;

    #[test]
    fn parent_of_account() {
        assert_eq!(call("parent", vec!["Assets:Bank:Checking".into()]), Value::from("Assets:Bank"));
        assert_eq!(call("parent", vec!["Assets:Bank".into()]), Value::from("Assets"));
        // top-level account: empty string, as in beanquery
        assert_eq!(call("parent", vec!["Assets".into()]), Value::from(""));
        assert_eq!(call("parent", vec!["".into()]), Value::Null);
        assert_eq!(call("parent", vec![Value::Null]), Value::Null);
    }

    #[test]
    fn leaf_of_account() {
        assert_eq!(call("leaf", vec!["Assets:Bank:Checking".into()]), Value::from("Checking"));
        assert_eq!(call("leaf", vec!["Assets".into()]), Value::from("Assets"));
        assert_eq!(call("leaf", vec!["".into()]), Value::Null);
    }
}
