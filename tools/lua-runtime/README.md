# The Lua interpreter module

`zhang-core/src/plugin/lua/lua-runtime.wasm` is the Lua 5.4.8 interpreter zhang pairs with every `plugin "x.lua"`
directive: an Extism plugin exporting the plugin ABI v1 identity functions (`name`, `version`, `supported_type`),
`capabilities`, `router`, and an analyzer's `manifest` and `analyze`, which run the script the host hands over in the
config key `zhang.script`. It is vendored in the tree with
its sha256 beside it and embedded in the zhang binary with `include_bytes!` under the `plugin_runtime` feature, so
`cargo install zhang` and the release binaries need nothing but Rust; a user never installs a wasm toolchain.

This directory is everything needed to rebuild it, byte for byte:

| file | role |
| --- | --- |
| `vendor/` | the sources, in the tree: Lua 5.4.8 as lua.wasm 0.2.0's `onelua.c` with its wasm shim and the Extism C PDK header, with their licenses, their provenance and any local change (`vendor/README.md`) and the sha256 of every file (`vendor/SHA256SUMS`) |
| `PINS.toml` | the one download, wasi-sdk 34.0 (clang 23) for `arm64-macos` and `x86_64-linux`, with its URL and sha256; and the upstream URL, version and sha256 of each vendored source |
| `prepare.py` | fetches and verifies the wasi-sdk into a directory outside the repo (`--tools`), picking this host's (`--host` overrides) |
| `build.py` | the one-command build: checks `vendor/` against `vendor/SHA256SUMS`, compiles `runtime.cpp`, `json.cpp`, the vendored `onelua.c` and `pdk.c` into the module, writes `<out>.sha256` |
| `runtime.cpp` | the dispatcher: the host-function bridges, the memory budget, the `zhang`/`json` Lua API, the sandbox and the dispatch of each export (sections: imports, memory budget, bridges, prelude, the `zhang.decimal` library, the analyzer's exports, exports) |
| `json.cpp` | the JSON codec of the `json` table, over the Lua C API (see [The JSON codec](#the-json-codec)) |
| `pdk.c` | the translation unit defining the Extism C PDK |
| `seed.h` | the random string-hash seed of every Lua state, included ahead of the C++ sources |

## Rebuild

Python 3 (standard library only) and network access for the first run, which downloads the wasi-sdk; the sources are
in the tree:

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

- **Lua as C++ with wasm exception handling.** lua.wasm's embedding contract (`doc/embedding.md` in its repository):
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
- **A random hash seed.** `seed.h` defines `luai_makeseed` with 32 bits of WASI randomness per Lua state, where Lua's
  default would mix the clock with addresses that are the same on every wasm run, so keys cannot be crafted to
  collide. `pairs` order therefore changes from run to run; `json.cpp` writes an object's keys sorted, so
  identical values still encode to identical JSON.
- **The JSON codec in C.** `json.cpp` is compiled with the rest; the prelude in `runtime.cpp` exposes it as the `json`
  library (shared object/array markers, a fixed `null`). User scripts are never built into the module.
- **Three chunks of Lua, two of them read only when needed.** Every call parses the prelude again, in a fresh state,
  at about a microsecond a line, so what most calls do not use is a chunk of its own: the `zhang.decimal` library
  (`DECIMAL_LIBRARY`), read the first time a script uses `zhang.decimal`, and the code of the analyzer's exports
  (`ANALYZER`), read by `manifest` and `analyze` only. A call that uses neither, a router request, costs about 0.1 ms
  more than before analyzers and decimals (0.88 ms against 0.80 ms for a script that does nothing, the instance
  included). Those chunks run in an environment of the library functions the prelude took before the script ran, and
  the script's `json` is a copy of the codec's table, so a script that replaces `string.format` or `json.object`
  changes its own calls only.

## Canonical bytes and CI

CI's `lua-runtime` job (`.github/workflows/build-latest.yml`) rebuilds the module on `ubuntu-22.04` with the pinned
Linux wasi-sdk on every pull request, uploads the rebuilt module as a workflow artifact (always, mismatch included) and
fails unless it matches the vendored checksum. The Linux build is the canonical one: when a module rebuilt on another
host differs from CI's, download the job's artifact, vendor it, run the Lua fixtures (`cargo test -p zhang-core
--features plugin_runtime`) against it and commit. `[artifact]` in `PINS.toml` records which host built the vendored
bytes and whether the Linux build matched.

## The JSON codec

`json.cpp` is the `json` table of every script: about 900 lines of C++ over the Lua C API, written for this runtime.
It replaced dkjson 2.11 (pure Lua, about 3 MB/s, patched here to sort keys) and keeps its rules; the Lua plugins guide
(`docs/src/content/docs/guides/lua-plugins.md`, "JSON") is the contract. Why not a library:

- **lua-cjson** (MIT) would need patches to most of its encoder and part of its decoder to keep the rules: sorted
  keys, `[]` for an empty table, sparse arrays as objects, a table as `json.null`, the object marker, errors with a
  line and a column, UTF-8 checks (it passes bytes through both ways) and round-trip float digits; and it allocates
  with `malloc`, outside the memory budget.
- **yyjson** and **cJSON** (MIT) parse into a tree of their own, which a Lua table would copy: 2–4 times the text in
  memory on top of the result, under a 64 MiB budget. cJSON keeps numbers as doubles, so 64-bit integers lose digits.

A `zhang.decimal` value (a table whose metatable's `__jsontype` is `"decimal"`, read raw) is written as the string
of its `value` field, so a script puts decimal values into JSON as it would strings. The codec reads and writes in
one pass, straight between the text and Lua values, with an explicit stack instead of
recursion (1000 levels at most, so a deep document grows neither the C nor the wasm stack). Its buffers are Lua
userdata, so the budget counts them and refuses them with Lua's own memory error, and the collector is paused while
it runs, so no finalizer changes a table it is writing. The encoder points into the string keys of the table it
writes; a growth the budget refuses still runs Lua's emergency collection, which clears a weak table's garbage
values and frees their keys, so a weak table (a string `__mode`) is written from a strong copy. The object and array
markers are sealed with `__metatable` (`getmetatable` gives a read-only look-alike), so no script can make every decoded
document weak through them, nor `setmetatable` a marked table (a plugin copies decoded data to attach methods). A table holding a number key and the same key as a string raises rather than writing the key
twice. Numbers: an integer within 64 bits is exact both ways; a float is decoded with Clinger's exact fast path or
wasi-libc's `strtod`, and written with `%.15g`, `%.16g` or `%.17g`, the first that reads back as the same float (the
shortest form for a normal float; a subnormal may get more digits than it needs).

Before it replaced dkjson it was checked natively against JSONTestSuite (every `y_` document accepted, every `n_`
refused) and against the patched dkjson on random documents (byte-identical output wherever the rules did not
change); a later review fuzzed it under ASan and UBSan and compared it with Python's `json`. `zhang-core/tests/plugins/json_rules.lua` holds its rules as checks that run inside the module; change it with
the codec.

## Exact decimals

`zhang.decimal` (`DECIMAL_LIBRARY` in `runtime.cpp`) is exact addition, subtraction and comparison of decimals in
pure Lua: a value keeps its signed coefficient as a Lua integer while it has at most 18 digits, and its digits as a
string past that, added 18 at a time; a sum keeps the larger scale of the two, as `BigDecimal`'s `a + b` and `&a - &b`
do, and a value's text is `plain_decimal`'s. There is no multiplication and no division, whose rounding and scale rules
stay in the host (RFC #698 §4, decision 5: no libmpdec, which rounds every operation). The codec writes a value as its
text (`__jsontype = "decimal"`). `zhang-core/tests/core/wasm_plugins/lua/decimal.rs` compares every operation with
`BigDecimal` on 12,000 random pairs (signs, scales up to the 1000 digits a value may have, `-0`, trailing and leading
zeros, `.5` and `5.`, sums that cancel) byte for byte, and its ignored `the_cost_of_zhang_decimal` prints what an
operation costs next to a host call (`cargo test --release -p zhang-core --features plugin_runtime --test core --
--ignored --nocapture the_cost_of`): about 1 µs an addition of amounts, 0.3 µs a comparison, 4 µs an addition of
40-digit numbers, against 7 to 9 µs a crossing to the host and back. A batched host operation would win only for
batches of more than about a dozen independent sums; a budget's running balances are sequential, and the sums that
batch are the host's own (`sum` in a query).

## Changing the runtime

`runtime.cpp` is the only glue; everything a script can call is defined there. Adding a host function means:

1. importing it in the imports section (`EXTISM_IMPORT_USER`), which makes the host link it into every instance;
2. a bridge in the bridges section and an entry in `BRIDGES`;
3. its `zhang.*` wrapper in the prelude, which unwraps the host's `{"Ok": ...}`/`{"Err": ...}` envelope, and
   `api_version` in the prelude raised by one, with the new version listed in the Lua plugins guide
   (`docs/src/content/docs/guides/lua-plugins.md` and its zh-cn mirror), unless no release has shipped the current
   version yet (version 1 is the API of the first release with Lua scripts, routers, analyzers and `zhang.decimal`
   included);
4. a fixture in `zhang-core/tests/plugins/` and a test in `zhang-core/tests/core/wasm_plugins.rs`;
5. `build.py`, then commit the module and its checksum together.

Bumping the wasi-sdk means changing `PINS.toml`, running `prepare.py` again (it refuses a checksum mismatch) and
rebuilding. Taking a new version of a vendored source means replacing its files under `vendor/`, updating
`vendor/SHA256SUMS` and its provenance in `PINS.toml`, and rebuilding (`vendor/README.md`).
