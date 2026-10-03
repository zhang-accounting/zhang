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

No queries yet.

### Budgets and commodities

The budget pages read [`#budgets`](/user-guide/query-language/#budgets) and [`#budget_events`](/user-guide/query-language/#budget-events). A month is given as its first day, such as `2024-06-01`; without one, the pages ask for the current month in the ledger's timezone.

#### `budgets.month`

Every budget as of a month: its last month in `#budgets` up to that month. The budgets page lists these rows. `#budgets` has a row for every month of a budget through the current month, so `last_month` is the requested month, unless the page asks about a later month. Nothing can have happened to the budget since `last_month`, so the page shows such a month starting with `available` and spending nothing. Budgets that start after the month are not listed.

| Parameter | Type | Value |
|-----------|------|-------|
| `month` | `date` | the first day of the month |

```sql
SELECT name, last(alias) AS alias, last(category) AS category, last(currency) AS currency,
       last(date) AS last_month, last(assigned) AS assigned, last(activity) AS activity,
       last(available) AS available, last(closed) AS closed
FROM #budgets
WHERE date <= :month
GROUP BY name
ORDER BY name
```

#### `budgets.budget`

One budget: its display name, category, commodity, and the accounts whose postings are its activity. No row if there is no such budget.

| Parameter | Type | Value |
|-----------|------|-------|
| `name` | `str` | the budget |

```sql
SELECT name, first(alias) AS alias, first(category) AS category, first(currency) AS currency,
       first(accounts) AS accounts
FROM #budgets
WHERE name = :name
GROUP BY name
```

#### `budgets.budget_month`

One budget as of a month, as in `budgets.month`. No row if the budget starts after the month; its page then shows nothing assigned or spent.

| Parameter | Type | Value |
|-----------|------|-------|
| `name` | `str` | the budget |
| `month` | `date` | the first day of the month |

```sql
SELECT name, last(alias) AS alias, last(category) AS category, last(currency) AS currency,
       last(date) AS last_month, last(assigned) AS assigned, last(activity) AS activity,
       last(available) AS available, last(closed) AS closed
FROM #budgets
WHERE name = :name AND date <= :month
GROUP BY name
```

#### `budgets.events`

What the `budget-add` and `budget-transfer` directives put into a budget in a month, newest first, as written: a transfer out is negative.

| Parameter | Type | Value |
|-----------|------|-------|
| `name` | `str` | the budget |
| `month` | `date` | the first day of the month |

```sql
SELECT date, time, timestamp, type, amount
FROM #budget_events
WHERE name = :name AND type != 'close' AND yearmonth(date) = :month
ORDER BY timestamp DESC
```

#### `budgets.postings`

The postings of a budget's accounts in a month, newest first, each with its account's balance in the posting's currency after it. The budget's page lists them together with the events of `budgets.events`, newest first.

| Parameter | Type | Value |
|-----------|------|-------|
| `accounts` | `set` | the budget's accounts, the `accounts` of `budgets.budget` |
| `month` | `date` | the first day of the month |

```sql
SELECT date, time, timestamp, account, id, payee, narration, units(position) AS units,
       only(currency, account_balance) AS balance
WHERE account IN :accounts AND yearmonth(date) = :month
ORDER BY timestamp DESC
```

What a commodity is (its precision, prefix, suffix, rounding and group) comes from its `commodity` directive. How much of it the ledger holds, in which lots, and its prices come from the queries below. Holdings are those of the Assets and Liabilities accounts, chosen with [`under`](/user-guide/query-language/#account-functions) so that the query reads only their postings.

#### `commodities.totals`

How many units of each commodity the Assets and Liabilities accounts hold, for the commodities they hold. A commodity without a row holds nothing.

```sql
SELECT currency, sum(number) AS total
WHERE under(account, 'Assets') OR under(account, 'Liabilities')
GROUP BY currency
HAVING sum(number) != 0
ORDER BY currency
```

#### `commodities.total`

How many units of one commodity the Assets and Liabilities accounts hold. No row if they hold none.

| Parameter | Type | Value |
|-----------|------|-------|
| `commodity` | `str` | the commodity |

```sql
SELECT currency, sum(number) AS total
WHERE currency = :commodity AND (under(account, 'Assets') OR under(account, 'Liabilities'))
GROUP BY currency
HAVING sum(number) != 0
```

#### `commodities.latest_prices`

The latest price of each commodity quoted in a currency, with its date and time.

| Parameter | Type | Value |
|-----------|------|-------|
| `currency` | `str` | the currency of the prices, the operating currency |

```sql
SELECT currency, last(date) AS date, last(time) AS time, last(amount) AS price
FROM #prices
WHERE currency(amount) = :currency
GROUP BY currency
ORDER BY currency
```

#### `commodities.latest_price`

The latest price of one commodity quoted in a currency, with its date and time.

| Parameter | Type | Value |
|-----------|------|-------|
| `commodity` | `str` | the commodity |
| `currency` | `str` | the currency of the price, the operating currency |

```sql
SELECT currency, last(date) AS date, last(time) AS time, last(amount) AS price
FROM #prices
WHERE currency = :commodity AND currency(amount) = :currency
GROUP BY currency
```

#### `commodities.lots`

The lots of a commodity that the Assets and Liabilities accounts hold: the units per account, cost and acquisition date, by account, then oldest first. Units held without a cost are one lot per account.

| Parameter | Type | Value |
|-----------|------|-------|
| `commodity` | `str` | the commodity |

```sql
SELECT account, cost_date, cost_number, cost_currency, sum(number) AS units
WHERE currency = :commodity AND (under(account, 'Assets') OR under(account, 'Liabilities'))
GROUP BY account, cost_date, cost_number, cost_currency
HAVING sum(number) != 0
ORDER BY account, cost_date, cost_number
```

#### `commodities.prices`

Every price of a commodity, in any currency, oldest first.

| Parameter | Type | Value |
|-----------|------|-------|
| `commodity` | `str` | the commodity |

```sql
SELECT date, time, amount
FROM #prices
WHERE currency = :commodity
ORDER BY date, time
```
