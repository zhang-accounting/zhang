use itertools::Itertools;
use log::debug;
use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;
use zhang_ast::{Directive, DirectiveType, Spanned};
use zhang_core::error::IoErrorIntoZhangError;
use zhang_core::transform::{TransformResult, Transformer};
use zhang_core::{ZhangError, ZhangResult};

#[allow(clippy::upper_case_acronyms)]
#[allow(clippy::type_complexity)]
pub mod p;

pub use p::parse_zhang;

#[derive(Clone, Default)]
pub struct TextTransformer {}

impl TextTransformer {
    fn load_from_file(entry: PathBuf) -> ZhangResult<Vec<Spanned<Directive>>> {
        let content = std::fs::read_to_string(&entry).with_path(&entry)?;
        parse_zhang(&content, entry).map_err(|it| ZhangError::PestError(it.to_string()))
    }
}

impl Transformer for TextTransformer {
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
            let entity_directives = TextTransformer::load_from_file(load_entity)?;
            entity_directives
                .iter()
                .filter(|it| it.directive_type() == DirectiveType::Include)
                .for_each(|it| match &it.data {
                    Directive::Include(include_directive) => {
                        let buf = PathBuf::from(include_directive.file.clone().to_plain_string());
                        let include_path = path.parent().map(|it| it.join(&buf)).unwrap_or(buf);
                        load_queue.push_back(include_path)
                    }
                    _ => {
                        unreachable!()
                    }
                });
            visited.insert(path);
            directives.extend(entity_directives)
        }
        Ok(TransformResult {
            directives,
            visited_files: visited.into_iter().collect_vec(),
        })
    }
}