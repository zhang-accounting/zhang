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
| `"<Path>"` | yes | The file to read, in double quotes. A relative path is relative to the directory of the file that holds the `include`. Any part of it may contain `*`, see [Wildcards](#wildcards). |

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
- An absolute path is read within the ledger root, which is all the server reads: on the local disk, a path inside
  the ledger's folder is read, and a path outside it names no file. A relative path that climbs out of the ledger
  root with `..`, such as `include "../shared/accounts.zhang"` in the main file, names no file either.
- A file has one name however a path spells it: `accounts.zhang`, `./accounts.zhang` and `data/../accounts.zhang` in
  the main file are the same file, read once.
- A file that does not exist is an [`IncludeNotFound`](/reference/error-codes/#includenotfound) error on the
  `include`, and the rest of the ledger loads without it. It is not listed in the file list of the web UI.

### Wildcards

`*` stands for any run of characters other than `/`, within one part of the path, and can be used in any part, more
than once in a part if needed (`2024-*-*.zhang`). Every other character is taken literally, and a part matches a
whole name: `*.zhang` matches `01.zhang`, not `01.zhang.bak`, and `report(*).zhang` matches `report(1).zhang`.

- `data/*.zhang` matches the files directly in `data`, and `*.zhang` in the main file at the ledger root matches
  the files next to it.
- `data/*/*.zhang` matches the files one directory below `data`. A part without `*` can come before or after a part
  with one, as in `data/*/archive/*.zhang` or `data/*/accounts.zhang`.
- The last part names files, the parts before it directories. A `*` at the start of a part does not match a hidden
  name, one starting with `.`, as in a shell: `*.zhang` leaves an editor's `.#01.zhang` out.
- The matching files are read in the order of their names.
- A pattern that matches no file is an [`IncludeNotFound`](/reference/error-codes/#includenotfound) error on the
  `include`, as for a file that does not exist.

### Files read once

Each file is read once, however many times it is included. A file that includes the main file, or two files that
include each other, are not a problem.

Directives are ordered by their dates, so the order of files only matters for undated directives. Zhang reads the
main file first, then the files it includes in order, then the files those include, and so on. When the same option
is set in several files, the value read last wins.

### Reloading

With a ledger on the local disk, `zhang serve` watches the ledger root and reloads the ledger when one of its files
changes, or when a missing included file is created. A new file that matches a pattern is read at the next reload:
when a file of the ledger changes, or when you choose **Reload ledger** in the web UI. Ledgers on S3, WebDAV or
GitHub are not watched; reload them from the web UI. Includes and patterns work the same on every data source.

### New entries from the web UI

The web UI writes new entries into the file that the
[`directive_output_path`](/reference/directives/options/#directive_output_path) option selects. If that file is not
part of the ledger yet, Zhang creates it and appends an `include` of it to the main file.

## Errors

| Error | When |
|---|---|
| [`IncludeNotFound`](/reference/error-codes/#includenotfound) | No file is at the path, the path is outside the ledger root (an absolute path elsewhere, or a relative one climbing out with `..`), or no file matches the pattern. The error points at the `include`, and the rest of the ledger loads. |

A file that cannot be parsed stops the ledger from loading, with an error naming the file, line and column. So does an
included file that is not UTF-8 text, with an error naming the file and the line of the first byte that is not UTF-8.

## Beancount compatibility

Beancount's `include` has the same syntax and also accepts patterns. Zhang differs:

- Beancount's patterns follow Python's glob rules. Zhang's `*` works the same way, but `?` and `[...]` are not
  special in Zhang: they match themselves.

## Related

- [Local file system](/deployment/data-sources/local/): where the ledger root is and how it is watched.
- [Options](/reference/directives/options/#directive_output_path): where new entries are written.
