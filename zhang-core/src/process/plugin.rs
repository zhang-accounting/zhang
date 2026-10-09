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
            // the module is read from the ledger's source on every load, and handed to the runtime as it is
            let module = plugin.module.as_str();
            let path = module_path(&ledger.entry.0, module)?;
            let module_bytes = ledger.data_source.get(path).map_err(|error| module_error(module, error))?;
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
