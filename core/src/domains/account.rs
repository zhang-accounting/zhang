use crate::ZhangResult;
use sqlx::SqliteConnection;

pub struct AccountDomain;

impl AccountDomain {
    pub async fn exists(name: &str, conn: &mut SqliteConnection) -> ZhangResult<bool> {
        Ok(sqlx::query("select 1 from accounts where name = $1")
            .bind(name)
            .fetch_optional(conn)
            .await?
            .is_some())
    }
}
