use duckdb::{Connection, ToSql};
use itertools::Itertools;
use uuid::Uuid;
use zhang_core::{
    constants::*,
    domains::schemas::{AccountDomain, CommodityDomain, MetaDomain, PriceDomain},
    options::InMemoryOptions,
    store::{CommodityLotRecord, DocumentDomain, DocumentType, PostingDomain, TransactionDomain},
};

#[derive(Debug)]
pub struct ColumnDefinition {
    name: String,
    data_type: ColumnType,
    nullable: bool,
}

#[derive(Debug)]
pub enum ColumnType {
    Uuid,
    String,
    Int,
    Decimal,
    Date,
}

impl ColumnDefinition {
    fn new(name: &'static str, data_type: ColumnType) -> Self {
        Self {
            name: name.to_string(),
            data_type,
            nullable: false,
        }
    }
    fn into_nullable(self) -> Self {
        Self {
            name: self.name.clone(),
            data_type: self.data_type,
            nullable: true,
        }
    }
    fn as_sql_type(&self) -> String {
        let ret = match self.data_type {
            ColumnType::Uuid => "UUID".to_string(),
            ColumnType::String => "TEXT".to_string(),
            ColumnType::Int => "INTEGER".to_string(),
            ColumnType::Date => "TIMESTAMPTZ".to_string(),
            ColumnType::Decimal => "DECIMAL".to_string(),
        };
        if self.nullable {
            format!("{} NULL", ret)
        } else {
            ret
        }
    }
}
#[derive(Debug)]
pub struct TableDefinition {
    name: &'static str,
    columns: Vec<ColumnDefinition>,
}

impl TableDefinition {
    pub fn as_sql(&self) -> String {
        format!(
            "CREATE TABLE {} ({})",
            self.name,
            self.columns.iter().map(|c| format!("{} {}", c.name, c.as_sql_type())).join(", ")
        )
    }
    pub fn as_insert_sql(&self) -> String {
        format!(
            "INSERT INTO {} ({}) VALUES ({})",
            self.name,
            self.columns.iter().map(|c| c.name.clone()).join(", "),
            self.columns.iter().map(|_| "?".to_string()).join(", ")
        )
    }
}

pub trait AsTableDefinition {
    fn as_table_definition() -> TableDefinition;
    fn insert_data(&self, conn: &Connection);
}

impl AsTableDefinition for InMemoryOptions {
    fn as_table_definition() -> TableDefinition {
        TableDefinition {
            name: "options",
            columns: vec![
                ColumnDefinition::new("key", ColumnType::String),
                ColumnDefinition::new("value", ColumnType::String),
            ],
        }
    }
    fn insert_data(&self, conn: &Connection) {
        let table_definition = Self::as_table_definition();
        let mut stmt = conn.prepare(table_definition.as_insert_sql().as_str()).unwrap();
        stmt.execute([
            serde_json::to_string(KEY_OPERATING_CURRENCY).unwrap(),
            serde_json::to_string(&self.operating_currency).unwrap(),
        ])
        .unwrap();
        stmt.execute([
            serde_json::to_string(KEY_DEFAULT_ROUNDING).unwrap(),
            serde_json::to_string(&self.default_rounding).unwrap(),
        ])
        .unwrap();
        stmt.execute([
            serde_json::to_string(KEY_DEFAULT_BALANCE_TOLERANCE_PRECISION).unwrap(),
            serde_json::to_string(&self.default_balance_tolerance_precision).unwrap(),
        ])
        .unwrap();
        stmt.execute([
            serde_json::to_string(KEY_DEFAULT_BOOKING_METHOD).unwrap(),
            serde_json::to_string(&self.default_booking_method).unwrap(),
        ])
        .unwrap();
        stmt.execute([
            serde_json::to_string(KEY_TIMEZONE).unwrap(),
            serde_json::to_string(&self.timezone.to_string()).unwrap(),
        ])
        .unwrap();
        stmt.execute([
            serde_json::to_string(KEY_DIRECTIVE_OUTPUT_PATH).unwrap(),
            serde_json::to_string(&self.directive_output_path).unwrap(),
        ])
        .unwrap();
    }
}

impl AsTableDefinition for AccountDomain {
    fn as_table_definition() -> TableDefinition {
        TableDefinition {
            name: "accounts",
            columns: vec![
                ColumnDefinition::new("date", ColumnType::Date),
                ColumnDefinition::new("type", ColumnType::String),
                ColumnDefinition::new("name", ColumnType::String),
                ColumnDefinition::new("status", ColumnType::String),
                ColumnDefinition::new("alias", ColumnType::String).into_nullable(),
            ],
        }
    }
    fn insert_data(&self, conn: &Connection) {
        let table_definition = Self::as_table_definition();
        let mut stmt = conn.prepare(table_definition.as_insert_sql().as_str()).unwrap();
        let params: &[&dyn ToSql] = &[&self.date, &self.r#type, &self.name, &self.status.as_ref(), &self.alias.clone()];
        stmt.execute(params).unwrap();
    }
}

impl AsTableDefinition for TransactionDomain {
    fn as_table_definition() -> TableDefinition {
        TableDefinition {
            name: "transactions",
            columns: vec![
                ColumnDefinition::new("id", ColumnType::Uuid),
                ColumnDefinition::new("sequence", ColumnType::Int),
                ColumnDefinition::new("date", ColumnType::Date),
                ColumnDefinition::new("flag", ColumnType::String),
                ColumnDefinition::new("payee", ColumnType::String).into_nullable(),
                ColumnDefinition::new("narration", ColumnType::String).into_nullable(),
            ],
        }
    }
    fn insert_data(&self, conn: &Connection) {
        let table_definition = Self::as_table_definition();
        let mut stmt = conn.prepare(table_definition.as_insert_sql().as_str()).unwrap();
        let params: &[&dyn ToSql] = &[
            &self.id,
            &self.sequence,
            &self.datetime,
            &self.flag.to_string(),
            &self.payee.clone(),
            &self.narration.clone(),
        ];
        stmt.execute(params).unwrap();
    }
}

impl AsTableDefinition for PostingDomain {
    fn as_table_definition() -> TableDefinition {
        TableDefinition {
            name: "postings",
            columns: vec![
                ColumnDefinition::new("id", ColumnType::Uuid),
                ColumnDefinition::new("trx_id", ColumnType::Uuid),
                ColumnDefinition::new("trx_sequence", ColumnType::Int),
                ColumnDefinition::new("trx_datetime", ColumnType::Date),
                ColumnDefinition::new("account", ColumnType::String),
                ColumnDefinition::new("unit", ColumnType::Decimal).into_nullable(),
                ColumnDefinition::new("unit_commodity", ColumnType::String).into_nullable(),
                ColumnDefinition::new("cost", ColumnType::Decimal).into_nullable(),
                ColumnDefinition::new("cost_commodity", ColumnType::String).into_nullable(),
                ColumnDefinition::new("inferred_amount", ColumnType::Decimal),
                ColumnDefinition::new("inferred_amount_commodity", ColumnType::String),
                ColumnDefinition::new("previous_amount", ColumnType::Decimal),
                ColumnDefinition::new("previous_amount_commodity", ColumnType::String),
                ColumnDefinition::new("after_amount", ColumnType::Decimal),
                ColumnDefinition::new("after_amount_commodity", ColumnType::String),
            ],
        }
    }
    fn insert_data(&self, conn: &Connection) {
        let table_definition = Self::as_table_definition();
        let mut stmt = conn.prepare(table_definition.as_insert_sql().as_str()).unwrap();
        let params: &[&dyn ToSql] = &[
            &self.id,
            &self.trx_id,
            &self.trx_sequence,
            &self.trx_datetime,
            &self.account.name(),
            &self.unit.as_ref().map(|u| u.number.to_string()),
            &self.unit.as_ref().map(|u| u.currency.clone()),
            &self.cost.as_ref().map(|c| c.number.to_string()),
            &self.cost.as_ref().map(|c| c.currency.clone()),
            &self.inferred_amount.number.to_string(),
            &self.inferred_amount.currency,
            &self.previous_amount.number.to_string(),
            &self.previous_amount.currency,
            &self.after_amount.number.to_string(),
            &self.after_amount.currency,
        ];
        stmt.execute(params).unwrap();
    }
}

pub struct TrxTag {
    pub trx_id: Uuid,
    pub tag: String,
}

impl AsTableDefinition for TrxTag {
    fn as_table_definition() -> TableDefinition {
        TableDefinition {
            name: "transaction_tags",
            columns: vec![
                ColumnDefinition::new("trx_id", ColumnType::Uuid),
                ColumnDefinition::new("tag", ColumnType::String),
            ],
        }
    }
    fn insert_data(&self, conn: &Connection) {
        let table_definition = Self::as_table_definition();
        let mut stmt = conn.prepare(table_definition.as_insert_sql().as_str()).unwrap();
        let params: &[&dyn ToSql] = &[&self.trx_id, &self.tag];
        stmt.execute(params).unwrap();
    }
}

pub struct TrxLink {
    pub trx_id: Uuid,
    pub link: String,
}

impl AsTableDefinition for TrxLink {
    fn as_table_definition() -> TableDefinition {
        TableDefinition {
            name: "transaction_links",
            columns: vec![
                ColumnDefinition::new("trx_id", ColumnType::Uuid),
                ColumnDefinition::new("link", ColumnType::String),
            ],
        }
    }
    fn insert_data(&self, conn: &Connection) {
        let table_definition = Self::as_table_definition();
        let mut stmt = conn.prepare(table_definition.as_insert_sql().as_str()).unwrap();
        let params: &[&dyn ToSql] = &[&self.trx_id, &self.link];
        stmt.execute(params).unwrap();
    }
}

impl AsTableDefinition for CommodityDomain {
    fn as_table_definition() -> TableDefinition {
        TableDefinition {
            name: "commodities",
            columns: vec![
                ColumnDefinition::new("name", ColumnType::String),
                ColumnDefinition::new("precision", ColumnType::Int),
                ColumnDefinition::new("prefix", ColumnType::String).into_nullable(),
                ColumnDefinition::new("suffix", ColumnType::String).into_nullable(),
                ColumnDefinition::new("rounding", ColumnType::String),
            ],
        }
    }
    fn insert_data(&self, conn: &Connection) {
        let table_definition = Self::as_table_definition();
        let mut stmt = conn.prepare(table_definition.as_insert_sql().as_str()).unwrap();
        let params: &[&dyn ToSql] = &[
            &self.name,
            &self.precision,
            &self.prefix.clone(),
            &self.suffix.clone(),
            &self.rounding.to_string(),
        ];
        stmt.execute(params).unwrap();
    }
}

impl AsTableDefinition for PriceDomain {
    fn as_table_definition() -> TableDefinition {
        TableDefinition {
            name: "prices",
            columns: vec![
                ColumnDefinition::new("datetime", ColumnType::Date),
                ColumnDefinition::new("commodity", ColumnType::String),
                ColumnDefinition::new("amount", ColumnType::Decimal),
                ColumnDefinition::new("target_commodity", ColumnType::String),
            ],
        }
    }
    fn insert_data(&self, conn: &Connection)    {
        let table_definition = Self::as_table_definition();
        let mut stmt = conn.prepare(table_definition.as_insert_sql().as_str()).unwrap();
        let params: &[&dyn ToSql] = &[&self.datetime, &self.commodity, &self.amount.to_string(), &self.target_commodity];
        stmt.execute(params).unwrap();
    }
}

pub struct AccountCommodityLot {
    pub account: String,
    pub lot: CommodityLotRecord,
}

impl AsTableDefinition for AccountCommodityLot {
    fn as_table_definition() -> TableDefinition {
        TableDefinition {
            name: "commodity_lots",
            columns: vec![
                ColumnDefinition::new("account", ColumnType::String),
                ColumnDefinition::new("commodity", ColumnType::String),
                ColumnDefinition::new("amount", ColumnType::Decimal),
                ColumnDefinition::new("cost", ColumnType::Decimal).into_nullable(),
                ColumnDefinition::new("cost_commodity", ColumnType::String).into_nullable(),
                ColumnDefinition::new("acquisition_date", ColumnType::Date),
            ],
        }
    }
    fn insert_data(&self, conn: &Connection) {
        let table_definition = Self::as_table_definition();
        let mut stmt = conn.prepare(table_definition.as_insert_sql().as_str()).unwrap();
        let params: &[&dyn ToSql] = &[
            &self.account,
            &self.lot.commodity,
            &self.lot.amount.to_string(),
            &self.lot.cost.as_ref().map(|c| c.number.to_string()),
            &self.lot.cost.as_ref().map(|c| c.currency.clone()),
            &self.lot.acquisition_date,
        ];
        stmt.execute(params).unwrap();
    }
}


impl AsTableDefinition for DocumentDomain {
    fn as_table_definition() -> TableDefinition {
        TableDefinition {
            name: "documents",
            columns: vec![
                ColumnDefinition::new("datetime", ColumnType::Date),
                ColumnDefinition::new("document_type", ColumnType::String),
                ColumnDefinition::new("document_id", ColumnType::Uuid),
                ColumnDefinition::new("filename", ColumnType::String).into_nullable(),
                ColumnDefinition::new("path", ColumnType::String),
            ],
        }
    }
    fn insert_data(&self, conn: &Connection) {
        let table_definition = Self::as_table_definition();
        let mut stmt = conn.prepare(table_definition.as_insert_sql().as_str()).unwrap();
        let document_type = match &self.document_type {
            DocumentType::Trx(_) => "trx",
            DocumentType::Account(_) => "account",
        };
        let document_id = match &self.document_type {
            DocumentType::Trx(id) => id.to_string(),
            DocumentType::Account(account) => account.name().to_string(),
        };
        let params: &[&dyn ToSql] = &[&self.datetime, &document_type, &document_id, &self.filename.clone(), &self.path];
        stmt.execute(params).unwrap();
    }
}   

impl AsTableDefinition for MetaDomain {
    fn as_table_definition() -> TableDefinition {
        TableDefinition {
            name: "metas",
            columns: vec![
                ColumnDefinition::new("meta_type", ColumnType::String),
                ColumnDefinition::new("type_identifier", ColumnType::String),
                ColumnDefinition::new("key", ColumnType::String),
                ColumnDefinition::new("value", ColumnType::String),
            ],
        }
    }
    fn insert_data(&self, conn: &Connection) {
        let table_definition = Self::as_table_definition();
        let mut stmt = conn.prepare(table_definition.as_insert_sql().as_str()).unwrap();
        let params: &[&dyn ToSql] = &[&self.meta_type, &self.type_identifier, &self.key, &self.value];
        stmt.execute(params).unwrap();
    }
}
