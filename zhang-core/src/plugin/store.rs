use std::path::PathBuf;
use std::str::FromStr;
use std::time::Duration;

#[cfg(feature = "plugin_runtime")]
use extism::convert::Json as WasmJson;
#[cfg(feature = "plugin_runtime")]
use extism::{Manifest, Plugin as WasmPlugin, Wasm};
use log::{info, warn};
use sha256::digest;
use zhang_ast::{Directive, Plugin, SpanInfo, Spanned};

use crate::domains::schemas::OptionDomain;
use crate::pipeline::StageContext;
use crate::plugin::capabilities::{PluginCapabilities, PluginDeclaration};
use crate::plugin::host::PluginHost;
use crate::plugin::router::unavailable_host_functions;
use crate::plugin::PluginType;
use crate::{ZhangError, ZhangResult};

#[derive(Default)]
pub struct PluginStore {
    pub processors: Vec<RegisteredPlugin>,
    pub mappers: Vec<RegisteredPlugin>,
    pub routers: Vec<RegisteredPlugin>,
    /// registration order with the types each plugin supports — this is the execution order
    pub ordered: Vec<(RegisteredPlugin, Vec<PluginType>)>,
}

impl PluginStore {
    /// register the plugin `_plugin` declares, as parsed into `declaration`; `span` is the directive's span
    pub fn insert_plugin(&mut self, _plugin: &Plugin, declaration: PluginDeclaration, span: &SpanInfo) -> ZhangResult<()> {
        let plugin_name = _plugin.module.as_str().to_string();
        let plugin_hash = digest(&plugin_name);
        let plugin_cache_file = PathBuf::from_str(".cache/plugins")
            .expect("Cannot create path")
            .join(format!("{}.wasm", plugin_hash));
        let module_bytes = std::fs::read(&plugin_cache_file)?;

        let wasm = Wasm::data(module_bytes.clone());
        let timeout = declaration.capabilities.timeout;
        let manifest = Manifest::new([wasm]).with_timeout(timeout);

        // a plugin importing a host function cannot be instantiated without it, so the router host
        // functions are linked too, answering that they are unavailable
        let host = PluginHost::new(_plugin.module.as_str(), span.clone());
        let functions = host.functions().into_iter().chain(unavailable_host_functions());
        let mut plugin = WasmPlugin::new(manifest, functions, true).map_err(|e| ZhangError::CustomError(format!("Failed to create WasmPlugin: {}", e)))?;
        let name = plugin
            .call::<(), WasmJson<String>>("name", ())
            .map_err(|e| call_error(&plugin_name, "name", timeout, e))?
            .0;
        let version = plugin
            .call::<(), WasmJson<String>>("version", ())
            .map_err(|e| call_error(&plugin_name, "version", timeout, e))?
            .0;
        let declared_types = plugin
            .call::<(), WasmJson<Vec<serde_json::Value>>>("supported_type", ())
            .map_err(|e| call_error(&plugin_name, "supported_type", timeout, e))?
            .0;
        let plugin_types = known_plugin_types(&name, declared_types)?;
        let ignored_errors = host.take_errors().len();
        if ignored_errors > 0 {
            warn!("plugin {name} reported {ignored_errors} error(s) while registering; only its processor and mapper can report errors");
        }

        let registered_plugin = RegisteredPlugin {
            name,
            version,
            module_bytes,
            declaration,
            span: span.clone(),
        };
        if plugin_types.contains(&PluginType::Processor) {
            self.processors.push(registered_plugin.clone())
        }
        if plugin_types.contains(&PluginType::Mapper) {
            self.mappers.push(registered_plugin.clone())
        }
        if plugin_types.contains(&PluginType::Router) {
            self.add_router(registered_plugin.clone())
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

/// the plugin types this host knows among the ones a plugin declares.
/// An unknown type (from a newer zhang, say) is dropped with a warning, so the plugin still loads.
fn known_plugin_types(plugin_name: &str, declared: Vec<serde_json::Value>) -> ZhangResult<Vec<PluginType>> {
    let mut known = vec![];
    for value in declared {
        let plugin_type = serde_json::from_value::<PluginType>(value.clone())
            .map_err(|e| ZhangError::CustomError(format!("plugin {plugin_name} declares an invalid plugin type {value}: {e}")))?;
        match plugin_type {
            PluginType::Unknown => warn!("plugin {plugin_name} declares the plugin type {value}, which this version of zhang does not know; ignoring it"),
            plugin_type => known.push(plugin_type),
        }
    }
    Ok(known)
}

/// the error of a failed call into a plugin; a call the host stopped at the plugin's timeout says so
fn call_error(plugin: &str, export: &str, timeout: Duration, error: extism::Error) -> ZhangError {
    // extism reports a call it interrupted at the manifest's timeout as exactly "timeout"
    if error.to_string() == "timeout" {
        ZhangError::CustomError(format!(
            "plugin {plugin} timed out: its `{export}` call ran longer than {timeout:?}. A `timeout` meta on its `plugin` directive raises the limit"
        ))
    } else {
        ZhangError::CustomError(format!("plugin {plugin} failed in its `{export}` call: {error}"))
    }
}

#[derive(Clone)]
pub struct RegisteredPlugin {
    pub name: String,
    pub version: String,
    /// the wasm module, kept in memory so executions don't re-read the cache file
    module_bytes: Vec<u8>,
    /// the capabilities and config declared by the plugin's directive
    declaration: PluginDeclaration,
    /// the span of the plugin's directive, where the errors it reports go unless they carry a span
    span: SpanInfo,
}

impl RegisteredPlugin {
    /// what the plugin's directive grants it
    pub fn capabilities(&self) -> &PluginCapabilities {
        &self.declaration.capabilities
    }

    /// the manifest of every instance of the plugin, whatever it runs as: config, allowed hosts and timeout
    pub(super) fn manifest(&self, options: &[OptionDomain]) -> Manifest {
        let config = self.declaration.config_with(options, self.declaration.host_config());
        let wasm = Wasm::data(self.module_bytes.clone());
        Manifest::new([wasm])
            .with_config(config.into_iter())
            // no declared host means the plugin gets no network access at all
            .with_allowed_hosts(self.declaration.capabilities.allowed_hosts.iter().cloned())
            .with_timeout(self.declaration.capabilities.timeout)
    }

    /// the host side of a new instance of this plugin
    pub fn host(&self) -> PluginHost {
        PluginHost::new(self.name.clone(), self.span.clone())
    }

    /// a new instance of the plugin, with the host functions of `host` linked in, and the router host
    /// functions answering that they are unavailable
    pub fn load_as_plugin(&self, options: &[OptionDomain], host: &PluginHost) -> ZhangResult<WasmPlugin> {
        info!("loading plugin {} {}", self.name, self.version);
        let functions = host.functions().into_iter().chain(unavailable_host_functions());
        let plugin = WasmPlugin::new(self.manifest(options), functions, true)
            .map_err(|e| ZhangError::CustomError(format!("cannot load plugin {}: {}", self.name, e)))?;

        Ok(plugin)
    }

    /// run the plugin's processor over the whole stream; the errors it reports go to `ctx`
    pub fn execute_as_processor(&self, directive: Vec<Spanned<Directive>>, ctx: &mut StageContext) -> ZhangResult<Vec<Spanned<Directive>>> {
        let host = self.host();
        let mut plugin = self.load_as_plugin(ctx.options, &host)?;
        let ret = plugin
            .call::<WasmJson<Vec<Spanned<Directive>>>, WasmJson<Vec<Spanned<Directive>>>>("processor", WasmJson(directive))
            .map_err(|e| call_error(&self.name, "processor", self.declaration.capabilities.timeout, e))?
            .0;
        host.forward_to(ctx);
        Ok(ret)
    }

    /// map every directive through the plugin, reusing a single instance for the whole stream;
    /// the errors it reports go to `ctx`
    pub fn execute_as_mapper(&self, directives: Vec<Spanned<Directive>>, ctx: &mut StageContext) -> ZhangResult<Vec<Spanned<Directive>>> {
        let host = self.host();
        let mut plugin = self.load_as_plugin(ctx.options, &host)?;
        let mut ret = vec![];
        for directive in directives {
            let mapped = plugin
                .call::<WasmJson<Spanned<Directive>>, WasmJson<Vec<Spanned<Directive>>>>("mapper", WasmJson(directive))
                .map_err(|e| call_error(&self.name, "mapper", self.declaration.capabilities.timeout, e))?
                .0;
            ret.extend(mapped);
        }
        host.forward_to(ctx);
        Ok(ret)
    }
}

#[cfg(test)]
mod test {
    use std::collections::BTreeMap;
    use std::time::Duration;

    use serde_json::json;
    use zhang_ast::{Meta, Plugin, SpanInfo, ZhangString};

    use crate::domains::schemas::OptionDomain;
    use crate::plugin::capabilities::PluginDeclaration;
    use crate::plugin::store::{call_error, known_plugin_types, RegisteredPlugin};
    use crate::plugin::PluginType;

    fn registered_with_meta(meta: &[(&str, &str)]) -> RegisteredPlugin {
        let directive = Plugin {
            module: ZhangString::quote("slow.wasm"),
            value: vec![],
            meta: meta.iter().map(|(key, value)| (key.to_string(), ZhangString::quote(*value))).collect(),
        };
        RegisteredPlugin {
            name: "slow".to_owned(),
            version: "0.1.0".to_owned(),
            module_bytes: vec![],
            declaration: PluginDeclaration::parse(&directive),
            span: SpanInfo::default(),
        }
    }

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
            span: SpanInfo::default(),
        };

        let manifest = plugin.manifest(&options);

        // the flat keys are unchanged: a repeated key keeps its last value, and the positional
        // value and `allowed_hosts` only reach the plugin through `zhang.plugin`
        let expected: BTreeMap<String, String> = [
            ("base_currency", "USD"),
            ("operating_currency", "EUR"),
            ("tag", "second"),
            ("timezone", "UTC"),
            ("zhang.abi", "1"),
            ("zhang.mine", "kept"),
            (
                "zhang.plugin",
                r#"{"module":"fx-rate.wasm","args":["positional"],"meta":{"allowed_hosts":["api.frankfurter.dev","api.example.com"],"base_currency":["USD"],"operating_currency":["EUR"],"tag":["first","second"],"zhang.mine":["kept"]}}"#,
            ),
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
            span: SpanInfo::default(),
        };

        assert_eq!(plugin.manifest(&[]).allowed_hosts, Some(vec![]));
    }

    #[test]
    fn should_ignore_unknown_plugin_types() {
        let declared = vec![json!("Teleporter"), json!("Processor"), json!("Router"), json!("Mapper")];

        let known = known_plugin_types("future", declared).unwrap();

        assert_eq!(known, vec![PluginType::Processor, PluginType::Router, PluginType::Mapper]);
    }

    #[test]
    fn should_reject_a_plugin_type_that_is_not_a_name() {
        assert!(known_plugin_types("broken", vec![json!(42)]).is_err());
    }

    #[test]
    fn should_give_every_call_a_default_timeout_of_a_minute() {
        assert_eq!(registered_with_meta(&[]).manifest(&[]).timeout_ms, Some(60_000));
    }

    #[test]
    fn should_let_the_timeout_meta_override_the_default() {
        assert_eq!(registered_with_meta(&[("timeout", "2m")]).manifest(&[]).timeout_ms, Some(120_000));
        assert_eq!(registered_with_meta(&[("timeout", "1")]).manifest(&[]).timeout_ms, Some(1_000));
        assert_eq!(
            registered_with_meta(&[("timeout", "soon")]).manifest(&[]).timeout_ms,
            Some(60_000),
            "an invalid timeout falls back to the default"
        );
    }

    #[test]
    fn should_say_that_a_plugin_timed_out() {
        let timed_out = call_error("slow", "processor", Duration::from_secs(1), extism::Error::msg("timeout")).to_string();
        assert!(timed_out.contains("plugin slow timed out"), "{timed_out}");
        assert!(timed_out.contains("`processor` call ran longer than 1s"), "{timed_out}");

        let failed = call_error("slow", "processor", Duration::from_secs(1), extism::Error::msg("boom")).to_string();
        assert!(failed.contains("plugin slow failed in its `processor` call: boom"), "{failed}");
    }
}
