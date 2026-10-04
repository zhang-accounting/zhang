---
title: Note and Event
description: Reference for the note and event directives, which attach a dated comment to an account or record the value of a named event.
sidebar:
  order: 8
---

A `note` attaches a dated comment to an account, such as "called the bank about a fee". An `event` records that a
named variable of your life took a new value on a date, such as your location or employer. Neither changes a
balance. You read them back with queries.

## Syntax

```text
YYYY-MM-DD [HH:MM[:SS]] note <Account> "<Text>" [#tag …] [^link …]
YYYY-MM-DD [HH:MM[:SS]] event "<Type>" "<Value>"
```

| Directive | Part | Required | Description |
|---|---|---|---|
| `note` | `<Account>` | yes | The account the note is about. |
| | `"<Text>"` | yes | The comment. |
| | `#tag`, `^link` | no | Tags and links, as on a transaction. |
| `event` | `"<Type>"` | yes | The name of the variable, such as `location`. |
| | `"<Value>"` | yes | Its new value, such as a city. |

Both take a date, optionally with a time of day, and metadata lines below. Zhang also reads a string that is a single
word, without spaces, quotes, colons, parentheses or commas, when it is written without quotes. Beancount requires
the quotes.

## Examples

```zhang
2024-01-01 open Assets:Bank:Checking CNY

2024-03-02 note Assets:Bank:Checking "Called the bank about the card fee" #fees ^case-1024
  contact: "support desk"

2024-04-01 event "location" "Tokyo"
2024-09-15 event "location" "Shanghai"
2024-05-01 event "employer" "ACME Ltd"
```

## Behavior

- A `note` needs its account to be open at its date. A note may follow the account's `close`, as in Beancount.
- An `event` is not checked against anything.
- The web UI does not show notes or events on its pages; you see them in the files and in queries. Plugins receive
  them with the rest of the ledger.
- In queries, [`#notes`](/reference/query-language/#prices-balances-notes-events-documents-and-commodities) has one
  row per note and `#events` one row per event, with the columns `date, type, description`. Both are also rows of
  `#entries`.

```sql
SELECT date, description FROM #events WHERE type = 'location' ORDER BY date
```

## Errors

| Error | When |
|---|---|
| [`AccountDoesNotExist`](/reference/error-codes/#accountdoesnotexist) | The account of a `note` was never opened, or is opened only later. |

An `event` produces no error.

## Beancount compatibility

Both directives have the same syntax in Beancount. In a Beancount file, a time of day is written as
`time: "HH:MM:SS"` metadata.

## Related

- [Querying](/guides/querying/): reading notes and events back.
- [Custom](/reference/directives/custom/): dated values for plugins and tools.
