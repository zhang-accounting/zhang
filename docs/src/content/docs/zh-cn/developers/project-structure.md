---
title: 项目结构
description: 张记账仓库中的 crate 和文件夹，账本如何在其中流转，以及如何在本地构建、运行和测试张记账。
sidebar:
  order: 2
---

张记账由一个 Rust workspace、一个 React 网页界面和这个文档站点组成，全部位于 [zhang-accounting/zhang](https://github.com/zhang-accounting/zhang) 仓库中。本页是写给贡献者的地图。

## 仓库结构

| 路径 | Crate | 作用 |
| --- | --- | --- |
| `zhang-ast/` | `zhang-ast` | 所有 crate 共用的指令类型（`Directive`、`Transaction`、`Posting`、`Account`、金额、日期），以及表示账本错误种类的 `ErrorKind`。 |
| `zhang-core/` | `zhang-core` | 加载账本：张记账文本格式的解析器和导出器（`src/data_type/text/`）、`DataSource` trait、选项、处理流水线（`src/pipeline/`）、记账、内存存储，以及 WASM 插件运行时（`src/plugin/`，feature `plugin_runtime`，基于 [Extism](https://extism.org/)）。 |
| `extensions/beancount/` | `beancount` | Beancount 的解析器和导出器，用于以 `.bean`、`.beancount` 或 `.bc` 结尾的主文件。 |
| `zhang-query/` | `zhang-query` | 兼容 BQL 的查询引擎，在内存存储上执行查询。 |
| `zhang-server/` | `zhang-server` | HTTP API（axum 和 gotcha，后者也负责生成 OpenAPI 描述）、身份认证（`src/auth/`）、文件监视和重新加载（`src/watch.rs`），以及网页界面：启用 `frontend` feature 时从 `frontend/dist` 嵌入。 |
| `zhang-cli/` | `zhang` | `zhang` 二进制文件（`src/main.rs`），以及本地文件系统、S3、WebDAV 和 GitHub 的数据源，基于 [Apache OpenDAL](https://opendal.apache.org/)（`src/opendal.rs`）。 |
| `zhang-plugin-sdk/` | `zhang-plugin-sdk` | WASM 插件的 Rust SDK，`examples/` 中有两个示例插件。见[编写插件](/zh-cn/developers/writing-plugins/)。 |
| `bindings/wasm/` | `zhang-wasm` | 用 wasm-pack 编译为 WebAssembly 的解析器，供在线 Playground 使用。 |
| `bindings/python/` | `zhang-python` | 实验性的 Python 绑定（PyO3，用 maturin 构建）。 |
| `frontend/` | | 网页界面：React、TypeScript、Vite 和 Tailwind CSS。 |
| `docs/` | | 本站点，用 Astro 和 Starlight 构建。见 `docs/README.md`。 |
| `integration-tests/` | | 端到端测试用例：每个用例一个文件夹，包含一个账本以及它必须产生的 API 响应。 |
| `examples/` | | 一个示例账本，`examples/main.zhang`。 |

## 账本在代码中的流转

1. **读取。** `zhang serve` 为所选的后端构建数据源（`zhang-cli/src/opendal.rs`），并调用 `Ledger::async_load`（`zhang-core/src/ledger.rs`）。数据源读取主文件，用张记账解析器或 Beancount 解析器解析它，并顺着 `include` 指令（包括通配符）继续读取。结果是一组指令（`zhang-ast`），每条指令都带有它所在的文件和位置。
2. **选项与插件。** 首先应用选项。然后获取并注册 `plugin` 指令的模块（`zhang-core/src/plugin/`）。
3. **流水线。** 指令按日期排序，并依次经过 `zhang-core/src/pipeline/` 中的各个阶段：先是按声明顺序执行的 WASM 插件（processor 和 mapper），然后是内置阶段 `ActiveAccounts`、`Pad` 和 `BalanceCheck`。每个阶段都能看到整个指令流，可以修改它，也可以报告错误。
4. **存储。** 结果被逐条指令地（`zhang-core/src/process/`）合并进内存存储（`zhang-core/src/store/`）。记账器（`zhang-core/src/booking/`）把记账行与批次进行匹配，每个问题都会成为一个带有 `ErrorKind` 的错误。
5. **查询与服务。** `zhang-query` 在存储上执行查询。`zhang-server` 在 `/api/` 下以 HTTP API 的形式提供账本（处理函数位于 `zhang-server/src/routes/`），在 `/openapi.json` 提供 OpenAPI 描述，把请求路由给 router 插件，并在文件变化时重新加载账本。
6. **网页界面。** `frontend/` 中的 React 应用通过根据 OpenAPI 描述生成的类型化客户端（`frontend/src/api/schemas.ts`）调用 API，并监听 `/api/sse`，在重新加载后刷新。

写入则沿相反方向进行：路由构建一条指令，数据类型把它导出为文本（张记账或 Beancount 语法），数据源把它追加到 `directive_output_path` 选项所指定的文件中，然后账本重新加载。

## 在本地构建和运行

你需要稳定版 Rust 工具链、Node.js 和 [pnpm](https://pnpm.io/) 9，运行 Python 绑定的测试还需要 Python 3。插件 SDK 的测试还需要 `wasm32-unknown-unknown` target（`rustup target add wasm32-unknown-unknown`）；没有它时，这些测试在本地会被跳过。

`zhang-server` 的 `frontend` feature 会嵌入 `frontend/dist`，启用这个 feature 时该文件夹必须存在。如果你还没有构建网页界面，可以像 CI 一样创建一个空文件夹：

```shell
mkdir -p frontend/dist
```

开发服务器时，在示例账本上运行它。不启用 `frontend` feature 时，它只提供 API：

```shell
cargo run -p zhang -- serve examples --no-report
```

开发网页界面时，先按上面的方式在 8000 端口启动服务器，再启动 Vite 开发服务器，它会把 `/api` 代理到 `http://localhost:8000`（设置 `VITE_API_ENDPOINT` 可以使用其他地址）：

```shell
cd frontend
pnpm install
pnpm dev
```

然后打开 `http://localhost:3000`。

像发布工作流那样构建嵌入了网页界面的发布版二进制文件：

```shell
cd frontend && pnpm install && pnpm build && cd ..
cargo build --release --bin zhang --features frontend
```

## 测试与检查

CI（`.github/workflows/build-latest.yml`）会在每个 pull request 上运行以下检查：

| 检查 | 命令 |
| --- | --- |
| Rust 测试 | `cargo test`，以及 `cargo test -p zhang-core`（不含插件运行时） |
| 格式 | `cargo +nightly fmt --all -- --check` |
| Lint | `cargo clippy --all-features --all-targets -- -D warnings -D clippy::dbg_macro -A clippy::empty_docs`，以及针对 `--target wasm32-unknown-unknown -p zhang-plugin-sdk -p zhang-plugin-example-guard -p zhang-plugin-example-summary` 的同样检查 |
| WebAssembly 绑定 | 在 `bindings/wasm` 中运行 `wasm-pack build` |
| 网页界面 | 在 `frontend` 中运行 `pnpm run prettier:check` 和 `pnpm build` |
| 拼写 | [typos](https://github.com/crate-ci/typos)，配置在 `_typos.toml` 中 |

`cargo test` 也会运行 `integration-tests/` 中的端到端用例。每个文件夹包含一个 `main.zhang` 或 `main.bean`，以及一个 `validations.json`：一组 API URI，每个 URI 带有若干 JSONPath 表达式及其必须返回的值。网页界面也有单元测试：在 `frontend` 中运行 `pnpm test`。

文档只在推送到 `main` 和 `develop` 时构建和部署，pull request 上不会构建。修改 `docs/` 之前请先自己构建一次，见 `docs/README.md`。

## 从哪里入手

- **语法**：张记账的解析器和导出器在 `zhang-core/src/data_type/text/`，Beancount 的在 `extensions/beancount/`。
- **新的检查或错误**：在 `zhang-ast/src/error.rs` 中为 `ErrorKind` 添加一个变体，并在 `zhang-core` 的某个流水线阶段或 `process` 处理函数中报告它。把它的消息添加到 `frontend/public/locales/*/translation.json` 的 `ERROR` 中，并在[错误码](/zh-cn/reference/error-codes/)中添加一节。
- **API 接口**：`zhang-server/src/routes/` 中的处理函数，在 `zhang-server/src/lib.rs` 中注册。然后在 `frontend` 中运行 `pnpm api` 重新生成类型化客户端，它会从正在运行的服务器读取 `http://localhost:8000/openapi.json`。
- **查询语言**：`zhang-query`，以及[查询语言参考](/zh-cn/reference/query-language/)。
- **网页界面的页面**：`frontend/src/pages/`，路由在 `frontend/src/router.tsx`，导航在 `frontend/src/layout/nav-links.ts`，文本在 `frontend/public/locales/en/` 和 `frontend/public/locales/zh/`。
- **插件**：[编写插件](/zh-cn/developers/writing-plugins/)以及 `zhang-plugin-sdk/examples/` 中的示例。
- **账本读取方面的 bug**：在 `integration-tests/` 中添加一个能复现它的用例。
