//! `str` and `length`.

use crate::functions::FunctionContext;
use crate::value::{position_sort_cmp, Inventory, Value};

/// beancount's `Inventory.__str__`: the positions in position sort order (common currencies
/// first), comma separated, in parentheses.
pub(crate) fn inventory_to_string(inventory: &Inventory) -> String {
    let mut positions = inventory.positions().collect::<Vec<_>>();
    positions.sort_by(position_sort_cmp);
    format!("({})", positions.iter().map(ToString::to_string).collect::<Vec<_>>().join(", "))
}

/// beanquery `str`: `TRUE`/`FALSE` for booleans and beancount's text forms for amounts
/// (`10.00 USD`), positions (`10 AAPL {100 USD, 2024-01-01, "lot"}`) and inventories
/// (`(10.00 USD, 10 AAPL {100 USD, 2024-01-01})`). Numbers never use exponent notation
/// and sets are comma separated.
pub(super) fn str_(args: &[Value], _ctx: &dyn FunctionContext) -> Result<Value, String> {
    Ok(Value::Str(match &args[0] {
        Value::Inventory(inventory) => inventory_to_string(inventory),
        other => other.to_string(),
    }))
}

pub(super) fn length(args: &[Value], _ctx: &dyn FunctionContext) -> Result<Value, String> {
    let length = match &args[0] {
        Value::Str(string) => string.chars().count(),
        Value::Set(set) => set.len(),
        _ => return Err("length() expects a string or a set".to_owned()),
    };
    i64::try_from(length)
        .map(Value::Int)
        .map_err(|_| "length() overflows a 64-bit integer".to_owned())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::super::testing::*;
    use super::*;
    use crate::value::{Cost, Position};

    #[test]
    fn str_of_scalars() {
        assert_eq!(call("str", vec![Value::Int(-3)]), Value::from("-3"));
        assert_eq!(call("str", vec![Value::Decimal(d("1.50"))]), Value::from("1.50"));
        assert_eq!(call("str", vec![Value::Decimal(d("1E+3"))]), Value::from("1000"));
        assert_eq!(call("str", vec![Value::Bool(true)]), Value::from("TRUE"));
        assert_eq!(call("str", vec![Value::Bool(false)]), Value::from("FALSE"));
        assert_eq!(call("str", vec!["text".into()]), Value::from("text"));
        assert_eq!(call("str", vec![Value::Date(date("2024-01-05"))]), Value::from("2024-01-05"));
        let tags = ["tag2", "tag1"].iter().map(|it| it.to_string()).collect::<BTreeSet<_>>();
        assert_eq!(call("str", vec![Value::Set(tags)]), Value::from("tag1, tag2"));
        assert_eq!(call("str", vec![Value::Null]), Value::Null);
    }

    #[test]
    fn str_of_accounting_types() {
        assert_eq!(call("str", vec![Value::Amount(amount("-1000.00", "USD"))]), Value::from("-1000.00 USD"));
        let lot = Position::new(
            amount("10", "AAPL"),
            Some(Cost {
                number: d("100"),
                currency: "USD".to_owned(),
                date: Some(date("2024-01-05")),
                label: Some("lot1".to_owned()),
            }),
        );
        assert_eq!(
            call("str", vec![Value::Position(lot.clone())]),
            Value::from("10 AAPL {100 USD, 2024-01-05, \"lot1\"}")
        );
        assert_eq!(call("str", vec![Value::Position(position("5", "XYZ", None))]), Value::from("5 XYZ"));
        // common currencies first, then by currency length, then number
        let inventory = inventory(vec![lot, position("5", "XYZ", Some(("2", "EUR"))), position("-1011.00", "USD", None)]);
        assert_eq!(
            call("str", vec![Value::Inventory(inventory)]),
            Value::from("(-1011.00 USD, 5 XYZ {2 EUR, 2024-01-05}, 10 AAPL {100 USD, 2024-01-05, \"lot1\"})")
        );
        assert_eq!(call("str", vec![Value::Inventory(Inventory::new())]), Value::from("()"));
    }

    #[test]
    fn length_of_string_and_set() {
        assert_eq!(call("length", vec!["Assets".into()]), Value::Int(6));
        assert_eq!(call("length", vec!["".into()]), Value::Int(0));
        assert_eq!(call("length", vec!["资产:银行".into()]), Value::Int(5));
        let tags = ["a", "b"].iter().map(|it| it.to_string()).collect::<BTreeSet<_>>();
        assert_eq!(call("length", vec![Value::Set(tags)]), Value::Int(2));
        assert_eq!(call("length", vec![Value::Set(BTreeSet::new())]), Value::Int(0));
    }
}
