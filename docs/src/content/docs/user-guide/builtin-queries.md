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

The **Report** page and the dashboard (`GET /api/statistic/summary`, `/api/statistic/graph` and `/api/statistic/{account_type}`). Their range is two ledger dates, `from` and `to`, both included; the endpoints also accept an instant, which stands for its day in the ledger's timezone. `currency` is the ledger's operating currency.

- **Valuation.** The summary and the rankings are valued at the prices of `to`. Each point of the graph is valued at the prices of its own last day, or of `to` for the last one. A price is used in either direction, and a holding at cost without a price of its own is valued through its cost currency (see [`convert`](/user-guide/query-language/#valuation-functions)). Amounts that no price converts keep their currency and are left out of the totals in the operating currency.
- **The graph** has one point per day, per week (Monday to Sunday) or per month, named by its first day, so its first and last weeks or months may reach outside the range; the Report page labels the first one by the first day of the range. A point without postings in the range has the net worth of the point before, or that of `report.net_worth` on the day before `from`, valued at its own last day (see `report.net_worth_trend`).
- **Limits.** The figures obey the [limits](/user-guide/query-language/#limits) of every query. A graph can have at most half as many points as the result size limit (`ZHANG_QUERY_MAX_RESULT_VALUES`, so 500,000 by default), and its points, with a value per currency, count against that limit too. A longer range by day is answered with HTTP 400; ask for weeks or months instead.

#### `report.net_worth`

The net worth, the balance of the assets and the liabilities, at the end of `to`, valued in `currency` at the prices of that day: the summary's balance. The graph runs it for the day before `from`, as the balance it starts from; `balance` keeps the lots, to value it at other days.

| Parameter | Type | Value |
|-----------|------|-------|
| `to` | `date` | the day of the balance |
| `currency` | `str` | the currency of the values |

```sql
SELECT sum(position) AS balance, units(sum(position)) AS units, convert(sum(position), :currency, :to) AS value
WHERE (under(account, 'Assets') OR under(account, 'Liabilities')) AND date <= :to
```

#### `report.liabilities`

The balance of the liabilities at the end of `to`, valued in `currency` at the prices of that day. It is negative, as in the ledger.

| Parameter | Type | Value |
|-----------|------|-------|
| `to` | `date` | the day of the balance |
| `currency` | `str` | the currency of the values |

```sql
SELECT units(sum(position)) AS units, convert(sum(position), :currency, :to) AS value
WHERE under(account, 'Liabilities') AND date <= :to
```

#### `report.flows`

The income and the expenses of the range, valued in `currency` at the prices of `to`. Income is negative, as in the ledger.

| Parameter | Type | Value |
|-----------|------|-------|
| `from` | `date` | the first day |
| `to` | `date` | the last day |
| `currency` | `str` | the currency of the values |

```sql
SELECT root(account, 1) AS type, units(sum(position)) AS units, convert(sum(position), :currency, :to) AS value
WHERE (under(account, 'Income') OR under(account, 'Expenses')) AND date >= :from AND date <= :to
GROUP BY type
ORDER BY type
```

#### `report.transaction_count`

The number of transactions of the range. The padding transactions of `balance ... with pad` (flag `P`) are not counted, and balance assertions are not transactions.

| Parameter | Type | Value |
|-----------|------|-------|
| `from` | `date` | the first day |
| `to` | `date` | the last day |

```sql
SELECT count(*) AS transactions
FROM #transactions
WHERE flag != 'P' AND date >= :from AND date <= :to
```

#### `report.net_worth_trend`

The net worth, the balance of the assets and the liabilities, at the end of every day, week or month of the range that has postings, valued in `currency` at the prices of its last day in the range. `interval` is `'1 day'`, `'1 week'` or `'1 month'`: the bins of [`date_bin`](/user-guide/query-language/#date-functions) from 2001-01-01, a Monday and the first of a month, are calendar days, weeks starting on Monday and months, each named by its first day. [`least`](/user-guide/query-language/#comparison-functions) keeps the last bin's valuation date within the range. `balance` keeps the lots, to value the points without postings.

The query lists only the days, weeks or months with postings, so **Open query** shows fewer rows than the chart has points. The chart fills a day, week or month without postings with the last balance before it, from this query or from `report.net_worth` on the day before `from`, valued at its own last day in the range, as `report.net_worth` of that day values it. The query language cannot list days without postings yet.

| Parameter | Type | Value |
|-----------|------|-------|
| `from` | `date` | the first day |
| `to` | `date` | the last day |
| `interval` | `str` | `'1 day'`, `'1 week'` or `'1 month'` |
| `currency` | `str` | the currency of the values |

```sql
SELECT date_bin(:interval, date, 2001-01-01) AS bucket, last(balance) AS balance, units(last(balance)) AS units,
  convert(last(balance), :currency, least(max(date_bin(:interval, date, 2001-01-01)) + interval(:interval) - 1, :to)) AS value
WHERE (under(account, 'Assets') OR under(account, 'Liabilities')) AND date <= :to
GROUP BY bucket
HAVING max(date) >= :from
ORDER BY bucket
```

#### `report.changes`

What each account type changed by in every day, week or month of the range, valued in `currency` at the prices of its last day in the range: the bars of the income and expenses chart. The bins are those of `report.net_worth_trend`; the first one only counts the postings from `from` on.

| Parameter | Type | Value |
|-----------|------|-------|
| `from` | `date` | the first day |
| `to` | `date` | the last day |
| `interval` | `str` | `'1 day'`, `'1 week'` or `'1 month'` |
| `currency` | `str` | the currency of the values |

```sql
SELECT date_bin(:interval, date, 2001-01-01) AS bucket, root(account, 1) AS type, units(sum(position)) AS units,
  convert(sum(position), :currency, least(max(date_bin(:interval, date, 2001-01-01)) + interval(:interval) - 1, :to)) AS value
WHERE date >= :from AND date <= :to
GROUP BY bucket, type
ORDER BY bucket, type
```

#### `report.account_totals`

What every account of one type, such as `'Expenses'`, changed by in the range, valued in `currency` at the prices of `to`, smallest value first: the income and expense breakdowns.

| Parameter | Type | Value |
|-----------|------|-------|
| `type` | `str` | `'Assets'`, `'Liabilities'`, `'Equity'`, `'Income'` or `'Expenses'` |
| `from` | `date` | the first day |
| `to` | `date` | the last day |
| `currency` | `str` | the currency of the values |

```sql
SELECT account, units(sum(position)) AS units, convert(sum(position), :currency, :to) AS value
WHERE under(account, :type) AND date >= :from AND date <= :to
GROUP BY account
ORDER BY number(only(:currency, convert(sum(position), :currency, :to))), account
```

#### `report.top_postings`

The ten largest postings to accounts of one type in the range, by their value in `currency` at the prices of `to`: the top expenses and incomes. [`possign`](/user-guide/query-language/#amounts-and-numbers) turns income and liabilities positive, so the largest income comes first. Postings that no price converts to `currency` come last. `account_balance` is the balance of the posting's account right after it, in the posting's currency.

| Parameter | Type | Value |
|-----------|------|-------|
| `type` | `str` | `'Assets'`, `'Liabilities'`, `'Equity'`, `'Income'` or `'Expenses'` |
| `from` | `date` | the first day |
| `to` | `date` | the last day |
| `currency` | `str` | the currency of the values |

```sql
SELECT date, time, timestamp, account, id, payee, narration, units(position) AS units,
  only(currency, account_balance) AS account_balance, convert(position, :currency, :to) AS value
WHERE under(account, :type) AND date >= :from AND date <= :to
ORDER BY currency(convert(position, :currency, :to)) = :currency DESC, number(possign(convert(position, :currency, :to), account)) DESC
LIMIT 10
```

### Accounts

No queries yet.

### Journals

No queries yet.

### Budgets and commodities

No queries yet.
