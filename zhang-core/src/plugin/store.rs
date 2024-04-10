use std::path::PathBuf;
use std::str::FromStr;

#[cfg(feature = "plugin")]
use extism::convert::Json as WasmJson;
#[cfg(feature = "plugin")]
use extism::{Manifest, Plugin as WasmPlugin, Wasm};
use log::info;
use zhang_ast::{Directive, Plugin, Spanned};

use crate::error::IoErrorIntoZhangError;
use crate::plugin::PluginType;
use crate::ZhangResult;

#[derive(Default)]
pub struct PluginStore {
    pub(crate) processors: Vec<RegisteredPlugin>,
    pub(crate) mappers: Vec<RegisteredPlugin>,
    pub(crate) routers: Vec<RegisteredPlugin>,
}

impl PluginStore {
    #[cfg(feature = "plugin")]
    pub fn insert_plugin(&mut self, _plugin: &Plugin, content: &[u8]) -> ZhangResult<()> {
        let wasm = Wasm::data(content);
        let manifest = Manifest::new([wasm]);
        let mut plugin = WasmPlugin::new(manifest, [], true).unwrap();
        let name = plugin.call::<(), WasmJson<String>>("name", ()).unwrap().0;
        let version = plugin.call::<(), WasmJson<String>>("version", ()).unwrap().0;
        let plugin_types = plugin.call::<(), WasmJson<Vec<PluginType>>>("supported_type", ()).unwrap().0;

        let plugin_cache_folder = PathBuf::from_str(".cache/plugins").expect("Cannot create path");

        // create plugin folder if not exist
        std::fs::create_dir_all(&plugin_cache_folder).with_path(plugin_cache_folder.as_path())?;

        // save the file into cache folder
        info!("saving the plugin into cache folder: .cache/plugins/{}-{}.wasm", name, version);
        let wasm_cache_file = plugin_cache_folder.join(format!("{}-{}.wasm", name, version));
        std::fs::write(&wasm_cache_file, content).with_path(wasm_cache_file.as_path())?;

        let registered_plugin = RegisteredPlugin {
            name,
            version,
            path: wasm_cache_file,
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
    name: String,
    version: String,
    path: PathBuf,
}

impl RegisteredPlugin {
    #[cfg(feature = "plugin")]
    pub fn load_as_plugin(&self) -> ZhangResult<WasmPlugin> {
        info!("executing the processor plugin {} {}", &self.name, &self.version);
        let module_bytes = std::fs::read(&self.path).with_path(self.path.as_path())?;
        let wasm = Wasm::data(module_bytes);
        let manifest = Manifest::new([wasm]);
        let plugin = WasmPlugin::new(manifest, [], true).unwrap();

        Ok(plugin)
    }

    #[cfg(feature = "plugin")]
    pub fn execute_as_processor(&self, directive: Vec<Spanned<Directive>>) -> ZhangResult<Vec<Spanned<Directive>>> {
        let mut plugin = self.load_as_plugin()?;
        let ret = plugin
            .call::<WasmJson<Vec<Spanned<Directive>>>, WasmJson<Vec<Spanned<Directive>>>>("processor", WasmJson(directive))
            .unwrap()
            .0;
        Ok(ret)
    }

    #[cfg(feature = "plugin")]
    pub fn execute_as_mapper(&self, directive: Spanned<Directive>) -> ZhangResult<Vec<Spanned<Directive>>> {
        let mut plugin = self.load_as_plugin()?;
        let ret = plugin
            .call::<WasmJson<Spanned<Directive>>, WasmJson<Vec<Spanned<Directive>>>>("mapper", WasmJson(directive))
            .unwrap()
            .0;
        Ok(ret)
    }
}
