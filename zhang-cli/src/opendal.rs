use std::collections::VecDeque;
use std::path::PathBuf;
use std::str::FromStr;

use crate::{FileSystem, ServerOpts};
use async_recursion::async_recursion;
use beancount::Beancount;
use chrono::Datelike;
use log::{debug, info, warn};
use minijinja::{context, Environment};
use opendal::services::{Fs, Github, Webdav};
use opendal::{ErrorKind, Operator};
use zhang_ast::{Directive, Include, SpanInfo, Spanned, ZhangString};
use zhang_core::data_source::{DataSource, LoadResult};
use zhang_core::data_type::text::parser::parse as zhang_parse;
use zhang_core::data_type::text::ZhangDataType;
use zhang_core::data_type::DataType;
use zhang_core::ledger::Ledger;
use zhang_core::utils::has_path_visited;
use zhang_core::{utils, ZhangError, ZhangResult};
pub struct OpendalDataSource {
    operator: Operator,
    data_type: Box<dyn DataType<Carrier = String> + 'static + Send + Sync>,
    is_beancount: bool,
}


async fn is_wildcard_pathbuf(pathbuf: &PathBuf) -> bool {
    pathbuf.to_string_lossy().to_string().contains("*")
}

#[derive(Debug)]
struct WildcardPathComponent {
    path: String,
    remaining: Vec<String>,
}

#[async_trait::async_trait]
impl DataSource for OpendalDataSource {
    fn export(&self, directive: Directive) -> ZhangResult<Vec<u8>> {
        Ok(self.data_type.export(Spanned::new(directive, SpanInfo::default())).into_bytes())
    }

    fn get(&self, path: String) -> ZhangResult<Vec<u8>> {
        self.operator
            .blocking()
            .read(path.as_str())
            .map(|data| data.to_vec())
            .map_err(|e| ZhangError::CustomError(format!("fail to get file content [{}] : {}", path, e)))
    }

    async fn async_load(&self, entry: String, endpoint: String) -> ZhangResult<LoadResult> {
        let entry = PathBuf::from(entry);
        let main_endpoint = entry.join(endpoint);

        let mut load_queue: VecDeque<PathBuf> = VecDeque::new();
        load_queue.push_back(main_endpoint);

        let mut visited: Vec<PathBuf> = Vec::new();
        let mut directives = vec![];
        while let Some(pathbuf) = load_queue.pop_front() {
            let striped_pathbuf = &pathbuf.strip_prefix(&entry).expect("Cannot strip entry").to_path_buf();
            if is_wildcard_pathbuf(&striped_pathbuf).await {
                // Split path into components and find wildcard level
                let mut path_components: Vec<String> = striped_pathbuf.components().map(|c| c.as_os_str().to_string_lossy().to_string()).collect();
                let first_component = path_components.remove(0);
                let wildcard_component = WildcardPathComponent {
                    path: first_component,
                    remaining: path_components,
                };
                let mut queue: VecDeque<WildcardPathComponent> = VecDeque::new();
                queue.push_back(wildcard_component);

                let mut final_file_paths: Vec<PathBuf> = vec![];
                while let Some(mut current_component) = queue.pop_front() {

                    let mut current_path = PathBuf::new();
                    current_path.push(current_component.path);

                    let next_component = current_component.remaining.remove(0);

                    let next_component_path = current_path.join(&next_component);
                    if !next_component.contains('*') {
                        // if the next component is not a wildcard, we can just add it to the current path
                        queue.push_back(WildcardPathComponent {
                            path: next_component_path.to_string_lossy().to_string(),
                            remaining: current_component.remaining,
                        });
                        continue;
                    }
                    // if the next component is a wildcard, we need to add all the files in the current path to the final file paths

                    let current_path_str = format!("{}/", current_path.to_string_lossy().to_string());
                    let files = self
                        .operator
                        .list(&current_path_str)
                        .await
                        .map_err(|e| ZhangError::CustomError(format!("fail to list files in parent directory [{}] : {}", current_path.display(), e)))?;

                    let re = regex::Regex::new(&next_component.replace('*', "[^/]+")).unwrap();

                    for entry in files {
                        let entry_name = entry.path();

                        if entry.metadata().is_dir() {
                            let striped_entry_name = entry.path().strip_prefix(&current_path_str).unwrap().strip_suffix("/").unwrap();
                            if re.is_match(&striped_entry_name) {
                                // Build full path
                                if !current_component.remaining.is_empty() {
                                    queue.push_back(dbg!(WildcardPathComponent {
                                        path: current_path.join(striped_entry_name).to_string_lossy().to_string(),
                                        remaining: current_component.remaining.clone(),
                                    }));
                                }
                            }
                        } else {
                            let striped_entry_name = entry_name.strip_prefix(&current_path_str).unwrap();
                            if re.is_match(&striped_entry_name) {
                                // Build full path
                                let is_remaining_empty = current_component.remaining.is_empty();
                                if is_remaining_empty {
                                    final_file_paths.push(current_path.join(striped_entry_name));
                                }
                            }
                        }
                    }
                }
                for file_path in final_file_paths {
                    let fullpath = if file_path.as_path().starts_with("/") {
                        file_path
                    } else {
                        entry.join(file_path)
                    };
                    load_queue.push_back(fullpath);
                }
                continue;
            } else {
                debug!("visited entry file: {:?}", striped_pathbuf.display());
            }
            if utils::has_path_visited(&visited, &pathbuf) {
                continue;
            }
            let file_content = self.get_file_content(striped_pathbuf.clone()).await?;
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
        Ok(LoadResult {
            directives: self.transform(directives)?,
            visited_files: visited,
        })
    }

    async fn async_get(&self, path: String) -> ZhangResult<Vec<u8>> {
        let path_for_read = path.to_owned();
        let result = self.operator.read(&path_for_read).await;
        match result {
            Ok(data) => Ok(data.to_vec()),
            Err(err) => {
                if err.kind() == ErrorKind::NotFound {
                    Ok(Vec::new())
                } else {
                    Err(ZhangError::CustomError(format!("Error getting file content from {}: {}", path, err)))
                }
            }
        }
    }

    async fn async_append(&self, ledger: &Ledger, directives: Vec<Directive>) -> ZhangResult<()> {
        for directive in directives {
            self.append_directive(ledger, directive, None, true).await?;
        }
        Ok(())
    }

    async fn async_save(&self, _ledger: &Ledger, path: String, content: &[u8]) -> ZhangResult<()> {
        info!("[opendal] save content path={}", &path);
        let vec = content.to_vec();

        self.operator.write(&path, vec).await.expect("cannot write");
        Ok(())
    }
}

impl OpendalDataSource {
    #[async_recursion]
    async fn append_directive(&self, ledger: &Ledger, directive: Directive, file: Option<PathBuf>, check_file_visit: bool) -> ZhangResult<()> {
        let (entry, main_file_endpoint) = &ledger.entry;

        let endpoint = if let Some(file) = file {
            file
        } else if let Some(datetime) = directive.datetime() {
            let date = datetime.date();
            let mut env = Environment::new();
            env.add_template("directive_output_path", &ledger.options.directive_output_path).map_err(|e| {
                warn!("{}", e);
                ZhangError::InvalidOptionValue
            })?;

            let tmpl = env.get_template("directive_output_path").map_err(|_e| {
                warn!("{}", _e);
                ZhangError::InvalidOptionValue
            })?;

            let save_path = tmpl
                .render(&context! {
                    type => directive.directive_type().to_string(),
                    year => date.year(),
                    month => date.month(),
                    month_str => date.format("%m").to_string(),
                    day => date.day(),
                    day_str => date.format("%d").to_string(),
                })
                .map_err(|_e| ZhangError::InvalidOptionValue)?;
            let path = PathBuf::from(save_path);
            entry.join(path)
        } else {
            entry.join(main_file_endpoint)
        };
        let striped_endpoint = endpoint.strip_prefix(entry).expect("cannot strip entry prefix");

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
            )
            .await?;
        }

        let content_buf = ledger.data_source.async_get(striped_endpoint.to_string_lossy().to_string()).await?;
        let content = String::from_utf8(content_buf)?;

        let appended_content = format!("{}\n{}\n", content, self.data_type.export(Spanned::new(directive, SpanInfo::default())));

        ledger
            .data_source
            .async_save(ledger, striped_endpoint.to_string_lossy().to_string(), appended_content.as_bytes())
            .await?;
        Ok(())
    }
    pub async fn from_env(source: FileSystem, server_opts: &mut ServerOpts) -> OpendalDataSource {
        let operator = match source {
            FileSystem::Fs => {
                let mut builder = Fs::default();
                builder.root(server_opts.path.to_string_lossy().to_string().as_str());
                // Operator::new(builder).unwrap().finish()
                Operator::new(builder).unwrap().finish()
            }
            FileSystem::WebDav => {
                let mut webdav_builder = Webdav::default();
                webdav_builder.endpoint(&std::env::var("ZHANG_WEBDAV_ENDPOINT").expect("ZHANG_WEBDAV_ENDPOINT must be set"));
                let webdav_root = std::env::var("ZHANG_WEBDAV_ROOT").expect("ZHANG_WEBDAV_ROOT must be set");
                webdav_builder.root(&webdav_root);
                webdav_builder.username(std::env::var("ZHANG_WEBDAV_USERNAME").ok().as_deref().unwrap_or_default());
                webdav_builder.password(std::env::var("ZHANG_WEBDAV_PASSWORD").ok().as_deref().unwrap_or_default());
                server_opts.path = PathBuf::from(&webdav_root);
                Operator::new(webdav_builder).unwrap().finish()
            }
            FileSystem::Github => {
                let mut builder = Github::default();
                builder.root("/");
                builder.token(&std::env::var("ZHANG_GITHUB_TOKEN").expect("ZHANG_GITHUB_TOKEN must be set"));
                builder.owner(&std::env::var("ZHANG_GITHUB_USER").expect("ZHANG_GITHUB_USER must be set"));
                builder.repo(&std::env::var("ZHANG_GITHUB_REPO").expect("ZHANG_GITHUB_REPO must be set"));

                Operator::new(builder).unwrap().finish()
            }
        };
        let is_beancount = match PathBuf::from(&server_opts.endpoint)
            .extension()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string()
            .as_str()
        {
            "bc" | "bean" | "beancount" => {
                info!("detected ledger type: beancount");
                true
            }
            "zhang" => {
                info!("detected ledger type: zhang");
                false
            }
            _ => unreachable!("not supported data format"),
        };
        let new_data_type: Box<dyn DataType<Carrier = String> + Send + Sync> = if is_beancount { Box::new(Beancount {}) } else { Box::new(ZhangDataType {}) };
        Self {
            operator,
            data_type: new_data_type,
            is_beancount,
        }
    }

    fn parse(&self, content: &str, path: PathBuf) -> ZhangResult<Vec<Spanned<Directive>>> {
        let path_string = path.to_string_lossy().to_string();
        if self.is_beancount {
            let beancount_parser = beancount::Beancount {};
            beancount_parser
                .transform(content.to_string(), Some(path_string.clone()))
                .map_err(|it| ZhangError::PestError {
                    path: path_string,
                    msg: it.to_string(),
                })
        } else {
            zhang_parse(content, path).map_err(|it| ZhangError::PestError {
                path: path_string,
                msg: it.to_string(),
            })
        }
    }
    fn go_next(&self, directive: &Spanned<Directive>) -> Option<String> {
        match &directive.data {
            Directive::Include(include) => Some(include.file.clone().to_plain_string()),
            _ => None,
        }
    }
    fn transform(&self, directives: Vec<Spanned<Directive>>) -> ZhangResult<Vec<Spanned<Directive>>> {
        Ok(directives)
    }
    async fn get_file_content(&self, path: PathBuf) -> ZhangResult<String> {
        let path = path.to_str().expect("cannot convert path to string");

        let vec = self.async_get(path.to_string()).await?;
        Ok(String::from_utf8(vec).expect("invalid utf8 content"))
    }
}
