//! Valuation functions: `units`, `cost`, `convert`, `value` and `getprice`.
//!
//! Semantics follow beancount's `convert` module as used by beanquery:
//! - `cost` multiplies the units by the per-unit cost; a position without cost is its units.
//! - `convert` first tries a direct market rate from the units currency to the target; for a
//!   position held at cost it then tries the implied two-step market rate units → cost
//!   currency → target (it never converts the book cost). Without a rate the units are
//!   returned as-is.
//! - `value` prices the units in the cost currency; positions without cost, or without a
//!   price, are returned as their units.
//!
//! `cost` is exact (`Position::at_cost`); products with market rates are rounded like
//! Python's default decimal context (28 significant digits, half-even), so results match
//! beanquery even with inverted (28-digit) rates. See `crate::decimal` for the policy.

use chrono::NaiveDate;

use crate::decimal::mul_in_context as mul;
use crate::functions::FunctionContext;
use crate::prices::PriceMap;
use crate::value::{Position, Value};
use crate::Amount;

fn date_arg(args: &[Value], idx: usize, function: &str) -> Result<Option<NaiveDate>, String> {
    args.get(idx)
        .map(|it| it.as_date().ok_or_else(|| format!("{}() expects a date", function)))
        .transpose()
}

/// beancount `convert.get_cost`: units × per-unit cost in the cost currency, or the units.
/// Exact, and the same as `Inventory::at_cost` per position.
fn position_cost(position: &Position) -> Amount {
    position.at_cost()
}

/// beancount `convert.get_value`: the units priced in the cost currency, or the units.
fn position_value(position: &Position, prices: &PriceMap, date: Option<NaiveDate>) -> Amount {
    if let Some(cost) = &position.cost {
        if let Some(rate) = prices.rate(&position.units.commodity, &cost.currency, date) {
            return Amount::new(mul(&position.units.number, &rate), cost.currency.clone());
        }
    }
    position.units.clone()
}

/// beancount `convert.convert_amount`: a direct market rate, else the implied rate through
/// `via` (skipped when `via` is the target), else the amount unchanged.
fn convert_units(units: &Amount, target: &str, via: Option<&str>, prices: &PriceMap, date: Option<NaiveDate>) -> Amount {
    if let Some(rate) = prices.rate(&units.commodity, target, date) {
        return Amount::new(mul(&units.number, &rate), target);
    }
    if let Some(via) = via.filter(|via| *via != target) {
        if let (Some(rate1), Some(rate2)) = (prices.rate(&units.commodity, via, date), prices.rate(via, target, date)) {
            // two roundings, like beancount's `number * rate1 * rate2`
            return Amount::new(mul(&mul(&units.number, &rate1), &rate2), target);
        }
    }
    units.clone()
}

/// beancount `convert.convert_position`: convert the units, stepping through the cost
/// currency when there is no direct rate.
pub(crate) fn convert_position(position: &Position, target: &str, prices: &PriceMap, date: Option<NaiveDate>) -> Amount {
    let via = position.cost.as_ref().map(|cost| cost.currency.as_str());
    convert_units(&position.units, target, via, prices, date)
}

pub(super) fn units(args: &[Value], _ctx: &dyn FunctionContext) -> Result<Value, String> {
    match &args[0] {
        Value::Position(position) => Ok(Value::Amount(position.units.clone())),
        Value::Inventory(inventory) => Ok(Value::Inventory(inventory.units())),
        _ => Err("units() expects a position or an inventory".to_owned()),
    }
}

pub(super) fn cost(args: &[Value], _ctx: &dyn FunctionContext) -> Result<Value, String> {
    match &args[0] {
        Value::Position(position) => Ok(Value::Amount(position_cost(position))),
        Value::Inventory(inventory) => Ok(Value::Inventory(inventory.reduce(position_cost))),
        _ => Err("cost() expects a position or an inventory".to_owned()),
    }
}

pub(super) fn convert(args: &[Value], ctx: &dyn FunctionContext) -> Result<Value, String> {
    let target = args[1].as_str().ok_or("convert() expects a target currency")?;
    let date = date_arg(args, 2, "convert")?;
    let prices = ctx.prices();
    match &args[0] {
        Value::Amount(amount) => Ok(Value::Amount(convert_units(amount, target, None, prices, date))),
        Value::Position(position) => Ok(Value::Amount(convert_position(position, target, prices, date))),
        Value::Inventory(inventory) => Ok(Value::Inventory(inventory.reduce(|position| convert_position(position, target, prices, date)))),
        _ => Err("convert() expects an amount, a position or an inventory".to_owned()),
    }
}

pub(super) fn value(args: &[Value], ctx: &dyn FunctionContext) -> Result<Value, String> {
    let date = date_arg(args, 1, "value")?;
    let prices = ctx.prices();
    match &args[0] {
        Value::Position(position) => Ok(Value::Amount(position_value(position, prices, date))),
        Value::Inventory(inventory) => Ok(Value::Inventory(inventory.reduce(|position| position_value(position, prices, date)))),
        _ => Err("value() expects a position or an inventory".to_owned()),
    }
}

pub(super) fn getprice(args: &[Value], ctx: &dyn FunctionContext) -> Result<Value, String> {
    let base = args[0].as_str().ok_or("getprice() expects a base currency")?;
    let quote = args[1].as_str().ok_or("getprice() expects a quote currency")?;
    let date = date_arg(args, 2, "getprice")?;
    Ok(ctx.prices().rate(&base.to_uppercase(), &quote.to_uppercase(), date).into())
}

#[cfg(test)]
mod tests {
    use bigdecimal::BigDecimal;

    use super::super::testing::*;
    use super::*;
    use crate::functions::TestContext;

    /// AAPL is quoted in USD (150, then 160), USD in CNY, EUR in USD, XYZ in EUR, and
    /// CNY/JPY only in the inverse direction.
    fn ctx() -> TestContext {
        context_with_prices(&[
            ("2024-01-01", "AAPL", "USD", "150"),
            ("2024-02-01", "AAPL", "USD", "160"),
            ("2024-01-01", "USD", "CNY", "7"),
            ("2024-01-01", "EUR", "USD", "1.1"),
            ("2024-01-01", "EUR", "CNY", "8"),
            ("2024-01-01", "XYZ", "EUR", "3"),
            ("2024-01-10", "XYZ", "EUR", "4"),
            ("2024-01-01", "JPY", "CNY", "0.05"),
        ])
    }

    fn aapl_lot() -> Value {
        Value::Position(position("10", "AAPL", Some(("100", "USD"))))
    }

    fn mixed_inventory() -> Value {
        Value::Inventory(inventory(vec![
            position("10", "AAPL", Some(("100", "USD"))),
            position("5", "XYZ", Some(("2", "EUR"))),
            position("-1011.00", "USD", None),
        ]))
    }

    #[test]
    fn mul_rounds_like_python_decimal() {
        let plain = |it: BigDecimal| crate::decimal::to_plain_string(&it);
        assert_eq!(plain(mul(&d("10.00"), &d("7"))), "70.00");
        assert_eq!(plain(mul(&d("-1000.00"), &d("1"))), "-1000.00");
        assert_eq!(plain(mul(&d("1"), &d("2.50"))), "2.50");
        assert_eq!(plain(mul(&d("0.00"), &d("3"))), "0.00");
        assert_eq!(
            crate::decimal::to_plain_string(&mul(&d("1600"), &d("0.9090909090909090909090909091"))),
            "1454.545454545454545454545455"
        );
        assert_eq!(
            crate::decimal::to_plain_string(&mul(&d("9.999999999999999999999999999"), &d("1.0000000000000000000000000001"))),
            "10.00000000000000000000000000"
        );
    }

    #[test]
    fn units_of_position_and_inventory() {
        assert_eq!(call_str("units", vec![aapl_lot()]), "10 AAPL");
        assert_eq!(call_str("units", vec![Value::Position(position("-3.50", "USD", None))]), "-3.50 USD");
        let mut lots = inventory(vec![position("10", "AAPL", Some(("100", "USD"))), position("5", "AAPL", Some(("120", "USD")))]);
        lots.add_amount(&amount("4.00", "USD"));
        assert_eq!(call_str("units", vec![Value::Inventory(lots)]), "(4.00 USD, 15 AAPL)");
        assert_eq!(call_str("units", vec![Value::Inventory(inventory(vec![]))]), "()");
    }

    #[test]
    fn cost_of_position_and_inventory() {
        assert_eq!(call_str("cost", vec![aapl_lot()]), "1000 USD");
        // no cost: the units
        assert_eq!(call_str("cost", vec![Value::Position(position("2.50", "EUR", None))]), "2.50 EUR");
        assert_eq!(
            call_str("cost", vec![Value::Position(position("-2", "AAPL", Some(("100.5", "USD"))))]),
            "-201.0 USD"
        );
        assert_eq!(call_str("cost", vec![mixed_inventory()]), "(-11.00 USD, 10 EUR)");
    }

    #[test]
    fn convert_amount_uses_direct_and_inverse_rates() {
        let ctx = ctx();
        let usd = || Value::Amount(amount("-1000.00", "USD"));
        assert_eq!(call_str_with(&ctx, "convert", vec![usd(), "CNY".into()]), "-7000.00 CNY");
        // same currency: rate 1
        assert_eq!(call_str_with(&ctx, "convert", vec![usd(), "USD".into()]), "-1000.00 USD");
        // inverse of CNY→USD = 1/7
        assert_eq!(
            call_str_with(&ctx, "convert", vec![Value::Amount(amount("7", "CNY")), "USD".into()]),
            "1.000000000000000000000000000 USD"
        );
        // only JPY→CNY is quoted: CNY→JPY uses the inverse
        assert_eq!(call_str_with(&ctx, "convert", vec![Value::Amount(amount("1", "CNY")), "JPY".into()]), "20 JPY");
        // missing price: unchanged
        assert_eq!(call_str_with(&ctx, "convert", vec![Value::Amount(amount("5", "GBP")), "CNY".into()]), "5 GBP");
        // no AAPL→CNY price (only AAPL→USD): unchanged
        assert_eq!(
            call_str_with(&ctx, "convert", vec![Value::Amount(amount("10", "AAPL")), "CNY".into()]),
            "10 AAPL"
        );
        // the product with an inverted (28-digit) rate is rounded like Python's decimal context
        assert_eq!(
            call_str_with(&ctx, "convert", vec![Value::Amount(amount("1600", "USD")), "EUR".into()]),
            "1454.545454545454545454545455 EUR"
        );
    }

    #[test]
    fn convert_with_date_uses_latest_price_on_or_before() {
        let ctx = ctx();
        let aapl = || Value::Amount(amount("10", "AAPL"));
        assert_eq!(call_str_with(&ctx, "convert", vec![aapl(), "USD".into()]), "1600 USD");
        assert_eq!(
            call_str_with(&ctx, "convert", vec![aapl(), "USD".into(), Value::Date(date("2024-01-31"))]),
            "1500 USD"
        );
        assert_eq!(
            call_str_with(&ctx, "convert", vec![aapl(), "USD".into(), Value::Date(date("2024-02-01"))]),
            "1600 USD"
        );
        assert_eq!(
            call_str_with(&ctx, "convert", vec![aapl(), "USD".into(), Value::Date(date("2023-12-31"))]),
            "10 AAPL"
        );
    }

    #[test]
    fn convert_position_steps_through_the_cost_currency() {
        let ctx = ctx();
        // a direct market price wins, and the book cost is never used
        assert_eq!(call_str_with(&ctx, "convert", vec![aapl_lot(), "USD".into()]), "1600 USD");
        assert_eq!(
            call_str_with(&ctx, "convert", vec![aapl_lot(), "USD".into(), Value::Date(date("2024-01-05"))]),
            "1500 USD"
        );
        // the beanquery oracle example: AAPL→CNY is not quoted, so AAPL→USD (160) then USD→CNY (7)
        assert_eq!(call_str_with(&ctx, "convert", vec![aapl_lot(), "CNY".into()]), "11200 CNY");
        // both steps use prices on or before the date
        assert_eq!(
            call_str_with(&ctx, "convert", vec![aapl_lot(), "CNY".into(), Value::Date(date("2024-01-05"))]),
            "10500 CNY"
        );
        // an inverted second step (USD→EUR = 1/1.1), rounded to 28 significant digits
        assert_eq!(
            call_str_with(&ctx, "convert", vec![aapl_lot(), "EUR".into()]),
            "1454.545454545454545454545455 EUR"
        );
        let xyz = || Value::Position(position("5", "XYZ", Some(("2", "EUR"))));
        assert_eq!(call_str_with(&ctx, "convert", vec![xyz(), "EUR".into()]), "20 EUR");
        assert_eq!(call_str_with(&ctx, "convert", vec![xyz(), "CNY".into()]), "160 CNY");
        // no path: no second step (no USD→GBP price) leaves the units unconverted
        assert_eq!(call_str_with(&ctx, "convert", vec![aapl_lot(), "GBP".into()]), "10 AAPL");
        // no path: no first step (no GHOST price at all), even though EUR→CNY exists
        let ghost = || Value::Position(position("5", "GHOST", Some(("2", "EUR"))));
        assert_eq!(call_str_with(&ctx, "convert", vec![ghost(), "CNY".into()]), "5 GHOST");
        assert_eq!(call_str_with(&ctx, "convert", vec![ghost(), "EUR".into()]), "5 GHOST");
        // no path before the first price
        assert_eq!(
            call_str_with(&ctx, "convert", vec![aapl_lot(), "CNY".into(), Value::Date(date("2023-12-31"))]),
            "10 AAPL"
        );
        // positions without cost convert like amounts (no intermediate currency)
        assert_eq!(
            call_str_with(&ctx, "convert", vec![Value::Position(position("2", "EUR", None)), "CNY".into()]),
            "16 CNY"
        );
        assert_eq!(
            call_str_with(&ctx, "convert", vec![Value::Position(position("10", "AAPL", None)), "CNY".into()]),
            "10 AAPL"
        );
    }

    #[test]
    fn convert_inventory_reduces_every_lot() {
        let ctx = ctx();
        // -1011.00 + 1600 (direct) + 22 (XYZ→EUR→USD); values cross-checked with beanquery
        assert_eq!(call_str_with(&ctx, "convert", vec![mixed_inventory(), "USD".into()]), "(611.00 USD)");
        assert_eq!(
            call_str_with(&ctx, "convert", vec![mixed_inventory(), "USD".into(), Value::Date(date("2024-01-05"))]),
            "(505.50 USD)"
        );
        // -7077.00 (direct) + 11200 (via USD) + 160 (via EUR)
        assert_eq!(call_str_with(&ctx, "convert", vec![mixed_inventory(), "CNY".into()]), "(4283.00 CNY)");
        // -7077.00 + 10500 + 120 with the prices of 2024-01-05
        assert_eq!(
            call_str_with(&ctx, "convert", vec![mixed_inventory(), "CNY".into(), Value::Date(date("2024-01-05"))]),
            "(3543.00 CNY)"
        );
        let mut partial = inventory(vec![position("5", "GHOST", Some(("2", "EUR")))]);
        partial.add_amount(&amount("1.00", "USD"));
        assert_eq!(
            call_str_with(&ctx, "convert", vec![Value::Inventory(partial), "CNY".into()]),
            "(7.00 CNY, 5 GHOST)"
        );
    }

    #[test]
    fn value_prices_units_in_the_cost_currency() {
        let ctx = ctx();
        assert_eq!(call_str_with(&ctx, "value", vec![aapl_lot()]), "1600 USD");
        assert_eq!(call_str_with(&ctx, "value", vec![aapl_lot(), Value::Date(date("2024-01-15"))]), "1500 USD");
        // no price before the first quote: units
        assert_eq!(call_str_with(&ctx, "value", vec![aapl_lot(), Value::Date(date("2023-06-01"))]), "10 AAPL");
        // no cost: units, even when a price exists
        assert_eq!(
            call_str_with(&ctx, "value", vec![Value::Position(position("-1000.00", "USD", None))]),
            "-1000.00 USD"
        );
        assert_eq!(call_str_with(&ctx, "value", vec![Value::Position(position("10", "AAPL", None))]), "10 AAPL");
        // no price for the units: units
        assert_eq!(
            call_str_with(&ctx, "value", vec![Value::Position(position("5", "GHOST", Some(("2", "EUR"))))]),
            "5 GHOST"
        );
        assert_eq!(call_str_with(&ctx, "value", vec![mixed_inventory()]), "(589.00 USD, 20 EUR)");
        assert_eq!(
            call_str_with(&ctx, "value", vec![mixed_inventory(), Value::Date(date("2024-01-05"))]),
            "(489.00 USD, 15 EUR)"
        );
        // with an empty price map nothing is valued
        assert_eq!(call_str("value", vec![mixed_inventory()]), "(-1011.00 USD, 5 XYZ, 10 AAPL)");
    }

    #[test]
    fn getprice_looks_up_rates() {
        let ctx = ctx();
        let price = |args: Vec<Value>| call_with(&ctx, "getprice", args);
        assert_eq!(price(vec!["AAPL".into(), "USD".into()]), Value::Decimal(d("160")));
        assert_eq!(
            price(vec!["aapl".into(), "usd".into(), Value::Date(date("2024-01-15"))]),
            Value::Decimal(d("150"))
        );
        assert_eq!(price(vec!["AAPL".into(), "USD".into(), Value::Date(date("2023-01-01"))]), Value::Null);
        assert_eq!(price(vec!["USD".into(), "AAPL".into()]), Value::Decimal(d("0.00625")));
        assert_eq!(price(vec!["GBP".into(), "USD".into()]), Value::Null);
        assert_eq!(price(vec!["GBP".into(), "GBP".into()]), Value::Decimal(d("1")));
    }
}
