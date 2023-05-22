use std::collections::HashMap;

use actix_web::body::EitherBody;
use actix_web::http::StatusCode;
use actix_web::{HttpRequest, HttpResponse, Responder, ResponseError};
use chrono::{NaiveDate, NaiveDateTime};
use serde::Serialize;
use sqlx::FromRow;
use zhang_ast::amount::Amount;
use zhang_ast::Currency;
use zhang_core::database::type_ext::big_decimal::ZhangBigDecimal;
use zhang_core::domains::schemas::{AccountJournalDomain, MetaDomain};

use crate::{ServerError, ServerResult};

pub enum ResponseWrapper<T: Serialize> {
    Json(T),
    Created,
}

impl<T: Serialize> ResponseWrapper<T> {
    pub fn json(data: T) -> ServerResult<ResponseWrapper<T>> {
        Ok(ResponseWrapper::Json(data))
    }
    pub fn created() -> ServerResult<ResponseWrapper<()>> {
        Ok(ResponseWrapper::Created)
    }
}

impl<T: Serialize> Responder for ResponseWrapper<T> {
    type Body = EitherBody<String>;

    fn respond_to(self, req: &HttpRequest) -> HttpResponse<Self::Body> {
        #[derive(Serialize)]
        pub struct SuccessWrapper<T: Serialize> {
            data: T,
        }
        match self {
            ResponseWrapper::Json(data) => {
                let wrapper = SuccessWrapper { data };
                let json = actix_web::web::Json(wrapper);
                json.respond_to(req)
            }
            ResponseWrapper::Created => HttpResponse::Created().message_body(EitherBody::new("".to_string())).unwrap(),
        }
    }
}

impl ResponseError for ServerError {
    fn status_code(&self) -> StatusCode {
        StatusCode::INTERNAL_SERVER_ERROR
    }
}

#[derive(Serialize)]
pub struct Pageable<T: Serialize> {
    pub total_count: u32,
    pub total_page: u32,
    pub page_size: u32,
    pub current_page: u32,
    pub records: Vec<T>,
}

impl<T: Serialize> Pageable<T> {
    pub fn new(total_count: u32, page: u32, size: u32, records: Vec<T>) -> Self {
        let total_page = total_count / size + u32::from(total_count % size != 0);
        Self {
            total_count,
            total_page,
            page_size: size,
            current_page: page,
            records,
        }
    }
}

#[derive(Serialize)]
pub struct AccountResponse {
    pub name: String,
    pub status: String,
    pub commodities: HashMap<Currency, ZhangBigDecimal>,
}

#[derive(Serialize, FromRow)]
pub struct DocumentResponse {
    pub datetime: NaiveDateTime,
    pub filename: String,
    pub path: String,
    pub extension: Option<String>,
    pub account: Option<String>,
    pub trx_id: Option<String>,
}

#[derive(Serialize)]
pub struct StatisticFrameResponse {
    datetime: NaiveDateTime,
    amount: ZhangBigDecimal,
    commodity: String,
}

#[derive(Serialize)]
pub struct StatisticResponse {
    pub changes: HashMap<NaiveDate, HashMap<String, AmountResponse>>, // summaries:
    pub details: HashMap<NaiveDate, HashMap<String, AmountResponse>>,
}

#[derive(Serialize, FromRow)]
pub struct MetaResponse {
    key: String,
    value: String,
}
impl From<MetaDomain> for MetaResponse {
    fn from(value: MetaDomain) -> Self {
        MetaResponse {
            key: value.key,
            value: value.value,
        }
    }
}

#[derive(Serialize)]
#[serde(tag = "type")]
pub enum JournalItemResponse {
    Transaction(JournalTransactionItemResponse),
    BalanceCheck(JournalBalanceCheckItemResponse),
    BalancePad(JournalBalancePadItemResponse),
}

impl JournalItemResponse {
    pub fn sequence(&self) -> i64 {
        match self {
            JournalItemResponse::Transaction(inner) => inner.sequence,
            JournalItemResponse::BalanceCheck(inner) => inner.sequence,
            JournalItemResponse::BalancePad(inner) => inner.sequence,
        }
    }
}

#[derive(Serialize)]
pub struct JournalTransactionItemResponse {
    pub id: String,
    pub sequence: i64,
    pub datetime: NaiveDateTime,
    pub payee: String,
    pub narration: Option<String>,
    pub tags: Vec<String>,
    pub links: Vec<String>,
    pub flag: String,
    pub is_balanced: bool,
    pub postings: Vec<JournalTransactionPostingResponse>,
    pub metas: Vec<MetaResponse>,
}
#[derive(Serialize)]
pub struct JournalTransactionPostingResponse {
    pub account: String,
    pub unit_number: Option<ZhangBigDecimal>,
    pub unit_commodity: Option<String>,
    pub cost_number: Option<ZhangBigDecimal>,
    pub cost_commodity: Option<String>,
    pub price_number: Option<ZhangBigDecimal>,
    pub price_commodity: Option<String>,
    pub inferred_unit_number: ZhangBigDecimal,
    pub inferred_unit_commodity: String,
    pub account_before_number: ZhangBigDecimal,
    pub account_before_commodity: String,
    pub account_after_number: ZhangBigDecimal,
    pub account_after_commodity: String,
}

#[derive(Serialize)]
pub struct JournalBalanceCheckItemResponse {
    pub id: String,
    pub sequence: i64,
    pub datetime: NaiveDateTime,
    pub payee: String,
    pub narration: Option<String>,
    pub type_: String,
    pub(crate) postings: Vec<JournalTransactionPostingResponse>,
}

#[derive(Serialize)]
pub struct JournalBalancePadItemResponse {
    pub id: String,
    pub sequence: i64,
    pub datetime: NaiveDateTime,
    pub payee: String,
    pub narration: Option<String>,
    pub type_: String,
    pub(crate) postings: Vec<JournalTransactionPostingResponse>,
}

#[derive(Serialize)]
pub struct InfoForNewTransaction {
    pub payee: Vec<String>,
    pub account_name: Vec<String>,
}

#[derive(Serialize)]
pub struct CalculatedAmount {
    pub calculated: AmountResponse,
    pub detail: HashMap<String, ZhangBigDecimal>,
}

#[derive(Serialize, Clone)]
pub struct AmountResponse {
    pub number: ZhangBigDecimal,
    pub commodity: String,
}

impl From<Amount> for AmountResponse {
    fn from(value: Amount) -> Self {
        AmountResponse {
            number: ZhangBigDecimal(value.number),
            commodity: value.currency,
        }
    }
}

#[derive(FromRow, Serialize)]
pub struct CommodityListItemResponse {
    pub name: String,
    pub precision: i32,
    pub prefix: Option<String>,
    pub suffix: Option<String>,
    pub rounding: Option<String>,
    pub total_amount: ZhangBigDecimal,
    pub latest_price_date: Option<NaiveDateTime>,
    pub latest_price_amount: Option<ZhangBigDecimal>,
    pub latest_price_commodity: Option<String>,
}

#[derive(FromRow, Serialize)]
pub struct CommodityLot {
    pub datetime: Option<NaiveDateTime>,
    pub amount: ZhangBigDecimal,
    pub price_amount: Option<ZhangBigDecimal>,
    pub price_commodity: Option<String>,
    pub account: String,
}

#[derive(FromRow, Serialize)]
pub struct CommodityPrice {
    pub datetime: NaiveDateTime,
    pub amount: ZhangBigDecimal,
    pub target_commodity: Option<String>,
}

#[derive(Serialize)]
pub struct CommodityDetailResponse {
    pub info: CommodityListItemResponse,
    pub lots: Vec<CommodityLot>,
    pub prices: Vec<CommodityPrice>,
}

#[derive(Serialize)]
pub struct FileDetailResponse {
    pub path: String,
    pub content: String,
}

#[derive(Serialize)]
pub struct CurrentStatisticResponse {
    pub balance: CalculatedAmount,
    pub liability: CalculatedAmount,
    pub income: AmountResponse,
    pub expense: AmountResponse,
}

#[derive(Serialize)]
pub struct ReportResponse {
    pub from: NaiveDateTime,
    pub to: NaiveDateTime,

    pub balance: AmountResponse,
    pub liability: AmountResponse,
    pub income: AmountResponse,
    pub expense: AmountResponse,
    pub transaction_number: i64,

    pub income_rank: Vec<ReportRankItemResponse>,
    pub income_top_transactions: Vec<AccountJournalDomain>,
    pub expense_rank: Vec<ReportRankItemResponse>,
    pub expense_top_transactions: Vec<AccountJournalDomain>,
}
#[derive(Serialize)]
pub struct ReportRankItemResponse {
    pub account: String,
    pub percent: ZhangBigDecimal,
}

#[derive(Serialize)]
pub struct BasicInfo {
    pub title: Option<String>,
    pub version: String,
    pub build_date: String,
}
