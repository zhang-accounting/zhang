//! The commodity pages. What a commodity is (precision, prefix, suffix, rounding, group) is read
//! from the store; its holdings, lots and prices come from [built-in queries](crate::builtin)
//! (`commodities.*`).

use std::collections::HashMap;

use axum::extract::{Path, State};
use bigdecimal::BigDecimal;
use gotcha::api;
use itertools::Itertools;
use zhang_ast::amount::Amount;
use zhang_core::constants::COMMODITY_GROUP;
use zhang_core::domains::schemas::{CommodityDomain, MetaType};
use zhang_core::ledger::Ledger;
use zhang_query::{Params, QueryResult};

use crate::builtin::{execute, with_ledger};
use crate::cells::{first_row, rows, Row};
use crate::response::{CommodityDetailEntity, CommodityListItemEntity, CommodityLotEntity, CommodityPriceEntity, ResponseWrapper};
use crate::state::SharedLedger;
use crate::{ApiResult, ServerResult};

/// A commodity of the list, from the store's commodity, its group, its total and its latest price.
fn commodity_item(commodity: CommodityDomain, group: Option<String>, total: BigDecimal, latest_price: Option<&Row<'_>>) -> CommodityListItemEntity {
    let latest_price_amount = latest_price.and_then(|row| row.amount("price"));
    CommodityListItemEntity {
        name: commodity.name,
        precision: commodity.precision,
        prefix: commodity.prefix,
        suffix: commodity.suffix,
        rounding: commodity.rounding.to_string(),
        group,
        total_amount: total,
        latest_price_date: latest_price.and_then(|row| row.datetime("date", "time")),
        latest_price_amount: latest_price_amount.as_ref().map(|it| it.number.clone()),
        latest_price_commodity: latest_price_amount.map(|it| it.commodity),
    }
}

/// The commodities of the store (all of them, or the one named `name`), in the store's order,
/// with their groups.
fn stored_commodities(ledger: &Ledger, name: Option<&str>) -> Vec<(CommodityDomain, Option<String>)> {
    let store = ledger.store.read().expect("poison lock detect");
    let group = |commodity: &str| {
        store
            .metas
            .iter()
            .find(|meta| meta.meta_type == MetaType::CommodityMeta.as_ref() && meta.type_identifier == commodity && meta.key == COMMODITY_GROUP)
            .map(|meta| meta.value.clone())
    };
    store
        .commodities
        .values()
        .filter(|commodity| name.is_none_or(|name| commodity.name == name))
        .map(|commodity| (commodity.clone(), group(&commodity.name)))
        .collect_vec()
}

fn by_currency(result: &QueryResult) -> HashMap<String, Row<'_>> {
    rows(result).filter_map(|row| Some((row.str("currency")?, row))).collect()
}

/// Every commodity, with how much of it the Assets and Liabilities accounts hold and its latest
/// price in the operating currency.
#[api(group = "commodity")]
pub async fn get_all_commodities(ledger: State<SharedLedger>) -> ApiResult<Vec<CommodityListItemEntity>> {
    let items = with_ledger(&ledger, |ledger| {
        let commodities = stored_commodities(ledger, None);
        let totals = execute(ledger, "commodities.totals", &Params::new(), false)?;
        let totals = by_currency(&totals);
        let params = Params::new().bind("currency", ledger.options.operating_currency.as_str());
        let prices = execute(ledger, "commodities.latest_prices", &params, false)?;
        let prices = by_currency(&prices);
        let items = commodities
            .into_iter()
            .map(|(commodity, group)| {
                let total = totals.get(&commodity.name).and_then(|row| row.decimal("total")).unwrap_or_default();
                let latest_price = prices.get(&commodity.name);
                commodity_item(commodity, group, total, latest_price)
            })
            .collect_vec();
        Ok(items)
    })
    .await?;
    ResponseWrapper::json(items)
}

/// One commodity: how much of it the Assets and Liabilities accounts hold and in which lots, its
/// latest price in the operating currency, and all its prices. An unknown commodity is a 404.
#[api(group = "commodity")]
pub async fn get_single_commodity(ledger: State<SharedLedger>, params: Path<(String,)>) -> ApiResult<CommodityDetailEntity> {
    let commodity_name = params.0 .0;
    let detail = with_ledger(&ledger, move |ledger| {
        let Some((commodity, group)) = stored_commodities(ledger, Some(&commodity_name)).into_iter().next() else {
            return Ok(None);
        };
        single_commodity(ledger, commodity, group).map(Some)
    })
    .await?;
    match detail {
        Some(detail) => ResponseWrapper::json(detail),
        None => ResponseWrapper::not_found(),
    }
}

fn single_commodity(ledger: &Ledger, commodity: CommodityDomain, group: Option<String>) -> ServerResult<CommodityDetailEntity> {
    let commodity_param = || Params::new().bind("commodity", commodity.name.as_str());
    let total = execute(ledger, "commodities.total", &commodity_param(), false)?;
    let total = first_row(&total).and_then(|row| row.decimal("total")).unwrap_or_default();
    let params = commodity_param().bind("currency", ledger.options.operating_currency.as_str());
    let latest_price = execute(ledger, "commodities.latest_price", &params, false)?;

    let lots = execute(ledger, "commodities.lots", &commodity_param(), false)?;
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

    let prices = execute(ledger, "commodities.prices", &commodity_param(), false)?;
    let prices = rows(&prices)
        .filter_map(|row| {
            Some(CommodityPriceEntity {
                datetime: row.datetime("date", "time")?,
                amount: row.amount("amount")?,
            })
        })
        .collect_vec();

    let info = commodity_item(commodity, group, total, first_row(&latest_price).as_ref());
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
