//! Engine values as the response's amounts.

use std::collections::HashMap;

use bigdecimal::BigDecimal;
use zhang_ast::amount::{Amount, CalculatedAmount};
use zhang_query::Inventory;

/// The response's [`CalculatedAmount`] of a query's inventories: the one adapter from engine
/// values to it.
///
/// - `detail` is `units` per currency, the lots of a currency held at different costs merged
///   (lots that net to zero are not in an inventory, so neither is their currency);
/// - `calculated` is the `operating_currency` part of `value`, which the query computes from
///   the units, e.g. `convert(sum(position), :currency, :date)`. Units the query cannot
///   convert stay in their own currency there, and are left out of `calculated`.
pub fn calculated_amount(units: &Inventory, value: &Inventory, operating_currency: &str) -> CalculatedAmount {
    let detail = units
        .units()
        .positions()
        .map(|position| (position.units.commodity, position.units.number))
        .collect::<HashMap<_, _>>();
    let total = value
        .units()
        .positions()
        .filter(|position| position.units.commodity == operating_currency)
        .map(|position| position.units.number)
        .sum::<BigDecimal>();
    CalculatedAmount {
        calculated: Amount::new(total, operating_currency),
        detail,
    }
}

#[cfg(test)]
mod test {
    use std::collections::HashMap;
    use std::str::FromStr;

    use bigdecimal::BigDecimal;
    use chrono::NaiveDate;
    use zhang_ast::amount::Amount;
    use zhang_query::{Cost, Inventory, Position, PriceMap};

    use super::calculated_amount;

    fn d(number: &str) -> BigDecimal {
        BigDecimal::from_str(number).unwrap()
    }

    fn position(number: &str, currency: &str, cost: Option<(&str, &str)>) -> Position {
        Position::new(
            Amount::new(d(number), currency),
            cost.map(|(number, currency)| Cost {
                number: d(number),
                currency: currency.to_owned(),
                date: None,
                label: None,
            }),
        )
    }

    fn inventory(positions: &[Position]) -> Inventory {
        let mut inventory = Inventory::new();
        positions.iter().for_each(|position| inventory.add_position(position));
        inventory
    }

    #[test]
    fn units_are_the_detail_and_the_operating_currency_value_is_calculated() {
        let units = inventory(&[
            position("100.00", "USD", None),
            position("2", "AAPL", Some(("150", "USD"))),
            position("1", "AAPL", Some(("160", "USD"))),
            position("50", "EUR", None),
            position("7", "XYZ", None),
        ]);
        let prices = PriceMap::from_points([
            (NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(), "AAPL", "USD", &d("200")),
            // only the inverse rate
            (NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(), "USD", "EUR", &d("0.5")),
        ]);
        // what `convert(sum(position), 'USD')` gives: XYZ has no price and stays
        let value = units.convert("USD", &prices, None);

        let amount = calculated_amount(&units, &value, "USD");
        // 100 + 3 × 200 + 50 / 0.5
        assert_eq!(amount.calculated, Amount::new(d("800.00"), "USD"));
        assert_eq!(
            amount.detail,
            HashMap::from([
                ("USD".to_owned(), d("100.00")),
                ("AAPL".to_owned(), d("3")),
                ("EUR".to_owned(), d("50")),
                ("XYZ".to_owned(), d("7"))
            ])
        );
    }

    #[test]
    fn nothing_is_zero_in_the_operating_currency() {
        let amount = calculated_amount(&Inventory::new(), &Inventory::new(), "CNY");
        assert_eq!(amount.calculated, Amount::new(d("0"), "CNY"));
        assert!(amount.detail.is_empty());

        // a value without the operating currency counts for nothing
        let units = inventory(&[position("5", "XYZ", None)]);
        let amount = calculated_amount(&units, &units, "CNY");
        assert_eq!(amount.calculated, Amount::new(d("0"), "CNY"));
        assert_eq!(amount.detail, HashMap::from([("XYZ".to_owned(), d("5"))]));
    }
}
