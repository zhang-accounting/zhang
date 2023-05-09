use serde::Serialize;
use std::fmt::Debug;
use std::ops::Deref;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SpanInfo {
    pub start: usize,
    pub end: usize,
    pub content: String,
    pub filename: Option<PathBuf>,
}

#[derive(Debug, PartialEq, Eq)]
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
