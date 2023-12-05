use std::sync::Arc;

use actix_web::web::{Data, Json, Path};
use actix_web::{get, put};
use itertools::Either;
use log::error;
use tokio::sync::RwLock;
use zhang_core::ledger::Ledger;

use crate::request::FileUpdateRequest;
use crate::response::{FileDetailResponse, ResponseWrapper};
use crate::ApiResult;

#[get("/api/files")]
pub async fn get_files(ledger: Data<Arc<RwLock<Ledger>>>) -> ApiResult<Vec<Option<String>>> {
    let ledger = ledger.read().await;
    let entry_path = &ledger.entry.0;

    let mut ret = vec![];
    for visited in &ledger.visited_files {
        match visited {
            Either::Left(pattern) => {
                for entry in glob::glob(pattern.as_str()).unwrap() {
                    match entry {
                        Ok(path) => {
                            let p = path.strip_prefix(entry_path).unwrap().to_str().map(|it| it.to_string());
                            ret.push(p);
                        }
                        Err(e) => error!("{:?}", e),
                    }
                }
            }
            Either::Right(path) => {
                ret.push(path.to_str().map(|it| it.to_string()));
            }
        }
    }
    ResponseWrapper::json(ret)
}

#[get("/api/files/{file_path}")]
pub async fn get_file_content(ledger: Data<Arc<RwLock<Ledger>>>, path: Path<(String,)>) -> ApiResult<FileDetailResponse> {
    let encoded_file_path = path.into_inner().0;
    let filename = String::from_utf8(base64::decode(encoded_file_path).unwrap()).unwrap();
    let ledger = ledger.read().await;
    let entry = &ledger.entry.0;
    let full_path = entry.join(&filename);

    ResponseWrapper::json(FileDetailResponse {
        path: filename,
        content: std::fs::read_to_string(full_path)?,
    })
}

#[put("/api/files/{file_path}")]
pub async fn update_file_content(ledger: Data<Arc<RwLock<Ledger>>>, path: Path<(String,)>, Json(payload): Json<FileUpdateRequest>) -> ApiResult<()> {
    let encoded_file_path = path.into_inner().0;
    let filename = String::from_utf8(base64::decode(encoded_file_path).unwrap()).unwrap();
    let ledger = ledger.read().await;
    let entry = &ledger.entry.0;
    let full_path = entry.join(&filename);

    // todo(refact) test the syntax valid
    // if parse_zhang(&payload.content, None).is_ok() {
    std::fs::write(full_path, payload.content)?;
    // }
    ResponseWrapper::<()>::created()
}
