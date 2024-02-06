use chrono::Datelike;
use std::path::PathBuf;

use zhang_ast::{Directive, Include, Spanned, ZhangString};

use crate::data_type::text::exporter::TextExporter;
use crate::data_type::text::parser::parse;
use crate::error::IoErrorIntoZhangError;
use crate::exporter::Exporter;
use crate::ledger::Ledger;
use crate::transform::TextFileBasedTransformer;
use crate::utils::has_path_visited;
use crate::{ZhangError, ZhangResult};

pub(crate) fn create_folder_if_not_exist(filename: &std::path::Path) {
    std::fs::create_dir_all(filename.parent().unwrap()).expect("cannot create folder recursive");
}

#[derive(Clone, Default)]
pub struct TextTransformer {
    exporter: TextExporter,
}

impl TextTransformer {
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

        let content_buf = ledger.transformer.get_content(endpoint.to_string_lossy().to_string())?;
        let content = String::from_utf8(content_buf)?;

        let appended_content = format!("{}\n{}\n", content, self.exporter.export_directive(directive));

        ledger
            .transformer
            .save_content(ledger, endpoint.to_string_lossy().to_string(), appended_content.as_bytes())?;
        Ok(())
    }
}

impl TextFileBasedTransformer for TextTransformer {
    type FileOutput = Spanned<Directive>;

    fn parse(&self, content: &str, path: PathBuf) -> ZhangResult<Vec<Self::FileOutput>> {
        parse(content, path).map_err(|it| ZhangError::PestError(it.to_string()))
    }

    fn go_next(&self, directive: &Self::FileOutput) -> Option<String> {
        match &directive.data {
            Directive::Include(include) => Some(include.file.clone().to_plain_string()),
            _ => None,
        }
    }
    fn transform(&self, directives: Vec<Self::FileOutput>) -> ZhangResult<Vec<Spanned<Directive>>> {
        Ok(directives)
    }

    fn get_content(&self, path: String) -> ZhangResult<Vec<u8>> {
        Ok(std::fs::read(PathBuf::from(path))?)
    }

    fn append_directives(&self, ledger: &Ledger, directives: Vec<Directive>) -> ZhangResult<()> {
        for directive in directives {
            self.append_directive(ledger, directive, None, true)?;
        }
        Ok(())
    }

    fn save_content(&self, _: &Ledger, path: String, content: &[u8]) -> ZhangResult<()> {
        std::fs::write(&path, content).with_path(PathBuf::from(path).as_path())
    }
}
