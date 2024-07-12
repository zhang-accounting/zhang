---
title: GitHub as a Data Source
description: Detailed guide on configuring and using GitHub as a data source in Zhang Accounting.
---

## Overview

GitHub, a widely used platform for code hosting and version control, can also serve as a robust data source for Zhang Accounting. This guide provides an in-depth look at how to configure GitHub for storing and managing your accounting data, leveraging GitHub's Restful API for seamless data access and manipulation.

### Key GitHub APIs Utilized

Zhang Accounting integrates with GitHub through the following Restful APIs:

- [Get repository content](https://docs.github.com/en/rest/repos/contents?apiVersion=2022-11-28#get-repository-content): Retrieves files and directories in the repository.
- [Create or update file contents](https://docs.github.com/en/rest/repos/contents?apiVersion=2022-11-28#create-or-update-file-contents): Allows adding new files or updating existing ones.
- [Delete a file](https://docs.github.com/en/rest/repos/contents?apiVersion=2022-11-28#delete-a-file): Enables file deletion within the repository.

### Required Permissions

To ensure smooth operation, your GitHub Token must include the following permissions:

- `contents:read`: Permission to read repository contents.
- `contents:write`: Permission to create, update, or delete repository contents.

### Configuration Parameters

To set up GitHub as your data source, the following parameters are required:

| Parameter | Command Line Argument | Environment Variable | Required | Value | Remarks |
|-----------|-----------------------|----------------------|----------|-------|---------|
| Data Source | source | ZHANG_DATA_SOURCE | Yes | `github` | Identifies GitHub as the data source. |
| GitHub Username | N/A | ZHANG_GITHUB_USER | Yes | | Your GitHub username, e.g., `zhang-accounting`. |
| GitHub Repository Name | N/A | ZHANG_GITHUB_REPO | Yes | | The repository name, e.g., `ledger-test`. |
| GitHub Token | N/A | ZHANG_GITHUB_TOKEN | Yes | | Your GitHub access token, e.g., `ght_123123123`. |

### Docker Command for Setup

To start Zhang Accounting with GitHub as the data source, use the following Docker command:

```shell
docker run --name zhang \
-e "ZHANG_DATA_SOURCE=github" \
-e "ZHANG_GITHUB_USER=zhang-accounting" \
-e "ZHANG_GITHUB_REPO=ledger-test" \
-e "ZHANG_GITHUB_TOKEN=ght_12321321" \
kilerd/zhang:0.1
```

## Common Configurations and Troubleshooting

### Cloning a Repository

To clone your accounting data repository for local editing or backup, use the standard `git clone` command with your repository's URL.

### Handling Merge Conflicts

Merge conflicts may arise when multiple users update the data simultaneously. Resolve these by manually editing the conflicting files and committing the changes.

### Token Security

Keep your GitHub token secure to prevent unauthorized access. If compromised, regenerate the token immediately through GitHub's settings.

### Connectivity Issues

Ensure your network allows access to GitHub's API endpoints. Use tools like `curl` or `ping` to test connectivity.

For more detailed examples and advanced configurations, refer to the official GitHub documentation and Zhang Accounting's community forums.
