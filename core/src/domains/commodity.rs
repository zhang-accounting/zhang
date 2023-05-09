use crate::ZhangResult;
use sqlx::{FromRow, SqliteConnection};

pub struct CommodityDomain;

#[derive(Debug, Clone, FromRow)]
pub struct CommodityDetail {
    pub name: String,
    pub precision: i32,
    pub prefix: Option<String>,
    pub suffix: Option<String>,
    pub rounding: Option<String>,
}

impl CommodityDomain {
    pub async fn get_by_name(name: &str, conn: &mut SqliteConnection) -> ZhangResult<Option<CommodityDetail>> {
        let option = sqlx::query_as::<_, CommodityDetail>(
            r#"
                select * from commodities where name = $1
                "#,
        )
        .bind(name)
        .fetch_optional(conn)
        .await?;
        Ok(option)
    }
    pub async fn exists(name: &str, conn: &mut SqliteConnection) -> ZhangResult<bool> {
        Ok(sqlx::query("select 1 from commodities where name = $1")
            .bind(name)
            .fetch_optional(conn)
            .await?
            .is_some())
    }
}
