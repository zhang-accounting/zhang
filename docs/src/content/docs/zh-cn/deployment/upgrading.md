---
title: 升级
description: 升级用 Docker 或二进制文件安装的张记账，以及升级前后需要检查的事项。
sidebar:
  order: 6
---

除了你的账本文件，以及需要时会重新生成的[插件编译代码缓存](/zh-cn/guides/plugins/#编译后的代码)，张记账自己不保存任何数据，所以升级就是替换程序并重新启动。账本文件不会被转换。

## 升级之前

- 备份账本文件夹，如果它是 Git 仓库，就提交一次。如果你使用[通行密钥](/zh-cn/deployment/authentication/#通行密钥的存储位置)，备份中要包含 `.zhang/passkeys.json`。
- 阅读从你当前版本到新版本之间各个版本的[发布说明](https://github.com/zhang-accounting/zhang/releases)。你的版本号显示在网页界面导航栏中“Zhang”的旁边，以及**设置**页面上。

## Docker

拉取新镜像，并用相同的选项（卷、端口、环境变量和参数）重新创建容器：

```shell
docker pull kilerd/zhang:latest
docker stop zhang
docker rm zhang
docker run --name zhang -d -v "/path/to/ledger:/data" -p "8000:8000" kilerd/zhang:latest
```

使用 Docker Compose：

```shell
docker compose pull
docker compose up -d
```

如果你固定了版本，例如 `kilerd/zhang:0.2.0`，请把标签改成新版本。账本保存在挂载的文件夹中，删除容器不会影响它。

## 预编译二进制文件

`zhang update` 会从 GitHub 下载适用于你的平台的最新发布版，替换当前的二进制文件：

```shell
zhang update --verbose
```

加上 `--verbose` 时，它会显示当前版本、最新版本以及下载进度。有更新的发布版时，它会先请你确认，再替换可执行文件。它需要对该文件有写入权限，所以如果二进制文件位于 `/usr/local/bin` 这样的系统文件夹中，请用 `sudo` 运行。之后请重启 `zhang serve`：正在运行的服务器在重启之前仍是旧版本。

你也可以从[发布页面](https://github.com/zhang-accounting/zhang/releases)下载新的二进制文件来替换旧的，见[安装](/zh-cn/getting-started/installation/#预编译二进制文件)。

你自己构建的二进制文件也会被替换为发布版的二进制文件。如果要继续使用源码构建的版本，请拉取新的源码并[重新构建](/zh-cn/getting-started/installation/#从源码构建)。

## 更新提示

当网页界面在浏览器中打开时，服务器每分钟向 GitHub 检查一次是否有更新的发布版。如果有，导航栏会显示“新版本 X 可用”，并附上指向本页的链接。这项检查与 `--no-report` 无关，后者只会停止匿名使用报告。

## 升级之后

打开网页界面，在**设置**页面上确认版本，并查看**总览**页面上的错误列表。较新的发布版可能会报告旧版本默默接受的问题。从 0.2.0 升级到之后的发布版时，尤其要注意：

- 对尚未开设或已经关闭的账户的记账行，会报告为 [`AccountDoesNotExist`](/zh-cn/reference/error-codes/#accountdoesnotexist) 或 [`AccountClosed`](/zh-cn/reference/error-codes/#accountclosed)。账户名中的笔误会以这种方式暴露出来。
- 开立时列出了商品的账户（例如 `open Assets:Bank USD`）只接受这些商品：使用其他商品的记账行或余额断言会被报告为 [`CommodityNotAllowed`](/zh-cn/reference/error-codes/#commoditynotallowed)，与 Beancount 一致。把该商品加进 `open`，或者不写列表。
- 不成立的[余额断言](/zh-cn/reference/directives/balance/)仍然报告为 [`AccountBalanceCheckError`](/zh-cn/reference/error-codes/#accountbalancecheckerror)，但不再把账户余额改成断言的金额：余额、报表和补齐都以记账行为准。
- 交易必须按商品分别平衡，并以该商品的精度计算。混用多种商品或价格却不平衡的交易，会报告为 [`UnbalancedTransaction`](/zh-cn/reference/error-codes/#unbalancedtransaction)。
- 使用不受支持的记账方法（`NONE`、`AVERAGE`、`AVERAGE_ONLY`）或无效记账方法的账户，会带着一个错误、以默认方法加载，而不是让加载中止。见[记账方法](/zh-cn/reference/directives/account/)。
- 浏览器的密码弹窗被登录页面取代，并且可以使用通行密钥。脚本可以继续以 HTTP Basic 请求头发送密码，见[身份认证](/zh-cn/deployment/authentication/)。
- **除非服务器允许更多，插件以只读方式运行，没有网络访问，每次调用最多 30 秒。** 指令授予了 `allowed_hosts` 的插件（例如汇率插件）会失去网络访问，直到你用 `ZHANG_PLUGIN_ALLOWED_HOSTS` 允许它的主机；超过 30 秒的 `timeout` 会被降低，除非 `ZHANG_PLUGIN_MAX_TIMEOUT` 允许。错误列表会在每条这样的指令上显示一个 [`PluginCapabilityReduced`](/zh-cn/reference/error-codes/#plugincapabilityreduced) 错误，指出相应的设置。插件也不再跟随重定向，不能访问私有或回环地址。见[服务器允许的范围](/zh-cn/guides/plugins/#服务器允许的范围)。
- [Router 插件](/zh-cn/guides/router-plugins/)的页面在沙箱中运行：页面中的脚本不能再使用你的会话，从张记账加载脚本、样式、图片或 JSON 的页面需要把它们内联。插件只会收到允许列表中的请求头，也只能发送允许列表中的响应头；一个请求最多运行 30 秒；每个插件实例最多使用 256 MiB 内存，除非 `ZHANG_PLUGIN_MAX_MEMORY` 允许更多。

- **`ZHANG_PLUGINS=off` 也会关闭预算。** 预算由官方插件 `zhang:budget` 解释：每个账本都有它，不需要 `plugin` 指令，也不受 `features.plugin` 选项影响，而运维者的开关会像停止其他插件一样停止它。以 `--plugins off` 或 `ZHANG_PLUGINS=off` 运行的服务器会在启动时记录预算已关闭；预算页面和查询会返回预算表不可用的错误，错误列表会显示一个 [`ExtensionUnavailable`](/zh-cn/reference/error-codes/#extensionunavailable) 错误。查询函数 `account_budgets()` 仍然会返回结果，因为它只读取账户 `open` 指令的 `budget` 元数据。预算需要 `ZHANG_PLUGINS` 为 `read`（默认值）或 `write`。
- **插件编译后的代码保存在张记账自己的缓存中**：放在 `~/.cache/zhang`（或 [`ZHANG_PLUGIN_CACHE_DIR`](/zh-cn/guides/plugins/#编译后的代码) 指定的目录）的子目录 `zhang-wasmtime-cache` 里，最多约 128 MiB。wasmtime 默认目录（Linux 上为 `~/.cache/wasmtime`）中的缓存不再使用，如果没有其他程序使用 wasmtime，可以删除它；`EXTISM_CACHE_CONFIG` 和 wasmtime 的配置文件也不再读取。由于缓存保存服务器会运行的代码，张记账只在属于运行它的用户、其他用户不能写入的目录中使用缓存，并且从不在账本文件夹中使用。张记账无法写入或拒绝使用的目录（例如在只读文件系统上）不会再让所有插件都无法加载：张记账会在没有缓存的情况下运行它们，并在启动时记录一条警告。

[错误码](/zh-cn/reference/error-codes/)解释了每种错误的修复方法。
