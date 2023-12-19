use log::info;
use std::path::PathBuf;

use opendal::layers::BlockingLayer;
use opendal::services::{Fs, Webdav};
use opendal::{BlockingOperator, Operator};
use tokio::runtime::{Handle, Runtime};

use beancount::Beancount;
use zhang_ast::{Directive, Include, Spanned, ZhangString};
use zhang_core::exporter::Exporter;
use zhang_core::ledger::Ledger;
use zhang_core::text::exporter::TextExporter;
use zhang_core::text::parser::parse as zhang_parse;
use zhang_core::transform::TextFileBasedTransformer;
use zhang_core::utils::has_path_visited;
use zhang_core::{ZhangError, ZhangResult};

use crate::{DataSource, ServerOpts};
use opendal::raw::Accessor;

// static RUNTIME: Lazy<tokio::runtime::Runtime> = Lazy::new(|| tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap());

pub struct OpendalTextTransformer {
    // operator: Operator,
    operator: BlockingOperator,
    data_type: Box<dyn Exporter<Output = String> + 'static + Send + Sync>,
    is_beancount: bool,
}

impl OpendalTextTransformer {
    fn append_directive(&self, ledger: &Ledger, directive: Directive, file: Option<PathBuf>, check_file_visit: bool) -> ZhangResult<()> {
        let (entry, main_file_endpoint) = &ledger.entry;

        let endpoint = file.unwrap_or_else(|| {
            if let Some(datetime) = directive.datetime() {
                let folder = datetime.format("data/%Y/").to_string();

                // futures::executor::block_on(async {
                //     info!("[opendal] trying to create folder {}", &folder);
                //     self.operator.create_dir(&folder).await.unwrap();
                // });
                tokio::task::block_in_place(move || {
                    self.operator.create_dir(&folder).unwrap();
                });

                let path = format!("data/{}.zhang", datetime.format("%Y/%m"));
                entry.join(PathBuf::from(path))
            } else {
                entry.join(main_file_endpoint)
            }
        });
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
            )?;
        }

        let content_buf = ledger.transformer.get_content(striped_endpoint.to_string_lossy().to_string())?;
        let content = String::from_utf8(content_buf)?;

        let appended_content = format!("{}\n{}\n", content, self.data_type.export_directive(directive));

        ledger
            .transformer
            .save_content(&ledger, striped_endpoint.to_string_lossy().to_string(), appended_content.as_bytes())?;
        Ok(())
    }
    pub async fn from_env(source: DataSource, x: &ServerOpts) -> OpendalTextTransformer {
        let operator = match source {
            DataSource::Fs => {
                let mut builder = Fs::default();
                builder.root(x.path.to_string_lossy().to_string().as_str());
                // Operator::new(builder).unwrap().finish()
                Operator::new(builder).unwrap().finish().blocking()
            }
            DataSource::WebDav => {
                let mut webdav_builder = Webdav::default();
                webdav_builder.endpoint(&std::env::var("ZHANG_WEBDAV_ENDPOINT").expect("ZHANG_WEBDAV_ENDPOINT must be set"));
                webdav_builder.root(&std::env::var("ZHANG_WEBDAV_ROOT").expect("ZHANG_WEBDAV_ROOT must be set"));
                webdav_builder.username(&std::env::var("ZHANG_WEBDAV_USERNAME").ok().as_deref().unwrap_or_default());
                webdav_builder.password(&std::env::var("ZHANG_WEBDAV_PASSWORD").ok().as_deref().unwrap_or_default());
                Operator::new(webdav_builder)
                    .unwrap()
                    .layer(BlockingLayer::create().unwrap())
                    .finish()
                    .blocking()
            }
            _ => {
                todo!()
            }
        };
        let is_beancount = match PathBuf::from(&x.endpoint)
            .extension()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string()
            .as_str()
        {
            "bc" | "bean" => true,
            "zhang" => false,
            _ => unreachable!(),
        };
        let data_type: Box<dyn Exporter<Output = String> + Send + Sync> = if is_beancount { Box::new(Beancount {}) } else { Box::new(TextExporter {}) };

        Self {
            operator,
            data_type,
            is_beancount,
        }
    }
}

impl TextFileBasedTransformer for OpendalTextTransformer {
    type FileOutput = Spanned<Directive>;

    fn get_file_content(&self, path: PathBuf) -> ZhangResult<String> {
        let path = dbg!(path.to_str().expect("cannot convert path to string"));

        // let vec = tokio::task::block_in_place(move || Handle::current().block_on(async move { self.get_content(path.to_string()).expect("cannot read file") }));
        let vec = tokio::task::block_in_place(move || self.get_content(path.to_string()).expect("cannot read file"));
        Ok(String::from_utf8(vec).expect("invalid utf8 content"))
    }

    fn parse(&self, content: &str, path: PathBuf) -> ZhangResult<Vec<Self::FileOutput>> {
        if self.is_beancount {
            let beancount_parser = beancount::Beancount {};
            beancount_parser
                .parse(content, path)
                .map_err(|it| ZhangError::PestError(it.to_string()))
                .and_then(|data| beancount_parser.transform(data))
        } else {
            zhang_parse(content, path).map_err(|it| ZhangError::PestError(it.to_string()))
        }
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
        tokio::task::block_in_place(move || {
            if self.operator.is_exist(&path).expect("error") {
                let vec = self.operator.read(&path).expect("cannot read file");
                Ok(vec)
            } else {
                Ok(Vec::new())
            }
        })
        // tokio::task::block_in_place(move || {
        //     Handle::current().block_on(async move {
        //         info!("[opendal] get content path={}", &path);
        //         if self.operator.is_exist(&path).await.expect("error") {
        //             let vec = self.operator.read(&path).await.expect("cannot read file");
        //             Ok(vec)
        //         } else {
        //             Ok(Vec::new())
        //         }
        //     })
        // })

        // if self.operator.is_exist(&path).expect("error") {
        //     let vec = self.operator.read(&path).expect("cannot read file");
        //     Ok(vec)
        // } else {
        //     Ok(Vec::new())
        // }
    }

    fn append_directives(&self, ledger: &Ledger, directives: Vec<Directive>) -> ZhangResult<()> {
        for directive in directives {
            self.append_directive(ledger, directive, None, true)?;
        }
        Ok(())
    }

    fn save_content(&self, ledger: &Ledger, path: String, content: &[u8]) -> ZhangResult<()> {
        info!("[opendal] save content path={}", &path);
        let vec = content.to_vec();
        // futures::executor::block_on(async { Ok(self.operator.write(&path, vec).await.unwrap()) })
        tokio::task::block_in_place(move || Ok(self.operator.write(&path, vec).unwrap()))
        // tokio::task::block_in_place(move || Handle::current().block_on(async move { Ok(self.operator.write(&path, vec).await.unwrap()) }))
    }
}
