---
title: GitHub 数据源配置与使用
description: 本指南详细介绍了如何将 GitHub 作为数据源在张记账中配置和使用。
---

## 概览

GitHub 不仅是一个广泛使用的代码托管和版本控制平台，还可以作为张记账的强大数据源。本指南将深入探讨如何为存储和管理您的会计数据配置 GitHub，利用 GitHub 的 Restful API 实现无缝数据访问和操作。

### 关键 GitHub API

张记账通过以下 Restful API 与 GitHub 集成：

- [获取仓库内容](https://docs.github.com/en/rest/repos/contents?apiVersion=2022-11-28#get-repository-content)：检索仓库中的文件和目录。
- [创建或更新文件内容](https://docs.github.com/en/rest/repos/contents?apiVersion=2022-11-28#create-or-update-file-contents)：允许添加新文件或更新现有文件。
- [删除文件](https://docs.github.com/en/rest/repos/contents?apiVersion=2022-11-28#delete-a-file)：在仓库中启用文件删除功能。

### 所需权限

为确保操作顺利进行，您的 GitHub Token 必须包含以下权限：

- `contents:read`：读取仓库内容的权限。
- `contents:write`：创建、更新或删除仓库内容的权限。

### 配置参数

要将 GitHub 设置为数据源，需要以下参数：

| 参数             | 命令行参数       | 环境变量                 | 必填 | 值          | 备注                          |
|----------------|--------------|----------------------|----|------------|-----------------------------|
| 数据源            | source       | ZHANG_DATA_SOURCE    | 是  | `github`   | 标识 GitHub 为数据源。            |
| GitHub 用户名    | N/A          | ZHANG_GITHUB_USER    | 是  |            | 例如：`zhang-accounting`。       |
| GitHub 仓库名    | N/A          | ZHANG_GITHUB_REPO    | 是  |            | 仓库名称，例如：`ledger-test`。   |
| GitHub Token   | N/A          | ZHANG_GITHUB_TOKEN   | 是  |            | 您的 GitHub 访问令牌，例如：`ght_12321321`。 |

### Docker 命令示例

使用以下 Docker 命令启动张记账，将 GitHub 设置为数据源：

```shell
docker run --name zhang \
-e "ZHANG_DATA_SOURCE=github" \
-e "ZHANG_GITHUB_USER=zhang-accounting" \
-e "ZHANG_GITHUB_REPO=ledger-test" \
-e "ZHANG_GITHUB_TOKEN=ght_12321321" \
kilerd/zhang:0.1
```

## 常见配置和故障排除

### 克隆仓库

为了本地编辑或备份您的会计数据仓库，请使用标准的 `git clone` 命令和您的仓库 URL。

### 处理合并冲突

当多个用户同时更新数据时，可能会出现合并冲突。通过手动编辑冲突文件并提交更改来解决这些冲突。

### 令牌安全

保持您的 GitHub 令牌安全，以防止未经授权的访问。如果令牌被泄露，请立即通过 GitHub 的设置重新生成令牌。

### 连接问题

确保您的网络允许访问 GitHub 的 API 端点。使用 `curl` 或 `ping` 等工具测试连接性。

有关更详细的示例和高级配置，请参考官方 GitHub 文档和张记账的社区论坛。
