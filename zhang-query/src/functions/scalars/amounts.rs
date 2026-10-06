//! Functions on numbers, amounts, positions and inventories: `abs`, `neg`, `possign`,
//! `number`, `currency`, `only` and `filter_currency`.

use bigdecimal::{BigDecimal, Zero};

use crate::value::{Inventory, Position, Value};
use crate::Amount;

pub(super) fn abs(args: &[Value]) -> Result<Value, String> {
    match &args[0] {
        Value::Int(number) => number
            .checked_abs()
            .map(Value::Int)
            .ok_or_else(|| "abs() overflows a 64-bit integer".to_owned()),
        Value::Decimal(number) => Ok(Value::Decimal(number.abs())),
        Value::Amount(amount) => Ok(Value::Amount(amount.abs())),
        Value::Position(position) => Ok(Value::Position(Position::new(position.units.abs(), position.cost.clone()))),
        Value::Inventory(inventory) => Ok(Value::Inventory(
            inventory
                .positions()
                .map(|position| Position::new(position.units.abs(), position.cost))
                .collect(),
        )),
        _ => Err("abs() expects a number, an amount, a position or an inventory".to_owned()),
    }
}

fn negate(value: &Value, function: &str) -> Result<Value, String> {
    match value {
        Value::Int(number) => number
            .checked_neg()
            .map(Value::Int)
            .ok_or_else(|| format!("{}() overflows a 64-bit integer", function)),
        Value::Decimal(number) => Ok(Value::Decimal(-number)),
        Value::Amount(amount) => Ok(Value::Amount(amount.neg())),
        Value::Position(position) => Ok(Value::Position(-position.clone())),
        Value::Inventory(inventory) => Ok(Value::Inventory(-inventory.clone())),
        _ => Err(format!("{}() expects a number, an amount, a position or an inventory", function)),
    }
}

pub(super) fn neg(args: &[Value]) -> Result<Value, String> {
    negate(&args[0], "neg")
}

/// beancount `account_types.get_account_sign` with the default account types: accounts
/// under `Assets` and `Expenses` are debit-normal (+1), every other root is credit-normal.
fn is_debit_normal(account: &str) -> bool {
    matches!(account.split(':').next(), Some("Assets" | "Expenses"))
}

pub(super) fn possign(args: &[Value]) -> Result<Value, String> {
    let account = args[1].as_str().ok_or("possign() expects an account name")?;
    if is_debit_normal(account) {
        Ok(args[0].clone())
    } else {
        negate(&args[0], "possign")
    }
}

pub(super) fn number(args: &[Value]) -> Result<Value, String> {
    let amount = args[0].as_amount().ok_or("number() expects an amount")?;
    Ok(Value::Decimal(amount.number.clone()))
}

pub(super) fn currency(args: &[Value]) -> Result<Value, String> {
    let amount = args[0].as_amount().ok_or("currency() expects an amount")?;
    Ok(Value::Str(amount.commodity.clone()))
}

pub(super) fn only(args: &[Value]) -> Result<Value, String> {
    let currency = args[0].as_str().ok_or("only() expects a currency")?;
    let inventory = args[1].as_inventory().ok_or("only() expects an inventory")?;
    let total = inventory
        .positions()
        .filter(|position| position.units.commodity == currency)
        .fold(BigDecimal::zero(), |total, position| total + position.units.number);
    Ok(Value::Amount(Amount::new(total, currency)))
}

pub(super) fn filter_currency(args: &[Value]) -> Result<Value, String> {
    let currency = args[1].as_str().ok_or("filter_currency() expects a currency")?;
    match &args[0] {
        Value::Position(position) if position.units.commodity == currency => Ok(Value::Position(position.clone())),
        Value::Position(_) => Ok(Value::Null),
        Value::Inventory(inventory) => Ok(Value::Inventory(
            inventory
                .positions()
                .filter(|position| position.units.commodity == currency)
                .collect::<Inventory>(),
        )),
        _ => Err("filter_currency() expects a position or an inventory".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::super::testing::*;
    use super::*;

    /// Inventories render in beancount's position order: common currencies first, then by
    /// cost number, then by units.
    fn lots() -> Value {
        Value::Inventory(inventory(vec![
            position("10", "AAPL", Some(("100", "USD"))),
            position("-5", "AAPL", Some(("120", "USD"))),
            position("-1011.00", "USD", None),
            position("3", "EUR", None),
        ]))
    }

    #[test]
    fn abs_of_every_type() {
        assert_eq!(call("abs", vec![Value::Int(-3)]), Value::Int(3));
        assert_eq!(call("abs", vec![Value::Int(0)]), Value::Int(0));
        assert!(try_call_with(&crate::functions::TestContext::default(), "abs", vec![Value::Int(i64::MIN)]).is_err());
        assert_eq!(call_str("abs", vec![Value::Decimal(d("-1.50"))]), "1.50");
        assert_eq!(call_str("abs", vec![Value::Decimal(d("2"))]), "2");
        assert_eq!(call_str("abs", vec![Value::Amount(amount("-12.30", "USD"))]), "12.30 USD");
        assert_eq!(
            call_str("abs", vec![Value::Position(position("-5", "AAPL", Some(("120", "USD"))))]),
            "5 AAPL {120 USD, 2024-01-05}"
        );
        assert_eq!(
            call_str("abs", vec![lots()]),
            "(1011.00 USD, 3 EUR, 10 AAPL {100 USD, 2024-01-05}, 5 AAPL {120 USD, 2024-01-05})"
        );
    }

    #[test]
    fn neg_of_every_type() {
        assert_eq!(call("neg", vec![Value::Int(3)]), Value::Int(-3));
        assert!(try_call_with(&crate::functions::TestContext::default(), "neg", vec![Value::Int(i64::MIN)]).is_err());
        assert_eq!(call_str("neg", vec![Value::Decimal(d("1.50"))]), "-1.50");
        assert_eq!(call_str("neg", vec![Value::Decimal(d("0"))]), "0");
        assert_eq!(call_str("neg", vec![Value::Amount(amount("-12.30", "USD"))]), "12.30 USD");
        assert_eq!(
            call_str("neg", vec![Value::Position(position("5", "AAPL", Some(("120", "USD"))))]),
            "-5 AAPL {120 USD, 2024-01-05}"
        );
        assert_eq!(
            call_str("neg", vec![lots()]),
            "(1011.00 USD, -3 EUR, -10 AAPL {100 USD, 2024-01-05}, 5 AAPL {120 USD, 2024-01-05})"
        );
    }

    #[test]
    fn possign_flips_credit_normal_accounts() {
        let amt = || Value::Amount(amount("-500.00", "CNY"));
        assert_eq!(call_str("possign", vec![amt(), "Income:Salary".into()]), "500.00 CNY");
        assert_eq!(call_str("possign", vec![amt(), "Liabilities:Card".into()]), "500.00 CNY");
        assert_eq!(call_str("possign", vec![amt(), "Equity:Opening".into()]), "500.00 CNY");
        assert_eq!(call_str("possign", vec![amt(), "Assets:Bank".into()]), "-500.00 CNY");
        assert_eq!(call_str("possign", vec![amt(), "Expenses:Food".into()]), "-500.00 CNY");
        // only the root component counts, and it is case-sensitive like beancount
        assert_eq!(call_str("possign", vec![amt(), "Income:Assets".into()]), "500.00 CNY");
        assert_eq!(call_str("possign", vec![Value::Decimal(d("2.5")), "Income".into()]), "-2.5");
        // integers are widened to decimals
        assert_eq!(call("possign", vec![Value::Int(2), "Income".into()]), Value::Decimal(d("-2")));
        assert_eq!(
            call_str(
                "possign",
                vec![Value::Position(position("5", "AAPL", Some(("1", "USD")))), "Liabilities".into()]
            ),
            "-5 AAPL {1 USD, 2024-01-05}"
        );
        assert_eq!(
            call_str("possign", vec![lots(), "Income:X".into()]),
            "(1011.00 USD, -3 EUR, -10 AAPL {100 USD, 2024-01-05}, 5 AAPL {120 USD, 2024-01-05})"
        );
        assert_eq!(
            call_str("possign", vec![lots(), "Assets:X".into()]),
            "(-1011.00 USD, 3 EUR, 10 AAPL {100 USD, 2024-01-05}, -5 AAPL {120 USD, 2024-01-05})"
        );
    }

    #[test]
    fn number_and_currency_of_amount() {
        assert_eq!(call_str("number", vec![Value::Amount(amount("-12.30", "USD"))]), "-12.30");
        assert_eq!(call("currency", vec![Value::Amount(amount("1", "USD"))]), Value::from("USD"));
        assert_eq!(call("commodity", vec![Value::Amount(amount("1", "USD"))]), Value::from("USD"));
    }

    #[test]
    fn only_sums_the_units_of_one_currency() {
        assert_eq!(call_str("only", vec!["AAPL".into(), lots()]), "5 AAPL");
        assert_eq!(call_str("only", vec!["USD".into(), lots()]), "-1011.00 USD");
        assert_eq!(call_str("only", vec!["GBP".into(), lots()]), "0 GBP");
    }

    #[test]
    fn filter_currency_keeps_matching_lots() {
        assert_eq!(
            call_str("filter_currency", vec![lots(), "AAPL".into()]),
            "(10 AAPL {100 USD, 2024-01-05}, -5 AAPL {120 USD, 2024-01-05})"
        );
        assert_eq!(call_str("filter_currency", vec![lots(), "GBP".into()]), "()");
        let usd = || Value::Position(position("1", "USD", None));
        assert_eq!(call_str("filter_currency", vec![usd(), "USD".into()]), "1 USD");
        assert_eq!(call("filter_currency", vec![usd(), "EUR".into()]), Value::Null);
    }
}
