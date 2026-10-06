//! Search functions, zhang extensions for keyword search: `icontains`, `any_icontains`,
//! `intersects` and the `set` constructor.
//!
//! Case-insensitive means compared after Unicode lower-casing ([`str::to_lowercase`]) of both
//! sides. With a constant needle (a literal, or a parameter of the execution) the optimizer
//! prepares the lower-cased needle once ([`crate::compiler::StrTestKind`]); these
//! implementations are the reference it must agree with.

use crate::value::Value;

/// `icontains(text, needle)`: whether `text` contains `needle`, ignoring case.
pub(super) fn icontains(args: &[Value]) -> Result<Value, String> {
    let text = args[0].as_str().ok_or("icontains() expects a string")?;
    let needle = args[1].as_str().ok_or("icontains() expects a string needle")?;
    Ok(Value::Bool(text.to_lowercase().contains(&needle.to_lowercase())))
}

/// `any_icontains(set, needle)`: whether an element of `set` contains `needle`, ignoring case.
pub(super) fn any_icontains(args: &[Value]) -> Result<Value, String> {
    let set = args[0].as_set().ok_or("any_icontains() expects a set")?;
    let needle = args[1].as_str().ok_or("any_icontains() expects a string needle")?.to_lowercase();
    Ok(Value::Bool(set.iter().any(|item| item.to_lowercase().contains(&needle))))
}

/// `intersects(a, b)`: whether the two sets share an element.
pub(super) fn intersects(args: &[Value]) -> Result<Value, String> {
    let a = args[0].as_set().ok_or("intersects() expects sets")?;
    let b = args[1].as_set().ok_or("intersects() expects sets")?;
    let (small, large) = if a.len() <= b.len() { (a, b) } else { (b, a) };
    Ok(Value::Bool(small.iter().any(|item| large.contains(item))))
}

/// `set(a, b, ...)`: the set of the given strings.
pub(super) fn set(args: &[Value]) -> Result<Value, String> {
    args.iter()
        .map(|arg| arg.as_str().map(str::to_owned).ok_or_else(|| "set() expects strings".to_owned()))
        .collect::<Result<_, _>>()
        .map(Value::Set)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::super::testing::*;
    use super::*;
    use crate::functions::TestContext;

    fn set(items: &[&str]) -> Value {
        Value::Set(items.iter().map(|it| it.to_string()).collect::<BTreeSet<_>>())
    }

    #[test]
    fn icontains_ignores_case_with_unicode_lower_case() {
        assert_eq!(call("icontains", vec!["Whole Foods Market".into(), "FOODS".into()]), Value::Bool(true));
        assert_eq!(call("icontains", vec!["Whole Foods".into(), "market".into()]), Value::Bool(false));
        assert_eq!(call("icontains", vec!["ÉCOLE".into(), "éco".into()]), Value::Bool(true));
        assert_eq!(call("icontains", vec!["午餐 Lunch".into(), "午餐".into()]), Value::Bool(true));
        // the empty needle is in every string
        assert_eq!(call("icontains", vec!["".into(), "".into()]), Value::Bool(true));
        assert_eq!(call("icontains", vec![Value::Null, "x".into()]), Value::Null);
        assert_eq!(call("icontains", vec!["x".into(), Value::Null]), Value::Null);
    }

    #[test]
    fn any_icontains_looks_into_every_element() {
        assert_eq!(call("any_icontains", vec![set(&["trip-Boston", "food"]), "BOSTON".into()]), Value::Bool(true));
        assert_eq!(call("any_icontains", vec![set(&["trip-boston"]), "chicago".into()]), Value::Bool(false));
        assert_eq!(call("any_icontains", vec![set(&[]), "".into()]), Value::Bool(false));
        assert_eq!(call("any_icontains", vec![Value::Null, "x".into()]), Value::Null);
    }

    #[test]
    fn intersects_finds_a_shared_element() {
        assert_eq!(call("intersects", vec![set(&["a", "b"]), set(&["c", "b"])]), Value::Bool(true));
        assert_eq!(call("intersects", vec![set(&["a", "b"]), set(&["A"])]), Value::Bool(false));
        assert_eq!(call("intersects", vec![set(&[]), set(&[])]), Value::Bool(false));
        assert_eq!(call("intersects", vec![set(&["a"]), Value::Null]), Value::Null);
    }

    #[test]
    fn set_builds_a_set_of_any_number_of_strings() {
        assert_eq!(call("set", vec![]), set(&[]));
        assert_eq!(call("set", vec!["b".into(), "a".into(), "b".into()]), set(&["a", "b"]));
        assert_eq!(call("set", vec!["it's \\ \"x\"".into()]), set(&["it's \\ \"x\""]));
        assert_eq!(call("set", vec!["a".into(), Value::Null]), Value::Null);
        let error = try_call_with(&TestContext::default(), "set", vec!["a".into(), Value::Int(1)]).unwrap_err();
        assert_eq!(error, "no overload of set(str, int); available: set(str, ...) -> set");
    }
}
