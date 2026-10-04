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

The **Report** page and the dashboard (`GET /api/statistic/summary`, `/api/statistic/graph` and `/api/statistic/{account_type}`). Their range is two ledger dates, `from` and `to`, both included; the endpoints also accept an instant, which stands for its day in the ledger's timezone. `currency` is the ledger's operating currency.

- **Valuation.** The summary and the rankings are valued at the prices of `to`. Each point of the graph is valued at the prices of its own last day, or of `to` for the last one. A price is used in either direction, and a holding at cost without a price of its own is valued through its cost currency (see [`convert`](/reference/query-language/#valuation-functions)). Amounts that no price converts keep their currency and are left out of the totals in the operating currency.
- **The graph** has one point per day, per week (Monday to Sunday) or per month, named by its first day, so its first and last weeks or months may reach outside the range; the Report page labels the first one by the first day of the range. A point without postings in the range has the net worth of the point before, or the net worth on the day before `from`, valued at its own last day (see `report.net_worth_trend`).
- **Limits.** A graph has at most 50,000 points, about 137 years of days; a longer range by day is answered with HTTP 400, so ask for weeks or months. The figures also obey the [limits](/reference/query-language/#limits) of every query: the graph's points, with a value per currency, count against the result size limit (`ZHANG_QUERY_MAX_RESULT_VALUES`), and a graph that goes over it, or over the time limit, is answered with HTTP 400 too. What a graph costs grows with its range, not with the history of the ledger before it.

#### `report.net_worth`

The net worth, the balance of the assets and the liabilities, at the end of `to`, valued in `currency` at the prices of that day: the summary's balance. `balance` keeps the lots.

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

The net worth, the balance of the assets and the liabilities, at the end of every day, week or month of the range that has postings, valued in `currency` at the prices of its last day in the range. [`OPEN ON :from`](/reference/query-language/#accounting-periods) replaces everything before `from` with opening balances dated the day before, lot by lot, so the running `balance` starts from them and the query only groups the days of the range, plus the bucket of that day before. `interval` is `'1 day'`, `'1 week'` or `'1 month'`: the bins of [`date_bin`](/reference/query-language/#date-functions) from 2001-01-01, a Monday and the first of a month, are calendar days, weeks starting on Monday and months, each named by its first day. [`least`](/reference/query-language/#comparison-functions) keeps the last bin's valuation date within the range. `balance` keeps the lots, to value the points without postings.

The query lists only the days, weeks or months with postings, so **Open query** shows fewer rows than the chart has points. The chart fills a day, week or month without postings with the last balance before it, the opening balance included, valued at its own last day in the range, as `report.net_worth` of that day values it. The query language cannot list days without postings yet.

| Parameter | Type | Value |
|-----------|------|-------|
| `from` | `date` | the first day |
| `to` | `date` | the last day |
| `interval` | `str` | `'1 day'`, `'1 week'` or `'1 month'` |
| `currency` | `str` | the currency of the values |

```sql
SELECT date_bin(:interval, date, 2001-01-01) AS bucket, last(balance) AS balance, units(last(balance)) AS units,
  convert(last(balance), :currency, least(max(date_bin(:interval, date, 2001-01-01)) + interval(:interval) - 1, :to)) AS value
FROM OPEN ON :from
WHERE (under(account, 'Assets') OR under(account, 'Liabilities')) AND date <= :to
GROUP BY bucket
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

The ten largest postings to accounts of one type in the range, by their value in `currency` at the prices of `to`: the top expenses and incomes. [`possign`](/reference/query-language/#amounts-and-numbers) turns income and liabilities positive, so the largest income comes first. Postings that no price converts to `currency` come last. `account_balance` is the balance of the posting's account right after it, in the posting's currency.

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

The budget pages read [`#budgets`](/reference/query-language/#budgets), [`#budget_definitions`](/reference/query-language/#budget-definitions) and [`#budget_events`](/reference/query-language/#budget-events). A month is given as its first day, such as `2024-06-01`; without one, the pages ask for the current month in the ledger's timezone.

#### `budgets.month`

Every budget as of a month: its last month in `#budgets` up to that month, as the budgets page lists them. `#budgets` has a row for every month of a budget through the current month, so `last_month` is the requested month, unless it is a later one. Nothing can have happened to the budget since `last_month`, so the [`CASE`](/reference/query-language/#case) carries it over: the month starts with `available` and spends nothing. `activity` is a number, in the budget's `currency`. Budgets that start after the month are not listed. `WHERE date <= :month` also makes `#budgets` generate no later month, so a date typo far ahead in the ledger does not get in the way.

| Parameter | Type | Value |
|-----------|------|-------|
| `month` | `date` | the first day of the month |

```sql
SELECT name, last(alias) AS alias, last(category) AS category, last(currency) AS currency,
       last(date) AS last_month,
       CASE WHEN last(date) < :month THEN last(available) ELSE last(assigned) END AS assigned,
       CASE WHEN last(date) < :month THEN 0 ELSE number(last(activity)) END AS activity,
       last(available) AS available, last(closed) AS closed
FROM #budgets
WHERE date <= :month
GROUP BY name
ORDER BY name
```

#### `budgets.budget`

One budget: its display name, category, commodity, and the accounts whose postings are its activity, from `#budget_definitions`, which has no months. No row if there is no such budget.

| Parameter | Type | Value |
|-----------|------|-------|
| `name` | `str` | the budget |

```sql
SELECT name, alias, category, currency, accounts
FROM #budget_definitions
WHERE name = :name
```

#### `budgets.budget_month`

One budget as of a month, as in `budgets.month`. No row if the budget starts after the month; its page then shows nothing assigned or spent.

| Parameter | Type | Value |
|-----------|------|-------|
| `name` | `str` | the budget |
| `month` | `date` | the first day of the month |

```sql
SELECT name, last(alias) AS alias, last(category) AS category, last(currency) AS currency,
       last(date) AS last_month,
       CASE WHEN last(date) < :month THEN last(available) ELSE last(assigned) END AS assigned,
       CASE WHEN last(date) < :month THEN 0 ELSE number(last(activity)) END AS activity,
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

The postings of a budget in a month, newest first, each with its account's balance in the posting's currency after it: those of its accounts that count in it at their date, by [`account_budgets`](/reference/query-language/#account-and-commodity-directives), so a posting of an account closed and opened again with another budget is listed in the budget it counts in. The budget's page lists them together with the events of `budgets.events`, newest first.

| Parameter | Type | Value |
|-----------|------|-------|
| `accounts` | `set` | the budget's accounts, the `accounts` of `budgets.budget` |
| `month` | `date` | the first day of the month |
| `name` | `str` | the budget |

```sql
SELECT date, time, timestamp, account, id, payee, narration, units(position) AS units,
       only(currency, account_balance) AS balance
WHERE account IN :accounts AND yearmonth(date) = :month AND :name IN account_budgets(account, date)
ORDER BY timestamp DESC
```

What a commodity is (its precision, prefix, suffix, rounding and group) comes from its `commodity` directive. How much of it the ledger holds, in which lots, and its prices come from the queries below. Holdings are those of the Assets and Liabilities accounts, chosen with [`under`](/reference/query-language/#account-functions) so that the query reads only their postings.

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
