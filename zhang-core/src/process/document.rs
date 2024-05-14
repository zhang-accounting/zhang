use crate::ledger::Ledger;
use crate::process::DirectiveProcess;
use crate::store::DocumentType;
use crate::{process, ZhangResult};
use std::path::PathBuf;
use zhang_ast::{Document, SpanInfo};

impl DirectiveProcess for Document {
    fn validate(&mut self, ledger: &mut Ledger, span: &SpanInfo) -> ZhangResult<bool> {
        process::check_account_existed(self.account.name(), ledger, span)?;
        process::check_account_closed(self.account.name(), ledger, span)?;
        Ok(true)
    }

    fn process(&mut self, ledger: &mut Ledger, _span: &SpanInfo) -> ZhangResult<()> {
        let mut operations = ledger.operations();

        let path = self.filename.clone().to_plain_string();

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
