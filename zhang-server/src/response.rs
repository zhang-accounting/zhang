use std::collections::{BTreeMap, HashMap};

use axum::response::{IntoResponse, Response};
use axum::Json;
use bigdecimal::BigDecimal;
use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use gotcha::oas::{Referenceable, Responses};
use gotcha::{Responsable, Schematic};
use serde::Serialize;
use uuid::Uuid;
use zhang_ast::amount::{Amount, CalculatedAmount};
use zhang_ast::{AccountType, Currency};
use zhang_core::domains::schemas::{AccountJournalDomain, AccountStatus, MetaDomain, OptionDomain};
use zhang_core::plugin::PluginType;
use zhang_core::store::{BudgetEvent, PostingDomain};

use crate::error::ServerError;
use crate::ServerResult;
use zhang_core::store::BudgetEventType;
use zhang_ast::SpanInfo;
use zhang_core::domains::schemas::ErrorDomain;


pub struct Created;


impl Responsable for Created {
    fn response() -> Responses {
        let mut response = Responses {
            default: None,
            data: BTreeMap::default(),
        };
        response.data.insert(
            "204".to_string(),
            Referenceable::Data(gotcha::oas::Response {
                description: "no content".to_string(),
                headers: None,
                content: None,
                links: None,
            }),
        );
        response
    }
}

impl IntoResponse for Created {
    fn into_response(self) -> Response {
        (axum::http::StatusCode::CREATED, "").into_response()
    }
}

#[derive(Serialize, Schematic)]
pub struct ResponseWrapper<T: Serialize + Schematic> {
    pub data: T,
}


impl<T: Serialize + Schematic> ResponseWrapper<T> {
    pub fn json(data: T) -> ServerResult<Self> {
        Ok(Self { data })
    }

  
    pub fn not_found() -> ServerResult<Self> {
        Err(ServerError::NotFound)
    }

    pub fn bad_request() -> ServerResult<Self> {
        Err(ServerError::BadRequest)
    }
    

}

impl <T: Serialize + Schematic> IntoResponse for ResponseWrapper<T> {
    fn into_response(self) -> Response {
        (axum::http::StatusCode::OK, Json(self)).into_response()
    }
}





#[derive(Serialize, Schematic)]
pub struct Pageable<T: Serialize + Schematic> {
    pub total_count: u32,
    pub total_page: u32,
    pub page_size: u32,
    pub current_page: u32,
    pub records: Vec<T>,
}

impl<T: Serialize + Schematic> Pageable<T> {
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


#[derive(Serialize, Schematic)]
/// the status of the account
pub enum AccountStatusEntity {
    /// the account is open
    Open,
    /// the account is close
    Close,
}

impl From<AccountStatus> for AccountStatusEntity {
    fn from(value: AccountStatus) -> Self {
        match value {
            AccountStatus::Open => AccountStatusEntity::Open,
            AccountStatus::Close => AccountStatusEntity::Close,
        }
    }
}


#[derive(Serialize, Schematic)]
/// represent the number and currency pair
pub struct AmountEntity {
    /// the number of the amount
    pub number: BigDecimal,
    /// the currency of the amount
    pub currency: String,
}

impl From<Amount> for AmountEntity {
    fn from(value: Amount) -> Self {
        AmountEntity {
            number: value.number,
            currency: value.currency,
        }
    }
}


#[derive(Serialize, Schematic)]
/// represent the calculated amount, normally used for multiple currency
pub struct CalculatedAmountEntity {
    /// the calculated amount
    pub calculated: AmountEntity,
    /// the detail of the calculated amount
    pub detail: HashMap<String, BigDecimal>,
}

impl From<CalculatedAmount> for CalculatedAmountEntity {
    fn from(value: CalculatedAmount) -> Self {
        CalculatedAmountEntity {
            calculated: value.calculated.into(),
            detail: value.detail,
        }
    }
}


#[derive(Serialize, Schematic)]
pub struct AccountResponse {
    pub name: String,
    pub status: AccountStatusEntity,
    pub alias: Option<String>,
    pub amount: CalculatedAmountEntity,
}

#[derive(Serialize, Schematic)]
pub struct DocumentResponse {
    pub datetime: NaiveDateTime,
    pub filename: String,
    pub path: String,
    pub extension: Option<String>,
    pub account: Option<String>,
    pub trx_id: Option<String>,
}

#[derive(Serialize, Schematic)]
pub struct StatisticResponse {
    pub changes: HashMap<NaiveDate, HashMap<String, AmountResponse>>, // summaries:
    pub details: HashMap<NaiveDate, HashMap<String, AmountResponse>>,
}

#[derive(Serialize, Schematic)]
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

#[derive(Serialize, Schematic)]
#[serde(tag = "type")]
pub enum JournalItemResponse {
    Transaction(JournalTransactionItemResponse),
    BalanceCheck(JournalBalanceCheckItemResponse),
    BalancePad(JournalBalancePadItemResponse),
}

impl JournalItemResponse {
    pub fn sequence(&self) -> i32 {
        match self {
            JournalItemResponse::Transaction(inner) => inner.sequence,
            JournalItemResponse::BalanceCheck(inner) => inner.sequence,
            JournalItemResponse::BalancePad(inner) => inner.sequence,
        }
    }
}

#[derive(Serialize, Schematic)]
pub struct JournalTransactionItemResponse {
    pub id: Uuid,
    pub sequence: i32,
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
#[derive(Serialize, Schematic)]
pub struct JournalTransactionPostingResponse {
    pub account: String,
    pub unit_number: Option<BigDecimal>,
    pub unit_commodity: Option<String>,
    pub cost_number: Option<BigDecimal>,
    pub cost_commodity: Option<String>,
    pub inferred_unit_number: BigDecimal,
    pub inferred_unit_commodity: String,
    pub account_before_number: BigDecimal,
    pub account_before_commodity: String,
    pub account_after_number: BigDecimal,
    pub account_after_commodity: String,
}

impl From<PostingDomain> for JournalTransactionPostingResponse {
    fn from(arm: PostingDomain) -> Self {
        JournalTransactionPostingResponse {
            account: arm.account.name().to_owned(),
            unit_number: arm.unit.as_ref().map(|it| it.number.clone()),
            unit_commodity: arm.unit.as_ref().map(|it| it.currency.clone()),
            cost_number: arm.cost.as_ref().map(|it| it.number.clone()),
            cost_commodity: arm.cost.as_ref().map(|it| it.currency.clone()),
            inferred_unit_number: arm.inferred_amount.number,
            inferred_unit_commodity: arm.inferred_amount.currency,
            account_before_number: arm.previous_amount.number,
            account_before_commodity: arm.previous_amount.currency,
            account_after_number: arm.after_amount.number,
            account_after_commodity: arm.after_amount.currency,
        }
    }
}

#[derive(Serialize, Schematic)]
pub struct JournalBalanceCheckItemResponse {
    pub id: Uuid,
    pub sequence: i32,
    pub datetime: NaiveDateTime,
    pub payee: String,
    pub narration: Option<String>,
    pub type_: String,
    pub(crate) postings: Vec<JournalTransactionPostingResponse>,
}

#[derive(Serialize, Schematic)]
pub struct JournalBalancePadItemResponse {
    pub id: Uuid,
    pub sequence: i32,
    pub datetime: NaiveDateTime,
    pub payee: String,
    pub narration: Option<String>,
    pub type_: String,
    pub(crate) postings: Vec<JournalTransactionPostingResponse>,
}

#[derive(Serialize, Schematic)]
pub struct InfoForNewTransaction {
    pub payee: Vec<String>,
    pub account_name: Vec<String>,
}

#[derive(Serialize, Clone, Schematic)]
pub struct AmountResponse {
    pub number: BigDecimal,
    pub commodity: String,
}

impl From<Amount> for AmountResponse {
    fn from(value: Amount) -> Self {
        AmountResponse {
            number: value.number,
            commodity: value.currency,
        }
    }
}

#[derive(Serialize,  Schematic)]
pub struct CommodityListItemResponse {
    pub name: String,
    pub precision: i32,
    pub prefix: Option<String>,
    pub suffix: Option<String>,
    pub rounding: String,
    pub group: Option<String>,

    pub total_amount: BigDecimal,
    pub latest_price_date: Option<NaiveDateTime>,
    pub latest_price_amount: Option<BigDecimal>,
    pub latest_price_commodity: Option<String>,
}

#[derive(Serialize, Schematic)]
pub struct CommodityLotResponse {
    pub account: String,
    pub amount: BigDecimal,

    pub cost: Option<AmountEntity>,
    pub price: Option<AmountEntity>,
    pub acquisition_date: Option<NaiveDate>,
}

#[derive(Serialize, Schematic)]
pub struct CommodityPrice {
    pub datetime: NaiveDateTime,
    pub amount: BigDecimal,
    pub target_commodity: Option<String>,
}

#[derive(Serialize, Schematic)]
pub struct CommodityDetailResponse {
    pub info: CommodityListItemResponse,
    pub lots: Vec<CommodityLotResponse>,
    pub prices: Vec<CommodityPrice>,
}

#[derive(Serialize, Schematic)]
pub struct FileDetailResponse {
    pub path: String,
    pub content: String,
}

#[derive(Serialize, Schematic)]
pub struct StatisticSummaryResponse {
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,

    pub balance: CalculatedAmountEntity,
    pub liability: CalculatedAmountEntity,

    pub income: CalculatedAmountEntity,
    pub expense: CalculatedAmountEntity,
    pub transaction_number: i64,
}

#[derive(Serialize, Schematic)]
pub struct CurrentStatisticResponse {
    pub balance: CalculatedAmountEntity,
    pub liability: CalculatedAmountEntity,
    pub income: AmountResponse,
    pub expense: AmountResponse,
}


#[derive(Serialize, Schematic)]
pub struct AccountJournalEntity {
    pub datetime: NaiveDateTime,
    pub timestamp: i64,
    pub account: String,
    pub trx_id: String,
    pub payee: Option<String>,
    pub narration: Option<String>,
    pub inferred_unit_number: BigDecimal,
    pub inferred_unit_commodity: String,
    pub account_after_number: BigDecimal,
    pub account_after_commodity: String,
}

impl From<AccountJournalDomain> for AccountJournalEntity {
    fn from(value: AccountJournalDomain) -> Self {
        AccountJournalEntity {
            datetime: value.datetime,
            timestamp: value.timestamp,
            account: value.account,
            trx_id: value.trx_id,
            payee: value.payee,
            narration: value.narration,
            inferred_unit_number: value.inferred_unit_number,
            inferred_unit_commodity: value.inferred_unit_commodity,
            account_after_number: value.account_after_number,
            account_after_commodity: value.account_after_commodity,
        }
    }
}
#[derive(Serialize, Schematic)]
pub struct ReportResponse {
    pub from: NaiveDateTime,
    pub to: NaiveDateTime,

    pub balance: AmountResponse,
    pub liability: AmountResponse,
    pub income: AmountResponse,
    pub expense: AmountResponse,
    pub transaction_number: i64,

    pub income_rank: Vec<ReportRankItemResponse>,
    pub income_top_transactions: Vec<AccountJournalEntity>,
    pub expense_rank: Vec<ReportRankItemResponse>,
    pub expense_top_transactions: Vec<AccountJournalEntity>,
}

#[derive(Serialize, Schematic)]
pub struct StatisticRankResponse {
    pub from: NaiveDateTime,
    pub to: NaiveDateTime,

    pub detail: Vec<ReportRankItemResponse>,
    pub top_transactions: Vec<AccountJournalEntity>,
}

#[derive(Serialize, Schematic)]
pub struct StatisticGraphResponse {
    pub from: NaiveDateTime,
    pub to: NaiveDateTime,

    pub balances: HashMap<NaiveDate, CalculatedAmountEntity>,
    pub changes: HashMap<NaiveDate, HashMap<AccountType, CalculatedAmountEntity>>,
}

#[derive(Serialize, Schematic)]
pub struct ReportRankItemResponse {
    pub account: String,
    pub amount: CalculatedAmountEntity,
}

#[derive(Serialize, Schematic)]
/// the basic info of the server
pub struct BasicInfo {
    /// title of ledger
    pub title: Option<String>,
    /// version of zhang accounting
    pub version: String,
    /// docker build date of zhang accounting
    pub build_date: String,
}

#[derive(Serialize, Schematic)]
pub struct AccountInfoResponse {
    pub date: NaiveDateTime,
    pub r#type: String,
    pub name: String,
    pub status: AccountStatusEntity,
    pub alias: Option<String>,
    pub amount: CalculatedAmountEntity,
}

#[derive(Serialize, Schematic)]
pub struct BudgetListItemResponse {
    pub name: String,
    pub alias: Option<String>,
    pub category: Option<String>,
    pub closed: bool,
    pub assigned_amount: AmountEntity,
    pub activity_amount: AmountEntity,
    pub available_amount: AmountEntity,
}

#[derive(Serialize, Schematic)]
pub struct BudgetInfoResponse {
    pub name: String,
    pub alias: Option<String>,
    pub category: Option<String>,
    pub closed: bool,

    pub related_accounts: Vec<String>,

    pub assigned_amount: AmountEntity,
    pub activity_amount: AmountEntity,
    pub available_amount: AmountEntity,
}

#[derive(Serialize, Schematic)]
pub struct BudgetEventEntity {
    pub timestamp: i64,
    pub amount: AmountEntity,
    pub event_type: BudgetEventTypeEnum,
}

impl From<BudgetEvent> for BudgetEventEntity {
    fn from(value: BudgetEvent) -> Self {
        BudgetEventEntity {
            timestamp: value.timestamp,
            amount: value.amount.into(),
            event_type: value.event_type.into(),
        }
    }
}
#[derive(Serialize, Schematic)]
pub enum BudgetEventTypeEnum {
    AddAssignedAmount,
    Transfer,
}

impl From<BudgetEventType> for BudgetEventTypeEnum {
    fn from(value: BudgetEventType) -> Self {
        match value {
            BudgetEventType::AddAssignedAmount => BudgetEventTypeEnum::AddAssignedAmount,
            BudgetEventType::Transfer => BudgetEventTypeEnum::Transfer,
        }
    }
}
#[derive(Serialize, Schematic)]
#[serde(tag = "type")]
pub enum BudgetIntervalEventResponse {
    BudgetEvent(BudgetEventEntity),
    Posting(AccountJournalEntity),
}

impl BudgetIntervalEventResponse {
    pub(crate) fn naive_datetime(&self) -> NaiveDateTime {
        match self {
            BudgetIntervalEventResponse::BudgetEvent(budget_event) => {
                DateTime::from_timestamp(budget_event.timestamp, 0)
                    .unwrap_or_else(|| DateTime::from_timestamp_millis(0).unwrap())
                    .naive_local()
            },
            BudgetIntervalEventResponse::Posting(posting) => posting.datetime,
        }
    }
}


#[derive(Serialize, Schematic)]
pub enum PluginTypeEnum {
    Processor,
    Mapper,
    Router,
}
impl From<PluginType> for PluginTypeEnum {
    fn from(value: PluginType) -> Self {
        match value {
            PluginType::Processor => PluginTypeEnum::Processor,
            PluginType::Mapper => PluginTypeEnum::Mapper,
            PluginType::Router => PluginTypeEnum::Router,
        }
    }
}


#[derive(Serialize, Schematic)]
pub struct PluginResponse {
    pub name: String,
    pub version: String,
    pub plugin_type: Vec<PluginTypeEnum>,
}

#[derive(Serialize, Schematic)]
pub struct AccountBalanceItemResponse {
    pub date: NaiveDate,
    pub balance: AmountResponse,
}


#[derive(Serialize, Schematic)]
pub enum ZhangAstErrorKind {
    UnbalancedTransaction,
    TransactionCannotInferTradeAmount,
    TransactionHasMultipleImplicitPosting,
    TransactionExplicitPostingHaveMultipleCommodity,

    AccountBalanceCheckError,
    AccountDoesNotExist,
    AccountClosed,

    CommodityDoesNotDefine,
    NoEnoughCommodityLot,
    CloseNonZeroAccount,

    BudgetDoesNotExist,
    DefineDuplicatedBudget,

    MultipleOperatingCurrencyDetect,

    ParseInvalidMeta,
}
impl From<zhang_ast::error::ErrorKind> for ZhangAstErrorKind {
    fn from(value: zhang_ast::error::ErrorKind) -> Self {
        match value {
            zhang_ast::error::ErrorKind::UnbalancedTransaction => ZhangAstErrorKind::UnbalancedTransaction,
            zhang_ast::error::ErrorKind::TransactionCannotInferTradeAmount => ZhangAstErrorKind::TransactionCannotInferTradeAmount,
            zhang_ast::error::ErrorKind::TransactionHasMultipleImplicitPosting => ZhangAstErrorKind::TransactionHasMultipleImplicitPosting,
            zhang_ast::error::ErrorKind::TransactionExplicitPostingHaveMultipleCommodity => ZhangAstErrorKind::TransactionExplicitPostingHaveMultipleCommodity,
            zhang_ast::error::ErrorKind::AccountBalanceCheckError => ZhangAstErrorKind::AccountBalanceCheckError,
            zhang_ast::error::ErrorKind::AccountDoesNotExist => ZhangAstErrorKind::AccountDoesNotExist,
            zhang_ast::error::ErrorKind::AccountClosed => ZhangAstErrorKind::AccountClosed,
            zhang_ast::error::ErrorKind::CommodityDoesNotDefine => ZhangAstErrorKind::CommodityDoesNotDefine,
            zhang_ast::error::ErrorKind::NoEnoughCommodityLot => ZhangAstErrorKind::NoEnoughCommodityLot,
            zhang_ast::error::ErrorKind::CloseNonZeroAccount => ZhangAstErrorKind::CloseNonZeroAccount,
            zhang_ast::error::ErrorKind::BudgetDoesNotExist => ZhangAstErrorKind::BudgetDoesNotExist,
            zhang_ast::error::ErrorKind::DefineDuplicatedBudget => ZhangAstErrorKind::DefineDuplicatedBudget,
            zhang_ast::error::ErrorKind::MultipleOperatingCurrencyDetect => ZhangAstErrorKind::MultipleOperatingCurrencyDetect,
            zhang_ast::error::ErrorKind::ParseInvalidMeta => ZhangAstErrorKind::ParseInvalidMeta,
        }
    }
}

#[derive(Serialize, Schematic)]
pub struct SpanInfoEntity {
    pub start: usize,
    pub end: usize,
    pub content: String,
    pub filename: Option<String>,
}

impl From<SpanInfo> for SpanInfoEntity {
    fn from(value: SpanInfo) -> Self {
        SpanInfoEntity { start: value.start, end: value.end, content: value.content, filename: value.filename.map(|it| it.to_string_lossy().to_string()) }
    }
}

#[derive(Serialize, Schematic)]
pub struct ErrorEntity {
    pub id: String,
    pub span: Option<SpanInfoEntity>,
    pub error_type: ZhangAstErrorKind,
    pub metas: HashMap<String, String>,
}

impl From<ErrorDomain> for ErrorEntity {
    fn from(value: ErrorDomain) -> Self {
        ErrorEntity {
            id: value.id,
            span: value.span.map(|it| it.into()),
            error_type: value.error_type.into(),
            metas: value.metas,
        }
    }
}

#[derive(Debug, Clone, Serialize, Schematic)]
pub struct OptionEntity {
    pub key: String,
    pub value: String,
}
impl From<OptionDomain> for OptionEntity {
    fn from(value: OptionDomain) -> Self {
        OptionEntity { key: value.key, value: value.value }
    }
}


#[derive(Serialize, Schematic)]
pub struct AccountBalanceHistoryResponse {
    pub balance: HashMap<Currency, Vec<AccountBalanceItemResponse>>,
}
