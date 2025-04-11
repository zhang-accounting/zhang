use chrono::{DateTime, Utc};
use duckdb::types::{TimeUnit, ValueRef};
use zhang_core::domains::schemas::{CommodityDomain, MetaDomain, PriceDomain};
use zhang_core::store::{DocumentDomain, PostingDomain, TransactionDomain};
use zhang_core::{domains::schemas::AccountDomain, ledger::Ledger, options::InMemoryOptions};

mod table_definition;
use table_definition::{AccountCommodityLot, AsTableDefinition, TableDefinition, TrxLink, TrxTag};
use serde::Serialize;
use gotcha::Schematic;

use duckdb::{Connection, Result};

#[derive(Debug, Serialize, Schematic)]
pub struct Column {
    name: String,
    value: serde_json::Value,
}

#[derive(Debug, Serialize, Schematic)]
pub struct Row {
    columns: Vec<Column>,
}


fn value_ref_to_value(value: ValueRef<'_>) -> serde_json::Value {
    match value {
        ValueRef::Text(s) => serde_json::Value::String(String::from_utf8_lossy(s).to_string()),
        ValueRef::Int(i) => serde_json::Value::Number(serde_json::Number::from(i)),
        ValueRef::Float(f) => serde_json::Value::Number(serde_json::Number::from_f64(f as f64).unwrap()),
        ValueRef::Boolean(b) => serde_json::Value::Bool(b),
        ValueRef::Timestamp(TimeUnit::Microsecond, i) => {
            serde_json::Value::String(
                DateTime::<Utc>::from_timestamp_micros(i).unwrap().to_string()
            )
        },
        _ => serde_json::Value::Null,
    }
}


pub struct Executor {
    pub table_definitions: Vec<TableDefinition>,
    conn: Connection,
}

#[derive(Debug, Serialize, Schematic)]
pub struct ExecutionResult {
    columns: Vec<String>,
    rows: Vec<Row>,
}

impl Executor {
    pub fn new(table_definitions: Vec<TableDefinition>) -> Self {
        let conn = Connection::open_in_memory().unwrap();

        // create tables
        for table_definition in &table_definitions {
            conn.execute(table_definition.as_sql().as_str(), []).unwrap();
        }
        Self { table_definitions, conn }
    }

    pub fn execute(&self, query: &str) -> Result<ExecutionResult, duckdb::Error> {
        let mut stmt = self.conn.prepare(query)?;
        let mut rows = stmt.query([])?;
        
        let mut ret = vec![];
        let mut is_first = true;
        let mut column_names = vec![];
        while let Some(row) = rows.next().unwrap() {
            if is_first {
                column_names = row.as_ref().column_names();
                is_first = false;
            }

            let mut columns = Vec::with_capacity(column_names.len());
            for (i, col_name) in column_names.iter().enumerate() {
                let value: ValueRef = row.get_ref(i)?;
                let value = value_ref_to_value(value);
                columns.push(Column  { name: col_name.to_string(), value });
            }
            ret.push(Row { columns });
        }

        Ok(ExecutionResult { columns: column_names, rows: ret })
    }
}

pub trait AsExecutor {
    fn as_executor(&self) -> Executor;
}

impl AsExecutor for Ledger {
    fn as_executor(&self) -> Executor {
        let table_definitions = vec![
            InMemoryOptions::as_table_definition(), 
            AccountDomain::as_table_definition(),
            CommodityDomain::as_table_definition(),
            TransactionDomain::as_table_definition(),
            PostingDomain::as_table_definition(),
            AccountCommodityLot::as_table_definition(),
            TrxTag::as_table_definition(),
            TrxLink::as_table_definition(),
            PriceDomain::as_table_definition(),
            DocumentDomain::as_table_definition(),
            MetaDomain::as_table_definition(),
        ];
        let executor = Executor::new(table_definitions);
        self.options.insert_data(&executor.conn);
        let store = self.store.read().unwrap();
        for account in store.accounts.values() {
            account.insert_data(&executor.conn);
        }
        for trx in store.transactions.values() {
            trx.insert_data(&executor.conn);
            for tag in &trx.tags {
                TrxTag {
                    trx_id: trx.id,
                    tag: tag.clone(),
                }.insert_data(&executor.conn);
            }
            for link in &trx.links {
                TrxLink {
                    trx_id: trx.id,
                    link: link.clone(),
                }.insert_data(&executor.conn);
            }
            
        }
        for posting in store.postings.iter() {
            posting.insert_data(&executor.conn);
        }
        for commodity in store.commodities.values() {
            commodity.insert_data(&executor.conn);
        }
        for price in store.prices.iter() {
            price.insert_data(&executor.conn);
        }
        for (account, commodity_lots) in store.commodity_lots.iter() {
            for commodity_lot in commodity_lots.iter() {
                AccountCommodityLot {
                    account: account.clone(),
                    lot: commodity_lot.clone(),
                }.insert_data(&executor.conn);
            }
        }
        for document in store.documents.iter() {
            document.insert_data(&executor.conn);
        }
        for meta in store.metas.iter() {
            meta.insert_data(&executor.conn);
        }
        executor
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use indoc::indoc;
    use tempfile::tempdir;
    use zhang_core::{data_source::LocalFileSystemDataSource, data_type::text::ZhangDataType, ledger::Ledger};

    fn load_from_temp_str(content: &str) -> Ledger {
        let temp_dir = tempdir().unwrap().into_path();
        let example = temp_dir.join("example.zhang");
        std::fs::write(example, content).unwrap();
        let source = LocalFileSystemDataSource::new(ZhangDataType {});
        Ledger::load_with_data_source(temp_dir, "example.zhang".to_string(), Arc::new(source)).unwrap()
    }

    #[test]
    fn it_works() {
        let ledger = load_from_temp_str(indoc! {r#"
            option "title" "Accounting"
        "#});
        let executor = ledger.as_executor();
        let result = executor.execute("select * from options").unwrap();
        println!("{:?}", result);
        assert_eq!(result.rows.len(), 6);
    }

    #[test]
    fn it_works2() {
        let ledger = load_from_temp_str(indoc! {r#"
            1970-01-01 open Assets:Cash
        "#});
        let executor = ledger.as_executor();
        let result = executor.execute("select * from accounts").unwrap();
        println!("{:?}", result);
        assert_eq!(result.rows.len(), 1);
    }

}
