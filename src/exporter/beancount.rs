use crate::core::data::Date;
use crate::core::models::Directive;
use crate::error::ZhangResult;

use crate::core::ledger::Ledger;
use crate::core::models::ZhangString;
use crate::target::ZhangTarget;
use std::path::PathBuf;

pub fn run(file: PathBuf, output: Option<PathBuf>) -> ZhangResult<()> {
    let mut ledger = Ledger::load(file)?;
    ledger = ledger.apply(convert_datetime_to_date);
    let beancount_content = ledger.to_target();
    if let Some(output_file) = output {
        std::fs::write(output_file, beancount_content)?;
    } else {
        println!("{}", beancount_content);
    };
    Ok(())
}

macro_rules! convert_to_datetime {
    ($directive: expr) => {
        if let Date::Datetime(datetime) = $directive.date {
            let (date, time) = (datetime.date(), datetime.time());
            $directive.date = Date::Date(date);
            $directive.meta.insert(
                "time".to_string(),
                ZhangString::QuoteString(time.format("%H:%M:%S").to_string()),
            );
            $directive
        } else {
            $directive
        }
    };
}

fn convert_datetime_to_date(directive: Directive) -> Directive {
    match directive {
        Directive::Open(mut directive) => Directive::Open(convert_to_datetime!(directive)),
        Directive::Close(mut directive) => Directive::Close(convert_to_datetime!(directive)),
        Directive::Commodity(mut directive) => {
            Directive::Commodity(convert_to_datetime!(directive))
        }
        Directive::Transaction(mut directive) => {
            Directive::Transaction(convert_to_datetime!(directive))
        }
        Directive::Balance(mut directive) => Directive::Balance(convert_to_datetime!(directive)),
        Directive::Note(mut directive) => Directive::Note(convert_to_datetime!(directive)),
        Directive::Document(mut directive) => Directive::Document(convert_to_datetime!(directive)),
        Directive::Price(mut directive) => Directive::Price(convert_to_datetime!(directive)),
        Directive::Event(mut directive) => Directive::Event(convert_to_datetime!(directive)),
        Directive::Custom(mut directive) => Directive::Custom(convert_to_datetime!(directive)),
        _ => directive,
    }
}
