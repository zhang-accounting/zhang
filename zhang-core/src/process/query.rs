use zhang_ast::{Query, SpanInfo};

use crate::ledger::Ledger;
use crate::process::DirectiveProcess;
use crate::ZhangResult;

/// A `query` directive only saves a named query. Its text is not validated at load
/// time, so the directive never emits an error; duplicate names are all kept.
impl DirectiveProcess for Query {
    fn process(&mut self, ledger: &mut Ledger, _span: &SpanInfo) -> ZhangResult<()> {
        let mut operations = ledger.operations();
        operations.insert_query(self.date.naive_date(), self.name.as_str().to_owned(), self.query_string.as_str().to_owned())
    }
}
