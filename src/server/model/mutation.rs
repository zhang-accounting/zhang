use std::fs::{File, OpenOptions};
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::str::FromStr;

use async_graphql::{Context, Object, Upload};
use chrono::{Datelike, Local, NaiveDateTime};
use itertools::{Either, Itertools};
use log::info;
use uuid::Uuid;

use crate::core::account::Account;
use crate::core::data::{Date, Document, Include};
use crate::core::models::{Directive, ZhangString};
use crate::server::LedgerState;
use crate::target::ZhangTarget;

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
        let (entry, endpoint) = match &ledger_stage.entry {
            Either::Left(path) => path,
            Either::Right(_) => {
                return false;
            }
        };
        let target_file_path = entry.join(format!("data/{}/{}.zhang", time.year(), time.month()));

        if !target_file_path.exists() {
            std::fs::create_dir_all(&target_file_path.parent().unwrap()).expect("cannot create folder recursive");
            std::fs::write(&target_file_path, "").expect("cannot generate empty file");
        }

        let buf = target_file_path.canonicalize().unwrap();
        if !ledger_stage.visited_files.contains(&buf) {
            let path = match target_file_path.strip_prefix(entry) {
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
                .open(&entry.join(endpoint))
                .unwrap();
            ledger_base_file
                .write_all(format!("\n{}\n", include_directive).as_bytes())
                .unwrap();
        }

        let mut file = OpenOptions::new().append(true).create(true).open(buf).unwrap();
        file.write_all(content.as_bytes()).unwrap();
        true
    }

    async fn upload_account_document(&self, ctx: &Context<'_>, account_name: String, files: Vec<Upload>) -> bool {
        let ledger_stage = ctx.data_unchecked::<LedgerState>().write().await;
        let (entry, _endpoint) = match &ledger_stage.entry {
            Either::Left(path) => path,
            Either::Right(_) => {
                return false;
            }
        };
        let documents = files
            .into_iter()
            .map(|file| {
                let file = file.value(ctx).unwrap();
                let v4 = Uuid::new_v4();

                let buf = entry.join("attachments").join(v4.to_string()).join(&file.filename);
                info!("upload: filename={} direction={}", file.filename, buf.display());
                create_folder_if_not_exist(&buf);
                let file1 = file.content;
                let mut reader = BufReader::new(file1);

                let mut buf_content = vec![];
                reader.read_to_end(&mut buf_content).expect("Cannot read file");

                let f = File::create(&buf).expect("Unable to create file");
                let mut f = BufWriter::new(f);
                f.write_all(&buf_content).expect("cannot wirte content");
                let path = match buf.strip_prefix(&entry) {
                    Ok(relative_path) => relative_path.to_str().unwrap(),
                    Err(_) => buf.to_str().unwrap(),
                };
                Document {
                    date: Date::Datetime(Local::now().naive_local()),
                    account: Account::from_str(&account_name).unwrap(),
                    filename: ZhangString::QuoteString(path.to_string()),
                    tags: None,
                    links: None,
                    meta: Default::default(),
                }
            })
            .map(Directive::Document)
            .collect_vec();
        let time = Local::now().naive_local();

        ledger_stage.append_directives(documents, format!("data/{}/{}.zhang", time.year(), time.month()));
        true
    }
}

pub(crate) fn create_folder_if_not_exist(filename: &Path) {
    std::fs::create_dir_all(&filename.parent().unwrap()).expect("cannot create folder recursive");
}
