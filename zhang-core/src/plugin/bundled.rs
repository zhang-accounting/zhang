//! The plugins built into zhang: `plugin "zhang:<name>"` loads one of them from the binary instead of reading a file
//! of the ledger, so a ledger declares it with nothing to copy, wherever it is stored (a local directory, S3 or R2,
//! WebDAV, GitHub).
//!
//! A bundled plugin is the repository file it is built from, unchanged, and is loaded as a copy of that file declared
//! by its path would be: the file's extension says what it is, it registers under the name and version its own code
//! gives, and its directive still grants its capabilities (`allowed_writes`, `allowed_paths`, ...) and carries its
//! settings; being bundled grants nothing. What differs: nothing is read through the ledger's data source, messages
//! name the module as written (`zhang:mcp`), and the module is not an input of the load, since it changes only with
//! the binary.
//!
//! The `zhang:` prefix is reserved: a module starting with it is never read as a file, and an unknown name is refused
//! listing the bundled plugins. A file of the ledger whose name starts with `zhang:` is declared as `./zhang:...`.

use itertools::Itertools;

use crate::plugin::lua::PluginModule;
use crate::{ZhangError, ZhangResult};

/// the prefix of a module naming a bundled plugin
pub const PREFIX: &str = "zhang:";

/// a plugin built into zhang
#[derive(Debug)]
pub struct BundledPlugin {
    /// what follows `zhang:` in the directive declaring it
    pub name: &'static str,
    /// the file of the repository it is built from; its extension says what it is
    pub file: &'static str,
    /// the file's content
    pub content: &'static [u8],
}

/// every bundled plugin
pub const BUNDLED: &[BundledPlugin] = &[BundledPlugin {
    name: "mcp",
    file: "plugins/mcp/mcp.lua",
    content: include_bytes!("../../../plugins/mcp/mcp.lua"),
}];

impl BundledPlugin {
    /// the bundled plugin the module `module` names: `None` for a module outside the `zhang:` namespace, a file of
    /// the ledger; an error listing the bundled plugins for a name that is none of them
    pub fn lookup(module: &str) -> Option<ZhangResult<&'static BundledPlugin>> {
        let name = module.strip_prefix(PREFIX)?;
        Some(BUNDLED.iter().find(|it| it.name == name).ok_or_else(|| {
            ZhangError::CustomError(format!(
                "plugin module {module} is not a plugin bundled with this zhang; the bundled plugins are {}",
                BUNDLED.iter().map(|it| format!("{PREFIX}{}", it.name)).join(", ")
            ))
        }))
    }

    /// the module the directive declaring this plugin as `module` runs
    pub fn module(&self, module: &str) -> ZhangResult<PluginModule> {
        PluginModule::parse_as(module, self.file, || Ok(self.content.to_vec()))
    }
}

#[cfg(test)]
mod test {
    use super::{BundledPlugin, BUNDLED};
    use crate::plugin::lua::PluginModule;

    #[test]
    fn a_zhang_module_names_a_bundled_plugin_and_anything_else_a_file() {
        let mcp = BundledPlugin::lookup("zhang:mcp").expect("in the namespace").expect("bundled");
        assert_eq!((mcp.name, mcp.file), ("mcp", "plugins/mcp/mcp.lua"));

        for module in [
            "plugins/mcp.lua",
            "zhang-mcp.lua",
            "./zhang:mcp",
            "Zhang:mcp",
            "https://example.com/zhang:mcp.lua",
        ] {
            assert!(BundledPlugin::lookup(module).is_none(), "{module} is a file");
        }
        for module in ["zhang:nope", "zhang:", "zhang:MCP", "zhang:mcp.lua", "zhang:mcp "] {
            let error = BundledPlugin::lookup(module).expect("in the namespace").unwrap_err().to_string();
            assert!(
                error.contains(&format!(
                    "plugin module {module} is not a plugin bundled with this zhang; the bundled plugins are zhang:mcp"
                )),
                "{error}"
            );
        }
    }

    #[test]
    fn a_bundled_plugin_is_its_file_named_as_declared() {
        for bundled in BUNDLED {
            let module = format!("zhang:{}", bundled.name);
            match bundled.module(&module).unwrap() {
                PluginModule::Script(script) => {
                    assert_eq!(script.name, module);
                    assert_eq!(script.source.as_bytes(), bundled.content);
                }
                PluginModule::Wasm(bytes) => assert_eq!(bytes, bundled.content),
            }
        }
        let mcp = BundledPlugin::lookup("zhang:mcp").unwrap().unwrap().module("zhang:mcp").unwrap();
        assert!(matches!(mcp, PluginModule::Script(_)), "zhang:mcp is a Lua script");
    }
}
