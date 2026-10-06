use zhang_ast::{Plugin, SpanInfo};

use crate::ledger::Ledger;
use crate::ZhangResult;

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
            // the module is read from the ledger's source on every load, and handed to the runtime as it is
            let module = plugin.module.as_str();
            let module_bytes = ledger.data_source.get(module.to_owned()).map_err(|error| module_error(module, error))?;
            let declaration = crate::plugin::capabilities::PluginDeclaration::parse(plugin);
            // a meta value the host cannot use is reported on the directive, and the plugin runs with the default
            for error in &declaration.errors {
                ledger.report(error.kind.clone(), span, error.metas.clone());
            }
            let (clock, timezone) = (ledger.clock.clone(), ledger.options.timezone);
            let files = crate::plugin::files::FileAccess::new(declaration.capabilities.allowed_paths.clone(), ledger.data_source.clone(), &ledger.entry.0);
            ledger.plugins.insert_plugin(plugin, module_bytes, declaration, span, &clock, timezone, files)?;
            // a rebuilt local module makes the ledger stale
            if let Some(input) = crate::inputs::ExtraInput::plugin_module(&ledger.entry.0, plugin.module.as_str()) {
                ledger.extra_inputs.insert(input);
            }
        }
    });

    Ok(())
}
