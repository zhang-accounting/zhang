---
title: 本地文件系统
description: 为存放在本地文件系统中的账本提供服务，以及张记账如何发现文件的变化。
sidebar:
  order: 1
---

本地文件系统是默认的数据源（`--source fs` 或 `ZHANG_DATA_SOURCE=fs`）。张记账从传给 `zhang serve` 的文件夹读取账本，并把在网页界面中记录的条目写回这个文件夹。它是唯一能察觉到你自己修改了文件的数据源。

```shell
zhang serve /home/me/ledger
```

使用 Docker 时，这个文件夹就是挂载到 `/data` 的文件夹，见[安装](/zh-cn/getting-started/installation/#docker)。

## 目录结构

张记账从主文件开始（除非传入 `--endpoint`，否则为 `main.zhang`），并加载它直接或通过其他被引入的文件间接[引入](/zh-cn/reference/directives/include/)的每一个文件。`include` 中的路径相对于包含它的文件。张记账提供服务的文件夹可以是这样的：

```text
ledger/
├── main.zhang             主文件（--endpoint）
├── accounts.zhang         由 main.zhang 引入
├── data/
│   └── 2024/
│       ├── 01.zhang       由网页界面写入
│       └── 02.zhang
├── attachments/           在网页界面中上传的文档
│   └── 6f1c…/receipt.pdf
└── .zhang/
    └── passkeys.json      已注册的通行密钥
```

主文件的扩展名决定整个账本的格式：`.zhang`，或者 Beancount 的 `.bean`、`.beancount` 和 `.bc`。如果主文件不存在，张记账会以空账本启动。

文件使用 UTF-8 编码。以字节顺序标记（BOM）开头的文件（某些 Windows 编辑器保存时会添加）会被当作没有该标记来读取；张记账写入该文件时会保留它。

网页界面会写入这些文件：

- 新的交易、余额断言或文档会追加到 [`directive_output_path`](/zh-cn/reference/directives/options/) 选项为其日期指定的文件中，默认为 `data/{year}/{month}.zhang`（扩展名与主文件相同）。某个文件第一次被使用时，张记账会向主文件追加一条引入它的 `include`。
- 编辑交易时，会在它所在的文件中重写这笔交易。
- 上传的文档保存为 `attachments/<random id>/<file name>`，并关联到对应的账户或交易。
- **编辑**页面会保存你编辑的整个文件。
- 注册通行密钥时会写入 `.zhang/passkeys.json`，见[身份认证](/zh-cn/deployment/authentication/#通行密钥的存储位置)。

## 张记账何时重新加载

张记账监视账本文件夹及其子文件夹，并在以下情况重新加载账本：

- 账本的某个文件（主文件或它引入的文件）被修改；
- [插件](/zh-cn/guides/plugins/)所依赖的文件发生变化：它的模块文件（位于账本文件夹内时），或者插件通过 `allowed_paths` 读取过的文件或文件夹（在它列出的文件夹中创建、修改或删除文件也算）；
- 你在网页界面中记录或修改了内容，写入后会重新加载；
- 你使用了网页界面的重新加载按钮；
- 日期变化（在账本时区的午夜），但仅限于加载期间有插件读取过当前日期的情况。

事件会被合并：张记账在第一次变化后等待半秒，然后为这期间发生的所有变化只重新加载一次。重新加载后，网页界面会自动刷新。

有些变化不会触发重新加载：

- 匹配通配符引入（例如 `include "data/*.zhang"`）的新文件只会在下一次重新加载时被载入。请使用重新加载按钮，或者保存账本中的某个文件。
- 账本文件夹内 `.zhang/` 和 `.cache/` 中的变化，以及不属于账本的文件的变化。

:::caution[使用绝对路径]
张记账通过路径识别它加载过的文件。请用不含符号链接的绝对路径启动它，例如 `zhang serve /home/me/ledger` 或 `zhang serve "$(realpath ledger)"`。如果使用 `zhang serve .` 这样的相对路径，或者在 macOS 上使用经过符号链接的路径（例如 `/tmp/…`），修改账本文件不会触发重新加载，你只能使用重新加载按钮。Docker 镜像提供的是 `/data`，没有这个问题。
:::

### 重新加载失败时

如果某个文件根本无法读取（例如存在语法错误），重新加载会失败，张记账继续提供修改之前的账本。网页界面对此不显示任何错误：原因只会写入日志。修正文件后再保存一次即可。账目中的问题，例如不平衡的交易，不会让重新加载失败：它们会列在网页界面中。

启动时，如果账本无法读取，`zhang serve` 会以退出码 1 退出。

## 文件权限

张记账需要读取账本的每一个文件。要在网页界面中记录任何内容，它还需要写入账本文件夹：它会向已有文件追加内容，并创建 `data/2024/` 和 `attachments/` 这样的文件夹和文件。没有写入权限时，在网页界面中记录会失败，但读取仍然正常。

Docker 镜像以 `root` 身份运行张记账，所以它在挂载文件夹中创建的文件和文件夹在宿主机上属于 `root`。如果你以其他用户身份编辑它们，请用 `chown` 更改所有者。

## `.cache` 文件夹

张记账在它的工作目录中维护一个 `.cache` 文件夹，工作目录不一定是账本文件夹：

- `.cache/plugins/` 存放账本所声明的[插件](/zh-cn/guides/plugins/)的模块。
- `.cache/documents/` 存放网页界面从远程数据源（[S3](/zh-cn/deployment/data-sources/s3/)、[WebDAV](/zh-cn/deployment/data-sources/webdav/) 或 [GitHub](/zh-cn/deployment/data-sources/github/)）打开过的每个文档的副本，之后会直接从这里提供。本地磁盘上的文档每次都从磁盘读取。早期版本把副本保存在 `.cache/data/` 中，它不再被读取。

如果你在账本文件夹内启动 `zhang serve`，`.cache` 会出现在你的文件旁边，张记账会忽略它在其中做出的变化。在 Docker 镜像中，工作目录是 `/application`，所以缓存留在容器内，容器重新创建后缓存为空。

可以在张记账停止时删除这个文件夹。当你用同名的新文件替换远程数据源上的某个文档时请这样做，否则网页界面会一直显示缓存的旧副本。
