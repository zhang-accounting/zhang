use std::path::PathBuf;

pub mod bigdecimal_ext;
pub mod calculable;
pub mod date_range;
pub mod hashmap;
pub mod id;
pub mod logging;
pub mod price_grip;
pub mod string_;

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
