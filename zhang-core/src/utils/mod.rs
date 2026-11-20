use std::path::PathBuf;

use chrono::NaiveTime;

pub mod date_range;
pub mod hashmap;
pub mod id;
pub mod string_;

/// The UTF-8 byte order mark some Windows editors write at the start of a file. It is no part of the text: the
/// parsers skip it, and a file that has one keeps it when it is written back ([`crate::data_source::FileText`])
pub const BOM: &str = "\u{feff}";

/// The time of day in the `time` metadata of a beancount directive as earlier versions read it: `H:M:S`, spaces around
/// it trimmed. The balance check compares with it what those versions made of a balance's `time`
pub fn read_time(text: &str) -> Option<NaiveTime> {
    let parts = text.trim().split(':').map(|it| it.parse::<u32>().ok()).collect::<Option<Vec<_>>>()?;
    match parts[..] {
        [hour, minute, second] => NaiveTime::from_hms_opt(hour, minute, second),
        _ => None,
    }
}

/// The time of day in the `time` metadata of a directive of a beancount file, which becomes its time: `H:M:S` or
/// `H:M` of digits only (no sign), spaces around it trimmed; `None` for any other text, which stays plain metadata
pub fn read_time_of_day(text: &str) -> Option<NaiveTime> {
    let part = |part: &str| {
        (!part.is_empty() && part.bytes().all(|it| it.is_ascii_digit()))
            .then(|| part.parse::<u32>().ok())
            .flatten()
    };
    let parts = text.trim().split(':').map(part).collect::<Option<Vec<_>>>()?;
    match parts[..] {
        [hour, minute, second] => NaiveTime::from_hms_opt(hour, minute, second),
        [hour, minute] => NaiveTime::from_hms_opt(hour, minute, 0),
        _ => None,
    }
}

pub fn has_path_visited<'a>(visited: impl IntoIterator<Item = &'a PathBuf>, path: &PathBuf) -> bool {
    visited.into_iter().any(|pathbuf| pathbuf.eq(path))
}

macro_rules! feature_enable {
    ($feature_name: expr, $feature_process:expr) => {
        if $feature_name {
            $feature_process
        }
    };
    ($feature_name: expr, $feature_process:expr, $not_feature_process: expr) => {
        if $feature_name {
            $feature_process
        } else {
            $not_feature_process
        }
    };
}

/// A decimal written out in plain notation, which both zhang and beancount read ([`zhang_shared::decimal::plain_decimal`])
pub use zhang_shared::decimal::plain_decimal;
