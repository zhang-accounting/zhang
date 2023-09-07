use crate::domains::schemas::{
    AccountBalanceDomain, AccountDailyBalanceDomain, AccountDomain, AccountJournalDomain, CommodityDomain, ErrorDomain, ErrorType, MetaDomain, MetaType,
    OptionDomain, PriceDomain, TransactionInfoDomain,
};
use crate::ZhangResult;
use chrono::{NaiveDate, NaiveDateTime, TimeZone};
use chrono_tz::Tz;
use itertools::Itertools;
use sqlx::pool::PoolConnection;
use sqlx::{Acquire, FromRow, Sqlite};
use std::collections::HashMap;
use std::path::PathBuf;
use uuid::Uuid;
use zhang_ast::{Meta, SpanInfo};
use crate::database::type_ext::big_decimal::ZhangBigDecimal;

pub mod schemas;

#[derive(FromRow)]
struct ValueRow {
    value: String,
}


#[derive(FromRow)]
pub struct StaticRow {
    pub date: NaiveDate,
    pub account_type: String,
    pub amount: ZhangBigDecimal,
    pub commodity: String,
}


pub struct Operations {
    pub(crate) pool: PoolConnection<Sqlite>,
    pub timezone: Tz,
}


impl Operations {
    pub async fn options(&mut self) -> ZhangResult<Vec<OptionDomain>> {
        let conn = self.pool.acquire().await?;

        let options = sqlx::query_as::<_, OptionDomain>(
            r#"
                select key, value from options
                "#,
        )
        .fetch_all(conn)
        .await?;
        Ok(options)
    }
    pub async fn option(&mut self, key: impl AsRef<str>) -> ZhangResult<Option<OptionDomain>> {
        let conn = self.pool.acquire().await?;

        let option = sqlx::query_as::<_, OptionDomain>(
            r#"
                select key, value from options where key = $1
                "#,
        )
        .bind(key.as_ref())
        .fetch_optional(conn)
        .await?;
        Ok(option)
    }

    pub async fn accounts_latest_balance(&mut self) -> ZhangResult<Vec<AccountDailyBalanceDomain>> {
        let conn = self.pool.acquire().await?;
        Ok(sqlx::query_as::<_, AccountDailyBalanceDomain>(
            r#"
                SELECT
                    date(datetime) AS date,
                    account,
                    balance_number,
                    balance_commodity
                FROM
                    account_daily_balance
                GROUP BY
                    account
                HAVING
                    max(datetime)
            "#,
        )
        .fetch_all(conn)
        .await?)
    }

    pub async fn get_price(&mut self, date: NaiveDateTime, from: impl AsRef<str>, to: impl AsRef<str>) -> ZhangResult<Option<PriceDomain>> {
        let datetime = self.timezone.from_local_datetime(&date).unwrap();
        let conn = self.pool.acquire().await?;
        Ok(sqlx::query_as::<_, PriceDomain>(
            "select datetime, commodity, amount, target_commodity from prices where datetime <= $1 and commodity = $2 and target_commodity = $3",
        )
        .bind(datetime)
        .bind(from.as_ref())
        .bind(to.as_ref())
        .fetch_optional(conn)
        .await?)
    }

    pub async fn metas(&mut self, type_: MetaType, type_identifier: impl AsRef<str>) -> ZhangResult<Vec<MetaDomain>> {
        let conn = self.pool.acquire().await?;

        let rows = sqlx::query_as::<_, MetaDomain>(
            r#"
            select type as meta_type, type_identifier, key, value from metas where type = $1 and type_identifier = $2
        "#,
        )
        .bind(type_.as_ref())
        .bind(type_identifier.as_ref())
        .fetch_all(conn)
        .await?;
        Ok(rows)
    }

    pub async fn trx_tags(&mut self, trx_id: impl AsRef<str>) -> ZhangResult<Vec<String>> {
        let conn = self.pool.acquire().await?;

        let rows = sqlx::query_as::<_, ValueRow>(
            r#"
        select tag as value from transaction_tags where trx_id = $1
        "#,
        )
        .bind(trx_id.as_ref())
        .fetch_all(conn)
        .await?;
        Ok(rows.into_iter().map(|it| it.value).collect_vec())
    }

    pub async fn trx_links(&mut self, trx_id: impl AsRef<str>) -> ZhangResult<Vec<String>> {
        let conn = self.pool.acquire().await?;

        let rows = sqlx::query_as::<_, ValueRow>(
            r#"
        select link as value from transaction_links where trx_id = $1
        "#,
        )
        .bind(trx_id.as_ref())
        .fetch_all(conn)
        .await?;
        Ok(rows.into_iter().map(|it| it.value).collect_vec())
    }

    pub async fn commodity(&mut self, name: &str) -> ZhangResult<Option<CommodityDomain>> {
        let conn = self.pool.acquire().await?;

        let option = sqlx::query_as::<_, CommodityDomain>(
            r#"
                select * from commodities where name = $1
                "#,
        )
        .bind(name)
        .fetch_optional(conn)
        .await?;
        Ok(option)
    }
    pub async fn exist_commodity(&mut self, name: &str) -> ZhangResult<bool> {
        let conn = self.pool.acquire().await?;

        Ok(sqlx::query("select 1 from commodities where name = $1")
            .bind(name)
            .fetch_optional(conn)
            .await?
            .is_some())
    }

    pub async fn exist_account(&mut self, name: &str) -> ZhangResult<bool> {
        let conn = self.pool.acquire().await?;

        Ok(sqlx::query("select 1 from accounts where name = $1")
            .bind(name)
            .fetch_optional(conn)
            .await?
            .is_some())
    }

    pub async fn transaction_counts(&mut self) -> ZhangResult<i64> {
        let conn = self.pool.acquire().await?;
        Ok(sqlx::query_as::<_, (i64,)>(r#"select count(1) from transactions"#).fetch_one(conn).await?.0)
    }

    pub async fn transaction_span(&mut self, id: &str) -> ZhangResult<TransactionInfoDomain> {
        let conn = self.pool.acquire().await?;
        Ok(
            sqlx::query_as::<_, TransactionInfoDomain>(r#"select id, source_file, span_start, span_end from transactions where id = $1"#)
                .bind(id)
                .fetch_one(conn)
                .await?,
        )
    }

    pub async fn account_balances(&mut self) -> ZhangResult<Vec<AccountBalanceDomain>> {
        let conn = self.pool.acquire().await?;
        Ok(sqlx::query_as::<_, AccountBalanceDomain>(
            r#"
                        select datetime, account, account_status, balance_number, balance_commodity
                        from account_balance
            "#,
        )
        .fetch_all(conn)
        .await?)
    }

    pub async fn single_account_balances(&mut self, account_name: &str) -> ZhangResult<Vec<AccountBalanceDomain>> {
        let conn = self.pool.acquire().await?;
        Ok(sqlx::query_as::<_, AccountBalanceDomain>(
            r#"
                select datetime, account, account_status, balance_number, balance_commodity
                from account_balance
                where account = $1
            "#,
        )
        .bind(account_name)
        .fetch_all(conn)
        .await?)
    }

    pub async fn account_journals(&mut self, account: &str) -> ZhangResult<Vec<AccountJournalDomain>> {
        let conn = self.pool.acquire().await?;
        Ok(sqlx::query_as::<_, AccountJournalDomain>(
            r#"
                    select datetime,
                           trx_id,
                           account,
                           payee,
                           narration,
                           inferred_unit_number,
                           inferred_unit_commodity,
                           account_after_number,
                           account_after_commodity
                    from transaction_postings
                             join transactions on transactions.id = transaction_postings.trx_id
                    where account = $1
                    order by datetime desc, transactions.sequence desc
            "#,
        )
        .bind(account)
        .fetch_all(conn)
        .await?)
    }
    pub async fn account_dated_journals(&mut self, account_type: &str, from: NaiveDateTime, to: NaiveDateTime) -> ZhangResult<Vec<AccountJournalDomain>> {
        let conn = self.pool.acquire().await?;
        Ok(sqlx::query_as::<_, AccountJournalDomain>(
            r#"
                select datetime,
                       trx_id,
                       account,
                       payee,
                       narration,
                       inferred_unit_number,
                       inferred_unit_commodity,
                       account_after_number,
                       account_after_commodity
                from transaction_postings
                         join transactions on transactions.id = transaction_postings.trx_id
                         join accounts on accounts.name = transaction_postings.account
                where datetime >= $1
                  and datetime <= $2
                  and accounts.type = $3
            "#,
        )
        .bind(from)
        .bind(to)
        .bind(account_type)
        .fetch_all(conn)
        .await?)
    }

    pub async fn errors(&mut self) -> ZhangResult<Vec<ErrorDomain>> {
        let conn = self.pool.acquire().await?;

        #[derive(FromRow)]
        struct ErrorRow {
            pub id: String,
            pub filename: Option<String>,
            pub span_start: Option<i64>,
            pub span_end: Option<i64>,
            pub content: String,
            pub error_type: ErrorType,
            pub metas: String,
        }

        let rows = sqlx::query_as::<_, ErrorRow>(
            r#"
            select
                id, filename, span_start, span_end, content, content, error_type, metas
            from errors
        "#,
        )
        .fetch_all(conn)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| {
                let span = match (row.span_start, row.span_end) {
                    (Some(start), Some(end)) => Some(SpanInfo {
                        start: start as usize,
                        end: end as usize,
                        content: row.content,
                        filename: row.filename.map(PathBuf::from),
                    }),
                    _ => None,
                };
                ErrorDomain {
                    id: row.id,
                    span,
                    error_type: row.error_type,
                    metas: serde_json::from_str(&row.metas).unwrap(),
                }
            })
            .collect_vec())
    }

    pub async fn account(&mut self, account_name: &str) -> ZhangResult<Option<AccountDomain>> {
        let conn = self.pool.acquire().await?;
        Ok(
            sqlx::query_as::<_, AccountDomain>(r#"select date, type, name, status, alias from accounts where name = $1"#)
                .bind(account_name)
                .fetch_optional(conn)
                .await?,
        )
    }
    pub async fn all_open_accounts(&mut self) -> ZhangResult<Vec<AccountDomain>> {
        let conn = self.pool.acquire().await?;
        Ok(
            sqlx::query_as::<_, AccountDomain>(r#"select date, type, name, status, alias from accounts WHERE status = 'Open'"#)
                .fetch_all(conn)
                .await?,
        )
    }
    pub async fn all_accounts(&mut self) -> ZhangResult<Vec<String>> {
        let conn = self.pool.acquire().await?;
        let accounts = sqlx::query_as::<_, ValueRow>("select name as value from accounts")
            .fetch_all(conn)
            .await?
            .into_iter()
            .map(|it| it.value)
            .collect_vec();
        Ok(accounts)
    }

    pub async fn all_payees(&mut self) -> ZhangResult<Vec<String>> {
        let conn = self.pool.acquire().await?;

        #[derive(FromRow)]
        struct PayeeRow {
            payee: String,
        }
        // todo(sqlx): move to operation
        let payees = sqlx::query_as::<_, PayeeRow>(
            r#"
        select distinct payee from transactions
        "#,
        )
            .fetch_all( conn)
            .await?;
        Ok(
            payees.into_iter().map(|it| it.payee).filter(|it| !it.is_empty()).collect_vec()
        )
    }

    pub async fn static_duration(&mut self, from: NaiveDateTime, to: NaiveDateTime) -> ZhangResult<Vec<StaticRow>> {

        let conn = self.pool.acquire().await?;
        let rows = sqlx::query_as::<_, StaticRow>(
            r#"
        SELECT
            date(datetime) AS date,
            accounts.type AS account_type,
            sum(inferred_unit_number) AS amount,
            inferred_unit_commodity AS commodity
        FROM
            transaction_postings
            JOIN transactions ON transactions.id = transaction_postings.trx_id
            JOIN accounts ON accounts.name = transaction_postings.account
            where transactions.datetime >= $1 and transactions.datetime <= $2
        GROUP BY
            date(datetime),
            accounts.type,
            inferred_unit_commodity
    "#,
        )
            .bind(from)
            .bind(to)
            .fetch_all(conn)
            .await?;

        Ok(rows)
    }
}

// for insert and new operations
impl Operations {
    pub async fn new_error(&mut self, error_type: ErrorType, span: &SpanInfo, metas: HashMap<String, String>) -> ZhangResult<()> {
        let conn = self.pool.acquire().await?;
        sqlx::query(
            r#"
            INSERT OR REPLACE INTO
                errors(id, filename, span_start, span_end, content, error_type, metas)
            VALUES
                ($1, $2, $3, $4, $5, $6, $7);
        "#,
        )
        .bind(Uuid::new_v4().to_string())
        .bind(span.filename.as_ref().and_then(|it| it.to_str()))
        .bind(span.start as i64)
        .bind(span.end as i64)
        .bind(&span.content)
        .bind(error_type)
        .bind(serde_json::to_string(&metas).unwrap())
        .execute(conn)
        .await?;
        Ok(())
    }

    pub async fn insert_or_update_options(&mut self, key: &str, value: &str) -> ZhangResult<()> {
        let conn = self.pool.acquire().await?;
        sqlx::query(r#"INSERT OR REPLACE INTO options VALUES ($1, $2);"#)
            .bind(key)
            .bind(value)
            .execute(conn)
            .await?;
        Ok(())
    }

    pub async fn insert_meta(&mut self, type_: MetaType, type_identifier: impl AsRef<str>, meta: Meta) -> ZhangResult<()> {
        let conn = self.pool.acquire().await?;
        for (meta_key, meta_value) in meta.get_flatten() {
            sqlx::query(r#"INSERT OR REPLACE INTO metas VALUES ($1, $2, $3, $4);"#)
                .bind(type_.as_ref())
                .bind(type_identifier.as_ref())
                .bind(meta_key)
                .bind(meta_value.as_str())
                .execute(&mut *conn)
                .await?;
        }
        Ok(())
    }

    pub async fn close_account(&mut self, account_name: &str) -> ZhangResult<()> {
        let conn = self.pool.acquire().await?;
        sqlx::query(r#"update accounts set status = 'Close' where name = $1"#)
            .bind(account_name)
            .execute(conn)
            .await?;
        Ok(())
    }

    pub async fn insert_commodity(&mut self, name: &String, precision: Option<i32>, prefix: Option<String>, suffix: Option<String>, rounding: Option<String>) -> ZhangResult<()> {
        let conn = self.pool.acquire().await?;
        sqlx::query(
            r#"INSERT OR REPLACE INTO commodities (name, precision, prefix, suffix, rounding)
                        VALUES ($1, $2, $3, $4, $5);"#,
        )
            .bind(name)
            .bind(precision)
            .bind(prefix)
            .bind(suffix)
            .bind(rounding)
            .execute(conn)
            .await?;
        Ok(())
    }
}
