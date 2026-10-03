use std::collections::HashMap;
use std::path::{Path, PathBuf};

use zhang_ast::error::ErrorKind;
use zhang_ast::{Document, SpanInfo};

use crate::data_source::has_file;
use crate::data_type::{document_path_in_file, document_path_in_ledger, file_in_ledger, is_beancount_endpoint};
use crate::ledger::Ledger;
use crate::process::DirectiveProcess;
use crate::store::DocumentType;
use crate::utils::hashmap::HashMapOfExt;
use crate::{process, ZhangResult};

impl DirectiveProcess for Document {
    fn validate(&mut self, ledger: &mut Ledger, span: &SpanInfo) -> ZhangResult<bool> {
        process::check_account_existed(self.account.name(), ledger, span)?;
        process::check_account_closed(self.account.name(), ledger, span)?;
        Ok(true)
    }

    fn process(&mut self, ledger: &mut Ledger, span: &SpanInfo) -> ZhangResult<()> {
        let mut operations = ledger.operations();

        // the path within the ledger
        let mut path = self.filename.clone().to_plain_string();
        if is_beancount_endpoint(&ledger.entry.1) {
            path = beancount_document_path(ledger, &path, span)?;
        }

        let document_pathbuf = PathBuf::from(&path);
        operations.insert_document(
            self.date.to_timezone_datetime(&ledger.options.timezone),
            document_pathbuf.file_name().and_then(|it| it.to_str()),
            path,
            DocumentType::Account(self.account.clone()),
        )?;
        Ok(())
    }
}

/// The path within the ledger of the document a `document` of a beancount ledger names with `written`, at `span`.
/// Beancount reads it relative to the file of the `document`. Earlier versions of zhang wrote it relative to the
/// ledger's root in any file: a document found only there is kept, with a
/// [`DocumentPathRelativeToRoot`](ErrorKind::DocumentPathRelativeToRoot) notice telling how beancount would find it.
/// A document found nowhere is [`DocumentNotFound`](ErrorKind::DocumentNotFound), as beancount reports it; where the
/// source cannot tell, nothing is reported.
fn beancount_document_path(ledger: &mut Ledger, written: &str, span: &SpanInfo) -> ZhangResult<String> {
    let file = span.filename.as_ref().map(|file| file_in_ledger(&ledger.entry.0, file)).unwrap_or_default();
    let from_file = document_path_in_ledger(written, &file);
    if has_file(ledger, &from_file) != Some(false) {
        return Ok(from_file);
    }
    let from_root = document_path_in_ledger(written, Path::new(""));
    let mut operations = ledger.operations();
    if from_root != from_file && has_file(ledger, &from_root) == Some(true) {
        let metas = HashMap::of2(
            "file",
            file.to_string_lossy().replace('\\', "/"),
            "written_as",
            document_path_in_file(&from_root, &file),
        );
        operations.new_error(ErrorKind::DocumentPathRelativeToRoot, span, metas)?;
        return Ok(from_root);
    }
    operations.new_error(ErrorKind::DocumentNotFound, span, HashMap::of("path", from_file.clone()))?;
    Ok(from_file)
}
