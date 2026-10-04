---
title: Built-in Queries
description: The documented BQL queries behind the figures Zhang shows, how to open one on the Query page and adapt it, and the HTTP endpoints that list them and fill in their parameters.
---

The figures Zhang shows are moving onto *built-in queries* ([#479](https://github.com/zhang-accounting/zhang/issues/479)). A built-in query is a named query in Zhang's [query language](/reference/query-language/) that a page runs against your ledger, with a few parameters such as the dates of a report. The page only arranges the result, so the logic behind a figure is a query you can read on this page.

You can open the query behind a figure on the **Query** page, with the values the page used filled in, and change it: another date range, more accounts, another grouping, a chart. Anything the app shows, you can also query, and vary, without waiting for a new release.

:::note[Moving in progress]
The sections below list the built-in queries by page. A page whose section has no queries yet still computes its figures in code.
:::

## Opening a query

A figure that comes from a built-in query has an **Open query** action. It opens the Query page (`/explore`) with the query in the editor and runs it. The parameters are written into the query as values, so the query in the editor is complete. Edit it and run it again like any other query, or save it in your ledger with a [`query` directive](/reference/directives/query/).

## Parameters

A built-in query names its inputs as parameters, written `:name`, such as `WHERE date >= :from AND date <= :to`. Zhang binds their values when it runs the query. What you type into a page, such as a search keyword, is only ever bound as a parameter and never pasted into the query text.

When a query is opened on the Query page, every parameter is replaced by its value, written so that the query reads it back exactly:

| Type | Written as | Examples |
|------|------------|----------|
| `date` | `YYYY-MM-DD` | `2024-01-31` |
| `str` | in single quotes, or in double quotes if it contains a single quote | `'Assets:Bank'`, `"O'Brien"` |
| `set` | a call of [`set`](/reference/query-language/#search-functions) | `set('food', 'trip')`, `set()` |
| `int` | the number, in parentheses if negative | `12`, `(-3)` |
| `decimal` | the number with its decimal places, always with a point, in parentheses if negative | `12.50`, `12.`, `(-0.5)` |
| `bool` | `TRUE` or `FALSE` | |
| any type | `NULL` for no value, which usually leaves a filter out | `NULL` |

Strings in the query language have no escape sequences: a string runs to the next quote of the same kind, and backslashes stay as they are, so `'C:\temp\'` is the text `C:\temp\`. A text that contains both kinds of quotes is written as a concatenation, such as `("it's " + '"quoted"')`.

The query with its values written in returns the same rows as the page got. Only the name of a column written without `AS` changes, since it is the column's text.

## HTTP API

Both endpoints sit behind the same [authentication](/deployment/authentication/) as the rest of the API.

### List the built-in queries

`GET /api/query/builtins` lists every built-in query with its name, a description, its BQL and its parameters with their [types](/reference/query-language/#types):

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

Send the text to [`POST /api/query`](/reference/query-language/#run-a-query) to run it. `params` must give every parameter of the query, and no other, as a JSON value of its type:

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

An account page shows the account **and its sub-accounts**, as the account tree does: its journal, its balance history, its total and its documents cover the whole subtree. The page of `Assets:Bank` includes the postings of `Assets:Bank:Checking`, and its journal names the account of each posting. Balances are valued in the operating currency at today's prices with [`convert`](/reference/query-language/#valuation-functions), which uses inverse prices and the cost currency of a holding.

The balances with sub-accounts add up rows of these queries: the account list and the account page add the rows of `accounts.balances` or `accounts.subtree_balances` of an account and of every account under it, as the account tree of the web UI does. To get the total of a subtree yourself, filter with [`under`](/reference/query-language/#account-functions):

```sql
SELECT currency, sum(number) AS units
WHERE under(account, 'Assets:Bank')
GROUP BY currency
```

The journal of an account page merges the rows of `accounts.journal` and `accounts.balance_assertions` by their [`seq`](/reference/query-language/#processing-order), the order in which Zhang processed the ledger, newest first. An assertion therefore stands right after the postings its balance includes, wherever Zhang checked it: a balance written with a time after the transactions of its day before that time, a plain balance after a padding written before it, a `balance ... with pad` after the other balance entries of its time. Its balance is the running balance where it stands, and its `trx_id` is the id of its check, which `GET /api/journals` lists it with; a posting's is the id of its transaction. The rows of one transaction are newest first, by posting, and the rows of a posting booked against several lots are shown as one row. On a day daylight saving skips a time, the time written decides the order, so a row written in the gap can stand before one with an earlier stored time.

The page lists the journal by pages of 100 rows (`GET /api/accounts/{account}/journals?page=1&size=100`), each row of `accounts.journal` and each assertion counting as one; a page reads its rows with `accounts.journal_rows` and `accounts.journal_page`, from the end of the journal.

#### `accounts.list`

Every account with an `open` or `close` directive, with its open and close dates and its alias, by name. With `accounts.balances`, it makes the account list: an account with postings but no `open` directive is listed too.

```sql
SELECT account, open, close, meta('alias') AS alias
FROM #accounts
ORDER BY account
```

#### `accounts.balances`

The balance of every account that has postings, of its own postings, per currency: the units, their value in the operating currency at today's prices, and the date of the first posting. The account list adds up the rows of an account and of the accounts under it for the balance with sub-accounts.

| Parameter | Type | Value |
|-----------|------|-------|
| `operating_currency` | `str` | the `operating_currency` option of the ledger |

```sql
SELECT account, currency, sum(number) AS units,
       convert(sum(position), :operating_currency, today()) AS value,
       min(date) AS first_date
GROUP BY account, currency
ORDER BY account, currency
```

#### `accounts.subtree`

An account and its sub-accounts that have an `open` or `close` directive. The account page reads its dates, status and alias from it.

| Parameter | Type | Value |
|-----------|------|-------|
| `account` | `str` | the account of the page |

```sql
SELECT account, open, close, meta('alias') AS alias
FROM #accounts
WHERE under(account, :account)
ORDER BY account
```

#### `accounts.subtree_balances`

The balance of an account and of each of its sub-accounts, as in `accounts.balances`. The account page shows the total of its rows, the balance that a `balance` assertion on the account is checked against, and the balance of the account alone.

| Parameter | Type | Value |
|-----------|------|-------|
| `account` | `str` | the account of the page |
| `operating_currency` | `str` | the `operating_currency` option of the ledger |

```sql
SELECT account, currency, sum(number) AS units,
       convert(sum(position), :operating_currency, today()) AS value,
       min(date) AS first_date
WHERE under(account, :account)
GROUP BY account, currency
ORDER BY account, currency
```

#### `accounts.journal`

The postings of the account and its sub-accounts, in ledger order, each with the [running balance](/reference/query-language/#the-running-balance) of the account and its sub-accounts in the posting's currency right after it. The padding transactions of `balance ... with pad` are listed like the others. A posting booked against several [lots](/reference/query-language/#lot-booking) has a row per lot; they share its `seq` and `posting_index`. Add `ORDER BY seq DESC, posting_index DESC` to list the newest first, as the page does.

| Parameter | Type | Value |
|-----------|------|-------|
| `account` | `str` | the account of the page |

```sql
SELECT date, time, timestamp, flag, id, account, payee, narration, seq, posting_index,
       number AS units, currency, only(currency, units(balance)) AS balance
WHERE under(account, :account)
```

#### `accounts.journal_rows`

The number of rows of `accounts.journal`: with the assertions, the number of rows of all the pages of the journal.

| Parameter | Type | Value |
|-----------|------|-------|
| `account` | `str` | the account of the page |

```sql
SELECT count(*) AS rows
WHERE under(account, :account)
```

#### `accounts.journal_page`

Some rows of `accounts.journal`, in ledger order: a page of the journal reads its rows counting from the end. Only the rows of the page are built, whatever the offset.

| Parameter | Type | Value |
|-----------|------|-------|
| `account` | `str` | the account of the page |
| `limit` | `int` | how many rows |
| `offset` | `int` | how many rows before them |

```sql
SELECT date, time, timestamp, flag, id, account, payee, narration, seq, posting_index,
       number AS units, currency, only(currency, units(balance)) AS balance
WHERE under(account, :account)
LIMIT :limit OFFSET :offset
```

#### `accounts.balance_assertions`

The balance assertions on the account, newest first by [`seq`](/reference/query-language/#processing-order). `actual` is the balance of the account and its sub-accounts that the assertion was checked against, which is the running balance of the journal where its `seq` puts it. `pad` is the account a `balance ... with pad` pads from.

| Parameter | Type | Value |
|-----------|------|-------|
| `account` | `str` | the account of the page |

```sql
SELECT date, time, timestamp, id, account, amount, actual, passed, pad, seq
FROM #balances
WHERE account = :account
ORDER BY seq DESC
```

#### `accounts.balance_history`

The balance history chart of an account page: the balance of the account and its sub-accounts at the end of every day with a posting, per currency, in date order.

| Parameter | Type | Value |
|-----------|------|-------|
| `account` | `str` | the account of the page |

```sql
SELECT date, currency, last(only(currency, units(balance))) AS balance
WHERE under(account, :account)
GROUP BY date, currency
ORDER BY date, currency
```

#### `accounts.documents`

The documents of an account page: the `document` directives of the account and its sub-accounts, in ledger order. `path` is the path of the file relative to the ledger's directory, which the page downloads it with.

| Parameter | Type | Value |
|-----------|------|-------|
| `account` | `str` | the account of the page |

```sql
SELECT date, time, account, path
FROM #documents
WHERE source = 'directive' AND under(account, :account)
```

### Journals

No queries yet.

### Budgets and commodities

No queries yet.
