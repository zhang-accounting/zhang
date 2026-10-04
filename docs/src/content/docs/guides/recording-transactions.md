---
title: Recording Transactions
description: Record transactions in ledger files or in the web UI, and learn where new entries are written, how tags and links work, and how to attach metadata.
sidebar:
  order: 1
---

A transaction moves amounts between accounts: your salary arrives, you pay for lunch, you settle a credit card bill. You can write transactions in your ledger files with any text editor, or enter them in the web UI. Either way they end up as plain text in your files.

This guide uses a small household ledger. The full syntax is in [Transaction](/reference/directives/transaction/).

## Write a transaction

```zhang title="data/2024-03.zhang"
2024-03-01 * "ACME Corp" "March salary"
  Assets:Bank:Checking 12,000.00 CNY
  Income:Salary
```

- The header line has the date, a flag, the payee and the narration. `*` marks a completed transaction and `!` one you still want to check.
- Each indented line below it is a **posting**: an account and an amount.
- The postings of a transaction must add up to zero in each commodity. Here `Income:Salary` has no amount, so Zhang gives it whatever balances the transaction: `-12,000.00 CNY`.
- Only one posting of a transaction can leave out its amount, and the other postings must then add up in a single commodity. Otherwise Zhang cannot tell what to fill in: it reports an error and leaves the transaction out of the ledger.
- Amounts may use `,` or `_` to group digits, and simple arithmetic: `-(2 * 420) CNY` is `-840 CNY`.

With a single string after the flag, the string is the narration:

```zhang
2024-03-10 ! "Taxi"
  Assets:Cash -35 CNY
  Expenses:Travel 35 CNY
```

Every account a transaction uses must be opened first with [`open`](/reference/directives/account/). Every commodity must be declared with [`commodity`](/reference/directives/commodity/), unless it is the [operating currency](/reference/directives/options/#operating_currency).

## Tags, links and metadata

```zhang
2024-03-09 * "Hotel" "Two nights in Hangzhou" #trip-hangzhou ^booking-8812
  invoice: "INV-2024-0309"
  Liabilities:CreditCard -840 CNY
  Expenses:Travel
    receipt: "hotel.pdf"
```

- A **tag** (`#trip-hangzhou`) groups transactions by theme, such as a trip or a project. A **link** (`^booking-8812`) ties together the transactions of one event, such as a booking and its refund. Tags and links come after the narration, in any order.
- **Metadata** lines are `key: value` pairs. A line right under the header belongs to the transaction (`invoice`). A line indented deeper than a posting belongs to that posting (`receipt`).
- On the Journals page, selecting a tag or a link filters the journal by it.
- In [queries](/guides/querying/), `'trip-hangzhou' IN tags` selects the tagged postings, `entry_meta('invoice')` reads the transaction's metadata and `meta('receipt')` the posting's.
- A `document` metadata entry attaches a file to the transaction. See [Documents](/guides/documents/).

## Date and time

A date in a Zhang ledger can carry a time of day, as `HH:MM` or `HH:MM:SS`:

```zhang
2024-03-02 12:30 * "Noodle House" "Lunch" #work
  Liabilities:CreditCard -48 CNY
  Expenses:Food 48 CNY
```

- Times are in the ledger's timezone, the [`timezone` option](/reference/directives/options/#timezone). It defaults to the timezone of the machine Zhang runs on.
- Entries are ordered by date and time. An entry without a time counts as the start of its day (`00:00:00`). Entries with the same date and time keep the order of the files.
- The time matters most for [balance assertions](/guides/balances/), which check the balance at the moment they are dated.
- Beancount dates have no time of day. In a beancount ledger, Zhang reads a `time: "12:30:00"` metadata entry as the time, and writes the time that way too.

## Split the ledger into files

As a ledger grows, keep the accounts and each period's transactions in files of their own, and [`include`](/reference/directives/include/) them from the main file:

```zhang title="main.zhang"
option "title" "Household"
option "operating_currency" "CNY"

include "accounts.zhang"
include "data/*.zhang"
```

- A relative path is relative to the file that includes it.
- `*` matches within one directory: `data/*.zhang` reads every `.zhang` file directly in `data/`, but not the files in `data/2024/`. [Include](/reference/directives/include/#wildcards) has the limits of patterns.
- The extension of the main file decides the syntax of every file in the ledger: Zhang syntax for `main.zhang`, beancount syntax for `main.bean`.

## Record a transaction in the web UI

Select **New transaction** at the top of the sidebar, and fill in:

- **Date**: pick the day. A new transaction gets the current time of day.
- **Payee** and **Narration**.
- **Postings**: an account and an amount written as `amount commodity`, such as `-28 CNY`. Leave one amount empty and Zhang fills it in.
- **Metadata**: key and value pairs for the transaction, and for each posting.

The form has no fields for tags, links, costs or prices. For those, write the transaction in a file, for example on the **Raw Editing** page, which edits the ledger files in the browser.

To change a transaction, open the menu of its row on the Journals page and choose **Edit**. Zhang writes the edited transaction back in place, in the file it came from. It rewrites the whole transaction from the form, so comments on its postings are dropped. Transactions with a cost or a price cannot be edited in the form.

### Where new entries are written

Zhang appends what you record in the web UI to a file chosen by the entry's date and the [`directive_output_path` option](/reference/directives/options/#directive_output_path). The default is `data/{{year}}/{{month_str}}.{{ext}}`, where `ext` is the extension of the main file. A coffee recorded for 3 April 2024 goes to `data/2024/04.zhang`:

```zhang title="data/2024/04.zhang"
2024-04-03 13:15:00 * "Coffee Lab" "Flat white"
  Assets:Cash -28 CNY
  Expenses:Food
```

- If the ledger does not read that file yet, Zhang also adds `include "data/2024/04.zhang"` at the end of the main file.
- Everything else the web UI adds as a new entry goes to the same place: balance assertions written on an account page or with the **Batch balance** tool, and documents uploaded to an account.
- A different template changes the layout. `option "directive_output_path" "data/{{year}}.{{ext}}"` keeps one file per year. The template can use `year`, `month`, `month_str`, `day`, `day_str`, `type` (the kind of entry) and `ext`.
- In a beancount ledger the file has the main file's extension, such as `.bean`, and is written in beancount syntax, with the time of day in a `time` metadata entry.

## When something is wrong

Zhang checks the ledger every time it loads it.

- **Problems in an entry** go to the ledger's error list. The **Overview** item in the sidebar shows how many there are, and the Overview page lists them: select one to see the entry it is about. Typical problems are a transaction that does not balance, an account that is not open and a commodity that is not declared. [Error Codes](/reference/error-codes/) explains each one.
  - A transaction that does not balance stays in the ledger, marked **Unbalanced** in the journal.
  - A transaction whose missing amount cannot be filled in, for example one with two postings without an amount, is left out of the ledger until you fix it.
- **A syntax error** stops the whole ledger from loading. When Zhang starts, it exits with the error and the file, line and column where it found it. When the error appears while Zhang is running, Zhang keeps serving the last version that loaded and only logs the error: the web UI shows no error. If your changes do not show up, look at the terminal, or at `docker logs`.

## Reloading

For a ledger on the local disk, `zhang serve` watches the ledger directory and reloads when one of the ledger's files changes, so an edit in your text editor shows up within a second or two. This works only when Zhang is given the ledger directory as an absolute path that does not go through a symbolic link: see [Local File System](/deployment/data-sources/local/). Recording something in the web UI, or saving a file on the Raw Editing page, reloads the ledger too.

The **Reload ledger** button, the circular arrow next to the ledger's title in the sidebar, reloads it on demand. Use it for a ledger on [S3, WebDAV or GitHub](/deployment/data-sources/s3/): there Zhang does not notice changes made by other programs.
