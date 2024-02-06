use zhang_ast::Directive;

use crate::data_type::DataType;
use crate::ledger::Ledger;
use crate::transform::TransformResult;
use crate::ZhangResult;

/// `DataSource` is the protocol to describe how the `DataType` be stored and be transformed into standard directives.
/// The Data Source have two capabilities:
/// - given the endpoint, `DataSource` need to retrieve the raw data from source and feed it to associated `DataType` and get the directives from `DataType` processor.
/// - given the directive, `DataSource` need to update or insert the given directive into source, which is the place where the raw data is stored.
pub trait DataSource {
    type DataType: DataType;

    fn get(&self, path: String) -> ZhangResult<Vec<u8>>;

    fn load(&self, entry: String, path: String) -> ZhangResult<TransformResult>;

    fn save(&self, ledger: &Ledger, path: String, content: &[u8]) -> ZhangResult<()>;

    fn append(&self, ledger: &Ledger, directives: Vec<Directive>) -> ZhangResult<()>;

    async fn async_load(&self, entry: String, endpoint: String) -> ZhangResult<TransformResult> {
        self.load(entry, endpoint)
    }

    async fn async_get(&self, path: String) -> ZhangResult<Vec<u8>> {
        self.get(path)
    }
    async fn async_append(&self, ledger: &Ledger, directives: Vec<Directive>) -> ZhangResult<()> {
        self.append(ledger, directives)
    }

    async fn async_save(&self, ledger: &Ledger, path: String, content: &[u8]) -> ZhangResult<()> {
        self.save(ledger, path, content)
    }
}
