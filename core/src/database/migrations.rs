use sqlx::sqlite::SqliteConnection;
use sqlx::Acquire;

use crate::ZhangResult;

pub struct Migration;

static TABLES: [&str; 11] = [
    "options",
    "accounts",
    "metas",
    "commodities",
    "documents",
    "transactions",
    "transaction_links",
    "transaction_tags",
    "transaction_postings",
    "prices",
    "commodity_lots",
];
static VIEWS: [&str; 2] = ["account_balance", "account_daily_balance"];

static TABLES_SQL: [&str; 13] = [
    include_str!("./schemas/options.sql"),
    include_str!("./schemas/prices.sql"),
    include_str!("./schemas/accounts.sql"),
    include_str!("./schemas/metas.sql"),
    include_str!("./schemas/commodities.sql"),
    include_str!("./schemas/commodity_lots.sql"),
    include_str!("./schemas/documents.sql"),
    include_str!("./schemas/transactions.sql"),
    include_str!("./schemas/transaction_links.sql"),
    include_str!("./schemas/transaction_tags.sql"),
    include_str!("./schemas/transaction_postings.sql"),
    include_str!("./schemas/account_balance.sql"),
    include_str!("./schemas/account_daily_balance.sql"),
];

impl Migration {
    pub async fn init_database_if_missing(conn: &mut SqliteConnection) -> ZhangResult<()> {
        Migration::clear_tables(conn).await?;

        let mut trx = conn.begin().await?;
        for sql in TABLES_SQL {
            sqlx::query(sql).execute(&mut trx).await?;
        }
        trx.commit().await?;

        Ok(())
    }
    pub async fn clear_tables(conn: &mut SqliteConnection) -> ZhangResult<()> {
        let mut trx = conn.begin().await?;

        for table_name in TABLES {
            sqlx::query(&format!("DROP TABLE IF EXISTS {table_name}"))
                .execute(&mut trx)
                .await?;
        }

        for view_name in VIEWS {
            sqlx::query(&format!("DROP VIEW IF EXISTS {view_name}"))
                .execute(&mut trx)
                .await?;
        }
        trx.commit().await?;
        Ok(())
    }
}
