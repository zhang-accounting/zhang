---
title: WebDAV
description: 把账本存放在 WebDAV 服务器上，例如 Nextcloud、ownCloud 或 NAS。
sidebar:
  order: 3
---

WebDAV 是一种在远程服务器上管理文件的协议。许多文件托管服务和 NAS 系统都提供它，例如 Nextcloud、ownCloud 或群晖（Synology）。张记账可以直接在 WebDAV 服务器上读写账本文件。

## 配置

用 `--source web-dav` 或 `ZHANG_DATA_SOURCE=web-dav` 选择这个数据源，并用环境变量配置它：

| 环境变量 | 必填 | 示例 | 说明 |
| --- | --- | --- | --- |
| `ZHANG_WEBDAV_ENDPOINT` | 是 | `https://dav.example.com/dav` | WebDAV 服务器的 URL。 |
| `ZHANG_WEBDAV_ROOT` | 是 | `/accounting` | 账本在服务器上所在的文件夹，相对于服务器地址。 |
| `ZHANG_WEBDAV_USERNAME` | 否 | `your_username` | 用户名，服务器要求时填写。 |
| `ZHANG_WEBDAV_PASSWORD` | 否 | `your_password` | 密码。许多服务允许你为此创建一个应用专用密码。 |

`zhang serve` 的 `<PATH>` 参数会被忽略：账本就是 `ZHANG_WEBDAV_ROOT` 文件夹。主文件仍由 `--endpoint` 指定（默认为 `main.zhang`），相对于这个文件夹。按示例中的值，张记账读取 `https://dav.example.com/dav/accounting/main.zhang`。

## 设置步骤

1. 把账本文件上传到 WebDAV 服务器上的某个文件夹。
2. 设置好变量后启动张记账：

```shell
docker run --name zhang -d -p 8000:8000 \
  -e ZHANG_DATA_SOURCE=web-dav \
  -e ZHANG_WEBDAV_ENDPOINT=https://dav.example.com/dav \
  -e ZHANG_WEBDAV_ROOT=/accounting \
  -e ZHANG_WEBDAV_USERNAME=your_username \
  -e ZHANG_WEBDAV_PASSWORD=your_password \
  kilerd/zhang:latest
```

或者使用二进制文件：

```shell
ZHANG_DATA_SOURCE=web-dav \
ZHANG_WEBDAV_ENDPOINT=https://dav.example.com/dav \
ZHANG_WEBDAV_ROOT=/accounting \
ZHANG_WEBDAV_USERNAME=your_username \
ZHANG_WEBDAV_PASSWORD=your_password \
zhang serve .
```

## 注意事项

- 张记账不会监视服务器。文件在张记账之外发生变化时（例如通过同步客户端），请使用网页界面的重新加载按钮，或者重启张记账。你在网页界面中记录的内容会写入服务器并立即重新加载。
- 网页界面会把新条目、上传的文档和通行密钥写入服务器，位置与[本地文件系统](/zh-cn/deployment/data-sources/local/#目录结构)相同。
- 插件模块和网页界面打开过的文档会缓存在运行张记账的机器上、工作目录的 `.cache` 文件夹中，见 [`.cache` 文件夹](/zh-cn/deployment/data-sources/local/#cache-文件夹)。

## 故障排除

- **张记账启动时报 `ZHANG_WEBDAV_ENDPOINT must be set` 或 `ZHANG_WEBDAV_ROOT must be set` 并退出**：这两个变量都是必填项。
- **身份验证失败**：检查用户名和密码。如果服务启用了两步验证，请创建一个应用专用密码。
- **网页界面显示空账本**：张记账没有找到主文件，于是以空账本启动。请检查 `ZHANG_WEBDAV_ENDPOINT`、`ZHANG_WEBDAV_ROOT` 和 `--endpoint`。
