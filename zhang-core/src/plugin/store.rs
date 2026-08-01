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

/// meta key on a `plugin` directive granting it HTTP access to the given hosts
const ALLOWED_HOSTS_KEY: &str = "allowed_hosts";

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

        // the plugin's own declaration carries its configuration:
        //   plugin "fx-rate.wasm"
        //     allowed_hosts: "api.frankfurter.dev"
        //     base_currency: "USD"
        // `allowed_hosts` grants network access to those hosts only (no entry = no network),
        // every meta entry is handed to the plugin as its config.
        let allowed_hosts = _plugin.meta.get_all(ALLOWED_HOSTS_KEY).into_iter().map(|it| it.as_str().to_owned()).collect_vec();
        let config = _plugin
            .meta
            .clone()
            .get_flatten()
            .into_iter()
            .filter(|(key, _)| key != ALLOWED_HOSTS_KEY)
            .map(|(key, value)| (key, value.to_plain_string()))
            .collect_vec();

        let registered_plugin = RegisteredPlugin {
            name,
            version,
            path: plugin_cache_file,
            allowed_hosts,
            config,
        };
        if plugin_types.contains(&PluginType::Processor) {
            self.processors.push(registered_plugin.clone())
        }
        if plugin_types.contains(&PluginType::Mapper) {
            self.mappers.push(registered_plugin.clone())
        }
        if plugin_types.contains(&PluginType::Router) {
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
    /// hosts this plugin may reach over HTTP; empty means no network access
    allowed_hosts: Vec<String>,
    /// the plugin's own configuration, taken from its directive's meta
    config: Vec<(String, String)>,
}

impl RegisteredPlugin {
    pub fn load_as_plugin(&self, options: &[OptionDomain]) -> ZhangResult<WasmPlugin> {
        info!("loading plugin {} {}", &self.name, &self.version);
        // the ledger's options, then the plugin's own config (the latter wins on conflict)
        let config = options
            .iter()
            .map(|it| (it.key.clone(), it.value.clone()))
            .chain(self.config.iter().cloned())
            .collect_vec();
        let module_bytes = std::fs::read(&self.path).with_path(self.path.as_path())?;
        let wasm = Wasm::data(module_bytes);
        let manifest = Manifest::new([wasm])
            .with_config(config.into_iter())
            // no declared host means the plugin gets no network access at all
            .with_allowed_hosts(self.allowed_hosts.iter().cloned());
        let plugin =
            WasmPlugin::new(manifest, [], true).map_err(|e| ZhangError::CustomError(format!("cannot load plugin {}: {}", self.name, e)))?;

        Ok(plugin)
    }

    pub fn execute_as_processor(&self, directive: Vec<Spanned<Directive>>, options: &[OptionDomain]) -> ZhangResult<Vec<Spanned<Directive>>> {
        let mut plugin = self.load_as_plugin(options)?;
        let ret = plugin
            .call::<WasmJson<Vec<Spanned<Directive>>>, WasmJson<Vec<Spanned<Directive>>>>("processor", WasmJson(directive))
            .map_err(|e| ZhangError::CustomError(format!("plugin {} failed as processor: {}", self.name, e)))?
            .0;
        Ok(ret)
    }

    /// map every directive through the plugin, reusing a single instance for the whole stream
    pub fn execute_as_mapper(&self, directives: Vec<Spanned<Directive>>, options: &[OptionDomain]) -> ZhangResult<Vec<Spanned<Directive>>> {
        let mut plugin = self.load_as_plugin(options)?;
        let mut ret = vec![];
        for directive in directives {
            let mapped = plugin
                .call::<WasmJson<Spanned<Directive>>, WasmJson<Vec<Spanned<Directive>>>>("mapper", WasmJson(directive))
                .map_err(|e| ZhangError::CustomError(format!("plugin {} failed as mapper: {}", self.name, e)))?
                .0;
            ret.extend(mapped);
        }
        Ok(ret)
    }
}
