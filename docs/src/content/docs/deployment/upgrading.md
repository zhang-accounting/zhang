---
title: Upgrading
description: Upgrade a Docker or binary installation of Zhang, and what to check before and after.
sidebar:
  order: 6
---

Zhang keeps no data of its own besides your ledger files, and a [cache of compiled plugin code](/guides/plugins/#compiled-code) it rebuilds when needed, so upgrading means replacing the program and restarting it. The files are not converted.

## Before you upgrade

- Back up the ledger folder, or commit it if it is a Git repository. Include `.zhang/passkeys.json` if you use [passkeys](/deployment/authentication/#where-passkeys-are-stored).
- Read the [release notes](https://github.com/zhang-accounting/zhang/releases) or the [changelog](https://github.com/zhang-accounting/zhang/blob/main/CHANGELOG.md) of the versions between yours and the new one, and the guide for your upgrade: [from 0.2 to 0.3](/deployment/upgrading-to-0-3/). Your version is shown next to "Zhang" in the navigation of the web UI and on the **Settings** page.

## Docker

Pull the new image and re-create the container with the same options (volume, ports, environment variables and arguments):

```shell
docker pull kilerd/zhang:latest
docker stop zhang
docker rm zhang
docker run --name zhang -d -v "/path/to/ledger:/data" -p "8000:8000" kilerd/zhang:latest
```

With Docker Compose:

```shell
docker compose pull
docker compose up -d
```

If you pinned a version, such as `kilerd/zhang:0.2.0`, change the tag to the new version instead. The ledger lives in the mounted folder and is not affected by removing the container.

## Release binaries

`zhang update` replaces the binary with the latest release for your platform from GitHub:

```shell
zhang update --verbose
```

With `--verbose`, it shows the current and the latest version and the download progress. When a newer release exists, it asks for confirmation, then replaces the executable. It needs write access to the file, so run it with `sudo` if the binary is in a system folder such as `/usr/local/bin`. Restart `zhang serve` afterwards: a running server keeps the old version until it is restarted.

You can also download the new binary from the [releases page](https://github.com/zhang-accounting/zhang/releases) and replace the old one, see [Installation](/getting-started/installation/#release-binaries).

A binary you built yourself is replaced by the release binary too. To stay on a source build, pull the new sources and [build again](/getting-started/installation/#building-from-source).

## The update notice

While the web UI is open in a browser, the server checks GitHub for a newer release every minute. When there is one, the navigation shows "Version X is available" with a link to this page. The check does not depend on `--no-report`, which only stops the anonymous usage report.

## After upgrading

Open the web UI, check the version on the **Settings** page, and look at the error list on the **Overview** page. A newer release can report problems that an older one accepted without a word: [Upgrading from 0.2 to 0.3](/deployment/upgrading-to-0-3/) lists those 0.3 reports. [Error Codes](/reference/error-codes/) explains how to fix each error.
