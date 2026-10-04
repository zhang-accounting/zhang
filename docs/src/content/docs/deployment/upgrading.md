---
title: Upgrading
description: Upgrade a Docker or binary installation of Zhang, and what to check before and after.
sidebar:
  order: 6
---

Zhang keeps no data of its own besides your ledger files, so upgrading means replacing the program and restarting it. The files are not converted.

## Before you upgrade

- Back up the ledger folder, or commit it if it is a Git repository. Include `.zhang/passkeys.json` if you use [passkeys](/deployment/authentication/#where-passkeys-are-stored).
- Read the [release notes](https://github.com/zhang-accounting/zhang/releases) of the versions between yours and the new one. Your version is shown next to "Zhang" in the navigation of the web UI and on the **Settings** page.

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

Open the web UI, check the version on the **Settings** page, and look at the error list on the **Overview** page. A newer release can report problems that an older one accepted without a word. When you upgrade from 0.2.0 to a later release, expect in particular:

- Postings to an account that is not open yet, or already closed, are reported as [`AccountDoesNotExist`](/reference/error-codes/#accountdoesnotexist) or [`AccountClosed`](/reference/error-codes/#accountclosed). A typo in an account name shows up this way.
- A failing [balance assertion](/reference/directives/balance/) is still reported as [`AccountBalanceCheckError`](/reference/error-codes/#accountbalancecheckerror), but it no longer moves the account to the asserted amount: balances, reports and pads follow the postings.
- Transactions must balance per commodity, at the commodity's precision. Transactions that mix commodities or prices without balancing are reported as [`UnbalancedTransaction`](/reference/error-codes/#unbalancedtransaction).
- An account that uses an unsupported booking method (`NONE`, `AVERAGE`, `AVERAGE_ONLY`) or an invalid one loads with an error and the default method, instead of stopping the load. See [booking methods](/reference/directives/account/).
- The browser's password popup is replaced by a login page, and passkeys are available. Scripts can keep sending the password as an HTTP Basic header, see [Authentication](/deployment/authentication/).

[Error Codes](/reference/error-codes/) explains how to fix each error.
