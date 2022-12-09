use std::fmt::Debug;
use std::ops::Deref;
use std::path::PathBuf;
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SpanInfo {
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) content: String,
    pub(crate) filename: Option<PathBuf>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Spanned<T: Debug + PartialEq> {
    pub(crate) data: T,
    pub(crate) span: SpanInfo,
}

impl<T: Debug + PartialEq> Deref for Spanned<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.data
    }
}
