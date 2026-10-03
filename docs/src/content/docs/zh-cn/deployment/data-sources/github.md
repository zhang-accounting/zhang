---
title: GitHub
description: 把账本存放在 GitHub 仓库中，网页界面中的每次修改都会生成一次提交。
sidebar:
  order: 4
---

张记账可以通过 GitHub REST API 在 GitHub 仓库中读写账本文件，无需本地克隆。网页界面写入的每个文件都会成为一次提交，所以仓库的历史就是你的修改历史。

## 配置

用 `--source github` 或 `ZHANG_DATA_SOURCE=github` 选择这个数据源，并用环境变量配置它：

| 环境变量 | 必填 | 示例 | 说明 |
| --- | --- | --- | --- |
| `ZHANG_GITHUB_USER` | 是 | `zhang-accounting` | 仓库的所有者，可以是用户或组织。 |
| `ZHANG_GITHUB_REPO` | 是 | `my-ledger` | 仓库的名称。 |
| `ZHANG_GITHUB_TOKEN` | 是 | `github_pat_…` | 能够读写该仓库内容的令牌。 |

账本从仓库默认分支的根目录读取。`zhang serve` 的 `<PATH>` 参数会被忽略。主文件由 `--endpoint` 指定（默认为 `main.zhang`），相对于仓库的根目录。

### 令牌

创建一个仅限于账本仓库的 [fine-grained personal access token](https://github.com/settings/personal-access-tokens/new)，把仓库权限 **Contents** 设为 **Read and write**。请妥善保管：任何拿到这个令牌的人都能读取和修改账本。如果令牌泄露，请在 GitHub 设置中撤销它并创建新的令牌。

## 设置步骤

1. 创建一个仓库（如果账本不公开，请设为私有），把账本文件推送到它的默认分支，主文件放在根目录。
2. 创建令牌。
3. 设置好变量后启动张记账：

```shell
docker run --name zhang -d -p 8000:8000 \
  -e ZHANG_DATA_SOURCE=github \
  -e ZHANG_GITHUB_USER=zhang-accounting \
  -e ZHANG_GITHUB_REPO=my-ledger \
  -e ZHANG_GITHUB_TOKEN=github_pat_xxxxxxxx \
  kilerd/zhang:latest
```

## 注意事项

- 网页界面写入的每个文件都单独提交，提交信息形如 `Write data/2024/02.zhang at … via opendal`。在一个还没有文件的月份记录交易会写入两个文件，因此产生两次提交：新的月份文件，以及添加到主文件中的 `include`。
- 张记账不会监视仓库。从别处推送修改后，请使用网页界面的重新加载按钮，或者重启张记账。在本地克隆中编辑之前先拉取，因为网页界面可能在此期间已经提交过。
- 网页界面会把新条目、上传的文档和通行密钥写入仓库，位置与[本地文件系统](/zh-cn/deployment/data-sources/local/#目录结构)相同。
- 插件模块和网页界面打开过的文档会缓存在运行张记账的机器上、工作目录的 `.cache` 文件夹中，见 [`.cache` 文件夹](/zh-cn/deployment/data-sources/local/#cache-文件夹)。

## 故障排除

- **张记账启动时报 `ZHANG_GITHUB_TOKEN must be set`（或 `ZHANG_GITHUB_USER`、`ZHANG_GITHUB_REPO`）并退出**：这三个变量都是必填项。
- **网页界面显示空账本**：张记账没有找到主文件，于是以空账本启动。请检查所有者、仓库名称、`--endpoint`，以及令牌能否读取该仓库。
- **在网页界面中记录失败**：令牌需要对仓库内容有写入权限。
