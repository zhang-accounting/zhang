use zhang_core::data_source::{DataSource, LoadResult};
use zhang_core::data_type::DataType;
use zhang_core::ZhangResult;

pub struct InMemoryDataSource {
    pub data_type: Box<dyn DataType<Carrier = String> + 'static + Send + Sync>,
}

impl DataSource for InMemoryDataSource {
    fn load(&self, _entry: String, _endpoint: String) -> ZhangResult<LoadResult> {
        let directive = self.data_type.transform(_entry, None)?;
        Ok(LoadResult {
            directives: directive,
            visited_files: vec![],
        })
    }
}
