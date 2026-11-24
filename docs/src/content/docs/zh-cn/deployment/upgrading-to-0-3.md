---
title: 从 0.2 升级到 0.3
description: 把张记账从 0.2 升级到 0.3 时，账本、插件和调用 API 的脚本会有哪些变化，以及应该如何处理。
---

张记账 0.3 按 Beancount 的方式检查账本，由服务器决定插件可以做什么，并移除了旧网页界面使用的 API 接口。在 0.2 上没有错误的账本，在 0.3 上可能会报告错误；插件可能失去网络访问；脚本可能发现某个接口已经不存在。张记账不会转换任何文件：你的文件保持原样，你只需要修复新版本报告的问题。

本页列出需要检查的事项。[更新日志](https://github.com/zhang-accounting/zhang/blob/main/CHANGELOG.md)列出了每一项变更及对应的拉取请求。替换程序本身的方法见[升级](/zh-cn/deployment/upgrading/)。

## 升级之前

1. 备份账本文件夹，如果它是 Git 仓库，就提交一次。如果你使用[通行密钥](/zh-cn/deployment/authentication/#通行密钥)，备份中要包含 `.zhang/passkeys.json`。
2. 记下账本加载了哪些插件（`plugin` 指令），以及它们需要什么：网络访问、较长的 `timeout`、账本中的文件。服务器现在会限制这三项，见[插件](#插件)。如果账本使用了预算，请保持插件开启：在 0.3 中，预算也是一个插件。
3. 如果你使用 Docker 标签 `kilerd/zhang:snapshot`，请改用 `kilerd/zhang:latest` 或版本标签，例如 `kilerd/zhang:0.3.0`：镜像只在发布时推送，`snapshot` 不再更新。要在重新创建容器后保留插件编译后的代码，请在 `/root/.cache/zhang` 挂载一个单独的卷，见 [Docker](/zh-cn/getting-started/installation/#docker)。

## 插件

现在由服务器而不是账本决定插件可以做什么。默认情况下，插件以只读方式运行，没有网络访问，每次调用最多 30 秒。要求超出这些范围的 `plugin` 指令，会各自得到一个 [`PluginCapabilityReduced`](/zh-cn/reference/error-codes/#plugincapabilityreduced) 错误，指出允许它的设置：

| 插件需要 | 允许它的设置 |
|---|---|
| 访问 `allowed_hosts` 中的主机 | `ZHANG_PLUGIN_ALLOWED_HOSTS` |
| 超过 30 秒的 `timeout` | `ZHANG_PLUGIN_MAX_TIMEOUT` |
| 写入条目（`allowed_writes`） | `zhang serve --plugins write` 或 `ZHANG_PLUGINS=write` |
| 超过 256 MiB 的内存 | `ZHANG_PLUGIN_MAX_MEMORY` |

`--plugins off`（`ZHANG_PLUGINS=off`）不加载任何插件。每项设置的说明见[服务器允许的范围](/zh-cn/guides/plugins/#服务器允许的范围)。插件的 HTTP 请求也不再跟随重定向，不再使用环境中的代理设置，并且永远不会访问回环、私有或链路本地地址。

**`ZHANG_PLUGINS=off` 也会关闭预算。** 预算由官方插件 `zhang:budget` 解释：每个账本都有它，不需要 `plugin` 指令，也不受 `features.plugin` 选项影响，而运维者的开关会像停止其他插件一样停止它。以 `--plugins off` 或 `ZHANG_PLUGINS=off` 运行的服务器会在启动时记录预算已关闭；预算页面和查询会返回预算表不可用的错误，错误列表会显示一个 [`ExtensionUnavailable`](/zh-cn/reference/error-codes/#extensionunavailable) 错误。查询函数 `account_budgets()` 仍然会返回结果，因为它只读取账户 `open` 指令的 `budget` 元数据。预算需要 `ZHANG_PLUGINS` 为 `read`（默认值）或 `write`。

**预算是一个插件。** `zhang:budget` 是张记账内置的 Lua 脚本，从 0.3 起是预算唯一的实现：张记账的原生预算代码已经删除。每个账本都会运行它，每次加载时都会运行，所以每次写入后也会运行。预算页面、预算表、内置查询和预算错误的回答与上文所述一致。作为运维者你会看到：

- **第一次启动。** 现在每个账本都会运行一个插件，所以第一次启动会编译插件运行时，期间会多占用一些内存，编译完后归还：[编译后的代码缓存](/zh-cn/guides/plugins/#编译后的代码)为空时，10,000 笔交易的账本启动约需 0.5 到 0.7 秒，有缓存时约 0.3 秒。之后的启动从缓存读取代码，重新加载（也就是每次写入）复用它。没有缓存时（例如重新创建的容器没有挂载缓存卷），每次启动都会重新编译，所以请为容器挂载缓存卷。
- **内存。** 服务器会为插件运行时和预算分析多占用一些内存：10,000 笔交易的账本约多 10 到 20 MiB，100,000 笔约多 40 到 60 MiB。升级后请检查容器的内存限制。
- **日志。** 每次加载都会在 `info` 级别记录分析情况，例如 `analyzer zhang:budget analysed the ledger in 19.182292ms: 0 diagnostics`；没有预算的账本则记录 `analyzer zhang:budget has nothing to read in the ledger: its tables are empty`。
- **设置。** **设置**页面和 `GET /api/plugins` 会列出 `zhang:budget`，类型为 `Analyzer`。
- **限制。** 它在服务器允许每个插件使用的范围内运行：预算所需时间超过 `ZHANG_PLUGIN_MAX_TIMEOUT` 或内存超过 `ZHANG_PLUGIN_MAX_MEMORY` 的账本加载时没有预算页面，也没有预算错误，并带有一个说明应调大哪个限制的 [`PluginAnalysisFailed`](/zh-cn/reference/error-codes/#pluginanalysisfailed) 错误。默认的 256 MiB 中，脚本的 Lua 内存为 224 MiB；35 个类别十年间每个类别每月一个预算约需 50 到 60 MiB。
- **没有其他实现。** `zhang:budget` 不运行的地方不会解释预算，错误列表会用一个 [`ExtensionUnavailable`](/zh-cn/reference/error-codes/#extensionunavailable) 错误提示一次：插件关闭时（`ZHANG_PLUGINS=off`）、在线 Playground 中，以及用 `zhang-core` 加载账本、却没有注册内置脚本和分析宿主的程序中（见[项目结构](/zh-cn/developers/project-structure/)）。在这些地方，预算表和查询仍然列出，并回答它们不可用。
- **与 0.2 的预算相比的已知差异**（发布前测得）：10,000 笔交易时，一次写入及其后的预算页面或错误页面多需 35 到 50 毫秒；有数千个已关闭预算的账本加载约慢 2.5 倍（3,600 个已关闭的预算：1.2 秒，而不是 0.5 秒）。每个类别每月一个预算的账本加载约慢 10% 到 20%（4,200 个预算：0.53 秒，而不是 0.47 秒）。

**插件编译后的代码保存在张记账自己的缓存中**：放在 `~/.cache/zhang`（或 [`ZHANG_PLUGIN_CACHE_DIR`](/zh-cn/guides/plugins/#编译后的代码) 指定的目录）的子目录 `zhang-wasmtime-cache` 里，最多约 128 MiB。wasmtime 默认目录（Linux 上为 `~/.cache/wasmtime`）中的缓存不再使用，如果没有其他程序使用 wasmtime，可以删除它；`EXTISM_CACHE_CONFIG` 和 wasmtime 的配置文件也不再读取。由于缓存保存服务器会运行的代码，张记账只在属于运行它的用户、其他用户不能写入的目录中使用缓存，并且从不在账本文件夹中使用。张记账无法写入或拒绝使用的目录（例如在只读文件系统上）不会再让所有插件都无法加载：张记账会在没有缓存的情况下运行它们，并在启动时记录一条警告。如果你曾用 `EXTISM_CACHE_CONFIG` 改变 wasmtime 缓存的位置，请改为设置 `ZHANG_PLUGIN_CACHE_DIR`；缓存的大小固定为约 128 MiB。

**哪些内存计入插件的限制。** `ZHANG_PLUGIN_MAX_MEMORY`（默认 256 MiB）限制插件的一个实例从创建起占用的全部内存：它的内存和表，以及它的启动函数或初始化所占用的内存；启动函数和初始化也要在插件的 `timeout` 之内完成。需要更多内存的模块会加载失败，错误信息会指出这个设置。[Lua 脚本](/zh-cn/guides/lua-plugins/#限制)可用的内存是这个限制减去张记账为宿主保留的 32 MiB，默认为 224 MiB；调大 `ZHANG_PLUGIN_MAX_MEMORY` 也会调大它。一次运行中报告超过 1000 个错误的插件会多得到一个错误，说明丢弃了多少个。

插件指令本身也有变化：

- **模块必须位于账本文件夹之内**，并以 `.wasm`、`.wat` 或 `.lua` 为扩展名（或者没有扩展名）。位于文件夹之外的路径、`../` 或指向文件夹之外的符号链接会让加载中止。把模块移到账本中，并用相对路径引用它。
- **处理器和映射器看到的是记账之后的记账行**：账本中省略的金额会被补全，成本会按批次确定。需要按书写原样处理交易的插件，要在它的 `plugin` 指令上加 `stage: "raw"`。
- **插件按 `plugin` 指令的顺序运行**，而 0.2 会先运行所有处理器，再运行所有映射器。如果一个插件依赖另一个插件，请调整指令的顺序。

[Router 插件](/zh-cn/guides/router-plugins/)是 0.3 新增的，0.2 的账本中不会有；它们的页面从一开始就在沙箱中运行，请求也受到限制。

## 你的账本

启动 0.3，打开网页界面，查看**总览**页面上的错误列表。下面是 0.2 的账本最可能遇到的错误和行为变化；每种错误的修复方法见[错误码](/zh-cn/reference/error-codes/)。

- **账户的生命周期。** 对尚未开设的账户的记账行、`pad` 或 `balance … with pad` 会报告为 [`AccountDoesNotExist`](/zh-cn/reference/error-codes/#accountdoesnotexist)，账户 `close` 之后的会报告为 [`AccountClosed`](/zh-cn/reference/error-codes/#accountclosed)。账户名中的笔误会以这种方式暴露出来。不带时间的 `close` 在当天结束时生效。
- **账户的商品。** 开立时列出了商品的账户（例如 `open Assets:Bank USD`）只接受这些商品：使用其他商品的记账行或余额断言会报告为 [`CommodityNotAllowed`](/zh-cn/reference/error-codes/#commoditynotallowed)，与 Beancount 一致。把该商品加进 `open`，或者不写列表。
- **余额断言不再改变余额。** 不成立的[余额断言](/zh-cn/reference/directives/balance/)仍然报告为 [`AccountBalanceCheckError`](/zh-cn/reference/error-codes/#accountbalancecheckerror)，但余额、报表和补齐都以记账行为准。请修正记账行，或者用 `pad` 或 `balance … with pad` 有意地调整余额。
- **断言包含子账户。** 对 `Assets:Bank` 的余额断言包含 `Assets:Bank:Checking` 等子账户，与 Beancount 一致。请重新检查父账户上的断言。
- **Beancount 账本中断言的时间。** 在 `.bean` 账本中，`balance` 在其日期的开始时检查，与它的 `time` 元数据无关；结果因此改变的断言会带有 [`BalanceTimeIgnored`](/zh-cn/reference/error-codes/#balancetimeignored) 提示。把它的日期改为第二天，或者去掉它的 `time`。
- **交易按商品分别平衡**，并以该商品的精度计算。混用多种商品或价格却不平衡的交易会报告为 [`UnbalancedTransaction`](/zh-cn/reference/error-codes/#unbalancedtransaction)。
- **批次按 Beancount 的方式记账。** 请检查按成本持有的账户的持仓和收益：没有批次可以抵减的减少会让整笔交易被拒绝（[`NoEnoughCommodityLot`](/zh-cn/reference/error-codes/#noenoughcommoditylot)、[`TransactionCannotInferTradeAmount`](/zh-cn/reference/error-codes/#transactioncannotinfertradeamount)），使用 `STRICT` 的账户在一次减少匹配到多个批次时会报告 [`AmbiguousLotMatch`](/zh-cn/reference/error-codes/#ambiguouslotmatch)。见[批次与成本](/zh-cn/guides/lots-and-cost-basis/)。
- **记账方法。** 使用不受支持的记账方法（`NONE`、`AVERAGE`、`AVERAGE_ONLY`）的账户会报告 [`UnsupportedBookingMethod`](/zh-cn/reference/error-codes/#unsupportedbookingmethod)，使用无效记账方法的会报告 [`ParseInvalidMeta`](/zh-cn/reference/error-codes/#parseinvalidmeta)；账户以默认方法加载，而不是让加载中止。见[记账方法](/zh-cn/reference/directives/account/)。
- **`.zhang` 文件中的注释。** 记账行可以带有自己的标记，所以以 `*` 开头的行不再是注释，会解析失败。请用 `;` 作为注释行的开头。
- **包含。** 没有匹配到任何文件、或者指向账本之外的 `include` 会报告为 [`IncludeNotFound`](/zh-cn/reference/error-codes/#includenotfound)，账本的其余部分照常加载。
- **补齐与预算。** 没有补齐任何金额的补齐会报告 [`UnusedPad`](/zh-cn/reference/error-codes/#unusedpad)；预算 `budget-close` 之后的记账行不再计入（[`BudgetClosed`](/zh-cn/reference/error-codes/#budgetclosed)）；同名的第二个 `budget` 会报告 [`DefineDuplicatedBudget`](/zh-cn/reference/error-codes/#defineduplicatedbudget)。
- **Beancount 账本中的新条目**写入 `.bean` 文件：`directive_output_path` 的默认值是 `data/{{year}}/{{month_str}}.{{ext}}`。
- **只读文件。** 张记账保存本地文件时，先在旁边写一个临时文件，再把它重命名为原文件。张记账无权写入的文件会以 HTTP 403 拒绝，而不会只写了一半；有硬链接的文件仍然原地写入。

## 预算数字

对同一个账本，0.3 的**预算**页面可能显示与 0.2 不同的数字。这不是账本的问题：0.2 算错了，0.3 按[预算指南](/zh-cn/guides/budgets/)所述计算。

- **其他货币的已支出会被换算。** 0.2 不管记账行是什么货币，都直接把数字相加，所以从 CNY 预算中花掉的 300 USD 被当作 300 CNY。0.3 按记账行日期当时有效的价格，把每条记账行换算为预算的货币。有 `2025-03-31 price USD 7.0 CNY` 时，一个 4 月花了 3,190 CNY 和 300 USD 的 CNY 预算，在 0.2 中显示已支出 3,490 CNY，在 0.3 中显示 5,290 CNY：300 USD 按 2,100 CNY 计入。它的可用金额以及结转到 5 月的金额都少了 1,800 CNY。其他货币的 `budget-add` 和 `budget-transfer` 也同样换算。没有价格可以换算的金额不计入，并报告为 [`BudgetCommodityMismatch`](/zh-cn/reference/error-codes/#budgetcommoditymismatch)：添加 `price` 即可计入。
- **`closed` 按月计算。** 只要账本中有某个预算的 `budget-close`，无论它的日期是哪天，0.2 都在每个月把该预算显示为已关闭。0.3 从第一条 `budget-close` 所在的月份起才把它显示为已关闭：2025-05-02 关闭的预算在 4 月仍是开放的（`budgets.month` 对 `2025-04-01` 返回的 `closed` 为 `false`），从 5 月起才是已关闭。关闭之后的记账行不再计入，见上文的**补齐与预算**。
- **记账行计入其日期当时账户 `open` 所指的预算。** 关闭后又用其他 `budget` 元数据重新开启的账户，不再把全部历史同时计入两个预算。

账本无需修改。如果拿 0.2 中记下的数字来比较，在多种货币中支出的预算，以及预算关闭之前的月份，数字会有所不同。

## 脚本与 API 客户端

- **已移除的接口。** 旧网页界面使用的那些类型化读取接口已经移除：`GET /api/store`、`/api/budgets…`、`/api/documents`、`/api/accounts/{account}/documents`、`/api/accounts/{account}/balances`、`/api/errors`、`/api/statistic/…` 和 `/api/for-new-transaction`。请改为执行内置查询：[已移除的接口](/zh-cn/reference/builtin-queries/#已移除的接口)给出了每个接口对应的查询。`POST /api/sql` 随 SQL 探索页面一起移除；请用 [BQL](/zh-cn/reference/query-language/) 写查询，并发送到 `POST /api/query`。
- **写入的响应。** `POST /api/transactions` 和 `PUT /api/transactions/{id}` 在账本重新加载之后回答 `{"data": {"id": …, "version": …}}`，而不是 `"Ok"` 和 `null`。
- **元数据的顺序。** 元数据保持书写或发送时的顺序；不要假定流水中的 `metas` 按键排序。
- **错误的 id。** 每个错误都有自己的 id：通过错误的 `txn_id` 元数据把它与交易关联起来，而不是通过它的 id。
- **分页。** `page` 从 1 开始，`size` 为 1 到 1000；`page=0` 会得到 400。
- **账户流水**包含该账户的子账户。
- **凭据。** 未认证的请求会得到 401，但不带 `WWW-Authenticate` 质询：每个请求都要以 HTTP Basic 请求头发送密码，或者以 `Authorization: Bearer` 发送会话令牌。见[身份认证](/zh-cn/deployment/authentication/)。
- **错误**是 JSON `{"message": …}`，未知的 `/api` 路径会得到 JSON 格式的 404。
- **命令行。** `zhang parse` 和 `zhang export` 已移除；`zhang serve` 和 `zhang update` 保留。

## 登录

浏览器的密码弹窗被登录页面取代，并且可以使用通行密钥。登录状态保持 30 天；设置 `ZHANG_SESSION_SECRET`，登录状态才能在服务器重启后保留。移动端通过网页界面登录，见[移动端登录](/zh-cn/deployment/authentication/#移动端登录)；`ZHANG_APP_RETURN_SCHEMES` 会忽略 `https` 等网页协议。

## 停止服务器与日志

- **`zhang serve` 收到 SIGTERM 时会正常停止**，与按 Ctrl+C 时一样；`docker stop`、Podman、Kubernetes 和 systemd 发送的都是 SIGTERM。它不再接受新连接，让进行中的请求完成（写入不会中途被打断），然后以退出码 0 退出，整个过程最多约 7 秒。`docker stop` 不再等满 10 秒、再以退出码 137 杀掉容器。如果你曾为此设置 `--stop-signal SIGINT` 或 `KillSignal=`，现在可以去掉。第二次收到信号时会立即退出，SIGINT 的退出码为 130，SIGTERM 为 143。
- **日志更安静了。** 在 `info` 级别下，日志不再包含依赖库的内部信息（`wasmtime_wasi`、`gotcha`、`tracing::span`）：除非过滤器点名它们（例如 `RUST_LOG=info,wasmtime_wasi=trace`），它们只显示警告和错误。`ZHANG_LOG` 现在作用于每一行日志。只有输出是终端时日志才带颜色，所以 `docker logs` 和 journald 得到的是纯文本；`RUST_LOG_STYLE=always` 可以恢复颜色。

## 内存

0.3 的 `zhang` 程序使用 [mimalloc](https://github.com/microsoft/mimalloc) 而不是系统的内存分配器。每次写入和每次账本文件的修改都会触发重新加载；重新加载之后，被替换的账本占用的内存会还给系统，而不是留在服务器进程中，所以经常写入的服务器占用的内存比使用系统分配器时少得多；刚加载完一个小账本时，它可能会多占用几十 MiB。如果你为张记账设置了内存上限（例如容器的内存上限），请先在 0.3 上进行几次写入、观察它的内存，再决定是否调整上限。使用 `--no-default-features` 从源码构建的程序使用系统的分配器。

插件在自己的一个线程上编译，所以它的第一次编译（缓存里还没有它的代码时）对 Lua 插件约需 60 MiB，与 CPU 的数量无关，编译完后归还；容器的内存上限不需要随主机的 CPU 数量增大。

## 升级之后

在**设置**页面上确认版本，处理**总览**页面上的错误列表，并打开几个你熟悉的账户和报表页面，确认其中的数字符合预期。
