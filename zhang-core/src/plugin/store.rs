use std::sync::Arc;
use std::time::Duration;

use chrono_tz::Tz;
#[cfg(feature = "plugin_runtime")]
use extism::convert::Json as WasmJson;
#[cfg(feature = "plugin_runtime")]
use extism::Manifest;
use log::{info, warn};
use zhang_ast::{Directive, Plugin, SpanInfo, Spanned};
use zhang_shared::plugin_abi::export;

use crate::clock::LoadClock;
use crate::domains::schemas::OptionDomain;
use crate::options::InMemoryOptions;
use crate::pipeline::StageContext;
use crate::plugin::capabilities::{PluginCapabilities, PluginDeclaration, PluginStage, WriteKind};
use crate::plugin::files::FileAccess;
use crate::plugin::host::PluginHost;
use crate::plugin::lua::{LuaScript, PluginModule};
use crate::plugin::runtime::{PluginInstance, PluginRuntime};
use crate::plugin::write::{declared_writes, DeclaredCapabilities};
use crate::plugin::{log_safe, PluginType};
use crate::plugin_policy::{PluginPolicy, Size, MAX_MEMORY_ENV};
use crate::{ZhangError, ZhangResult};

#[derive(Default)]
pub struct PluginStore {
    pub routers: Vec<RegisteredPlugin>,
    /// registration order with the types each plugin supports — this is the execution order
    pub ordered: Vec<(RegisteredPlugin, Vec<PluginType>)>,
}

impl PluginStore {
    /// register the plugin `_plugin` declares, whose module is `module`, as parsed into `declaration`; `span` is
    /// the directive's span. `clock` is the clock of the load, which the plugin reads in the ledger timezone
    /// `timezone`, `files` what the plugin may read once it runs as a processor or mapper, `options` the ledger's
    /// options, part of the config every instance of the plugin gets, and `policy` what the operator lets it use. The
    /// answer is the problem to report on the directive, if any: a router named like an earlier one
    /// ([`PluginStore::add_router`])
    #[allow(clippy::too_many_arguments)]
    pub fn insert_plugin(
        &mut self, _plugin: &Plugin, module: PluginModule, declaration: PluginDeclaration, span: &SpanInfo, clock: &LoadClock, timezone: Tz, files: FileAccess,
        options: &[OptionDomain], policy: Arc<PluginPolicy>,
    ) -> ZhangResult<Option<String>> {
        let plugin_name = _plugin.module.as_str().to_string();

        // the runtime compiles the module for the settings it is asked for and builds every instance
        // of the plugin, this registering one included, from the compiled code. Registering uses the
        // settings every other instance gets, so the plugin's `capabilities` export sees its config and
        // the compiled code is reused by the first stage run or request. A script's module is the
        // embedded Lua interpreter, and the script rides in the config of every manifest
        let (module_bytes, script) = module.into_parts();
        let runtime = Arc::new(PluginRuntime::new(module_bytes));
        let timeout = declaration.capabilities.timeout;
        let occurrence = self.occurrences(&declaration.directive.module);
        let manifest = settings(&declaration, occurrence, options, script.as_ref(), &policy);

        // registering, the file functions deny every path and the router functions answer that they
        // are unavailable
        let host = PluginHost::registering(_plugin.module.as_str(), span.clone(), clock.clone(), timezone);
        let mut plugin = runtime
            .instantiate(manifest, host.clone())
            .map_err(|e| ZhangError::CustomError(format!("Failed to create WasmPlugin: {}", e)))?;
        // a script's identity exports run its chunk, so this is where a script that does not parse, or
        // returns no module table, fails to load
        let failed = |export: &str, error: extism::Error| match script.is_some() {
            true => script_error(&plugin_name, export, timeout, &policy, error),
            false => call_error(&plugin_name, export, timeout, &policy, error),
        };
        let name = plugin.call::<(), WasmJson<String>>(export::NAME, ()).map_err(|e| failed(export::NAME, e))?.0;
        let version = plugin
            .call::<(), WasmJson<String>>(export::VERSION, ())
            .map_err(|e| failed(export::VERSION, e))?
            .0;
        let declared_types = plugin
            .call::<(), WasmJson<Vec<serde_json::Value>>>(export::SUPPORTED_TYPE, ())
            .map_err(|e| failed(export::SUPPORTED_TYPE, e))?
            .0;
        let plugin_types = known_plugin_types(&name, declared_types)?;
        // a script with no type serves nothing: `types = {Router = true}` or `{}` yields none, and a plugin that
        // never runs is a mistake, not a choice (a wasm module keeps today's behaviour, its unknown types are warned)
        if script.is_some() && plugin_types.is_empty() {
            return Err(ZhangError::CustomError(format!(
                "plugin script {plugin_name} declares no plugin type; write `types = {{\"Router\"}}` in its module table"
            )));
        }
        let writes = declared_capabilities(&mut plugin, &name, timeout, &policy)?;
        let ignored_errors = host.take_errors().len();
        if ignored_errors > 0 {
            warn!(
                "plugin {} reported {ignored_errors} error(s) while registering; only its processor and mapper can report errors",
                log_safe(&name)
            );
        }

        let registered_plugin = RegisteredPlugin {
            name,
            version,
            runtime,
            script,
            declaration,
            span: span.clone(),
            occurrence,
            files,
            policy,
            writes,
        };
        let conflict = match plugin_types.contains(&PluginType::Router) {
            true => {
                let stages = plugin_types.iter().any(|it| matches!(it, PluginType::Processor | PluginType::Mapper));
                self.add_router(registered_plugin.clone(), stages)
            }
            false => None,
        };
        self.ordered.push((registered_plugin, plugin_types));

        Ok(conflict)
    }

    /// how many plugins registered so far declare `module`, as written
    fn occurrences(&self, module: &str) -> usize {
        self.ordered.iter().filter(|(plugin, _)| plugin.declaration.directive.module == module).count()
    }

    /// build the pipeline stages of the plugins declared to run in `stage` (their `stage` meta,
    /// [`PluginStage`]), in plugin declaration order. A plugin supporting both types contributes its
    /// processor stage first, then its mapper stage.
    pub fn build_stages(&self, stage: PluginStage) -> Vec<Box<dyn crate::pipeline::ProcessStage>> {
        use crate::plugin::stage::{WasmMapperStage, WasmProcessorStage};
        let mut stages: Vec<Box<dyn crate::pipeline::ProcessStage>> = vec![];
        for (plugin, types) in &self.ordered {
            if plugin.declaration.capabilities.stage != stage {
                continue;
            }
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
            PluginType::Unknown => warn!(
                "plugin {} declares the plugin type {value}, which this version of zhang does not know; ignoring it",
                log_safe(plugin_name)
            ),
            plugin_type => known.push(plugin_type),
        }
    }
    Ok(known)
}

/// the settings of every instance of the plugin `declaration` declares, the `occurrence`-th of its module, within
/// what `policy` allows: its config (the ledger's `options`, its own meta, the values the host sets, and a Lua `script`
/// under the reserved keys, so a script edit compiles anew), its allowed hosts, its timeout capped at the policy's
/// `max_timeout` (the cap a router call gets, so registering, a stage and a request share one compiled code), and the
/// policy's memory limit. The module itself belongs to the runtime, which compiles it for these settings
fn settings(declaration: &PluginDeclaration, occurrence: usize, options: &[OptionDomain], script: Option<&LuaScript>, policy: &PluginPolicy) -> Manifest {
    let reserved = declaration
        .host_config(occurrence)
        .into_iter()
        .chain(script.into_iter().flat_map(LuaScript::config));
    let config = declaration.config_with(options, reserved);
    Manifest::default()
        .with_config(config.into_iter())
        // no declared host means the plugin gets no network access at all
        .with_allowed_hosts(declaration.capabilities.allowed_hosts.iter().cloned())
        .with_timeout(declaration.capabilities.timeout.min(policy.max_timeout))
        .with_memory_max(policy.max_memory_pages())
}

/// the write kinds the plugin's optional `capabilities` export declares, read once while it registers: none without
/// the export. An export that answers something else fails the load, as `supported_type` would
fn declared_capabilities(plugin: &mut PluginInstance, name: &str, timeout: Duration, policy: &PluginPolicy) -> ZhangResult<Vec<WriteKind>> {
    if !plugin.function_exists(export::CAPABILITIES) {
        return Ok(vec![]);
    }
    let declared = plugin
        .call::<(), WasmJson<DeclaredCapabilities>>(export::CAPABILITIES, ())
        .map_err(|e| call_error(name, export::CAPABILITIES, timeout, policy, e))?
        .0;
    Ok(declared_writes(name, &declared))
}

/// the error of a failed registration call into a Lua script: the message the interpreter set, which for a
/// script that does not parse carries `<script>:<line>: ...`; a call stopped at the timeout or out of memory says
/// so as for a module
fn script_error(plugin: &str, export: &str, timeout: Duration, policy: &PluginPolicy, error: extism::Error) -> ZhangError {
    if matches!(error.to_string().as_str(), "timeout" | "oom") {
        return call_error(plugin, export, timeout, policy, error);
    }
    ZhangError::CustomError(format!(
        "plugin script {} cannot be loaded: {}",
        log_safe(plugin),
        log_safe(&format!("{error:#}"))
    ))
}

/// the error of a failed call into a plugin: a call the host stopped at the plugin's timeout, or that ran out of the
/// memory `policy` allows, says so. What the plugin wrote into the error is escaped ([`log_safe`]), as the error ends
/// up in the server's log
fn call_error(plugin: &str, export: &str, timeout: Duration, policy: &PluginPolicy, error: extism::Error) -> ZhangError {
    let plugin = log_safe(plugin);
    match error.to_string().as_str() {
        // extism reports a call it interrupted at the manifest's timeout as exactly "timeout"
        "timeout" => ZhangError::CustomError(format!(
            "plugin {plugin} timed out: its `{export}` call ran longer than {timeout:?}. A `timeout` meta on its `plugin` directive raises the limit"
        )),
        // and one whose memory would grow past the manifest's limit as exactly "oom"
        "oom" => ZhangError::CustomError(format!(
            "plugin {plugin} ran out of memory: its `{export}` call needed more than the {} the server allows a plugin ({MAX_MEMORY_ENV})",
            Size(policy.max_memory)
        )),
        _ => ZhangError::CustomError(format!("plugin {plugin} failed in its `{export}` call: {}", log_safe(&format!("{error:#}")))),
    }
}

#[derive(Clone)]
pub struct RegisteredPlugin {
    pub name: String,
    pub version: String,
    /// the plugin's module and its compiled code, shared by every clone of this registration and every
    /// instance they build (see [`PluginRuntime`])
    pub(super) runtime: Arc<PluginRuntime>,
    /// the Lua script the runtime's interpreter runs, for a `plugin "x.lua"` directive; `None` for a wasm module
    script: Option<LuaScript>,
    /// the capabilities and config declared by the plugin's directive
    declaration: PluginDeclaration,
    /// the span of the plugin's directive, where the errors it reports go unless they carry a span
    span: SpanInfo,
    /// how many plugins registered before this one declare the same module; it makes their seeds differ
    occurrence: usize,
    /// the files the plugin's `allowed_paths` let it read
    files: FileAccess,
    /// what the operator lets the plugin use
    policy: Arc<PluginPolicy>,
    /// the write kinds the plugin declared in its `capabilities` export, each once
    writes: Vec<WriteKind>,
}

impl RegisteredPlugin {
    /// what the plugin's directive grants it
    pub fn capabilities(&self) -> &PluginCapabilities {
        &self.declaration.capabilities
    }

    /// what the operator lets the plugin use
    pub fn policy(&self) -> &PluginPolicy {
        &self.policy
    }

    /// the plugin's module as the load read it, from the ledger's storage or from a URL; for a script, the
    /// interpreter (the script is [`RegisteredPlugin::script`])
    pub fn module_bytes(&self) -> &[u8] {
        self.runtime.module_bytes()
    }

    /// the write kinds the plugin declared in its `capabilities` export; it writes those its directive also grants
    pub fn declared_writes(&self) -> &[WriteKind] {
        &self.writes
    }

    /// the module of the plugin's directive, as written
    pub fn module(&self) -> &str {
        &self.declaration.directive.module
    }

    /// the Lua script this plugin runs, for a `plugin "x.lua"` directive
    pub fn script(&self) -> Option<&LuaScript> {
        self.script.as_ref()
    }

    /// the settings of every instance of the plugin, whatever it runs as: config, allowed hosts, timeout and memory
    /// limit, the ones it registered with ([`settings`])
    pub(super) fn manifest(&self, options: &[OptionDomain]) -> Manifest {
        settings(&self.declaration, self.occurrence, options, self.script.as_ref(), &self.policy)
    }

    /// [`RegisteredPlugin::manifest`] with the timeout `timeout` instead of the plugin's own
    pub(super) fn manifest_with_timeout(&self, options: &[OptionDomain], timeout: Duration) -> Manifest {
        self.manifest(options).with_timeout(timeout)
    }

    /// what a failed call of the plugin's `export` is, see [`call_error`]
    pub(super) fn call_error(&self, export: &str, timeout: Duration, error: extism::Error) -> ZhangError {
        call_error(&self.name, export, timeout, &self.policy, error)
    }

    /// the host side of a new instance of this plugin, running as a stage with the context `ctx`, with the
    /// files the plugin's `allowed_paths` let it read
    pub fn host(&self, ctx: &StageContext) -> PluginHost {
        PluginHost::new(self.name.clone(), self.span.clone(), ctx.clock().clone(), ctx.timezone()).with_files(self.files.clone())
    }

    /// the host side of a new instance of this plugin handling an HTTP request as a router: `zhang_now`
    /// reads `clock` afresh for the request and records nothing (see [`PluginHost::routing`]), and the file
    /// functions deny every path, whatever `allowed_paths` grants
    pub fn routing_host(&self, clock: LoadClock, timezone: Tz) -> PluginHost {
        PluginHost::routing(self.name.clone(), self.span.clone(), clock, timezone)
    }

    /// a new instance of the plugin, whose host functions reach `host`
    pub fn load_as_plugin(&self, options: &InMemoryOptions, host: &PluginHost) -> ZhangResult<PluginInstance> {
        info!("loading plugin {} {}", log_safe(&self.name), log_safe(&self.version));
        let plugin = self
            .runtime
            .instantiate(self.manifest(&options.all()), host.clone())
            .map_err(|e| ZhangError::CustomError(format!("cannot load plugin {}: {}", log_safe(&self.name), log_safe(&format!("{e:#}")))))?;

        Ok(plugin)
    }

    /// run the plugin's processor over the whole stream; the errors it reports and the files it reads go to `ctx`
    pub fn execute_as_processor(&self, directive: Vec<Spanned<Directive>>, ctx: &mut StageContext) -> ZhangResult<Vec<Spanned<Directive>>> {
        let host = self.host(ctx);
        let mut plugin = self.load_as_plugin(ctx.options, &host)?;
        let ret = plugin
            .call::<WasmJson<Vec<Spanned<Directive>>>, WasmJson<Vec<Spanned<Directive>>>>(export::PROCESSOR, WasmJson(directive))
            .map_err(|e| self.call_error(export::PROCESSOR, self.declaration.capabilities.timeout, e))?
            .0;
        host.forward_to(ctx);
        Ok(ret)
    }

    /// map every directive through the plugin, reusing a single instance for the whole stream;
    /// the errors it reports and the files it reads go to `ctx`
    pub fn execute_as_mapper(&self, directives: Vec<Spanned<Directive>>, ctx: &mut StageContext) -> ZhangResult<Vec<Spanned<Directive>>> {
        let host = self.host(ctx);
        let mut plugin = self.load_as_plugin(ctx.options, &host)?;
        let mut ret = vec![];
        for directive in directives {
            let mapped = plugin
                .call::<WasmJson<Spanned<Directive>>, WasmJson<Vec<Spanned<Directive>>>>(export::MAPPER, WasmJson(directive))
                .map_err(|e| self.call_error(export::MAPPER, self.declaration.capabilities.timeout, e))?
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
    use std::path::Path;
    use std::sync::Arc;
    use std::time::Duration;

    use serde_json::json;
    use zhang_ast::{Meta, Plugin, SpanInfo, ZhangString};

    use crate::clock::LoadClock;
    use crate::data_source::LocalFileSystemDataSource;
    use crate::data_type::text::ZhangDataType;
    use crate::domains::schemas::OptionDomain;
    use crate::plugin::capabilities::PluginDeclaration;
    use crate::plugin::files::FileAccess;
    use crate::plugin::host::PluginHost;
    use crate::plugin::lua::{LuaScript, PluginModule};
    use crate::plugin::runtime::PluginRuntime;
    use crate::plugin::store::{call_error, known_plugin_types, script_error, PluginStore, RegisteredPlugin};
    use crate::plugin::PluginType;
    use crate::plugin_policy::{PluginPolicy, MIB};

    fn no_files() -> FileAccess {
        FileAccess::new(vec![], Arc::new(LocalFileSystemDataSource::new(ZhangDataType {})), Path::new("/ledger"))
    }

    fn registered_with_meta(meta: &[(&str, &str)]) -> RegisteredPlugin {
        registered_as("slow.wasm", meta, 0)
    }

    /// a plugin of `module` with `meta`, the `occurrence`-th of its module
    fn registered_as(module: &str, meta: &[(&str, &str)], occurrence: usize) -> RegisteredPlugin {
        let directive = Plugin {
            module: ZhangString::quote(module),
            value: vec![],
            meta: meta.iter().map(|(key, value)| (key.to_string(), ZhangString::quote(*value))).collect(),
        };
        RegisteredPlugin {
            name: "slow".to_owned(),
            version: "0.1.0".to_owned(),
            runtime: Arc::new(PluginRuntime::new(vec![])),
            script: None,
            declaration: PluginDeclaration::parse(&directive),
            span: SpanInfo::default(),
            occurrence,
            files: no_files(),
            policy: Arc::new(PluginPolicy::default()),
            writes: vec![],
        }
    }

    #[test]
    fn should_count_only_earlier_plugins_of_the_same_module() {
        let mut store = PluginStore::default();
        assert_eq!(store.occurrences("fx-rate.wasm"), 0);

        store.ordered.push((registered_as("other.wasm", &[], 0), vec![PluginType::Processor]));
        assert_eq!(store.occurrences("fx-rate.wasm"), 0, "a plugin of another module changes nothing");

        store
            .ordered
            .push((registered_as("fx-rate.wasm", &[("seed", "ids")], 0), vec![PluginType::Mapper]));
        store.ordered.push((registered_as("other.wasm", &[], 1), vec![]));
        assert_eq!(store.occurrences("fx-rate.wasm"), 1);
        assert_eq!(store.occurrences("other.wasm"), 2);
        assert_eq!(store.occurrences("./fx-rate.wasm"), 0, "the module as written");
    }

    #[test]
    fn should_hand_each_occurrence_of_a_module_its_own_stable_seed() {
        let seed = |occurrence: usize, meta: &[(&str, &str)]| registered_as("fx-rate.wasm", meta, occurrence).manifest(&[]).config["zhang.seed"].clone();

        assert_eq!(seed(0, &[]), "1811957226761548848");
        assert_eq!(seed(0, &[]), seed(0, &[]));
        assert_eq!(seed(1, &[]), "14470161454731210553");
        assert_eq!(seed(0, &[("seed", "ids")]), "3501562816358462189");
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
            runtime: Arc::new(PluginRuntime::new(vec![])),
            script: None,
            declaration: PluginDeclaration::parse(&directive),
            span: SpanInfo::default(),
            occurrence: 0,
            files: no_files(),
            policy: Arc::new(PluginPolicy::default()),
            writes: vec![],
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
            ("zhang.seed", "1811957226761548848"),
            ("zhang.allowed_hosts", r#"["api.frankfurter.dev","api.example.com"]"#),
            ("zhang.timeout_ms", "60000"),
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
    fn should_pair_a_script_with_the_interpreter_through_the_reserved_config_keys() {
        let meta: Meta = [("zhang.script", "spoofed"), ("zhang.script_name", "spoofed"), ("greeting", "hi")]
            .into_iter()
            .map(|(key, value)| (key.to_owned(), ZhangString::quote(value)))
            .collect();
        let directive = Plugin {
            module: ZhangString::quote("plugins/report.lua"),
            value: vec![],
            meta,
        };
        let plugin = RegisteredPlugin {
            name: "report".to_owned(),
            version: "0.1.0".to_owned(),
            runtime: Arc::new(PluginRuntime::new(vec![])),
            script: Some(LuaScript {
                name: "plugins/report.lua".to_owned(),
                source: "return {}".to_owned(),
            }),
            declaration: PluginDeclaration::parse(&directive),
            span: SpanInfo::default(),
            occurrence: 0,
            files: no_files(),
            policy: Arc::new(PluginPolicy::default()),
            writes: vec![],
        };

        let config = plugin.manifest(&[]).config;

        // the host's script wins over a meta of the same name, like every reserved key
        assert_eq!(config["zhang.script"], "return {}");
        assert_eq!(config["zhang.script_name"], "plugins/report.lua");
        assert_eq!(config["greeting"], "hi");
        assert_eq!(config["zhang.abi"], "1");
        assert!(!registered_as("slow.wasm", &[], 0).manifest(&[]).config.contains_key("zhang.script"));
    }

    #[test]
    fn should_register_with_the_settings_every_call_gets_so_the_load_compiles_once() {
        let directive = Plugin {
            module: ZhangString::quote("config_echo.wat"),
            value: vec![],
            meta: [("greeting", "hi")]
                .into_iter()
                .map(|(key, value)| (key.to_owned(), ZhangString::quote(value)))
                .collect(),
        };
        let options = [OptionDomain {
            key: "title".to_owned(),
            value: "Home".to_owned(),
        }];
        let module = PluginModule::Wasm(include_bytes!("../../tests/plugins/config_echo.wat").to_vec());
        let clock = LoadClock::new(crate::clock::Clock::System);
        let mut store = PluginStore::default();
        let conflict = store
            .insert_plugin(
                &directive,
                module,
                PluginDeclaration::parse(&directive),
                &SpanInfo::default(),
                &clock,
                chrono_tz::UTC,
                no_files(),
                &options,
                Arc::new(PluginPolicy::default()),
            )
            .unwrap();
        assert_eq!(conflict, None);
        let (plugin, _) = &store.ordered[0];

        // registering compiled once (generation 1) with the config a run gets: the option and the meta
        assert_eq!(plugin.runtime.pool_state(), (1, 1));
        let manifest = plugin.manifest(&options);
        assert_eq!(manifest.config["title"], "Home");
        assert_eq!(manifest.config["greeting"], "hi");
        // a run with the same settings reuses that compiled code: the generation does not move
        let host = PluginHost::new("config-echo", SpanInfo::default(), clock, chrono_tz::UTC);
        let instance = plugin.runtime.instantiate(manifest, host).unwrap();
        assert_eq!(plugin.runtime.pool_state(), (0, 1), "the registering compile is the one in use");
        drop(instance);
        assert_eq!(plugin.runtime.pool_state(), (1, 1));
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
            runtime: Arc::new(PluginRuntime::new(vec![])),
            script: None,
            declaration: PluginDeclaration::parse(&directive),
            span: SpanInfo::default(),
            occurrence: 0,
            files: no_files(),
            policy: Arc::new(PluginPolicy::default()),
            writes: vec![],
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

    /// `registered_with_meta` under a policy without a ceiling on the timeout
    fn unbounded(meta: &[(&str, &str)]) -> RegisteredPlugin {
        RegisteredPlugin {
            policy: Arc::new(PluginPolicy::unrestricted()),
            ..registered_with_meta(meta)
        }
    }

    #[test]
    fn should_give_every_call_a_default_timeout_of_a_minute() {
        assert_eq!(unbounded(&[]).manifest(&[]).timeout_ms, Some(60_000));
    }

    #[test]
    fn should_let_the_timeout_meta_override_the_default() {
        assert_eq!(unbounded(&[("timeout", "2m")]).manifest(&[]).timeout_ms, Some(120_000));
        assert_eq!(unbounded(&[("timeout", "1")]).manifest(&[]).timeout_ms, Some(1_000));
        assert_eq!(
            unbounded(&[("timeout", "soon")]).manifest(&[]).timeout_ms,
            Some(60_000),
            "an invalid timeout falls back to the default"
        );
    }

    /// Every manifest of a plugin, registering, a stage and a request, caps the timeout at the policy's `max_timeout`,
    /// the cap a router call gets: one manifest, so one compiled code per load (a default Lua router compiled twice
    /// per load while registering used the uncapped minute and a request the cap)
    #[test]
    fn should_cap_the_timeout_of_every_manifest_as_a_router_call_is_capped() {
        let plugin = registered_with_meta(&[]);
        let cap = plugin.policy().max_timeout;
        assert!(cap < Duration::from_secs(60), "the default ceiling is below the default timeout");
        assert_eq!(plugin.manifest(&[]).timeout_ms, Some(cap.as_millis() as u64));
        assert_eq!(plugin.manifest(&[]), plugin.manifest_with_timeout(&[], plugin.router_timeout()));
        let short = registered_with_meta(&[("timeout", "1")]);
        assert_eq!(short.manifest(&[]).timeout_ms, Some(1_000), "a timeout under the ceiling is kept");
    }

    #[test]
    fn should_say_that_a_plugin_timed_out() {
        let policy = PluginPolicy::default();
        let timed_out = call_error("slow", "processor", Duration::from_secs(1), &policy, extism::Error::msg("timeout")).to_string();
        assert!(timed_out.contains("plugin slow timed out"), "{timed_out}");
        assert!(timed_out.contains("`processor` call ran longer than 1s"), "{timed_out}");

        let failed = call_error("slow", "processor", Duration::from_secs(1), &policy, extism::Error::msg("boom")).to_string();
        assert!(failed.contains("plugin slow failed in its `processor` call: boom"), "{failed}");

        let script = script_error(
            "x.lua",
            "name",
            Duration::from_secs(1),
            &policy,
            extism::Error::msg("x.lua:4: unexpected symbol near '='"),
        )
        .to_string();
        assert!(
            script.ends_with("plugin script x.lua cannot be loaded: x.lua:4: unexpected symbol near '='"),
            "{script}"
        );
        let timed_out = script_error("x.lua", "name", Duration::from_secs(1), &policy, extism::Error::msg("timeout")).to_string();
        assert!(timed_out.contains("plugin x.lua timed out"), "{timed_out}");
    }

    #[test]
    fn should_say_that_a_plugin_ran_out_of_memory() {
        let policy = PluginPolicy {
            max_memory: 16 * MIB,
            ..PluginPolicy::default()
        };
        let error = call_error("hog", "mapper", Duration::from_secs(1), &policy, extism::Error::msg("oom")).to_string();
        assert_eq!(
            error,
            "custom error: plugin hog ran out of memory: its `mapper` call needed more than the 16 MiB the server allows a plugin (ZHANG_PLUGIN_MAX_MEMORY)"
        );
    }

    #[test]
    fn should_escape_what_the_plugin_wrote_into_its_error() {
        let error = call_error(
            "evil\nname",
            "processor",
            Duration::from_secs(1),
            &PluginPolicy::default(),
            extism::Error::msg("boom\n[2024-01-01T00:00:00Z ERROR zhang] forged"),
        )
        .to_string();
        assert_eq!(
            error,
            "custom error: plugin evil\\nname failed in its `processor` call: boom\\n[2024-01-01T00:00:00Z ERROR zhang] forged"
        );
        assert!(!error.contains('\n'));
    }

    #[test]
    fn should_cap_the_memory_of_every_instance_by_the_policy() {
        assert_eq!(registered_with_meta(&[]).manifest(&[]).memory.max_pages, Some(4096), "256 MiB by default");
        let mut plugin = registered_with_meta(&[]);
        plugin.policy = Arc::new(PluginPolicy {
            max_memory: 16 * MIB,
            ..PluginPolicy::default()
        });
        assert_eq!(plugin.manifest(&[]).memory.max_pages, Some(256));
        assert_eq!(plugin.manifest_with_timeout(&[], Duration::from_secs(1)).memory.max_pages, Some(256));
    }
}
