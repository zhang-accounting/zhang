use std::path::{Component, Path};

use zhang_ast::{Directive, Spanned};

use crate::{ZhangError, ZhangResult};

pub mod text;

/// The format of a ledger: the grammar every file of it is read with, whatever the file's own extension, and the
/// rules that differ between zhang and beancount (how a `document` path is written, when a `balance` is checked, which
/// account names a write accepts, how strings are quoted). It is decided once, from the extension of the main file
/// ([`Dialect::of`]), and kept on the ledger ([`Ledger::dialect`](crate::ledger::Ledger::dialect)): every place that
/// depends on the format reads it there.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Dialect {
    /// a zhang ledger, whose main file is a `.zhang` file
    Zhang,
    /// a beancount ledger, whose main file is a `.bean`, `.beancount` or `.bc` file
    Beancount,
}

impl Dialect {
    /// The format of the ledger whose main file is `endpoint`, by its extension: `zhang` for zhang, `bean`,
    /// `beancount` or `bc` for beancount. Any other extension, or none, is [`ZhangError::UnknownLedgerFormat`]: the
    /// extension is all that tells the format, and reading a file in the wrong one would only report its every line
    pub fn of(endpoint: impl AsRef<Path>) -> ZhangResult<Dialect> {
        match endpoint.as_ref().extension().and_then(|it| it.to_str()) {
            Some("zhang") => Ok(Dialect::Zhang),
            Some("bean" | "beancount" | "bc") => Ok(Dialect::Beancount),
            _ => Err(ZhangError::UnknownLedgerFormat(endpoint.as_ref().display().to_string())),
        }
    }

    /// the name of the format, as `/api/info` gives it: `zhang` or `beancount`
    pub fn name(self) -> &'static str {
        match self {
            Dialect::Zhang => "zhang",
            Dialect::Beancount => "beancount",
        }
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

#[cfg(test)]
mod dialect_test {
    use std::sync::Arc;

    use super::text::ZhangDataType;
    use super::Dialect;
    use crate::data_source::LocalFileSystemDataSource;
    use crate::ledger::Ledger;
    use crate::ZhangError;

    /// The extension of the main file tells the format; any other extension, or none, is an error naming the file,
    /// never a guess
    #[test]
    fn the_format_is_the_extension_of_the_main_file() {
        assert_eq!(Dialect::of("main.zhang").unwrap(), Dialect::Zhang);
        assert_eq!(Dialect::of("books/2024.zhang").unwrap(), Dialect::Zhang);
        for beancount in ["main.bean", "main.beancount", "main.bc"] {
            assert_eq!(Dialect::of(beancount).unwrap(), Dialect::Beancount, "{beancount}");
        }
        for unknown in ["main.txt", "main", "main.ZHANG", "main.zhang.bak", ""] {
            assert!(
                matches!(Dialect::of(unknown), Err(ZhangError::UnknownLedgerFormat(file)) if file == unknown),
                "{unknown}"
            );
        }
        assert_eq!((Dialect::Zhang.name(), Dialect::Beancount.name()), ("zhang", "beancount"));
    }

    /// A ledger whose main file has another extension is an error naming it and the extensions that tell a format,
    /// before anything is read: it was read as zhang text here, while `zhang serve` panicked on it
    #[test]
    fn a_main_file_of_no_known_format_is_an_error_naming_it() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("main.txt"), "1970-01-01 open Assets:Cash\n").unwrap();
        let source = Arc::new(LocalFileSystemDataSource::new(ZhangDataType {}));

        let error = Ledger::load_with_data_source(dir.path().to_path_buf(), "main.txt".to_owned(), source)
            .err()
            .expect("the ledger does not load");

        assert_eq!(
            error.to_string(),
            "cannot tell the format of the ledger from its main file main.txt: name it with the extension .zhang for a zhang \
             ledger, or .bean, .beancount or .bc for a beancount one"
        );
    }
}
