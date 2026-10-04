---
title: WebDAV
description: Store the ledger on a WebDAV server, such as Nextcloud, ownCloud or a NAS.
sidebar:
  order: 3
---

WebDAV is a protocol for managing files on a remote server. Many file hosting services and NAS systems offer it, for example Nextcloud, ownCloud or Synology. Zhang can read and write the ledger files directly on a WebDAV server.

## Configuration

Select the data source with `--source web-dav` or `ZHANG_DATA_SOURCE=web-dav`, and configure it with environment variables:

| Environment variable | Required | Example | Description |
| --- | --- | --- | --- |
| `ZHANG_WEBDAV_ENDPOINT` | yes | `https://dav.example.com/dav` | The URL of the WebDAV server. |
| `ZHANG_WEBDAV_ROOT` | yes | `/accounting` | The folder of the ledger on the server, relative to the endpoint. |
| `ZHANG_WEBDAV_USERNAME` | no | `your_username` | The username, when the server requires one. |
| `ZHANG_WEBDAV_PASSWORD` | no | `your_password` | The password. Many services let you create an app password for this. |

The `<PATH>` argument of `zhang serve` is ignored: the ledger is the folder `ZHANG_WEBDAV_ROOT`. The main file is still selected with `--endpoint` (default `main.zhang`), relative to that folder. With the example values, Zhang reads `https://dav.example.com/dav/accounting/main.zhang`.

## Setup

1. Upload your ledger files to a folder on the WebDAV server.
2. Start Zhang with the variables set:

```shell
docker run --name zhang -d -p 8000:8000 \
  -e ZHANG_DATA_SOURCE=web-dav \
  -e ZHANG_WEBDAV_ENDPOINT=https://dav.example.com/dav \
  -e ZHANG_WEBDAV_ROOT=/accounting \
  -e ZHANG_WEBDAV_USERNAME=your_username \
  -e ZHANG_WEBDAV_PASSWORD=your_password \
  kilerd/zhang:latest
```

or, with the binary:

```shell
ZHANG_DATA_SOURCE=web-dav \
ZHANG_WEBDAV_ENDPOINT=https://dav.example.com/dav \
ZHANG_WEBDAV_ROOT=/accounting \
ZHANG_WEBDAV_USERNAME=your_username \
ZHANG_WEBDAV_PASSWORD=your_password \
zhang serve .
```

## What to know

- Zhang does not watch the server. When the files change outside Zhang, for example through a sync client, use the reload button of the web UI, or restart Zhang. What you record in the web UI is written to the server and reloaded right away.
- The web UI writes new entries, uploaded documents and passkeys to the server, in the same places as on the [local file system](/deployment/data-sources/local/#layout).
- Plugin modules and the documents the web UI has opened are cached in the `.cache` folder of the working directory, on the machine that runs Zhang, see [The `.cache` folder](/deployment/data-sources/local/#the-cache-folder).

## Troubleshooting

- **Zhang stops at startup with `ZHANG_WEBDAV_ENDPOINT must be set` or `ZHANG_WEBDAV_ROOT must be set`**: both variables are required.
- **Authentication failures**: check the username and the password. If the service uses two-factor authentication, create an app password.
- **The web UI shows an empty ledger**: Zhang did not find the main file and started with an empty ledger. Check `ZHANG_WEBDAV_ENDPOINT`, `ZHANG_WEBDAV_ROOT` and `--endpoint`.
