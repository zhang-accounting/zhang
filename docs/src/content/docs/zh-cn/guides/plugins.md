---
title: 插件
description: 用 plugin 指令在账本中启用 WASM 插件，为插件授予能力，并决定信任哪些插件。
sidebar:
  order: 7
---

插件可以在不修改张记账本身的情况下为它增加功能。插件可以生成条目（周期性交易、分摊支出），按你自己的规则检查账本，或者提供自己的报表页面。插件是 WebAssembly（WASM）模块，张记账在加载账本时在沙箱中运行它们。

本指南介绍如何使用插件。要编写插件，见[编写插件](/zh-cn/developers/writing-plugins/)。这条指令的确切语法见 [`plugin`](/zh-cn/reference/directives/plugin/)。

## 启用插件

插件默认关闭。在账本中打开它：

```zhang
option "features.plugin" "true"
```

写成 `features.plugins` 也可以。没有这个选项时，张记账会忽略所有 `plugin` 指令，也从不读取模块。

## 添加插件

把插件的 `.wasm` 文件复制到账本目录中，例如 `plugins/` 下，然后声明它：

```zhang
option "features.plugin" "true"

plugin "plugins/large_expense.wasm"
  threshold: "500 CNY"
```

- 第一个值是模块。它的路径相对于账本根目录，与这条指令写在哪个文件中无关。对于存放在 [S3、WebDAV 或 GitHub](/zh-cn/deployment/data-sources/s3/) 上的账本，张记账也从该存储中读取模块，所以请把它和账本文件放在一起。
- 模块之后的值是传给插件的参数，元数据行是它的设置（这里是 `threshold`）。插件支持哪些参数和设置由它的作者决定。插件还会收到账本的选项。
- 有些元数据键不是设置，而是你授予插件的**能力**，见[下文](#授予能力)。
- 插件按声明的顺序运行。声明两次的模块会运行两次。
- 张记账把每个模块复制到它运行目录下的 `.cache/plugins/`，并从那里加载。本地磁盘上的模块发生变化时，`zhang serve` 会重新加载账本。

## 插件能做什么

插件会声明自己是什么，并且可以同时是以下几种：

- **processor** 接收账本的所有条目，返回张记账接下来使用的条目。它可以添加、修改或删除条目，也可以只检查条目并报告问题。
- **mapper** 做同样的事，但一次处理一个条目。
- **router** 在 `/api/plugins/{name}` 下提供页面和数据，例如自定义报表。见 [Router 插件](/zh-cn/guides/router-plugins/)。

由 processor 或 mapper 所做的修改只在账本加载期间存在。你的文件保持原样。

## 授予能力

除了自己的内存之外，插件不能做任何其指令没有授予的事：

| 元数据 | 允许插件 | 没有时 |
|---|---|---|
| `allowed_hosts` | 向这些主机发送 HTTP 请求。有多个主机时重复这个键。 | 无网络访问 |
| `allowed_paths` | 读取账本中的这些文件和目录。有多个时重复这个键。 | 无文件访问 |
| `timeout` | 每次调用最多运行这么久：整秒数（`"90"`），或带单位 `ms`、`s`、`m` 或 `h` 的数字（`"500ms"`、`"2m"`），最长一天 | 60 秒 |
| `seed` | 不增加任何权限。可以是任意文本：它会改变插件派生随机值所用的种子。 | 默认种子 |

```zhang
plugin "plugins/fx-rates.wasm"
  allowed_hosts: "api.frankfurter.dev"
  timeout: "30s"

plugin "plugins/receipts.wasm"
  allowed_paths: "documents"
  allowed_paths: "statements/2024.csv"
```

关于 `allowed_paths`：

- 每个值是一个文件或目录，相对于账本根目录，用 `/` 分隔。`"."` 表示整个根目录。
- 访问是只读的，并且限于账本根目录，无论在本地磁盘还是远程存储上都是如此。
- 以点开头的名称是隐藏的：在已授权的目录下，只有某个值明确指定了 `.git` 或 `.env`，插件才能读取它们。
- 绝对路径或包含 `..` 的值不授予任何权限。
- 只有 processor 和 mapper 能读取文件。对于本地账本，插件读取过的文件发生变化时，`zhang serve` 会重新加载。

张记账无法使用的值，例如 `timeout: "soon"`，会在 `plugin` 指令上报告为 [`ParseInvalidMeta`](/zh-cn/reference/error-codes/#parseinvalidmeta) 错误，插件则使用默认值。

:::danger[同时授予文件和网络访问]
同时拥有 `allowed_paths` 和 `allowed_hosts` 的插件可以把它读到的内容发送到你的机器之外。插件本来就会收到你的整个账本，所以只有当你在这两方面都信任一个插件时，才同时授予它两者。
:::

## 决定信任哪些插件

每个插件都会收到你的**整个账本**：每一个条目和每一个选项。它可以在张记账检查和显示条目之前修改它们，从而改变张记账展示给你的内容。

沙箱阻止的是：

- 网络访问，`allowed_hosts` 中的主机除外；
- 读取文件，`allowed_paths` 在账本根目录内授予的除外；
- 写入文件，包括你的账本文件；
- 运行超过 `timeout`。

Router 插件的页面由张记账自己的地址提供。这样的页面上的脚本可以凭你浏览器的会话调用张记账的 API，包括修改账本文件的接口。

只声明你信任的插件，最好是能读到源代码的插件，并且只授予每个插件它所需要的能力。

## 查看已加载的插件

- **设置**页面按运行顺序列出已加载的插件，包括它们的名称、版本和类型。Router 插件附有指向其页面的链接。
- `GET /api/plugins` 以 JSON 返回同样的列表，并附上每个插件被授予的 `allowed_hosts` 以及每个 Router 插件的路由。要检查 `allowed_paths` 和 `timeout`，请阅读账本中的 `plugin` 指令。

## 插件出错时

- 插件报告的问题会以 [`PluginError`](/zh-cn/reference/error-codes/#pluginerror) 的形式出现在账本的错误列表中，附带插件的名称和消息，位于插件所指向的条目或它的 `plugin` 指令上。账本仍会加载。
- 插件失败，即返回错误、崩溃或运行超时，会让整个账本无法加载，无法加载的模块也是如此，例如路径上没有模块。张记账启动时遇到这种情况会带着错误退出。运行期间，它会继续提供最后一次成功加载的版本，只在日志中记录错误：网页界面不显示错误。超时消息会指出是哪次调用运行过久；如果插件需要更多时间，请调大它的 `timeout`。

## 插件、补齐与余额断言

张记账先运行你的插件，再检查每个账户是否已开设，然后补齐，最后检查余额断言。因此：

- 插件添加的交易会计入补齐所要达到的金额和断言检查的余额。插件向未开设的账户添加的记账行，会像你自己写的一样被报告。
- 插件能看到你的 `balance` 和 `balance … with pad` 指令，但看不到补齐交易，因为它们在插件运行之后才添加。
- 插件看到的是你所写的交易：缺失的金额还没有填上，卖出也还没有与批次匹配。
- 插件添加的 `option` 或 `plugin` 指令不起作用。

补齐和断言的工作方式见[余额](/zh-cn/guides/balances/)。

## Beancount 插件

Beancount 的插件是 Python 代码，张记账不运行它们。在没有 `features.plugin` 的 Beancount 账本中，张记账会忽略 `plugin "beancount.plugins.auto_accounts"` 这样的行。打开 `features.plugin` 后，它会尝试把这些行作为 WASM 模块加载，账本将无法加载。见[从 Beancount 迁移](/zh-cn/getting-started/from-beancount/#兼容性)。
