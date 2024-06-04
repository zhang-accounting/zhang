use std::sync::Arc;

use axum::extract::{Path, State};
use itertools::Itertools;
use tokio::sync::RwLock;
use zhang_core::constants::COMMODITY_GROUP;
use zhang_core::domains::schemas::{CommodityDomain, MetaType};
use zhang_core::ledger::Ledger;

use crate::response::{CommodityDetailResponse, CommodityListItemResponse, CommodityLotResponse, CommodityPrice, ResponseWrapper};
use crate::ApiResult;

pub async fn get_all_commodities(ledger: State<Arc<RwLock<Ledger>>>) -> ApiResult<Vec<CommodityListItemResponse>> {
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
        ret.push(CommodityListItemResponse {
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

pub async fn get_single_commodity(ledger: State<Arc<RwLock<Ledger>>>, params: Path<(String,)>) -> ApiResult<CommodityDetailResponse> {
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
    let commodity_item = CommodityListItemResponse {
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
        .map(|it| CommodityLotResponse {
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
        .map(|price| CommodityPrice {
            datetime: price.datetime,
            amount: price.amount,
            target_commodity: Some(price.target_commodity),
        })
        .collect_vec();

    ResponseWrapper::json(CommodityDetailResponse {
        info: commodity_item,
        lots,
        prices,
    })
}
