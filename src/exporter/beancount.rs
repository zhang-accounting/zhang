use crate::core::data::Date;
use crate::error::{AvaroError, AvaroResult};
use crate::{core, Directive};
use itertools::Itertools;
use std::fs::DirEntry;
use std::path::PathBuf;

pub fn run(file: PathBuf, output: Option<PathBuf>) -> AvaroResult<()> {
    let avaro_content = std::fs::read_to_string(file)?;
    let vec = core::load(&avaro_content)?
        .into_iter()
        .map(|mut it| {
            convert_datetime_to_date(&mut it);
            it
        })
        .collect_vec();
    dbg!(vec);
    Ok(())
}

macro_rules! convert_to_datetime {
    ($directive: expr) => {
        if let Date::Datetime(datetime) = $directive.date {
            let (date, time) = (datetime.date(), datetime.time());
            $directive.date = Date::Date(date);
            $directive
                .meta
                .insert("time".to_string(), time.format("%H:%M:%S").to_string());
        }
    };
}

fn convert_datetime_to_date(directive: &mut Directive) {
    match directive {
        Directive::Open(directive) => {
            convert_to_datetime!(directive)
        }
        Directive::Close(directive) => {
            convert_to_datetime!(directive)
        }
        Directive::Commodity(directive) => {
            convert_to_datetime!(directive)
        }
        Directive::Transaction(directive) => {
            convert_to_datetime!(directive)
        }
        Directive::Balance(directive) => {
            convert_to_datetime!(directive)
        }
        Directive::Pad(directive) => {
            convert_to_datetime!(directive)
        }
        Directive::Note(directive) => {
            convert_to_datetime!(directive)
        }
        Directive::Document(directive) => {
            convert_to_datetime!(directive)
        }
        Directive::Price(directive) => {
            convert_to_datetime!(directive)
        }
        Directive::Event(directive) => {
            convert_to_datetime!(directive)
        }
        Directive::Custom(directive) => {
            convert_to_datetime!(directive)
        }
        Directive::Option { .. } => {}
        Directive::Plugin { .. } => {}
        Directive::Include { .. } => {}
        Directive::Comment { .. } => {}
    }
}
