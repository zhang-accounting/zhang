use crate::server::LedgerState;
use async_graphql::{Context, Object};
use std::path::PathBuf;
use std::str::FromStr;

pub struct MutationRoot;

#[Object]
impl MutationRoot {
    async fn update_file(&self, ctx: &Context<'_>, path: String, content: String) -> bool {
        let (path_buf, contains_file) = {
            let ledger_stage = ctx.data_unchecked::<LedgerState>().read().await;
            let path_buf = PathBuf::from_str(&path).expect("cannot be a path");
            let contains_file = ledger_stage.visited_files.contains(&path_buf);
            (path_buf, contains_file)
        };
        if contains_file {
            std::fs::write(path_buf, content).expect("cannot read");
            let mut ledger_stage = ctx.data_unchecked::<LedgerState>().write().await;
            ledger_stage.reload();
            true
        } else {
            false
        }
    }
}
