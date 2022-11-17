use crate::core::data::{Balance, Date};
use crate::core::models::Directive;
use crate::error::{IoErrorIntoZhangError, ZhangResult};

use crate::core::ledger::Ledger;
use crate::core::models::ZhangString;
use crate::target::ZhangTarget;
use std::path::PathBuf;

pub async fn run(file: PathBuf, output: Option<PathBuf>) -> ZhangResult<()> {
    let file_parent = file.parent().unwrap().to_path_buf();
    let file_name = file.file_name().unwrap().to_str().unwrap().to_string();
    let mut ledger = Ledger::load(file_parent, file_name).await?;
    ledger = ledger.apply(convert_datetime_to_date);
    let beancount_content = ledger.to_target();
    if let Some(output_file) = output {
        std::fs::write(&output_file, beancount_content).with_path(&output_file)?;
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
        Directive::Commodity(mut directive) => Directive::Commodity(convert_to_datetime!(directive)),
        Directive::Transaction(mut directive) => Directive::Transaction(convert_to_datetime!(directive)),
        Directive::Balance(mut directive) => Directive::Balance(match &mut directive {
            Balance::BalanceCheck(check) => match check.date {
                Date::Date(_) => directive,
                Date::DateHour(date_hour) => {
                    let (date, time) = (date_hour.date(), date_hour.time());
                    check.date = Date::Date(date);
                    check.meta.insert(
                        "time".to_string(),
                        ZhangString::QuoteString(time.format("%H:%M:%S").to_string()),
                    );
                    directive
                }
                Date::Datetime(datetime) => {
                    let (date, time) = (datetime.date(), datetime.time());
                    check.date = Date::Date(date);
                    check.meta.insert(
                        "time".to_string(),
                        ZhangString::QuoteString(time.format("%H:%M:%S").to_string()),
                    );
                    directive
                }
            },
            Balance::BalancePad(pad) => match pad.date {
                Date::Date(_) => directive,
                Date::DateHour(date_hour) => {
                    let (date, time) = (date_hour.date(), date_hour.time());
                    pad.date = Date::Date(date);
                    pad.meta.insert(
                        "time".to_string(),
                        ZhangString::QuoteString(time.format("%H:%M:%S").to_string()),
                    );
                    directive
                }
                Date::Datetime(datetime) => {
                    let (date, time) = (datetime.date(), datetime.time());
                    pad.date = Date::Date(date);
                    pad.meta.insert(
                        "time".to_string(),
                        ZhangString::QuoteString(time.format("%H:%M:%S").to_string()),
                    );
                    directive
                }
            },
        }),
        Directive::Note(mut directive) => Directive::Note(convert_to_datetime!(directive)),
        Directive::Document(mut directive) => Directive::Document(convert_to_datetime!(directive)),
        Directive::Price(mut directive) => Directive::Price(convert_to_datetime!(directive)),
        Directive::Event(mut directive) => Directive::Event(convert_to_datetime!(directive)),
        Directive::Custom(mut directive) => Directive::Custom(convert_to_datetime!(directive)),
        _ => directive,
    }
}
