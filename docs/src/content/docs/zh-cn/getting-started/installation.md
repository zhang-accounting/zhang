---
title: 安装
description: 用 Docker、预编译二进制文件或从源码构建来运行张记账，以及 zhang serve 命令的选项。
sidebar:
  order: 2
---

张记账只有一个程序：`zhang`。它的 `serve` 命令加载账本，并在同一个端口上提供网页界面和 HTTP API。你可以用 Docker 运行它，下载预编译的二进制文件，或者从源码构建。

## Docker

镜像是 Docker Hub 上的 [`kilerd/zhang`](https://hub.docker.com/r/kilerd/zhang/tags)，支持 `linux/amd64` 和 `linux/arm64`：

- `kilerd/zhang:latest`：最新发布版。
- `kilerd/zhang:<version>`，例如 `kilerd/zhang:0.2.0`：指定的发布版。

镜像运行的是 `zhang serve /data --port 8000`。把存放账本的文件夹挂载到 `/data`，并发布 8000 端口：

```shell
docker run --name zhang -v "/path/to/ledger:/data" -p "8000:8000" kilerd/zhang:latest
```

打开 `http://localhost:8000`。张记账读取挂载文件夹中的 `main.zhang`。如果这个文件还不存在，张记账会以空账本启动，等你在网页界面中记录内容时再创建文件。

镜像名之后的参数会追加到 `zhang serve` 命令上，环境变量用 `-e` 传入。例如，读取一个 Beancount 主文件并启用密码登录：

```shell
docker run --name zhang -v "/path/to/ledger:/data" -p "8000:8000" \
  -e "ZHANG_AUTH=admin:change-me" \
  kilerd/zhang:latest --endpoint main.bean
```

镜像中没有系统时区。没有 [`timezone` 选项](/zh-cn/reference/directives/options/#timezone)的账本在镜像中使用 UTC，除非 `TZ` 给出它的时区，即 IANA 名称，例如 `Asia/Shanghai`：此时“今天”、“本月”以及新交易的时间都按这个时区计算。在 `docker run` 命令中加上 `-e "TZ=Asia/Shanghai"`，或者在下面 Compose 文件的 `environment` 中设置 `TZ`。账本中的选项优先于 `TZ`。

使用 Docker Compose：

```yaml
services:
  zhang:
    image: kilerd/zhang:latest
    ports:
      - "8000:8000"
    volumes:
      - ./ledger:/data
      # 插件编译后的代码，重新创建容器后仍然保留：使用单独的卷，不要用存放账本的卷，
      # 因为能写入它的任何东西都能在容器里运行代码（见下文）
      - zhang-cache:/root/.cache/zhang
    environment:
      ZHANG_AUTH: "admin:change-me"
      TZ: "Asia/Shanghai"
    restart: unless-stopped

volumes:
  zhang-cache:
```

镜像设置了 `RUST_LOG=info`，所以 `docker logs zhang` 会显示张记账在做什么，包括账本无法加载时的原因。镜像以 `root` 身份运行张记账：它在 `/data` 中创建的文件在宿主机上属于 `root`（见[文件权限](/zh-cn/deployment/data-sources/local/#文件权限)）。

[插件](/zh-cn/guides/plugins/#编译后的代码)编译后的代码保存在容器内的 `/root/.cache/zhang/zhang-wasmtime-cache`。重新创建的容器没有这些代码，会重新编译插件，每个插件需要一点时间：每个账本至少有一个插件，即计算[预算](/zh-cn/guides/budgets/#张记账如何计算预算)的 `zhang:budget`。要在重新创建容器后保留这些代码，请在 `/root/.cache/zhang` 挂载一个卷，例如 `-v zhang-cache:/root/.cache/zhang`（上面的 Compose 文件在这里挂载了命名卷 `zhang-cache`）；根文件系统只读（`--read-only`）时，如果没有挂载这样的卷，插件会在没有缓存的情况下运行。这个卷保存服务器会运行的原生代码，能写入它的任何东西都能在张记账的容器里运行代码：请使用单独的卷，属于 `root` 且只有它能写入（新建的命名卷就是这样），不要使用存放账本的卷，张记账会拒绝它。

## 预编译二进制文件

每个[发布版](https://github.com/zhang-accounting/zhang/releases)都附带名为 `zhang-<version>-<target>` 的二进制文件，其中已内置网页界面：

| 平台 | Target |
| --- | --- |
| Linux，x86-64 | `x86_64-unknown-linux-gnu` |
| Linux，ARM64 | `aarch64-unknown-linux-gnu` |
| macOS，Intel | `x86_64-apple-darwin` |
| macOS，Apple 芯片 | `aarch64-apple-darwin` |

没有 Windows 版的二进制文件，在 Windows 上请使用 Docker。下载对应平台的文件，重命名为 `zhang`，赋予可执行权限，并放到 `PATH` 中的某个文件夹里：

```shell
mv zhang-*-x86_64-unknown-linux-gnu zhang
chmod +x zhang
sudo mv zhang /usr/local/bin/
zhang serve /path/to/ledger
```

如果 macOS 拒绝打开用浏览器下载的二进制文件，用 `xattr -d com.apple.quarantine zhang` 去掉隔离标记。

## 从源码构建

你需要稳定版 Rust 工具链、用于构建网页界面的 Node.js 和 [pnpm](https://pnpm.io/) 9，以及 C 编译器、`make` 和 `perl`（OpenSSL 需要从源码编译）。

```shell
git clone https://github.com/zhang-accounting/zhang.git
cd zhang/frontend
pnpm install
pnpm build
cd ..
cargo build --release --bin zhang --features frontend
```

生成的二进制文件是 `target/release/zhang`。`frontend` feature 会把 `frontend/dist` 中构建好的网页界面嵌入其中。没有这个 feature 时，服务器只返回一段简短的提示而不是网页界面，只有 HTTP API 可用。

## `zhang serve`

```shell
zhang serve [OPTIONS] <PATH>
```

| 选项 | 环境变量 | 默认值 | 说明 |
| --- | --- | --- | --- |
| `<PATH>` | | 必填 | 账本所在的文件夹。S3、WebDAV 和 GitHub 数据源会忽略它，使用各自的设置。 |
| `-e`, `--endpoint <FILE>` | | `main.zhang` | 主文件，相对于 `<PATH>`。扩展名决定格式：`.zhang`，或者 Beancount 的 `.bean`、`.beancount` 和 `.bc`。其他扩展名会使张记账报错退出。 |
| `--addr <ADDRESS>` | | `0.0.0.0` | 监听的地址。用 `127.0.0.1` 则只接受来自本机的连接。 |
| `-p`, `--port <PORT>` | | `8000` | 监听的端口。 |
| `--auth <USER:PASSWORD>` | `ZHANG_AUTH` | 无 | 启用密码登录。见[身份认证](/zh-cn/deployment/authentication/)。 |
| `--passkey <SECRET>` | `ZHANG_PASSKEY` | 无 | 启用通行密钥登录。值是注册通行密钥时需要的密钥。见[身份认证](/zh-cn/deployment/authentication/#通行密钥)。 |
| `--plugins <off\|read\|write>` | `ZHANG_PLUGINS` | `read` | [插件](/zh-cn/guides/plugins/#服务器允许的范围)能做什么：`off` 不加载插件，包括官方插件 `zhang:budget`，因此[预算](/zh-cn/guides/budgets/)也会关闭；`read` 运行插件但不修改任何东西，`write` 还允许插件在其指令授予时写入账本。 |
| `--source <SOURCE>` | `ZHANG_DATA_SOURCE` | `fs` | 账本存放的位置：`fs`（[本地文件系统](/zh-cn/deployment/data-sources/local/)）、`s3`（[S3](/zh-cn/deployment/data-sources/s3/)）、`web-dav`（[WebDAV](/zh-cn/deployment/data-sources/webdav/)）或 `github`（[GitHub](/zh-cn/deployment/data-sources/github/)）。 |
| `--no-report` | | 关闭 | 停止匿名使用报告：张记账每小时向 `https://zhang-cloud.kilerd.me/client_report` 发送一次它的版本号和构建日期。 |

命令行选项优先于对应的环境变量。不含冒号的 `--auth` 或 `ZHANG_AUTH` 值会被忽略，未知的 `ZHANG_DATA_SOURCE` 值会回退为 `fs`。无法读取的 `ZHANG_PLUGINS` 或 `ZHANG_PLUGIN_*` 值会让 `zhang serve` 报错退出并指出该变量，所以限制永远不会被悄悄忽略；无法使用或被张记账拒绝的 `ZHANG_PLUGIN_CACHE_DIR` 只会关闭缓存，并给出一条警告。`zhang serve` 启动时会在日志中记录插件能做什么。

其他环境变量：

| 变量 | 说明 |
| --- | --- |
| `ZHANG_SESSION_SECRET`、`ZHANG_PASSKEY_ORIGIN`、`ZHANG_PASSKEY_RP_ID`、`ZHANG_APP_RETURN_SCHEMES` | 会话、通行密钥和移动端登录，见[身份认证](/zh-cn/deployment/authentication/)。 |
| `ZHANG_S3_*`、`ZHANG_WEBDAV_*`、`ZHANG_GITHUB_*` | [S3](/zh-cn/deployment/data-sources/s3/)、[WebDAV](/zh-cn/deployment/data-sources/webdav/) 和 [GitHub](/zh-cn/deployment/data-sources/github/) 数据源的设置。 |
| `ZHANG_QUERY_MAX_RESULT_VALUES` | 一条[查询](/zh-cn/reference/query-language/)的结果最多能包含多少个值，超出后查询会被中止。默认为 `1000000`。在内存较小的机器上可以调低。 |
| `ZHANG_PLUGIN_ALLOWED_HOSTS` | [插件](/zh-cn/guides/plugins/#服务器允许的范围)最多能访问的主机，用逗号分隔，例如 `api.frankfurter.dev`；`*` 允许任何主机。默认为无：插件不能访问网络。 |
| `ZHANG_PLUGIN_MAX_TIMEOUT` | 对插件的一次调用最长能运行多久，例如 `90s` 或 `2m`。默认为 `30s`。 |
| `ZHANG_PLUGIN_MAX_MEMORY` | 一个[插件](/zh-cn/guides/plugins/)实例最多能使用的内存，例如 `512MiB`。默认为 `256MiB`。 |
| `ZHANG_PLUGIN_CACHE_DIR` | [插件](/zh-cn/guides/plugins/#编译后的代码)在两次启动之间保存编译后代码的目录，代码放在其中的子目录 `zhang-wasmtime-cache` 里，最多约 128 MiB。默认为 `$XDG_CACHE_HOME/zhang`，否则为 `~/.cache/zhang`。设为空值会关闭缓存。它保存服务器会运行的代码：它必须属于运行张记账的用户、只有该用户能写入，并且位于账本文件夹之外，否则张记账不使用缓存。 |
| `ZHANG_PLUGIN_MAX_BODY`、`ZHANG_PLUGIN_MAX_CONCURRENCY` | [Router 插件](/zh-cn/guides/router-plugins/#限制)能收到的最大请求体（默认为 `4MiB`），以及一个 Router 插件能同时处理的请求数（默认为 `4`）。 |
| `TZ` | 没有 [`timezone` 选项](/zh-cn/reference/directives/options/#timezone)的账本所用的时区，即 IANA 名称，例如 `Asia/Shanghai` 或 `UTC`。默认为系统的时区，否则为 `UTC`（Docker 镜像中没有系统时区，所以是 `UTC`）。不是 IANA 名称的值（例如 POSIX 规则 `UTC0`）会被忽略，并留下一条警告。 |
| `ZHANG_LOG` | 日志过滤器，例如 `info`、`debug` 或 `zhang_core=trace`（[env_logger 语法](https://docs.rs/env_logger/latest/env_logger/#enabling-logging)）。不设置时使用 `RUST_LOG`；两者都不设置时级别为 `info`。Docker 镜像设置了 `RUST_LOG=info`。部分依赖的内部日志（`wasmtime_wasi`、`gotcha`、`tracing::span`）只显示警告和错误，除非过滤器写明了它们，例如 `info,wasmtime_wasi=trace`。只有输出是终端时日志才带颜色；`RUST_LOG_STYLE=always` 或 `never` 可以覆盖这一行为。 |

如果启动时账本无法加载（例如存在语法错误），或者端口已被占用，`zhang serve` 会在标准错误输出中打印原因并以退出码 1 退出，这样 systemd、Docker 和托管平台都能识别出启动失败。账目本身的问题，例如不平衡的交易，不会让服务器停止：它们会列在网页界面中。

## 其他命令

- `zhang update` 把二进制文件替换为适用于你的平台的最新发布版，见[升级](/zh-cn/deployment/upgrading/#预编译二进制文件)。
- 没有单独检查账本的命令：要检查账本，请启动 `zhang serve` 并查看网页界面中的错误列表。

## 下一步

- [你的第一个账本](/zh-cn/getting-started/first-ledger/)编写一个小账本，并带你浏览网页界面。
- [从 Beancount 迁移](/zh-cn/getting-started/from-beancount/)介绍如何为现有的 Beancount 账本提供服务。
- [身份认证](/zh-cn/deployment/authentication/)保护其他人也能访问到的实例。
