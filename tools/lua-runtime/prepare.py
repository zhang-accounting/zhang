#!/usr/bin/env python3
"""Download and verify the pinned toolchain and sources of the Lua interpreter module (see PINS.toml).

    python3 tools/lua-runtime/prepare.py --tools /path/outside/the/repo

Everything lands under `--tools`: the wasi-sdk for this host (`--host` overrides the detection), the lua.wasm
tarball, dkjson and the Extism C PDK header. Every download is checked against the sha256 of PINS.toml before it
is extracted or used, and a file already present with the right checksum is not downloaded again. Python 3
standard library only.
"""

import argparse
import hashlib
import platform
import re
import sys
import tarfile
import urllib.request
from pathlib import Path

HERE = Path(__file__).resolve().parent
PINS = HERE / "PINS.toml"

# the wasi-sdk release assets this build is pinned for, keyed by the host that runs the build
HOSTS = {
    ("Darwin", "arm64"): "arm64-macos",
    ("Linux", "x86_64"): "x86_64-linux",
}


def pins(path=PINS):
    """the pins as `{section: {key: value}}`. PINS.toml only holds `key = "value"` strings under `[section]`
    headers, so this reads it without `tomllib`, which Python 3.9 and 3.10 lack."""
    sections, current = {}, None
    for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        header = re.fullmatch(r"\[([A-Za-z0-9_.-]+)\]", line)
        if header:
            current = sections.setdefault(header.group(1), {})
            continue
        pair = re.fullmatch(r'([A-Za-z0-9_.-]+)\s*=\s*"([^"]*)"', line)
        if pair and current is not None:
            current[pair.group(1)] = pair.group(2)
            continue
        raise SystemExit(f"{path}:{number}: cannot read the pin line {line!r}")
    return sections


def host_name(override=None):
    """the wasi-sdk asset name of this host, e.g. `arm64-macos`"""
    if override:
        return override
    key = (platform.system(), platform.machine())
    if key not in HOSTS:
        raise SystemExit(f"no wasi-sdk is pinned for {key[0]} {key[1]}; pass --host with one of {sorted(HOSTS.values())}")
    return HOSTS[key]


def sha256(path):
    digest = hashlib.sha256()
    with path.open("rb") as file:
        for chunk in iter(lambda: file.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def fetch(url, destination, expected):
    """download `url` to `destination` unless it is already there with the sha256 `expected`; refuse a mismatch"""
    if destination.exists() and sha256(destination) == expected:
        print(f"{destination.name}: present, checksum ok", flush=True)
        return
    print(f"{destination.name}: downloading {url}", flush=True)
    request = urllib.request.Request(url, headers={"User-Agent": "zhang-lua-runtime-prepare"})
    partial = destination.with_name(destination.name + ".partial")
    with urllib.request.urlopen(request) as source, partial.open("wb") as target:
        for chunk in iter(lambda: source.read(1 << 20), b""):
            target.write(chunk)
    actual = sha256(partial)
    if actual != expected:
        partial.unlink()
        raise SystemExit(f"{destination.name}: sha256 {actual} does not match the pin {expected}; not using it")
    partial.replace(destination)
    print(f"{destination.name}: checksum ok", flush=True)


def extract(archive, directory):
    """extract the verified tarball `archive` into `directory`, once (a `.ready` marker records it)"""
    marker = directory / ".ready"
    if marker.exists():
        return
    directory.mkdir(parents=True, exist_ok=True)
    with tarfile.open(archive) as tar:
        # a downloaded archive is untrusted data: refuse members that would land outside `directory`
        if hasattr(tarfile, "data_filter"):
            tar.extractall(directory, filter="data")
        else:
            for member in tar.getmembers():
                target = (directory / member.name).resolve()
                if directory.resolve() not in target.parents and target != directory.resolve():
                    raise SystemExit(f"{archive.name}: refusing the member {member.name}, outside {directory}")
            tar.extractall(directory)
    marker.touch()


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--tools", type=Path, required=True, help="where the toolchain and sources are kept (outside the repo)")
    parser.add_argument("--host", choices=sorted(HOSTS.values()), help="the wasi-sdk asset to fetch; detected from this machine by default")
    args = parser.parse_args()
    tools = args.tools.resolve()
    tools.mkdir(parents=True, exist_ok=True)
    pinned = pins()
    host = host_name(args.host)

    sdk = pinned["wasi-sdk"]
    sdk_archive = tools / f"wasi-sdk-{sdk['version']}-{host}.tar.gz"
    fetch(sdk[f"{host}-url"], sdk_archive, sdk[f"{host}-sha256"])
    extract(sdk_archive, tools / "wasi-sdk")

    lua = pinned["lua"]
    lua_archive = tools / f"lua.wasm-{lua['version']}.tar.gz"
    fetch(lua["url"], lua_archive, lua["sha256"])
    extract(lua_archive, tools / "lua")

    fetch(pinned["dkjson"]["url"], tools / "dkjson.lua", pinned["dkjson"]["sha256"])
    fetch(pinned["extism-c-pdk"]["url"], tools / "extism-pdk.h", pinned["extism-c-pdk"]["sha256"])
    print(f"ready: {tools} ({host})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
