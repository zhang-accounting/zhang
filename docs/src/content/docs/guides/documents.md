---
title: Documents
description: Attach receipts and statements to accounts and transactions, from ledger files or by uploading them in the web UI.
sidebar:
  order: 5
---

Keep the paper trail next to the numbers: attach bank statements to accounts and receipts to transactions. The files stay with your ledger, and the web UI shows them where they belong.

## Attach a statement to an account

Put the file in the ledger directory and point a [`document`](/reference/directives/document/) directive at it:

```zhang
2024-02-01 document Assets:Bank:Checking "statements/2024/2024-01-bank.pdf" #statement
```

- In a Zhang ledger, the path is relative to the ledger's root directory, the directory `zhang serve` serves. In a beancount ledger, it is relative to the file that holds the directive, as beancount reads it. Keep documents inside the ledger's root: Zhang does not open a file outside, also not through a symbolic link. See [Paths](/reference/directives/document/#paths).
- The date is the document's date, for example the day the statement was issued. Tags and links can follow the path.
- The account must be open on that date.

## Attach a receipt to a transaction

Add a `document` metadata entry to the transaction, once per file:

```zhang
2024-03-09 * "Hotel" "Two nights in Hangzhou"
  document: "receipts/hotel-0309.jpg"
  Liabilities:CreditCard -840 CNY
  Expenses:Travel
```

A `document` entry on one of the postings also belongs to the transaction.

The path of a `document` metadata entry is relative to the ledger's root, in either kind of ledger. Opening a document whose file does not exist tells you so. In a beancount ledger on the local disk, a `document` directive whose file does not exist is also reported as [`DocumentNotFound`](/reference/error-codes/#documentnotfound).

## Upload in the web UI

You can upload files from three places:

- the **Documents** page: select **Upload**, choose the account and the files;
- an account's page, on its **Documents** tab: drop files on the upload tile;
- a transaction's preview on the Journals page: drop files on the upload tile under its documents.

Zhang stores each uploaded file as `attachments/<random id>/<file name>` under the ledger root, and records it in the ledger:

- For an account, it adds a `document` directive dated now, in the file the [`directive_output_path`](/guides/recording-transactions/#where-new-entries-are-written) option names:

  ```zhang
  2024-04-03 21:55:03 document Assets:Bank:Checking "attachments/78e12a54-d9e5-4de5-9de3-f140308e1c79/scan.pdf"
  ```

  In a beancount ledger, the path is written relative to that file, such as `"../../attachments/78e12a54-d9e5-4de5-9de3-f140308e1c79/scan.pdf"` in `data/2024/04.bean`, so beancount finds the file too.

- For a transaction, it adds a `document:` line right under the transaction's header, in the file the transaction is written in.

A file name must be a plain name, without a directory, of at most 255 bytes. The `attachments/` directory is fixed; no option changes it. On a ledger stored on [S3, WebDAV or GitHub](/deployment/data-sources/s3/), uploads are written to the same place in that storage, and documents are read from it.

## View documents

- The **Documents** page lists every document, newest first, as a grid or a list. The grid shows a document's account, or its transaction when it has no account; the list shows both. A document written on a posting has the posting's account and its transaction. The documents of one transaction come in the order they are written. The list's **Type** column shows a document's file extension, such as `PDF`. Images in a format browsers display (PNG, JPEG, GIF, WebP, AVIF and BMP) can be previewed; other files open in a new tab.
- An account's **Documents** tab lists the `document` directives of that account and of its sub-accounts. A document named in a transaction's `document:` metadata shows on the **Documents** page and in the transaction's preview, not on an account's page.
- A transaction's preview on the Journals page shows its documents, and the journal marks the transactions that have some.

For a ledger on a remote data source, Zhang keeps a copy of each document it has shown in `.cache/documents/`, in the directory it runs in, and serves that copy from then on. If you replace a file but keep its name, delete the copy, or give the new file another name. If Zhang cannot write to that directory, it serves the document anyway, without keeping a copy, and says so in its log. Documents on the local disk are read from the disk each time. Copies that earlier versions kept in `.cache/data/` are no longer used; you can delete that directory.

Documents are also available to [queries](/guides/querying/), in the `#documents` table.
