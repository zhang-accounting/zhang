use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;

use glob::{glob, Pattern};
use itertools::Itertools;
use log::debug;
use zhang_ast::{Directive, Spanned};

use crate::error::IoErrorIntoZhangError;
use crate::{utils, ZhangError, ZhangResult};

pub struct TransformResult {
    pub directives: Vec<Spanned<Directive>>,
    pub visited_files: Vec<Pattern>,
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
    fn go_next(&self, directive: &Self::FileOutput) -> Option<String>;
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
        let mut load_queue: VecDeque<Pattern> = VecDeque::new();
        load_queue.push_back(Pattern::new(main_endpoint.as_path().to_str().unwrap()).unwrap());

        let mut visited: HashSet<Pattern> = HashSet::new();
        let mut directives = vec![];
        while let Some(load_entity) = load_queue.pop_front() {
            debug!("visited path pattern: {}", load_entity);
            for entry in glob(load_entity.as_str()).unwrap() {
                match entry {
                    Ok(path) => {
                        debug!("visited entry file: {:?}", path.display());
                        if utils::has_path_visited(&visited, &path) {
                            continue;
                        }
                        let file_content = self.get_file_content(path.clone())?;
                        let entity_directives = self.parse(&file_content, path.clone())?;

                        entity_directives.iter().filter_map(|directive| self.go_next(directive)).for_each(|buf| {
                            let fullpath = if buf.starts_with('/') {
                                buf
                            } else {
                                path.parent()
                                    .map(|it| it.join(buf))
                                    .map(|it| it.as_path().to_str().unwrap().to_owned())
                                    .unwrap()
                            };
                            load_queue.push_back(Pattern::new(&fullpath).unwrap());
                        });
                        directives.extend(entity_directives)
                    }
                    // if the path matched but was unreadable,
                    // thereby preventing its contents from matching
                    Err(e) => println!("{:?}", e),
                }
            }
            visited.insert(load_entity);
        }
        Ok(TransformResult {
            directives: self.transform(directives)?,
            visited_files: visited.into_iter().collect_vec(),
        })
    }
}

