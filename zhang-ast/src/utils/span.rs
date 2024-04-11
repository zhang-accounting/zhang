use std::fmt::Debug;
use std::ops::Deref;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SpanInfo {
    pub start: usize,
    pub end: usize,
    pub content: String,
    pub filename: Option<PathBuf>,
}

impl SpanInfo {
    pub fn simple(start: usize, end: usize) -> Self {
        Self {
            start,
            end,
            content: "".to_string(),
            filename: None,
        }
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub struct Spanned<T: Debug + PartialEq> {
    pub data: T,
    pub span: SpanInfo,
}

impl<T: Debug + PartialEq> Spanned<T> {
    pub fn new(data: T, span: SpanInfo) -> Self {
        Spanned { data, span }
    }
}

impl<T: Debug + PartialEq> Deref for Spanned<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.data
    }
}
