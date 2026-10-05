"""Build fixed Python/Lua Extism modules using an explicitly supplied tools directory."""

import argparse
import os
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parent


def run(command, **kwargs):
    print(" ".join(map(str, command)), flush=True)
    subprocess.run(list(map(str, command)), check=True, **kwargs)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--tools", type=Path, required=True)
    args = parser.parse_args()
    tools = args.tools.resolve()
    artifacts = ROOT / "artifacts"
    artifacts.mkdir(exist_ok=True)
    sdk = tools / "wasi-sdk" / "wasi-sdk-34.0-arm64-macos"
    lua = next((tools / "lua").glob("*/src"))
    binaryen = tools / "binaryen" / "binaryen-version_133" / "bin"
    python_pdk = tools / "python-pdk" / "extism-py" / "bin" / "extism-py"
    # extism-py clears its child's environment, so its env override is lost.
    # Use its supported relative lookup within the experiment's build directory.
    deps_link = artifacts / "lib/target/wasm32-wasi/wasi-deps"
    deps_link.parent.mkdir(parents=True, exist_ok=True)
    if not deps_link.exists():
        deps_link.symlink_to(tools / "python-pdk/extism-py/share/extism-py", target_is_directory=True)
    environment = dict(os.environ, PATH=str(binaryen) + os.pathsep + os.environ["PATH"])
    run([python_pdk, ROOT / "python/runtime.py", "-o", artifacts / "python-runtime.wasm"], env=environment, cwd=artifacts)

    # Only the JSON convenience library is embedded; user scripts are never built into Lua's module.
    json_source = (tools / "dkjson.lua").read_text()
    (artifacts / "json_lua.h").write_text('static const char JSON_LUA[] = R"jsonlua(' + json_source + ')jsonlua";\n')
    common = ["--target=wasm32-wasip1", "--sysroot=" + str(sdk / "share/wasi-sysroot"), "-O2"]
    run([sdk / "bin/clang", *common, "-I" + str(tools), "-c", ROOT / "lua/pdk.c", "-o", artifacts / "pdk.o"])
    run([sdk / "bin/clang++", *common, "-fwasm-exceptions", "-nostdlib++", "-fno-strict-aliasing",
         "-mllvm", "-wasm-use-legacy-eh=false", "-D_WASI_EMULATED_SIGNAL", "-D_WASI_EMULATED_PROCESS_CLOCKS",
         "-DLUA_USE_JUMPTABLE=0", "-DMAKE_LIB", "-I" + str(lua), "-I" + str(lua / "wasi"),
         "-I" + str(tools), "-I" + str(artifacts), "-mexec-model=reactor", "-Wl,-z,stack-size=8388608",
         "-Wl,--export-dynamic", "-x", "c++", lua / "onelua.c", ROOT / "lua/runtime.cpp", "-x", "none", artifacts / "pdk.o",
         "-lwasi-emulated-signal", "-lwasi-emulated-process-clocks", "-o", artifacts / "lua-runtime.wasm"])


if __name__ == "__main__":
    main()
