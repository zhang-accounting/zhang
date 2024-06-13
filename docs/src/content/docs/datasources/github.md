---
title: Github
description: This is a page in my Starlight-powered site
---

The Github data source uses the
official [Restful API](https://docs.github.com/en/rest/repos/contents?apiVersion=2022-11-28) to access remote data. At
the same time, we rely on the following specific APIs:

- [Get repository content](https://docs.github.com/en/rest/repos/contents?apiVersion=2022-11-28#get-repository-content)
- [Create or update file contents](https://docs.github.com/en/rest/repos/contents?apiVersion=2022-11-28#create-or-update-file-contents)
- [Delete a file](https://docs.github.com/en/rest/repos/contents?apiVersion=2022-11-28#delete-a-file)

Therefore, when creating a Github Token, please ensure that your Token includes the following permissions:

- `contents:read`
- `contents:write`

The following are the required parameter configurations for starting Github:
| Name | Command Line Argument | Environment Variable | Required | Value | Remarks |
|--------------|--------|--------------------|----|----------|-----------------------|
| Data Source | source | ZHANG_DATA_SOURCE | Yes | `github` |
| Github Username | | ZHANG_GITHUB_USER | Yes | | For example: `zhang-accounting` |
| Github Repository Name | | ZHANG_GITHUB_REPO | Yes | | For example: `ledger-test` |
| Github Token | | ZHANG_GITHUB_TOKEN | Yes | | For example: `ght_123123123` |

## Docker Command

```shell {2-5}
docker run --name zhang \
-e "ZHANG_DATA_SOURCE=github" \
-e "ZHANG_GITHUB_USER=zhang-accounting" \
-e "ZHANG_GITHUB_REPO=ledger-test" \
-e "ZHANG_GITHUB_TOKEN=ght_12321321" \
kilerd/zhang:0.1
```