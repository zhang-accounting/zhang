//! The commodity pages. What a commodity is (precision, prefix, suffix, rounding, group) is read
//! from the options and the `commodity` directives; its holdings, lots and prices come from [built-in queries](crate::builtin)
//! (`commodities.*`).

use std::collections::HashMap;

use axum::extract::{Path, State};
use bigdecimal::BigDecimal;
use gotcha::api;
use zhang_ast::amount::Amount;
use zhang_core::domains::schemas::CommodityDomain;
use zhang_core::ledger::Ledger;
use zhang_query::{Params, QueryResult};

use crate::builtin::execute;
use crate::cells::{first_row, rows, Row};
use crate::response::{CommodityDetailEntity, CommodityListItemEntity, CommodityLotEntity, CommodityPriceEntity, ResponseWrapper};
use crate::routes::query::with_ledger;
use crate::state::SharedLedger;
use crate::{ApiResult, ServerResult};

/// A commodity of the list, from the ledger's commodity, its group, its total and its latest
/// price: a row of `commodities.latest_price(s)`, the engine's rate of one unit in `currency` as
/// of today and the date and time of the quote it comes from.
fn commodity_item(
    commodity: CommodityDomain, group: Option<String>, total: BigDecimal, latest_price: Option<&Row<'_>>, currency: &str,
) -> ServerResult<CommodityListItemEntity> {
    let (latest_rate, latest_price_date) = match latest_price {
        Some(row) => (row.decimal("rate")?, row.datetime("date", "time")?),
        None => (None, None),
    };
    let latest_price_commodity = latest_rate.is_some().then(|| currency.to_owned());
    Ok(CommodityListItemEntity {
        name: commodity.name,
        precision: commodity.precision,
        prefix: commodity.prefix,
        suffix: commodity.suffix,
        rounding: commodity.rounding.to_string(),
        group,
        total_amount: total,
        latest_price_date,
        latest_price_amount: latest_rate,
        latest_price_commodity,
    })
}

/// The commodities of the ledger (all of them, or the one named `name`), in the order they were first defined, with
/// their groups ([`Ledger::commodities`]).
fn stored_commodities(ledger: &Ledger, name: Option<&str>) -> Vec<(CommodityDomain, Option<String>)> {
    let mut commodities = ledger.commodities();
    commodities.retain(|(commodity, _)| name.is_none_or(|name| commodity.name == name));
    commodities
}

/// The rows of the result of `query` by their `currency`.
fn by_currency<'a>(query: &str, result: &'a QueryResult) -> ServerResult<HashMap<String, Row<'a>>> {
    let mut by_currency = HashMap::new();
    for row in rows(query, result) {
        if let Some(currency) = row.str("currency")? {
            by_currency.insert(currency, row);
        }
    }
    Ok(by_currency)
}

/// Every commodity, with how much of it the Assets and Liabilities accounts hold and its latest
/// price in the operating currency.
#[api(group = "commodity")]
pub async fn get_all_commodities(ledger: State<SharedLedger>) -> ApiResult<Vec<CommodityListItemEntity>> {
    let items = with_ledger(&ledger, |ledger| {
        let commodities = stored_commodities(ledger, None);
        let totals = execute(ledger, "commodities.totals", &Params::new(), false)?;
        let totals = by_currency("commodities.totals", &totals)?;
        let params = Params::new().bind("currency", ledger.options.operating_currency.as_str());
        let prices = execute(ledger, "commodities.latest_prices", &params, false)?;
        let prices = by_currency("commodities.latest_prices", &prices)?;
        commodities
            .into_iter()
            .map(|(commodity, group)| {
                let total = match totals.get(&commodity.name) {
                    Some(row) => row.decimal("total")?.unwrap_or_default(),
                    None => BigDecimal::default(),
                };
                let latest_price = prices.get(&commodity.name);
                commodity_item(commodity, group, total, latest_price, &ledger.options.operating_currency)
            })
            .collect()
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
    let total = match first_row("commodities.total", &total) {
        Some(row) => row.decimal("total")?.unwrap_or_default(),
        None => BigDecimal::default(),
    };
    let params = commodity_param().bind("currency", ledger.options.operating_currency.as_str());
    let latest_price = execute(ledger, "commodities.latest_price", &params, false)?;

    let lots = execute(ledger, "commodities.lots", &commodity_param(), false)?;
    let lots = rows("commodities.lots", &lots)
        .map(|row| {
            Ok(CommodityLotEntity {
                account: row.str("account")?.unwrap_or_default(),
                amount: row.decimal("units")?.unwrap_or_default(),
                cost: row
                    .decimal("cost_number")?
                    .zip(row.str("cost_currency")?)
                    .map(|(number, currency)| Amount::new(number, currency)),
                price: None,
                acquisition_date: row.date("cost_date")?,
                // `cost_label` is "" for a lot held without cost and NULL for a cost lot without a label
                label: row.str("cost_label")?.filter(|label| !label.is_empty()),
            })
        })
        .collect::<ServerResult<Vec<_>>>()?;

    let quotes = execute(ledger, "commodities.prices", &commodity_param(), false)?;
    let mut prices = vec![];
    for row in rows("commodities.prices", &quotes) {
        if let Some((datetime, amount)) = row.datetime("date", "time")?.zip(row.amount("amount")?) {
            prices.push(CommodityPriceEntity { datetime, amount });
        }
    }

    let info = commodity_item(
        commodity,
        group,
        total,
        first_row("commodities.latest_price", &latest_price).as_ref(),
        &ledger.options.operating_currency,
    )?;
    Ok(CommodityDetailEntity { info, lots, prices })
}
