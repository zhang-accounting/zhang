# Extism source-script experiment

This experiment builds one reusable Python interpreter plugin and one reusable Lua
interpreter plugin. User business scripts remain source files. The Rust harness
uses Zhang's pinned Extism 1.30.0 and real Zhang host functions; it does not embed a
native Python or Lua interpreter in Zhang. All changes live in this directory.

## Reproduce

The download helper currently pins **macOS arm64** tools. Rust/Cargo and Python 3
are prerequisites. Tools are downloaded into a separate directory; nothing is
installed globally.

```sh
python3 experiments/extism-scripts/prepare.py --tools /private/tmp/zhang-script-runtimes-tools
python3 experiments/extism-scripts/build.py --tools /private/tmp/zhang-script-runtimes-tools
cargo run --manifest-path experiments/extism-scripts/Cargo.toml --features lua-exceptions
```

Generated WASM modules are in `artifacts/`, excluded from Git. The standalone
Cargo package has its own lockfile and does not change the production workspace.

## Where the interpreters come from

- Python: [Extism Python PDK v0.1.5](https://github.com/extism/python-pdk/tree/v0.1.5).
  `extism-py` packages the fixed `python/runtime.py` entry point, including its
  Python engine. That entry point subsequently receives arbitrary script source,
  compiles it to Python bytecode inside the WASM interpreter, and calls the script.
  Zhang's host imports are declared by the fixed entry point, never discovered by
  compiling each business script.
- Lua: [lua.wasm 0.2.0](https://github.com/andy-emerson/lua.wasm/tree/0.2.0),
  [Extism C PDK](https://github.com/extism/c-pdk/tree/54dbb4096dd07c9bd29bff48668209cbbd01abc7),
  and WASI SDK 34. `lua/runtime.cpp` links the Lua interpreter into a WASI reactor
  and implements the Extism entry points and host bridge. Lua scripts are loaded
  with `load` at call time. A pinned copy of
  [dkjson 2.11](https://dkolf.de/dkjson-lua/)
  supplies JSON convenience functions; its license is preserved in the embedded source.
  Its object/array markers and explicit JSON null preserve Zhang's directive
  shapes, including empty metadata maps. A simpler table codec was rejected by
  the real processor test because it turned empty maps into arrays.

The Lua build uses standardized WASM exception handling and LLVM's exception-tag
layout. Its host needs Extism's `wasmtime-exceptions` feature; the experiment
exposes this as `--features lua-exceptions` without changing Zhang's dependency.

The default-feature run was also checked: Python passed its full harness, while
Lua was rejected during module loading with `exceptions proposal not enabled`.

## What the harness checks

For each language it:

1. Runs two different source scripts through the **same WASM instance**, obtaining
   `3` and `30` without rebuilding the interpreter module.
2. Reads the real `zhang_now` host and reports errors through `zhang_emit_error`.
   A separate identity script preserves empty objects/arrays, nulls, false, an
   integer above 2^53, and an exact decimal string through the JSON bridge.
3. Receives a script exception and then successfully calls that instance again.
4. Stops an infinite script loop through Extism's 100 ms timeout.
5. Loads the interpreter as a real Zhang `Processor` and `Router` plugin. The
   processor reads the business script through `zhang_read_file`, respecting
   `allowed_paths`, and records the script in `ledger.extra_inputs`.
6. Executes the source router through `RegisteredPlugin::execute_as_router`. It
   queries a real ledger through Zhang's existing `zhang_query` host bridge and
   produces an expenses report of exactly `12.35` CNY from `12.34 + 0.01`.

The harness prints JSON results, including observed instance/call times and module
sizes. These are smoke-test timings, not a comparative performance benchmark.

## Scope

This is a feasibility experiment, not a production language SDK or a new loader.
The ledger still declares a `.wasm` runtime, with `script` and `allowed_paths`
metadata. There is no `plugin "business.py"` / `plugin "business.lua"` syntax yet.

Processor scripts are read through the current file capability API. Router file
access is deliberately denied by that API, so the harness supplies router source
through configuration for this experiment. A production source loader would need
to resolve and track source independently of whether a plugin is registering,
processing, or routing.

Registration exports currently return fixed runtime names/types. Multiple router
scripts in the same language would therefore share a name; per-script manifests
and identity need to be designed before using this as a general plugin platform.

Python's snapshot includes modules imported by the fixed entry point. Arbitrary
third-party packages and native Python extensions are outside this experiment.
Lua uses a small upstream WASM exception shim; this experiment does not establish
its suitability for a long-running production service.

The demonstration query adapter serializes its string/decimal cells as strings.
It is sufficient for the report above, but is not Zhang server's complete typed
query JSON encoder. Monetary aggregation stays in Zhang's exact-decimal BQL engine.
