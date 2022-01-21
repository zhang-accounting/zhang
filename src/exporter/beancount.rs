use crate::core::data::Date;
use crate::error::{AvaroError, AvaroResult};
use crate::{core, core::models::Directive};
use itertools::Itertools;
use std::fs::DirEntry;
use std::path::PathBuf;
use crate::core::ledger::Ledger;
use crate::target::AvaroTarget;

pub fn run(file: PathBuf, output: Option<PathBuf>) -> AvaroResult<()> {
    let mut ledger = Ledger::load(file)?;
    ledger = ledger.apply(convert_datetime_to_date);
    println!("{}", ledger.to_target());
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
            $directive
        }else {
            $directive
        }
    };
}

fn convert_datetime_to_date(directive: Directive) -> Directive {
    match directive {
        Directive::Open(mut directive) => {
            Directive::Open(convert_to_datetime!(directive))
        }
        Directive::Close(mut directive) => {
            Directive::Close(convert_to_datetime!(directive))
        }
        Directive::Commodity(mut directive) => {
            Directive::Commodity(convert_to_datetime!(directive))
        }
        Directive::Transaction(mut directive) => {
            Directive::Transaction(convert_to_datetime!(directive))
        }
        Directive::Balance(mut directive) => {
            Directive::Balance(convert_to_datetime!(directive))
        }
        Directive::Pad(mut directive) => {
            Directive::Pad(convert_to_datetime!(directive))
        }
        Directive::Note(mut directive) => {
            Directive::Note(convert_to_datetime!(directive))
        }
        Directive::Document(mut directive) => {
            Directive::Document(convert_to_datetime!(directive))
        }
        Directive::Price(mut directive) => {
            Directive::Price(convert_to_datetime!(directive))
        }
        Directive::Event(mut directive) => {
            Directive::Event(convert_to_datetime!(directive))
        }
        Directive::Custom(mut directive) => {
            Directive::Custom(convert_to_datetime!(directive))
        }
        _ => directive
    }
}
