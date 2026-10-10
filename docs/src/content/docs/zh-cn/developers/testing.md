---
title: 测试
description: 一个测试应该放在张记账仓库的什么位置，每一层测试各自负责什么，夹具、参照（oracle）与黄金文件的约定，以及如何运行各个测试套件。
sidebar:
  order: 3
---

张记账有大约 1,700 个 Rust 测试和 150 个网页界面测试，按层组织。每个事实只在拥有它的最低一层固定一次；上层只在属于自己的契约里再次涉及它：日志会列出一个 posting 的元数据，但不会再去证明元数据是如何挂到这个 posting 上的。本页说明哪一层负责什么、新测试放在哪里，以及各套件共同遵守的约定。

## 分层

| 层 | 位置 | 负责 | 夹具 |
| --- | --- | --- | --- |
| 单元 | 代码旁边的 `#[cfg(test)] mod tests` | 孤立的一个函数、阶段或路由处理函数 | 只用内联文本：不读文件、不用夹具、不用 `zhang-testkit` |
| 方言 | `zhang-core` 和 `extensions/beancount` 的 `tests/` | 一种格式的语法、转义、元数据归属、标记和往返；同一组场景分别经过两个解析器，各自有自己方言的预期 | 内联文本，经由 `zhang_testkit::dialect` |
| 引擎与流水线 | `zhang-core` 的 `tests/`（记账、pad、余额检查、账户生命周期、插件宿主）和 `zhang-query` 的 `tests/`（BQL、表、函数、`EXPLAIN`、优化器等价性检查） | 语义 | 内联文本，或按名字取用的夹具账本 |
| 参照 | `zhang-query/tests`（beanquery，八组用例共用一个比对框架）和 `extensions/beancount/tests` 下的参照目录（beancount） | 与参照工具的一致性，每一处已知差异都记入偏差列表 | 基于 `integration-tests/fava-demo-ledger` 和参照账本生成的用例文件 |
| API | `zhang-server` 的 `tests/` | JSON 形状、状态码、错误体、写入、id、分页、认证 | 临时账本目录和测试工具包的 `http` 框架 |
| 黄金 | `zhang-server/tests` 中的黄金套件 | 在每一个夹具账本上，API 的回答与引擎、存储或独立计算一致；交易 id 与黄金文件保持一致 | `zhang_testkit::fixtures` |
| 端到端 | `zhang-cli`（`integration-tests/` 中每个目录一个测试，外加对可执行文件的进程测试）和 `frontend`（`pnpm test`） | 整个应用 | 夹具语料 |

网页界面的测试是放在被测代码旁边的普通 `node --test` 文件（`frontend/src/**/*.test.ts`）。

## 新测试放在哪里

- **`.zhang` 或 `.bean` 的解析或书写规则**：该格式所在 crate 的方言测试。两种格式共有的规则进入共享场景，每种方言各给一个预期。
- **记账、pad、余额、账户或插件宿主的事实**：`zhang-core` 的 `tests/`。**BQL 的事实**：`zhang-query` 的 `tests/`。
- **"beanquery 的答案是 X"**：`zhang-query/tests` 的 beanquery 用例集中的一个用例，用 `generate.py --set <set>` 生成，绝不手写预期值。**"beancount 的行为是 X"**：`extensions/beancount/tests` 中该行为的参照目录，由其 `generate.py` 重新生成。
- **一条 HTTP 契约**：`zhang-server` 的 `tests/`，默认在处理函数层面（`respond(handler(state, ..).await)`）；只有当中间件、路径编码、内容类型或提取器的拒绝才是重点时，才经过路由器（`send`、`call`）。
- **对每一个账本都成立的不变量**：`zhang-server/tests` 中的一个黄金套件，遍历 `zhang_testkit::fixtures::every_fixture_ledger()`，并拆成若干部分，让单个测试不超过几秒。
- **需要真实账本形态和 API 回答的 bug**：在 `integration-tests/` 下新建一个目录，放入 `main.zhang`（或 `main.bean`，或两者）和 `validations.json`。端到端运行会检查这些校验，此后每个黄金套件也会加载这个账本。
- **网页界面的函数**：`frontend/src/**/<name>.test.ts`。

## 约定

- 测试名是陈述事实的 `snake_case` 句子：`a_pad_books_the_difference_now`、`every_ledger_has_an_oracle`。
- 测试不运行 cargo、网络或 shell，唯一的例外是 `zhang-server/tests/sdk_plugins.rs`，它为 `wasm32-unknown-unknown` 构建示例插件。它需要安装该 target；本地没有时会跳过，也可以用 `-E 'not binary(sdk_plugins)'` 排除。
- 运行超过几秒的测试按布局拆分：把输入切成若干片，每片一个测试，再加一个测试断言这些片覆盖了全部输入。输入本身永远不会被删减。
- 黄金文件放在它的测试旁边，用 `UPDATE_GOLDEN=1` 运行测试即可重写（`zhang_testkit::golden`）；评审者读的就是这个文件的 diff。参照用例只能由生成器按其 `README.md` 锁定的工具版本重新生成；张记账有意保留的差异记入那里的偏差列表，而不是改掉预期。
- 单元测试保留自己的小工具函数。`zhang-testkit` 只供 `tests/` 使用：带 `cfg(test)` 编译的库和工具包所链接的同一个库的副本，在编译器眼里是两个不同的 crate，类型不互通。
- 测试账本里的余额断言是精确的，除非写了 `~` 容差。在 `.zhang` 文件中，缩进行以 `*` 开头是 posting 标记，以 `#` 开头是注释；`.bean` 文件遵循 beancount。
- 一个 crate 的源内测试必须在没有其他 crate 开启的 feature 时也能构建：`cargo test -p zhang-core` 在没有插件运行时的情况下运行，而 `zhang-server` 会开启它。

## 测试工具包

`zhang-testkit` 是带集成测试的 crate 的 dev-dependency。它负责加载和比对；测试的预期留在测试里。

| 需要 | 使用 |
| --- | --- |
| 从文本得到一个账本 | `ledger::load_text`、`ledger::load_text_as`（主文件为 `.bean`）、`ledger::load_text_at`（带时钟）、`ledger::load_transformed` |
| 测试会写入或重新加载的磁盘账本 | `ledger::Scratch`（`zhang`、`beancount`、`with_files`、`copy_of`；drop 时删除） |
| fava 示例账本 | `fixtures::fava_demo`（每个进程只加载一次）或 `ledger::fava_demo_ledger`（每次重新加载） |
| `integration-tests/` 和 `examples/` 中的每个账本，或按名字取一个 | `fixtures::every_fixture_ledger`、`fixtures::fixture_ledger`、`fixtures::fixture_dir` |
| beancount 参照账本 | `fixtures::oracle_ledgers` |
| 从目录按格式加载账本 | `fixtures::load_dir` |
| 可复现的随机源 | `XorShift` |
| 方言场景：随机交易与文本布局、元数据落在哪里 | `dialect` |
| 黄金文件 | `golden::assert_text`、`golden::assert_json` |
| 用同一套规则运行并比对一组 beanquery 用例（feature `query`） | `oracle` |
| 处理函数的 `State`、JSON 形式的响应、服务器的路由器及经由它的请求（feature `server`） | `http` |

crate 自身的文档（`cargo doc -p zhang-testkit --features query,server --open`）列出了每一个入口。

## 运行测试套件

```bash
cargo nextest run                       # 全部 Rust 测试
cargo nextest run -p zhang-query        # 一个 crate
cargo nextest run -E 'test(journals)'   # 名字匹配的测试
cargo test --doc                        # 文档测试（nextest 不运行它们）
cargo test -p zhang-core                # 单独运行 zhang-core，不带插件运行时
cd frontend && pnpm test                # 网页界面
```

单独运行某个集成测试二进制仍然可以用 `cargo test -p <crate> --test <binary>`。CI 把 Rust 测试分成三组运行：`zhang-query`、`zhang-server` 和其余部分，每组都是带包过滤的 `cargo nextest run`，最后一组还运行文档测试和 `cargo test -p zhang-core`；一个测试属于哪一组由它所在的包决定。

## 二进制布局

一个 crate 的集成测试是**一个二进制**：`tests/<name>/main.rs` 声明各个模块，`tests/<name>/` 下每个领域一个文件（`zhang-core/tests/core/`、`zhang-server/tests/api/` 和 `tests/golden/`，等等）。每个集成测试二进制都要链接该 crate 和几百个依赖 crate，74 个这样的二进制占了 CI 一个任务构建阶段的一半，而合并后的 crate 多出的前端编译开销实测远不到一秒。只有当一个测试会改变**进程级全局状态**、从而影响到旁边运行的测试时，它才保留自己的二进制：每个进程只读取一次的环境变量（`zhang-server/tests/account_journal_limit.rs`）、工作目录（`zhang-server/tests/document_cache.rs`），以及会运行 cargo 的 `sdk_plugins.rs`。nextest 本来就让每个测试在自己的进程里运行，所以合并不改变运行时的隔离。

把测试加到负责该领域的模块里；只有新的领域才新增模块（`tests/<name>/` 下的一个文件和 `main.rs` 里的一行 `mod`）；只有涉及进程级全局状态时才新增二进制，并在它的第一段文档注释里说明原因。
