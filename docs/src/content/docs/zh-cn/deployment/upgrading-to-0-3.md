---
title: 从 0.2 升级到 0.3
description: 把张记账从 0.2 升级到 0.3 时，账本、插件和调用 API 的脚本会有哪些变化，以及应该如何处理。
---

张记账 0.3 按 Beancount 的方式检查账本，由服务器决定插件可以做什么，并移除了旧网页界面使用的 API 接口。在 0.2 上没有错误的账本，在 0.3 上可能会报告错误；插件可能失去网络访问；脚本可能发现某个接口已经不存在。张记账不会转换任何文件：你的文件保持原样，你只需要修复新版本报告的问题。

本页列出需要检查的事项。[更新日志](https://github.com/zhang-accounting/zhang/blob/main/CHANGELOG.md)列出了每一项变更及对应的拉取请求。替换程序本身的方法见[升级](/zh-cn/deployment/upgrading/)。

## 升级之前

1. 备份账本文件夹，如果它是 Git 仓库，就提交一次。如果你使用[通行密钥](/zh-cn/deployment/authentication/#通行密钥)，备份中要包含 `.zhang/passkeys.json`。
2. 记下账本加载了哪些插件（`plugin` 指令），以及它们需要什么：网络访问、较长的 `timeout`、账本中的文件。服务器现在会限制这三项，见[插件](#插件)。
3. 如果你使用 Docker 标签 `kilerd/zhang:snapshot`，请改用 `kilerd/zhang:latest` 或版本标签，例如 `kilerd/zhang:0.3.0`：镜像只在发布时推送，`snapshot` 不再更新。

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

**插件编译后的代码保存在张记账自己的缓存中**：放在 `~/.cache/zhang`（或 [`ZHANG_PLUGIN_CACHE_DIR`](/zh-cn/guides/plugins/#编译后的代码) 指定的目录）的子目录 `zhang-wasmtime-cache` 里，最多约 128 MiB。wasmtime 默认目录（Linux 上为 `~/.cache/wasmtime`）中的缓存不再使用，如果没有其他程序使用 wasmtime，可以删除它；`EXTISM_CACHE_CONFIG` 和 wasmtime 的配置文件也不再读取。由于缓存保存服务器会运行的代码，张记账只在属于运行它的用户、其他用户不能写入的目录中使用缓存，并且从不在账本文件夹中使用。张记账无法写入或拒绝使用的目录（例如在只读文件系统上）不会再让所有插件都无法加载：张记账会在没有缓存的情况下运行它们，并在启动时记录一条警告。

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

## 升级之后

在**设置**页面上确认版本，处理**总览**页面上的错误列表，并打开几个你熟悉的账户和报表页面，确认其中的数字符合预期。
