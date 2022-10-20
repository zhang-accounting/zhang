use std::cmp::min;
use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};
use std::option::Option::None;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use async_graphql::{Context, Object, Upload};
use chrono::{Datelike, Local, NaiveDateTime};
use itertools::{Either, Itertools};
use log::info;
use uuid::Uuid;

use crate::core::account::Account;
use crate::core::data::{Date, Document, Meta};
use crate::core::models::{Directive, ZhangString};
use crate::parse_zhang;
use crate::server::LedgerState;
use crate::target::ZhangTarget;

pub struct MutationRoot;

#[Object]
impl MutationRoot {
    async fn update_file(&self, ctx: &Context<'_>, path: String, content: String) -> bool {
        if parse_zhang(&content, None).is_err() {
            return false;
        }
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
        match parse_zhang(&content, None) {
            Ok(directives) => {
                let directives = directives.into_iter().map(|it| it.data).collect_vec();
                ledger_stage.append_directives(directives, format!("data/{}/{}.zhang", time.year(), time.month()));
                true
            }
            Err(_) => false,
        }
    }
    async fn modify_data(&self, ctx: &Context<'_>, file: String, content: String, start: usize, end: usize) -> bool {
        if parse_zhang(&content, None).is_err() {
            return false;
        }
        let mut ledger_stage = ctx.data_unchecked::<LedgerState>().write().await;
        let (entry, _endpoint) = match &ledger_stage.entry {
            Either::Left(path) => path,
            Either::Right(_) => {
                return false;
            }
        };
        let file_path = entry.join(file);
        replace_file_via_lines(file_path, &content, start, end);
        ledger_stage.reload().is_ok()
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

    async fn upload_transaction_document(
        &self, ctx: &Context<'_>, transaction_file: String, transaction_end_line: usize, files: Vec<Upload>,
    ) -> bool {
        let ledger_stage = ctx.data_unchecked::<LedgerState>().write().await;
        let (entry, _endpoint) = match &ledger_stage.entry {
            Either::Left(path) => path,
            Either::Right(_) => {
                return false;
            }
        };
        let mut metas = Meta::default();
        files.into_iter().for_each(|file| {
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

            metas.insert("document".to_string(), ZhangString::QuoteString(path.to_string()));
        });

        let text = metas
            .to_target()
            .into_iter()
            .map(|line| format!("  {}", line))
            .join("\n");

        insert_line(PathBuf::from(transaction_file), text.as_str(), transaction_end_line);
        true
    }
}

pub(crate) fn create_folder_if_not_exist(filename: &Path) {
    std::fs::create_dir_all(&filename.parent().unwrap()).expect("cannot create folder recursive");
}

pub(crate) fn replace_file_via_lines(file: PathBuf, content: &str, start: usize, end: usize) {
    let file_content = std::fs::read_to_string(&file).expect("cannot read file");
    let mut lines = file_content
        .lines()
        .enumerate()
        .filter(|(idx, _c)| idx < &(start - 1) || idx > &(end - 1))
        .map(|(_, c)| c)
        .collect_vec();
    lines.insert(start - 1, content);
    std::fs::write(file, lines.join("\n")).expect("cannot write file");
}

pub(crate) fn insert_line(file: PathBuf, content: &str, at: usize) {
    let file_content = std::fs::read_to_string(&file).expect("cannot read file");
    let mut lines = file_content.lines().collect_vec();
    let at = min(lines.len(), at);
    lines.insert(at, content);
    std::fs::write(file, lines.join("\n")).expect("cannot write file");
}
