use std::convert::Infallible;
use std::sync::Arc;

use async_stream::try_stream;
use axum::extract::{Query, State};
use axum::response::sse::{Event, KeepAlive};
use axum::response::Sse;
use futures_util::Stream;
use itertools::Itertools;
use tokio::sync::RwLock;

use zhang_core::domains::schemas::{ErrorDomain, OptionDomain};
use zhang_core::ledger::Ledger;

use crate::broadcast::Broadcaster;
use crate::request::JournalRequest;
use crate::response::{BasicInfo, Pageable, ResponseWrapper};
use crate::{ApiResult, ReloadSender};

pub async fn sse(broadcaster: State<Arc<Broadcaster>>) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let mut receiver = broadcaster.new_client().await;
    Sse::new(try_stream! {
        loop {
            if let Some(event) = receiver.recv().await { yield event; }
        }
    })
    .keep_alive(KeepAlive::default())
}

pub async fn reload(reload_sender: State<Arc<ReloadSender>>) -> ApiResult<String> {
    reload_sender.0 .0.try_send(1).ok();
    ResponseWrapper::json("Ok".to_string())
}

pub async fn get_basic_info(ledger: State<Arc<RwLock<Ledger>>>) -> ApiResult<BasicInfo> {
    let ledger = ledger.read().await;
    let mut operations = ledger.operations();

    ResponseWrapper::json(BasicInfo {
        title: operations.option("title")?.map(|it| it.value),
        version: env!("CARGO_PKG_VERSION").to_string(),
        build_date: env!("ZHANG_BUILD_DATE").to_string(),
    })
}

pub async fn get_errors(ledger: State<Arc<RwLock<Ledger>>>, params: Query<JournalRequest>) -> ApiResult<Pageable<ErrorDomain>> {
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

pub async fn get_all_options(ledger: State<Arc<RwLock<Ledger>>>) -> ApiResult<Vec<OptionDomain>> {
    let ledger = ledger.read().await;
    let mut operations = ledger.operations();
    let options = operations.options()?;
    ResponseWrapper::json(options)
}

pub async fn get_store_data(ledger: State<Arc<RwLock<Ledger>>>) -> ApiResult<serde_json::Value> {
    let ledger = ledger.read().await;
    let store = ledger.store.read().unwrap();
    let value = serde_json::to_value(&*store).unwrap();
    ResponseWrapper::json(value)
}
