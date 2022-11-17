use sqlx::Acquire;
use crate::error::ZhangResult;
use sqlx::sqlite::SqliteConnection;

pub struct Migration;


static TABLES: [&'static str; 4] = ["options", "accounts", "metas", "commodities"];

impl Migration {
    pub async fn init_database_if_missing(conn: &mut SqliteConnection) -> ZhangResult<()> {
        Migration::clear_tables(conn).await?;
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
        sqlx::query(r#"create table commodities(name      varchar not null constraint commodities_pk primary key, precision INTEGER, prefix    varchar, suffix    varchar, rounding  varchar);"#)
            .execute(&mut trx)
            .await?;


        //clear all tables

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
        trx.commit().await?;
        Ok(())
    }
}