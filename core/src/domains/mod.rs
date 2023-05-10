use crate::domains::schemas::{AccountDailyBalanceDomain, MetaDomain, PriceDomain};
use crate::ZhangResult;
use chrono::NaiveDateTime;
use sqlx::pool::PoolConnection;
use sqlx::{Acquire, Sqlite};

pub mod account;
pub mod commodity;
pub mod options;
pub mod schemas;

pub struct Operations {
    pub(crate) pool: PoolConnection<Sqlite>,
}

impl Operations {
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

    pub async fn get_price(
        &mut self, date: NaiveDateTime, from: impl AsRef<str>, to: impl AsRef<str>,
    ) -> ZhangResult<Option<PriceDomain>> {
        let conn = self.pool.acquire().await?;
        Ok(sqlx::query_as::<_, PriceDomain>(
            "select * from prices where datetime <= $1 and commodity = $2 and target_commodity = $3",
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
}
