//! Account name functions: `parent`, `leaf` and `account_sortkey` (`root` lives with the
//! registry examples).

use std::str::FromStr;

use zhang_ast::AccountType;

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

/// beanquery `account_sortkey`: `'<type index>-<account>'`, where the index follows the
/// account types in the order Assets, Liabilities, Equity, Income, Expenses, so the keys sort
/// accounts by type, then by name (BALANCES orders by it). A name whose first component is
/// not an account type gets index 5 and sorts after them (beanquery raises an error).
pub(super) fn account_sortkey(args: &[Value], _ctx: &dyn FunctionContext) -> Result<Value, String> {
    let account = args[0].as_str().ok_or("account_sortkey() expects an account name")?;
    let root = account.split(':').next().unwrap_or_default();
    let index = match AccountType::from_str(root) {
        Ok(AccountType::Assets) => 0,
        Ok(AccountType::Liabilities) => 1,
        Ok(AccountType::Equity) => 2,
        Ok(AccountType::Income) => 3,
        Ok(AccountType::Expenses) => 4,
        Err(_) => 5,
    };
    Ok(Value::Str(format!("{}-{}", index, account)))
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

    #[test]
    fn account_sortkey_orders_by_type_then_name() {
        assert_eq!(call("account_sortkey", vec!["Assets:Bank".into()]), Value::from("0-Assets:Bank"));
        assert_eq!(call("account_sortkey", vec!["Liabilities".into()]), Value::from("1-Liabilities"));
        assert_eq!(call("account_sortkey", vec!["Equity:Opening".into()]), Value::from("2-Equity:Opening"));
        assert_eq!(call("account_sortkey", vec!["Income:Salary".into()]), Value::from("3-Income:Salary"));
        assert_eq!(call("account_sortkey", vec!["Expenses:Food".into()]), Value::from("4-Expenses:Food"));
        // not an account type (beanquery raises an error): after the five types
        assert_eq!(call("account_sortkey", vec!["assets:Bank".into()]), Value::from("5-assets:Bank"));
        assert_eq!(call("account_sortkey", vec!["".into()]), Value::from("5-"));
        assert_eq!(call("account_sortkey", vec![Value::Null]), Value::Null);
        let mut accounts =
            ["Expenses:A", "Assets:Z", "Income:B", "Equity:C", "Liabilities:D", "Assets:B"].map(|it| match call("account_sortkey", vec![it.into()]) {
                Value::Str(key) => key,
                other => panic!("{:?}", other),
            });
        accounts.sort();
        assert_eq!(
            accounts,
            ["0-Assets:B", "0-Assets:Z", "1-Liabilities:D", "2-Equity:C", "3-Income:B", "4-Expenses:A"]
        );
    }
}
