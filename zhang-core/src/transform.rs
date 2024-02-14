use std::collections::VecDeque;
use std::path::PathBuf;
use std::str::FromStr;

use log::debug;

use zhang_ast::{Directive, Spanned};

use crate::error::IoErrorIntoZhangError;
use crate::ledger::Ledger;
use crate::{utils, ZhangResult};

pub struct TransformResult {
    pub directives: Vec<Spanned<Directive>>,
    pub visited_files: Vec<PathBuf>,
}

#[async_trait::async_trait]
pub trait Transformer
where
    Self: Send + Sync,
{
    fn load(&self, entry: PathBuf, endpoint: String) -> ZhangResult<TransformResult>;

    fn get_content(&self, path: String) -> ZhangResult<Vec<u8>>;
    fn append_directives(&self, ledger: &Ledger, directives: Vec<Directive>) -> ZhangResult<()>;

    fn save_content(&self, ledger: &Ledger, path: String, content: &[u8]) -> ZhangResult<()>;

    async fn async_load(&self, entry: PathBuf, endpoint: String) -> ZhangResult<TransformResult> {
        self.load(entry, endpoint)
    }

    async fn async_get_content(&self, path: String) -> ZhangResult<Vec<u8>> {
        self.get_content(path)
    }
    async fn async_append_directives(&self, ledger: &Ledger, directives: Vec<Directive>) -> ZhangResult<()> {
        self.append_directives(ledger, directives)
    }

    async fn async_save_content(&self, ledger: &Ledger, path: String, content: &[u8]) -> ZhangResult<()> {
        self.save_content(ledger, path, content)
    }
}

pub trait TextFileBasedTransformer
where
    Self: Send + Sync,
{
    type FileOutput;
    fn get_file_content(&self, path: PathBuf) -> ZhangResult<String> {
        let content = std::fs::read_to_string(&path).with_path(&path)?;
        Ok(content)
    }
    fn parse(&self, content: &str, path: PathBuf) -> ZhangResult<Vec<Self::FileOutput>>;
    fn go_next(&self, directive: &Self::FileOutput) -> Option<String>;
    fn transform_old(&self, directives: Vec<Self::FileOutput>) -> ZhangResult<Vec<Spanned<Directive>>>;

    fn get_content(&self, path: String) -> ZhangResult<Vec<u8>>;
    fn append_directives(&self, ledger: &Ledger, directives: Vec<Directive>) -> ZhangResult<()>;
    fn save_content(&self, ledger: &Ledger, path: String, content: &[u8]) -> ZhangResult<()>;
}

impl<T> Transformer for T
where
    T: TextFileBasedTransformer,
{
    fn load(&self, entry: PathBuf, endpoint: String) -> ZhangResult<TransformResult> {
        let entry = entry.canonicalize().with_path(&entry)?;
        let main_endpoint = entry.join(endpoint);
        let main_endpoint = main_endpoint.canonicalize().with_path(&main_endpoint)?;

        let mut load_queue: VecDeque<PathBuf> = VecDeque::new();
        load_queue.push_back(main_endpoint);

        let mut visited: Vec<PathBuf> = Vec::new();
        let mut directives = vec![];
        while let Some(pathbuf) = load_queue.pop_front() {
            let striped_pathbuf = &pathbuf.strip_prefix(&entry).expect("Cannot strip entry").to_path_buf();
            debug!("visited entry file: {:?}", striped_pathbuf.display());

            if utils::has_path_visited(&visited, &pathbuf) {
                continue;
            }
            let file_content = self.get_file_content(striped_pathbuf.clone())?;
            let entity_directives = self.parse(&file_content, striped_pathbuf.clone())?;

            entity_directives.iter().filter_map(|directive| self.go_next(directive)).for_each(|buf| {
                let fullpath = if buf.starts_with('/') {
                    PathBuf::from_str(&buf).unwrap()
                } else {
                    pathbuf.parent().map(|it| it.join(buf)).unwrap()
                };
                load_queue.push_back(fullpath);
            });
            directives.extend(entity_directives);
            visited.push(pathbuf);
        }
        Ok(TransformResult {
            directives: self.transform_old(directives)?,
            visited_files: visited,
        })
    }

    fn get_content(&self, path: String) -> ZhangResult<Vec<u8>> {
        self.get_content(path)
    }

    fn append_directives(&self, ledger: &Ledger, directives: Vec<Directive>) -> ZhangResult<()> {
        self.append_directives(ledger, directives)
    }

    fn save_content(&self, ledger: &Ledger, path: String, content: &[u8]) -> ZhangResult<()> {
        self.save_content(ledger, path, content)
    }
}
