---
title: Project Structure
description: The crates and folders of the Zhang repository, how a ledger flows through them, and how to build, run and test Zhang locally.
sidebar:
  order: 2
---

Zhang is a Rust workspace, a React web UI and this documentation site, all in the [zhang-accounting/zhang](https://github.com/zhang-accounting/zhang) repository. This page is a map for contributors.

## Repository layout

| Path | Crate | What it does |
| --- | --- | --- |
| `zhang-ast/` | `zhang-ast` | The directive types shared by every crate (`Directive`, `Transaction`, `Posting`, `Account`, amounts, dates), and `ErrorKind`, the kinds of ledger errors. |
| `zhang-core/` | `zhang-core` | Loading a ledger: the Zhang text parser and exporter (`src/data_type/text/`), the `DataSource` trait, options, the processing pipeline (`src/pipeline/`), booking, the in-memory store, and the WASM plugin runtime (`src/plugin/`, feature `plugin_runtime`, built on [Extism](https://extism.org/)). |
| `extensions/beancount/` | `beancount` | The beancount parser and exporter, used for main files ending in `.bean`, `.beancount` or `.bc`. |
| `zhang-query/` | `zhang-query` | The BQL-compatible query engine, which evaluates queries over the in-memory store. |
| `zhang-server/` | `zhang-server` | The HTTP API (axum and gotcha, which also generates the OpenAPI description), authentication (`src/auth/`), file watching and reloads (`src/watch.rs`), and the web UI, embedded from `frontend/dist` with the `frontend` feature. |
| `zhang-cli/` | `zhang` | The `zhang` binary (`src/main.rs`) and the data source for the local file system, S3, WebDAV and GitHub, built on [Apache OpenDAL](https://opendal.apache.org/) (`src/opendal.rs`). |
| `zhang-plugin-sdk/` | `zhang-plugin-sdk` | The Rust SDK for WASM plugins, with two example plugins in `examples/`. See [Writing Plugins](/developers/writing-plugins/). |
| `bindings/wasm/` | `zhang-wasm` | The parsers compiled to WebAssembly with wasm-pack, for the online playground. |
| `frontend/` | | The web UI: React, TypeScript, Vite and Tailwind CSS. |
| `docs/` | | This site, built with Astro and Starlight. See `docs/README.md`. |
| `integration-tests/` | | End-to-end test cases: one folder per case, with a ledger and the API responses it must produce. |
| `examples/` | | An example ledger, `examples/main.zhang`. |

## How a ledger flows through the code

1. **Read.** `zhang serve` builds the data source for the chosen backend (`zhang-cli/src/opendal.rs`) and calls `Ledger::load` (`zhang-core/src/ledger.rs`) on a blocking thread. The data source reads the main file, parses it with the Zhang parser or the beancount parser, and follows the `include` directives, wildcards included. The result is a list of directives (`zhang-ast`) with the file and position each one comes from.
2. **Options and plugins.** The options are applied first. Then the modules of the `plugin` directives are fetched and registered (`zhang-core/src/plugin/`).
3. **Pipeline.** The directives are sorted by date and go through the stages of `zhang-core/src/pipeline/`, in order: the WASM plugins (processors and mappers) in the order they are declared, then the built-in stages `ActiveAccounts`, `Pad` and `BalanceCheck`. Every stage sees the whole stream, may change it, and may report errors.
4. **Store.** The result is folded into the in-memory store (`zhang-core/src/store/`), directive by directive (`zhang-core/src/process/`). Postings are booked against lots by the booker (`zhang-core/src/booking/`), and every problem becomes an error with an `ErrorKind`.
5. **Query and serve.** `zhang-query` runs queries over the store. `zhang-server` exposes the ledger as the HTTP API under `/api/` (handlers in `zhang-server/src/routes/`), with its OpenAPI description at `/openapi.json`, routes requests to router plugins, and reloads the ledger when its files change.
6. **Web UI.** The React app in `frontend/` calls the API through a typed client generated from the OpenAPI description (`frontend/src/api/schemas.ts`), and listens to `/api/sse` to refresh after a reload.

Writes go the other way: a route builds a directive, the data type exports it as text (Zhang or beancount syntax), and the data source appends it to the file that the `directive_output_path` option names, then the ledger reloads.

## Building and running locally

You need a stable Rust toolchain and Node.js 22.22 or newer with [pnpm](https://pnpm.io/) 9. The plugin SDK tests also need the `wasm32-unknown-unknown` target (`rustup target add wasm32-unknown-unknown`); without it they are skipped locally.

The `frontend` feature of `zhang-server` embeds `frontend/dist`, which must exist when the feature is on. Like CI, create an empty folder if you have not built the web UI:

```shell
mkdir -p frontend/dist
```

To work on the server, run it on the example ledger. Without the `frontend` feature it serves the API only:

```shell
cargo run -p zhang -- serve examples --no-report
```

To work on the web UI, start the server as above on port 8000, then the Vite dev server, which proxies `/api` to `http://localhost:8000` (set `VITE_API_ENDPOINT` to use another address):

```shell
cd frontend
pnpm install
pnpm dev
```

and open `http://localhost:3000`.

To build a release binary with the web UI embedded, as the release workflow does:

```shell
cd frontend && pnpm install && pnpm build && cd ..
cargo build --release --bin zhang --features frontend
```

## Tests and checks

CI (`.github/workflows/build-latest.yml`) runs these on every pull request:

| Check | Command |
| --- | --- |
| Rust tests | `cargo test`, and `cargo test -p zhang-core` (without the plugin runtime) |
| Formatting | `cargo +nightly fmt --all -- --check` |
| Lints | `cargo clippy --all-features --all-targets -- -D warnings -D clippy::dbg_macro -A clippy::empty_docs`, and the same for `--target wasm32-unknown-unknown -p zhang-plugin-sdk -p zhang-plugin-example-guard -p zhang-plugin-example-summary` |
| WebAssembly bindings | `wasm-pack build` in `bindings/wasm` |
| Web UI | `pnpm run prettier:check` and `pnpm build` in `frontend` |
| Spelling | [typos](https://github.com/crate-ci/typos), configured in `_typos.toml` |

`cargo test` also runs the end-to-end cases of `integration-tests/`. Each folder holds a `main.zhang` or `main.bean` and a `validations.json`: a list of API URIs, each with JSONPath expressions and the values they must return. The web UI has unit tests too: `pnpm test` in `frontend`.

The documentation is built and deployed only on pushes to `main` and `develop`, not on pull requests. Build it yourself before you send a change to `docs/`, see `docs/README.md`.

## Where to start

- **Syntax**: the Zhang parser and exporter in `zhang-core/src/data_type/text/`, the beancount ones in `extensions/beancount/`.
- **A new check or error**: a variant of `ErrorKind` in `zhang-ast/src/error.rs`, reported from a pipeline stage or a `process` handler in `zhang-core`. Add its message to `ERROR` in `frontend/public/locales/*/translation.json` and a section to [Error Codes](/reference/error-codes/).
- **An API endpoint**: a handler in `zhang-server/src/routes/`, registered in `zhang-server/src/lib.rs`. Then regenerate the typed client with `pnpm api` in `frontend`, which reads `http://localhost:8000/openapi.json` from a running server. Answer an error with a `ServerError` (`zhang-server/src/error.rs`): every API error has one JSON body, `{"message": "..."}`, which the web UI shows, the rejections of the extractors included.
- **The query language**: `zhang-query`, and the [query language reference](/reference/query-language/).
- **A page of the web UI**: `frontend/src/pages/`, the routes in `frontend/src/router.tsx`, the navigation in `frontend/src/layout/nav-links.ts`, and the texts in `frontend/public/locales/en/` and `frontend/public/locales/zh/`.
- **A plugin**: [Writing Plugins](/developers/writing-plugins/) and the examples in `zhang-plugin-sdk/examples/`.
- **A bug in how a ledger is read**: add a case to `integration-tests/` that reproduces it.
