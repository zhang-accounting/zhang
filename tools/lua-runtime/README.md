# The Lua interpreter module

`zhang-core/src/plugin/lua/lua-runtime.wasm` is the Lua 5.4.8 interpreter zhang pairs with every `plugin "x.lua"`
directive: an Extism plugin exporting the plugin ABI v1 identity functions (`name`, `version`, `supported_type`) and
`router`, which run the script the host hands over in the config key `zhang.script`. It is vendored in the tree with
its sha256 beside it and embedded in the zhang binary with `include_bytes!` under the `plugin_runtime` feature, so
`cargo install zhang` and the release binaries need nothing but Rust; a user never installs a wasm toolchain.

This directory is everything needed to rebuild it, byte for byte:

| file | role |
| --- | --- |
| `PINS.toml` | every download, with its URL and sha256: wasi-sdk 34.0 (clang 23) for `arm64-macos` and `x86_64-linux`, lua.wasm 0.2.0, dkjson 2.11, the Extism C PDK header (by commit and by content) |
| `prepare.py` | fetches and verifies the pins into a directory outside the repo (`--tools`), picking this host's wasi-sdk (`--host` overrides) |
| `build.py` | the one-command build: compiles `runtime.cpp`, lua.wasm's `onelua.c` and `pdk.c` into the module, writes `<out>.sha256` |
| `runtime.cpp` | the dispatcher: the host-function bridges, the memory budget, the `zhang`/`json` Lua API, the sandbox and the dispatch of each export (sections: imports, memory budget, bridges, prelude, exports) |
| `pdk.c` | the translation unit defining the Extism C PDK |

## Rebuild

Python 3 (standard library only) and network access for the first run:

```sh
python3 -I tools/lua-runtime/prepare.py --tools /path/outside/the/repo
python3 -I tools/lua-runtime/build.py --tools /path/outside/the/repo
```

`build.py` writes `zhang-core/src/plugin/lua/lua-runtime.wasm` and its `.sha256` (`--out` chooses another path) and prints
the size and checksum. The build is deterministic: the same pins and sources give the same bytes on every run, and
`-ffile-prefix-map` keeps the building machine's directories out of the module. Check the vendored module with

```sh
cd zhang-core/src/plugin/lua && shasum -a 256 -c lua-runtime.wasm.sha256
```

## How the build is defined

- **Lua as C++ with wasm exception handling.** lua.wasm's embedding contract (`doc/embedding.md` in its tree):
  `onelua.c` compiled as C++ (`-x c++`) so `LUAI_THROW`/`LUAI_TRY` become `throw`/`catch`, `-fwasm-exceptions` in the
  standardized encoding (`-mllvm -wasm-use-legacy-eh=false`, LLVM 20+), `-nostdlib++` because `onelua.c` carries its own
  `catch(...)`-only EH shim (no distribution ships a wasm-EH libc++abi), `-DMAKE_LIB`, `-fno-strict-aliasing` and WASI's
  emulated signal and process clocks. This is why the host needs Extism's `wasmtime-exceptions` feature, which only
  sets `wasm_exceptions(true)` on the engine.
- **An Extism reactor.** `-mexec-model=reactor`: the host calls the exports, nothing runs at start. The 8 MiB stack is
  lua.wasm's default, which its test suite is witnessed against.
- **No debug sections.** `-Wl,--strip-debug` drops the DWARF the sysroot's prebuilt libc objects carry (each
  wasi-sdk host asset builds its libc with its own paths, which made a macOS and a Linux build differ by a few
  hundred bytes of `.debug_str`/`.debug_line` while every code section matched); the `name` section stays.
- **dkjson embedded.** `build.py` writes dkjson into a generated `json_lua.h` as a C string; the prelude in `runtime.cpp`
  wraps it as the `json` library (shared object/array markers, a fixed `null`, strict decoding). User scripts are never
  built into the module.

## Canonical bytes and CI

CI's `lua-runtime` job (`.github/workflows/build-latest.yml`) rebuilds the module on `ubuntu-22.04` with the pinned
Linux wasi-sdk on every pull request, uploads the rebuilt module as a workflow artifact (always, mismatch included) and
fails unless it matches the vendored checksum. The Linux build is the canonical one: when a module rebuilt on another
host differs from CI's, download the job's artifact, vendor it, run the Lua fixtures (`cargo test -p zhang-core
--features plugin_runtime`) against it and commit. `[artifact]` in `PINS.toml` records which host built the vendored
bytes and whether the Linux build matched.

## Changing the runtime

`runtime.cpp` is the only glue; everything a script can call is defined there. Adding a host function means:

1. importing it in the imports section (`EXTISM_IMPORT_USER`), which makes the host link it into every instance;
2. a bridge in the bridges section and an entry in `BRIDGES`;
3. its `zhang.*` wrapper in the prelude, which unwraps the host's `{"Ok": ...}`/`{"Err": ...}` envelope;
4. a fixture in `zhang-core/tests/plugins/` and a test in `zhang-core/tests/core/wasm_plugins.rs`;
5. `build.py`, then commit the module and its checksum together.

Bumping a pin means changing `PINS.toml`, running `prepare.py` again (it refuses a checksum mismatch) and rebuilding.
