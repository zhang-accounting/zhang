use crate::domains::schemas::AccountDailyBalance;
use crate::ZhangResult;
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
    pub async fn accounts_latest_balance(&mut self) -> ZhangResult<Vec<AccountDailyBalance>> {
        let mut conn = self.pool.acquire().await?;
        Ok(sqlx::query_as::<_, AccountDailyBalance>(
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
}
