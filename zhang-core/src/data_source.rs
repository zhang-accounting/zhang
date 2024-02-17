use chrono::{Datelike, Local};
use log::debug;
use std::collections::VecDeque;
use std::path::PathBuf;
use zhang_ast::{Directive, Include, SpanInfo, Spanned, ZhangString};

use crate::data_type::text::transformer::create_folder_if_not_exist;
use crate::data_type::DataType;
use crate::error::IoErrorIntoZhangError;
use crate::ledger::Ledger;
use crate::transform::TransformResult;
use crate::utils::has_path_visited;
use crate::{utils, ZhangResult};

/// `DataSource` is the protocol to describe how the `DataType` be stored and be transformed into standard directives.
/// The Data Source have two capabilities:
/// - given the endpoint, `DataSource` need to retrieve the raw data from source and feed it to associated `DataType` and get the directives from `DataType` processor.
/// - given the directive, `DataSource` need to update or insert the given directive into source, which is the place where the raw data is stored.
#[async_trait::async_trait]
pub trait DataSource
where
    Self: Send + Sync,
{
    fn get(&self, path: String) -> ZhangResult<Vec<u8>>;

    fn load(&self, entry: String, endpoint: String) -> ZhangResult<TransformResult>;

    fn save(&self, ledger: &Ledger, path: String, content: &[u8]) -> ZhangResult<()>;

    fn append(&self, ledger: &Ledger, directives: Vec<Directive>) -> ZhangResult<()>;

    async fn async_load(&self, entry: String, endpoint: String) -> ZhangResult<TransformResult> {
        self.load(entry, endpoint)
    }

    async fn async_get(&self, path: String) -> ZhangResult<Vec<u8>> {
        self.get(path)
    }
    async fn async_append(&self, ledger: &Ledger, directives: Vec<Directive>) -> ZhangResult<()> {
        self.append(ledger, directives)
    }

    async fn async_save(&self, ledger: &Ledger, path: String, content: &[u8]) -> ZhangResult<()> {
        self.save(ledger, path, content)
    }
}

pub struct LocalFileSystemDataSource {
    data_type: Box<dyn DataType<Carrier = String> + 'static + Send + Sync>,
}

impl LocalFileSystemDataSource {
    pub fn new<DT: DataType<Carrier = String> + Send + Sync + 'static>(data_type: DT) -> Self {
        LocalFileSystemDataSource {
            data_type: Box::new(data_type),
        }
    }
    fn go_next(&self, directive: &Spanned<Directive>) -> Option<String> {
        match &directive.data {
            Directive::Include(include) => Some(include.file.clone().to_plain_string()),
            _ => None,
        }
    }

    fn append_directive(&self, ledger: &Ledger, directive: Directive, file: Option<PathBuf>, check_file_visit: bool) -> ZhangResult<()> {
        let (entry, main_file_endpoint) = &ledger.entry;

        let endpoint = file.unwrap_or_else(|| {
            if let Some(datetime) = directive.datetime() {
                entry.join(PathBuf::from(format!("data/{}/{}.zhang", datetime.year(), datetime.month())))
            } else {
                entry.join(main_file_endpoint)
            }
        });

        create_folder_if_not_exist(&endpoint);

        if !has_path_visited(&ledger.visited_files, &endpoint) && check_file_visit {
            let path = match endpoint.strip_prefix(entry) {
                Ok(relative_path) => relative_path.to_str().unwrap(),
                Err(_) => endpoint.to_str().unwrap(),
            };
            self.append_directive(
                ledger,
                Directive::Include(Include {
                    file: ZhangString::QuoteString(path.to_string()),
                }),
                None,
                false,
            )?;
        }

        let content_buf = ledger.transformer.get(endpoint.to_string_lossy().to_string())?;
        let content = String::from_utf8(content_buf)?;

        let appended_content = format!("{}\n{}\n", content, self.data_type.export(Spanned::new(directive, SpanInfo::default())));

        ledger
            .transformer
            .save(ledger, endpoint.to_string_lossy().to_string(), appended_content.as_bytes())?;
        Ok(())
    }
}

#[async_trait::async_trait]
impl DataSource for LocalFileSystemDataSource {
    fn get(&self, path: String) -> ZhangResult<Vec<u8>> {
        Ok(std::fs::read(PathBuf::from(path))?)
    }

    fn load(&self, entry: String, endpoint: String) -> ZhangResult<TransformResult> {
        let entry = PathBuf::from(entry);
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
            let file_content = self.get(striped_pathbuf.to_string_lossy().to_string())?;
            //todo: remove utf8 string unwrap
            let entity_directives = self
                .data_type
                .transform(String::from_utf8(file_content).unwrap(), Some(striped_pathbuf.to_string_lossy().to_string()))?;

            entity_directives.iter().filter_map(|directive| self.go_next(directive)).for_each(|buf| {
                let fullpath = if buf.starts_with('/') {
                    PathBuf::from(&buf)
                } else {
                    pathbuf.parent().map(|it| it.join(buf)).unwrap()
                };
                load_queue.push_back(fullpath);
            });
            directives.extend(entity_directives);
            visited.push(pathbuf);
        }
        Ok(TransformResult {
            directives,
            visited_files: visited,
        })
    }

    fn save(&self, ledger: &Ledger, path: String, content: &[u8]) -> ZhangResult<()> {
        std::fs::write(&path, content).with_path(PathBuf::from(path).as_path())
    }

    fn append(&self, ledger: &Ledger, directives: Vec<Directive>) -> ZhangResult<()> {
        for directive in directives {
            self.append_directive(ledger, directive, None, true)?;
        }
        Ok(())
    }
}
