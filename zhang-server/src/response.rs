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
use zhang_core::domains::schemas::{AccountJournalDomain, AccountStatus, ErrorDomain, MetaDomain};
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

#[derive(Serialize, Schematic)]
pub struct PluginEntity {
    pub name: String,
    pub version: String,
    pub plugin_type: Vec<PluginType>,
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
    pub date: Option<String>,
    pub label: Option<String>,
}

#[derive(Serialize, Schematic)]
pub struct QueryPositionEntity {
    pub units: QueryAmountEntity,
    pub cost: Option<QueryCostEntity>,
}

/// Positions sorted by units currency, then cost.
#[derive(Serialize, Schematic)]
pub struct QueryInventoryEntity {
    pub positions: Vec<QueryPositionEntity>,
}

/// One result cell. Booleans and integers are JSON booleans and numbers; decimals, strings
/// and dates (`YYYY-MM-DD`) are strings; sets are sorted string arrays.
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
            cost: value.cost.as_ref().map(|cost| QueryCostEntity {
                number: zhang_query::decimal::to_plain_string(&cost.number),
                currency: cost.currency.clone(),
                date: cost.date.map(|date| date.format("%Y-%m-%d").to_string()),
                label: cost.label.clone(),
            }),
        }
    }
}

impl QueryCell {
    pub fn encode(value: &zhang_query::Value) -> Option<QueryCell> {
        use zhang_query::Value;
        Some(match value {
            Value::Null => return None,
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
        })
    }
}

/// The result of `POST /api/query`; any cell may be `null`.
#[derive(Serialize, Schematic)]
pub struct QueryResultEntity {
    pub columns: Vec<QueryColumnEntity>,
    pub rows: Vec<Vec<Option<QueryCell>>>,
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
}

/// The queryable columns and functions of `POST /api/query`.
#[derive(Serialize, Schematic)]
pub struct QuerySchemaEntity {
    pub columns: Vec<QuerySchemaColumnEntity>,
    pub functions: Vec<QuerySchemaFunctionEntity>,
}

impl From<zhang_query::Schema> for QuerySchemaEntity {
    fn from(value: zhang_query::Schema) -> Self {
        QuerySchemaEntity {
            columns: value
                .columns
                .into_iter()
                .map(|column| QuerySchemaColumnEntity {
                    name: column.name.to_owned(),
                    column_type: column.ty.into(),
                    description: column.description.to_owned(),
                })
                .collect(),
            functions: value
                .functions
                .into_iter()
                .map(|function| QuerySchemaFunctionEntity {
                    name: function.name.to_owned(),
                    signature: function.signature,
                    description: function.description.to_owned(),
                })
                .collect(),
        }
    }
}

#[cfg(test)]
mod query_test {
    use std::str::FromStr;

    use bigdecimal::BigDecimal;
    use chrono::NaiveDate;
    use serde_json::json;
    use zhang_query::{Amount, ColumnInfo, Cost, DataType, Inventory, Position, QueryResult, Value};

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
            ]],
        };
        let encoded = serde_json::to_value(QueryResultEntity::from(result)).unwrap();
        assert_eq!(
            encoded,
            json!({
                "columns": [
                    {"name": "b", "type": "bool"}, {"name": "i", "type": "int"}, {"name": "d", "type": "decimal"},
                    {"name": "s", "type": "str"}, {"name": "date", "type": "date"}, {"name": "tags", "type": "set"},
                    {"name": "a", "type": "amount"}, {"name": "p", "type": "position"}, {"name": "inv", "type": "inventory"},
                    {"name": "n", "type": "null"}
                ],
                "rows": [[
                    true, 42, "-12.50", "午餐", "2024-01-31", ["a", "b"],
                    {"number": "1000", "currency": "USD"},
                    {"units": {"number": "10", "currency": "AAPL"}, "cost": {"number": "150.00", "currency": "USD", "date": "2024-01-31", "label": null}},
                    {"positions": [
                        {"units": {"number": "10", "currency": "AAPL"}, "cost": {"number": "150.00", "currency": "USD", "date": "2024-01-31", "label": null}},
                        {"units": {"number": "-12.50", "currency": "EUR"}, "cost": null}
                    ]},
                    null
                ]]
            })
        );
    }
}
