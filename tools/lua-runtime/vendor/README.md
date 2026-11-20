# Vendored sources of the Lua interpreter module

Everything `build.py` compiles besides `runtime.cpp` and `pdk.c`, so the module builds from this repository and the
pinned wasi-sdk alone (`prepare.py` downloads nothing else). `SHA256SUMS` lists every file with its sha256;
`build.py` refuses to build when a file differs from its line or is not listed, so a change here is always a
deliberate one that updates `SHA256SUMS` in the same commit.

| directory | what | from | license |
| --- | --- | --- | --- |
| `lua/` | Lua 5.4.8 (PUC-Rio) as one translation unit, `onelua.c`, with the wasm exception-handling shim and WASI support of lua.wasm, and its `wasi/setjmp.h`: the files `onelua.c` includes with `-DMAKE_LIB` and `-DLUA_USE_JUMPTABLE=0` (`lua.c`, `luac.c`, `ljumptab.h`, `lopnames.h`, `lua.hpp` and the Makefile are left out) | `src/` of lua.wasm 0.2.0, <https://github.com/andy-emerson/lua.wasm> at commit `4858557`, tarball sha256 in `PINS.toml` | MIT: `lua/LICENSE` (lua.wasm); Lua's own MIT notice is at the end of `lua/lua.h` |
| `dkjson/` | dkjson 2.11, the `json` library of every script | <https://dkolf.de/dkjson-lua/>, sha256 in `PINS.toml` | MIT, in the header of `dkjson.lua` |
| `extism-c-pdk/` | the Extism C PDK header | <https://github.com/extism/c-pdk> at the commit in `PINS.toml` | BSD-3-Clause: `extism-c-pdk/LICENSE` |

Every file is byte for byte the upstream file, except the local changes listed here:

- none yet.

To take a new upstream version: replace the files, list the new checksums
(`cd tools/lua-runtime/vendor && find . -type f ! -name SHA256SUMS ! -name README.md | sed 's|^\./||' | LC_ALL=C sort | xargs shasum -a 256 > SHA256SUMS`),
update the provenance in `PINS.toml`, rebuild the module and commit everything together.
