//! Lua script plugins: a `plugin "x.lua"` directive runs a Lua script in the interpreter module zhang embeds.
//!
//! The only wasm involved is [`INTERPRETER`], a Lua 5.4.8 interpreter built reproducibly by `tools/lua-runtime`
//! (its sha256 is vendored beside it and CI rebuilds it). The host pairs it with the script: the manifest of a
//! script plugin names the interpreter as its module and sets two reserved config keys, [`SCRIPT_CONFIG_KEY`]
//! (`zhang.script`, the source) and [`SCRIPT_NAME_CONFIG_KEY`] (`zhang.script_name`, the module path as written,
//! for messages). The interpreter exports the plugin ABI v1 identity functions and `router`, which run the script:
//! the chunk returns a module table `{name, version, types, router}`, so a script's name and version are its own.
//! Everything else (registration, the first-declared-wins rule, `GET /api/plugins`, the timeout, the host
//! functions) is the wasm path, unchanged.
//!
//! A module is a script by its extension: `.lua`, in any letter case. `.wasm` and `.wat` are wasm modules, and
//! any other extension is refused with an error naming the three, so a typo never reaches extism as a module.
//! A script is validated here before the interpreter sees it: UTF-8 (the error names the first bad byte) and at
//! most [`MAX_SCRIPT_LEN`]; its syntax is checked when registration runs the chunk through the interpreter's
//! `name` export, where a Lua syntax error becomes a load error carrying `<script>:<line>: ...`.
//!
//! Lua scripts are router plugins only in this round: the interpreter refuses `Processor` and `Mapper` in a
//! script's `types`, because the directive stream carries amounts as bare JSON numbers and Lua would read them as
//! floats, which could change a balance silently.

pub use zhang_shared::plugin_abi::config::{SCRIPT as SCRIPT_CONFIG_KEY, SCRIPT_NAME as SCRIPT_NAME_CONFIG_KEY};

use crate::{ZhangError, ZhangResult};

/// the Lua interpreter module every script runs in, built by `tools/lua-runtime` (see its README and PINS.toml)
pub const INTERPRETER: &[u8] = include_bytes!("lua-runtime.wasm");

/// the longest script the host loads
pub const MAX_SCRIPT_LEN: usize = 1024 * 1024;

/// the module of a `plugin` directive, as the host runs it
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginModule {
    /// a wasm or wat module, run as it is
    Wasm(Vec<u8>),
    /// a Lua script, run by the embedded interpreter
    Script(LuaScript),
}

/// a Lua script plugin's source and the module path it was declared with
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LuaScript {
    /// the module as written in the directive, which messages name
    pub name: String,
    /// the script's text
    pub source: String,
}

/// what the extension of a module says it is
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ModuleKind {
    Wasm,
    Lua,
}

impl PluginModule {
    /// the module `module`, as written in its directive, read with `fetch`. The extension decides what it is
    /// before anything is read, so a wrong extension is refused by name even when no such file exists
    pub fn parse(module: &str, fetch: impl FnOnce() -> ZhangResult<Vec<u8>>) -> ZhangResult<PluginModule> {
        let kind = module_kind(module)?;
        let bytes = fetch()?;
        match kind {
            ModuleKind::Wasm => Ok(PluginModule::Wasm(bytes)),
            ModuleKind::Lua => LuaScript::parse(module, bytes).map(PluginModule::Script),
        }
    }

    /// what the runtime gets: the wasm module to compile (the interpreter for a script) and the script, if any
    pub fn into_parts(self) -> (Vec<u8>, Option<LuaScript>) {
        match self {
            PluginModule::Wasm(bytes) => (bytes, None),
            PluginModule::Script(script) => (INTERPRETER.to_vec(), Some(script)),
        }
    }
}

impl LuaScript {
    /// the config the host sets for the script under the reserved `zhang.` prefix, which pairs the interpreter
    /// with it: part of every manifest of the plugin, so the script is in the compile cache key
    pub fn config(&self) -> Vec<(String, String)> {
        vec![
            (SCRIPT_CONFIG_KEY.to_owned(), self.source.clone()),
            (SCRIPT_NAME_CONFIG_KEY.to_owned(), self.name.clone()),
        ]
    }

    /// the script `module` whose bytes are `bytes`: at most [`MAX_SCRIPT_LEN`] bytes of UTF-8 text, without a
    /// byte order mark (Lua's loader does not skip one, and an editor may write one)
    fn parse(module: &str, bytes: Vec<u8>) -> ZhangResult<LuaScript> {
        if bytes.len() > MAX_SCRIPT_LEN {
            return Err(ZhangError::CustomError(format!(
                "plugin script {module} is {} bytes, over the {MAX_SCRIPT_LEN} byte limit",
                bytes.len()
            )));
        }
        let text = String::from_utf8(bytes).map_err(|e| {
            ZhangError::CustomError(format!(
                "plugin script {module} is not UTF-8 text: the byte at offset {} is not UTF-8. Save the file with the UTF-8 encoding",
                e.utf8_error().valid_up_to()
            ))
        })?;
        let source = text.strip_prefix('\u{feff}').map(str::to_owned).unwrap_or(text);
        Ok(LuaScript {
            name: module.to_owned(),
            source,
        })
    }
}

/// what the module `module` is by its extension, compared in any letter case: `.lua` is a script, `.wasm` and
/// `.wat` are wasm modules, anything else is refused. For a URL the query and fragment are not part of the name
fn module_kind(module: &str) -> ZhangResult<ModuleKind> {
    let path = match module.contains("://") {
        true => module.split(['?', '#']).next().unwrap_or(module),
        false => module,
    };
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    let extension = name.rsplit_once('.').map(|(_, extension)| extension.to_ascii_lowercase());
    match extension.as_deref() {
        Some("lua") => Ok(ModuleKind::Lua),
        Some("wasm") | Some("wat") => Ok(ModuleKind::Wasm),
        _ => Err(ZhangError::CustomError(format!(
            "plugin module {module} is not a `.wasm`/`.wat`/`.lua` module: a plugin is a wasm module (`.wasm`, or `.wat` text) or a Lua script (`.lua`)"
        ))),
    }
}

#[cfg(test)]
mod test {
    use super::{module_kind, LuaScript, ModuleKind, PluginModule, INTERPRETER, MAX_SCRIPT_LEN};
    use crate::ZhangResult;

    #[test]
    fn the_embedded_interpreter_is_a_wasm_module() {
        assert_eq!(&INTERPRETER[..4], b"\0asm");
        assert!(INTERPRETER.len() > 300_000, "{} bytes", INTERPRETER.len());
    }

    #[test]
    fn the_extension_decides_the_kind_in_any_case() {
        for module in [
            "plugins/report.lua",
            "REPORT.LUA",
            "./a/b.Lua",
            "https://example.com/x.lua?v=2#top",
            "s3://bucket/dir/x.lua",
        ] {
            assert_eq!(module_kind(module).unwrap(), ModuleKind::Lua, "{module}");
        }
        for module in ["plugins/echo.wasm", "echo.WAT", "https://example.com/plugin.wasm?token=1", "dir.lua/echo.wasm"] {
            assert_eq!(module_kind(module).unwrap(), ModuleKind::Wasm, "{module}");
        }
    }

    #[test]
    fn another_extension_is_refused_naming_the_three() {
        for module in ["plugins/x.txt", "plugins/x", "x.lua.bak", "plugins/", "lua"] {
            let error = module_kind(module).unwrap_err().to_string();
            assert!(
                error.contains(&format!("plugin module {module} is not a `.wasm`/`.wat`/`.lua` module")),
                "{error}"
            );
        }
    }

    fn parsed(module: &str, bytes: &[u8]) -> ZhangResult<PluginModule> {
        PluginModule::parse(module, || Ok(bytes.to_vec()))
    }

    #[test]
    fn a_script_is_utf8_text_without_a_bom_under_the_size_limit() {
        let script = parsed("plugins/x.lua", b"\xEF\xBB\xBFreturn {}").unwrap();
        let expected = LuaScript {
            name: "plugins/x.lua".to_owned(),
            source: "return {}".to_owned(),
        };
        assert_eq!(script, PluginModule::Script(expected.clone()));
        assert_eq!(
            expected.config(),
            vec![
                ("zhang.script".to_owned(), "return {}".to_owned()),
                ("zhang.script_name".to_owned(), "plugins/x.lua".to_owned())
            ]
        );
        let (wasm, script) = script.into_parts();
        assert_eq!((wasm.as_slice(), script), (INTERPRETER, Some(expected)));

        let error = parsed("plugins/x.lua", b"-- latin1: \xE9\nreturn {}").unwrap_err().to_string();
        assert!(
            error.ends_with("plugin script plugins/x.lua is not UTF-8 text: the byte at offset 11 is not UTF-8. Save the file with the UTF-8 encoding"),
            "{error}"
        );

        let error = parsed("plugins/x.lua", &vec![b' '; MAX_SCRIPT_LEN + 1]).unwrap_err().to_string();
        assert!(
            error.ends_with("plugin script plugins/x.lua is 1048577 bytes, over the 1048576 byte limit"),
            "{error}"
        );
        assert!(parsed("plugins/x.lua", &vec![b' '; MAX_SCRIPT_LEN]).is_ok(), "exactly the limit loads");
    }

    #[test]
    fn a_wasm_module_is_passed_through_and_a_wrong_extension_is_refused_before_reading() {
        let module = parsed("echo.wasm", b"\0asm").unwrap();
        assert_eq!(module, PluginModule::Wasm(b"\0asm".to_vec()));
        assert_eq!(module.into_parts(), (b"\0asm".to_vec(), None));

        let error = PluginModule::parse("plugins/x.txt", || panic!("not read")).unwrap_err().to_string();
        assert!(error.contains("plugin module plugins/x.txt is not a `.wasm`/`.wat`/`.lua` module"), "{error}");
        let error = PluginModule::parse("missing.lua", || Err(crate::ZhangError::FileNotFound)).unwrap_err();
        assert!(error.is_file_not_found());
    }
}
