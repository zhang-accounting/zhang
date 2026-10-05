use std::convert::Infallible;

use async_stream::try_stream;
use axum::extract::{Query, State};
use axum::response::sse::{Event, KeepAlive};
use axum::response::Sse;
use futures_util::Stream;
use gotcha::api;
use zhang_core::domains::schemas::OptionDomain;

use crate::error::ServerError;
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

/// `POST /api/reload`: reload the ledger from its files, answered once it is done. A reload that fails, as a syntax
/// error makes it, is answered with the reason (HTTP 409): the ledger served is the one loaded before it (#492).
#[api(group = "common")]
pub async fn reload(State(reload_sender): State<SharedReloadSender>) -> ApiResult<String> {
    reload_sender.reload_and_wait().await.map_err(ServerError::ReloadFailed)?;
    ResponseWrapper::json("Ok".to_string())
}

#[api(group = "common")]
pub async fn get_basic_info(ledger: State<SharedLedger>, reload_sender: State<SharedReloadSender>) -> ApiResult<BasicInfoEntity> {
    let ledger = ledger.read().await;
    let operations = ledger.operations();

    ResponseWrapper::json(BasicInfoEntity {
        title: operations.option::<String>("title")?,
        version: env!("ZHANG_BUILD_VERSION").to_string(),
        build_date: env!("ZHANG_BUILD_DATE").to_string(),
        format: ledger.dialect.name().to_owned(),
        reload_failure: reload_sender.last_failure(),
    })
}

/// The ledger's errors, one page at a time, by file and then by position in the file: the built-in
/// query `journals.errors`. A page has 1 to 1000 errors (`size`, 100 by default); another size is a bad request, and a
/// page past the last one is empty.
#[api(group = "error")]
pub async fn get_errors(ledger: State<SharedLedger>, params: Query<JournalRequest>) -> ApiResult<Pageable<ErrorEntity>> {
    ResponseWrapper::json(journals::errors(&ledger, params.0).await?)
}

#[api(group = "common")]
pub async fn get_all_options(ledger: State<SharedLedger>) -> ApiResult<Vec<OptionDomain>> {
    let ledger = ledger.read().await;
    let mut operations = ledger.operations();
    let options = operations.options()?;
    ResponseWrapper::json(options)
}
