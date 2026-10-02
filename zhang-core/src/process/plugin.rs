use zhang_ast::{Plugin, SpanInfo};

use crate::ledger::Ledger;
use crate::process::{DirectivePreProcess, DirectiveProcess};
use crate::ZhangResult;

/// save the plugin's data into cache folder
#[cfg(feature = "plugin_runtime")]
pub(crate) fn save_plugin_content_into_cache_folder(plugin_hash: String, module_bytes: Vec<u8>) -> ZhangResult<()> {
    use std::path::PathBuf;
    use std::str::FromStr;

    use log::info;

    use crate::error::IoErrorIntoZhangError;

    let plugin_cache_folder = PathBuf::from_str(".cache/plugins").expect("Cannot create path");

    // create plugin folder if not exist
    std::fs::create_dir_all(&plugin_cache_folder).with_path(plugin_cache_folder.as_path())?;

    // save the file into cache folder
    info!("saving the plugin into cache folder: .cache/plugins/{}.wasm", plugin_hash);
    let wasm_cache_file = plugin_cache_folder.join(format!("{}.wasm", plugin_hash));
    std::fs::write(&wasm_cache_file, module_bytes).with_path(wasm_cache_file.as_path())?;
    Ok(())
}

/// mainly for fetch the plugin data from remote and save it into local cache folder
#[async_trait::async_trait]
impl DirectivePreProcess for Plugin {
    fn pre_process(&self, ledger: &mut Ledger) -> ZhangResult<()> {
        feature_enable!(ledger.options.features.plugins, {
            #[cfg(feature = "plugin_runtime")]
            {
                use sha256::digest;

                let plugin_name = self.module.as_str().to_string();
                let plugin_hash = digest(&plugin_name);
                let module_bytes = ledger.data_source.get(plugin_name)?;

                save_plugin_content_into_cache_folder(plugin_hash, module_bytes)?;
            }
        });
        Ok(())
    }

    async fn async_pre_process(&self, ledger: &mut Ledger) -> ZhangResult<()> {
        feature_enable!(ledger.options.features.plugins, {
            #[cfg(feature = "plugin_runtime")]
            {
                use sha256::digest;

                let plugin_name = self.module.as_str().to_string();
                let plugin_hash = digest(&plugin_name);
                let module_bytes = ledger.data_source.async_get(plugin_name).await?;

                save_plugin_content_into_cache_folder(plugin_hash, module_bytes)?;
            }
        });
        Ok(())
    }
}

impl DirectiveProcess for Plugin {
    fn validate(&mut self, _ledger: &mut Ledger, _span: &SpanInfo) -> ZhangResult<bool> {
        // todo: validate the hash for given plugin
        Ok(true)
    }

    // register plugin into ledger
    #[cfg_attr(not(feature = "plugin_runtime"), allow(unused_variables))]
    fn process(&mut self, ledger: &mut Ledger, span: &SpanInfo) -> ZhangResult<()> {
        feature_enable!(ledger.options.features.plugins, {
            #[cfg(feature = "plugin_runtime")]
            {
                let declaration = crate::plugin::capabilities::PluginDeclaration::parse(self);
                // a meta value the host cannot use is reported on the directive, and the plugin runs with the default
                let mut operations = ledger.operations();
                for error in &declaration.errors {
                    operations.new_error(error.kind.clone(), span, error.metas.clone())?;
                }
                ledger.plugins.insert_plugin(self, declaration)?;
                // a rebuilt local module makes the ledger stale
                if let Some(input) = crate::inputs::ExtraInput::plugin_module(&ledger.entry.0, self.module.as_str()) {
                    ledger.extra_inputs.insert(input);
                }
            }
        });

        Ok(())
    }
}
