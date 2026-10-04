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
| `"<Path>"` | yes | The path of the file, relative to the ledger root. |
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

- **Paths are relative to the ledger root**, the directory you start `zhang serve` with, whichever file holds the
  directive. In the example, the files are `statements/2024-01.pdf` and `receipts/2024-02-03-cafe.jpg` under the
  ledger root. The web UI cannot open a file outside the ledger root.
- Zhang does not check that the file exists when it loads the ledger. A path to a missing file is listed like any
  other, and opening it fails.
- A document changes no balance.
- Files are read through the ledger's [data source](/deployment/data-sources/local/), so documents work the same for
  ledgers on S3, WebDAV or GitHub.

### In the web UI

- The documents page lists every document, newest first: the `document` directives and the `document` metadata of
  transactions, a transaction's in the order written. A document written on a posting shows the posting's account next
  to its transaction. An account's page lists the documents of that account, and a transaction's preview shows its
  documents.
- Uploading a file on an account page saves it as `attachments/<random id>/<file name>` under the ledger root and
  adds a `document` directive dated now, in the file that the
  [`directive_output_path`](/reference/directives/options/#directive_output_path) option selects.
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

## Beancount compatibility

The `document` directive has the same syntax in Beancount. Two differences:

- Beancount resolves a relative path against the directory of the file that declares the directive. Zhang's web UI
  resolves it against the ledger root. Both agree for directives in files at the ledger root. In queries, the
  `filename` column of `#documents` follows Beancount's rule and the `path` column Zhang's.
- Beancount's `documents` option, which finds documents in a directory tree by their file names, has no effect in
  Zhang. Write a `document` directive or metadata entry for each file.

## Related

- [Documents](/guides/documents/): attaching receipts and statements.
- [Transaction](/reference/directives/transaction/#metadata): where a transaction's metadata goes.
