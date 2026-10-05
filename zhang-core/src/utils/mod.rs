use std::path::PathBuf;

use chrono::NaiveTime;

pub mod date_range;
pub mod hashmap;
pub mod id;
pub mod string_;

/// The UTF-8 byte order mark some Windows editors write at the start of a file. It is no part of the text: the
/// parsers skip it, and a file that has one keeps it when it is written back ([`crate::data_source::FileText`])
pub const BOM: &str = "\u{feff}";

/// The time of day in the `time` metadata of a beancount directive: `H:M:S`, spaces around it trimmed
pub fn read_time(text: &str) -> Option<NaiveTime> {
    let parts = text.trim().split(':').map(|it| it.parse::<u32>().ok()).collect::<Option<Vec<_>>>()?;
    match parts[..] {
        [hour, minute, second] => NaiveTime::from_hms_opt(hour, minute, second),
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
