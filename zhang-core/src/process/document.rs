use std::path::PathBuf;

use zhang_ast::{Document, SpanInfo};

use crate::data_type::{document_path_in_ledger, file_in_ledger, is_beancount_endpoint};
use crate::ledger::Ledger;
use crate::process::DirectiveProcess;
use crate::store::DocumentType;
use crate::{process, ZhangResult};

impl DirectiveProcess for Document {
    fn validate(&mut self, ledger: &mut Ledger, span: &SpanInfo) -> ZhangResult<bool> {
        process::check_account_existed(self.account.name(), ledger, span)?;
        process::check_account_closed(self.account.name(), ledger, span)?;
        Ok(true)
    }

    fn process(&mut self, ledger: &mut Ledger, span: &SpanInfo) -> ZhangResult<()> {
        let mut operations = ledger.operations();

        // the path within the ledger: in a beancount ledger, it is written relative to the file of the `document`
        let mut path = self.filename.clone().to_plain_string();
        if let (true, Some(file)) = (is_beancount_endpoint(&ledger.entry.1), &span.filename) {
            path = document_path_in_ledger(&path, &file_in_ledger(&ledger.entry.0, file));
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
