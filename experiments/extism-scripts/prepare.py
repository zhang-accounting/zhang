"""Fetch pinned build dependencies for this macOS arm64 experiment only."""

import argparse
from concurrent.futures import ThreadPoolExecutor
import hashlib
from pathlib import Path
import platform
import subprocess
import urllib.request

ARCHIVES = [
    ("python-pdk", "https://github.com/extism/python-pdk/releases/download/v0.1.5/extism-py-aarch64-macos-v0.1.5.tar.gz",
     "a05dcd25e9c942644e6bd7c36e5cc8b5808d8c7c8aee98604f29ad56d954a7ad"),
    ("binaryen", "https://github.com/WebAssembly/binaryen/releases/download/version_133/binaryen-version_133-arm64-macos.tar.gz",
     "ad66da82ac13f163e424b1643f16c6dfcccc98b5966296b43e52d3cab04f84a8"),
    ("wasi-sdk", "https://github.com/WebAssembly/wasi-sdk/releases/download/wasi-sdk-34/wasi-sdk-34.0-arm64-macos.tar.gz",
     "9c59398106b417f8f14913380fdf0097a8cc0ff4af9eb3ce0065a859e88d49e9"),
    ("lua", "https://api.github.com/repos/andy-emerson/lua.wasm/tarball/0.2.0",
     "afe9c98b4e1e40f25ae80fea268169b6f1431d68ef9d52b68eb08a6ccd6ff62c"),
]


def download(url, destination):
    if destination.exists():
        return
    request = urllib.request.Request(url, headers={"User-Agent": "zhang-script-runtime-spike"})
    temporary = destination.with_suffix(destination.suffix + ".partial")
    with urllib.request.urlopen(request) as source, temporary.open("wb") as target:
        while True:
            chunk = source.read(1024 * 1024)
            if not chunk:
                break
            target.write(chunk)
    temporary.rename(destination)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--tools", type=Path, required=True)
    args = parser.parse_args()
    if (platform.system(), platform.machine()) != ("Darwin", "arm64"):
        parser.error("this experiment pins macOS arm64 tools; choose matching upstream assets for other hosts")
    tools = args.tools.resolve()
    tools.mkdir(parents=True, exist_ok=True)

    def archive(item):
        name, url, digest = item
        path = tools / (name + ".tar.gz")
        download(url, path)
        if hashlib.sha256(path.read_bytes()).hexdigest() != digest:
            raise RuntimeError("archive checksum mismatch: " + name)
        destination = tools / name
        destination.mkdir(exist_ok=True)
        if not (destination / ".ready").exists():
            subprocess.run(["tar", "-xzf", str(path), "-C", str(destination)], check=True)
            (destination / ".ready").touch()
        print(name + " ready", flush=True)

    with ThreadPoolExecutor(max_workers=4) as pool:
        list(pool.map(archive, ARCHIVES))
    download("https://raw.githubusercontent.com/extism/c-pdk/54dbb4096dd07c9bd29bff48668209cbbd01abc7/extism-pdk.h", tools / "extism-pdk.h")
    download("https://dkolf.de/dkjson-lua/dkjson-2.11.lua", tools / "dkjson.lua")
    if hashlib.sha256((tools / "dkjson.lua").read_bytes()).hexdigest() != "197cb50834c642f84b4cf99fe724932c50e6d9c92faec7ad89aa25e91df4d481":
        raise RuntimeError("dkjson source checksum mismatch")


if __name__ == "__main__":
    main()
