---
title: Installation
description: Run Zhang with Docker, a release binary or a build from source, and the options of the zhang serve command.
sidebar:
  order: 2
---

Zhang is a single program, `zhang`. Its `serve` command loads a ledger and serves the web UI and the HTTP API on one port. Run it with Docker, download a release binary, or build it from source.

## Docker

The image is [`kilerd/zhang`](https://hub.docker.com/r/kilerd/zhang/tags) on Docker Hub, for `linux/amd64` and `linux/arm64`:

- `kilerd/zhang:latest`: the latest release.
- `kilerd/zhang:<version>`, such as `kilerd/zhang:0.2.0`: a specific release.

The image runs `zhang serve /data --port 8000`. Mount the folder that holds your ledger at `/data` and publish port 8000:

```shell
docker run --name zhang -v "/path/to/ledger:/data" -p "8000:8000" kilerd/zhang:latest
```

Open `http://localhost:8000`. Zhang reads `main.zhang` in the mounted folder. If the file does not exist yet, Zhang starts with an empty ledger and creates files when you record something in the web UI.

Arguments after the image name are added to the `zhang serve` command, and environment variables are passed with `-e`. For example, to read a beancount main file and enable the password login:

```shell
docker run --name zhang -v "/path/to/ledger:/data" -p "8000:8000" \
  -e "ZHANG_AUTH=admin:change-me" \
  kilerd/zhang:latest --endpoint main.bean
```

With Docker Compose:

```yaml
services:
  zhang:
    image: kilerd/zhang:latest
    ports:
      - "8000:8000"
    volumes:
      - ./ledger:/data
    environment:
      ZHANG_AUTH: "admin:change-me"
    restart: unless-stopped
```

The image sets `RUST_LOG=info`, so `docker logs zhang` shows what Zhang does, including the reason when the ledger cannot be loaded. It runs Zhang as `root`: files it creates in `/data` belong to `root` on the host (see [file permissions](/deployment/data-sources/local/#file-permissions)).

## Release binaries

Each [release](https://github.com/zhang-accounting/zhang/releases) has binaries named `zhang-<version>-<target>`, with the web UI built in:

| Platform | Target |
| --- | --- |
| Linux, x86-64 | `x86_64-unknown-linux-gnu` |
| Linux, ARM64 | `aarch64-unknown-linux-gnu` |
| macOS, Intel | `x86_64-apple-darwin` |
| macOS, Apple silicon | `aarch64-apple-darwin` |

There is no Windows binary; use Docker there. Download the file for your platform, rename it to `zhang`, make it executable and put it in a folder on your `PATH`:

```shell
mv zhang-*-x86_64-unknown-linux-gnu zhang
chmod +x zhang
sudo mv zhang /usr/local/bin/
zhang serve /path/to/ledger
```

If macOS refuses to open a binary downloaded with a browser, remove the quarantine flag with `xattr -d com.apple.quarantine zhang`.

## Building from source

You need a stable Rust toolchain, Node.js with [pnpm](https://pnpm.io/) 9 for the web UI, and a C compiler, `make` and `perl` (OpenSSL is compiled from source).

```shell
git clone https://github.com/zhang-accounting/zhang.git
cd zhang/frontend
pnpm install
pnpm build
cd ..
cargo build --release --bin zhang --features frontend
```

The binary is `target/release/zhang`. The `frontend` feature embeds the web UI built in `frontend/dist`. Without it, the server answers with a short notice instead of the web UI, and only the HTTP API works.

## `zhang serve`

```shell
zhang serve [OPTIONS] <PATH>
```

| Option | Environment variable | Default | Description |
| --- | --- | --- | --- |
| `<PATH>` | | required | The folder of the ledger. The S3, WebDAV and GitHub data sources ignore it and use their own settings. |
| `-e`, `--endpoint <FILE>` | | `main.zhang` | The main file, relative to `<PATH>`. Its extension selects the format: `.zhang`, or `.bean`, `.beancount` and `.bc` for beancount. Zhang stops with an error on any other extension. |
| `--addr <ADDRESS>` | | `0.0.0.0` | The address to listen on. Use `127.0.0.1` to accept connections from this computer only. |
| `-p`, `--port <PORT>` | | `8000` | The port to listen on. |
| `--auth <USER:PASSWORD>` | `ZHANG_AUTH` | none | Enables the password login. See [Authentication](/deployment/authentication/). |
| `--passkey <SECRET>` | `ZHANG_PASSKEY` | none | Enables the passkey login. The value is the secret needed to register a passkey. See [Authentication](/deployment/authentication/#passkeys). |
| `--source <SOURCE>` | `ZHANG_DATA_SOURCE` | `fs` | Where the ledger is stored: `fs` ([local file system](/deployment/data-sources/local/)), `s3` ([S3](/deployment/data-sources/s3/)), `web-dav` ([WebDAV](/deployment/data-sources/webdav/)) or `github` ([GitHub](/deployment/data-sources/github/)). |
| `--no-report` | | off | Stops the anonymous usage report: once an hour, Zhang sends its version and build date to `https://zhang-cloud.kilerd.me/client_report`. |

A command-line option takes precedence over its environment variable. An `--auth` or `ZHANG_AUTH` value without a colon is ignored, and an unknown `ZHANG_DATA_SOURCE` value falls back to `fs`.

Other environment variables:

| Variable | Description |
| --- | --- |
| `ZHANG_SESSION_SECRET`, `ZHANG_PASSKEY_ORIGIN`, `ZHANG_PASSKEY_RP_ID` | Sessions and passkeys, see [Authentication](/deployment/authentication/). |
| `ZHANG_S3_*`, `ZHANG_WEBDAV_*`, `ZHANG_GITHUB_*` | The settings of the [S3](/deployment/data-sources/s3/), [WebDAV](/deployment/data-sources/webdav/) and [GitHub](/deployment/data-sources/github/) data sources. |
| `ZHANG_QUERY_MAX_RESULT_VALUES` | How many values a [query](/reference/query-language/) result may hold before the query is stopped. Default `1000000`. Lower it on a machine with little memory. |
| `ZHANG_LOG` | The log filter, such as `info`, `debug` or `zhang_core=trace` ([env_logger syntax](https://docs.rs/env_logger/latest/env_logger/#enabling-logging)). When it is not set, `RUST_LOG` is used; without either, the level is `info`. The Docker image sets `RUST_LOG=info`. |

When the ledger cannot be loaded at startup, for example because of a syntax error, or the port is already in use, `zhang serve` prints the reason on stderr and exits with code 1, so systemd, Docker and hosting platforms see a failed start. Problems in the books themselves, such as an unbalanced transaction, do not stop the server: they are listed in the web UI.

## Other commands

- `zhang update` replaces the binary with the latest release for your platform, see [Upgrading](/deployment/upgrading/#release-binaries).
- `zhang parse` and `zhang export` are placeholders: they do not read the ledger yet. To check a ledger, start `zhang serve` and look at the error list in the web UI.

## Next steps

- [Your First Ledger](/getting-started/first-ledger/) writes a small ledger and tours the web UI.
- [Coming from Beancount](/getting-started/from-beancount/) explains how to serve an existing beancount ledger.
- [Authentication](/deployment/authentication/) protects an instance that others can reach.
