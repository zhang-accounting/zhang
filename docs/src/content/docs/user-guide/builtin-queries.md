---
title: Built-in Queries
description: The documented BQL queries behind the figures Zhang shows, how to open one on the Query page and adapt it, and the HTTP endpoints that list them and fill in their parameters.
---

The figures Zhang shows are moving onto *built-in queries* ([#479](https://github.com/zhang-accounting/zhang/issues/479)). A built-in query is a named query in Zhang's [query language](/user-guide/query-language/) that a page runs against your ledger, with a few parameters such as the dates of a report. The page only arranges the result, so the logic behind a figure is a query you can read on this page.

You can open the query behind a figure on the **Query** page, with the values the page used filled in, and change it: another date range, more accounts, another grouping, a chart. Anything the app shows, you can also query, and vary, without waiting for a new release.

:::note[Moving in progress]
The sections below list the built-in queries by page. A page whose section has no queries yet still computes its figures in code.
:::

## Opening a query

A figure that comes from a built-in query has an **Open query** action. It opens the Query page (`/explore`) with the query in the editor and runs it. The parameters are written into the query as values, so the query in the editor is complete. Edit it and run it again like any other query, or save it in your ledger with a [`query` directive](/directives/5-query/).

## Parameters

A built-in query names its inputs as parameters, written `:name`, such as `WHERE date >= :from AND date <= :to`. Zhang binds their values when it runs the query. What you type into a page, such as a search keyword, is only ever bound as a parameter and never pasted into the query text.

When a query is opened on the Query page, every parameter is replaced by its value, written so that the query reads it back exactly:

| Type | Written as | Examples |
|------|------------|----------|
| `date` | `YYYY-MM-DD` | `2024-01-31` |
| `str` | in single quotes, or in double quotes if it contains a single quote | `'Assets:Bank'`, `"O'Brien"` |
| `set` | a call of [`set`](/user-guide/query-language/#search-functions) | `set('food', 'trip')`, `set()` |
| `int` | the number, in parentheses if negative | `12`, `(-3)` |
| `decimal` | the number with its decimal places, always with a point, in parentheses if negative | `12.50`, `12.`, `(-0.5)` |
| `bool` | `TRUE` or `FALSE` | |
| any type | `NULL` for no value, which usually leaves a filter out | `NULL` |

Strings in the query language have no escape sequences: a string runs to the next quote of the same kind, and backslashes stay as they are, so `'C:\temp\'` is the text `C:\temp\`. A text that contains both kinds of quotes is written as a concatenation, such as `("it's " + '"quoted"')`.

The query with its values written in returns the same rows as the page got. Only the name of a column written without `AS` changes, since it is the column's text.

## HTTP API

Both endpoints sit behind the same [authentication](/installation/3-authentication/) as the rest of the API.

### List the built-in queries

`GET /api/query/builtins` lists every built-in query with its name, a description, its BQL and its parameters with their [types](/user-guide/query-language/#types):

```json
{
  "data": [
    {
      "name": "postings.between",
      "description": "Every posting between two ledger dates, both included, in ledger order.",
      "bql": "SELECT date, flag, payee, narration, account, position WHERE date >= :from AND date <= :to ORDER BY seq",
      "params": [
        { "name": "from", "type": "date" },
        { "name": "to", "type": "date" }
      ]
    }
  ]
}
```

### Write a query out with its values

`POST /api/query/builtins/{name}/text` returns the query `name` with its parameters replaced by the values in `params`, as the Query page receives it:

```shell
curl -X POST http://localhost:8000/api/query/builtins/postings.between/text \
  -H 'Content-Type: application/json' \
  -d '{"params": {"from": "2024-01-01", "to": "2024-01-31"}}'
```

```json
{
  "data": {
    "query": "SELECT date, flag, payee, narration, account, position WHERE date >= 2024-01-01 AND date <= 2024-01-31 ORDER BY seq"
  }
}
```

Send the text to [`POST /api/query`](/user-guide/query-language/#run-a-query) to run it. `params` must give every parameter of the query, and no other, as a JSON value of its type:

| Type | JSON value |
|------|------------|
| `date` | a string `YYYY-MM-DD` |
| `str` | a string |
| `set` | a list of strings |
| `int` | an integer |
| `decimal` | a number, or a string such as `"12.50"` to keep the decimal places |
| `bool` | `true` or `false` |
| any type | `null` |

An unknown query name is answered with HTTP 404. A missing, unknown or mistyped parameter is answered with HTTP 400 and a `message` that names it.

## The queries

### General

#### `postings.between`

Every posting between two ledger dates, both included, in ledger order.

| Parameter | Type | Value |
|-----------|------|-------|
| `from` | `date` | the first day |
| `to` | `date` | the last day |

```sql
SELECT date, flag, payee, narration, account, position
WHERE date >= :from AND date <= :to
ORDER BY seq
```

#### `postings.matching`

The postings of the transactions with a payee and any of some tags, in ledger order; a NULL parameter leaves its filter out.

| Parameter | Type | Value |
|-----------|------|-------|
| `payee` | `str` | the payee, or `NULL` for any |
| `tags` | `set` | the tags, any of which a transaction must have, or `NULL` for any |

```sql
SELECT date, payee, narration, tags, account, position
WHERE (:payee IS NULL OR payee = :payee) AND (:tags IS NULL OR intersects(tags, :tags))
ORDER BY seq
```

### Report

No queries yet.

### Accounts

No queries yet.

### Journals

The journal page (`GET /api/journals`), the payees and accounts the new-transaction form suggests (`GET /api/for-new-transaction`), the documents page (`GET /api/documents`) and the error list (`GET /api/errors`).

#### `journals.page`

One page of the journal, newest first: the transactions, padding transactions included, and the balance assertions that match a keyword, tags and links, where a NULL parameter leaves its filter out.

| Parameter | Type | Value |
|-----------|------|-------|
| `keyword` | `str` | the search text, or `NULL` for none: a transaction matches if its payee, narration, tags, links or accounts contain it, ignoring case, and a balance assertion if its accounts or the words `Balance Check` do. It is plain text, never a regular expression. |
| `tags` | `set` | the tags, any of which a transaction must have, or `NULL` for any; a balance assertion has none |
| `links` | `set` | the links, any of which a transaction must have, or `NULL` for any |
| `size` | `int` | the number of rows of a page, from 1 to 1000 |
| `offset` | `int` | the rows before the page: `(page - 1) × size` |

```sql
SELECT seq, type, id, date, time, flag, payee, narration, tags, links, metas
FROM #entries
WHERE (type = 'transaction'
       AND (:tags IS NULL OR intersects(tags, :tags))
       AND (:links IS NULL OR intersects(links, :links))
       AND (:keyword IS NULL OR icontains(payee, :keyword) OR icontains(narration, :keyword)
            OR any_icontains(tags, :keyword) OR any_icontains(links, :keyword) OR any_icontains(accounts, :keyword)))
   OR (type = 'balance' AND :tags IS NULL AND :links IS NULL
       AND (:keyword IS NULL OR icontains('Balance Check', :keyword) OR any_icontains(accounts, :keyword)))
ORDER BY seq DESC
LIMIT :size OFFSET :offset
```

- The page counts all its rows before `LIMIT` and `OFFSET` for its number of pages. `GET /api/journals` and `GET /api/errors` take a page `size` from 1 to 1000, 100 by default, and answer another size with HTTP 400 and the message `size must be between 1 and 1000`; a page past the last one is empty.
- Rows come newest first, in the [processing order](/user-guide/query-language/#processing-order): by date and the time written, then at one time the balance entries (balance assertions and every transaction flagged `P`) before the other transactions, in the order of your files, with a `balance ... with pad` after the other balance entries of its time, its padding among them, where Zhang checks it. A balance assertion therefore stands right above the postings its balance includes. On a day daylight saving skips a time, an entry written in the gap keeps its place but shows the time it is stored at, the first one after the gap.
- A transaction with the flag `P` is a padding transaction, which the page shows as a `BalancePad` item. A `balance` row is a `BalanceCheck` item, built from `journals.balance_checks`.

#### `journals.postings`

The postings of some transactions as written, in ledger order, with their units, whether those were inferred, the per-unit costs of their lots and the balance of their account in their currency before and after them.

| Parameter | Type | Value |
|-----------|------|-------|
| `ids` | `set` | the ids of the transactions, those of a page of `journals.page` |

```sql
SELECT id, posting_index, account, automatic, balanced,
       first(currency) AS currency,
       sum(number) AS number,
       count(*) AS lots, count(cost_number) AS lots_at_cost,
       min(cost_number) AS cost_number, max(cost_number) AS max_cost_number,
       min(cost_currency) AS cost_currency, max(cost_currency) AS max_cost_currency,
       number(last(only(currency, account_balance))) - sum(number) AS balance_before,
       number(last(only(currency, account_balance))) AS balance_after,
       first(metas) AS metas
WHERE id IN :ids
GROUP BY id, posting_index, account, automatic, balanced
```

- A posting that [lot booking](/user-guide/query-language/#lot-booking) splits into several rows is one row again, its units added up. Its cost is the per-unit cost of its lots, so a `{{1000 USD}}` cost of 10 units is `100 USD`. A reduction booked against lots of different costs has none: `cost_number` and `max_cost_number` (and the currencies) differ, or `lots_at_cost` is less than `lots`.
- A posting written without an amount (`automatic`) has no units in the journal, only the inferred ones.
- `balance_before` and `balance_after` are the balance of the posting's own account, in the posting's currency, around it: [`account_balance`](/user-guide/query-language/#the-account-balance) does not depend on `WHERE`.

#### `journals.balance_checks`

Some balance assertions with the asserted amount, the account's true balance, their difference and whether the assertion holds.

| Parameter | Type | Value |
|-----------|------|-------|
| `ids` | `set` | the ids of the assertions, those of a page of `journals.page` |

```sql
SELECT id, account, amount, tolerance, actual, passed,
       amount - actual AS difference,
       actual + (amount - actual) AS asserted
FROM #balances
WHERE id IN :ids
```

`asserted` is the asserted amount written with the decimal places of the balance too.

#### `journals.payees`

Every payee of the ledger's transactions, once and sorted, without those of the padding transactions.

```sql
SELECT DISTINCT payee
FROM #transactions
WHERE payee IS NOT NULL AND payee != '' AND flag != 'P'
ORDER BY payee
```

#### `journals.accounts`

The open accounts, sorted by name.

```sql
SELECT account
FROM #accounts
WHERE open IS NOT NULL AND close IS NULL
ORDER BY account
```

#### `journals.documents`

Every document of the ledger, newest first: the document directives and the documents that transactions and their postings name in their metadata.

```sql
SELECT date, time, path, account, transaction_id
FROM #documents
ORDER BY seq DESC
```

`path` is the path to download the document by, relative to the ledger's directory. A document named by a posting belongs to the posting's account.

#### `journals.errors`

One page of the ledger's errors, by file and then by position in the file.

| Parameter | Type | Value |
|-----------|------|-------|
| `size` | `int` | the number of errors of a page, from 1 to 1000 |
| `offset` | `int` | the errors before the page: `(page - 1) × size` |

```sql
SELECT id, kind, file, span_start, span_end, source, metas
FROM #errors
LIMIT :size OFFSET :offset
```

The error list shows where each error is as its file and the byte offsets of its directive in it.

### Budgets and commodities

No queries yet.
