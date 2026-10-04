---
title: Local File System
description: Serve a ledger stored on the local file system, and how Zhang picks up changes to the files.
sidebar:
  order: 1
---

The local file system is the default data source (`--source fs`, or `ZHANG_DATA_SOURCE=fs`). Zhang reads the ledger from the folder given to `zhang serve` and writes the entries recorded in the web UI back to it. It is the only data source that notices when you edit the files yourself.

```shell
zhang serve /home/me/ledger
```

With Docker, the folder is the one mounted at `/data`, see [Installation](/getting-started/installation/#docker).

## Layout

Zhang starts from the main file, `main.zhang` unless you pass `--endpoint`, and loads every file it [includes](/reference/directives/include/), directly or through other included files. Paths in an `include` are relative to the file that contains it. A folder served by Zhang can look like this:

```text
ledger/
├── main.zhang             the main file (--endpoint)
├── accounts.zhang         included from main.zhang
├── data/
│   └── 2024/
│       ├── 01.zhang       written by the web UI
│       └── 02.zhang
├── attachments/           documents uploaded in the web UI
│   └── 6f1c…/receipt.pdf
└── .zhang/
    └── passkeys.json      registered passkeys
```

The extension of the main file selects the format of the whole ledger: `.zhang`, or `.bean`, `.beancount` and `.bc` for beancount. If the main file does not exist, Zhang starts with an empty ledger.

The files are UTF-8. A file that starts with a byte order mark (BOM), as some Windows editors write it, is read as if it started without one, and Zhang keeps the mark when it writes the file.

The web UI writes to these files:

- A new transaction, balance check or document is appended to the file that the [`directive_output_path`](/reference/directives/options/) option names for its date, `data/{year}/{month}.zhang` by default (with the extension of the main file). The first time a file is used, Zhang appends an `include` for it to the main file.
- Editing a transaction rewrites it in the file it comes from.
- An uploaded document is stored as `attachments/<random id>/<file name>`, and linked from the account or the transaction.
- **Raw Editing** saves the whole file you edited. It does not overwrite a file that changed since you opened it, by a transaction or balance check recorded in the web UI, an uploaded document or an edit outside: the save is refused, and the editor offers to reload the file (discarding your edits) or to keep editing. Through the API, `GET /api/files/{path}` answers with the `sha256` of the content it serves; a `PUT` that sends it back as `expected_sha256` is refused with 409, writing nothing, when the file no longer matches it, and a `PUT` without it overwrites the file.
- Registering a passkey writes `.zhang/passkeys.json`, see [Authentication](/deployment/authentication/#where-passkeys-are-stored).

## When Zhang reloads

Zhang watches the ledger folder and its subfolders, and reloads the ledger when:

- one of the ledger's files (the main file or a file it includes) is modified;
- a file that a [plugin](/guides/plugins/) depends on changes: its module file when it is inside the ledger folder, or a file or folder the plugin read through its `allowed_paths` (creating, modifying or removing a file in a folder it listed counts too);
- you record or change something in the web UI, which reloads after writing;
- you use the reload button of the web UI;
- the date changes (at midnight, in the ledger's timezone), but only if a plugin read the current date during the load.

Events are grouped: Zhang waits half a second after the first change, then reloads once for everything that changed meanwhile. The web UI refreshes by itself after a reload.

Some changes do not trigger a reload:

- A new file that matches a wildcard include, such as `include "data/*.zhang"`, is loaded with the next reload only. Use the reload button, or save one of the ledger's files.
- Changes in `.zhang/` and `.cache/` inside the ledger folder, and in files that are not part of the ledger.

:::caution[Give an absolute path]
Zhang recognizes the files it loaded by their path. Start it with an absolute path that contains no symbolic links, such as `zhang serve /home/me/ledger` or `zhang serve "$(realpath ledger)"`. With a relative path such as `zhang serve .`, or on macOS a path through a symbolic link such as `/tmp/…`, edits to the ledger files do not trigger a reload, and you have to use the reload button. The Docker image serves `/data`, which is fine.
:::

### When a reload fails

If a file cannot be read at all, for example because of a syntax error, the reload fails and Zhang keeps serving the ledger as it was before the change. The web UI shows no error for this: the reason is only written to the log. Fix the file and save it again. Problems in the books, such as an unbalanced transaction, do not make the reload fail: they are listed in the web UI.

At startup, a ledger that cannot be read makes `zhang serve` exit with code 1.

## File permissions

Zhang needs to read every file of the ledger. To record anything from the web UI, it also needs to write to the ledger folder: it appends to existing files and creates folders and files such as `data/2024/` and `attachments/`. Without write access, recording from the web UI fails while reading keeps working.

The Docker image runs Zhang as `root`, so the files and folders it creates in the mounted folder belong to `root` on the host. Change their owner with `chown` if you edit them as another user.

## The `.cache` folder

Zhang keeps a `.cache` folder in its working directory, which is not necessarily the ledger folder:

- `.cache/plugins/` holds the modules of the [plugins](/guides/plugins/) the ledger declares.
- `.cache/documents/` holds a copy of every document the web UI has opened from a remote data source ([S3](/deployment/data-sources/s3/), [WebDAV](/deployment/data-sources/webdav/) or [GitHub](/deployment/data-sources/github/)), which is served from there afterwards. Documents on the local disk are read from the disk each time. Earlier versions kept copies in `.cache/data/`, which is no longer read.

When you start `zhang serve` from inside the ledger folder, `.cache` appears next to your files, and Zhang ignores the changes it makes there. In the Docker image, the working directory is `/application`, so the cache stays inside the container and starts empty when the container is re-created.

The folder can be deleted while Zhang is stopped. Do so when you replace a document of a remote data source with a new file under the same name: the web UI keeps showing the cached copy otherwise.
