use std::path::PathBuf;
use std::str::FromStr;

#[cfg(feature = "plugin_runtime")]
use extism::convert::Json as WasmJson;
#[cfg(feature = "plugin_runtime")]
use extism::{Manifest, Plugin as WasmPlugin, Wasm};
use log::{info, warn};
use sha256::digest;
use zhang_ast::{Directive, Plugin, Spanned};

use crate::domains::schemas::OptionDomain;
use crate::plugin::capabilities::{PluginCapabilities, PluginDeclaration};
use crate::plugin::PluginType;
use crate::{ZhangError, ZhangResult};

#[derive(Default)]
pub struct PluginStore {
    pub processors: Vec<RegisteredPlugin>,
    pub mappers: Vec<RegisteredPlugin>,
    /// registration order with the types each plugin supports — this is the execution order
    pub ordered: Vec<(RegisteredPlugin, Vec<PluginType>)>,
}

impl PluginStore {
    pub fn insert_plugin(&mut self, _plugin: &Plugin) -> ZhangResult<()> {
        let plugin_name = _plugin.module.as_str().to_string();
        let plugin_hash = digest(plugin_name);
        let plugin_cache_file = PathBuf::from_str(".cache/plugins")
            .expect("Cannot create path")
            .join(format!("{}.wasm", plugin_hash));
        let module_bytes = std::fs::read(&plugin_cache_file)?;

        let wasm = Wasm::data(module_bytes.clone());
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
        let declared_types = plugin
            .call::<(), WasmJson<Vec<serde_json::Value>>>("supported_type", ())
            .map_err(|e| ZhangError::CustomError(format!("Failed to call 'supported_type': {}", e)))?
            .0;
        let plugin_types = known_plugin_types(&name, declared_types)?;

        let registered_plugin = RegisteredPlugin {
            name,
            version,
            module_bytes,
            declaration: PluginDeclaration::parse(_plugin),
        };
        if plugin_types.contains(&PluginType::Processor) {
            self.processors.push(registered_plugin.clone())
        }
        if plugin_types.contains(&PluginType::Mapper) {
            self.mappers.push(registered_plugin.clone())
        }
        self.ordered.push((registered_plugin, plugin_types));

        Ok(())
    }

    /// build the pipeline stages in plugin declaration order.
    /// a plugin supporting both types contributes its processor stage first, then its mapper stage.
    pub fn build_stages(&self) -> Vec<Box<dyn crate::pipeline::ProcessStage>> {
        use crate::plugin::stage::{WasmMapperStage, WasmProcessorStage};
        let mut stages: Vec<Box<dyn crate::pipeline::ProcessStage>> = vec![];
        for (plugin, types) in &self.ordered {
            if types.contains(&PluginType::Processor) {
                stages.push(Box::new(WasmProcessorStage { plugin: plugin.clone() }));
            }
            if types.contains(&PluginType::Mapper) {
                stages.push(Box::new(WasmMapperStage { plugin: plugin.clone() }));
            }
        }
        stages
    }
}

/// a plugin type older versions of zhang registered but never ran. It now falls back to
/// [`PluginType::Unknown`] like any other type this host does not know.
const RETIRED_ROUTER_TYPE: &str = "Router";

/// the plugin types this host knows among the ones a plugin declares.
/// An unknown type (from a newer zhang, say) is dropped with a warning, so the plugin still loads.
fn known_plugin_types(plugin_name: &str, declared: Vec<serde_json::Value>) -> ZhangResult<Vec<PluginType>> {
    let mut known = vec![];
    for value in declared {
        let plugin_type = serde_json::from_value::<PluginType>(value.clone())
            .map_err(|e| ZhangError::CustomError(format!("plugin {plugin_name} declares an invalid plugin type {value}: {e}")))?;
        match plugin_type {
            PluginType::Unknown if value == RETIRED_ROUTER_TYPE => {
                warn!("plugin {plugin_name} declares the plugin type {value}, which zhang retired because it never ran; ignoring it")
            }
            PluginType::Unknown => warn!("plugin {plugin_name} declares the plugin type {value}, which this version of zhang does not know; ignoring it"),
            plugin_type => known.push(plugin_type),
        }
    }
    Ok(known)
}

#[derive(Clone)]
pub struct RegisteredPlugin {
    pub name: String,
    pub version: String,
    /// the wasm module, kept in memory so executions don't re-read the cache file
    module_bytes: Vec<u8>,
    /// the capabilities and config declared by the plugin's directive
    declaration: PluginDeclaration,
}

impl RegisteredPlugin {
    /// what the plugin's directive grants it
    pub fn capabilities(&self) -> &PluginCapabilities {
        &self.declaration.capabilities
    }

    fn manifest(&self, options: &[OptionDomain]) -> Manifest {
        // the host sets no reserved `zhang.*` config yet
        let config = self.declaration.config_with(options, []);
        let wasm = Wasm::data(self.module_bytes.clone());
        Manifest::new([wasm])
            .with_config(config.into_iter())
            // no declared host means the plugin gets no network access at all
            .with_allowed_hosts(self.declaration.capabilities.allowed_hosts.iter().cloned())
    }

    pub fn load_as_plugin(&self, options: &[OptionDomain]) -> ZhangResult<WasmPlugin> {
        info!("loading plugin {} {}", self.name, self.version);
        let plugin =
            WasmPlugin::new(self.manifest(options), [], true).map_err(|e| ZhangError::CustomError(format!("cannot load plugin {}: {}", self.name, e)))?;

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

#[cfg(test)]
mod test {
    use std::collections::BTreeMap;

    use serde_json::json;
    use zhang_ast::{Meta, Plugin, ZhangString};

    use crate::domains::schemas::OptionDomain;
    use crate::plugin::capabilities::PluginDeclaration;
    use crate::plugin::store::{known_plugin_types, RegisteredPlugin};
    use crate::plugin::PluginType;

    #[test]
    fn should_hand_the_plugin_options_and_meta_without_allowed_hosts() {
        let meta: Meta = [
            ("allowed_hosts", "api.frankfurter.dev"),
            ("base_currency", "USD"),
            ("allowed_hosts", "api.example.com"),
            ("operating_currency", "EUR"),
            ("tag", "first"),
            ("tag", "second"),
            ("zhang.mine", "kept"),
        ]
        .into_iter()
        .map(|(key, value)| (key.to_owned(), ZhangString::quote(value)))
        .collect();
        let directive = Plugin {
            module: ZhangString::quote("fx-rate.wasm"),
            value: vec![ZhangString::quote("positional")],
            meta,
        };
        let options = [("operating_currency", "CNY"), ("timezone", "UTC")].map(|(key, value)| OptionDomain {
            key: key.to_owned(),
            value: value.to_owned(),
        });
        let plugin = RegisteredPlugin {
            name: "fx-rate".to_owned(),
            version: "0.1.0".to_owned(),
            module_bytes: vec![],
            declaration: PluginDeclaration::parse(&directive),
        };

        let manifest = plugin.manifest(&options);

        let expected: BTreeMap<String, String> = [
            ("base_currency", "USD"),
            ("operating_currency", "EUR"),
            ("tag", "second"),
            ("timezone", "UTC"),
            ("zhang.mine", "kept"),
        ]
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect();
        assert_eq!(manifest.config, expected);
        assert_eq!(
            manifest.allowed_hosts,
            Some(vec!["api.frankfurter.dev".to_owned(), "api.example.com".to_owned()])
        );
    }

    #[test]
    fn should_deny_network_without_allowed_hosts() {
        let directive = Plugin {
            module: ZhangString::quote("offline.wasm"),
            value: vec![],
            meta: Meta::default(),
        };
        let plugin = RegisteredPlugin {
            name: "offline".to_owned(),
            version: "0.1.0".to_owned(),
            module_bytes: vec![],
            declaration: PluginDeclaration::parse(&directive),
        };

        assert_eq!(plugin.manifest(&[]).allowed_hosts, Some(vec![]));
    }

    #[test]
    fn should_ignore_unknown_plugin_types() {
        let declared = vec![json!("Teleporter"), json!("Processor"), json!("Router"), json!("Mapper")];

        let known = known_plugin_types("future", declared).unwrap();

        // the retired Router type is unknown too
        assert_eq!(known, vec![PluginType::Processor, PluginType::Mapper]);
    }

    #[test]
    fn should_reject_a_plugin_type_that_is_not_a_name() {
        assert!(known_plugin_types("broken", vec![json!(42)]).is_err());
    }
}
