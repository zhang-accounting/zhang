<div align="center">
  <img width="256" height="256" src="/docs/src/assets/logo-without-bg.png" />
  <h1>账 Zhang</h1>
  <p>A self-hosted, plain text double-entry accounting tool that speaks beancount.</p>
</div>

![Build](https://img.shields.io/github/actions/workflow/status/zhang-accounting/zhang/build-latest.yml)
![Release](https://img.shields.io/github/v/release/zhang-accounting/zhang)
![Docker Pulls](https://img.shields.io/docker/pulls/kilerd/zhang)
![License](https://img.shields.io/github/license/zhang-accounting/zhang)

[Online Playground](https://zhang-cloud.kilerd.me/playground) · [Documentation](https://zhang-accounting.kilerd.me/) · [Discord](https://discord.gg/EGjwhnV267)

Zhang keeps your books in plain text files that you own, and serves a web UI on top of them: a dashboard,
journals, accounts, commodities, budgets, documents, reports and a query page. Edit the files in your editor or
record transactions in the browser: Zhang writes new entries back to your files and reloads when local files change.

## Quick Start

### Docker

```shell
docker run --name zhang -v "/your/data/path:/data" -p "8000:8000" kilerd/zhang:latest
```

Open <http://localhost:8000>. Zhang reads `main.zhang` from the data directory; pass `--endpoint` to use another
main file.

### Binary

Download a build for Linux or macOS from the
[releases page](https://github.com/zhang-accounting/zhang/releases), then run:

```shell
zhang serve /your/data/path
```

`zhang serve --help` lists the options: `--endpoint`, `--port`, `--addr`, `--source`, `--auth` and `--passkey`.

### Already using beancount?

Point Zhang at your beancount main file. No conversion is needed: files ending in `.bean`, `.beancount` or `.bc`
are read as beancount.

```shell
docker run --name zhang -v "/path/to/your/beancount:/data" -p "8000:8000" kilerd/zhang:latest --endpoint main.bean
```

See [Launching with Beancount Data](https://zhang-accounting.kilerd.me/installation/2-beancount_launch/).

## Features

- **Plain text you own**: directives are independent, so a ledger can be split across files in any order.
  `include` accepts wildcards, such as `include "data/*/*.zhang"`.
- **Beancount compatible**: reads beancount ledgers directly, including `pad` and `balance`, `pushtag`/`poptag`,
  `pushmeta`/`popmeta`, costs and total costs, lot labels and posting metadata.
- **Lots and cost basis**: FIFO, LIFO and STRICT booking, interpolated amounts and exact decimal arithmetic.
  Balance assertions are exact unless you give a tolerance.
- **Query language**: a [BQL-compatible query language](https://zhang-accounting.kilerd.me/user-guide/query-language/)
  (`SELECT`, `BALANCES`, `JOURNAL`) in the web UI and over the API.
- **Budgets and documents**: [zero-based budgets](https://zhang-accounting.kilerd.me/directives/4-budget/) in the
  spirit of YNAB, and receipts or statements attached to transactions and accounts.
- **Plugins**: opt-in WebAssembly plugins that transform or validate the ledger, or serve their own pages and APIs.
  Each plugin gets only the capabilities you grant (network hosts, file paths, a time limit), and there is a Rust
  SDK. See [Writing Plugins](https://zhang-accounting.kilerd.me/developer-guides/plugins/).
- **Your data, anywhere**: local disk, [S3](https://zhang-accounting.kilerd.me/datasources/s3/) (including
  Cloudflare R2), [WebDAV](https://zhang-accounting.kilerd.me/datasources/webdav/) or a
  [GitHub repository](https://zhang-accounting.kilerd.me/datasources/github/) as the data source.
- **Sign-in**: an optional login page with a password and
  [passkeys](https://zhang-accounting.kilerd.me/installation/3-authentication/) (Face ID, Touch ID, Windows Hello,
  security keys).

## Documentation

- [Installation](https://zhang-accounting.kilerd.me/installation/1-installation/) and
  [upgrading](https://zhang-accounting.kilerd.me/installation/4-upgrade/)
- [Authentication](https://zhang-accounting.kilerd.me/installation/3-authentication/)
- [Directives](https://zhang-accounting.kilerd.me/directives/1-options/): options, accounts, commodities, budgets,
  queries and transactions
- [Query language](https://zhang-accounting.kilerd.me/user-guide/query-language/) and
  [error codes](https://zhang-accounting.kilerd.me/user-guide/error-code/)
- [Writing plugins](https://zhang-accounting.kilerd.me/developer-guides/plugins/) and
  [router plugins](https://zhang-accounting.kilerd.me/user-guide/router-plugins/)

## Community and Support

Join the [Discord server](https://discord.gg/EGjwhnV267) for discussions, support and news, or open an
[issue](https://github.com/zhang-accounting/zhang/issues). iOS users can try the latest features through
[TestFlight](https://testflight.apple.com/join/3pm50he2).

[![Discord Banner 2](https://discord.com/api/guilds/1217736070045896704/widget.png?style=banner2)](https://discord.gg/EGjwhnV267)

## Star History

[![Star History Chart](https://api.star-history.com/svg?repos=zhang-accounting/zhang&type=Date)](https://star-history.com/#zhang-accounting/zhang&Date)

## Sponsors

[![Powered by DartNode](https://dartnode.com/branding/DN-Open-Source-sm.png)](https://dartnode.com "Powered by DartNode - Free VPS for Open Source")
