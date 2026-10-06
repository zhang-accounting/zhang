//! Comparison functions, Zhang extensions: `least` and `greatest`, the smaller and the larger of
//! two values of a type that `<` compares (`bool`, `int`, `decimal`, `str` and `date`), such as
//! `least(date_add(date, 6), :to)` to keep a date within a range.
//!
//! Like every scalar function they are NULL when an argument is NULL (PostgreSQL's `LEAST` and
//! `GREATEST` skip NULL arguments instead). beanquery has neither.

use std::cmp::Ordering;

use crate::value::Value;

/// `least(a, b)`: the smaller argument, the first one when they are equal.
pub(super) fn least(args: &[Value]) -> Result<Value, String> {
    Ok(pick(args, Ordering::Less))
}

/// `greatest(a, b)`: the larger argument, the first one when they are equal.
pub(super) fn greatest(args: &[Value]) -> Result<Value, String> {
    Ok(pick(args, Ordering::Greater))
}

/// The second argument when it compares `wanted` to the first, otherwise the first.
fn pick(args: &[Value], wanted: Ordering) -> Value {
    if args[1].sort_cmp(&args[0]) == wanted {
        args[1].clone()
    } else {
        args[0].clone()
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use bigdecimal::BigDecimal;
    use chrono::NaiveDate;

    use super::super::testing::*;
    use crate::functions::TestContext;
    use crate::value::Value;

    fn on(day: &str) -> Value {
        Value::Date(NaiveDate::from_str(day).unwrap())
    }

    fn dec(number: &str) -> Value {
        Value::Decimal(BigDecimal::from_str(number).unwrap())
    }

    #[test]
    fn least_and_greatest_of_dates() {
        assert_eq!(call("least", vec![on("2024-05-05"), on("2024-04-30")]), on("2024-04-30"));
        assert_eq!(call("least", vec![on("2024-04-28"), on("2024-04-30")]), on("2024-04-28"));
        assert_eq!(call("greatest", vec![on("2024-03-25"), on("2024-04-01")]), on("2024-04-01"));
        assert_eq!(call("greatest", vec![on("2024-04-08"), on("2024-04-01")]), on("2024-04-08"));
    }

    #[test]
    fn least_and_greatest_of_numbers_and_text() {
        assert_eq!(call("least", vec![Value::Int(3), Value::Int(-2)]), Value::Int(-2));
        assert_eq!(call("greatest", vec![Value::Int(3), Value::Int(-2)]), Value::Int(3));
        // an int and a decimal compare as numbers, and the result is a decimal
        assert_eq!(call("least", vec![Value::Int(2), dec("1.50")]), dec("1.50"));
        assert_eq!(call("greatest", vec![Value::Int(2), dec("1.50")]), dec("2"));
        assert_eq!(call("least", vec![Value::from("b"), Value::from("a")]), Value::from("a"));
        assert_eq!(call("greatest", vec![Value::Bool(false), Value::Bool(true)]), Value::Bool(true));
    }

    #[test]
    fn equal_arguments_give_the_first_and_null_gives_null() {
        // equal decimals keep the scale of the first argument
        assert_eq!(call_str("least", vec![dec("1.0"), dec("1.00")]), "1.0");
        assert_eq!(call_str("greatest", vec![dec("1.00"), dec("1.0")]), "1.00");
        assert_eq!(call("least", vec![Value::Null, on("2024-04-30")]), Value::Null);
        assert_eq!(call("greatest", vec![on("2024-04-30"), Value::Null]), Value::Null);
    }

    #[test]
    fn arguments_of_different_types_are_rejected() {
        assert!(try_call_with(&TestContext::default(), "least", vec![on("2024-04-30"), Value::Int(1)]).is_err());
        assert!(try_call_with(&TestContext::default(), "greatest", vec![Value::from("a"), Value::Int(1)]).is_err());
    }
}
