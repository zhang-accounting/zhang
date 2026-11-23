#[cfg(feature = "plugin_runtime")]
use std::collections::HashMap;

#[cfg(feature = "plugin_runtime")]
use zhang_ast::error::ErrorKind;
use zhang_ast::{Plugin, SpanInfo};

use crate::ledger::Ledger;
use crate::ZhangResult;

/// the path the data source reads the module `module` of a `plugin` directive by: a URL as written, a file by its path
/// within the ledger whose root is `root`, which an absolute path under the root is given by, like an `include`.
///
/// Where the file is on the local disk, it is where the disk resolves it that counts: a root spelled through a
/// symlink holds its files whichever way the module spells it (on macOS, `/var` is `/private/var`, and a root is
/// spelled either way by how the ledger was opened), and a symlink inside the root pointing outside leads outside.
/// Elsewhere (remote storage, or a module that is not there), the path as spelled decides. A module outside the
/// ledger's directory names no file of the ledger, on the local disk as on remote storage, and is refused naming it
/// and the rule, so a ledger loads alike wherever it is stored
#[cfg(feature = "plugin_runtime")]
fn module_path(root: &std::path::Path, module: &str) -> ZhangResult<String> {
    use crate::data_source::{path_in_ledger, slashed};
    if module.contains("://") {
        return Ok(module.to_owned());
    }
    let path = std::path::Path::new(module);
    let within = |root: &std::path::Path, path: &std::path::Path| path_in_ledger(root, path).filter(|it| !it.as_os_str().is_empty());
    let spelled = within(root, path);
    // `root.join` of an absolute path is that path
    let resolved = match (std::fs::canonicalize(root), std::fs::canonicalize(root.join(path))) {
        (Ok(real_root), Ok(real)) => Some((within(&real_root, &real), real)),
        _ => None,
    };
    let outside = |detail: String| {
        crate::ZhangError::CustomError(format!(
            "plugin module {module} {detail} the ledger's directory {}: a plugin module is a file of the ledger, named by a path within its directory, on the local disk as on remote storage",
            root.display()
        ))
    };
    match (spelled, resolved) {
        (None, Some((None, _))) | (None, None) => Err(outside("is outside".to_owned())),
        (Some(_), Some((None, real))) => Err(outside(format!("resolves to {}, outside", real.display()))),
        (Some(spelled), _) => Ok(slashed(&spelled)),
        (None, Some((Some(resolved), _))) => Ok(slashed(&resolved)),
    }
}

/// the error of reading the module `module`: a module that is not there stops the load naming it (#487)
#[cfg(feature = "plugin_runtime")]
fn module_error(module: &str, error: crate::ZhangError) -> crate::ZhangError {
    match error.is_file_not_found() {
        true => crate::ZhangError::CustomError(format!("plugin module not found: {module}")),
        false => error,
    }
}

/// register the plugin of `plugin` into `ledger`
#[cfg_attr(not(feature = "plugin_runtime"), allow(unused_variables))]
pub(crate) fn register(plugin: &Plugin, ledger: &mut Ledger, span: &SpanInfo) -> ZhangResult<()> {
    feature_enable!(ledger.options.features.plugins, {
        #[cfg(feature = "plugin_runtime")]
        {
            let module = plugin.module.as_str();
            // the operator turned plugins off: the directive says so, and nothing of the plugin is read
            if !ledger.plugin_policy.loads_plugins() {
                let metas = HashMap::from([
                    ("plugin".to_owned(), module.to_owned()),
                    ("setting".to_owned(), crate::plugin_policy::LEVEL_ENV.to_owned()),
                ]);
                ledger.report(ErrorKind::PluginsDisabled, span, metas);
                return Ok(());
            }
            // the module is read from the ledger's source on every load, and handed to the runtime as it is: a
            // wasm module as its bytes, a Lua script (by its `.lua` extension) as text the interpreter runs. A
            // `zhang:` module is a plugin built into zhang, which nothing is read for. It is looked up after the check
            // above, so plugins off loads no bundled plugin either
            let bundled = crate::plugin::bundled::BundledPlugin::lookup(module).transpose()?;
            let parsed = match bundled {
                Some(bundled) => bundled.module(module)?,
                None => {
                    let path = module_path(&ledger.entry.0, module)?;
                    crate::plugin::lua::PluginModule::parse(module, || ledger.data_source.get(path).map_err(|error| module_error(module, error)))?
                }
            };
            let mut declaration = crate::plugin::capabilities::PluginDeclaration::parse(plugin);
            // a meta value the host cannot use is reported on the directive, and the plugin runs with the default
            for error in &declaration.errors {
                ledger.report(error.kind.clone(), span, error.metas.clone());
            }
            // the plugin gets what its directive grants within what the operator allows, and the directive says what
            // it asked for beyond that
            let timeout_declared = declaration.declares_timeout();
            for reduced in declaration.capabilities.bound_by(&ledger.plugin_policy, timeout_declared) {
                ledger.report(ErrorKind::PluginCapabilityReduced, span, reduced.metas(module));
            }
            let (clock, timezone) = (ledger.clock.clone(), ledger.options.timezone);
            let files = crate::plugin::files::FileAccess::new(declaration.capabilities.allowed_paths.clone(), ledger.data_source.clone(), &ledger.entry.0);
            let (options, policy) = (ledger.options.all(), ledger.plugin_policy.clone());
            let conflict = ledger
                .plugins
                .insert_plugin(plugin, parsed, declaration, span, &clock, timezone, files, &options, policy)?;
            // a router named like an earlier one serves nothing: said on its directive, where the owner reads the
            // ledger's errors, while the ledger loads and the first one keeps serving
            if let Some(message) = conflict {
                let metas = [
                    (crate::plugin::host::PLUGIN_META.to_owned(), plugin.module.as_str().to_owned()),
                    (crate::plugin::host::MESSAGE_META.to_owned(), message),
                ];
                ledger.report(zhang_ast::error::ErrorKind::PluginError, span, metas.into_iter().collect());
            }
            // a rebuilt local module makes the ledger stale; a bundled one changes only with the binary
            if bundled.is_none() {
                if let Some(input) = crate::inputs::ExtraInput::plugin_module(&ledger.entry.0, plugin.module.as_str()) {
                    ledger.extra_inputs.insert(input);
                }
            }
        }
    });

    Ok(())
}

/// Register the plugins of `ledger`, whose `plugin` directives are `directives`: the official default plugins the
/// operator's policy stops are noted first ([`crate::extensions::stopped_by`]), then the embedder's default plugins are
/// registered ([`crate::analysis::LoadOptions::default_plugins`]), each configured by the first directive naming its
/// module, if any, and then the plugins of the other directives, in order. A directive naming a default plugin, the
/// embedder's or an official one ([`crate::extensions::DEFAULT_PROVIDERS`]), registers nothing of its own: a default
/// plugin runs once, whatever the ledger's `features.plugin`, so that no option of the ledger turns budgets off.
///
/// An official plugin has no options, whichever implementation serves it: natively, or the bundled script the embedder
/// registers for it (`zhang:budget`'s `plugins/budget/budget.lua` declares none). Its directive configures nothing, and
/// each of its arguments and metas says so ([`configure_default`]), as before the script.
pub(crate) fn register_all(ledger: &mut Ledger, directives: &[(Plugin, SpanInfo)]) -> ZhangResult<()> {
    ledger.extensions.unavailable = crate::extensions::stopped_by(&ledger.plugin_policy);
    let defaults = ledger.load_options.default_plugins.clone();
    for default in &defaults {
        let directive = directives.iter().find(|(plugin, _)| plugin.module.as_str() == default.module);
        let official = crate::extensions::DEFAULT_PROVIDERS.contains(&default.module.as_str());
        register_default(default, directive, !official, ledger)?;
    }
    let registered = |module: &str| defaults.iter().any(|default| default.module == module);
    // the first directive naming each default plugin: the one that configures it
    let mut configuring = std::collections::HashMap::<&str, &SpanInfo>::new();
    for (plugin, span) in directives {
        let module = plugin.module.as_str();
        let official = crate::extensions::DEFAULT_PROVIDERS.contains(&module);
        if official || registered(module) {
            let first = *configuring.entry(module).or_insert(span);
            let later = (!std::ptr::eq(first, span)).then_some(first);
            // an official plugin has no options, served natively or by the script the embedder registers for it
            configure_default(plugin, ledger, span, later, official);
        } else {
            register(plugin, ledger, span)?;
        }
    }
    Ok(())
}

/// A `plugin` directive naming a default plugin, at `span`. The first one configures the plugin, which
/// [`register_default`] registered with its metas; a later one configures nothing, and says so naming the first,
/// `configured_by`. An official plugin (`official`), such as `zhang:budget`, has no options, whether zhang serves it
/// natively or the embedder registers its bundled script: each argument and meta of the first directive naming it
/// configures nothing, and says so. Each says it with a
/// [`PluginError`](zhang_ast::error::ErrorKind::PluginError), as a router plugin that serves nothing does, whose
/// `message` says what to remove. With plugins off the directive says, whatever the ledger's `features.plugin`, that
/// the plugin is not loaded, and nothing else.
fn configure_default(plugin: &Plugin, ledger: &mut Ledger, span: &SpanInfo, configured_by: Option<&SpanInfo>, official: bool) {
    let module = plugin.module.as_str();
    if !ledger.plugin_policy.loads_plugins() {
        let metas = std::collections::HashMap::from([
            ("plugin".to_owned(), module.to_owned()),
            ("setting".to_owned(), crate::plugin_policy::LEVEL_ENV.to_owned()),
        ]);
        ledger.report(zhang_ast::error::ErrorKind::PluginsDisabled, span, metas);
        return;
    }
    let configures_nothing = |ledger: &mut Ledger, message: String| {
        let metas = std::collections::HashMap::from([("plugin".to_owned(), module.to_owned()), ("message".to_owned(), message)]);
        ledger.report(zhang_ast::error::ErrorKind::PluginError, span, metas);
    };
    if let Some(first) = configured_by {
        let message = format!(
            "this directive configures nothing: {module} is configured by the directive {}. Remove this one",
            located(ledger, first)
        );
        configures_nothing(ledger, message);
        return;
    }
    if official {
        for argument in &plugin.value {
            let message = format!("the argument {:?} configures nothing: {module} has no options. Remove it", argument.as_str());
            configures_nothing(ledger, message);
        }
        let mut keys = plugin.meta.clone().get_flatten().into_iter().map(|(key, _)| key).collect::<Vec<_>>();
        keys.dedup();
        for key in keys {
            configures_nothing(ledger, format!("the meta {key} configures nothing: {module} has no options. Remove it"));
        }
    }
}

/// Where the directive at `span` is, for a message: `at <file>:<line>`, the file named within the ledger
fn located(ledger: &Ledger, span: &SpanInfo) -> String {
    let file = span.filename.as_deref().map(|file| {
        ledger
            .path_in_ledger(file)
            .map_or_else(|| file.display().to_string(), |it| crate::data_source::slashed(&it))
    });
    match (file, span.line) {
        (Some(file), Some(line)) => format!("at {file}:{line}"),
        (Some(file), None) => format!("in {file}"),
        (None, Some(line)) => format!("at line {line}"),
        (None, None) => "before it".to_owned(),
    }
}

/// Register the default plugin `default` into `ledger` ([`crate::analysis::LoadOptions::default_plugins`]): as a
/// `plugin` directive naming its module would, but whatever the ledger's `features.plugin`, and, when `configurable`,
/// with the metas of `directive`, the first `plugin` directive naming its module, if the ledger has one: then what they
/// ask for and the errors of the plugin go on that directive. An official plugin is not `configurable`: it has no
/// options, so it runs as without its directive's metas, and its errors still go on that directive. What it declares
/// is kept across loads ([`crate::plugin::store::PluginStore::insert_default_plugin`]). The operator's
/// `ZHANG_PLUGINS=off` stops it as it stops every plugin.
#[cfg_attr(not(feature = "plugin_runtime"), allow(unused_variables))]
pub(crate) fn register_default(
    default: &crate::analysis::DefaultPlugin, directive: Option<&(Plugin, SpanInfo)>, configurable: bool, ledger: &mut Ledger,
) -> ZhangResult<()> {
    #[cfg(feature = "plugin_runtime")]
    {
        if !ledger.plugin_policy.loads_plugins() {
            return Ok(());
        }
        let bare = || Plugin {
            module: zhang_ast::ZhangString::quote(default.module.as_str()),
            value: vec![],
            meta: zhang_ast::Meta::default(),
        };
        let plugin = match directive {
            Some((plugin, _)) if configurable => plugin.clone(),
            _ => bare(),
        };
        let span = directive.map(|(_, span)| span.clone()).unwrap_or_default();
        let content = default.content.clone();
        let module = crate::plugin::lua::PluginModule::parse_as(&default.module, &default.file, || Ok(content.to_vec()))?;
        let mut declaration = crate::plugin::capabilities::PluginDeclaration::parse(&plugin);
        if directive.is_some() && configurable {
            for error in &declaration.errors {
                ledger.report(error.kind.clone(), &span, error.metas.clone());
            }
            let timeout_declared = declaration.declares_timeout();
            for reduced in declaration.capabilities.bound_by(&ledger.plugin_policy, timeout_declared) {
                ledger.report(ErrorKind::PluginCapabilityReduced, &span, reduced.metas(&default.module));
            }
        }
        let (clock, timezone) = (ledger.clock.clone(), ledger.options.timezone);
        let files = crate::plugin::files::FileAccess::new(vec![], ledger.data_source.clone(), &ledger.entry.0);
        let (options, policy) = (ledger.options.all(), ledger.plugin_policy.clone());
        ledger
            .plugins
            .insert_default_plugin(&plugin, module, declaration, &span, &clock, timezone, files, &options, policy)?;
    }
    Ok(())
}

/// Report, once, that `ledger` uses what an official default plugin the operator stopped interprets: on its first
/// budget directive, or `open` with `budget` metadata, when `zhang:budget` does not run
/// ([`zhang_ast::error::ErrorKind::ExtensionUnavailable`], with the metas `plugin` and `reason`). The ledger is folded.
pub(crate) fn report_unavailable(ledger: &mut Ledger) {
    let Some(reason) = ledger.extensions.unavailable(crate::analysis::BUDGET).map(str::to_owned) else {
        return;
    };
    let first = ledger.directives.iter().find(|directive| match &directive.data {
        zhang_ast::Directive::Open(open) => open.meta.get_one("budget").is_some(),
        data => matches!(
            data,
            zhang_ast::Directive::Budget(_)
                | zhang_ast::Directive::BudgetAdd(_)
                | zhang_ast::Directive::BudgetTransfer(_)
                | zhang_ast::Directive::BudgetClose(_)
        ),
    });
    let Some(span) = first.map(|directive| directive.span.clone()) else {
        return;
    };
    let metas = std::collections::HashMap::from([("plugin".to_owned(), crate::analysis::BUDGET.to_owned()), ("reason".to_owned(), reason)]);
    ledger.report(zhang_ast::error::ErrorKind::ExtensionUnavailable, &span, metas);
}
