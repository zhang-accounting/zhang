use std::collections::HashMap;
use std::path::Path;

use zhang_ast::error::ErrorKind;
use zhang_ast::{Document, SpanInfo};

use crate::data_type::{document_path_in_file, document_path_in_ledger, Dialect};
use crate::ledger::Ledger;
use crate::outcome::Detail;
use crate::utils::hashmap::HashMapOfExt;

/// The path `document`, at `span`, is at within the ledger, and where else it may be. A document of an account that is
/// not active at its date is still listed: the active-accounts stage reports it
pub(crate) fn resolve(document: &Document, ledger: &mut Ledger, span: &SpanInfo) -> Detail {
    let written = document.filename.clone().to_plain_string();
    let (path, alternate) = match ledger.dialect {
        Dialect::Beancount => beancount_document_path(ledger, &written, span),
        Dialect::Zhang => (written, None),
    };
    Detail::Document { path, alternate }
}

/// The path within the ledger of the document a `document` of a beancount ledger names with `written`, at `span`, and
/// where else it may be. Beancount reads the path relative to the file of the `document`; earlier versions of zhang
/// wrote it relative to the ledger's root, in any file.
///
/// On the local disk, where looking costs a stat, a document found only relative to the root is kept, with a
/// [`DocumentPathRelativeToRoot`](ErrorKind::DocumentPathRelativeToRoot) notice telling how beancount would find it,
/// and a document found nowhere is [`DocumentNotFound`](ErrorKind::DocumentNotFound), as beancount reports it. A remote
/// source is not asked: the path relative to the file is kept with the one relative to the root as its alternate, which
/// opening the document looks at when nothing is at the path. Nothing is reported then: what is not known is neither
/// an error nor a reason to pick a path.
fn beancount_document_path(ledger: &mut Ledger, written: &str, span: &SpanInfo) -> (String, Option<String>) {
    let file = span
        .filename
        .as_ref()
        .map(|file| ledger.path_in_ledger(file).unwrap_or_else(|| file.clone()))
        .unwrap_or_default();
    let from_file = document_path_in_ledger(written, &file);
    let from_root = document_path_in_ledger(written, Path::new(""));
    // within the ledger's directory, and other than the path
    let alternate = (from_root != from_file && !from_root.starts_with("../") && !Path::new(&from_root).is_absolute()).then_some(from_root);
    let Some(root) = ledger.data_source.local_root(&ledger.entry.0) else {
        return (from_file, alternate);
    };
    let exists = |path: &str| root.join(path).is_file();
    if exists(&from_file) {
        return (from_file, None);
    }
    match alternate {
        Some(from_root) if exists(&from_root) => {
            let metas = HashMap::of2(
                "file",
                file.to_string_lossy().replace('\\', "/"),
                "written_as",
                document_path_in_file(&from_root, &file),
            );
            ledger.report(ErrorKind::DocumentPathRelativeToRoot, span, metas);
            (from_root, None)
        }
        _ => {
            ledger.report(ErrorKind::DocumentNotFound, span, HashMap::of("path", from_file.clone()));
            (from_file, None)
        }
    }
}
