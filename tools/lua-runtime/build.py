#!/usr/bin/env python3
"""Build the Lua interpreter module zhang embeds, from the vendored sources and the toolchain `prepare.py` fetched.

    python3 tools/lua-runtime/build.py --tools /path/outside/the/repo [--out zhang-core/src/plugin/lua/lua-runtime.wasm]

One command, one artifact: `runtime.cpp` (the Extism dispatcher), the vendored `onelua.c` of lua.wasm (Lua 5.4.8
compiled as C++ so its error handling rides wasm exception handling) and `pdk.c` (the Extism C PDK) are compiled
with wasi-sdk's clang into a wasm reactor, with the vendored dkjson embedded as a C string. Every vendored file is
checked against `vendor/SHA256SUMS` first, and the build refuses a file that differs or is not listed. The flags are lua.wasm's documented embedding
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
from prepare import host_name, pins, sha256  # noqa: E402

REPO = HERE.parent.parent
VENDOR = HERE / "vendor"
DEFAULT_OUT = REPO / "zhang-core" / "src" / "plugin" / "lua" / "lua-runtime.wasm"

# the Lua stack: lua.wasm's default, which its test suite is witnessed against
STACK_SIZE = 8 * 1024 * 1024


def check_vendored():
    """refuse to build unless every file under vendor/ is listed in vendor/SHA256SUMS with its sha256"""
    listed = {}
    for line in (VENDOR / "SHA256SUMS").read_text(encoding="utf-8").splitlines():
        digest, name = line.split(maxsplit=1)
        listed[name.lstrip("*")] = digest
    present = sorted(
        path.relative_to(VENDOR).as_posix()
        for path in VENDOR.rglob("*")
        if path.is_file() and path.name not in ("SHA256SUMS", "README.md")
    )
    unlisted = [name for name in present if name not in listed]
    missing = [name for name in listed if not (VENDOR / name).is_file()]
    changed = [name for name, digest in listed.items() if (VENDOR / name).is_file() and sha256(VENDOR / name) != digest]
    if unlisted or missing or changed:
        raise SystemExit(
            "the vendored sources do not match vendor/SHA256SUMS: "
            f"not listed {unlisted}, missing {missing}, changed {changed}. A deliberate change of a vendored file "
            "updates its line there (see vendor/README.md)"
        )


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
    if not (sdk / "bin" / "clang").exists():
        raise SystemExit(f"{sdk / 'bin' / 'clang'} is missing: run prepare.py --tools {tools} first")
    check_vendored()
    lua = VENDOR / "lua"
    pdk = VENDOR / "extism-c-pdk"

    build = Path(tempfile.mkdtemp(prefix="zhang-lua-runtime-"))
    try:
        # dkjson is embedded as a C string; user scripts are never built into the module
        json_source = (VENDOR / "dkjson" / "dkjson.lua").read_text(encoding="utf-8")
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
        run([sdk / "bin" / "clang", *common, "-I" + str(pdk), "-c", HERE / "pdk.c", "-o", build / "pdk.o"])
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
            # a random string-hash seed for every Lua state (seed.h), against hash flooding; JSON stays
            # byte-identical for identical values because dkjson writes an object's keys sorted
            "-include", HERE / "seed.h",
            "-I" + str(lua),
            "-I" + str(lua / "wasi"),
            "-I" + str(pdk),
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
