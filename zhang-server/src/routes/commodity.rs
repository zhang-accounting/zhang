//! The commodity pages. What a commodity is (precision, prefix, suffix, rounding, group) is read
//! from the store; its holdings, lots and prices come from [built-in queries](crate::builtin).

use std::collections::HashMap;

use axum::extract::{Path, State};
use bigdecimal::BigDecimal;
use gotcha::api;
use itertools::Itertools;
use zhang_ast::amount::Amount;
use zhang_core::constants::COMMODITY_GROUP;
use zhang_core::domains::schemas::{CommodityDomain, MetaType};
use zhang_query::{DataType, Params, QueryResult};

use crate::builtin::{run, BuiltinQuery};
use crate::cells::{first_row, rows, Row};
use crate::response::{CommodityDetailEntity, CommodityListItemEntity, CommodityLotEntity, CommodityPriceEntity, ResponseWrapper};
use crate::state::SharedLedger;
use crate::{ApiResult, ServerResult};

/// How much of every commodity the ledger holds.
pub(crate) static COMMODITY_TOTALS: BuiltinQuery = BuiltinQuery {
    name: "commodity_totals",
    description: "How many units of each commodity the Assets and Liabilities accounts hold, for the commodities they \
                  hold.",
    bql: "SELECT currency, sum(number) AS total
WHERE root(account, 1) IN ('Assets', 'Liabilities')
GROUP BY currency
HAVING sum(number) != 0
ORDER BY currency",
    params: &[],
};

/// How much of one commodity the ledger holds.
pub(crate) static COMMODITY_TOTAL: BuiltinQuery = BuiltinQuery {
    name: "commodity_total",
    description: "How many units of a commodity the Assets and Liabilities accounts hold. No row if they hold none.",
    bql: "SELECT currency, sum(number) AS total
WHERE currency = :commodity AND root(account, 1) IN ('Assets', 'Liabilities')
GROUP BY currency
HAVING sum(number) != 0",
    params: &[("commodity", DataType::Str)],
};

/// The latest price of every commodity in one currency.
pub(crate) static LATEST_PRICES: BuiltinQuery = BuiltinQuery {
    name: "latest_prices",
    description: "The latest price directive of each commodity quoted in a currency (the operating currency), with its \
                  date and time.",
    bql: "SELECT currency, last(date) AS date, last(time) AS time, last(amount) AS price
FROM #prices
WHERE currency(amount) = :currency
GROUP BY currency
ORDER BY currency",
    params: &[("currency", DataType::Str)],
};

/// The latest price of one commodity in one currency.
pub(crate) static LATEST_PRICE: BuiltinQuery = BuiltinQuery {
    name: "latest_price",
    description: "The latest price directive of a commodity quoted in a currency (the operating currency), with its \
                  date and time. No row if there is none.",
    bql: "SELECT currency, last(date) AS date, last(time) AS time, last(amount) AS price
FROM #prices
WHERE currency = :commodity AND currency(amount) = :currency
GROUP BY currency",
    params: &[("commodity", DataType::Str), ("currency", DataType::Str)],
};

/// The lots of one commodity held in the Assets and Liabilities accounts.
pub(crate) static COMMODITY_LOTS: BuiltinQuery = BuiltinQuery {
    name: "commodity_lots",
    description: "The lots of a commodity that the Assets and Liabilities accounts hold: the units per account, cost \
                  and acquisition date, by account, then oldest first. Units held without cost are one lot per account.",
    bql: "SELECT account, cost_date, cost_number, cost_currency, sum(number) AS units
WHERE currency = :commodity AND root(account, 1) IN ('Assets', 'Liabilities')
GROUP BY account, cost_date, cost_number, cost_currency
HAVING sum(number) != 0
ORDER BY account, cost_date, cost_number",
    params: &[("commodity", DataType::Str)],
};

/// Every price of one commodity.
pub(crate) static COMMODITY_PRICES: BuiltinQuery = BuiltinQuery {
    name: "commodity_prices",
    description: "Every price directive of a commodity, in any currency, oldest first.",
    bql: "SELECT date, time, amount
FROM #prices
WHERE currency = :commodity
ORDER BY date, time",
    params: &[("commodity", DataType::Str)],
};

/// What the store knows of a commodity: its directive's settings and its group.
fn commodity_item(commodity: CommodityDomain, group: Option<String>, total: BigDecimal, latest_price: Option<Row<'_>>) -> CommodityListItemEntity {
    let latest_price_amount = latest_price.as_ref().and_then(|row| row.amount("price"));
    CommodityListItemEntity {
        name: commodity.name,
        precision: commodity.precision,
        prefix: commodity.prefix,
        suffix: commodity.suffix,
        rounding: commodity.rounding.to_string(),
        group,
        total_amount: total,
        latest_price_date: latest_price.as_ref().and_then(|row| row.datetime("date", "time")),
        latest_price_amount: latest_price_amount.as_ref().map(|it| it.number.clone()),
        latest_price_commodity: latest_price_amount.map(|it| it.commodity),
    }
}

/// The commodities of the store, in the store's order, with their groups.
async fn stored_commodities(ledger: &SharedLedger, name: Option<&str>) -> (Vec<(CommodityDomain, Option<String>)>, String) {
    let ledger = ledger.read().await;
    let operations = ledger.operations();
    let store = operations.read();
    let group = |commodity: &str| {
        store
            .metas
            .iter()
            .find(|meta| meta.meta_type == MetaType::CommodityMeta.as_ref() && meta.type_identifier == commodity && meta.key == COMMODITY_GROUP)
            .map(|meta| meta.value.clone())
    };
    let commodities = store
        .commodities
        .values()
        .filter(|commodity| name.is_none_or(|name| commodity.name == name))
        .map(|commodity| (commodity.clone(), group(&commodity.name)))
        .collect_vec();
    (commodities, ledger.options.operating_currency.clone())
}

fn by_currency(result: &QueryResult) -> HashMap<String, Row<'_>> {
    rows(result).filter_map(|row| Some((row.str("currency")?, row))).collect()
}

/// Every commodity, with how much of it the Assets and Liabilities accounts hold and its latest
/// price in the operating currency.
#[api(group = "commodity")]
pub async fn get_all_commodities(ledger: State<SharedLedger>) -> ApiResult<Vec<CommodityListItemEntity>> {
    let (commodities, operating_currency) = stored_commodities(&ledger, None).await;
    let totals = run(&ledger, &COMMODITY_TOTALS, Params::new()).await?;
    let totals = by_currency(&totals);
    let prices = run(&ledger, &LATEST_PRICES, Params::new().bind("currency", operating_currency)).await?;
    let mut prices = by_currency(&prices);

    let items = commodities
        .into_iter()
        .map(|(commodity, group)| {
            let total = totals.get(&commodity.name).and_then(|row| row.decimal("total")).unwrap_or_default();
            let latest_price = prices.remove(&commodity.name);
            commodity_item(commodity, group, total, latest_price)
        })
        .collect_vec();
    ResponseWrapper::json(items)
}

/// One commodity: how much of it the Assets and Liabilities accounts hold and in which lots, its
/// latest price in the operating currency, and all its prices. An unknown commodity is a 404.
#[api(group = "commodity")]
pub async fn get_single_commodity(ledger: State<SharedLedger>, params: Path<(String,)>) -> ApiResult<CommodityDetailEntity> {
    let commodity_name = params.0 .0;
    let (commodities, operating_currency) = stored_commodities(&ledger, Some(&commodity_name)).await;
    let Some((commodity, group)) = commodities.into_iter().next() else {
        return ResponseWrapper::not_found();
    };
    let detail = single_commodity(&ledger, commodity, group, operating_currency).await?;
    ResponseWrapper::json(detail)
}

async fn single_commodity(
    ledger: &SharedLedger, commodity: CommodityDomain, group: Option<String>, operating_currency: String,
) -> ServerResult<CommodityDetailEntity> {
    let name = commodity.name.clone();
    let total = run(ledger, &COMMODITY_TOTAL, Params::new().bind("commodity", name.as_str())).await?;
    let total = first_row(&total).and_then(|row| row.decimal("total")).unwrap_or_default();
    let latest_price = run(
        ledger,
        &LATEST_PRICE,
        Params::new().bind("commodity", name.as_str()).bind("currency", operating_currency),
    )
    .await?;
    let info = commodity_item(commodity, group, total, first_row(&latest_price));

    let lots = run(ledger, &COMMODITY_LOTS, Params::new().bind("commodity", name.as_str())).await?;
    let lots = rows(&lots)
        .map(|row| CommodityLotEntity {
            account: row.str("account").unwrap_or_default(),
            amount: row.decimal("units").unwrap_or_default(),
            cost: row
                .decimal("cost_number")
                .zip(row.str("cost_currency"))
                .map(|(number, currency)| Amount::new(number, currency)),
            price: None,
            acquisition_date: row.date("cost_date"),
        })
        .collect_vec();

    let prices = run(ledger, &COMMODITY_PRICES, Params::new().bind("commodity", name.as_str())).await?;
    let prices = rows(&prices)
        .filter_map(|row| {
            Some(CommodityPriceEntity {
                datetime: row.datetime("date", "time")?,
                amount: row.amount("amount")?,
            })
        })
        .collect_vec();

    Ok(CommodityDetailEntity { info, lots, prices })
}

/// The handlers as they were before they ran built-in queries, kept to compare the two until they
/// are removed (#479).
#[cfg(test)]
pub(crate) mod legacy {
    use axum::extract::{Path, State};
    use itertools::Itertools;
    use zhang_ast::amount::Amount;
    use zhang_core::constants::COMMODITY_GROUP;
    use zhang_core::domains::schemas::{CommodityDomain, MetaType};

    use crate::response::{CommodityDetailEntity, CommodityListItemEntity, CommodityLotEntity, CommodityPriceEntity, ResponseWrapper};
    use crate::state::SharedLedger;
    use crate::ApiResult;

    pub async fn get_all_commodities(ledger: State<SharedLedger>) -> ApiResult<Vec<CommodityListItemEntity>> {
        let ledger = ledger.read().await;

        let operations = ledger.operations();
        let operating_currency = ledger.options.operating_currency.as_str();
        let store = operations.read();
        let mut ret = vec![];
        for commodity in store.commodities.values().cloned() {
            let commodity: CommodityDomain = commodity;
            let latest_price = operations.get_latest_price(&commodity.name, operating_currency)?;

            let amount = operations.get_commodity_balances(&commodity.name)?;
            let group = operations
                .meta(MetaType::CommodityMeta, commodity.name.as_str(), COMMODITY_GROUP)?
                .map(|it| it.value);
            ret.push(CommodityListItemEntity {
                name: commodity.name,
                precision: commodity.precision,
                prefix: commodity.prefix,
                suffix: commodity.suffix,
                rounding: commodity.rounding.to_string(),
                group,
                total_amount: amount,
                latest_price_date: latest_price.as_ref().map(|it| it.datetime),
                latest_price_amount: latest_price.as_ref().map(|it| it.amount.clone()),
                latest_price_commodity: latest_price.map(|it| it.target_commodity),
            });
        }

        ResponseWrapper::json(ret)
    }

    pub async fn get_single_commodity(ledger: State<SharedLedger>, params: Path<(String,)>) -> ApiResult<CommodityDetailEntity> {
        let commodity_name = params.0 .0;
        let ledger = ledger.read().await;
        let operating_currency = ledger.options.operating_currency.clone();

        let operations = ledger.operations();
        let commodity = operations.commodity(&commodity_name)?.expect("cannot find commodity");
        let latest_price = operations.get_latest_price(&commodity_name, operating_currency)?;

        let amount = operations.get_commodity_balances(&commodity_name)?;
        let group = operations
            .meta(MetaType::CommodityMeta, commodity.name.as_str(), COMMODITY_GROUP)?
            .map(|it| it.value);
        let commodity_item = CommodityListItemEntity {
            name: commodity.name,
            precision: commodity.precision,
            prefix: commodity.prefix,
            suffix: commodity.suffix,
            rounding: commodity.rounding.to_string(),
            total_amount: amount,
            group,
            latest_price_date: latest_price.as_ref().map(|it| it.datetime),
            latest_price_amount: latest_price.as_ref().map(|it| it.amount.clone()),
            latest_price_commodity: latest_price.map(|it| it.target_commodity),
        };

        let lots = operations
            .commodity_lots(&commodity_name)?
            .into_iter()
            .map(|it| CommodityLotEntity {
                account: it.account.name().to_owned(),
                amount: it.amount,
                cost: it.cost,
                price: it.price,
                acquisition_date: it.acquisition_date,
            })
            .collect_vec();

        let prices = operations
            .commodity_prices(&commodity_name)?
            .into_iter()
            .map(|price| CommodityPriceEntity {
                datetime: price.datetime,
                amount: Amount::new(price.amount, price.target_commodity),
            })
            .collect_vec();

        ResponseWrapper::json(CommodityDetailEntity {
            info: commodity_item,
            lots,
            prices,
        })
    }
}
