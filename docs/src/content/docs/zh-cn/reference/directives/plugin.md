---
title: 插件
description: "plugin 指令的参考：语法、配置，以及可以授予插件的能力。"
sidebar:
  order: 11
---

`plugin` 指令把一个插件（WebAssembly 模块或 Lua 脚本）加载到账本中。插件可以在账本加载时转换账本、报告账本中的问题，或者在 `/api/plugins/` 下提供页面。这条指令指定模块、授予插件能力，并把设置传给插件。

## 语法

```text
plugin "<Module>" ["<Argument>" …]
  <key>: "<value>"
  …
```

| 部分 | 必填 | 说明 |
|---|---|---|
| `"<Module>"` | 是 | 模块的路径，相对于账本根目录：按扩展名（不区分大小写）区分 `.wasm` 模块、`.wat` 模块（WebAssembly 文本）或 `.lua` 脚本（见 [Lua 插件](/zh-cn/guides/lua-plugins/)）。没有扩展名的路径和以前一样是 wasm 模块；其他扩展名会被指名拒绝。绝对路径只有在账本根目录之下才被接受；根目录之外的模块会被拒绝，账本无法加载。 |
| `"<Argument>"` | 否 | 位置参数，按原样传给插件。 |
| `<key>: "<value>"` | 否 | 元数据行：张记账授予插件的[能力](#能力)，以及插件自己的设置。 |

`plugin` 指令没有日期。它可以写在账本的任何文件中。

账本用一个选项启用插件之前，插件都处于关闭状态：

```zhang
option "features.plugin" "true"
```

`features.plugins` 同样可用。值为 `true`（不区分大小写）；其他任何值都让插件保持关闭。没有这个选项时，`plugin` 指令会被忽略，它们的模块也从不会被读取。

## 示例

```zhang
option "features.plugin" "true"

plugin "plugins/fx-rate.wasm" "USD"
  allowed_hosts: "api.frankfurter.dev"
  timeout: "30s"
  base_currency: "USD"

plugin "plugins/receipts.wasm"
  allowed_paths: "documents"
  allowed_paths: "statements/2024.csv"
```

第一个插件可以向 `api.frankfurter.dev` 发送 HTTP 请求，对它的每次调用最多可以运行 30 秒，并且它会收到设置 `base_currency`。第二个插件可以读取 `documents` 下的文件和 `statements/2024.csv` 这个文件。

## 能力

除非指令授予，否则插件不能在它自己的内存之外做任何事，而且永远不会超过服务器允许的范围（[见下文](#服务器允许的范围)）。以下元数据键是能力：

| 键 | 授予 | 默认值 |
|---|---|---|
| `allowed_hosts` | 向这些主机发送 HTTP 请求，限于服务器允许的主机。多个主机就重复这个键。 | 不能访问网络 |
| `allowed_paths` | 以只读方式访问账本中的这些文件和目录。多个就重复这个键。 | 不能访问文件 |
| `allowed_writes` | 通过 router 插件的预览和提交，向账本追加这些种类的指令：`"transaction"` 或 `"open"`。两种都要就重复这个键。仅在服务器允许写入时（`ZHANG_PLUGINS=write`）生效。 | 不能写入 |
| `timeout` | 对插件的一次调用最多可以运行多久，不超过服务器允许的时长。 | 60 秒，不超过服务器的限制（未设置时为 30 秒） |
| `seed` | 不授予任何东西。任意文本，混入插件生成随机值所用的种子。 | 无 |
| `stage` | 插件在加载账本时运行的阶段：`"booked"`，在张记账完成记账之后；或 `"raw"`，在记账之前。 | `"booked"` |

**`allowed_hosts`**：每个值是一个主机名，例如 `api.example.com`，与插件发出的每个请求的主机进行匹配。其中的 `*` 匹配任意字符，所以 `*.example.com` 授予 `example.com` 的所有子域名。端口和路径不参与匹配。无论主机是什么，请求都只会发往公网地址：解析到回环、私有、链路本地（例如云元数据服务 `169.254.169.254`）或其他非公网地址，或者解析到 Azure 的主机地址 `168.63.129.16` 的名称会被拒绝，请求连接的正是检查名称时得到的地址。重定向不会被跟随：插件收到 `3xx` 响应，对其 `location` 发出的请求和其他请求一样接受检查。允许的主机的任何端口都可以访问，服务器自己的公网地址也算作公网地址：只授予插件需要的主机，只对你信任的插件授予 `*`。

**`allowed_paths`**：每个值是一个文件或目录，相对于账本根目录，用 `/` 分隔：

- `"."` 授予整个账本根目录。目录授予它下面的所有内容，按路径的各部分逐一比较：`documents` 不授予 `documents-private`。
- 在已授予的目录下，隐藏的名称（以 `.` 开头的名称）只有在某个值明确写出它时才可读取。所以 `"."` 不会暴露 `.git` 或 `.env`，而 `".config"` 授予 `.config` 及其下的所有内容。
- 访问是只读的，账本根目录之外的任何内容都无法读取。单个文件最大 16 MiB，单次目录列表最多 10,000 个条目。
- 空值或绝对路径，或者含有 `..` 部分、反斜杠或 NUL 字符的值，不授予任何东西，并报告为 [`ParseInvalidMeta`](/zh-cn/reference/error-codes/#parseinvalidmeta)。
- 文件通过账本的数据源读取，所以这对存放在 S3、WebDAV 或 GitHub 上的账本同样有效。插件读取本地账本的某个文件时，`zhang serve` 会在该文件变化时重新加载账本。

:::danger[同时授予文件和网络]
同时获得 `allowed_paths` 和 `allowed_hosts` 的插件，可以把它读到的内容发送到你的机器之外。插件本来就会收到整个账本，所以只有对你在这两方面都信任的插件，才同时授予文件和主机。
:::

**`allowed_writes`**：每个值是插件可以追加的一种指令：`"transaction"` 或 `"open"`。它只对声明了自己会写入该种类的插件生效（插件的 `capabilities` 导出；Lua 脚本的 `writes` 表），并且只在插件作为 router 处理请求时生效。插件先预览一批指令，张记账用与 `POST /api/transactions/preview` 完全相同的方式校验它们，按 `POST /api/transactions` 的方式写入由 `directive_output_path` 选项选定的文件；然后插件用预览给出的令牌提交同一批指令。其他值会报告为 [`ParseInvalidMeta`](/zh-cn/reference/error-codes/#parseinvalidmeta)，并且不授予任何东西。宿主函数见[写入](/zh-cn/developers/writing-plugins/#写入)；每个插件申请的（`writes_requested`）和被授予的（`writes_granted`）写入种类见 `GET /api/plugins`。

:::danger[写入]
获得 `allowed_writes` 的插件可以往你的账本里添加条目。它不能修改或删除任何内容。行为良好的插件会在提交之前把每个条目展示给你，但张记账无法检查它是否这样做了：预览令牌只阻止在预览之后已经变化的账本上提交，而插件可以在一个请求里预览并提交。只对你信任的插件授予它。
:::

**`timeout`**：整数秒（`"90"`），或者带单位 `ms`、`s`、`m` 或 `h` 的整数（`"500ms"`、`"30s"`、`"2m"`）。它必须大于零，且不超过一天。无效的值会报告为 [`ParseInvalidMeta`](/zh-cn/reference/error-codes/#parseinvalidmeta)，插件使用默认值。如果一个键重复出现，以它的最后一个值为准。

**`stage`**：`"booked"` 让插件的 processor 和 mapper 在张记账完成记账之后运行：没有写金额的记账行带有张记账推算出的金额，成本写明它所匹配批次的单位成本和取得日期，跨多个批次的卖出是每个批次一行，与 Beancount 插件看到的一样。`"raw"` 让它们在记账之前运行，看到的是按原样写下的交易。其他值会报告为 [`ParseInvalidMeta`](/zh-cn/reference/error-codes/#parseinvalidmeta)，插件按 `"booked"` 运行。如果这个键重复出现，以它的最后一个值为准。见[阶段顺序约定](/zh-cn/developers/writing-plugins/#阶段顺序约定)。

**`seed`**：插件的种子只取决于它的指令：按原样书写的模块、在它之前声明同一模块的 `plugin` 指令的数量，以及 `seed` 的值。修改 `seed` 会改变插件由它派生的随机值（例如生成的 id），而不必移动这条指令。

## 行为

### 插件收到的设置

插件收到一组字符串设置：

1. 账本的每个选项，以选项的键为名；
2. 它的指令的每个元数据键（重复的键取最后一个值），`allowed_hosts` 除外。这些设置优先于同名的选项；
3. 张记账自己设置的键：`zhang.abi`，插件接口的版本；`zhang.plugin`，以 JSON 表示的整条指令，包括位置参数和每个元数据键的每个值；`zhang.seed`，插件的种子。以 `zhang.` 开头的键保留给张记账：对这三个键，张记账的值优先于同名的元数据键或选项。

插件还可以从 [`custom`](/zh-cn/reference/directives/custom/) 指令中读取随时间变化的设置。

### 加载与顺序

- 每次加载账本时，张记账都通过账本的数据源，从账本根目录读取模块。本地模块发生变化时，`zhang serve` 会重新加载账本。模块是账本的一个文件，和 `include` 一样用账本根目录内的路径来命名：根目录之外的路径会被拒绝，无论账本在本地磁盘还是远程存储上，错误中会写明该路径。
- 插件按其 `plugin` 指令的顺序运行，每次加载账本时都会运行，在张记账完成记账之后（声明了 `stage: "raw"` 的插件在记账之前运行），并且在张记账自己的步骤之前：先检查未开立的账户，然后[补齐](/zh-cn/reference/directives/balance/#用-with-pad-补齐)，然后检查余额。因此插件看到的是记账后的交易，此时补齐交易还不存在。插件从不会看到 `pad` 指令：由某条 `pad` 补齐的 `balance` 会以 `balance … with pad` 的样子交给它（见[阶段顺序约定](/zh-cn/developers/writing-plugins/#阶段顺序约定)）。
- 同一个模块声明两次，会得到两个独立的插件，各自有自己的设置和种子。
- 模块缺失或无法加载，或者插件调用失败或运行超过 `timeout`，都会让账本无法加载。如果 `zhang serve` 已经在运行，它会继续提供重新加载之前的账本。
- 网页界面的设置页面列出已加载的插件。router 插件在 `/api/plugins/<name>` 提供服务；见 [Router 插件](/zh-cn/guides/router-plugins/)。

## 服务器允许的范围

账本是文本，除了运行 `zhang serve` 的人，其他人也可能修改它：通过网页界面的文件编辑器、Git 仓库或存储桶。所以服务器用 `zhang serve` 的选项为 `plugin` 指令能授予的能力设置上限：

| 设置 | 允许 | 默认值 |
|---|---|---|
| `--plugins <off\|read\|write>`、`ZHANG_PLUGINS` | `off`：不加载任何插件。`read`：插件运行，但不修改任何东西。`write`：插件还可以在指令授予时写入账本。 | `read` |
| `ZHANG_PLUGIN_ALLOWED_HOSTS` | 任何插件最多能访问的主机：与 `allowed_hosts` 相同的模式，用逗号分隔；`*` 允许所有主机。 | 无：没有插件能访问网络 |
| `ZHANG_PLUGIN_MAX_TIMEOUT` | 对插件的一次调用最长能运行多久，写法与 `timeout` 相同。 | `30s` |
| `ZHANG_PLUGIN_MAX_MEMORY` | 一个插件实例最多能使用的内存，例如 `512MiB`。 | `256MiB` |

插件得到的是它的指令授予**并且**服务器允许的能力：指令写 `allowed_hosts: "*"`，而 `ZHANG_PLUGIN_ALLOWED_HOSTS=api.example.com` 时，插件只能访问 `api.example.com`。当指令请求的超过服务器允许的范围时，插件仍会加载，只是能力更少，并且这条指令会得到一个 [`PluginCapabilityReduced`](/zh-cn/reference/error-codes/#plugincapabilityreduced) 错误，指出需要修改的设置。使用 `--plugins off` 时，每条 `plugin` 指令都会得到一个 [`PluginsDisabled`](/zh-cn/reference/error-codes/#pluginsdisabled) 错误，没有插件运行。`zhang serve` 启动时会在日志中记录这些设置。

## 错误

| 错误 | 触发条件 |
|---|---|
| [`ParseInvalidMeta`](/zh-cn/reference/error-codes/#parseinvalidmeta) | `timeout`、`allowed_paths`、`allowed_writes` 或 `stage` 的值无效。错误指向这条 `plugin` 指令。 |
| [`PluginCapabilityReduced`](/zh-cn/reference/error-codes/#plugincapabilityreduced) | 指令授予的主机、超时或写入超过了服务器允许的范围。插件以服务器允许的能力运行。 |
| [`PluginsDisabled`](/zh-cn/reference/error-codes/#pluginsdisabled) | 服务器关闭了插件。插件不会加载。 |
| [`PluginError`](/zh-cn/reference/error-codes/#pluginerror) | 插件报告了账本中的问题。 |

## Beancount 兼容性

- Beancount 的插件是 Python 模块，例如 `plugin "beancount.plugins.auto_accounts"`。它们不能在张记账中运行。插件关闭时，这些指令会被忽略，账本可以加载。启用 `features.plugin` 时，张记账会查找以这个名字命名的模块文件，账本无法加载。
- Beancount 不接受 `plugin` 指令下的元数据。授予了能力的账本无法通过 `bean-check`。

## 相关页面

- [插件](/zh-cn/guides/plugins/)：启用和信任插件。
- [编写插件](/zh-cn/developers/writing-plugins/)：用 Rust SDK 构建插件，以及插件接口。
- [Router 插件](/zh-cn/guides/router-plugins/)：提供页面的插件。\n- [Lua 插件](/zh-cn/guides/lua-plugins/)：用 Lua 脚本编写 router 插件。
