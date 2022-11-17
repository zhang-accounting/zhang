use sqlx::Acquire;
use crate::error::ZhangResult;
use sqlx::sqlite::SqliteConnection;
pub struct Migration;


static TABLES: [&'static str; 3] = ["options", "accounts", "metas"];

impl Migration {

    pub async fn init_database_if_missing(conn: &mut SqliteConnection) -> ZhangResult<()> {
        let mut trx = conn.begin().await?;
        // options
        sqlx::query(r#"CREATE TABLE if not exists "options" ("key" varchar NOT NULL,"value" varchar, PRIMARY KEY (key));"#)
            .execute(&mut trx)
            .await?;
        sqlx::query(r#"CREATE TABLE if not exists "accounts" ("date" datetime NOT NULL, "name" varchar NOT NULL,"status" varchar NOT NULL,"alias" varchar, PRIMARY KEY (name));"#)
            .execute(&mut trx)
            .await?;
        sqlx::query(r#"CREATE TABLE if not exists "metas" ("type" varchar NOT NULL, "type_identifier" varchar NOT NULL, "key" varchar NOT NULL,"value" varchar);"#)
            .execute(&mut trx)
            .await?;


        //clear all tables

        trx.commit().await?;
        Migration::clear_tables(conn).await?;
        Ok(())
    }
    pub async fn clear_tables(conn:&mut SqliteConnection) -> ZhangResult<()> {
        let mut trx = conn.begin().await?;

        for table_name in TABLES {
            sqlx::query(&format!("DELETE FROM {table_name}"))
                .execute(&mut trx)
                .await?;
        }
        trx.commit().await?;
        Ok(())
    }
}