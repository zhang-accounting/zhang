use crate::ZhangResult;
use sqlx::SqliteConnection;

pub struct OptionDomain;

impl OptionDomain {
    pub async fn get(key: &str, conn: &mut SqliteConnection) -> ZhangResult<Option<String>> {
        let option = sqlx::query_as::<_, (String,)>(
            r#"
                select value from options where key = $1
                "#,
        )
        .bind(key)
        .fetch_optional(conn)
        .await?;
        Ok(option.map(|it| it.0))
    }
    pub async fn insert_or_update(key: &str, value: &str, conn: &mut SqliteConnection) -> ZhangResult<()> {
        sqlx::query(r#"INSERT OR REPLACE INTO options VALUES ($1, $2);"#)
            .bind(key)
            .bind(value)
            .execute(conn)
            .await?;
        Ok(())
    }
}
