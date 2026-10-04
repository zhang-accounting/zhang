---
title: Document
description: Reference for the document directive, which links a file such as a receipt or statement to an account.
sidebar:
  order: 7
---

A `document` directive links a file, such as a bank statement, a receipt or a contract, to an account on a date. A
`document` metadata entry links a file to a transaction instead. Zhang lists both on the documents page of the web UI
and lets you open the file from there.

## Syntax

```text
YYYY-MM-DD [HH:MM[:SS]] document <Account> "<Path>" [#tag …] [^link …]
```

| Part | Required | Description |
|---|---|---|
| Date and time | yes | The date of the document, optionally with a time of day. |
| `<Account>` | yes | The account the document belongs to. |
| `"<Path>"` | yes | The path of the file: relative to the ledger root in a Zhang file, relative to the file that holds the directive in a Beancount file. See [Paths](#paths). |
| `#tag`, `^link` | no | Tags and links, as on a transaction. |

Metadata lines can follow the directive.

To link a file to a transaction, give the transaction a `document` metadata entry. Repeat the key for several files.
The entry can also be written on one of the postings: it still links the file to the transaction.

```text
YYYY-MM-DD * "<Payee>" "<Narration>"
  document: "<Path>"
  <postings>
```

## Examples

```zhang
2024-01-01 open Assets:Bank:Checking CNY
2024-01-01 open Expenses:Food CNY

2024-01-31 document Assets:Bank:Checking "statements/2024-01.pdf" #statement
  bank: "ACME Bank"

2024-02-03 * "Cafe" "team lunch"
  document: "receipts/2024-02-03-cafe.jpg"
  Assets:Bank:Checking -120.00 CNY
  Expenses:Food
```

## Behavior

- A document changes no balance.
- Files are read through the ledger's [data source](/deployment/data-sources/local/), so documents work the same for
  ledgers on S3, WebDAV or GitHub.

### Paths

- **In a Zhang file**, a relative path is relative to the ledger root, the directory you start `zhang serve` with,
  whichever file holds the directive. In the example, the files are `statements/2024-01.pdf` and
  `receipts/2024-02-03-cafe.jpg` under the ledger root.
- **In a Beancount file**, a relative path is relative to the directory of the file that holds the directive, as
  Beancount reads it: `"../../attachments/a.pdf"` in `data/2026/10.bean` is `attachments/a.pdf` under the ledger root.
  When a file exists under both readings, the one relative to the file is used.
- Earlier versions of Zhang wrote the documents you uploaded into Beancount files with paths relative to the ledger
  root. Zhang still finds them there. On the local disk, it lists a
  [`DocumentPathRelativeToRoot`](/reference/error-codes/#documentpathrelativetoroot) notice on each, with the path to
  write instead, and reports a document of a Beancount file found nowhere as
  [`DocumentNotFound`](/reference/error-codes/#documentnotfound). On a remote data source, Zhang checks nothing when
  it loads the ledger: opening a document looks for it relative to its file first, then relative to the ledger root.
- A `document` metadata entry of a transaction is relative to the ledger root, in both kinds of files.
- In a Zhang file, Zhang does not check that the file exists when it loads the ledger. A path to a missing file is
  listed like any other.

### Opening a document

The web UI opens a document through `/api/documents/<base64 of its path>`, which serves the file at that path, never
a directory:

- a path outside the ledger root, also through a symbolic link on the local disk, is refused with 403, and so is a
  file the storage refuses to read;
- a missing file, a directory or another entry that is no regular file gives 404;
- a path that is not valid base64 or UTF-8, or that holds a line break or a NUL, gives 400.

A document read from a remote data source is kept in `.cache/documents/`, in the directory Zhang runs in, and served
from that copy afterwards. Documents on the local disk are read from the disk each time.

### In the web UI

- The documents page lists every document, newest first: the `document` directives and the `document` metadata of
  transactions. An account's page lists the `document` directives of that account and of its sub-accounts, and
  a transaction's preview shows its documents.
- Uploading a file on an account page saves it as `attachments/<random id>/<file name>` under the ledger root and
  adds a `document` directive dated now, in the file that the
  [`directive_output_path`](/reference/directives/options/#directive_output_path) option selects. In a Beancount file,
  the path is written relative to that file, such as `"../../attachments/<random id>/<file name>"`. The file name must
  be a plain name, without a directory, of at most 255 bytes.
- Uploading a file on a transaction saves it the same way and adds a `document` metadata line right under the
  transaction's header, in the file that holds the transaction.

### In queries

[`#documents`](/reference/query-language/#prices-balances-notes-events-documents-and-commodities) has one row per
`document` directive, then one row per `document` metadata value of a transaction or posting.

## Errors

| Error | When |
|---|---|
| [`AccountDoesNotExist`](/reference/error-codes/#accountdoesnotexist) | The account of a `document` directive is not open at its date. The document is still listed. |
| [`AccountClosed`](/reference/error-codes/#accountclosed) | The account is already closed. The document is still listed. |
| [`DocumentPathRelativeToRoot`](/reference/error-codes/#documentpathrelativetoroot) | A notice: in a Beancount file on the local disk, the path names a file relative to the ledger root only. The document is still listed and opens. |
| [`DocumentNotFound`](/reference/error-codes/#documentnotfound) | In a Beancount file on the local disk, the file is found neither relative to the file of the directive nor relative to the ledger root. |

## Beancount compatibility

The `document` directive has the same syntax in Beancount. Two differences:

- Zhang resolves a relative path in a Beancount file against the directory of the file that declares the directive,
  as Beancount does, and keeps finding the documents earlier versions wrote relative to the ledger root (see
  [Paths](#paths)). In queries, the `filename` column of `#documents` follows Beancount's rule, and the `path` column
  gives the path within the ledger that the web UI opens.
- Beancount's `documents` option, which finds documents in a directory tree by their file names, has no effect in
  Zhang. Write a `document` directive or metadata entry for each file.

## Related

- [Documents](/guides/documents/): attaching receipts and statements.
- [Transaction](/reference/directives/transaction/#metadata): where a transaction's metadata goes.
