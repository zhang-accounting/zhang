use crate::error::IoErrorIntoZhangError;
use crate::ZhangResult;
use itertools::Itertools;
use log::debug;
use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;
use zhang_ast::{Directive, Spanned};

pub struct TransformResult {
    pub directives: Vec<Spanned<Directive>>,
    pub visited_files: Vec<PathBuf>,
}

pub trait Transformer
where
    Self: Send + Sync,
{
    fn load(&self, entry: PathBuf, endpoint: String) -> ZhangResult<TransformResult>;
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
    fn go_next(&self, directive: &Self::FileOutput) -> Option<PathBuf>;
    fn transform(&self, directives: Vec<Self::FileOutput>) -> ZhangResult<Vec<Spanned<Directive>>>;
}

impl<T> Transformer for T
where
    T: TextFileBasedTransformer,
{
    fn load(&self, entry: PathBuf, endpoint: String) -> ZhangResult<TransformResult> {
        let entry = entry.canonicalize().with_path(&entry)?;
        let main_endpoint = entry.join(endpoint);
        let main_endpoint = main_endpoint.canonicalize().with_path(&main_endpoint)?;
        let mut load_queue = VecDeque::new();
        load_queue.push_back(main_endpoint);

        let mut visited = HashSet::new();
        let mut directives = vec![];
        while let Some(load_entity) = load_queue.pop_front() {
            let path = load_entity.canonicalize().with_path(&load_entity)?;
            debug!("visited entry file: {}", path.to_str().unwrap());

            if visited.contains(&path) {
                continue;
            }
            let file_content = self.get_file_content(path.clone())?;
            let entity_directives = self.parse(&file_content, path.clone())?;

            entity_directives
                .iter()
                .filter_map(|directive| self.go_next(directive))
                .for_each(|buf| {
                    let include_path = path.parent().map(|it| it.join(&buf)).unwrap_or(buf);
                    load_queue.push_back(include_path);
                });

            visited.insert(path);
            directives.extend(entity_directives)
        }
        Ok(TransformResult {
            directives: self.transform(directives)?,
            visited_files: visited.into_iter().collect_vec(),
        })
    }
}
