---
title: Github 数据源
description: This is a page in my Starlight-powered site
---


Github 数据源使用了官方提供的[ Restful API](https://docs.github.com/en/rest/repos/contents?apiVersion=2022-11-28)
来完成远程数据的访问. 同时我们依赖了以下具体的API：

- [Get repository content](https://docs.github.com/en/rest/repos/contents?apiVersion=2022-11-28#get-repository-content)
- [Create or update file contents](https://docs.github.com/en/rest/repos/contents?apiVersion=2022-11-28#create-or-update-file-contents)
- [Delete a file](https://docs.github.com/en/rest/repos/contents?apiVersion=2022-11-28#delete-a-file)

因此在创建Github Token时，请确保你的Token包含了以下的权限：

- `contents:read`
- `contents:write`

下面是启动 github 所需要的参数配置：

| 名称           | 命令行参数  | 环境变量               | 必填 | 值        | 备注                    |
|--------------|--------|--------------------|----|----------|-----------------------|
| 数据源          | source | ZHANG_DATA_SOURCE  | 是  | `github` |
| Github 用户名   |        | ZHANG_GITHUB_USER  | 是  |          | 例如：`zhang-accounting` |
| Github 仓库名   |        | ZHANG_GITHUB_REPO  | 是  |          | 例如： `ledger-test`     |
| Github Token |        | ZHANG_GITHUB_TOKEN | 是  |          | 例如： `ght_123123123`   |

## Docker 命令

```shell {2-5}

docker run --name zhang \
    -e "ZHANG_DATA_SOURCE=github" \
    -e "ZHANG_GITHUB_USER=zhang-accounting" \
    -e "ZHANG_GITHUB_REPO=ledger-test" \
    -e "ZHANG_GITHUB_TOKEN=ght_12321321" \
    kilerd/zhang:0.1
```