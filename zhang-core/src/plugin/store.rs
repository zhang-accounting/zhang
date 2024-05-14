use std::path::PathBuf;
use std::str::FromStr;

#[cfg(feature = "plugin_runtime")]
use extism::convert::Json as WasmJson;
#[cfg(feature = "plugin_runtime")]
use extism::{Manifest, Plugin as WasmPlugin, Wasm};
use itertools::Itertools;
use log::info;
use sha256::digest;
use zhang_ast::{Directive, Plugin, Spanned};

use crate::domains::schemas::OptionDomain;
use crate::error::IoErrorIntoZhangError;
use crate::plugin::PluginType;
use crate::{ZhangError, ZhangResult};

#[derive(Default)]
pub struct PluginStore {
    pub processors: Vec<RegisteredPlugin>,
    pub mappers: Vec<RegisteredPlugin>,
    pub routers: Vec<RegisteredPlugin>,
}

impl PluginStore {
    pub fn insert_plugin(&mut self, _plugin: &Plugin) -> ZhangResult<()> {
        let plugin_name = _plugin.module.as_str().to_string();
        let plugin_hash = digest(plugin_name);
        let plugin_cache_file = PathBuf::from_str(".cache/plugins")
            .expect("Cannot create path")
            .join(format!("{}.wasm", plugin_hash));
        let content = std::fs::read(&plugin_cache_file)?;

        let wasm = Wasm::data(content);
        let manifest = Manifest::new([wasm]);

        let mut plugin = WasmPlugin::new(manifest, [], true).map_err(|e| ZhangError::CustomError(format!("Failed to create WasmPlugin: {}", e)))?;
        let name = plugin
            .call::<(), WasmJson<String>>("name", ())
            .map_err(|e| ZhangError::CustomError(format!("Failed to call 'name': {}", e)))?
            .0;
        let version = plugin
            .call::<(), WasmJson<String>>("version", ())
            .map_err(|e| ZhangError::CustomError(format!("Failed to call 'version': {}", e)))?
            .0;
        let plugin_types = plugin
            .call::<(), WasmJson<Vec<PluginType>>>("supported_type", ())
            .map_err(|e| ZhangError::CustomError(format!("Failed to call 'supported_type': {}", e)))?
            .0;

        let registered_plugin = RegisteredPlugin {
            name,
            version,
            path: plugin_cache_file,
        };
        if plugin_types.contains(&PluginType::Processor) {
            self.processors.push(registered_plugin.clone())
        }
        if plugin_types.contains(&PluginType::Mapper) {
            self.mappers.push(registered_plugin.clone())
        }
        if plugin_types.contains(&PluginType::Mapper) {
            self.routers.push(registered_plugin)
        }

        Ok(())
    }
}

#[derive(Clone)]
pub struct RegisteredPlugin {
    pub name: String,
    pub version: String,
    path: PathBuf,
}

impl RegisteredPlugin {
    pub fn load_as_plugin(&self, options: &[OptionDomain]) -> ZhangResult<WasmPlugin> {
        info!("executing the processor plugin {} {}", &self.name, &self.version);
        let options = options.iter().map(|it| (it.key.as_str(), it.value.as_str())).collect_vec();
        let module_bytes = std::fs::read(&self.path).with_path(self.path.as_path())?;
        let wasm = Wasm::data(module_bytes);
        let manifest = Manifest::new([wasm]).with_config(options.into_iter());
        let plugin = WasmPlugin::new(manifest, [], true).unwrap();

        Ok(plugin)
    }

    pub fn execute_as_processor(&self, directive: Vec<Spanned<Directive>>, options: &[OptionDomain]) -> ZhangResult<Vec<Spanned<Directive>>> {
        let mut plugin = self.load_as_plugin(options)?;
        let ret = plugin
            .call::<WasmJson<Vec<Spanned<Directive>>>, WasmJson<Vec<Spanned<Directive>>>>("processor", WasmJson(directive))
            .unwrap()
            .0;
        Ok(ret)
    }

    pub fn execute_as_mapper(&self, directive: Spanned<Directive>, options: &[OptionDomain]) -> ZhangResult<Vec<Spanned<Directive>>> {
        let mut plugin = self.load_as_plugin(options)?;
        let ret = plugin
            .call::<WasmJson<Spanned<Directive>>, WasmJson<Vec<Spanned<Directive>>>>("mapper", WasmJson(directive))
            .unwrap()
            .0;
        Ok(ret)
    }
}
