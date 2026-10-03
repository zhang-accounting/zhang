use std::convert::Infallible;

use async_stream::try_stream;
use axum::extract::{Query, State};
use axum::response::sse::{Event, KeepAlive};
use axum::response::Sse;
use futures_util::Stream;
use gotcha::api;
use itertools::Itertools;
use zhang_core::domains::schemas::OptionDomain;

use crate::request::JournalRequest;
use crate::response::{BasicInfoEntity, ErrorEntity, Pageable, ResponseWrapper};
use crate::state::{SharedBroadcaster, SharedLedger, SharedReloadSender};
use crate::{journals, ApiResult};

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
        format: if zhang_core::data_type::is_beancount_endpoint(&ledger.entry.1) {
            "beancount"
        } else {
            "zhang"
        }
        .to_owned(),
    })
}

/// The ledger's errors, one page at a time, by file and then by position in the file: the built-in
/// query `journals.errors`. A page size of 0, or a page beyond what an offset can count, is a bad request.
#[api(group = "error")]
pub async fn get_errors(ledger: State<SharedLedger>, params: Query<JournalRequest>) -> ApiResult<Pageable<ErrorEntity>> {
    ResponseWrapper::json(journals::errors(&ledger, params.0).await?)
}

/// The hand-written [`get_errors`] the built-in query replaces, kept to compare them until it is
/// removed (#479).
pub async fn get_errors_legacy(ledger: State<SharedLedger>, params: Query<JournalRequest>) -> ApiResult<Pageable<ErrorEntity>> {
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
pub async fn get_all_options(ledger: State<SharedLedger>) -> ApiResult<Vec<OptionDomain>> {
    let ledger = ledger.read().await;
    let mut operations = ledger.operations();
    let options = operations.options()?;
    ResponseWrapper::json(options)
}

pub async fn get_store_data(ledger: State<SharedLedger>) -> ApiResult<serde_json::Value> {
    let ledger = ledger.read().await;
    let store = ledger.store.read().unwrap();
    let value = serde_json::to_value(&*store).unwrap();
    ResponseWrapper::json(value)
}
