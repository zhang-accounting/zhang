---
title: 升级
description: 升级用 Docker 或二进制文件安装的张记账，以及升级前后需要检查的事项。
sidebar:
  order: 6
---

除了你的账本文件，张记账自己不保存任何数据，所以升级就是替换程序并重新启动。账本文件不会被转换。

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

[错误码](/zh-cn/reference/error-codes/)解释了每种错误的修复方法。
