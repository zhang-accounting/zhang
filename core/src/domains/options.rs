use crate::ZhangResult;
use sqlx::SqliteConnection;

pub struct OptionDomain;

impl OptionDomain {
    pub async fn insert_or_update(key: &str, value: &str, conn: &mut SqliteConnection) -> ZhangResult<()> {
        sqlx::query(r#"INSERT OR REPLACE INTO options VALUES ($1, $2);"#)
            .bind(key)
            .bind(value)
            .execute(conn)
            .await?;
        Ok(())
    }
}
