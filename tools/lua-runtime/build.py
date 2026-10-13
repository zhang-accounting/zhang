#!/usr/bin/env python3
"""Build the Lua interpreter module zhang embeds, from the toolchain `prepare.py` fetched.

    python3 tools/lua-runtime/build.py --tools /path/outside/the/repo [--out zhang-core/src/plugin/lua/lua-runtime.wasm]

One command, one artifact: `runtime.cpp` (the Extism dispatcher), lua.wasm's `onelua.c` (Lua 5.4.8 compiled as C++
so its error handling rides wasm exception handling) and `pdk.c` (the Extism C PDK) are compiled with wasi-sdk's
clang into a wasm reactor, with dkjson embedded as a C string. The flags are lua.wasm's documented embedding
contract plus Extism's reactor model; the build is deterministic, so the same inputs give the same bytes on every
host (`-ffile-prefix-map` keeps the directories out of the module). The sha256 and size of the output are printed,
and `<out>.sha256` is written beside it in `sha256sum -c` format.
"""

import argparse
import hashlib
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
# prepare.py sits beside this file; `python3 -I` does not put the script's directory on the path
sys.path.insert(0, str(HERE))
from prepare import host_name, pins  # noqa: E402

REPO = HERE.parent.parent
DEFAULT_OUT = REPO / "zhang-core" / "src" / "plugin" / "lua" / "lua-runtime.wasm"

# the Lua stack: lua.wasm's default, which its test suite is witnessed against
STACK_SIZE = 8 * 1024 * 1024


def run(command):
    print(" ".join(str(part) for part in command), flush=True)
    subprocess.run([str(part) for part in command], check=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--tools", type=Path, required=True, help="the directory prepare.py filled")
    parser.add_argument("--out", type=Path, default=DEFAULT_OUT, help=f"where to write the module (default {DEFAULT_OUT.relative_to(REPO)})")
    parser.add_argument("--host", help="the wasi-sdk of this host under --tools; detected by default")
    parser.add_argument("--keep", action="store_true", help="keep the build directory (printed) instead of removing it")
    args = parser.parse_args()
    tools = args.tools.resolve()
    pinned = pins()
    host = host_name(args.host)

    sdk = tools / "wasi-sdk" / f"wasi-sdk-{pinned['wasi-sdk']['version']}-{host}"
    lua_sources = sorted((tools / "lua").glob("*/src"))
    for required in [sdk / "bin" / "clang", tools / "dkjson.lua", tools / "extism-pdk.h"]:
        if not required.exists():
            raise SystemExit(f"{required} is missing: run prepare.py --tools {tools} first")
    if len(lua_sources) != 1:
        raise SystemExit(f"expected one lua.wasm source tree under {tools / 'lua'}, found {len(lua_sources)}")
    lua = lua_sources[0]

    build = Path(tempfile.mkdtemp(prefix="zhang-lua-runtime-"))
    try:
        # dkjson is embedded as a C string; user scripts are never built into the module
        json_source = (tools / "dkjson.lua").read_text(encoding="utf-8")
        if ')jsonlua"' in json_source:
            raise SystemExit("dkjson.lua contains the raw string delimiter")
        (build / "json_lua.h").write_text('static const char JSON_LUA[] = R"jsonlua(' + json_source + ')jsonlua";\n', encoding="utf-8")

        common = [
            "--target=wasm32-wasip1",
            "--sysroot=" + str(sdk / "share" / "wasi-sysroot"),
            "-O2",
            # no directory of the building machine ends up in the module, so every host builds the same bytes
            f"-ffile-prefix-map={tools}=/tools",
            f"-ffile-prefix-map={HERE}=/src",
            f"-ffile-prefix-map={build}=/build",
        ]
        run([sdk / "bin" / "clang", *common, "-I" + str(tools), "-c", HERE / "pdk.c", "-o", build / "pdk.o"])
        run([
            sdk / "bin" / "clang++",
            *common,
            # lua.wasm's embedding contract (doc/embedding.md): C++ so LUAI_THROW/LUAI_TRY are throw/catch, wasm
            # exception handling in its standardized encoding, no libc++ (onelua.c carries the EH shim), the GC
            # miscompile mitigation, and WASI's emulated signal and clock
            "-fwasm-exceptions",
            "-nostdlib++",
            "-fno-strict-aliasing",
            "-mllvm", "-wasm-use-legacy-eh=false",
            "-D_WASI_EMULATED_SIGNAL",
            "-D_WASI_EMULATED_PROCESS_CLOCKS",
            "-DLUA_USE_JUMPTABLE=0",
            "-DMAKE_LIB",
            "-I" + str(lua),
            "-I" + str(lua / "wasi"),
            "-I" + str(tools),
            "-I" + str(build),
            # an Extism plugin is a reactor: the host calls the exports, nothing runs at start
            "-mexec-model=reactor",
            f"-Wl,-z,stack-size={STACK_SIZE}",
            "-Wl,--export-dynamic",
            # no DWARF: the sysroot's prebuilt libc objects carry debug sections with the build paths of each
            # wasi-sdk host asset, which made the macOS and Linux builds differ (and weighed 380 KiB); the `name`
            # section stays, so a trap still names the function
            "-Wl,--strip-debug",
            "-x", "c++", lua / "onelua.c", HERE / "runtime.cpp",
            "-x", "none", build / "pdk.o",
            "-lwasi-emulated-signal",
            "-lwasi-emulated-process-clocks",
            "-o", build / "lua-runtime.wasm",
        ])

        out = args.out.resolve()
        out.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(build / "lua-runtime.wasm", out)
    finally:
        if args.keep:
            print(f"build directory kept: {build}")
        else:
            shutil.rmtree(build, ignore_errors=True)

    digest = hashlib.sha256(out.read_bytes()).hexdigest()
    (out.parent / (out.name + ".sha256")).write_text(f"{digest}  {out.name}\n", encoding="utf-8")
    print(f"{out}: {out.stat().st_size} bytes, sha256 {digest}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
