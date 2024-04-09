use crate::error::IoErrorIntoZhangError;
use crate::ZhangResult;
#[cfg(feature = "plugin")]
use extism::convert::Json as WasmJson;
#[cfg(feature = "plugin")]
use extism::{Manifest, Plugin as WasmPlugin, Wasm};
use log::info;
pub use semver::Version;
use serde::{Deserialize, Serialize};
use std::fmt::format;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use zhang_ast::{Directive, Plugin, Spanned};

#[derive(Default)]
pub struct PluginStore {
    pub(crate) processors: Vec<RegisteredPlugin>,
    pub(crate) mappers: Vec<RegisteredPlugin>,
    pub(crate) routers: Vec<RegisteredPlugin>,
}

impl PluginStore {
    #[cfg(feature = "plugin")]
    pub fn insert_plugin(&mut self, plugin: &Plugin, content: &[u8]) -> ZhangResult<()> {
        let wasm = Wasm::data(content);
        let manifest = Manifest::new([wasm]);
        let mut plugin = WasmPlugin::new(&manifest, [], true).unwrap();
        let name = plugin.call::<(), WasmJson<String>>("name", ()).unwrap().0;
        let version = plugin.call::<(), WasmJson<String>>("version", ()).unwrap().0;
        let plugin_types = plugin.call::<(), WasmJson<Vec<PluginType>>>("supported_type", ()).unwrap().0;

        // save the file into cache folder
        info!("saving the plugin into cache folder: .cache/plugins/{}-{}.wasm", name, version);
        let wasm_cache_file = PathBuf::from_str(&format!(".cache/plugins/{}-{}.wasm", name, version)).expect("invalid plugin name");
        std::fs::write(&wasm_cache_file, &content).with_path(wasm_cache_file.as_path())?;

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
        let plugin = WasmPlugin::new(&manifest, [], true).unwrap();

        return Ok(plugin);
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

/// indicate which type the plugin belongs to
/// the plugin can be multiple types
#[derive(PartialEq, Debug, Clone, Serialize, Deserialize)]
pub enum PluginType {
    /// the plugin can handle batches of directive, usually used to filter or combine directives
    Processor,

    /// the plugin have the handler map directive to another directive, usually used to modify **single** directive
    /// the mapper signature would be like
    /// ```rust,ignore
    /// fn mapper(directive: Spanned<Directive>) -> Vec<Spanned<Directive>> {
    ///     // your logic here
    /// }
    /// ```
    Mapper,

    /// the plugin can handle the customized routes, usually used for new page's API
    /// like the request of URL `/api/plugins/{PLUGIN_NAME}/my-resources` will be forwarded to plugin's router by zhang-core
    Router,
}

pub trait PluginInfo {
    fn name() -> &'static str;
    fn version() -> semver::Version;
}
