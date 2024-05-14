use crate::ledger::Ledger;
use crate::process::DirectiveProcess;
use crate::ZhangResult;
use zhang_ast::{Plugin, SpanInfo};

impl DirectiveProcess for Plugin {
    fn validate(&mut self, _ledger: &mut Ledger, _span: &SpanInfo) -> ZhangResult<bool> {
        // todo: validate the hash for given plugin
        Ok(true)
    }

    // register plugin into ledger
    fn process(&mut self, ledger: &mut Ledger, _span: &SpanInfo) -> ZhangResult<()> {
        feature_enable!(ledger.options.features.plugins, {
            #[cfg(feature = "plugin_runtime")]
            {
                // todo: some service are not support blocking operation, so need to move the file fetch logic to pre handle logic via async handler
                let module_bytes = ledger.data_source.get(self.module.as_str().to_string())?;
                ledger.plugins.insert_plugin(self, &module_bytes)?;
            }
        });

        Ok(())
    }
}
