use crate::domains::schemas::{AccountDailyBalanceDomain, CommodityDomain, MetaDomain, OptionDomain, PriceDomain};
use crate::ZhangResult;
use chrono::NaiveDateTime;
use itertools::Itertools;
use sqlx::pool::PoolConnection;
use sqlx::{Acquire, FromRow, Sqlite};

pub mod options;
pub mod schemas;

#[derive(FromRow)]
struct ValueRow {
    value: String,
}

pub struct Operations {
    pub(crate) pool: PoolConnection<Sqlite>,
}
impl Operations {
    pub async fn options(&mut self, key: impl AsRef<str>) -> ZhangResult<Option<OptionDomain>> {
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
        let conn = self.pool.acquire().await?;
        Ok(sqlx::query_as::<_, PriceDomain>(
            "select datetime, commodity, amount, target_commodity from prices where datetime <= $1 and commodity = $2 and target_commodity = $3",
        )
        .bind(date)
        .bind(from.as_ref())
        .bind(to.as_ref())
        .fetch_optional(conn)
        .await?)
    }

    pub async fn metas(&mut self, type_: impl AsRef<str>, type_identifier: impl AsRef<str>) -> ZhangResult<Vec<MetaDomain>> {
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
}
