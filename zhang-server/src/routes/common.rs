use std::sync::Arc;

use actix_web::web::{Data, Query};
use actix_web::{get, Responder};
use itertools::Itertools;
use tokio::sync::RwLock;
use zhang_core::domains::schemas::{ErrorDomain, OptionDomain};
use zhang_core::ledger::Ledger;

use crate::broadcast::Broadcaster;
use crate::request::JournalRequest;
use crate::response::{BasicInfo, Pageable, ResponseWrapper};
use crate::ApiResult;

#[get("/api/sse")]
pub async fn sse(broadcaster: Data<Broadcaster>) -> impl Responder {
    broadcaster.new_client().await
}

#[get("/api/info")]
pub async fn get_basic_info(ledger: Data<Arc<RwLock<Ledger>>>) -> ApiResult<BasicInfo> {
    let ledger = ledger.read().await;
    let mut operations = ledger.operations();

    ResponseWrapper::json(BasicInfo {
        title: operations.option("title")?.map(|it| it.value),
        version: env!("CARGO_PKG_VERSION").to_string(),
        build_date: env!("ZHANG_BUILD_DATE").to_string(),
    })
}

#[get("api/errors")]
pub async fn get_errors(ledger: Data<Arc<RwLock<Ledger>>>, params: Query<JournalRequest>) -> ApiResult<Pageable<ErrorDomain>> {
    let ledger = ledger.read().await;
    let mut operations = ledger.operations();
    let errors = operations.errors()?;
    let total_count = errors.len();
    let ret = errors
        .iter()
        .skip(params.offset() as usize)
        .take(params.limit() as usize)
        .cloned()
        .collect_vec();
    ResponseWrapper::json(Pageable::new(total_count as u32, params.page(), params.limit(), ret))
}

#[get("/api/options")]
pub async fn get_all_options(ledger: Data<Arc<RwLock<Ledger>>>) -> ApiResult<Vec<OptionDomain>> {
    let ledger = ledger.read().await;
    let mut operations = ledger.operations();
    let options = operations.options()?;
    ResponseWrapper::json(options)
}

#[get("/api/store")]
pub async fn get_store_data(ledger: Data<Arc<RwLock<Ledger>>>) -> ApiResult<serde_json::Value> {
    let ledger = ledger.read().await;
    let store = ledger.store.read().unwrap();
    let value = serde_json::to_value(&*store).unwrap();
    ResponseWrapper::json(value)
}
