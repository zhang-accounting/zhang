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

使用 Docker Compose：

```yaml
services:
  zhang:
    image: kilerd/zhang:latest
    ports:
      - "8000:8000"
    volumes:
      - ./ledger:/data
    environment:
      ZHANG_AUTH: "admin:change-me"
    restart: unless-stopped
```

镜像设置了 `RUST_LOG=info`，所以 `docker logs zhang` 会显示张记账在做什么，包括账本无法加载时的原因。镜像以 `root` 身份运行张记账：它在 `/data` 中创建的文件在宿主机上属于 `root`（见[文件权限](/zh-cn/deployment/data-sources/local/#文件权限)）。

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
| `--source <SOURCE>` | `ZHANG_DATA_SOURCE` | `fs` | 账本存放的位置：`fs`（[本地文件系统](/zh-cn/deployment/data-sources/local/)）、`s3`（[S3](/zh-cn/deployment/data-sources/s3/)）、`web-dav`（[WebDAV](/zh-cn/deployment/data-sources/webdav/)）或 `github`（[GitHub](/zh-cn/deployment/data-sources/github/)）。 |
| `--no-report` | | 关闭 | 停止匿名使用报告：张记账每小时向 `https://zhang-cloud.kilerd.me/client_report` 发送一次它的版本号和构建日期。 |

命令行选项优先于对应的环境变量。不含冒号的 `--auth` 或 `ZHANG_AUTH` 值会被忽略，未知的 `ZHANG_DATA_SOURCE` 值会回退为 `fs`。

其他环境变量：

| 变量 | 说明 |
| --- | --- |
| `ZHANG_SESSION_SECRET`、`ZHANG_PASSKEY_ORIGIN`、`ZHANG_PASSKEY_RP_ID` | 会话和通行密钥，见[身份认证](/zh-cn/deployment/authentication/)。 |
| `ZHANG_S3_*`、`ZHANG_WEBDAV_*`、`ZHANG_GITHUB_*` | [S3](/zh-cn/deployment/data-sources/s3/)、[WebDAV](/zh-cn/deployment/data-sources/webdav/) 和 [GitHub](/zh-cn/deployment/data-sources/github/) 数据源的设置。 |
| `ZHANG_QUERY_MAX_RESULT_VALUES` | 一条[查询](/zh-cn/reference/query-language/)的结果最多能包含多少个值，超出后查询会被中止。默认为 `1000000`。在内存较小的机器上可以调低。 |
| `ZHANG_LOG` | 日志过滤器，例如 `info`、`debug` 或 `zhang_core=trace`（[env_logger 语法](https://docs.rs/env_logger/latest/env_logger/#enabling-logging)）。不设置时使用 `RUST_LOG`；两者都不设置时级别为 `info`。Docker 镜像设置了 `RUST_LOG=info`。 |

如果启动时账本无法加载（例如存在语法错误），或者端口已被占用，`zhang serve` 会以退出码 1 退出，这样 systemd、Docker 和托管平台都能识别出启动失败。用 `RUST_LOG=info` 运行即可看到原因。账目本身的问题，例如不平衡的交易，不会让服务器停止：它们会列在网页界面中。

## 其他命令

- `zhang update` 把二进制文件替换为适用于你的平台的最新发布版，见[升级](/zh-cn/deployment/upgrading/#预编译二进制文件)。
- `zhang parse` 和 `zhang export` 只是占位命令：它们目前还不会读取账本。要检查账本，请启动 `zhang serve` 并查看网页界面中的错误列表。

## 下一步

- [你的第一个账本](/zh-cn/getting-started/first-ledger/)编写一个小账本，并带你浏览网页界面。
- [从 Beancount 迁移](/zh-cn/getting-started/from-beancount/)介绍如何为现有的 Beancount 账本提供服务。
- [身份认证](/zh-cn/deployment/authentication/)保护其他人也能访问到的实例。
