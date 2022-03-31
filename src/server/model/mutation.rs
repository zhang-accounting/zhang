use crate::core::data::Include;
use crate::core::models::{Directive, ZhangString};
use crate::server::LedgerState;
use crate::target::ZhangTarget;
use async_graphql::{Context, Object};
use chrono::{Datelike, NaiveDateTime};
use itertools::Either;
use std::fs::OpenOptions;
use std::io::Write;
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
            ledger_stage.reload().is_ok()
        } else {
            false
        }
    }

    async fn append_data(&self, ctx: &Context<'_>, date: i64, content: String) -> bool {
        let time = NaiveDateTime::from_timestamp(date, 0);
        let ledger_stage = ctx.data_unchecked::<LedgerState>().write().await;
        let entry_path = match &ledger_stage.entry {
            Either::Left(path) => path,
            Either::Right(_) => {
                return false;
            }
        };
        let ledger_base_path = entry_path.parent().unwrap();
        let target_file_path =
            ledger_base_path.join(format!("data/{}/{}.zhang", time.year(), time.month()));

        if !target_file_path.exists() {
            std::fs::create_dir_all(&target_file_path.parent().unwrap())
                .expect("cannot create folder recursive");
            std::fs::write(&target_file_path, "").expect("cannot generate empty file");
        }

        let buf = target_file_path.canonicalize().unwrap();
        if !ledger_stage.visited_files.contains(&buf) {
            let path = match target_file_path.strip_prefix(ledger_base_path) {
                Ok(relative_path) => relative_path.to_str().unwrap(),
                Err(_) => target_file_path.to_str().unwrap(),
            };
            let include_directive = Directive::Include(Include {
                file: ZhangString::QuoteString(path.to_string()),
            })
            .to_target();
            let mut ledger_base_file = OpenOptions::new()
                .append(true)
                .create(true)
                .open(&entry_path)
                .unwrap();
            ledger_base_file
                .write_all(format!("\n{}\n", include_directive).as_bytes())
                .unwrap();
        }

        let mut file = OpenOptions::new()
            .append(true)
            .create(true)
            .open(buf)
            .unwrap();
        file.write_all(content.as_bytes()).unwrap();
        true
    }
}
