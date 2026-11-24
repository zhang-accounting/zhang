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

Zhang starts from the main file, `main.zhang` unless you pass `--endpoint`, and loads every file it [includes](/reference/directives/include/), directly or through other included files. Paths in an `include` are relative to the file that contains it. Zhang reads and writes only inside the folder: an `include` of a file outside it, by an absolute path or by a relative one climbing out with `..`, is reported as [`IncludeNotFound`](/reference/error-codes/#includenotfound), and the rest of the ledger loads. A folder served by Zhang can look like this:

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

The extension of the main file selects the format of the whole ledger: `.zhang`, or `.bean`, `.beancount` and `.bc` for beancount. Every file the ledger includes is read in that format, and its rules apply to all of them, whatever their own extension. Any other extension of the main file, or none, is an error that names the main file, and `zhang serve` exits with code 1. If the main file does not exist, Zhang starts with an empty ledger.

The files are UTF-8. A file that starts with a byte order mark (BOM), as some Windows editors write it, is read as if it started without one, and Zhang keeps the mark when it writes the file. A file of the ledger that is not UTF-8, such as one saved as Latin-1 or GBK, stops the load with an error that names the file and the line of the first byte that is not UTF-8; Zhang never reads it with that byte replaced. Save the file as UTF-8 to fix it. See [When the ledger does not load](/reference/error-codes/#when-the-ledger-does-not-load).

The web UI writes to these files:

- A new transaction, balance check or document is appended to the file that the [`directive_output_path`](/reference/directives/options/) option names for its date, `data/{year}/{month}.zhang` by default (with the extension of the main file). The first time a file is used, Zhang appends an `include` for it to the main file.
- Editing a transaction rewrites it in the file it comes from.
- An uploaded document is stored as `attachments/<random id>/<file name>`, and linked from the account or the transaction.
- **Raw Editing** saves the whole file you edited. It does not overwrite a file that changed since you opened it, by a transaction or balance check recorded in the web UI, an uploaded document or an edit outside: the save is refused, and the editor offers to reload the file (discarding your edits) or to keep editing. Through the API, `GET /api/files/{path}` answers with the `sha256` of the content it serves; a `PUT` that sends it back as `expected_sha256` is refused with 409, writing nothing, when the file no longer matches it, and a `PUT` without it overwrites the file. A `GET` of a file that does not exist is answered with 404, except for the main file of a ledger started without one, which is served empty so that you can write it. A `GET` of a file that is not UTF-8 text, such as an image in the ledger folder, is answered with 415 and a message naming the file. A file the storage refuses to read or write, such as one without read or write permission for the user Zhang runs as, is answered with 403 and the message `the storage refused to read <file>` (or `to write`), on every route that reads or writes it: the editor, an edit in place, a recorded transaction or balance, and a document download.
- Registering a passkey writes `.zhang/passkeys.json`, see [Authentication](/deployment/authentication/#where-passkeys-are-stored).

## When Zhang reloads

Zhang watches the ledger folder and its subfolders, and reloads the ledger when:

- one of the ledger's files (the main file or a file it includes) is modified;
- a file an `include` names, which did not exist, is created;
- a file that a [plugin](/guides/plugins/) depends on changes: its module file when it is inside the ledger folder, or a file or folder the plugin read through its `allowed_paths` (creating, modifying or removing a file in a folder it listed counts too);
- you record or change something in the web UI, which reloads after writing;
- you use the reload button of the web UI;
- the date changes (at midnight, in the ledger's timezone), but only if a plugin read the current date during the load.

Events are grouped: Zhang waits until nothing else changed for half a second (at most two seconds after the first change), then reloads once for everything that changed meanwhile. Reading a file is not a change. The web UI refreshes by itself after a reload. A reload does not hold up the web UI: until the new ledger is loaded, every page is answered from the ledger as it was, and the new one replaces it at once. A change to the ledger's files that leaves them holding exactly what the ledger was loaded from, as after something recorded in the web UI (which reloads before it answers), loads nothing again.

Some changes do not trigger a reload:

- A new file that matches a wildcard include, such as `include "data/*.zhang"`, is loaded with the next reload only. Use the reload button, or save one of the ledger's files.
- Changes in `.zhang/` and `.cache/` inside the ledger folder, and in files that are not part of the ledger.
- Edits made by another machine to a ledger folder on a network mount (NFS, SMB, a folder shared from a NAS): file watching only sees the changes made through the machine Zhang runs on. Reload with the reload button, or with `POST /api/reload`, after such an edit; a script can call it when it is done writing.

The folder can be named any way: `zhang serve .`, a relative path such as `zhang serve ../ledger`, or a path through a symbolic link such as `/tmp/…` on macOS. Zhang resolves it to its real location when it starts, and watches that.

### When a reload fails

If a file cannot be read at all, for example because of a syntax error, the reload fails and Zhang keeps serving the ledger as it was before the change. The web UI shows a notice with the reason at the top of every page, with a link to the file editor, and the reload button reports it too; the reason is also written to the log. Fix the file and save it again: the notice goes away once a reload succeeds. A transaction, balance or document recorded in the web UI while the files cannot be loaded writes nothing: it reloads the files first, and the reload's failure is the answer (HTTP 409), shown the same way. Problems in the books, such as an unbalanced transaction, do not make the reload fail: they are listed in the web UI.

At startup, a ledger that cannot be read makes `zhang serve` exit with code 1.

### When the folder cannot be watched

On Linux, Zhang watches the folder with inotify, whose limits are per user and shared by every program the user runs, on a server, and on a Docker host or Kubernetes node by every container running as that user: `fs.inotify.max_user_instances`, the number of programs that watch (often 128), and `fs.inotify.max_user_watches`, the number of folders watched. When one is used up, Zhang still serves the ledger, and logs one error that names the setting to raise, such as:

```text
cannot watch /data for edits made outside the server: Too many open files (os error 24): the inotify instances of the user the server runs as, or the server's open files, are used up; raise fs.inotify.max_user_instances (`sysctl -w fs.inotify.max_user_instances=1024`), or the open-file limit (`ulimit -n`). Until it can, the server checks the ledger's files for changes every 5s, and tries to watch the folder again every 30s at most
```

While the folder is not watched, `GET /api/info` answers the same reason as `watch_failure` (`null` while the folder is watched), and Zhang checks for changes every 5 seconds instead: the ledger's files and the files its plugins read, by their size, modification time and, on Linux and macOS, their inode and its change time, so that a save by an editor that replaces the file is noticed too (no file is read, so this costs next to nothing). An edit made outside then reloads the ledger within 5 seconds; of a folder a plugin lists, only a file added to it or removed from it is noticed. Zhang tries to watch the folder again after 1 second, then after waits that double, up to 30 seconds. Once it can, it logs `watching <folder>`, reloads the ledger once for what changed meanwhile, and reloads at once after an edit again. The same happens when a folder created in the ledger folder cannot be watched, and when the ledger folder itself is removed or moved away.

Raise the limits where the container runs (the host or the Kubernetes node: a container cannot change them), for example with a file `/etc/sysctl.d/90-inotify.conf`:

```text
fs.inotify.max_user_instances = 1024
fs.inotify.max_user_watches = 524288
```

and `sudo sysctl --system`. To see which programs hold the inotify instances, run `sudo find /proc/*/fd -lname 'anon_inode:inotify' | cut -d/ -f3 | sort | uniq -c | sort -rn`: it counts them by process id.

## File permissions

Zhang needs to read every file of the ledger. To record anything from the web UI, it also needs to write to the ledger folder: it appends to existing files and creates folders and files such as `data/2024/` and `attachments/`. Without write access, recording from the web UI fails while reading keeps working.

The Docker image runs Zhang as `root`, so the files and folders it creates in the mounted folder belong to `root` on the host. Change their owner with `chown` if you edit them as another user. A file it rewrites keeps its owner.

## How Zhang writes a file

Zhang replaces a file whole or not at all. It writes the new content to a temporary file in the same folder, named `.<file name>.<16 hex digits>.tmp`, flushes it to the disk, and renames it over the file. A crash, a power cut or a full disk in the middle of a write leaves the file as it was, never cut short. Each file is replaced on its own: a write that changes two files, such as the first entry of a month (the new month file and its `include` in the main file), can be interrupted between them.

- A file that is a symbolic link stays a link: the file it points to gets the new content.
- The new file keeps the permissions of the old one and its owner and group. It does not keep what else the file system attached to the old one: extended attributes, ACLs, macOS Finder tags and the creation time are lost.
- If Zhang is stopped in the middle of a write, a `.<file name>.<hex>.tmp` file can be left in the folder. It is not part of the ledger, and you can delete it.

Zhang writes the file in place instead, as earlier versions did, when the atomic write would do something the plain write would not:

- The file has more than one hard link, as some sync or backup tools make: renaming over it would leave the other names with the old content.
- The folder does not let Zhang create or rename the temporary file (no write permission on the folder), or the file system cannot rename it there.
- The temporary file cannot get the file's owner or permissions: a file of another user that Zhang may write, such as a group-writable one, which keeps its owner this way, or a file system that does not let Zhang set an owner or a mode, such as some network (SMB) or FUSE mounts and rootless containers.

A file Zhang may not write, such as a read-only file or another user's file without write permission for Zhang, is refused (403, `the storage refused to write <file>`), even where the folder would let Zhang replace it. A full disk, a full quota or a disk error is never retried in place: the file stays as it was, and the error is answered.

## The `.cache` folder

Zhang keeps a `.cache` folder in its working directory, which is not necessarily the ledger folder:

- `.cache/documents/` holds a copy of every document the web UI has opened from a remote data source ([S3](/deployment/data-sources/s3/), [WebDAV](/deployment/data-sources/webdav/) or [GitHub](/deployment/data-sources/github/)), which is served from there afterwards. Documents on the local disk are read from the disk each time. Earlier versions kept copies in `.cache/data/`, which is no longer read.
- Earlier versions also kept a copy of every [plugin](/guides/plugins/) module in `.cache/plugins/`. Plugin modules are now read from the ledger's storage on every load, and that folder is no longer written or read: you can delete it.

When you start `zhang serve` from inside the ledger folder, `.cache` appears next to your files, and Zhang ignores the changes it makes there. In the Docker image, the working directory is `/application`, so the cache stays inside the container and starts empty when the container is re-created.

The folder can be deleted while Zhang is stopped. Do so when you replace a document of a remote data source with a new file under the same name: the web UI keeps showing the cached copy otherwise.
