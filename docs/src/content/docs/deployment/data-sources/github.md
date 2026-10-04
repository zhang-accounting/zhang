---
title: GitHub
description: Store the ledger in a GitHub repository, with a commit for every change made in the web UI.
sidebar:
  order: 4
---

Zhang can read and write the ledger files in a GitHub repository through the GitHub REST API, without a local clone. Every file the web UI writes becomes a commit, so the history of the repository is the history of your changes.

## Configuration

Select the data source with `--source github` or `ZHANG_DATA_SOURCE=github`, and configure it with environment variables:

| Environment variable | Required | Example | Description |
| --- | --- | --- | --- |
| `ZHANG_GITHUB_USER` | yes | `zhang-accounting` | The owner of the repository, a user or an organization. |
| `ZHANG_GITHUB_REPO` | yes | `my-ledger` | The name of the repository. |
| `ZHANG_GITHUB_TOKEN` | yes | `github_pat_…` | A token that may read and write the contents of the repository. |

The ledger is read from the root of the repository, on its default branch. The `<PATH>` argument of `zhang serve` is ignored. The main file is selected with `--endpoint` (default `main.zhang`), relative to the root of the repository.

### The token

Create a [fine-grained personal access token](https://github.com/settings/personal-access-tokens/new) limited to the ledger repository, with the repository permission **Contents** set to **Read and write**. Keep it secret: anyone with the token can read and change the ledger. If it leaks, revoke it in the GitHub settings and create a new one.

## Setup

1. Create a repository, private if the ledger is not public, and push your ledger files to its default branch, with the main file at the root.
2. Create the token.
3. Start Zhang with the variables set:

```shell
docker run --name zhang -d -p 8000:8000 \
  -e ZHANG_DATA_SOURCE=github \
  -e ZHANG_GITHUB_USER=zhang-accounting \
  -e ZHANG_GITHUB_REPO=my-ledger \
  -e ZHANG_GITHUB_TOKEN=github_pat_xxxxxxxx \
  kilerd/zhang:latest
```

## What to know

- Every file the web UI writes is committed separately, with a message such as `Write data/2024/02.zhang at … via opendal`. Recording a transaction in a month that has no file yet writes two files, and so makes two commits: the new month file and the `include` added to the main file.
- Zhang does not watch the repository. After you push changes from elsewhere, use the reload button of the web UI, or restart Zhang. Pull before you edit a local clone, as the web UI may have committed in the meantime.
- The web UI writes new entries, uploaded documents and passkeys to the repository, in the same places as on the [local file system](/deployment/data-sources/local/#layout).
- Plugin modules and the documents the web UI has opened are cached in the `.cache` folder of the working directory, on the machine that runs Zhang, see [The `.cache` folder](/deployment/data-sources/local/#the-cache-folder).

## Troubleshooting

- **Zhang stops at startup with `ZHANG_GITHUB_TOKEN must be set`** (or `ZHANG_GITHUB_USER`, `ZHANG_GITHUB_REPO`): all three variables are required.
- **The web UI shows an empty ledger**: Zhang did not find the main file and started with an empty ledger. Check the owner, the repository name, `--endpoint`, and that the token can read the repository.
- **Recording in the web UI fails**: the token needs write access to the contents of the repository.
