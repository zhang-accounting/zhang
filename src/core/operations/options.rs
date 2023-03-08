use sqlx::SqliteConnection;
use crate::error::ZhangResult;

pub struct OptionDomain;

impl OptionDomain {
    pub async fn get(key: &str, conn: &mut SqliteConnection) -> ZhangResult<Option<String>> {
        let option = sqlx::query_as::<_, (String, )>(
            r#"
                select value from options where key = $1
                "#,
        )
            .bind(key)
            .fetch_optional(conn)
            .await?;
        Ok(option.map(|it|it.0))
    }
}
