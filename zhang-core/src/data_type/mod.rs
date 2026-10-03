use std::path::{Component, Path, PathBuf};

use zhang_ast::{Directive, Spanned};

use crate::ZhangResult;

pub mod text;

/// Whether a ledger whose main file is `endpoint` is a beancount ledger: its file
/// extension is `bc`, `bean` or `beancount`. Anything else is read as zhang text.
pub fn is_beancount_endpoint(endpoint: impl AsRef<Path>) -> bool {
    matches!(endpoint.as_ref().extension().and_then(|it| it.to_str()), Some("bc" | "bean" | "beancount"))
}

/// `file`, a file of the ledger in `root`, by its path within the ledger: a local source names its files by their full
/// path, a remote one by their path within the ledger already
pub fn file_in_ledger(root: &Path, file: &Path) -> PathBuf {
    if let Ok(within) = file.strip_prefix(root) {
        return within.to_path_buf();
    }
    // the same directory named another way, as `/tmp` is `/private/tmp`
    match (root.canonicalize(), file.canonicalize()) {
        (Ok(root), Ok(file)) => file.strip_prefix(root).map(Path::to_path_buf).unwrap_or(file),
        _ => file.to_path_buf(),
    }
}

/// The path a `document` of a beancount ledger is written with in `file`, a file of the ledger named by its path within
/// it, for the document at `path` within the ledger: beancount reads the path of a `document` relative to the directory
/// of the file it is in. An absolute path is written as it is
pub fn document_path_in_file(path: &str, file: &Path) -> String {
    if Path::new(path).is_absolute() {
        return path.to_owned();
    }
    let from = normalized(file.parent().unwrap_or(Path::new("")));
    let to = normalized(Path::new(path));
    let common = from.iter().zip(&to).take_while(|(from, to)| from == to).count();
    let mut parts = vec![".."; from.len() - common];
    parts.extend(to[common..].iter().map(String::as_str));
    parts.join("/")
}

/// The path within the ledger of the document a `document` of a beancount ledger names with `path` in `file`, a file
/// of the ledger named by its path within it; see [`document_path_in_file`]. An absolute path is kept as it is
pub fn document_path_in_ledger(path: &str, file: &Path) -> String {
    if Path::new(path).is_absolute() {
        return path.to_owned();
    }
    normalized(&file.parent().unwrap_or(Path::new("")).join(path)).join("/")
}

/// the parts of the relative `path`, without `.`, and with each `..` taking out the part before it, if there is one
fn normalized(path: &Path) -> Vec<String> {
    let mut parts: Vec<String> = vec![];
    for component in path.components() {
        match component {
            Component::Normal(part) => parts.push(part.to_string_lossy().into_owned()),
            Component::ParentDir if parts.last().is_some_and(|it| it != "..") => {
                parts.pop();
            }
            Component::ParentDir => parts.push("..".to_owned()),
            Component::CurDir | Component::RootDir | Component::Prefix(_) => {}
        }
    }
    parts
}

#[cfg(test)]
mod document_path_test {
    use std::path::Path;

    use super::{document_path_in_file, document_path_in_ledger};

    #[test]
    fn a_document_is_written_relative_to_its_file_and_read_back_within_the_ledger() {
        for (in_ledger, file, written) in [
            ("attachments/a b/收据.pdf", "data/2026/10.bean", "../../attachments/a b/收据.pdf"),
            ("attachments/x.pdf", "main.bean", "attachments/x.pdf"),
            ("data/2026/x.pdf", "data/2026/10.bean", "x.pdf"),
            ("data/2025/x.pdf", "data/2026/10.bean", "../2025/x.pdf"),
            ("/srv/documents/x.pdf", "data/2026/10.bean", "/srv/documents/x.pdf"),
        ] {
            assert_eq!(document_path_in_file(in_ledger, Path::new(file)), written, "{in_ledger} in {file}");
            assert_eq!(document_path_in_ledger(written, Path::new(file)), in_ledger, "{written} in {file}");
        }
        assert_eq!(document_path_in_ledger("./a/../b.pdf", Path::new("data/x.bean")), "data/b.pdf");
        assert_eq!(document_path_in_ledger("../../../b.pdf", Path::new("data/x.bean")), "../../b.pdf");
    }
}

/// `DataType` is the protocol to describe how the raw data be transformed into standard directives and vice versa.
/// `Carrier` is the type of raw data, it can be plain text, bytes, or even sql.
pub trait DataType
where
    Self: Send + Sync,
{
    type Carrier;

    fn transform(&self, raw_data: Self::Carrier, source: Option<String>) -> ZhangResult<Vec<Spanned<Directive>>>;

    fn export(&self, directive: Spanned<Directive>) -> Self::Carrier;
}
