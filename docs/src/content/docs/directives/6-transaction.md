---
title: Transaction
description: Write transactions, their postings and the metadata of each.
---

# Transactions

A transaction moves amounts between accounts. It is a header line with the date, an optional flag, the payee and the narration, followed by one indented line per posting.

## Basic Syntax

```zhang
{DATE} {FLAG} "{PAYEE}" "{NARRATION}" #{TAG} ^{LINK}
  {ACCOUNT} {AMOUNT} {COMMODITY}
  {ACCOUNT} {AMOUNT} {COMMODITY}
```

```zhang
2024-01-02 * "Cafe" "lunch" #trip
  Assets:Cash -10 CNY
  Expenses:Food 10 CNY
```

- The flag is `*` for a completed transaction and `!` for one to check. A single string after the flag is the narration.
- One posting may leave out its amount: it takes whatever balances the transaction.
- A line starting with `;`, `#`, `*` or `//` inside a transaction is a comment.

## Metadata

Metadata lines are `key: value` pairs. A transaction has its own metadata, and each posting can have metadata of its own:

```zhang
2024-01-02 * "Cafe" "lunch"
  invoice: "2024-001"
  Assets:Cash -10 CNY
    receipt: "r-17"
  Expenses:Food 10 CNY
    category: "meals"
```

Here `invoice` belongs to the transaction, `receipt` to the `Assets:Cash` posting and `category` to the `Expenses:Food` posting.

### Which lines belong to a posting

In a zhang file (`.zhang`), a metadata line belongs to the posting above it **only when it is indented deeper than that posting's line**. Every other metadata line belongs to the transaction, wherever it is: before the postings, between them or after them.

```zhang
2024-01-02 * "Cafe" "lunch"
  Assets:Cash -10 CNY
    receipt: "r-17"      ; deeper than the posting: the posting's
  Expenses:Food 10 CNY
  invoice: "2024-001"    ; at the postings' indentation: the transaction's
```

Older versions of Zhang wrote transaction metadata after the postings, at the same indentation as the postings, so with this rule existing zhang ledgers keep their meaning. When you indent with tabs, a tab counts up to the next multiple of four columns.

In a Beancount file (`.bean`, `.bc` or `.beancount`), Zhang follows Beancount instead: metadata before the first posting belongs to the transaction, and **every metadata line after a posting belongs to that posting, however it is indented**. This is how Beancount and Fava read the file. See [Launching with Beancount Data](/installation/2-beancount_launch/#posting-metadata).

### How Zhang writes metadata

When Zhang writes a transaction (for example when you create or edit one in the web UI), it writes the transaction's metadata right after the header, then each posting followed by its own metadata, indented two levels deeper:

```zhang
2024-01-02 * "Cafe" "lunch"
  invoice: "2024-001"
  Assets:Cash -10 CNY
    receipt: "r-17"
  Expenses:Food 10 CNY
```

Zhang reads this layout the same way in both file formats, and so do Beancount and Fava. A key that is not a single word, such as `"my key"`, is written in quotes. Beancount has no quoted keys, so in a Beancount ledger the web UI only takes a new key that Beancount can read.

### Using metadata

- Zhang's API returns the metadata of each posting with the posting, next to the transaction's own metadata, and takes both when a transaction is created or edited.
- In [queries](/user-guide/query-language/#metadata-functions), `meta('key')` reads the posting's metadata, `entry_meta('key')` the transaction's, and `any_meta('key')` the posting's and then the transaction's. The `meta` column of the postings table holds the posting's metadata as text.
- A `document` metadata entry links a file to the transaction, whether it is written on the transaction or on one of its postings.

## Plugins

WASM plugins receive and return transactions with the metadata of each posting in the `meta` field of the posting. A plugin built against an older version of Zhang, from before posting metadata, still works, but it reads and writes back every directive it is given, so **every** transaction that passes through it loses the metadata of its postings, not only the ones it changes. Rebuild such a plugin to keep it.
