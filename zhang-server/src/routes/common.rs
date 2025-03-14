use std::convert::Infallible;

use async_stream::try_stream;
use axum::extract::{Query, State};
use axum::response::sse::{Event, KeepAlive};
use axum::response::Sse;
use futures_util::Stream;
use gotcha::api;
use itertools::Itertools;

use crate::request::JournalRequest;
use crate::response::{BasicInfoEntity, ErrorEntity, OptionEntity, Pageable, ResponseWrapper};
use crate::state::{SharedBroadcaster, SharedLedger, SharedReloadSender};
use crate::ApiResult;

pub async fn backend_only_info() -> &'static str {
    "hello zhang,\n\
    seems you are trying to access the frontend UI, but the feature of frontend is not enable.\n\
    try to enable the feature and compile again"
}

pub async fn sse(broadcaster: State<SharedBroadcaster>) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let mut receiver = broadcaster.0.new_client().await;
    Sse::new(try_stream! {
        loop {
            if let Some(event) = receiver.recv().await { yield event; }
        }
    })
    .keep_alive(KeepAlive::default())
}

#[api(group = "common")]
pub async fn reload(State(reload_sender): State<SharedReloadSender>) -> ApiResult<String> {
    reload_sender.reload();
    ResponseWrapper::json("Ok".to_string())
}

#[api(group = "common")]
pub async fn get_basic_info(ledger: State<SharedLedger>) -> ApiResult<BasicInfoEntity> {
    let ledger = ledger.read().await;
    let operations = ledger.operations();

    ResponseWrapper::json(BasicInfoEntity {
        title: operations.option::<String>("title")?,
        version: env!("ZHANG_BUILD_VERSION").to_string(),
        build_date: env!("ZHANG_BUILD_DATE").to_string(),
    })
}

#[api(group = "error")]
pub async fn get_errors(ledger: State<SharedLedger>, params: Query<JournalRequest>) -> ApiResult<Pageable<ErrorEntity>> {
    let ledger = ledger.read().await;
    let mut operations = ledger.operations();
    let errors = operations.errors()?;
    let total_count = errors.len();
    let ret = errors
        .iter()
        .skip(params.offset() as usize)
        .take(params.limit() as usize)
        .cloned()
        .map(|it| it.into())
        .collect_vec();
    ResponseWrapper::json(Pageable::new(total_count as u32, params.page(), params.limit(), ret))
}

#[api(group = "common")]
pub async fn get_all_options(ledger: State<SharedLedger>) -> ApiResult<Vec<OptionEntity>> {
    let ledger = ledger.read().await;
    let mut operations = ledger.operations();
    let options = operations.options()?;
    ResponseWrapper::json(options.into_iter().map(|it| it.into()).collect_vec())
}

pub async fn get_store_data(ledger: State<SharedLedger>) -> ApiResult<serde_json::Value> {
    let ledger = ledger.read().await;
    let store = ledger.store.read().unwrap();
    let value = serde_json::to_value(&*store).unwrap();
    ResponseWrapper::json(value)
}
