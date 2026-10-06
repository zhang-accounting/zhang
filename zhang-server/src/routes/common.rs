use std::convert::Infallible;

use async_stream::try_stream;
use axum::extract::State;
use axum::http::{Method, StatusCode, Uri};
use axum::response::sse::{Event, KeepAlive};
use axum::response::{IntoResponse, Response, Sse};
use futures_util::Stream;
use gotcha::api;
use zhang_core::domains::schemas::OptionDomain;

use crate::error::{error_response, ServerError};
use crate::response::{BasicInfoEntity, ResponseWrapper};
use crate::state::{SharedBroadcaster, SharedLedger, SharedReloadSender};
use crate::ApiResult;

/// Every path no route takes. An `/api` path is a JSON 404 that names the method and the path, in every build: a
/// script that calls an endpoint that is gone (the typed read endpoints retire in favour of the built-in queries, #754)
/// gets an error it can read, never a page. Any other path is the frontend's, its single-page app, or in a build
/// without the frontend a note that says so.
pub async fn fallback(method: Method, uri: Uri) -> Response {
    let path = uri.path();
    if path == "/api" || path.starts_with("/api/") {
        return error_response(StatusCode::NOT_FOUND, format!("no route {} {}", method, path));
    }
    #[cfg(feature = "frontend")]
    {
        crate::routes::frontend::serve_frontend(uri).await.into_response()
    }
    #[cfg(not(feature = "frontend"))]
    {
        "hello zhang,\n\
         seems you are trying to access the frontend UI, but the feature of frontend is not enable.\n\
         try to enable the feature and compile again"
            .into_response()
    }
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

    ResponseWrapper::json(BasicInfoEntity {
        title: ledger.options.option::<String>("title")?,
        version: env!("ZHANG_BUILD_VERSION").to_string(),
        build_date: env!("ZHANG_BUILD_DATE").to_string(),
        format: ledger.dialect.name().to_owned(),
        reload_failure: reload_sender.last_failure(),
    })
}

#[api(group = "common")]
pub async fn get_all_options(ledger: State<SharedLedger>) -> ApiResult<Vec<OptionDomain>> {
    let ledger = ledger.read().await;
    ResponseWrapper::json(ledger.options.all())
}
