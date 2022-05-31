use std::fmt::Debug;
use std::ops::Deref;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct SpanInfo {
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) content: String,
    pub(crate) filename: Option<PathBuf>,
}

#[derive(Debug)]
pub struct Spanned<T: Debug> {
    pub(crate) data: T,
    pub(crate) span: SpanInfo,
}

impl<T: Debug> Deref for Spanned<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.data
    }
}
