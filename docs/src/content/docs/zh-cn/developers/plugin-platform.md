---
title: 插件平台设计（PRD2）
description: 插件 PR #798 至 #805 引用的设计文档 PRD2 的重建版本：它的决策、章节与 PR 栈各行，每个 PR 交付或改动了什么，哪些仍未完成，以及 2026-11-20 的评审结论。
sidebar:
  order: 4
---

:::caution[重建版本，供所有者评审]
PRD2 从未提交：仓库、issue 和分支里都没有它。本页根据引用它的材料重建，并沿用原编号（§1、§A 至 §G、D-A 至 D-E
各行、G.11 这样的条目、决策 D1 至 D6），让各 PR 中的引用都能找到对应。内容全部来自[来源](#来源索引)；来源没有说明或
相互矛盾之处，本页会明确指出，不自行添加任何要求。方括号中的章节标题是根据其下被引用的条目推断的。
:::

**状态说明。** *已交付（#n）*：在 PR #n 中实现。*有偏差（#n）*：实现方式不同，并附上 PR 给出的理由。*未完成*：没有实现。
*暂停*：未完成，且被 [2026-11-20 的评审](#评审结论-2026-11-20)暂缓。*未知*：被引用过，但信息不足以重建。截至
2026-11-20，PR 栈中的每个 PR 都还开着，所以“已交付”指“在开着的 PR 中”，而不是“已在 `main` 上”。

## 目的与范围

PRD2 的日期是 2026-10-13，是 MCP 服务器背后插件平台的第二轮设计。第一轮（“PRD”）产出了
[#799](https://github.com/zhang-accounting/zhang/pull/799)，即用 Rust 编写的 wasm crate 形式的 MCP 服务器。PRD2
“replaced it with this script and added writes”（[DESIGN.md][design]）。按各 PR 的引用，它的范围是：

- 在现有 Extism/wasm 边界上，把 Lua 脚本作为 router 插件运行（§A）；
- 复用已编译的模块（§A.5，D-A 行）；
- 面向 router 插件的通用写入能力，由插件声明、由账本所有者授权（§B）；
- 把 MCP 服务器改写为带写入工具的 Lua 插件（§C）；
- 把这个插件内置在 zhang 二进制中分发（§F、G.11）；
- 交付以上内容的 PR 栈（§D）。

来源明确列为范围之外的：Lua processor 和 mapper（§A.4）；`transaction` 和 `open` 以外的写入种类、限流、跨多个文件的
原子性（§B.4、§G.8、§G.12）；MCP 的 resources、prompts、SSE 和 elicitation（DESIGN.md P10）。

### 与 #698 的关系

[RFC #698](https://github.com/zhang-accounting/zhang/issues/698)（2026-10-05）以 Budget 作为第一个完整的官方插件，并
保持核心最小：核心与宿主提供记账、精确金额、通用查询和受控扩展能力，以及 HTTP 鉴权、通用路由、受控持久化与重载；业务
规则由插件提供。执行边界继续使用 Extism/wasm，Lua 是第一个脚本语言，插件中的金额不能退化为 Lua 或 Python 浮点数。
PRD2 覆盖了 #698 P1 阶段的一部分：

| #698 P1 条目 | 覆盖情况 |
|---|---|
| 参数化 BQL | [#798](https://github.com/zhang-accounting/zhang/pull/798)，在 PRD2 之前提出，被 D4 复用。它引用的是 #698 P1，而不是 PRD2。 |
| 命名查询、查询编译缓存、按类型读取金额、日期和元数据 | 未涉及。 |
| 正式接入已验证的编译复用和脚本加载，保持实例、上下文和权限隔离 | §A.5 / D-A（[#801][pr801]）、§A / D-B（[#802][pr802]）。 |
| 只读的分析 / 派生数据入口、按插件隔离的派生快照、带 schema 的派生查询表、派生计算的限额、诊断映射 | 未涉及。 |
| 插件的精确十进制 ABI / SDK | 未涉及。这正是 §A.4 不让 Lua 做 processor 和 mapper 的原因。 |

MCP 服务器不是 #698 的条目：#804 称它是“the MCP server the owner asked for without putting it into the core”。来源
没有说明 §B 是否就是 #698 中的“受控持久化”。

### 历史

- [#421](https://github.com/zhang-accounting/zhang/issues/421) / [#425](https://github.com/zhang-accounting/zhang/issues/425)
  建立了阶段管线、声明式能力和 Router 插件。#425 中所有者修订后的 Decision 10 让 Router 插件保持只读：“Writing to the
  ledger (e.g. a bank webhook that records a transaction) is out of scope for v1 and would need its own explicit
  capability.” §B 就是这项能力。#425 还把“caching compiled modules across router requests”列为后续工作（D-A）。
- 实验分支 [`spike/extism-script-runtimes`][spike]（2026-10-05）把 Lua 和 Python 解释器作为 Extism 模块运行。
  [PERFORMANCE.md][perf]：编译模块 Lua 需要 176 ms、Python 需要 3,724 ms，而从已编译代码创建实例只需 0.16 ms 和
  1.08 ms；10,000 笔交易的 JSON 往返需要数秒，而复用实例时对同一批交易做 BQL 汇总只需 4 到 5 ms。它的结论：先复用
  编译结果；插件边界只传小请求和聚合结果；计算原语和账务不变量留在 Rust。COMPILE_CACHE.md（#698 中引用）：10k 笔交易
  的 BQL router 调用中位数在 Lua 上从 35.72 ms 降到 8.89 ms，在 Python 上从 582.55 ms 降到 10.28 ms。
- 各 PR 还提到一些没有提交的工作文件：`/tmp/mcp/v2/` 下的路径（#802、#804）、“RULES”（#803）、“the stack plan”
  （#801）和“the lead's brief”（#805）。

## 决策（§1）

[DESIGN.md][design] 列出了“the owner's (PRD2 §1)”决策。没有任何 PR 正文引用 D1 至 D6；“来源中的理由”一列只在来源
给出理由时引用原文。

| # | 决策 | 来源中的理由 | 状态 |
|---|---|---|---|
| D1 | 不由用户编写 wasm：插件是 Lua 脚本；执行边界仍是 Extism/wasm，解释器是唯一的 wasm，内置在 zhang 中。 | “Nothing to compile, nothing to install”（#802）。#698：Lua 是第一个脚本语言，接口不绑定某种语言。 | 对 router 已交付（#802）。PR 栈保留了用户编写的 wasm 插件，并扩展了 Rust SDK（#803）；来源没有说明 D1 是否打算淘汰它们。 |
| D2 | Lua，**以及**写入。 | 未说明。 | 已交付（#802、#803、#804）。 |
| D3 | 金额运算保持精确；绝不使用 Lua 浮点数；十进制数以字符串传递。 | Lua 没有十进制类型，“would read `1234.56789012345678` as a floating-point number and write it back changed”（Lua 指南，#802）。“Owner ruling: floats never enter a write”（#803）。 | 已交付：router 只看到十进制字符串（#802）；写入批次中的 JSON 数字被拒绝（#803）；MCP 金额都是字符串（#804）。 |
| D4 | MCP 服务器是取代 #799 的 Lua 插件，复用 #799 中与语言无关的资产（BQL 文本、工具目录、传输与版本规则、评审修复、端到端会话）以及参数化查询宿主函数。 | 除决策本身外未说明。 | 已交付（#804）；#799 于 2026-10-13 关闭。 |
| D5 | 写入是通用的、受能力控制的宿主路径（`allowed_writes`），不专属于 MCP。 | “Any router plugin, wasm or Lua”；“this PR has value on its own”（#803）。 | 已交付（#803）。 |
| D6 | 长期规则：ABI v1 只做增量；业务逻辑在引擎中；任何东西都不能悄悄改变余额或丢弃用户文本；不隐式自动开户。 | ABI 只做增量始于 #425 Decision 1（“every plugin compiled today keeps working unchanged”）。“Business logic in the query engine, routes = BQL plus a thin mapping”（#798）。不自动开户：“a hallucinated account name never creates an account silently”（DESIGN.md §5）。 | 已交付：整个 PR 栈中 ABI 版本保持为 1；MCP 插件中与内置查询对应的 BQL 原样复制并由一致性测试固定；`open_account` 需要显式调用。 |

D1 至 D6 是 PRD2 自己的编号。#425 的决策编号为 1 至 17，#458 也有自己的 D6，它们是不同的列表。

### 实现过程中的裁定

| 裁定 | 由谁 | 结果 | 来源 |
|---|---|---|---|
| 浮点数绝不进入写入。 | 所有者 | 批次中任何位置的 JSON 数字都是 `invalid`。 | #803 |
| 没有扩展名的模块路径仍按 wasm 加载。 | lead | 保持向后兼容；与 §A.1 有偏差。 | #802 |
| 新文件的 `include` 先于文件本身写入。 | lead | 推翻 §B.3.6 和 §G.8。 | #803 |
| 保留发布附件 `zhang-mcp-<version>.lua`；Docker 镜像不带该脚本。 | 未说明（“Distribution ruling”） | 移除 `d1c96246` 中加入的 Docker 副本。 | [`4a5e6e1e`][c-4a5e6e1e] |
| 除了每次重载只允许一次提交以外，不做限流。 | 所有者 | §G.12 保持在范围之外（如果 G.12 指限流，见下文）。 | DESIGN.md §6 |

## §A [Lua router 插件]

“A `plugin "x.lua"` directive runs a Lua script as a router plugin (PRD2 §A)”（#802）。对应 D-B 行。

| 条目 | 意图 | 状态 |
|---|---|---|
| A.1 | 模块种类由扩展名决定：`.lua` 是脚本，`.wasm` 和 `.wat` 是模块；未知扩展名在读取任何内容之前就被拒绝。 | 已交付（#802）。有偏差：没有扩展名的路径仍按 wasm 加载（“lead ruling: backward compatible”）。#802 把它列为偏差，说明 §A.1 原本会拒绝它；来源没有直接这样说。 |
| A.2 | 解释器是一个 wasm 模块，连同校验和一起纳入仓库，并可从固定版本的源码可复现地重新构建。 | 已交付（#802）：Lua 5.4.8 和 dkjson 2.11，416,356 字节，`tools/lua-runtime`，CI 任务 `lua-runtime`；#803 中重新构建为 420,406 字节。有偏差：CI 任务在每个 PR 上运行，而不是每周定时运行（只需 27 s）；去掉了实验分支中未使用的 binaryen 和 python-pdk 固定版本。#802 把两者列为偏差但没有给出章节号；§A.2 是推断的。 |
| A.3 | Lua API：模块表 `{name, version, types, router}`、桥接到宿主函数的 `zhang.*`、`json`、参数构造函数；`allowed_paths` 下的 `zhang.read_file` / `zhang.list_dir`；每次调用 64 MiB 的内存预算，“the first memory bound in the plugin host”（§A.3、§G.3）。 | 已交付（#802）。有偏差：`types` 可省略，默认为 `{"Router"}`（“a 20-line plugin should not need it”）；文件函数总是抛出 `denied`，因为“a router has no file access (host rule, unchanged)”。未完成：面向所有插件的通用 `memory` 能力（#802 的后续工作）。 |
| A.4 | 本轮 Lua 脚本只能是 router 插件；`Processor` 和 `Mapper` 会被拒绝，因为指令中的金额以 JSON 数字传递。 | 已交付（#802）。未完成：配合 #698 P1 精确十进制桥接的 Lua processor 和 mapper。 |
| A.5 | 把实验分支的编译缓存作为独立 PR（D-A）；manifest 是缓存键，所以修改脚本后每次重载只重新编译一次。 | 已交付（#801、#802）。有偏差：§A.5“describes one `Arc<CompiledPlugin>` per registered plugin”；#801 为每个注册保留最多四个空闲的已编译插件，每个并发调用租用一个，因为“shared code would let one request's timeout fail a concurrent request of the same plugin”。#801 每次加载编译两次；#802（`13a10dd4`）和 #803 用完整设置注册，因此每次加载只编译一次。 |
| A.6 | 通过 wasm 测试框架运行的一套 Lua 夹具。 | 已交付（#802）：21 个测试（WAT 对应版本、沙箱、JSON 往返、内存预算、语法和结构错误、名称冲突、文件访问被拒绝）。 |

#802 还列出了两条没有章节号的偏差：非查询宿主调用失败时抛出 `<kind>: <message>` 而不是单纯的消息，“so a `pcall`
caller can tell `denied` from `not_found`”；不是 UTF-8 的 `body` 以 base64 发送而不是被拒绝，“so a Lua router can
serve an image”。

## §B [通用插件写入能力]

“Implements PRD2 §B (the generic plugin write capability), stack item D-C (§D)”（#803）。

| 条目 | 意图 | 状态 |
|---|---|---|
| B.1 | 授权：`plugin` 指令上可重复的 `allowed_writes` 元数据（`transaction`、`open`）；插件声明它写入什么；两者缺一不可。 | 已交付（#803）：可选的 `capabilities` 导出（Lua 模块表中的 `writes`），`GET /api/plugins` 中的 `writes_requested` 和 `writes_granted`。 |
| B.2 | 预览：宿主像 `POST /api/transactions/preview` 一样校验批次，导出文本，按 `directive_output_path` 选择文件，并且只在每个条目都没有问题时签发令牌（对 generation、插件、过期时间和写入计划做 HMAC；10 分钟）。上限：50 条指令、64 KiB 文本、1 MiB 输入。 | 已交付（#803）。有偏差：(1) §B.2“shows one `file` per batch”；因为一个批次可能跨月，每个条目各自给出文件。(2) §B.2“lists `conflict` for commits only”；账本过期或有写入待重载时，预览也返回 `conflict`，“so a plugin never holds a token it cannot use”。(3) §B.2“says an `open` with a different commodity set 'is an error'”；实际是 `commodities` 上的字段错误，因为“the ledger would accept the re-open; the refusal is the plugin path's”。(5) 交易的 `datetime` 可以只写日期，“which §B.2's example relies on”；API 也接受这种写法。 |
| B.3 | 提交：重新计算写入计划，在锁内校验令牌，一次写入追加，推进 generation，重载；每个重载周期只允许一次提交；错误种类 `unavailable`、`denied`、`invalid_input`、`invalid`、`conflict`、`io`。 | 已交付（#803）。评审中的改动：失败的写入只记录一次，并说明哪些已写入（`4fad54f0`）；由提交本身而不是路由处理函数标记待重载的写入（#803 评审条目 P7）；只有重载后的内容发生变化时 generation 才推进（`cc71b6ea`、`36aa0001`）。有偏差（4，未说明章节）：`open` 的 `booking_method` 在预览时校验。有偏差（6，未说明章节）：注册时的实例获得完整 manifest；“the PRD left registration as it was”。 |
| B.3.6 | 写入按文件原子完成，新的数据文件先于它的 `include` 写入。 | 原子保存已交付（#803），并在 [`35bab424`][c-35bab424] 中扩展到 `zhang serve` 的本地磁盘数据源（临时文件加重命名，保留链接和权限）。有偏差：`include` 先写（“§B.3.6 and §G.8 ... are overridden by the lead”），因为“a dangling `include` is visible and an un-included file is a silent duplicate on retry”。 |
| B.4 | [`transaction` 和 `open` 以外的写入种类] | 范围之外（#803 把“`balance`/`note`/`pad` kinds, rate limiting, a write journal”与“§B.4, §G.8, §G.12”并列，但没有一一对应；写入种类最可能是 §B.4）。暂停。 |
| B.5 | [写入的审计日志] | 部分完成：插件提交会记录一行日志（#803）。未完成：“An audit line for API writes (`wrote` still logs nothing), per §B.5.” |

#803 的偏差列表编号顺序是 1、2、7、3、4、5、6；上表沿用这些编号。

## §C [以 Lua 插件实现的 zhang-mcp]

“Replaces #799 ... (PRD2 §C, §D row D-D)”（#804）。MCP 服务器是 `plugins/mcp/mcp.lua`，含 12 个读取工具和 3 个写入
工具。

| 条目 | 意图 | 状态 |
|---|---|---|
| §C | 用一个 Lua 脚本实现 MCP 服务器，沿用 #799 的工具、BQL、传输和协议版本处理（D4）。 | 已交付（#804）：1,204 行，“4 over the ≤ 1,200 target”；来源没有说明这个目标在哪里设定。 |
| C.2 | 打开 `allow_writes` 后，三个写入工具总是列出；未授权的种类返回错误，并给出需要添加的 `allowed_writes` 行。 | 评审后交付（[`4618a2e2`][c-4618a2e2]，“PRD2 C.2”）。第一版（`d1c96246`）只在同时存在授权时才列出它们。 |
| C.2 | `preview_transaction` 返回“the host's preview object”。 | 有偏差（#804，DESIGN.md §7）：把单个条目的字段提升到顶层，“so the model never indexes `entries[0]` for the text it must show”。 |

#804 列出的、没有章节号的“from PRD2”偏差：`add_transaction` 只由宿主校验令牌（令牌“embeds its expiry in
seconds”，比较两次预览会拒绝任何超过一秒的令牌）；没有 `currency` 的 posting 使用运营币种（“unstated in PRD2”）；
写入工具的 `date` 可带可选的 `HH:MM:SS`。

## §D [PR 栈计划]

§D 以行的形式列出“the Lua/MCP stack”中的 PR。[#798](https://github.com/zhang-accounting/zhang/pull/798) 位于栈之下：
#801 称 D-A 为“the bottom of the Lua/MCP stack”。

| 行 | PR | 内容 | 状态 |
|---|---|---|---|
| — | [#798](https://github.com/zhang-accounting/zhang/pull/798) | 面向 router 插件的 `zhang_query_params` 和 `zhang_query_schema`；decimal 参数按账本数字字面量的上限约束。 | 已交付。不是 PRD2 中的一行。 |
| D-A | [#801][pr801] | 编译复用（§A.5）。 | 已交付。有偏差：§D 把它命名为 `reuse compiled wasm modules across calls`；实际使用了 stack plan 中的标题 `reuse compiled plugin modules`。 |
| D-B | [#802][pr802] | Lua router 插件（§A）。 | 已交付。 |
| D-C | [#803][pr803] | 受能力控制的写入（§B）。 | 已交付。 |
| D-D | [#804][pr804] | 带写入工具的 Lua 插件 zhang-mcp（§C）。 | 已交付。 |
| D-E | [#805][pr805] | 以 `plugin "zhang:mcp"` 内置分发（§F、G.11）。 | 已交付。有偏差：§D 的 D-E 行和 lead's brief 要求在 `GET /api/plugins` 中加入 `bundled` 字段；没有加入，因为这会改动该 PR 不允许修改的精确 JSON 列表测试。未完成（可选）：`bundled: true` 和设置页的标记。 |

## §E

未知：没有任何来源引用 §E，来源中也没有说明它的内容。

## §F [分发]

| 条目 | 状态 |
|---|---|
| MCP 插件内置在 zhang 中，账本声明 `plugin "zhang:mcp"` 即可，在任何数据源上都无需复制文件；内置不授予任何能力。 | 已交付（#805）。它面向的场景：账本通过 opendal 存放在 R2 上，在那里复制文件“is the painful step”（DESIGN.md §6）。 |
| 用于自定义脚本的发布附件 `zhang-mcp-<version>.lua`。 | 已交付（#804，`release.yml`）。 |
| Docker 镜像中的脚本副本。 | 在 `d1c96246` 中加入，被 `4a5e6e1e` 中的“Distribution ruling”移除。来源没有说明 §F 是否要求过它。 |
| 如果将来发布 zhang-core，`mcp.lua` 必须移入该 crate。 | 未完成，有前提条件（#805）。 |

## §G [限制、风险与延后事项]

只有下列条目被引用。G.1、G.2、G.4 至 G.7 和 G.9 未知：来源没有引用它们。

| 条目 | 意图 | 状态 |
|---|---|---|
| G.3 | 插件宿主中的内存上限。 | Lua 的 64 MiB 预算是“the first memory bound”（#802）。未完成：通用的 `memory` 能力。 |
| G.8 | 新数据文件先于它的 `include` 写入；跨两个文件的原子性。 | 写入顺序被 lead 推翻（见 B.3.6）。“A write journal for two-file atomicity”在范围之外（#803）；部分写入失败时的错误会说明哪些已写入。 |
| G.10 | 在 `allowed_hosts` 下为 Lua 提供 `zhang.http`。 | 未完成：“≈40 lines over `extism_http_request`”（#802）。暂停。 |
| G.11 | 内置分发，只在 D-D 通过后进行。 | 已交付（#805：“done because D-D is green”）。#804 称 D-E 为“required”；G.11 让它成为有条件的还是必需的，来源说法不一致。 |
| G.12 | [限流，或写入日志] | 范围之外（#803）。#803 的三项与 §B.4、§G.8、§G.12 的对应关系没有说明；由于别处把 §G.8 与文件顺序关联，G.12 最可能是限流。 |

## 矛盾与缺口

1. **MCP 写入工具何时列出。** #804 的概述和 DESIGN.md P8 说写入工具“hidden unless both the plugin's `allow_writes`
   and the ledger's `allowed_writes` grant are on”。#804 的写入流程一节、DESIGN.md §5、提交 `4618a2e2`（引用 PRD2
   C.2）和测试都说只要打开 `allow_writes` 就会列出。PR 栈顶端的代码按后者实现；概述和 P8 已过时。
2. **D1 与用户编写的 wasm。** D1 说“no wasm authored by users”，而 PR 栈保留并扩展了 wasm 插件和 Rust SDK。来源没有
   说明 D1 是否只针对官方插件。
3. **G.11 是有条件的还是必需的。** #805 说内置分发“done because D-D is green”；#804 称 D-E 为“required”。
4. **未对应的引用。** #803 把三项范围之外的工作与 §B.4、§G.8、§G.12 并列，但没有一一对应。
5. **没有章节号的偏差。** #802 的错误格式和 base64 偏差、#803 的偏差 4，以及 #804 的三条 PRD2 偏差都没有给出章节。
6. **§E** 和 §G 的大部分条目从未被引用。
7. **generation 的含义。** #803 起初在每次重载时都推进 generation；评审后改为基于内容。来源没有说明 §B.2 或 §B.3
   原本如何规定。

## 评审结论 (2026-11-20)

所有者在 2026-11-20 评审了整个 PR 栈，决定：

1. **先在 `main` 上建立安全基线。** 在 PR 栈中任何一个 PR 合入之前，先把安全基线合入 `main`。本页不描述其内容。
2. **运营者上限 `ZHANG_PLUGINS`**，取值 `off`、`read`、`write`，默认 `read`。部署者设定插件最多能做什么，不论账本
   授予了什么：在默认值下，账本的 `allowed_writes` 授权不会开启写入。
3. **先修复，再按顺序合入 PR 栈**：#798、#801、#802、#803、#804、#805。
4. **暂停新增插件接口**，直到 #698 的“Analyzer / derived-table”扩展点和 `zhang.decimal` API 设计完成。暂停的有：
   `zhang.http`（§G.10）、更多写入种类（§B.4）和 MCP resources（#804 的后续工作）。#802 中的 `zhang.decimal(v)` 是
   查询参数构造函数，不是这里所说的精确十进制 API。

## 来源索引

| 条目 | 来源 |
|---|---|
| 日期、两轮设计、§1、D1 至 D6、P8、§F、C.2 偏差、限流 | PR 栈顶端的 [plugins/mcp/DESIGN.md][design]（§1、§5、§6、§7） |
| #698 的范围、P1 条目、实验数据 | [#698](https://github.com/zhang-accounting/zhang/issues/698) |
| Router 只读、写入需要独立能力、编译模块缓存的后续工作 | [#425](https://github.com/zhang-accounting/zhang/issues/425)（2026-10-02 和 2026-10-03 的评论）、[#421](https://github.com/zhang-accounting/zhang/issues/421) |
| 编译和 JSON 的开销 | [`spike/extism-script-runtimes`][spike] 上的 [PERFORMANCE.md][perf] |
| §A、A.1 至 A.6、G.3、G.10 | [#802][pr802]（正文：What、Design、The interpreter module、Tests、Deviation、Follow-ups）；提交 [`6cdb848b`][c-6cdb848b]（“PRD2 A.4”）、[`47c5d139`][c-47c5d139]（“A.6”）、[`13a10dd4`][c-13a10dd4] |
| A.5、D-A 行 | [#801][pr801]（正文：What and why、Design、Deviation from PRD2、Follow-ups） |
| §B、B.1 至 B.5、G.8、G.12、D-C 行、所有者和 lead 的裁定 | [#803][pr803]（正文：What、Design decisions、After the fresh-eyes review、Deviation from PRD2、Follow-ups）；提交 [`ece2c1f3`][c-ece2c1f3]、[`4fad54f0`][c-4fad54f0]、[`cc71b6ea`][c-cc71b6ea]、[`36aa0001`][c-36aa0001]、[`35bab424`][c-35bab424] |
| §C、C.2、D-D 行 | [#804][pr804]（正文：What it is、The write flow、Deviations、Follow-ups）；提交 [`d1c96246`][c-d1c96246]、[`4618a2e2`][c-4618a2e2]（“PRD2 C.2”）、[`4a5e6e1e`][c-4a5e6e1e]（distribution ruling） |
| D-E 行、§F、G.11 | [#805][pr805]（正文：第一行、Design、Deviations） |
| #798 位于栈之下、D4 的宿主函数 | [#798](https://github.com/zhang-accounting/zhang/pull/798)、[#801][pr801]、[#804][pr804] |
| 第一轮设计（PRD） | [#799](https://github.com/zhang-accounting/zhang/pull/799)（“Deviations from the PRD”、关闭时的评论） |
| 评审结论 | 所有者 2026-11-20 的评审 |

这六个 PR 在 GitHub 上都没有评论或评审；偏差全部来自它们的正文和提交信息。

[design]: https://github.com/zhang-accounting/zhang/blob/e47a36b7bf18229348c92b92c61d86033dfd703c/plugins/mcp/DESIGN.md
[spike]: https://github.com/zhang-accounting/zhang/tree/fcfc90583c762ab0e51bea5d63a294408cd354f8/experiments/extism-scripts
[perf]: https://github.com/zhang-accounting/zhang/blob/fcfc90583c762ab0e51bea5d63a294408cd354f8/experiments/extism-scripts/PERFORMANCE.md
[pr801]: https://github.com/zhang-accounting/zhang/pull/801
[pr802]: https://github.com/zhang-accounting/zhang/pull/802
[pr803]: https://github.com/zhang-accounting/zhang/pull/803
[pr804]: https://github.com/zhang-accounting/zhang/pull/804
[pr805]: https://github.com/zhang-accounting/zhang/pull/805
[c-4a5e6e1e]: https://github.com/zhang-accounting/zhang/commit/4a5e6e1edd0621d9b3946dcfd36af2c4e6ab0be9
[c-35bab424]: https://github.com/zhang-accounting/zhang/commit/35bab424a4189d53b29e25219baf55f1cb15df68
[c-4618a2e2]: https://github.com/zhang-accounting/zhang/commit/4618a2e27250c802c4256cf61580f460d7daa9d9
[c-d1c96246]: https://github.com/zhang-accounting/zhang/commit/d1c9624642ca3524bba3871567eae167743a3b50
[c-6cdb848b]: https://github.com/zhang-accounting/zhang/commit/6cdb848ba606feeb79c54b104af9b0202993cefa
[c-47c5d139]: https://github.com/zhang-accounting/zhang/commit/47c5d139fe7b812ad16507583c0ae91b3e02b7ac
[c-13a10dd4]: https://github.com/zhang-accounting/zhang/commit/13a10dd4804b7f33fb2e8f9a8a481ac9079a3138
[c-ece2c1f3]: https://github.com/zhang-accounting/zhang/commit/ece2c1f3dd2a8d89629e701223eb102822d8eb12
[c-4fad54f0]: https://github.com/zhang-accounting/zhang/commit/4fad54f026db159234e3673f10fcceb52204de22
[c-cc71b6ea]: https://github.com/zhang-accounting/zhang/commit/cc71b6ea39080fa27f47349622ace0eb2708839f
[c-36aa0001]: https://github.com/zhang-accounting/zhang/commit/36aa0001ad28b687daed218a32493f687bb22fe5
