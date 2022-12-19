use crate::core::data::CommodityDetail;
use crate::error::ZhangResult;
use sqlx::SqliteConnection;

pub struct CommodityOperation;

impl CommodityOperation {
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
}
