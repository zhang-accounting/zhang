use std::fmt::Debug;
use std::ops::Deref;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Where a directive is in its file.
///
/// The parsers record both a byte range (`start..end`), which writers use to find and replace the directive's text,
/// and the line and column where it starts, which people read. A span that was not read from a file (one made up
/// for a synthetic directive, or by a plugin) has no line or column.
///
/// `line` and `column` were added after `start`, `end`, `content` and `filename`: construct a `SpanInfo` from a
/// literal with `..SpanInfo::default()` for the fields you do not set. The JSON form stays compatible: both are
/// optional when reading and left out when unknown, so a span without them serializes as before.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SpanInfo {
    /// byte offset of the directive's first character in the file
    pub start: usize,
    /// byte offset just after the directive's last character
    pub end: usize,
    /// the directive's text, `file[start..end]`
    pub content: String,
    pub filename: Option<PathBuf>,
    /// 1-based line of `start` in the file; `None` when the span was not read from a file
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>,
    /// 1-based column of `start` in its line, counting characters; `None` when the span was not read from a file
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub column: Option<usize>,
}

impl SpanInfo {
    pub fn simple(start: usize, end: usize) -> Self {
        Self {
            start,
            end,
            ..SpanInfo::default()
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
