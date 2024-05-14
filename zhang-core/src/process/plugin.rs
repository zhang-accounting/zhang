use std::path::PathBuf;
use std::str::FromStr;

use log::info;
use sha256::digest;
use zhang_ast::{Plugin, SpanInfo};

use crate::error::IoErrorIntoZhangError;
use crate::ledger::Ledger;
use crate::process::{DirectivePreProcess, DirectiveProcess};
use crate::ZhangResult;

/// save the plugin's data into cache folder
#[cfg(feature = "plugin_runtime")]
pub(crate) fn save_plugin_content_into_cache_folder(plugin_hash: String, module_bytes: Vec<u8>) -> ZhangResult<()> {
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
    fn process(&mut self, ledger: &mut Ledger, _span: &SpanInfo) -> ZhangResult<()> {
        feature_enable!(ledger.options.features.plugins, {
            #[cfg(feature = "plugin_runtime")]
            {
                ledger.plugins.insert_plugin(self)?;
            }
        });

        Ok(())
    }
}
