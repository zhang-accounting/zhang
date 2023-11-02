use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;
use std::str::FromStr;

use glob::{glob, Pattern};
use itertools::{Itertools, Either};
use log::debug;
use zhang_ast::{Directive, Spanned};

use crate::error::IoErrorIntoZhangError;
use crate::utils::to_glob_or_path;
use crate::{utils, ZhangResult};

pub struct TransformResult {
    pub directives: Vec<Spanned<Directive>>,
    pub visited_files: Vec<Either<Pattern, PathBuf>>,
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
        let mut load_queue: VecDeque<Either<Pattern, PathBuf>> = VecDeque::new();
        let a = to_glob_or_path(main_endpoint);
        load_queue.push_back(a);

        let mut visited: Vec<Either<Pattern, PathBuf>> = Vec::new();
        let mut directives = vec![];
        while let Some(load_entity) = load_queue.pop_front() {
//            debug!("visited path pattern: {}", load_entity);

            match &load_entity {
                Either::Left(pattern) => {

                    for entry in glob(pattern.as_str()).unwrap() {
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
                                        to_glob_or_path(PathBuf::from_str(&buf).unwrap())
                                    } else {
                                        path.parent()
                                    .map(|it| it.join(buf))
                                    .map(|it| to_glob_or_path(it))
                                    .unwrap()
                                    };
                                    load_queue.push_back(fullpath);
                                });
                                directives.extend(entity_directives);

                            }
                            // if the path matched but was unreadable,
                            // thereby preventing its contents from matching
                            Err(e) => println!("{:?}", e),
                        }
                    }

                },
                Either::Right(pathbuf) => {
                    debug!("visited entry file: {:?}", pathbuf.display());
                    if utils::has_path_visited(&visited, &pathbuf) {
                        continue;
                    }
                    let file_content = self.get_file_content(pathbuf.clone())?;
                    let entity_directives = self.parse(&file_content, pathbuf.clone())?;

                    entity_directives.iter().filter_map(|directive| self.go_next(directive)).for_each(|buf| {
                        let fullpath = if buf.starts_with('/') {
                            to_glob_or_path(PathBuf::from_str(&buf).unwrap())
                        } else {
                            pathbuf.parent()
                                    .map(|it| it.join(buf))
                                    .map(|it| to_glob_or_path(it))
                                    .unwrap()
                        };
                        load_queue.push_back(fullpath);
                    });
                    directives.extend(entity_directives);

    }
}
            visited.push(load_entity);

        }
        Ok(TransformResult {
            directives: self.transform(directives)?,
            visited_files: visited,
        })
    }
}
