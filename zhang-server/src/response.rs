use std::collections::{BTreeMap, HashMap};

use axum::response::{IntoResponse, Response};
use axum::Json;
use bigdecimal::BigDecimal;
use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use gotcha::oas::{Referenceable, Responses};
use gotcha::{Responsible, Schematic};
use serde::Serialize;
use uuid::Uuid;
use zhang_ast::amount::{Amount, CalculatedAmount};
use zhang_ast::error::ErrorKind;
use zhang_ast::{AccountType, Currency, SpanInfo};
use zhang_core::domains::schemas::{AccountJournalDomain, AccountStatus, ErrorDomain, MetaDomain, QueryDomain};
use zhang_core::plugin::PluginType;
use zhang_core::store::{BudgetEvent, BudgetEventType, PostingDomain};

use crate::error::ServerError;
use crate::ServerResult;

pub struct Created;

impl Responsible for Created {
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

impl<T: Serialize + Schematic> IntoResponse for ResponseWrapper<T> {
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
        let total_page = total_count / size + u32::from(!total_count.is_multiple_of(size));
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
pub struct AccountEntity {
    pub name: String,
    pub status: AccountStatus,
    pub alias: Option<String>,
    pub amount: CalculatedAmount,
}

#[derive(Serialize, Schematic)]
pub struct DocumentEntity {
    pub datetime: NaiveDateTime,
    pub filename: String,
    pub path: String,
    pub extension: Option<String>,
    pub account: Option<String>,
    pub trx_id: Option<String>,
}

#[derive(Serialize, Schematic)]
pub struct MetaEntity {
    key: String,
    value: String,
}
impl From<MetaDomain> for MetaEntity {
    fn from(value: MetaDomain) -> Self {
        MetaEntity {
            key: value.key,
            value: value.value,
        }
    }
}

#[derive(Serialize, Schematic)]
#[serde(tag = "type")]
pub enum JournalItemEntity {
    Transaction(JournalTransactionItemEntity),
    BalanceCheck(JournalBalanceItemEntity),
    BalancePad(JournalBalanceItemEntity),
}

impl JournalItemEntity {
    pub fn sequence(&self) -> i32 {
        match self {
            JournalItemEntity::Transaction(inner) => inner.sequence,
            JournalItemEntity::BalanceCheck(inner) => inner.sequence,
            JournalItemEntity::BalancePad(inner) => inner.sequence,
        }
    }
}

#[derive(Serialize, Schematic)]
pub struct JournalTransactionItemEntity {
    pub id: Uuid,
    pub sequence: i32,
    pub datetime: NaiveDateTime,
    pub payee: String,
    pub narration: Option<String>,
    pub tags: Vec<String>,
    pub links: Vec<String>,
    pub flag: String,
    pub is_balanced: bool,
    pub postings: Vec<JournalTransactionPostingEntity>,
    pub metas: Vec<MetaEntity>,
}
#[derive(Serialize, Schematic)]
pub struct JournalTransactionPostingEntity {
    pub account: String,
    pub unit: Option<Amount>,
    pub cost: Option<Amount>,
    pub inferred_unit: Amount,
    pub account_before: Amount,
    pub account_after: Amount,
    /// metadata of the posting, sorted by key
    pub metas: Vec<MetaEntity>,
}

impl From<PostingDomain> for JournalTransactionPostingEntity {
    fn from(arm: PostingDomain) -> Self {
        JournalTransactionPostingEntity {
            account: arm.account.name().to_owned(),
            unit: arm.unit,
            cost: arm.cost,
            inferred_unit: arm.inferred_amount,
            account_before: arm.previous_amount,
            account_after: arm.after_amount,
            metas: arm
                .metas
                .into_iter()
                .map(|meta| MetaEntity {
                    key: meta.key,
                    value: meta.value,
                })
                .collect(),
        }
    }
}

#[derive(Serialize, Schematic)]
pub struct JournalBalanceItemEntity {
    pub id: Uuid,
    pub sequence: i32,
    pub datetime: NaiveDateTime,
    pub payee: String,
    pub narration: Option<String>,
    pub type_: String,
    pub(crate) postings: Vec<JournalTransactionPostingEntity>,
}

#[derive(Serialize, Schematic)]
pub struct InfoForNewTransaction {
    pub payee: Vec<String>,
    pub account_name: Vec<String>,
}

#[derive(Serialize, Schematic)]
pub struct CommodityListItemEntity {
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
pub struct CommodityLotEntity {
    pub account: String,
    pub amount: BigDecimal,

    pub cost: Option<Amount>,
    pub price: Option<Amount>,
    pub acquisition_date: Option<NaiveDate>,
}

#[derive(Serialize, Schematic)]
pub struct CommodityPriceEntity {
    pub datetime: NaiveDateTime,
    pub amount: Amount,
}

#[derive(Serialize, Schematic)]
pub struct CommodityDetailEntity {
    pub info: CommodityListItemEntity,
    pub lots: Vec<CommodityLotEntity>,
    pub prices: Vec<CommodityPriceEntity>,
}

#[derive(Serialize, Schematic)]
pub struct FileDetailEntity {
    pub path: String,
    pub content: String,
}

#[derive(Serialize, Schematic)]
pub struct StatisticSummaryEntity {
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,

    pub balance: CalculatedAmount,
    pub liability: CalculatedAmount,

    pub income: CalculatedAmount,
    pub expense: CalculatedAmount,
    pub transaction_number: i64,
}

#[derive(Serialize, Schematic)]
pub struct StatisticRankEntity {
    pub from: NaiveDateTime,
    pub to: NaiveDateTime,

    pub detail: Vec<ReportRankItemEntity>,
    pub top_transactions: Vec<AccountJournalDomain>,
}

#[derive(Serialize, Schematic)]
pub struct StatisticGraphEntity {
    pub from: NaiveDateTime,
    pub to: NaiveDateTime,

    pub balances: HashMap<NaiveDate, CalculatedAmount>,
    pub changes: HashMap<NaiveDate, HashMap<AccountType, CalculatedAmount>>,
}

#[derive(Serialize, Schematic)]
pub struct ReportRankItemEntity {
    pub account: String,
    pub amount: CalculatedAmount,
}

#[derive(Serialize, Schematic)]
/// the basic info of the server
pub struct BasicInfoEntity {
    /// title of ledger
    pub title: Option<String>,
    /// version of zhang accounting
    pub version: String,
    /// docker build date of zhang accounting
    pub build_date: String,
}

#[derive(Serialize, Schematic)]
pub struct AccountInfoEntity {
    pub date: NaiveDateTime,
    pub r#type: String,
    pub name: String,
    pub status: AccountStatus,
    pub alias: Option<String>,
    pub amount: CalculatedAmount,
}

#[derive(Serialize, Schematic)]
pub struct BudgetListItemEntity {
    pub name: String,
    pub alias: Option<String>,
    pub category: Option<String>,
    pub closed: bool,
    pub assigned_amount: Amount,
    pub activity_amount: Amount,
    pub available_amount: Amount,
}

#[derive(Serialize, Schematic)]
pub struct BudgetInfoEntity {
    pub name: String,
    pub alias: Option<String>,
    pub category: Option<String>,
    pub closed: bool,

    pub related_accounts: Vec<String>,

    pub assigned_amount: Amount,
    pub activity_amount: Amount,
    pub available_amount: Amount,
}

#[derive(Serialize, Schematic)]
pub struct BudgetEventEntity {
    pub timestamp: i64,
    pub amount: Amount,
    pub event_type: BudgetEventType,
}

impl From<BudgetEvent> for BudgetEventEntity {
    fn from(value: BudgetEvent) -> Self {
        BudgetEventEntity {
            timestamp: value.timestamp,
            amount: value.amount,
            event_type: value.event_type,
        }
    }
}
#[derive(Serialize, Schematic)]
#[serde(tag = "type")]
pub enum BudgetIntervalEventEntity {
    BudgetEvent(BudgetEventEntity),
    Posting(AccountJournalDomain),
}

impl BudgetIntervalEventEntity {
    pub(crate) fn naive_datetime(&self) -> NaiveDateTime {
        match self {
            BudgetIntervalEventEntity::BudgetEvent(budget_event) => DateTime::from_timestamp(budget_event.timestamp, 0)
                .unwrap_or_else(|| DateTime::from_timestamp_millis(0).unwrap())
                .naive_local(),
            BudgetIntervalEventEntity::Posting(posting) => posting.datetime,
        }
    }
}

/// a loaded plugin
#[derive(Serialize, Schematic)]
pub struct PluginEntity {
    pub name: String,
    pub version: String,
    /// the types the plugin runs as; a declared type this zhang does not know is left out
    pub plugin_type: Vec<PluginTypeEntity>,
    pub capabilities: PluginCapabilitiesEntity,
    /// where a router plugin serves requests, `/api/plugins/{name}`; null for a plugin that is not a
    /// router, and for a router whose name an earlier router plugin already serves
    pub route: Nullable<String>,
}

/// a plugin type this zhang runs
#[derive(Serialize, Schematic)]
pub enum PluginTypeEntity {
    Processor,
    Mapper,
    Router,
}

impl PluginTypeEntity {
    /// `None` for a type this zhang does not run
    pub fn from_core(plugin_type: &PluginType) -> Option<PluginTypeEntity> {
        match plugin_type {
            PluginType::Processor => Some(PluginTypeEntity::Processor),
            PluginType::Mapper => Some(PluginTypeEntity::Mapper),
            PluginType::Router => Some(PluginTypeEntity::Router),
            PluginType::Unknown => None,
        }
    }
}

/// what a plugin's directive grants it
#[derive(Serialize, Schematic)]
pub struct PluginCapabilitiesEntity {
    /// hosts the plugin may reach over HTTP; empty means no network access
    pub allowed_hosts: Vec<String>,
}

#[derive(Serialize, Schematic)]
pub struct AccountBalanceItemEntity {
    pub date: NaiveDate,
    pub balance: Amount,
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
        SpanInfoEntity {
            start: value.start,
            end: value.end,
            content: value.content,
            filename: value.filename.map(|it| it.to_string_lossy().to_string()),
        }
    }
}

#[derive(Serialize, Schematic)]
pub struct ErrorEntity {
    pub id: String,
    pub span: Option<SpanInfoEntity>,
    pub error_type: ErrorKind,
    pub metas: HashMap<String, String>,
}

impl From<ErrorDomain> for ErrorEntity {
    fn from(value: ErrorDomain) -> Self {
        ErrorEntity {
            id: value.id,
            span: value.span.map(|it| it.into()),
            error_type: value.error_type,
            metas: value.metas,
        }
    }
}

#[derive(Serialize, Schematic)]
pub struct AccountBalanceHistoryEntity {
    pub balance: HashMap<Currency, Vec<AccountBalanceItemEntity>>,
}

/// A value that is always present in the JSON but may be `null`.
///
/// `Option<T>` makes gotcha mark a field optional (and a `Vec<Option<T>>` field too), while
/// the query API always sends these keys; this keeps them required and nullable in OpenAPI.
#[derive(Serialize, Debug)]
#[serde(transparent)]
pub struct Nullable<T>(pub Option<T>);

impl<T: Schematic> Schematic for Nullable<T> {
    fn name() -> &'static str {
        T::name()
    }

    fn required() -> bool {
        true
    }

    fn nullable() -> Option<bool> {
        Some(true)
    }

    fn type_() -> &'static str {
        T::type_()
    }

    fn doc() -> Option<String> {
        T::doc()
    }

    fn generate_schema() -> gotcha::EnhancedSchema {
        let mut schema = T::generate_schema();
        schema.schema.nullable = Some(true);
        schema.required = true;
        schema
    }
}

/// The body of a failed query (HTTP 400): positions are 1-based and count characters.
#[derive(Serialize, Schematic)]
pub struct QueryErrorEntity {
    pub message: String,
    pub line: Nullable<usize>,
    pub column: Nullable<usize>,
}

impl From<zhang_query::QueryError> for QueryErrorEntity {
    fn from(value: zhang_query::QueryError) -> Self {
        QueryErrorEntity {
            message: value.message,
            line: Nullable(value.line),
            column: Nullable(value.column),
        }
    }
}

/// The result of a query endpoint: documents the HTTP 400 [`QueryErrorEntity`] next to the
/// `200` body in OpenAPI.
pub struct QueryApiResult<T: Serialize + Schematic>(pub ServerResult<ResponseWrapper<T>>);

impl<T: Serialize + Schematic> IntoResponse for QueryApiResult<T> {
    fn into_response(self) -> Response {
        match self.0 {
            Ok(data) => data.into_response(),
            Err(error) => error.into_response(),
        }
    }
}

impl<T: Serialize + Schematic> Responsible for QueryApiResult<T> {
    fn response() -> Responses {
        let mut responses = <ResponseWrapper<T> as Responsible>::response();
        responses.data.insert("400".to_string(), query_error_response());
        responses
    }
}

/// The OpenAPI description of the HTTP 400 [`QueryErrorEntity`] of the query endpoints.
fn query_error_response() -> Referenceable<gotcha::oas::Response> {
    Referenceable::Data(gotcha::oas::Response {
        description: "the query cannot be parsed, compiled or run".to_string(),
        headers: None,
        content: Some(BTreeMap::from([(
            "application/json".to_string(),
            gotcha::oas::MediaType {
                schema: Some(Referenceable::Data(QueryErrorEntity::generate_schema().schema)),
                example: None,
                examples: None,
                encoding: None,
            },
        )])),
        links: None,
    })
}

/// The media type of `POST /api/query/csv`.
pub const QUERY_CSV_CONTENT_TYPE: &str = "text/csv; charset=utf-8";
/// The `Content-Disposition` of `POST /api/query/csv`: download as `query.csv`.
pub const QUERY_CSV_CONTENT_DISPOSITION: &str = "attachment; filename=\"query.csv\"";

/// The result of `POST /api/query/csv`: the numberified CSV text, or the HTTP 400
/// [`QueryErrorEntity`] of a failed query.
pub struct QueryCsvResult(pub ServerResult<String>);

impl IntoResponse for QueryCsvResult {
    fn into_response(self) -> Response {
        match self.0 {
            Ok(csv) => (
                [
                    (axum::http::header::CONTENT_TYPE, QUERY_CSV_CONTENT_TYPE),
                    (axum::http::header::CONTENT_DISPOSITION, QUERY_CSV_CONTENT_DISPOSITION),
                ],
                csv,
            )
                .into_response(),
            Err(error) => error.into_response(),
        }
    }
}

impl Responsible for QueryCsvResult {
    fn response() -> Responses {
        let header = |description: &str, example: &str| {
            Referenceable::Data(gotcha::oas::Header {
                description: Some(description.to_string()),
                required: Some(true),
                deprecated: None,
                allow_empty_value: None,
                style: None,
                explode: None,
                allow_reserved: None,
                schema: Some(Referenceable::Data(String::generate_schema().schema)),
                example: Some(serde_json::Value::String(example.to_string())),
                examples: None,
                content: None,
            })
        };
        let ok = gotcha::oas::Response {
            description: "the result as RFC 4180 CSV with a header row; amount, position and inventory columns are split into one \
                          numeric column per currency, named like `balance (USD)`"
                .to_string(),
            headers: Some(BTreeMap::from([(
                "Content-Disposition".to_string(),
                header("download as query.csv", QUERY_CSV_CONTENT_DISPOSITION),
            )])),
            content: Some(BTreeMap::from([(
                QUERY_CSV_CONTENT_TYPE.to_string(),
                gotcha::oas::MediaType {
                    schema: Some(Referenceable::Data(String::generate_schema().schema)),
                    example: None,
                    examples: None,
                    encoding: None,
                },
            )])),
            links: None,
        };
        Responses {
            default: None,
            data: BTreeMap::from([("200".to_string(), Referenceable::Data(ok)), ("400".to_string(), query_error_response())]),
        }
    }
}

/// The static type of a query result column.
#[derive(Serialize, Schematic, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum QueryColumnType {
    Null,
    Bool,
    Int,
    Decimal,
    Str,
    Date,
    Set,
    Amount,
    Position,
    Inventory,
    Interval,
    Metas,
}

impl From<zhang_query::DataType> for QueryColumnType {
    fn from(value: zhang_query::DataType) -> Self {
        use zhang_query::DataType;
        match value {
            DataType::Null => QueryColumnType::Null,
            DataType::Bool => QueryColumnType::Bool,
            DataType::Int => QueryColumnType::Int,
            DataType::Decimal => QueryColumnType::Decimal,
            DataType::Str => QueryColumnType::Str,
            DataType::Date => QueryColumnType::Date,
            DataType::Set => QueryColumnType::Set,
            DataType::Amount => QueryColumnType::Amount,
            DataType::Position => QueryColumnType::Position,
            DataType::Inventory => QueryColumnType::Inventory,
            DataType::Interval => QueryColumnType::Interval,
            DataType::Metas => QueryColumnType::Metas,
        }
    }
}

#[derive(Serialize, Schematic)]
pub struct QueryColumnEntity {
    pub name: String,
    #[serde(rename = "type")]
    pub column_type: QueryColumnType,
}

/// An amount with an exact decimal number, e.g. `{"number": "-12.50", "currency": "USD"}`.
#[derive(Serialize, Schematic)]
pub struct QueryAmountEntity {
    pub number: String,
    pub currency: String,
}

/// The cost lot of a position: per-unit cost, acquisition date and label.
#[derive(Serialize, Schematic)]
pub struct QueryCostEntity {
    pub number: String,
    pub currency: String,
    pub date: Nullable<String>,
    pub label: Nullable<String>,
}

#[derive(Serialize, Schematic)]
pub struct QueryPositionEntity {
    pub units: QueryAmountEntity,
    pub cost: Nullable<QueryCostEntity>,
}

/// Positions sorted by units currency, then cost.
#[derive(Serialize, Schematic)]
pub struct QueryInventoryEntity {
    pub positions: Vec<QueryPositionEntity>,
}

/// One metadata entry of a `metas` cell.
#[derive(Serialize, Schematic)]
pub struct QueryMetaEntity {
    pub key: String,
    pub value: String,
}

/// One result cell. Booleans and integers are JSON booleans and numbers; decimals, strings,
/// dates (`YYYY-MM-DD`) and intervals (`1 year 2 months`) are strings; sets are sorted string
/// arrays; metadata (`metas`) is an array of `{"key", "value"}` objects in order.
///
/// Integers are 64-bit and sent as JSON numbers; JavaScript parses numbers as doubles, so
/// an integer outside ±2^53 (unusual for counts and date parts) loses precision there.
#[derive(Serialize, Schematic)]
#[serde(untagged)]
pub enum QueryCell {
    Bool(bool),
    Int(i64),
    Text(String),
    Set(Vec<String>),
    Amount(QueryAmountEntity),
    Position(QueryPositionEntity),
    Inventory(QueryInventoryEntity),
    Metas(Vec<QueryMetaEntity>),
}

impl From<&zhang_query::Amount> for QueryAmountEntity {
    fn from(value: &zhang_query::Amount) -> Self {
        QueryAmountEntity {
            number: zhang_query::decimal::to_plain_string(&value.number),
            currency: value.commodity.clone(),
        }
    }
}

impl From<&zhang_query::Position> for QueryPositionEntity {
    fn from(value: &zhang_query::Position) -> Self {
        QueryPositionEntity {
            units: (&value.units).into(),
            cost: Nullable(value.cost.as_ref().map(|cost| QueryCostEntity {
                number: zhang_query::decimal::to_plain_string(&cost.number),
                currency: cost.currency.clone(),
                date: Nullable(cost.date.map(|date| date.format("%Y-%m-%d").to_string())),
                label: Nullable(cost.label.clone()),
            })),
        }
    }
}

impl QueryCell {
    pub fn encode(value: &zhang_query::Value) -> Nullable<QueryCell> {
        use zhang_query::Value;
        Nullable(Some(match value {
            Value::Null => return Nullable(None),
            Value::Bool(it) => QueryCell::Bool(*it),
            Value::Int(it) => QueryCell::Int(*it),
            Value::Decimal(it) => QueryCell::Text(zhang_query::decimal::to_plain_string(it)),
            Value::Str(it) => QueryCell::Text(it.clone()),
            Value::Date(it) => QueryCell::Text(it.format("%Y-%m-%d").to_string()),
            Value::Set(it) => QueryCell::Set(it.iter().cloned().collect()),
            Value::Amount(it) => QueryCell::Amount(it.into()),
            Value::Position(it) => QueryCell::Position(it.into()),
            Value::Inventory(it) => QueryCell::Inventory(QueryInventoryEntity {
                positions: it.positions().map(|position| (&position).into()).collect(),
            }),
            Value::Interval(it) => QueryCell::Text(it.to_string()),
            Value::Metas(it) => QueryCell::Metas(
                it.iter()
                    .map(|(key, value)| QueryMetaEntity {
                        key: key.clone(),
                        value: value.clone(),
                    })
                    .collect(),
            ),
        }))
    }
}

/// The result of `POST /api/query`; any cell may be `null`.
#[derive(Serialize, Schematic)]
pub struct QueryResultEntity {
    pub columns: Vec<QueryColumnEntity>,
    pub rows: Vec<Vec<Nullable<QueryCell>>>,
}

impl From<zhang_query::QueryResult> for QueryResultEntity {
    fn from(value: zhang_query::QueryResult) -> Self {
        QueryResultEntity {
            columns: value
                .columns
                .into_iter()
                .map(|column| QueryColumnEntity {
                    name: column.name,
                    column_type: column.ty.into(),
                })
                .collect(),
            rows: value.rows.iter().map(|row| row.iter().map(QueryCell::encode).collect()).collect(),
        }
    }
}

#[derive(Serialize, Schematic)]
pub struct QuerySchemaColumnEntity {
    pub name: String,
    #[serde(rename = "type")]
    pub column_type: QueryColumnType,
    pub description: String,
}

#[derive(Serialize, Schematic)]
pub struct QuerySchemaFunctionEntity {
    pub name: String,
    /// e.g. `root(str, int) -> str`
    pub signature: String,
    pub description: String,
    /// whether this is an aggregate function (`sum`, `count`, ...)
    pub aggregate: bool,
}

/// A table a query can read with `FROM #name` (`GET /api/query/schema`).
#[derive(Serialize, Schematic)]
pub struct QuerySchemaTableEntity {
    /// the table name, without `#`
    pub name: String,
    pub description: String,
    pub columns: Vec<QuerySchemaColumnEntity>,
}

/// The queryable tables, columns and functions of `POST /api/query`.
#[derive(Serialize, Schematic)]
pub struct QuerySchemaEntity {
    /// the columns of the `postings` table, the default table
    pub columns: Vec<QuerySchemaColumnEntity>,
    pub functions: Vec<QuerySchemaFunctionEntity>,
    /// every table, `postings` first
    pub tables: Vec<QuerySchemaTableEntity>,
}

impl From<zhang_query::ColumnDoc> for QuerySchemaColumnEntity {
    fn from(column: zhang_query::ColumnDoc) -> Self {
        QuerySchemaColumnEntity {
            name: column.name.to_owned(),
            column_type: column.ty.into(),
            description: column.description.to_owned(),
        }
    }
}

impl From<zhang_query::Schema> for QuerySchemaEntity {
    fn from(value: zhang_query::Schema) -> Self {
        QuerySchemaEntity {
            columns: value.columns.into_iter().map(QuerySchemaColumnEntity::from).collect(),
            tables: value
                .tables
                .into_iter()
                .map(|table| QuerySchemaTableEntity {
                    name: table.name.to_owned(),
                    description: table.description.to_owned(),
                    columns: table.columns.into_iter().map(QuerySchemaColumnEntity::from).collect(),
                })
                .collect(),
            functions: value
                .functions
                .into_iter()
                .map(|function| QuerySchemaFunctionEntity {
                    name: function.name.to_owned(),
                    signature: function.signature,
                    description: function.description.to_owned(),
                    aggregate: function.aggregate,
                })
                .collect(),
        }
    }
}

/// A named query saved in the ledger by a `query` directive (`GET /api/query/saved`).
#[derive(Serialize, Schematic)]
pub struct SavedQueryEntity {
    pub name: String,
    /// the query text, verbatim
    pub query: String,
    /// the directive's date, `YYYY-MM-DD`
    pub date: NaiveDate,
    /// whether the query compiles with the current engine; saved queries are never
    /// validated at load time, so one written for a future feature is simply `false`
    pub valid: bool,
    /// why the query does not compile (with its position when known), `null` when valid
    pub error: Nullable<String>,
}

impl From<QueryDomain> for SavedQueryEntity {
    fn from(value: QueryDomain) -> Self {
        let error = zhang_query::Query::compile(&value.query).err().map(|error| error.to_string());
        SavedQueryEntity {
            name: value.name,
            query: value.query,
            date: value.date,
            valid: error.is_none(),
            error: Nullable(error),
        }
    }
}

#[cfg(test)]
mod query_test {
    use std::str::FromStr;

    use bigdecimal::BigDecimal;
    use chrono::NaiveDate;
    use serde_json::json;
    use zhang_query::{Amount, ColumnInfo, Cost, DataType, Interval, Inventory, Position, QueryResult, Value};

    use crate::response::QueryResultEntity;

    fn amount(number: &str, currency: &str) -> Amount {
        Amount::new(BigDecimal::from_str(number).unwrap(), currency)
    }

    #[test]
    fn query_results_follow_the_api_cell_encoding() {
        let date = NaiveDate::from_ymd_opt(2024, 1, 31).unwrap();
        let lot = Position::new(
            amount("10", "AAPL"),
            Some(Cost {
                number: BigDecimal::from_str("150.00").unwrap(),
                currency: "USD".to_owned(),
                date: Some(date),
                label: None,
            }),
        );
        let mut inventory = Inventory::new();
        inventory.add_position(&lot);
        inventory.add_amount(&amount("-12.50", "EUR"));
        let columns = [
            ("b", DataType::Bool),
            ("i", DataType::Int),
            ("d", DataType::Decimal),
            ("s", DataType::Str),
            ("date", DataType::Date),
            ("tags", DataType::Set),
            ("a", DataType::Amount),
            ("p", DataType::Position),
            ("inv", DataType::Inventory),
            ("n", DataType::Null),
            ("every", DataType::Interval),
            ("metas", DataType::Metas),
        ];
        let result = QueryResult {
            columns: columns
                .iter()
                .map(|(name, ty)| ColumnInfo {
                    name: (*name).to_owned(),
                    ty: *ty,
                })
                .collect(),
            rows: vec![vec![
                Value::Bool(true),
                Value::Int(42),
                Value::Decimal(BigDecimal::from_str("-12.50").unwrap()),
                Value::from("午餐"),
                Value::Date(date),
                Value::Set(["b".to_owned(), "a".to_owned()].into_iter().collect()),
                Value::Amount(amount("1E+3", "USD")),
                Value::Position(lot),
                Value::Inventory(inventory),
                Value::Null,
                Value::Interval(Interval::new(14, -3)),
                Value::Metas(vec![("invoice".to_owned(), "a.pdf".to_owned()), ("invoice".to_owned(), "b.pdf".to_owned())]),
            ]],
            total: None,
        };
        let encoded = serde_json::to_value(QueryResultEntity::from(result)).unwrap();
        assert_eq!(
            encoded,
            json!({
                "columns": [
                    {"name": "b", "type": "bool"}, {"name": "i", "type": "int"}, {"name": "d", "type": "decimal"},
                    {"name": "s", "type": "str"}, {"name": "date", "type": "date"}, {"name": "tags", "type": "set"},
                    {"name": "a", "type": "amount"}, {"name": "p", "type": "position"}, {"name": "inv", "type": "inventory"},
                    {"name": "n", "type": "null"}, {"name": "every", "type": "interval"}, {"name": "metas", "type": "metas"}
                ],
                "rows": [[
                    true, 42, "-12.50", "午餐", "2024-01-31", ["a", "b"],
                    {"number": "1000", "currency": "USD"},
                    {"units": {"number": "10", "currency": "AAPL"}, "cost": {"number": "150.00", "currency": "USD", "date": "2024-01-31", "label": null}},
                    {"positions": [
                        {"units": {"number": "10", "currency": "AAPL"}, "cost": {"number": "150.00", "currency": "USD", "date": "2024-01-31", "label": null}},
                        {"units": {"number": "-12.50", "currency": "EUR"}, "cost": null}
                    ]},
                    null,
                    "1 year 2 months -3 days",
                    [{"key": "invoice", "value": "a.pdf"}, {"key": "invoice", "value": "b.pdf"}]
                ]]
            })
        );
    }
}

#[derive(Serialize, Schematic)]
pub struct AuthMethodsEntity {
    /// username and password, `ZHANG_AUTH`
    pub password: bool,
    /// passkeys, `ZHANG_PASSKEY`
    pub passkey: bool,
}

#[derive(Serialize, Schematic)]
pub struct AuthStatusEntity {
    /// whether any authentication method is enabled
    pub enabled: bool,
    /// whether the caller can use the API (always true when authentication is disabled)
    pub authenticated: bool,
    pub methods: AuthMethodsEntity,
    /// whether at least one passkey is registered
    pub passkey_registered: bool,
    /// who the caller is signed in as
    pub user: Option<String>,
    /// title of ledger
    pub title: Option<String>,
}

#[derive(Serialize, Schematic)]
pub struct PasskeyChallengeEntity {
    /// identifies the ceremony when finishing it
    pub state_id: String,
    /// the options to pass to `navigator.credentials.create` / `navigator.credentials.get`
    pub options: serde_json::Value,
}

#[derive(Serialize, Schematic)]
pub struct PasskeyEntity {
    pub id: String,
    pub name: String,
    pub created_at: DateTime<Utc>,
}
