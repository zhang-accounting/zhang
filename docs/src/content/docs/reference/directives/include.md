---
title: Include
description: Reference for the include directive, which splits a ledger across several files, wildcard patterns included.
sidebar:
  order: 10
---

An `include` directive reads another file into the ledger, so a ledger can be split into several files, for example
one per year or month. A pattern with `*` includes every matching file.

## Syntax

```text
include "<Path>"
```

| Part | Required | Description |
|---|---|---|
| `"<Path>"` | yes | The file to read, in double quotes. A relative path is relative to the directory of the file that holds the `include`. It may contain `*` in its last part. |

An `include` has no date and no metadata. It can be written in any file of the ledger, at any position.

## Examples

```zhang
include "accounts.zhang"
include "data/2024/*.zhang"
include "data/*/*.zhang"
```

With a main file at the ledger root, these lines read `accounts.zhang` at the root, every file of `data/2024` whose
name matches `*.zhang`, and the matching files of every directory under `data`.

## Behavior

### Paths

- A relative path is resolved against the directory of the file that holds the `include`: `include "sibling.zhang"`
  in `data/2024.zhang` reads `data/sibling.zhang`.
- Every included file is read in the format of the main file, whatever its own extension: a ledger whose main file is
  `main.zhang` reads every file as zhang text, one whose main file is `main.bean` reads every file as Beancount text.
- A file that does not exist is read as an empty file, without an error. It still appears in the file list of the
  web UI. Check the path when the entries of an included file do not show up.

### Wildcards

`*` stands for one or more characters other than `/`, within one part of the path:

- `data/*.zhang` matches the files directly in `data`.
- `data/*/*.zhang` matches the files one directory below `data`. A directory part without `*` can follow a part with
  one, as in `data/*/archive/*.zhang`.
- A pattern that matches no file includes nothing, without an error.

:::caution[Limits of wildcards]
- The last part of the path, the file name, must contain `*`. A pattern with `*` only in a directory part, such as
  `data/*/accounts.zhang`, stops Zhang from loading the ledger.
- The pattern must point into a sub-directory of the ledger root. `include "*.zhang"` in the main file at the root
  also stops Zhang from loading the ledger. Move the files into a directory and include `that-directory/*.zhang`.
- A pattern also matches longer names that contain a match: `*.zhang` matches `01.zhang.bak` too, and the `.` in
  it matches any character. Keep other files out of the directories you include with a pattern.
:::

### Files read once

Each file is read once, however many times it is included. A file that includes the main file, or two files that
include each other, are not a problem.

Directives are ordered by their dates, so the order of files only matters for undated directives. Zhang reads the
main file first, then the files it includes in order, then the files those include, and so on. When the same option
is set in several files, the value read last wins.

### Reloading

With a ledger on the local disk, `zhang serve` watches the ledger root and reloads the ledger when one of its files
changes. A new file that matches a pattern, or a missing included file that is created, is read at the next reload:
when a file of the ledger changes, or when you choose **Reload ledger** in the web UI. Ledgers on S3, WebDAV or
GitHub are not watched; reload them from the web UI. Includes and patterns work the same on every data source.

### New entries from the web UI

The web UI writes new entries into the file that the
[`directive_output_path`](/reference/directives/options/#directive_output_path) option selects. If that file is not
part of the ledger yet, Zhang creates it and appends an `include` of it to the main file.

## Errors

An `include` produces no ledger error. A file that cannot be parsed stops the ledger from loading, with an error
naming the file, line and column.

## Beancount compatibility

Beancount's `include` has the same syntax and also accepts patterns. Zhang differs:

- Beancount reports an `include` that matches no file. Zhang reads a missing file as empty, without an error.
- Beancount's patterns follow Python's glob rules. Zhang's `*` works only within the limits above.

## Related

- [Local file system](/deployment/data-sources/local/): where the ledger root is and how it is watched.
- [Options](/reference/directives/options/#directive_output_path): where new entries are written.
