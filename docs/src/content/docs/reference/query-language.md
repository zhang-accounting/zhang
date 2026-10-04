---
title: Query Language
description: Reference for Zhang's BQL-compatible query language, covering syntax, the BALANCES and JOURNAL statements, accounting periods, the postings table, types, functions, charts, saved queries, CSV export, the HTTP API and the differences from Beancount's query language.
sidebar:
  order: 14
---

Zhang can answer ad-hoc questions about your ledger with a small query language. It understands a subset of the [Beancount Query Language (BQL)](https://beancount.github.io/docs/beancount_query_language/), so most queries written for Beancount or Fava work without changes. When BQL v2 and its successor [beanquery](https://github.com/beancount/beanquery) disagree, Zhang does what beanquery does, except for the few deliberate differences listed at the end of this page.

Queries run directly against the ledger that Zhang has already loaded into memory. They are read-only, and all arithmetic uses exact decimals, so amounts never pass through floating point.

:::caution[Early version]
This page covers the query language as of Phase 3 of [#434](https://github.com/zhang-accounting/zhang/issues/434): `SELECT`, `BALANCES` and `JOURNAL` over the `postings` table, the [other tables](#other-tables) and the [Zhang-specific tables](#zhang-specific-tables), the accounting-period clauses `OPEN ON`, `CLOSE` and `CLEAR`, saved queries and CSV export. [Differences from BQL and beanquery](#differences-from-bql-and-beanquery) lists what is not available yet.
:::

## Running a query

### In the web UI

Open the **Query** page (**查询** in the Chinese interface) at `/explore`.

- Type a query in the editor. It runs only when you click the run button or press <kbd>Ctrl</kbd>+<kbd>Enter</kbd> (<kbd>Cmd</kbd>+<kbd>Enter</kbd> on macOS), never while you are typing.
- Results are shown as a table, and each cell is rendered according to its [type](#types). An inventory cell shows one position per line.
- A result with two columns, a label and a number or amount, is also drawn as a chart above the table. See [Charts](#charts).
- If the query has an error, the editor highlights the line and column where it was found.
- The **Examples** menu puts a ready-made query in the editor and runs it. Among them are an income statement, the balances at the end of a year and the journal of an account.
- The **Saved** menu lists the queries saved in your ledger. See [Saved queries](#saved-queries).
- The **Reference** panel lists every column and function.
- **Export CSV** downloads the result as a CSV file. See [Exporting to CSV](#exporting-to-csv).
- **Open query**, next to a figure elsewhere in the app, opens this page with the [built-in query](/reference/builtin-queries/) behind the figure in the editor and runs it. A link to `/explore?query=...` does the same with any query.

#### Charts

When a result has exactly two columns and at least one row, and its second column holds numbers or amounts (`int`, `decimal`, `amount`, `position` or `inventory`), the Explore page draws it as a chart. The first column decides the kind of chart:

| First column | Chart |
|--------------|-------|
| `date` | A line chart, with one point per date in ascending order. |
| `str`, where every label looks like an account name | A treemap of the account hierarchy. |
| any other `str` | A bar chart, with one bar per label in result order. At most 50 bars are drawn. |

- A label looks like an account name when it is made of `:`-separated parts without spaces. At least one label must contain a `:`, and empty or `NULL` labels are ignored.
- Positions and inventories are plotted by their units, without their costs. A chart shows one currency at a time. When the result holds several currencies, a **Currency** picker chooses the one to plot. It starts with the ledger's operating currency (the `operating_currency` option) if the result has it, and otherwise with the currency that appears in the most rows.
- Rows with the same label are added together, and rows whose value is `NULL` are skipped. In a line chart, a date with no value in the chosen currency is plotted as zero.
- In a treemap, accounts are nested by their parts, and a top level shared by every account, such as `Expenses`, is skipped. The area of a cell is the absolute value, and its colour shows whether the value is positive or negative.
- Values are added exactly, and are only converted to floating point to be drawn.
- The **Chart** switch hides or shows the chart. The browser remembers the setting.

This query draws a treemap of a year's expenses:

```sql
SELECT account, sum(position) WHERE account ~ '^Expenses' AND year = 2024 GROUP BY account
```

And this one draws monthly spending as a line chart:

```sql
SELECT yearmonth(date) AS month, sum(position) WHERE account ~ '^Expenses' GROUP BY month ORDER BY month
```

#### Saved queries

Queries saved in the ledger with the [`query` directive](/reference/directives/query/) appear in the **Saved** menu:

```zhang
2024-01-01 query "food by payee" "SELECT payee, sum(position) WHERE account ~ '^Expenses:Food' GROUP BY payee"
```

- Each entry shows the query's name, its date and the start of its text. Queries are listed in ledger order, and queries that share a name are all listed, so use the date to tell them apart.
- Choosing an entry puts its query in the editor and runs it.
- A query that does not compile with the current version of the engine is still listed. It is marked **(invalid)** and shows the error. Saved queries are not checked when the ledger is loaded, so an invalid one never makes the ledger report an error.
- The list is fetched again each time the menu opens, so queries added to the ledger after the page was opened appear without a page reload.
- The query is a quoted string of the ledger file. A backslash that does not start a known escape is kept, so `'\d+'` saves the regular expression `\d+`; the doubled form `'\\d+'` works too and also reads the same in Beancount. See [Escaping](/reference/directives/query/#escaping).

#### Exporting to CSV

**Export CSV** sends the query in the editor to [`POST /api/query/csv`](#export-as-csv) and downloads the result as `query.csv`. The query does not need to be run first. Amounts, positions and inventories are split into one numeric column per currency, so the file opens cleanly in a spreadsheet. If the query has an error, the error is shown just as for a failed run. Read the [caution about formulas](#export-as-csv) before you open an export in a spreadsheet application.

### Over HTTP

Send the query to `POST /api/query`, or to `POST /api/query/csv` to get the result as CSV. `GET /api/query/saved` lists the saved queries, and `GET /api/query/builtins` the [built-in queries](/reference/builtin-queries/) behind the app's figures. See [HTTP API](#http-api) for the request and response formats.

## A first query

```sql
SELECT date, payee, account, position
WHERE account ~ '^Expenses:Food'
ORDER BY date DESC
LIMIT 10
```

This returns the ten most recent postings to `Expenses:Food` and its sub-accounts.

- Each row is one posting. `date` and `payee` come from the transaction, while `account` and `position` come from the posting itself.
- `~` matches a regular expression anywhere in the text and ignores case. The `^` anchors the pattern to the start of the account name. Note that `'^Expenses:Food'` also matches `Expenses:Foodstuff`. Use `'^Expenses:Food(:|$)'` to match only the account and its children.
- `ORDER BY date DESC` puts the newest postings first, and `LIMIT 10` keeps the first ten rows.

## Query syntax

```text
SELECT [DISTINCT] target [, target ...] | *
  [FROM from_clause]
  [WHERE expression]
  [GROUP BY group_key [, group_key ...] [HAVING expression]]
  [ORDER BY order_key [ASC | DESC] [, order_key [ASC | DESC] ...]]
  [PIVOT BY pivot_key, pivot_key]
  [LIMIT count [OFFSET count]]
  [;]

BALANCES [AT function] [FROM from_clause] [WHERE expression] [;]

JOURNAL ['pattern'] [AT function] [FROM from_clause] [;]

target      = expression [AS name]
group_key   = expression | target name | target number
order_key   = expression | target name | target number
pivot_key   = target name | target number
from_clause = #table | [expression] [OPEN ON date] [CLOSE [ON date]] [CLEAR]
count       = integer | parameter
```

- A query is a single statement: a `SELECT`, or one of the shorthands [`BALANCES` and `JOURNAL`](#balances-and-journal). `PRINT` is not supported.
- The clauses must appear in the order shown. All of them except the first keyword are optional, and a single trailing `;` is allowed.
- Keywords, column names and function names are all case-insensitive: `SELECT account`, `select ACCOUNT` and `Select Account` are the same query. [Table names](#other-tables) are case-sensitive, as in beanquery.
- A field of a structured column is read with a dot, without spaces: `open.date` in [`#accounts`](#accounts).
- Names consist of ASCII letters, digits and underscores, and cannot start with a digit. The words `SELECT`, `DISTINCT`, `FROM`, `WHERE`, `GROUP`, `BY`, `ORDER`, `ASC`, `DESC`, `LIMIT`, `AS`, `AND`, `OR`, `NOT`, `IN`, `IS`, `NULL`, `TRUE`, `FALSE`, `HAVING` and `PIVOT` are reserved and cannot be used as column names. `OFFSET` is a keyword only right after the count of `LIMIT`, so elsewhere `offset` is an ordinary name, as in beanquery.
- Spaces and line breaks between tokens are not significant, so a query can span several lines.
- `--` starts a comment that runs to the end of the line.

### How a query is evaluated

1. `FROM #table` chooses the [table](#other-tables) to read; without it the query reads the postings. The [period clauses](#accounting-periods) of `FROM` (`OPEN ON`, `CLOSE` and `CLEAR`), if there are any, rewrite the postings of the whole ledger.
2. The expression of `FROM` and the `WHERE` clause choose which rows take part.
3. If the query reads the [running balance](#the-running-balance), it is added up over the chosen postings, in ledger order.
4. If the query uses an [aggregate function](#aggregate-functions) or has a `GROUP BY` clause, the chosen postings are grouped and each group produces one row. Otherwise each posting produces one row.
5. `HAVING` drops the groups that do not satisfy its condition.
6. `ORDER BY` sorts the rows.
7. `DISTINCT` removes duplicate rows, keeping the first of each.
8. `OFFSET` skips the first rows, and `LIMIT` keeps the next ones and drops the rest.
9. `PIVOT BY` turns the remaining rows into a table with one column per value of a target.

Before any posting is read, Zhang checks the query and simplifies it. Parts that involve only constants, such as `'^Expenses:' + 'Food'`, are computed once at that point. Functions that depend on the ledger or on the current date (`today`, `convert`, `value`, `getprice`, the [directive functions](#account-and-commodity-directives) and the metadata functions) are not. As a result, an invalid regular expression in a constant pattern is reported immediately, with its position, even if no posting would ever be matched against it.

Zhang books the postings of a ledger once, for the first query that reads them, and keeps them, together with the ledger's prices, until the ledger is reloaded. A query without [period clauses](#accounting-periods) whose `FROM` or `WHERE` can only hold for some accounts reads only the postings of those accounts: `account = 'Assets:Bank'`, `account = :account`, `account IN ('Assets:Bank', 'Assets:Cash')`, `account IN :accounts` and [`under(account, 'Assets:Bank')`](#account-functions) (the account and its sub-accounts, also with a parameter), alone, combined with `OR`, or as one of the conditions joined by `AND`. This only makes such queries faster: the results, including the [running balance](#the-running-balance), are the same.

### SELECT

The targets are the expressions to compute for each row, separated by commas.

- `SELECT *` is short for `SELECT date, flag, payee, narration, account, position`. Over another table it expands to [that table's columns](#other-tables).
- `AS name` gives a target a name. The name can be used in `GROUP BY` and `ORDER BY`, but not in `WHERE`.
- Each result column is named after its alias if it has one. Otherwise it is named after the target's text exactly as you wrote it, without surrounding spaces: `account`, `ACCOUNT`, `sum(position)` or `sum( position )`. The columns of `SELECT *` use the lowercase names listed above.
- `SELECT DISTINCT` removes rows that are identical to an earlier row. Only the selected values are compared.

### FROM

`FROM` is followed by a table, by an expression, by [period clauses](#accounting-periods), or by an expression and period clauses. The expression filters postings exactly like `WHERE`, and when a query has both, a posting must satisfy both:

```sql
SELECT account, sum(position)
FROM year = 2024
WHERE account ~ '^Expenses'
GROUP BY account
```

is the same as `... WHERE (year = 2024) AND (account ~ '^Expenses') ...`. This matches beanquery, and it means BQL queries that filter in `FROM` keep working.

The period clauses come after the expression. They rewrite the postings first, and the expression then filters the rewritten postings:

```sql
SELECT account, sum(position)
FROM OPEN ON 2024-01-01 CLOSE ON 2025-01-01
WHERE account ~ '^(Income|Expenses)'
GROUP BY account
```

`FROM #name` reads one of the [other tables](#other-tables), and `FROM #postings` names the default table. A table stands alone in `FROM`: filter its rows with `WHERE`. The period clauses only apply to the postings, so they cannot follow a table, and `BALANCES` and `JOURNAL` always read the postings.

```sql
SELECT date, account, amount, discrepancy
FROM #balances
WHERE discrepancy IS NOT NULL
```

As in beanquery, a bare name that is not a column of the postings table names a table too: `FROM prices` is `FROM #prices`. An unknown table is an error that points at its name.

### WHERE

`WHERE` keeps the postings for which the condition is `TRUE`. A condition that evaluates to `NULL` counts as not true, so the posting is dropped (see [NULL and three-valued logic](#null-and-three-valued-logic)). The condition of `WHERE` or `FROM` must be a boolean expression, and aggregate functions are not allowed in either clause.

### GROUP BY

A query is an aggregate query when one of its targets uses an [aggregate function](#aggregate-functions), or when it has a `GROUP BY` clause. Postings are then grouped and each group produces one row.

- A group key can be an expression, a target name (an alias, or the text of a target such as `account`), or a target's position in the `SELECT` list, counting from 1. `GROUP BY 1, 2` groups by the first two targets. Names are matched without regard to case.
- A group key does not have to be selected. `SELECT sum(position) GROUP BY account` returns one unlabeled total per account.
- **Without `GROUP BY`**, an aggregate query is grouped by all of its targets that are not aggregates. `SELECT account, sum(position)` is the same as `SELECT account, sum(position) GROUP BY account`. If every target is an aggregate, all matching postings form a single group.
- **With `GROUP BY`**, every target that reads a column, or calls one of the [metadata functions](#metadata-functions), outside of an aggregate function must be a group key. `SELECT account, payee, count(*) GROUP BY account` is an error because `payee` is not grouped. Targets that use neither, such as constants, need not be grouped.
- In an aggregate query, an `ORDER BY` expression that is not an aggregate must also be a group key.
- Values of type `set` (such as `tags`) and `inventory` cannot be group keys.
- A group key cannot contain an aggregate function, and aggregate functions cannot be nested (`sum(count(*))` is an error).
- A single target cannot mix an aggregate with a column used outside of it, even a grouped one. `number - sum(number)` and `possign(sum(position), account)` are errors; write the second one as `sum(possign(position, account))`. Expressions built only from aggregates and constants, such as `sum(number) / 12` or `units(sum(position))`, are aggregates themselves and are fine.
- An aggregate query that matches no postings returns no rows at all. Even `SELECT count(*) WHERE FALSE` returns an empty result, not a row containing `0`.

```sql
SELECT root(account, 2) AS category, sum(position) AS total
WHERE account ~ '^Expenses'
GROUP BY category
```

### HAVING

`HAVING` filters the groups of an aggregate query, the way `WHERE` filters postings. It comes right after `GROUP BY` and keeps the groups for which its condition is `TRUE`. A condition that is `FALSE` or `NULL` drops the group.

```sql
SELECT root(account, 2) AS category, sum(position) AS total
WHERE account ~ '^Expenses'
GROUP BY category
HAVING sum(number) > 1000
```

- `HAVING` needs a `GROUP BY` clause. Without one it is a syntax error, even when the query is grouped [implicitly](#group-by).
- The condition must be a boolean expression that uses an [aggregate function](#aggregate-functions). The aggregates do not have to be selected: `HAVING count(*) > 10` works even if `count(*)` is not a target.
- Names in `HAVING` are columns of the `postings` table, as in `WHERE`, never target aliases. `HAVING total > 1000` is an error; repeat the expression, as in `HAVING sum(number) > 1000`.
- Outside an aggregate function, a column may only be read through a group key: an expression equal to a group key is that group's value. In `GROUP BY account HAVING account ~ 'Food' AND count(*) > 10`, `account` is the account of each group. Any other column must be inside an aggregate function.
- `LIMIT` counts the groups that `HAVING` keeps.

### ORDER BY

- An order key can be an expression, a target name or a target number, just like a group key. An expression that is not selected is still used for sorting, but it does not appear in the result.
- Each key has its own direction: `ASC` (ascending, the default) or `DESC` (descending). In `ORDER BY 1 DESC, 2`, the first key is descending and the second ascending.
- Rows are compared by the first key. The second key only breaks ties, and so on. Rows that are equal on every key keep their original order.
- `NULL` sorts lower than any other value. It comes first in ascending order and last in descending order.
- See [Ordering and comparison](#ordering-and-comparison) for how values of each type are ordered.
- Without `ORDER BY`, a plain query returns postings in ledger order: by date and time, then in the order they appear in your files. An aggregate query returns its groups in the order in which their first posting appears in the ledger.

### LIMIT and OFFSET

`LIMIT n` keeps the first `n` rows, after sorting and `DISTINCT`. `LIMIT n OFFSET m` skips the first `m` rows and then keeps the next `n`, which pages through a result: `ORDER BY date DESC LIMIT 50 OFFSET 100` is the third page of fifty.

- `n` and `m` are non-negative integers, written as literals or given as [parameters](#parameters), such as `LIMIT :size OFFSET :offset`.
- `OFFSET` is a Zhang extension and only follows a `LIMIT`; beanquery has `LIMIT` only. It is a keyword only there, so `SELECT date AS offset ORDER BY offset LIMIT 5` still works.
- `LIMIT 0` returns no rows, and an offset past the end returns no rows either.
- A parameter must be bound to an integer. A negative or `NULL` value, or an offset and limit whose sum does not fit in 64 bits, is an error at the parameter, never a silently different window.
- Without `ORDER BY`, the rows come in ledger order, so a page is stable as long as the ledger does not change.
- Pages are cheap: without `ORDER BY` the query only counts the rows before `OFFSET`, without building them, and stops once it has the rows it keeps, so a page holds its own rows only; with `ORDER BY` it keeps only the first `OFFSET + LIMIT` rows while it scans instead of sorting them all.
- So, as for the rows past `LIMIT`, the targets of the rows before `OFFSET` of a query without `ORDER BY` are not computed, and an error only computing one of them would raise, such as an integer overflow, is not reported. Those rows still go through `WHERE`, so an error of the condition is.

A query can also ask for the total number of rows before `LIMIT` and `OFFSET`, for example to show the number of pages: the `count_total` option of the Rust API, and of [`POST /api/query`](#run-a-query). Rows past the window are only counted, not built.

### PIVOT BY

`PIVOT BY a, b` turns the result of an aggregate query into a table: one row for each value of the target `a`, and one column for each value of the target `b`.

```sql
SELECT root(account, 2) AS category, year, sum(position) AS total
WHERE account ~ '^Expenses:(Food|Home)'
GROUP BY category, year
PIVOT BY category, year
```

| category/year | 2015 | 2016 | 2017 |
|---|---|---|---|
| Expenses:Food | 6614.67 USD | 6859.72 USD | 4686.42 USD |
| Expenses:Home | 31304.42 USD | 31280.55 USD | 20866.27 USD |

- `PIVOT BY` comes after `ORDER BY` and before `LIMIT`, and takes exactly two targets, each given by its name or by its number in the `SELECT` list. Expressions are not allowed.
- The query must be an aggregate query, and the second target must be a group key. The two targets must be different.
- It applies last, to the rows that `HAVING`, `ORDER BY`, `DISTINCT` and `LIMIT` leave. Only those rows create columns.
- The rows are sorted by `a`, whatever the `ORDER BY`. The columns are sorted by the value of `b`.
- The first column is named `a/b`, after the two targets. Each other column is named after a value of `b`, such as `2016`, and holds the remaining target for that value. When more than one target remains, there is one column per value and target, named `<value>/<target>`, for example `2016/total` and `2016/count`.
- A value is written like this in a column name: `2016-01-31` for a date, `12.50` for a decimal, `True` and `False` for booleans, and `NULL` for `NULL`.
- As in beanquery, column names are not always unique: a `NULL` value and the string `'NULL'` are both named `NULL`, and a value that contains `/` can make a `<value>/<target>` name equal to another one. Columns are identified by their position, so no data is lost, but a spreadsheet or a program that looks columns up by name sees duplicates.
- A cell for a pair of `a` and `b` that has no row is `NULL`. If several rows have the same pair, which happens when the query groups by more than `a` and `b`, the last row in the result order fills the cells.
- The columns keep the types of their targets, so [CSV export](#export-as-csv) splits pivoted amounts and inventories per currency, as in `2016 (USD)`.

### Parameters

When Zhang runs a query from its own code, through the Rust API of the `zhang-query` crate, the query can contain parameters, `$1`, `$2`, ... or `:name`, wherever it would contain a value, and the values are bound separately. The pattern of `JOURNAL`, the dates of `OPEN ON` and `CLOSE ON`, and the counts of `LIMIT` and `OFFSET` can be parameters too. The HTTP API does not bind parameters, so a query sent over HTTP that contains one fails with `parameter $1 is not bound`. A [built-in query](/reference/builtin-queries/) can be written out with its parameters filled in, as a query that runs over HTTP.

A parameter is a constant of one execution: before the rows are read, every parameter is replaced by its value and the query is simplified again, exactly as if the value had been written in the query. A regular expression given as a parameter (`payee ~ :keyword`) is compiled once, a set given as a parameter (`account IN :accounts`) and a list of values (`IN ('a', 'b', :c)`) are looked up in a hash table, and the needle of [`icontains`](#search-functions) is lower-cased once, so a query with parameters is as fast as the same query written with literals. An invalid regular expression given as a parameter is reported, at the match, when a row is matched against it, as before.

## BALANCES and JOURNAL

`BALANCES` and `JOURNAL` are shorthands for two common queries, as in beanquery. Zhang turns each of them into the `SELECT` shown below before it runs it, so everything this page says about `SELECT` applies to them too.

### BALANCES

```text
BALANCES [AT function] [FROM from_clause] [WHERE expression]
```

is the same as

```text
SELECT account, sum(function(position))
FROM from_clause
WHERE expression
GROUP BY account, account_sortkey(account)
ORDER BY account_sortkey(account)
```

- It returns one row per account that has postings, with the sum of its positions. An account whose postings add up to nothing is still listed, with an empty inventory.
- Accounts are sorted by type, in the order `Assets`, `Liabilities`, `Equity`, `Income` and `Expenses`, and then by name. See [`account_sortkey`](#account-functions).
- `FROM` and `WHERE` choose the postings that are added up. `BALANCES FROM year = 2024` gives how much each account changed during 2024, not its balance at the end of the year. For that, use [`CLOSE ON`](#accounting-periods).
- `BALANCES` has no `GROUP BY`, `ORDER BY` or `LIMIT` clause.

```sql
BALANCES WHERE account ~ '^Assets'
```

### JOURNAL

```text
JOURNAL ['pattern'] [AT function] [FROM from_clause]
```

is the same as

```text
SELECT date, flag, maxwidth(payee, 48), maxwidth(narration, 80), account,
       function(position), function(balance)
FROM from_clause
WHERE account ~ 'pattern'
```

- It returns one row per posting to an account that matches the pattern, in ledger order, with the [running balance](#the-running-balance) in the last column. Without a pattern, every posting is listed.
- The pattern is a regular expression in quotes, matched with [`~`](#regular-expression-match), so it ignores case and matches any part of the account name. `JOURNAL 'checking'` lists every account whose name contains `Checking`.
- The running balance adds up every row of the journal, whatever its account. If the pattern matches several accounts, the balance is their combined balance.
- Payees are shortened to 48 characters and narrations to 80, with [`maxwidth`](#string-functions).
- `JOURNAL` has no `WHERE`, `GROUP BY`, `ORDER BY` or `LIMIT` clause. Filter its postings with `FROM`.

```sql
JOURNAL 'Assets:Bank:Checking' FROM year = 2024
```

### AT

`AT function` applies a function to every position, and in `JOURNAL` to the running balance as well:

| Statement | Result columns |
|-----------|----------------|
| `BALANCES` | `account`, `sum(position)` |
| `BALANCES AT cost` | `account`, `sum(cost(position))` |
| `JOURNAL 'Cash'` | `date`, `flag`, `maxwidth(payee, 48)`, `maxwidth(narration, 80)`, `account`, `position`, `balance` |
| `JOURNAL 'Cash' AT units` | `date`, `flag`, `maxwidth(payee, 48)`, `maxwidth(narration, 80)`, `account`, `units(position)`, `units(balance)` |

- The function is named without parentheses. It must accept a single `position`, and for `JOURNAL` also a single `inventory`. The [valuation functions](#valuation-functions) `units`, `cost` and `value` are the usual choices, and `abs` and `neg` work too.
- `AT units` drops costs, so the lots of a currency merge into one position. `AT cost` gives book values, and `AT value` market values at the latest prices.
- An unknown function, or one without a suitable overload, is an error that points at its name.
- As the table shows, the result columns are named like the equivalent `SELECT`.

## Accounting periods

`OPEN ON`, `CLOSE` and `CLEAR` in the `FROM` clause turn the ledger into the books of one accounting period, the way Beancount's reports do. With them, an income statement and a balance sheet are each a single query.

An income statement for 2024:

```sql
SELECT account, sum(position) AS total
FROM OPEN ON 2024-01-01 CLOSE ON 2025-01-01
WHERE account ~ '^(Income|Expenses)'
GROUP BY account
ORDER BY account
```

A balance sheet at the start of 2025:

```sql
BALANCES FROM CLOSE ON 2025-01-01 CLEAR
WHERE account ~ '^(Assets|Liabilities|Equity)'
```

### Period syntax

```text
FROM [expression] [OPEN ON date] [CLOSE [ON date]] [CLEAR]
```

- The clauses must come in this order, and each one at most once. `FROM CLEAR OPEN ON 2024-01-01` is an error.
- `FROM` needs an expression, at least one clause, or both.
- A date is a date literal such as `2024-01-01`, without quotes, or a [parameter](#parameters).
- When both dates are given, the `CLOSE` date cannot be before the `OPEN` date. The two dates can be equal.
- The clauses work with `SELECT`, `BALANCES` and `JOURNAL`.

### What the clauses do

The clauses rewrite every posting of the ledger, first `OPEN`, then `CLOSE`, then `CLEAR`. Only after that do the `FROM` expression and `WHERE` choose postings, so a filter never changes what the clauses compute. For example, `FROM year = 2023 OPEN ON 2024-01-01` keeps only the opening balances, which are dated 2023-12-31.

**`OPEN ON d`** starts the period on `d` and replaces everything before `d` with opening balances:

1. The conversions before `d` are moved to `Equity:Conversions:Previous`. The conversions are the amount by which all postings, valued at cost, fail to add up to zero. Transactions that exchange one currency for another at a price (`@`) leave such a remainder, and so does rounding in lot purchases.
2. The balances of the income and expense accounts before `d` are moved to `Equity:Earnings:Previous`, so those accounts start the period at zero.
3. Each account that still has a balance gets one summarization transaction, with the flag `S` and dated `d − 1`. It has one posting per lot of the balance, so holdings keep their cost, cost date and label, and each posting has a counterpart on `Equity:Opening-Balances` for the lot's cost.

Postings on or after `d` are kept as they are.

**`CLOSE ON d`** ends the period before `d`. Postings dated `d` or later are dropped, so `d` itself is not part of the period. If the remaining postings have conversions, a conversion transaction with the flag `C`, dated `d − 1`, books them to `Equity:Conversions:Current`. Its postings have a price of zero in the conversion currency (`NOTHING` by default).

**`CLOSE`** without a date drops nothing. It only adds that conversion transaction, dated like the last entry of the period.

**`CLEAR`** moves the balance of each income and expense account to `Equity:Earnings:Current`, with one transfer transaction per account, with the flag `T` and dated like the last entry of the period. Afterwards the income and expense accounts add up to zero, so the balance sheet balances.

The last entry of the period is the latest of its postings and of the ledger's other dated directives, such as prices and balance assertions, from the `OPEN` date up to the day before the `CLOSE` date. Budget directives do not count.

### Synthetic transactions

The transactions added by the clauses are rows of the postings table like any other:

| Flag | Added by | Date | Narration | Accounts |
|------|----------|------|-----------|----------|
| `S` | `OPEN ON d` | `d − 1` | `Opening balance for '<account>' (Summarization)` | the account, and `Equity:Opening-Balances` |
| `C` | `CLOSE` | `d − 1` for `CLOSE ON d`, otherwise the date of the last entry of the period | `Conversion for (<inventory>)` | `Equity:Conversions:Current` |
| `T` | `CLEAR` | the date of the last entry of the period | `Transfer balance for '<account>' (Transfer balance)` | the account, and `Equity:Earnings:Current` |

- Their `payee` is `NULL`, and they have no tags, links or metadata. All the postings of one transaction share an `id`.
- A counterpart posting has the narration of the transaction it belongs to, so it names the account it balances.
- The previous earnings and conversions of `OPEN` appear as `S` transactions of the accounts `Equity:Earnings:Previous` and `Equity:Conversions:Previous`.
- They count in the [running balance](#the-running-balance). After `OPEN ON`, the first row of each account is its opening balance.
- Filter them by `flag`. For example, `WHERE flag != 'S'` hides the opening balances.

```sql
SELECT flag, count(*) FROM OPEN ON 2024-01-01 CLOSE ON 2025-01-01 CLEAR WHERE flag IN ('S', 'C', 'T') GROUP BY flag
```

### Equity accounts

The accounts that the clauses post to do not need to be opened in your ledger. The Beancount options in the last column change them:

| Account | Used by | Option |
|---------|---------|--------|
| `Equity:Opening-Balances` | `OPEN ON` | `account_previous_balances` |
| `Equity:Earnings:Previous` | `OPEN ON` | `account_previous_earnings` |
| `Equity:Conversions:Previous` | `OPEN ON` | `account_previous_conversions` |
| `Equity:Earnings:Current` | `CLEAR` | `account_current_earnings` |
| `Equity:Conversions:Current` | `CLOSE` | `account_current_conversions` |

- An option gives the part of the name after `Equity:`. With `option "account_previous_balances" "Opening"`, `OPEN ON` uses `Equity:Opening`.
- A value that is not a valid account name, such as one that is empty or contains a space, is ignored, and the default is used.
- `option "conversion_currency" "..."` sets the currency of the zero price of the conversion postings. It is `NOTHING` by default.

## Literals

| Literal | Examples | Type |
|---------|----------|------|
| String | `'Food'`, `"USD"` | `str` |
| Integer | `0`, `42` | `int` |
| Decimal | `3.14`, `0.5`, `.5` | `decimal` |
| Date | `2024-01-31` | `date` |
| Boolean | `TRUE`, `FALSE` | `bool` |
| Null | `NULL` | `null` |

- Strings can use single or double quotes. Unlike standard SQL, `"USD"` is a string, not a column name.
- A string ends at the next quote of the same kind, and there are no escape sequences. To put a quote character inside a string, wrap the string in the other kind of quote: `"Joe's Diner"` or `'say "hi"'`.
- Dates are written **without** quotes, in `YYYY-MM-DD` form. An invalid date such as `2024-13-01` is an error.
- A quoted string compared with a date is read as a date, so `date >= '2024-01-01'` also works. A string that is not a valid `YYYY-MM-DD` date is then an error.
- Integers are 64-bit. A larger integer literal becomes a `decimal`. Exponent notation such as `1e3` is not supported.
- `TRUE`, `FALSE` and `NULL` are keywords and are case-insensitive.

## Operators

From highest to lowest precedence:

| Precedence | Operators | Meaning |
|------------|-----------|---------|
| 1 | `-x` `+x` | unary minus and plus |
| 2 | `*` `/` | multiplication, division |
| 3 | `+` `-` | addition, subtraction |
| 4 | `=` `==` `!=` `<>` `<` `<=` `>` `>=` | comparison |
| 4 | `~` `!~` `?~` | regular-expression match |
| 4 | `IN` `NOT IN` | membership |
| 4 | `IS NULL` `IS NOT NULL` | null test |
| 5 | `NOT` | logical negation |
| 6 | `AND` | logical and |
| 7 | `OR` | logical or |

- Use parentheses to group explicitly: `(a OR b) AND c`.
- Because `NOT` binds more loosely than comparisons, `NOT account ~ '^Assets'` means `NOT (account ~ '^Assets')`.
- The operators of level 4 cannot be chained: `a = b = c` is an error.

### Arithmetic

| Expression | Result | Notes |
|------------|--------|-------|
| `int` `+ - *` `int` | `int` | An overflow is an error. |
| `int` `/` `int` | `decimal` | `7 / 2` is `3.5`. |
| `int` or `decimal` `+ - * /` `int` or `decimal` | `decimal` | |
| `date` `+` `int`, `int` `+` `date`, `date` `-` `int` | `date` | Adds or subtracts days: `2024-01-31 + 1` is `2024-02-01`. A result outside the years 1 to 9999 is `NULL`. |
| `date` `-` `date` | `int` | Number of days between the dates. |
| `date` `+ -` `interval`, `interval` `+` `date` | `date` | Moves by the months of the interval first, keeping the day of the month unless that month is shorter (then its last day), then by its days: `2024-01-31 + interval('1 month')` is `2024-02-29`. A result outside the years 1 to 9999 is `NULL`. |
| `interval` `+ -` `interval` | `interval` | `interval('1 year') + interval('-1 month')` is `11 months`. |
| `str` `+` `str` | `str` | Concatenation. |
| `amount` `*` number, number `*` `amount` | `amount` | `units(position) * 2`. |
| `amount` `/` number | `amount` | |
| `amount` `+ -` `amount` | `amount` | Both amounts must have the same currency, otherwise the query fails. |

- Addition, subtraction and multiplication are exact. A product keeps all the decimal places of its operands: `1000.00 * 1` is `1000.00`.
- Division is exact when the result fits in 28 significant digits. Otherwise it is rounded to 28 significant digits, half to even, like Beancount: `1 / 3` is `0.3333333333333333333333333333`.
- Dividing by zero produces `NULL` instead of an error.
- Unary minus works on `int`, `decimal`, `amount`, `position` and `inventory`.
- `%` (remainder) is not supported.

### Comparison

- `=` (or `==`) and `!=` (or `<>`) compare two values of the same type. `int` and `decimal` can be compared with each other, numerically: `1 = 1.00` is `TRUE`.
- `<`, `<=`, `>` and `>=` work only on `bool`, `int`, `decimal`, `str` and `date`. To compare amounts, compare their numbers with [`number`](#amounts-and-numbers).
- String comparison is case-sensitive and compares character codes, so `'B' < 'a'`. Only `~` and `!~` ignore case.

### Regular-expression match

| Operator | Meaning |
|----------|---------|
| `text ~ pattern` | `TRUE` when `pattern` matches **any part** of `text`, ignoring case. |
| `text !~ pattern` | The opposite of `~`. |
| `pattern ?~ text` | Case-sensitive match. Note that the pattern comes **first**, as in beanquery. |

| Expression | Result |
|------------|--------|
| `'Expenses:Food:Dining' ~ 'food'` | `TRUE`, a partial match that ignores case |
| `'Expenses:Food:Dining' ~ '^Food'` | `FALSE`, because `^` anchors to the start |
| `'Expenses:Food:Dining' ~ 'Dining$'` | `TRUE` |
| `'Expenses:Food:Dining' !~ '^Income'` | `TRUE` |
| `'Food' ?~ 'Expenses:Food:Dining'` | `TRUE` |
| `'food' ?~ 'Expenses:Food:Dining'` | `FALSE`, because `?~` is case-sensitive |

- Anchor the pattern with `^` and `$` when you need a full match.
- Patterns use the syntax of the Rust [`regex`](https://docs.rs/regex/latest/regex/#syntax) crate. It is close to Python's, but has no look-around (`(?=...)`, `(?!...)`) and no back-references.
- An invalid pattern is an error that points at the pattern. So is a pattern that would compile to more than 1 MiB, such as `'a{1000}{1000}'`.
- Both sides must be strings. If either side is `NULL`, the result is `NULL`, for all three operators.

### Membership

`IN` tests whether a value belongs to a set or a list:

```sql
WHERE 'trip-new-york' IN tags
WHERE 'Assets:Cash' NOT IN other_accounts
WHERE account IN ('Assets:Cash', 'Assets:Bank:Checking')
WHERE year IN (2023, 2024)
WHERE payee IN ('Amazon')
```

- The right-hand side is either a `set` value, such as the `tags`, `links` or `other_accounts` columns, or a list of expressions in parentheses. A list may have a single element.
- With a set, the left-hand side must be a string. `'x' IN (tags)`, with the set in parentheses, also tests membership in the set.
- With a list, each element must be comparable with the left-hand side, as for `=`.
- `NOT IN` is the opposite of `IN`.
- If the left-hand side is `NULL`, the result is `NULL`. If the value is not found in a list that contains a `NULL`, the result is `NULL` too, as in standard SQL.

### NULL and three-valued logic

A value is `NULL` when it is missing, for example the payee of a transaction that has none, or the cost of a posting that is not held at cost.

- Arithmetic, comparisons, `~`, `!~`, `?~`, `IN`, `NOT IN` and function calls with a `NULL` operand produce `NULL`. The exceptions are `IS NULL`, `IS NOT NULL`, `AND`, `OR` and the aggregate functions.
- `AND`, `OR` and `NOT` follow standard SQL three-valued logic:

| Expression | Result |
|------------|--------|
| `TRUE AND NULL` | `NULL` |
| `FALSE AND NULL`, `NULL AND FALSE` | `FALSE` |
| `TRUE OR NULL`, `NULL OR TRUE` | `TRUE` |
| `FALSE OR NULL` | `NULL` |
| `NOT NULL` | `NULL` |

- `WHERE` and `FROM` treat `NULL` as not true, so the posting is dropped.
- Test for missing values with `IS NULL` and `IS NOT NULL`. `payee = NULL` is always `NULL`, never `TRUE`.
- Both `payee != 'Shop'` and `NOT (payee = 'Shop')` drop postings whose payee is `NULL`. Write `payee IS NULL OR payee != 'Shop'` to keep them.

## The postings table

The default table is `postings`. It has one row for every posting of every transaction, with the transaction's fields repeated on each of its postings. The [other tables](#other-tables) hold the directives, and the [Zhang-specific tables](#zhang-specific-tables) hold your budgets and the ledger's errors.

- **Included:** all transactions whatever their flag, and the padding transactions that Zhang creates for `balance ... with pad ...` directives. These have the flag `P`, the payee `Balance Pad` and a narration such as `pad Assets:Bank to Equity:Opening`. A query with [period clauses](#accounting-periods) also sees the [synthetic transactions](#synthetic-transactions) they add.
- **Not included:** balance assertions, and directives that are not transactions, such as `open`, `close`, `price`, `note`, `document` and budget directives.
- A posting written without an amount has the amount that Zhang inferred for it when it balanced the transaction.
- Postings held at cost are booked against lots, as described in [Lot booking](#lot-booking). A posting that reduces several lots produces one row per lot.
- Rows come in ledger order: by date and time, then in the order the transactions appear in your files.

### Lot booking

The `position`, `cost_*` and `weight` columns of a posting held at cost depend on the lot it is booked against. The query engine books lots per account and currency, in ledger order, the way Beancount does:

- **Reductions.** A posting at cost whose sign is opposite to an open lot (selling what you bought, for example) reduces lots. The fields written in its cost, that is the cost number and currency, the date and the label, must match the lot. Fields that are left out match any lot. `-4 AAPL {100 USD}` reduces lots bought at 100 USD on any date, `-4 AAPL {100 USD, 2024-01-02}` reduces only the one bought on 2024-01-02, `-4 AAPL {100 USD, "a"}` only the one labeled `a`, and `-4 AAPL {}` reduces any lot.
- **Choosing among matching lots.** Matching lots are used oldest first (FIFO), or newest first (LIFO) if the account's `booking_method` is `LIFO`. A reduction that spans several lots is split into one row per lot, each with the units taken from that lot and the lot's cost.
- **Augmentations.** Any other posting at cost opens a lot, or adds to an identical one. A cost without a date, such as `10 AAPL {100 USD}`, is dated by its transaction, so `cost_date` is never `NULL` for a posting held at cost.
- **Leftovers.** If no open lot covers all of a reduction, the remainder is booked as an augmentation. A cost without a number, such as `{}`, cannot open a lot, so that remainder has no cost.

:::note
Two booking cases are still handled differently by Zhang's ledger processing than by the query engine. Zhang currently matches a reduction that gives a cost but no date only against lots dated on the day of the reduction, and records an error otherwise ([#436](https://github.com/zhang-accounting/zhang/issues/436)). The `STRICT`, `AVERAGE`, `AVERAGE_ONLY` and `NONE` booking methods are not implemented yet ([#437](https://github.com/zhang-accounting/zhang/issues/437)), and the query engine books accounts that use them as FIFO for now.
:::

### Columns

| Column | Type | Description |
|--------|------|-------------|
| `date` | `date` | Date of the transaction. The time of day, if any, is dropped; it is in `time`. |
| `year` | `int` | Year of `date`. |
| `month` | `int` | Month of `date`, from 1 to 12. |
| `day` | `int` | Day of the month of `date`, from 1 to 31. |
| `flag` | `str` | Flag of the transaction: `*` (also used when no flag is written), `!`, `P` for padding, `S`, `C` or `T` for the [synthetic transactions](#synthetic-transactions) of the period clauses, or a custom flag. |
| `payee` | `str` | Payee of the transaction, or `NULL` if it has none. When the header has a single string, that string is the narration and the payee is `NULL`. |
| `narration` | `str` | Narration of the transaction, or `''` (an empty string) if it has none, as in Beancount. |
| `description` | `str` | Payee and narration joined with `" \| "`. Missing or empty parts are left out, so it is `''` when both are missing. |
| `tags` | `set` | Tags of the transaction, without the leading `#`. |
| `links` | `set` | Links of the transaction, without the leading `^`. |
| `id` | `str` | Zhang's identifier of the transaction, a UUID. All postings of a transaction share it. |
| `account` | `str` | Account of the posting. |
| `number` | `decimal` | Number of units of the posting. |
| `currency` | `str` | Currency (commodity) of the units. |
| `position` | `position` | Units of the posting together with its cost lot, if any. |
| `cost_number` | `decimal` | Cost per unit, or `NULL` if the posting is not held at cost. A total cost written with `{{...}}` is divided by the number of units. |
| `cost_currency` | `str` | Currency of the cost, or `NULL`. |
| `cost_date` | `date` | Date of the cost lot, or `NULL` if the posting is not held at cost. A lot without an explicit date is dated by the transaction that opened it. |
| `cost_label` | `str` | Label of the cost lot. It is `''` (an empty string) if the posting is not held at cost, and `NULL` if the lot has no label. |
| `price` | `amount` | Price per unit written with `@`, or `NULL` if there is none. A total price written with `@@` is divided by the number of units. |
| `weight` | `amount` | Amount that the posting contributes to balancing its transaction: units times the per-unit cost if the posting is held at cost, otherwise units times the price if it has one, otherwise the units. |
| `other_accounts` | `set` | Accounts of the other postings in the same transaction. |
| `meta` | `str` | Metadata of the posting as text: `key: "value"` pairs sorted by key and separated by `, `, or `''` if it has none. The transaction's own metadata is read with `entry_meta()`. |
| `metas` | `metas` | Metadata of the posting as a list of `(key, value)` pairs, sorted by key, with every value of a repeated key in the order written. See [Structured metadata](#structured-metadata). Zhang extension. |
| `entry_metas` | `metas` | Metadata of the posting's transaction, in the same form. Zhang extension. |
| `balance` | `inventory` | The [running balance](#the-running-balance): the sum of the positions of the rows up to and including this one. It cannot be used in `FROM` or `WHERE`. |
| `time` | `str` | Time of day of the transaction in the ledger's timezone, as `HH:MM:SS`: the time written, or midnight without one; on a day daylight saving skips that time, the first time after the gap, as Zhang stores it (`02:30` in New York on 2024-03-10 is `03:30:00`, midnight in São Paulo on 2018-11-04 is `01:00:00`). Zhang extension. |
| `timestamp` | `int` | Unix time of the transaction's date and time, in seconds. Zhang extension. |
| `seq` | `int` | Position of the transaction in the [processing order](#processing-order), counting from 0, as in [`#entries`](#entries). All postings of a transaction share it, so `ORDER BY seq DESC` lists the newest transactions first, in a stable order. `NULL` for the [synthetic transactions](#synthetic-transactions) of the period clauses. Zhang extension. |
| `posting_index` | `int` | Position of the posting in its transaction as written, counting from 0. When [booking](#lot-booking) splits a posting into one row per lot, the rows share it. Zhang extension. |
| `account_balance` | `inventory` | The [account balance](#the-account-balance): the balance of the posting's account right after this posting. Zhang extension. |
| `balanced` | `bool` | `FALSE` if Zhang found that the transaction does not balance (an `UnbalancedTransaction` error), otherwise `TRUE`. Zhang extension. |
| `errors` | `set` | The kinds of the errors Zhang recorded for the transaction, named as in the `kind` column of [`#errors`](#errors), such as `UnbalancedTransaction` or `AccountDoesNotExist`. Empty if there are none. Zhang extension. |

### The running balance

The `balance` column is the running total of `position`, as an inventory.

- It adds up the rows that pass `FROM` and `WHERE`, in ledger order, starting from an empty inventory. Postings that the filters drop are not counted. With `WHERE account = 'Assets:Bank:Checking' AND year = 2024`, the balance starts from zero at the first posting of 2024. To start from the account's real balance, limit the dates with [`OPEN ON` and `CLOSE ON`](#accounting-periods) instead: `FROM OPEN ON 2024-01-01 CLOSE ON 2025-01-01 WHERE account = 'Assets:Bank:Checking'`.
- It is a single total over all the rows, not one per account. To follow one account, select only that account's postings.
- It is computed before grouping, `ORDER BY`, `DISTINCT` and `LIMIT`, so sorting the rows does not change their balances. With `ORDER BY date DESC`, the first row carries the final balance.
- It keeps lots, so a holding bought at different costs shows up as several positions. `units(balance)` merges them, and `cost(balance)` gives the book value.
- In an aggregate query it can be used inside an aggregate function. `last(balance)` is the balance after the last posting of each group.
- It cannot be used in `FROM` or `WHERE`, because those clauses decide which rows it adds up. Doing so is an error.

```sql
SELECT date, payee, position, balance
WHERE account = 'Assets:Bank:Checking'
ORDER BY date DESC
LIMIT 10
```

This returns the ten most recent postings to the account, each with the balance after it. The first row shows the current balance.

### The account balance

The `account_balance` column is the balance of the posting's own account right after the posting. Like `balance`, it is an inventory that keeps lots. Unlike `balance`, it does not depend on the query:

- It adds up every posting of the account, in ledger order, whatever `FROM`, `WHERE` and `LIMIT` choose. A query that shows only some postings of an account still shows the account's real balance after each of them.
- It is one balance per account, so a query over several accounts shows each posting with the balance of its own account.
- With the [period clauses](#accounting-periods), it adds up the postings they produce, starting from the opening balance that `OPEN ON` adds.
- Unlike `balance`, it can be used in `WHERE`.

```sql
SELECT date, payee, position, account_balance
WHERE account = 'Assets:Bank:Checking' AND year = 2024
ORDER BY seq DESC
```

This lists the account's postings of 2024, newest first, each with the account's balance after it, including everything posted before 2024.

## Other tables

Besides `postings`, a query can read one of the tables below with `FROM #name`. They are beanquery's tables, with the same column names, types and row order:

| Table | One row per | `SELECT *` |
|-------|-------------|------------|
| `#entries` | directive of any kind | `id, type, filename, date, year, month, day, flag, payee, narration, description, tags, links, meta, accounts` |
| `#transactions` | transaction | `date, flag, payee, narration, tags, links, accounts` |
| `#prices` | `price` directive | `date, currency, amount` |
| `#balances` | balance assertion | `date, account, amount, tolerance, discrepancy` |
| `#notes` | `note` directive | `date, account, comment, tags, links` |
| `#events` | `event` directive | `date, type, description` |
| `#documents` | `document` directive, then `document` metadata of a transaction or posting | `date, account, filename, tags, links` |
| `#accounts` | account with an `open` or `close` directive | `account, open, close` |
| `#commodities` | `commodity` directive | `meta, date, name` |

Zhang adds three tables of its own, `#budgets`, `#budget_events` and `#errors`, described in [Zhang-specific tables](#zhang-specific-tables).

```sql
SELECT currency, last(amount) AS latest
FROM #prices
WHERE date >= 2024-01-01
GROUP BY currency
ORDER BY currency
```

- **Columns are per table.** A table has only the columns listed for it, and none of the `postings` columns. `year`, `month` and `day` exist only on `#entries` and `postings`; elsewhere use [`year(date)`](#date-functions) and the other date functions. All functions, aggregates, `GROUP BY`, `HAVING`, `ORDER BY`, `PIVOT BY`, `DISTINCT` and `LIMIT` work on every table.
- **Row order.** Without `ORDER BY`, rows come in ledger order: by date, then the order in which beancount sorts the directives of one day (`open` first, then balance assertions, the other directives, `document` and `close` last), then the order of your files.
- **Metadata.** Every directive table has a `meta` column, the directive's metadata as text: `key: "value"` pairs sorted by key and separated by `, `, or `''` without metadata. `#entries` and `#transactions` also have `metas`, the same metadata as [structured pairs](#structured-metadata). `meta(key)`, `entry_meta(key)` and `any_meta(key)` read one key of the row's directive (in `#accounts`, of its `open` directive), and `meta_values(key)` and `entry_meta_values(key)` every value of it.
- **Balance assertions are not transactions.** An assertion books nothing; it is a `balance` entry in `#entries` and a row of `#balances`. Transactions that Zhang rejected while loading the ledger are not rows either. The padding transactions of `balance ... with pad` (flag `P`) are transactions, as in beancount.
- **Zhang extensions.** Some tables have columns that beanquery does not have, marked *Zhang extension* below: `seq`, `time`, `timestamp` and `metas` on `#entries`; `id`, `seq`, `time`, `timestamp`, `balanced`, `errors` and `metas` on `#transactions`; `actual`, `passed`, `pad`, `id`, `seq`, `time` and `timestamp` on `#balances`; and `source`, `path`, `transaction_id`, `seq`, `time` and `timestamp` on `#documents`. They come after beanquery's columns and are not part of `SELECT *`, so `SELECT *` gives the same columns as in beanquery. The [postings table](#columns) has extensions of its own, and `#budgets`, `#budget_events` and `#errors` are Zhang's own tables.

### Processing order

The `seq` column of `#entries`, `#transactions`, `#balances`, `#documents` and the [postings](#columns) is the position of an entry in the order Zhang processes the ledger, counting from 0:

1. by date and the time written; a directive written without one is at midnight;
2. within one time, `open` and `commodity` directives first, then the balance entries (balance assertions, and every transaction flagged `P`: the padding transactions Zhang makes, and any written by hand), then every other directive;
3. then in the order of your files;
4. except that a `balance ... with pad` is checked after the other balance entries of its time, its padding among them, and its `seq` is where it is checked.

This is the order in which balances change: the [running balance](#the-running-balance) of the postings adds them up in this order, and an assertion comes right after the postings its `actual` balance includes, so merging the rows of `#balances` and of the postings by `seq` lists every assertion in its place. The rows of the postings table and of `#transactions` come in this order. Without `ORDER BY`, the rows of `#entries` and of the other directive tables keep beancount's order, by date and then by kind: `open` first (before a `commodity` of the same day), then the balance assertions, the other directives, and `document` and `close` last, whatever their times. The two orders differ only within a day: Zhang keeps same-day `commodity` and `open` directives in the order of your files, sorts the directives of a day by their time (a timed `open` after the transactions written without a time), puts a balance after the transactions before its time and after a padding written before it, and leaves a `document` or a `close` where it is. `ORDER BY seq` lists the rows in Zhang's order.

The time that decides the order is the time written. On a day daylight saving skips a time, a directive written in the gap is stored at the first time after it, and `ORDER BY seq` can list the `time` and `timestamp` columns out of order there: in New York on 2024-03-10, an entry written at `02:30`, stored at `03:30:00`, comes before one written at `03:15`.

```sql
SELECT seq, date, time, type FROM #entries WHERE date = 2024-01-05 ORDER BY seq
```

### #entries

| Column | Type | Description |
|--------|------|-------------|
| `id` | `str` | Unique id of the directive. For a transaction it is the transaction's id, the same as the `id` column of its postings; for a balance assertion, the id Zhang stored its check with. |
| `type` | `str` | Kind of directive, lowercase: `transaction`, `open`, `close`, `balance`, `price`, `note`, `document`, `event`, `commodity`, `custom` or `query`, and Zhang's `budget`, `budget-add`, `budget-transfer` and `budget-close`. A `balance ... with pad` is a `balance`. |
| `filename` | `str` | The ledger file that holds the directive. |
| `date`, `year`, `month`, `day` | `date`, `int` | Date of the directive and its parts. |
| `flag`, `payee`, `narration`, `description` | `str` | As in `postings`, for a transaction. `NULL` for other directives. |
| `tags`, `links` | `set` | Tags and links of a transaction, note or document. `NULL` for other directives. |
| `meta` | `str` | Metadata of the directive. |
| `accounts` | `set` | The accounts the directive refers to: the posting accounts of a transaction, the account of an `open`, `close`, `balance`, `note` or `document`, and the pad account of `balance ... with pad`. Empty for other directives. |
| `seq` | `int` | Position of the directive in the [processing order](#processing-order), counting from 0. `ORDER BY seq DESC` lists the newest entries first. Without `ORDER BY` the rows keep beancount's order, which can differ within a day. Zhang extension. |
| `time`, `timestamp` | `str`, `int` | Time of day of the directive in the ledger's timezone (`HH:MM:SS`), as in `postings`: the time written, or midnight without one; on a day daylight saving skips that time, the first time after the gap, as Zhang stores it (`02:30` in New York on 2024-03-10 is `03:30:00`, midnight in São Paulo on 2018-11-04 is `01:00:00`); and the Unix time of its date and time, in seconds. Zhang extension. |
| `metas` | `metas` | Metadata of the directive as `(key, value)` pairs, see [Structured metadata](#structured-metadata). Zhang extension, not part of `SELECT *`. |

### #transactions

| Column | Type | Description |
|--------|------|-------------|
| `date` | `date` | Date of the transaction. |
| `flag` | `str` | `*`, `!`, or `P` for padding. |
| `payee` | `str` | Payee, or `NULL`. |
| `narration` | `str` | Narration, `''` if there is none. |
| `tags`, `links` | `set` | Tags and links. |
| `accounts` | `set` | Accounts of the postings. |
| `meta` | `str` | Metadata of the transaction. |
| `id` | `str` | Zhang's identifier of the transaction: the `id` of its postings and of its row in `#entries`. Zhang extension. |
| `seq`, `time`, `timestamp` | `int`, `str`, `int` | As in `postings`: the position of the transaction in the [processing order](#processing-order), its time of day and its Unix time. Zhang extension. |
| `balanced`, `errors` | `bool`, `set` | As in `postings`: whether the transaction balances, and the kinds of the errors recorded for it. Zhang extension. |
| `metas` | `metas` | Metadata of the transaction as `(key, value)` pairs, see [Structured metadata](#structured-metadata). Zhang extension, not part of `SELECT *`. |

### #prices, #balances, #notes, #events, #documents and #commodities

| Table | Column | Type | Description |
|-------|--------|------|-------------|
| `#prices` | `date` | `date` | Date of the price. |
| | `currency` | `str` | The commodity being priced. |
| | `amount` | `amount` | The price of one unit. |
| `#balances` | `date` | `date` | Date of the assertion. |
| | `account` | `str` | The account whose balance is asserted. |
| | `amount` | `amount` | The asserted balance. |
| | `tolerance` | `decimal` | The explicit tolerance (`~ 0.01`), or `NULL`. |
| | `discrepancy` | `amount` | If the assertion fails, `actual` minus the asserted amount; `NULL` if it holds. A `balance ... with pad` holds unless a pad of the same time changes its balance after it, or it pads from the asserted account itself or one of its sub-accounts, which leaves its balance as it was. |
| | `actual` | `amount` | The true balance of the account in the asserted currency at the assertion: the units of every posting to the account and its sub-accounts before it, as Zhang checks a balance. An assertion never changes it. A `balance ... with pad` is checked once the pads of its time are booked. Zhang extension. |
| | `passed` | `bool` | Whether the assertion holds, as Zhang's balance check decides it: `actual` is within the tolerance of the asserted amount, or equal to it when the assertion has no tolerance. An assertion that fails is also an `AccountBalanceCheckError` in [`#errors`](#errors). Zhang extension. |
| | `pad` | `str` | For a `balance ... with pad`, the account it pads from; `NULL` for a balance without a pad. Zhang extension. |
| | `id` | `str` | The id Zhang stored the check of the assertion with, which `GET /api/journals` lists it with: the `id` of its row in [`#entries`](#entries). Zhang extension. |
| | `seq` | `int` | Position of the assertion in the [processing order](#processing-order), where Zhang checks it, as in [`#entries`](#entries): right after the postings its `actual` balance includes. Zhang extension. |
| | `time`, `timestamp` | `str`, `int` | Time of day of the assertion in the ledger's timezone (`HH:MM:SS`), as in [`#entries`](#entries), and its Unix time, in seconds. Zhang extension. |
| `#notes` | `date`, `account` | `date`, `str` | Date and account of the note. |
| | `comment` | `str` | The text of the note. |
| | `tags`, `links` | `set` | Tags and links. |
| `#events` | `date` | `date` | Date of the event. |
| | `type` | `str` | The kind of event, such as `location`. |
| | `description` | `str` | Its value, such as a city. |
| `#documents` | `date`, `account` | `date`, `str` | Date and account of the document. For a document named in metadata, the date of the transaction, and the account of the posting, or `NULL` for a document of the transaction itself. |
| | `filename` | `str` | Path of the file. A relative path is resolved against the directory of the ledger file that declares it, as in beancount. |
| | `tags`, `links` | `set` | Tags and links of the `document` directive, or of the transaction that names the document. |
| | `source` | `str` | What declares the document: `'directive'` for a `document` directive, `'transaction'` or `'posting'` for the `document` metadata of a transaction or of one of its postings. Zhang extension. |
| | `path` | `str` | Path of the file as written, relative to the ledger's directory: Zhang resolves document paths against the ledger's directory, and the web UI downloads the file with this path. An absolute path inside the directory is made relative to it. Zhang extension. |
| | `transaction_id` | `str` | For a document named in metadata, the `id` of its transaction, as in the postings table. `NULL` for a `document` directive. Zhang extension. |
| | `seq` | `int` | The [`seq`](#processing-order) of the `document` directive, or of the transaction that names the document. Zhang extension. |
| | `time`, `timestamp` | `str`, `int` | Time of day (`HH:MM:SS`, as in [`#entries`](#entries)) and Unix time, in seconds, of the `document` directive, or of the transaction that names the document. Zhang extension. |
| `#commodities` | `date` | `date` | Date of the `commodity` directive. |
| | `name` | `str` | The commodity, such as `USD`. |

Each of these tables also has a `meta` column. For a document named in metadata, `meta` and `meta(key)` read the metadata of the transaction or posting that names it.

*Zhang extension.* Besides its `document` directives, `#documents` lists every document a transaction names in its metadata, as the documents page of the web UI does: after the directives, one row per value of a `document` metadata key of a transaction, then of each of its postings, in ledger order. A repeated key gives one row per value. The documents of rejected transactions are not listed.

```sql
SELECT date, account, path, transaction_id
FROM #documents
WHERE source != 'directive'
ORDER BY date DESC
```

### #accounts

`open` and `close` are the account's `open` and `close` directives, as structured values. Read their fields with a dot:

| Column | Type | Description |
|--------|------|-------------|
| `account` | `str` | Name of the account. |
| `open`, `open.date` | `date` | Date of the `open` directive, or `NULL` if there is none. On its own, `open` reads as this date. |
| `open.account` | `str` | The account of the `open` directive. |
| `open.currencies` | `set` | The currencies the account is restricted to, or `NULL` if it accepts any. |
| `open.booking` | `str` | The booking method, from the `booking_method` metadata, or `NULL`. |
| `open.meta` | `str` | Metadata of the `open` directive. |
| `close`, `close.date` | `date` | Date of the `close` directive, or `NULL` while the account is open. On its own, `close` reads as this date. |
| `close.account`, `close.meta` | `str` | The account and the metadata of the `close` directive. |

```sql
SELECT account, open.date, open.currencies
FROM #accounts
WHERE close IS NULL
ORDER BY account
```

An attribute of a missing directive is `NULL`. An unknown attribute, such as `open.datum`, is an error that points at the attribute.

## Zhang-specific tables

Zhang has three tables of its own, for data that Beancount does not have: `#budgets`, the monthly figures of your [budgets](/reference/directives/budget/), `#budget_events`, what your budget directives did, and `#errors`, the problems Zhang found in your ledger. They are read like the [other tables](#other-tables), with `FROM #budgets`, `FROM #budget_events` and `FROM #errors`, and the same rules apply: a table has only its own columns, so `account` is not a column of `#budgets`, and every clause and function works on it. Unlike the directive tables, they have no `meta` column, and their rows come in the order given below for each table. `meta(key)`, `entry_meta(key)` and `any_meta(key)` all read the row's own metadata.

### Budgets

`#budgets` has one row per budget per month, with the figures the budget page of the web UI shows for that month.

- A budget has a row for every month from the month of its `budget` directive through the later of two months: the month of the budget's last `budget-add`, `budget-transfer` or `budget-close`, and the last month with a transaction in the ledger. A budget planned ahead with a `budget-add` in a future month therefore shows that month. Other directives, such as prices, events, notes and balance assertions, do not extend the months. Months without budget entries or spending have a row too, with the available amount carried over, as on the budget page. The rows only depend on the ledger, not on today's date: a later month is the budget's last row carried over, with nothing spent.
- The months are generated rather than read from the ledger, so each one counts toward the [result size limit](#limits), even if `WHERE` drops it. If a transaction or a budget directive is dated far in the future by mistake, the query fails with a "too large" error instead of using up memory. The error names the budget with the longest series and the directive whose date ends it, such as `budget 'food' runs from 2024-01 until 2204-05 because of a transaction dated 2204-05-01 (main.zhang); check that date`. Fix the date to query the table again.
- `assigned`, `activity` and `available` are the Assigned, Activity and Available columns of the budget page. `assigned` is what the month starts with, the amount still available at the end of the previous month, plus what the month's `budget-add` and `budget-transfer` directives put in (`added`). `activity` is what the budget's accounts spent in the month, and `available`, which is `assigned - activity`, carries over to the next month.
- All amounts are in the budget's commodity. `activity` adds up the postings of the budget's accounts, each converted to the budget's commodity at the posting's date, as [`convert(position, currency, date)`](#valuation-functions) does with the prices of the ledger: `activity` is what `sum(convert(position, 'CNY', date))` gives over those postings. A `budget-add` or `budget-transfer` amount in another commodity is converted the same way at the directive's date. A posting or amount that no price converts is left out, instead of being added as a number of another commodity.
- A budget exists from its `budget` directive on. A `budget-add`, `budget-transfer` or `budget-close` of a budget that does not exist yet has no effect, and postings before the budget's `budget` directive are not its spending; Zhang reports both as errors. A second `budget` directive of the same name is a duplicate and is ignored.
- Because `assigned` includes the carry-over, adding it up over several months counts the same money more than once. Add up `added` instead to see how much was budgeted over a period.
- A budget's accounts are the accounts whose `open` directive has a `budget` metadata entry naming it, such as `budget: food`. Their postings are the budget's activity.
- `meta(key)` reads the metadata of the `budget` directive.
- Rows are ordered by budget name, then by month. `SELECT *` is short for `SELECT name, date, assigned, activity, available`.

| Column | Type | Description |
|--------|------|-------------|
| `name` | `str` | Name of the budget, as written in its directives. |
| `alias` | `str` | Display name of the budget, from its `alias` metadata, or `NULL` if it has none. |
| `category` | `str` | Category the budget page groups the budget under, from its `category` metadata, or `NULL` if it has none. |
| `currency` | `str` | Commodity the budget is kept in. |
| `date` | `date` | First day of the month. |
| `year` | `int` | Year of the month. |
| `month` | `int` | Month of the year, from 1 to 12. |
| `assigned` | `amount` | Amount assigned to the budget for the month: the available amount carried over from the previous month plus `added`. |
| `added` | `amount` | Amount the month's `budget-add` and `budget-transfer` directives put into the budget, converted to its commodity at their date. A transfer out of the budget counts as negative. |
| `activity` | `amount` | Amount the budget's accounts spent in the month, each posting converted to the budget's commodity at its date. A refund counts as negative. |
| `available` | `amount` | Amount left at the end of the month, `assigned - activity`. It carries over to the next month, and is negative when the budget is overspent. |
| `accounts` | `set` | Accounts whose postings count as the budget's activity. |
| `closed` | `bool` | Whether the budget was closed with `budget-close` in or before the month. It is `FALSE` in the months before. |

What is left in each budget, grouped as on the budget page:

```sql
SELECT category, name, available
FROM #budgets
WHERE date = 2024-06-01
ORDER BY category, name
```

How much was budgeted and spent in 2024, per budget:

```sql
SELECT name, sum(added) AS budgeted, sum(activity) AS spent
FROM #budgets
WHERE year = 2024
GROUP BY name
ORDER BY spent DESC
```

The months in which an open budget was overspent:

```sql
SELECT date, name, available
FROM #budgets
WHERE number(available) < 0 AND NOT closed
```

Each budget as it is now, in its last month:

```sql
SELECT name, last(available) AS available, last(closed) AS closed
FROM #budgets
GROUP BY name
ORDER BY name
```

### Budget events

`#budget_events` has one row per effect of a budget directive, in ledger order: what the budget page lists as the events of a month, plus the closes.

- A `budget-add` is an `assign` of its amount. A `budget-transfer` is two rows: a `transfer_out` of the budget it takes from, with the amount negated, then a `transfer_in` of the budget it gives to. A `budget-close` is a `close`, without an amount.
- Amounts are as written, in the directive's commodity; [`#budgets`](#budgets) converts them to the budget's commodity. A positive amount adds to the budget.
- A directive without effect, because its budget does not exist yet, has no row.
- `meta(key)` reads the metadata of the directive. `SELECT *` gives every column.

| Column | Type | Description |
|--------|------|-------------|
| `name` | `str` | Name of the budget. |
| `date` | `date` | Date of the directive. |
| `time` | `str` | Time of day of the directive in the ledger's timezone, as `HH:MM:SS`; `00:00:00` for a directive without a time. |
| `timestamp` | `int` | Unix time in seconds of the directive's date and time in the ledger's timezone. |
| `type` | `str` | `'assign'`, `'transfer_out'`, `'transfer_in'` or `'close'`. |
| `amount` | `amount` | Amount put into the budget: negative for a transfer out, `NULL` for a close. |

What was put into each budget in 2024, as written:

```sql
SELECT name, sum(amount) AS added
FROM #budget_events
WHERE year(date) = 2024 AND type != 'close'
GROUP BY name
```

### Errors

`#errors` has one row per ledger error: the problems the errors page of the web UI lists and `GET /api/errors` returns.

- `kind` is the error code, such as `UnbalancedTransaction`. [Error Codes](/reference/error-codes/) explains each code and how to fix it. `message` is the sentence the errors page shows for it.
- `file` is the file of the directive that caused the error, relative to the ledger's directory, as in the web UI's file list. `source` is the text of that directive. `line` and `column` are `NULL` for now, because Zhang does not record line numbers yet.
- `date` is the date of the directive, or `NULL` for an undated one such as an `option`. `account` is the account the error is about, for errors that name one, such as `AccountDoesNotExist`, `AccountClosed` and `AccountBalanceCheckError`.
- `meta(key)` reads the other details Zhang records about an error. For an error in a transaction, `meta('txn_id')` is the transaction's `id` in the postings table. Undefined budgets referenced by a posting have `meta('budget_name')`.
- `id` is the id of the error in `GET /api/errors`, and `span_start` and `span_end` are where the directive that caused it starts and ends in its file, as byte offsets. The id is derived from the directive's position, so the errors of one directive share it, and an error in a transaction has the transaction's `id`.
- Rows are ordered by file, then by position in the file. `SELECT *` is short for `SELECT file, date, kind, account, message`.

| Column | Type | Description |
|--------|------|-------------|
| `kind` | `str` | Error code, such as `UnbalancedTransaction`. It is the `error_type` of `GET /api/errors`. |
| `message` | `str` | What the errors page says about the error. |
| `file` | `str` | File of the directive that caused the error, relative to the ledger's directory, or its full path if it is outside the directory. |
| `line` | `int` | Line of the directive in its file. Always `NULL` for now. |
| `column` | `int` | Column of the directive in its line. Always `NULL` for now. |
| `date` | `date` | Date of the directive, or `NULL` for an undated directive. |
| `account` | `str` | Account the error is about, or `NULL` if the error does not name one. |
| `source` | `str` | Text of the directive that caused the error. |
| `id` | `str` | Id of the error, the `id` of `GET /api/errors`. |
| `span_start` | `int` | Byte offset in its file where the directive that caused the error starts, or `NULL` if unknown. |
| `span_end` | `int` | Byte offset in its file where the directive that caused the error ends, or `NULL` if unknown. |

How many errors of each kind there are:

```sql
SELECT kind, count(*) AS errors
FROM #errors
GROUP BY kind
ORDER BY errors DESC
```

The errors of one file, in file order:

```sql
SELECT date, kind, account, source
FROM #errors
WHERE file = 'data/2024.zhang'
```

## Types

| Type | Description | Example |
|------|-------------|---------|
| `null` | The type of the `NULL` literal. Every other type can also hold `NULL`. | `NULL` |
| `bool` | `TRUE` or `FALSE`. | `TRUE` |
| `int` | A 64-bit whole number. | `2024` |
| `decimal` | An exact decimal number of any precision. | `12.50` |
| `str` | Text. | `'Expenses:Food'` |
| `date` | A calendar date. | `2024-01-31` |
| `set` | An unordered set of strings, used by `tags`, `links` and `other_accounts`. | `{'trip', 'food'}` |
| `amount` | A decimal number with a currency. | `12.50 USD` |
| `position` | Units (an amount) plus an optional cost lot. The lot has a per-unit cost number and currency, and optionally a date and a label. | `10 VTI {120.00 USD, 2024-01-02, "lot-a"}` |
| `inventory` | A collection of positions in any number of currencies and lots. | `-30.00 USD, 10 VTI {120.00 USD}` |
| `interval` | A calendar interval of months and days, made by [`interval()`](#date-functions), to add to a date or to bin dates by. | `1 year 2 months` |
| `metas` | Metadata as an ordered list of `(key, value)` pairs, see [Structured metadata](#structured-metadata). Zhang extension. | `invoice: a.pdf; invoice: b.pdf` |

How positions combine into an inventory:

- `sum(position)` adds every position into one inventory.
- Positions with the same currency and the same cost lot (number, currency, date and label) are merged by adding their numbers. Positions with different lots stay separate, so holdings bought at different prices remain distinct.
- A position whose number becomes zero is removed, so an account that nets to zero gives an empty inventory.

An `int` is converted to `decimal` when it is combined with a `decimal` or passed to a function that expects one. No other conversions happen implicitly, apart from the string-to-date rule under [Literals](#literals). Use the [valuation functions](#valuation-functions) to move between `position`, `amount` and `inventory`.

### Ordering and comparison

`ORDER BY`, `min` and `max` order values of the same type as follows. The comparison operators `<`, `<=`, `>` and `>=` use the same order, but accept only the first four types in this list and `bool`.

- `int` and `decimal`: by numeric value.
- `str`: by character code, so case matters.
- `date`: chronologically.
- `bool`: `FALSE` before `TRUE`.
- `set`: element by element, in sorted order.
- `amount`: by currency first, then by number.
- `position`: Beancount's position order. Positions in `USD`, `EUR`, `JPY`, `CAD`, `GBP`, `AUD`, `NZD` and `CHF` come first, in that order, and other currencies follow, shorter currency names first. Ties are broken by cost number, cost currency and then units.
- `inventory`: by its positions, sorted in position order and compared one by one. An inventory that holds a single currency therefore sorts by its number, which is what `ORDER BY total DESC` relies on in the [spending by payee](#spending-by-payee) example.
- `metas`: pair by pair, by key and then by value.
- `interval`: intervals have no order, as in beanquery (`1 month` is neither more nor less than `30 days`), so `<`, `<=`, `>`, `>=`, `ORDER BY`, `min`, `max` and `PIVOT BY` reject them. They can be equal or not: `=`, `!=`, `IN`, `GROUP BY` and `DISTINCT` compare their months (a year counts as twelve) and their days, so `interval('12 months') = interval('1 year')`. beanquery rejects `=` and `!=` on intervals.

`set`, `inventory` and `metas` values cannot be group keys.

## Functions

Each table lists one signature per overload, exactly as `GET /api/query/schema` reports it. `any` means an argument of any type. An `int` argument is accepted where `decimal` is expected. Function names are case-insensitive.

Scalar functions return `NULL` when any argument is `NULL`, without evaluating the function. Aggregate functions skip `NULL` values instead.

### Aggregate functions

An aggregate function turns the values of all postings in a group into a single value. See [GROUP BY](#group-by).

| Signature | Description |
|-----------|-------------|
| `count(*) -> int` | Number of postings in the group. |
| `count(any) -> int` | Number of postings in the group for which the argument is not `NULL`. |
| `sum(int) -> int` | Sum of the integers. |
| `sum(decimal) -> decimal` | Sum of the numbers. |
| `sum(amount) -> inventory` | Sum of the amounts, kept separate per currency. |
| `sum(position) -> inventory` | Sum of the positions, with lots merged as described in [Types](#types). |
| `sum(inventory) -> inventory` | Sum of the inventories. |
| `first(any) -> any` | The first non-`NULL` value of the group, in ledger order. It has the type of its argument. |
| `last(any) -> any` | The last non-`NULL` value of the group, in ledger order. It has the type of its argument. |
| `min(any) -> any` | The smallest non-`NULL` value, in the order described under [Ordering and comparison](#ordering-and-comparison). |
| `max(any) -> any` | The largest non-`NULL` value. |

- `first` and `last` follow ledger order. `ORDER BY` does not change which posting is first.
- A `sum` over a group whose values are all `NULL` is `0` (or an empty inventory), and `first`, `last`, `min` and `max` are `NULL`.

### Valuation functions

| Signature | Description |
|-----------|-------------|
| `units(position) -> amount` | The units of the position, without the cost. |
| `units(inventory) -> inventory` | The units of every position, without costs. Lots of the same currency merge into one position. |
| `cost(position) -> amount` | Total cost of the position (units times per-unit cost), in the cost currency. A position not held at cost returns its units. |
| `cost(inventory) -> inventory` | `cost` applied to every position, then summed per currency. |
| `convert(amount, str) -> amount` | The amount converted into the currency given as the second argument, at the latest price. |
| `convert(amount, str, date) -> amount` | Like the above, at the latest price on or before the date. |
| `convert(position, str) -> amount` | The units of the position converted into the currency. The cost is not used as a price, but its currency can serve as an intermediate step (see below). |
| `convert(position, str, date) -> amount` | Like the above, at the latest prices on or before the date. |
| `convert(inventory, str) -> inventory` | Every position converted into the currency, then summed. |
| `convert(inventory, str, date) -> inventory` | Like the above, at the latest prices on or before the date. |
| `value(position) -> amount` | Market value of the position in its cost currency, at the latest price. A position not held at cost, or without a price, returns its units. |
| `value(position, date) -> amount` | Like the above, at the latest price on or before the date. |
| `value(inventory) -> inventory` | `value` applied to every position, then summed. |
| `value(inventory, date) -> inventory` | Like the above, at the latest prices on or before the date. |
| `getprice(str, str) -> decimal` | The latest price of one unit of the first currency in the second, such as `getprice('VTI', 'USD')`, or `NULL` if there is none. Currency names are converted to upper case. |
| `getprice(str, str, date) -> decimal` | Like the above, at the latest price on or before the date. |

How prices are found:

- Prices come from the `price` directives in your ledger.
- With a `date` argument, a function uses the latest price dated on or before that date. Without one, it uses the latest price in the ledger, even if it is dated in the future.
- When several prices for the same pair share a date, the last one in the ledger wins.
- A price can be used in either direction. `price VTI 120 USD` converts VTI into USD at 120, and USD into VTI at 1/120. If a pair is quoted in both directions, the direction with fewer price points is inverted and merged into the other one.
- `convert` first looks for a price from the units' currency to the target currency. If there is none and the position is held at cost, it converts in two steps through the cost currency: units to cost currency, then cost currency to target. For example, `10 VTI {100 EUR}` converts to USD with a `VTI`/`EUR` price and an `EUR`/`USD` price.
- If no price is found, the value is left unchanged. It keeps its original currency and is not dropped or set to zero, so an inventory can still contain several currencies after `convert` or `value`.
- Converting a value into its own currency returns it unchanged.
- A product with a price is rounded to 28 significant digits when it needs more, like Beancount.

### Amounts and numbers

| Signature | Description |
|-----------|-------------|
| `number(amount) -> decimal` | The number of an amount. |
| `currency(amount) -> str` | The currency of an amount. |
| `commodity(amount) -> str` | The same as `currency`. |
| `only(str, inventory) -> amount` | The total units of one currency in an inventory, such as `only('USD', sum(position))`. It is `0` in that currency if the inventory has none. |
| `filter_currency(position, str) -> position` | The position if its units are in the currency, otherwise `NULL`. |
| `filter_currency(inventory, str) -> inventory` | The positions of the inventory whose units are in the currency. |
| `abs(int) -> int` | Absolute value. |
| `abs(decimal) -> decimal` | Absolute value. |
| `abs(amount) -> amount` | The amount with a non-negative number. |
| `abs(position) -> position` | The position with non-negative units. The cost is kept. |
| `abs(inventory) -> inventory` | `abs` applied to every position. |
| `neg(int) -> int` | The negated value, the same as unary minus. |
| `neg(decimal) -> decimal` | The negated value. |
| `neg(amount) -> amount` | The negated amount. |
| `neg(position) -> position` | The position with negated units. The cost is kept. |
| `neg(inventory) -> inventory` | Every position negated. |
| `possign(decimal, str) -> decimal` | The first argument, with its sign flipped unless the account given as the second argument is under `Assets` or `Expenses`. This makes income, liability and equity amounts read as positive numbers. |
| `possign(amount, str) -> amount` | The same for an amount. |
| `possign(position, str) -> position` | The same for a position. |
| `possign(inventory, str) -> inventory` | The same for an inventory. |

### Account functions

| Signature | Description | Example |
|-----------|-------------|---------|
| `root(str) -> str` | The first component of the account name. | `root('Expenses:Food:Dining')` is `'Expenses'` |
| `root(str, int) -> str` | The first `n` components of the account name. If the account has `n` components or fewer, it is returned whole. | `root('Expenses:Food:Dining', 2)` is `'Expenses:Food'` |
| `parent(str) -> str` | The account name without its last component. It is `''` for a top-level account. | `parent('Expenses:Food:Dining')` is `'Expenses:Food'` |
| `leaf(str) -> str` | The last component of the account name. | `leaf('Expenses:Food:Dining')` is `'Dining'` |
| `account_sortkey(str) -> str` | A key that sorts accounts by type, in the order `Assets`, `Liabilities`, `Equity`, `Income` and `Expenses`, and then by name. It is the type's position, from `0` to `4`, then `-` and the account name. A name whose first component is not exactly one of these types gets `5`, so it sorts after them. [`BALANCES`](#balances) sorts by this key. | `account_sortkey('Expenses:Food')` is `'4-Expenses:Food'` |
| `under(str, str) -> bool` | Whether the account is the second argument or one of its sub-accounts: it equals it, or starts with it followed by `:`. A sibling whose name only starts the same way is not under it. Zhang extension. | `under('Assets:Bank:Cash', 'Assets:Bank')` is `TRUE`, `under('Assets:Banking', 'Assets:Bank')` is `FALSE` |

### Account and commodity directives

These functions read the `open`, `close` and `commodity` directives of the ledger, as in beanquery, so they work on every table. An account's directives are its earliest `open` and its earliest `close`; a currency's is its last `commodity` directive. An unknown account or currency gives `NULL`.

| Signature | Description |
|-----------|-------------|
| `open_date(str) -> date` | Date of the account's `open` directive, or `NULL` if it has none. |
| `close_date(str) -> date` | Date of the account's `close` directive, or `NULL` while it is open. |
| `open_meta(str, str) -> str` | A metadata value of the account's `open` directive, such as `open_meta(account, 'institution')`, or `NULL` if it is not set. |
| `open_meta(str) -> metas` | All the metadata of the account's `open` directive, as [structured pairs](#structured-metadata): an empty list when it has none, `NULL` when the account has no `open` directive. |
| `commodity_meta(str, str) -> str` | A metadata value of the currency's `commodity` directive, such as `commodity_meta(currency, 'name')`. |
| `commodity_meta(str) -> metas` | All the metadata of the currency's `commodity` directive: an empty list when it has none, `NULL` without a `commodity` directive. |
| `currency_meta(str, str) -> str`, `currency_meta(str) -> metas` | The same as `commodity_meta`. |

Metadata is not inherited: `open_meta('Assets:Bank:Checking', 'institution')` is `NULL` even if `Assets:Bank` has an `institution`. beanquery's one-argument forms return dictionaries that also hold `filename` and `lineno`; Zhang returns only the directive's own metadata.

### Date functions

| Signature | Description | Example |
|-----------|-------------|---------|
| `year(date) -> int` | Year. | `year(2024-05-17)` is `2024` |
| `month(date) -> int` | Month, from 1 to 12. | `month(2024-05-17)` is `5` |
| `day(date) -> int` | Day of the month. | `day(2024-05-17)` is `17` |
| `quarter(date) -> str` | Year and quarter, as text. | `quarter(2024-05-17)` is `'2024-Q2'` |
| `weekday(date) -> str` | Three-letter English name of the day of the week. | `weekday(2024-01-05)` is `'Fri'` |
| `yearmonth(date) -> date` | First day of the date's month. | `yearmonth(2024-05-17)` is `2024-05-01` |
| `today() -> date` | The current date in the ledger's timezone (the `timezone` option). | |
| `date(int, int, int) -> date` | The date of a year, month and day, or `NULL` if there is no such day or the year is outside 1 to 9999. | `date(2024, 2, 29)` is `2024-02-29`, `date(2023, 2, 29)` is `NULL` |
| `date(str) -> date` | The date written in the text as `YYYY-MM-DD`, where the month and the day may have one digit, or `NULL` if it is not one. | `date('2024-2-9')` is `2024-02-09` |
| `date_add(date, int) -> date` | The date moved by a number of days, backwards for a negative number. | `date_add(2024-02-28, 1)` is `2024-02-29` |
| `date_diff(date, date) -> int` | The number of days from the second date to the first. | `date_diff(2024-03-01, 2024-02-01)` is `29` |
| `date_trunc(str, date) -> date` | The first day of the date's `week` (a Monday), `month`, `quarter`, `year`, `decade`, `century` (1901, 2001, ...) or `millennium` (1001, 2001, ...), or `NULL` for any other field. Fields are lower-case. | `date_trunc('month', 2024-05-17)` is `2024-05-01` |
| `date_part(str, date) -> int` | A field of the date: `weekday` or `dow` (Monday is 0), `isoweekday` or `isodow` (Monday is 1), `week` (the ISO week), `month`, `quarter`, `year`, `isoyear` (the year of the ISO week), `decade`, `century`, `millennium` or `epoch` (seconds since 1970-01-01), or `NULL` for any other field. There is no `day` field; use `day(date)`. | `date_part('week', 2016-01-03)` is `53` |
| `interval(str) -> interval` | An interval written as a number and a unit, `day`, `week`, `month` or `year` (or their plurals), with an optional sign, or `NULL` for any other text. | `interval('3 months')`, `interval('-1 year')` |
| `date_bin(interval, date, date) -> date` | The start of the bin that holds the date among the bins of the interval laid from the origin (the third argument). See below. | `date_bin(interval('7 days'), date, 2024-01-01)` |
| `date_bin(str, date, date) -> date` | The same, with the interval written as text. | `date_bin('1 month', date, 2024-01-01)` |

The `year`, `month` and `day` columns are shortcuts: `year` is the same as `year(date)`.

`date_trunc` buckets dates by calendar periods, and `date_bin` by any interval from any origin:

```sql
SELECT date_trunc('week', date) AS week, sum(position) AS spent
WHERE account ~ '^Expenses:'
GROUP BY week ORDER BY week
```

- The bins of `date_bin(stride, date, origin)` start at `origin + k × stride` for every whole number `k`, each computed from the origin, so the bins of `'1 month'` from `2024-01-31` start on `2024-02-29`, `2024-03-31`, `2024-04-30`, ... Dates before the origin fall in bins laid backwards from it.
- A date exactly on a bin boundary starts that bin: `date_bin('1 month', 2024-02-01, 2024-01-01)` is `2024-02-01`.
- A stride of zero or less, a stride whose months and days have opposite signs (such as `interval('2 months') - interval('61 days')`, whose bins would not follow each other in order), or a text that `interval()` cannot read, gives `NULL`.

Dates are those of beancount's calendar, the years 1 to 9999. A date function or date arithmetic whose result falls outside, such as `date_add(9999-12-31, 1)` or `date_trunc('decade', 0002-12-15)`, gives `NULL` (beanquery raises an error).

Intervals add to dates with `+` and `-`, see [Arithmetic](#arithmetic). Weeks are a Zhang extension: in beanquery, `interval('1 week')` is `NULL`.

### Metadata functions

| Signature | Description |
|-----------|-------------|
| `meta(str) -> str` | Value of a metadata key on the posting, or `NULL` if it is not set. |
| `entry_meta(str) -> str` | Value of a metadata key on the transaction, or `NULL` if it is not set. |
| `any_meta(str) -> str` | Value of a metadata key on the posting, falling back to the transaction, or `NULL` if neither has it. |
| `meta_values(str) -> set` | Every value of a metadata key on the posting, as a set; empty if it is not set. Zhang extension. |
| `entry_meta_values(str) -> set` | Every value of a metadata key on the transaction, as a set. Zhang extension. |

Metadata values are always returned as text. When a key is repeated, `meta`, `entry_meta` and `any_meta` return its first value, and `meta_values` and `entry_meta_values` all of them: `'b.pdf' IN entry_meta_values('invoice')` finds a transaction with several `invoice` lines. On the [other tables](#other-tables), all three read the metadata of the row's directive. On `#budgets` they read the metadata of the `budget` directive, on `#budget_events` that of the budget directive, and on `#errors` the details Zhang records about the error.

Which metadata lines of a transaction belong to a posting depends on the file format, see [Transactions](/reference/directives/transaction/#which-lines-belong-to-a-posting). For example, with

```zhang
2024-01-02 * "Cafe" "lunch"
  category: "meals"
  Assets:Cash -10 CNY
  Expenses:Food 10 CNY
    category: "food"
```

the `Expenses:Food` posting has `meta('category')` `'food'`, `entry_meta('category')` `'meals'` and `any_meta('category')` `'food'`, and the `Assets:Cash` posting has `NULL`, `'meals'` and `'meals'`.

### Structured metadata

The `metas` columns and `open_meta(account)` hold metadata as a `metas` value: a list of `(key, value)` pairs, sorted by key, with every value of a repeated key kept in the order it is written. Values are text. Zhang does not keep the order of different keys, so they are sorted. `str(metas)` writes the pairs as `key: value` joined with `; `, the [HTTP API](#cell-encoding) sends them as a list of `{"key": ..., "value": ...}` objects, and [CSV export](#export-as-csv) writes them like `str`. The text form is meant for reading and is not escaped, so it is ambiguous when a value itself contains `; ` or `: `; programs should read the HTTP API's pairs, or the values with `meta_values` and `entry_meta_values`. `metas` is a Zhang extension; to filter on a key, use `meta`, `entry_meta`, `meta_values` or `entry_meta_values`.

### Search functions

Zhang extensions for keyword search. "Ignoring case" compares the texts after converting both to lower case (Unicode).

| Signature | Description | Example |
|-----------|-------------|---------|
| `icontains(str, str) -> bool` | Whether the text contains the second argument, ignoring case. Unlike `~`, the needle is plain text, not a regular expression. | `icontains(payee, 'café')` |
| `any_icontains(set, str) -> bool` | Whether an element of the set contains the text, ignoring case. | `any_icontains(tags, 'trip')` |
| `intersects(set, set) -> bool` | Whether the two sets have an element in common. | `intersects(tags, :tags)` |
| `set(str, ...) -> set` | The set of the given strings, with any number of them. `set()` is the empty set. | `intersects(tags, set('trip', 'food'))` |

```sql
SELECT date, payee, narration, account, position
WHERE icontains(payee, 'coffee') OR icontains(narration, 'coffee') OR any_icontains(tags, 'coffee')
```

### String functions

| Signature | Description | Example |
|-----------|-------------|---------|
| `str(any) -> str` | Text form of any value. Booleans become `TRUE` and `FALSE`, sets are joined with `, `, and an inventory is written in parentheses. | `str(2024-01-31)` is `'2024-01-31'` |
| `length(str) -> int` | Number of characters in the string. | `length('Food')` is `4` |
| `length(set) -> int` | Number of elements in the set. | `length(tags)` |
| `maxwidth(str, int) -> str` | The text shortened to fit in `n` characters, like Python's `textwrap.shorten`. See below. | `maxwidth('Paying the  rent', 12)` is `'Paying [...]'` |

`maxwidth(text, n)` works in two steps:

1. Every run of whitespace becomes a single space, and spaces at both ends are removed. A text that now has at most `n` characters is returned as it is: `maxwidth('  Eating out ', 48)` is `'Eating out'`.
2. A longer text keeps as many whole words as fit in `n` characters together with the placeholder ` [...]`, which is added at the end. If not even the first word fits, the result is `'[...]'`. As in Python, a word can also be broken after a hyphen between letters: `maxwidth('abc-def-ghi jkl', 12)` is `'abc- [...]'`.

`n` must be at least 5, the length of `[...]`. A smaller width is an error. [`JOURNAL`](#journal) uses `maxwidth` to shorten payees and narrations.

## HTTP API

The Explore page uses the same HTTP endpoints, and you can call them from scripts. If [authentication](/deployment/authentication/) is enabled, sign in first or send the `ZHANG_AUTH` credentials as an HTTP Basic `Authorization` header, as for the rest of the API.

### Run a query

`POST /api/query` with a JSON body:

```shell
curl -X POST http://localhost:8000/api/query \
  -H 'Content-Type: application/json' \
  -d '{"query": "SELECT account, sum(position) WHERE account ~ \"^Assets:Bank\" GROUP BY account"}'
```

A successful response has HTTP status 200:

```json
{
  "data": {
    "columns": [
      { "name": "account", "type": "str" },
      { "name": "sum(position)", "type": "inventory" }
    ],
    "rows": [
      [
        "Assets:Bank:Checking",
        { "positions": [ { "units": { "number": "1520.35", "currency": "USD" }, "cost": null } ] }
      ]
    ]
  }
}
```

- `columns` lists the result columns in order. Each has a `name` and a `type`, which is one of `null`, `bool`, `int`, `decimal`, `str`, `date`, `set`, `amount`, `position`, `inventory`, `interval` and `metas`.
- `rows` is a list of rows. Each row is a list with one cell per column, in the same order.
- With `"count_total": true` in the request, the result also has `total`, the number of rows before `LIMIT` and `OFFSET`, to page through a result. Without it there is no `total`.

### Cell encoding

| Type | JSON | Example |
|------|------|---------|
| any `NULL` | `null` | `null` |
| `bool` | boolean | `true` |
| `int` | number | `2024` |
| `decimal` | string | `"1520.35"` |
| `str` | string | `"Expenses:Food"` |
| `date` | string, `YYYY-MM-DD` | `"2024-01-31"` |
| `set` | array of strings, sorted | `["food", "trip"]` |
| `amount` | object | `{"number": "12.50", "currency": "USD"}` |
| `position` | object | `{"units": {"number": "10", "currency": "VTI"}, "cost": {"number": "120.00", "currency": "USD", "date": "2024-01-02", "label": null}}` |
| `inventory` | object | `{"positions": [ ...positions... ]}` |
| `interval` | string | `"1 year 2 months"` |
| `metas` | array of objects, in order | `[{"key": "invoice", "value": "a.pdf"}, {"key": "invoice", "value": "b.pdf"}]` |

- Decimal numbers, including the `number` fields of amounts and costs, are sent as strings so that no precision is lost. They never use exponent notation and keep their decimal places (`"12.50"`). Parse them with a decimal library rather than as floating point.
- Integers are 64-bit and sent as JSON numbers. JavaScript reads JSON numbers as doubles, so an `int` cell beyond ±2^53 (9,007,199,254,740,992) loses precision in a JavaScript client. Counts and date parts never come close to that.
- In a position, `cost` is `null` when the position is not held at cost. Otherwise it holds the per-unit cost `number` and `currency`, and the lot's `date` and `label`. Either of the last two can be `null`.
- The positions of an inventory are sorted by units currency, then by cost, with the position that has no cost first. An empty inventory is `{"positions": []}`.

### Errors

A query that cannot be parsed, type-checked or run returns HTTP status 400. Unlike a successful response, the body is not wrapped in `data`. For example, `SELECT nosuchcolumn, position` gives:

```json
{
  "message": "unknown column 'nosuchcolumn'",
  "line": 1,
  "column": 8
}
```

- `line` and `column` give the position of the problem in the query text. Both start at 1.
- `column` counts Unicode characters, not bytes, so a Chinese character or an accented letter counts as one column.
- Errors include syntax errors, unknown columns or functions, arguments of the wrong type, invalid `GROUP BY` usage, invalid regular expressions, unsupported statements or clauses, and the [limits](#limits) below.
- Some errors have no position, and `line` and `column` are then `null`: a query that is too long, a query that runs out of time, a result that is too large, and a few errors found while rows are evaluated, such as an integer overflow inside `sum`.

### Limits

These limits protect the server from queries that would take too much memory or time. Exceeding one returns the HTTP 400 error described above.

| Limit | Value | Error |
|-------|-------|-------|
| Query length | 64 KiB (65,536 bytes of UTF-8 text) | `the query is too long (...)`, without a position. |
| Nesting depth | 64 levels | `the query is nested too deeply (at most 64 levels)`, at the position where the limit is reached. |
| Compiled size of one regular expression | 1 MiB | `invalid regular expression: Compiled regex exceeds size limit ...`, at the pattern. |
| Execution time | 10 seconds | `the query was stopped because it ran longer than the 10s time limit`, without a position. |
| Result size | 1,000,000 values by default | `the result is too large: ...`, without a position. |

- Nesting counts parentheses, function calls, `IN` lists, `NOT` and unary minus that are placed inside each other. A long chain of `AND`, `OR`, `+` or `*`, such as `account = 'A' OR account = 'B' OR ...`, is not nested and can be as long as the length limit allows.
- The execution time includes building the rows of the `postings` table and applying the period clauses. While a query runs, it holds a read lock on the ledger, and the time limit bounds how long that lock is held.
- The result size counts each cell as one value, plus one for each position of an inventory, each element of a set, each pair of a `metas` value and each 64 bytes of text, including the text of those elements and pairs. The rows that a query collects before `ORDER BY`, `DISTINCT` and `LIMIT` count too, and so do the groups of an aggregate query while they are built and the months that [`#budgets`](#budgets) generates, one value each. A `PIVOT BY` table counts all of its cells, including the empty ones, and is checked before it is built. A query that goes over the limit fails with an error that suggests narrowing it with `FROM` or `WHERE`, or adding a `LIMIT`.
- Server operators can raise or lower the result size limit with the environment variable `ZHANG_QUERY_MAX_RESULT_VALUES`.
- `LIMIT` keeps a result small, and so does the way the [running balance](#the-running-balance) is computed. `balance` is only built for the rows that end up in the result, unless the query sorts, groups or de-duplicates by it. `units(balance)` and `cost(balance)`, and so `JOURNAL ... AT units` and `AT cost`, are added up per currency without keeping the lots.
- The same limits apply to [CSV export](#export-as-csv).

### Export as CSV

`POST /api/query/csv` takes the same JSON body as `POST /api/query` and returns the result as a CSV file:

```shell
curl -X POST http://localhost:8000/api/query/csv \
  -H 'Content-Type: application/json' \
  -d '{"query": "SELECT account, sum(position) AS total WHERE account ~ \"^Assets:Broker\" GROUP BY account"}'
```

- The response has the content type `text/csv; charset=utf-8` and the header `Content-Disposition: attachment; filename="query.csv"`.
- A query that fails returns the same HTTP 400 error, with the same JSON body, as `POST /api/query`. See [Errors](#errors).

To make the file easy to use in a spreadsheet, amounts are turned into plain numbers, the way beanquery's *numberify* option (`bean-query -m`) does:

- An `amount`, `position` or `inventory` column named `name` becomes one `decimal` column per currency, named `name (CUR)`. A column in which no currency occurs is left out.
- The columns made from one column are ordered by the number of rows in which their currency occurs, most first. Ties are broken by currency name in descending order, so `USD` comes before `EUR`.
- An `amount` gives its number in the column of its currency. A zero amount counts as missing: its cell is empty, and it does not count towards the column's currencies.
- A `position` gives the number of its units, without the cost. Zero units are written as `0`.
- An `inventory` gives the total units of each currency, adding up all of its lots and ignoring their costs. A total of zero is an empty cell.
- Every other column is kept as it is.

For example, with a cash account and a gold holding, the query above gives:

```text
account,total (USD),total (GLD)
Assets:Broker:Cash,855.83,
Assets:Broker:GLD,,17
```

The file follows RFC 4180:

- The first record holds the column names. Every record ends with CRLF, the last one included.
- `NULL` is an empty field. Booleans are written `TRUE` and `FALSE`, dates `YYYY-MM-DD`, sets as their elements, sorted and joined with `,`, intervals like `1 year 2 months`, and `metas` as `key: value` pairs joined with `; ` (not escaped, see [Structured metadata](#structured-metadata)).
- Numbers are exact. They keep all their digits and decimal places, and are never padded, rounded or written with an exponent.
- A field that contains `,`, `"`, a carriage return or a line feed is put in double quotes, with each `"` doubled. A row with a single empty field is written as `""`, so that it is not a blank line.

:::caution[Formulas in spreadsheets]
Text is written as it is, without any protection against formulas, as in beanquery's CSV output. A text cell that begins with `=`, `+`, `-` or `@`, such as a payee or a narration, can be read as a formula when the file is opened in a spreadsheet application. Be careful with exports of ledgers whose text you did not write yourself, or import the columns as text.
:::

### List saved queries

`GET /api/query/saved` lists the queries saved in the ledger with the [`query` directive](/reference/directives/query/), in ledger order. Each has a `name`, the `query` text, the directive's `date`, and `valid` and `error`, which tell whether the query compiles with the current engine and why not. See the [`query` directive](/reference/directives/query/#http-api) for an example response. To run a saved query, send its `query` text to `POST /api/query`.

### Built-in queries

`GET /api/query/builtins` lists the queries behind the app's figures, and `POST /api/query/builtins/{name}/text` writes one out with its parameters filled in. See [Built-in queries](/reference/builtin-queries/).

### Schema

`GET /api/query/schema` describes every table and every function overload. The reference panel on the Explore page is built from it.

```json
{
  "data": {
    "columns": [
      { "name": "date", "type": "date", "description": "Date of the transaction." }
    ],
    "functions": [
      { "name": "count", "signature": "count(*) -> int", "description": "Number of rows.", "aggregate": true }
    ],
    "tables": [
      {
        "name": "postings",
        "description": "One row per posting of every transaction, ...",
        "columns": [{ "name": "date", "type": "date", "description": "Date of the transaction." }]
      }
    ]
  }
}
```

- `columns` has one entry per column, 33 in all, in the order of the [column table](#columns).
- `tables` has one entry per table, `postings` first, then the [other tables](#other-tables) in the order listed there, then `budgets`, `budget_events` and `errors`. `name` has no `#`. The `postings` entry has the same columns as `columns`, and the attributes of a structured column are listed as columns named like `open.date`.
- `functions` has one entry per overload, 89 in all: first the aggregate functions, then the scalar functions, including `account_sortkey` and `maxwidth`. `signature` uses the same form as the tables on this page, and `aggregate` is `true` for the [aggregate functions](#aggregate-functions) and `false` for all others.

## Examples

### Monthly expenses by category

```sql
SELECT year, month, root(account, 2), sum(position) WHERE account ~ "^Expenses" GROUP BY 1, 2, 3 ORDER BY 1, 2, 3
```

One row per month and second-level expense account (such as `Expenses:Food`), with the total spent. `GROUP BY 1, 2, 3` and `ORDER BY 1, 2, 3` refer to the first three targets by number.

### Categories above a threshold

```sql
SELECT root(account, 2) AS category, sum(position) AS total
WHERE account ~ '^Expenses' AND currency = 'USD'
GROUP BY category
HAVING sum(number) > 1000
ORDER BY category
```

The expense categories on which you spent more than 1000 USD. `HAVING` filters the groups after they are added up; `WHERE` could not, because it sees one posting at a time.

### Monthly expenses with one column per year

```sql
SELECT month, year, sum(position) AS total
WHERE account ~ '^Expenses:Food'
GROUP BY month, year
PIVOT BY month, year
```

One row per month and one column per year, so the same month of different years sits side by side. A month without postings in a year is empty.

### Spending by payee

```sql
SELECT payee, sum(cost(position)) AS total WHERE account ~ "^Expenses" GROUP BY payee ORDER BY total DESC LIMIT 20
```

The 20 payees you spent the most with. `cost(position)` values purchases made at cost by what you paid for them, and `ORDER BY total DESC` sorts by the alias.

### Postings with a tag

```sql
SELECT date, payee, account, position WHERE 'trip-new-york' IN tags
```

Every posting of every transaction tagged `#trip-new-york`.

### Holdings at cost and market value

```sql
SELECT account, units(sum(position)) AS qty, cost(sum(position)) AS book, convert(units(sum(position)), "USD") AS market WHERE account ~ "^Assets:Trading" GROUP BY account
```

For each trading account: the quantity held, its book value (what it cost), and its market value in USD at the latest prices. Holdings without a USD price stay in their own currency in the `market` column.

### Recent postings

```sql
SELECT date, payee, account, position ORDER BY date DESC LIMIT 20
```

The 20 most recent postings across all accounts.

### Expenses in a date range, excluding taxes

```sql
SELECT account, sum(position) AS total
WHERE account ~ '^Expenses' AND account !~ ':Taxes(:|$)'
  AND date >= 2024-01-01 AND date < 2024-04-01
GROUP BY account
ORDER BY account
```

Uses `!~` to exclude accounts and bare date literals for the range.

### Quarterly totals with a count

```sql
SELECT quarter(date) AS q, count(*) AS postings, sum(number) AS total, sum(number) / count(*) AS average
WHERE account ~ '^Expenses:Food' AND currency = 'USD'
GROUP BY q
ORDER BY q
```

Postings, total and average amount per quarter. Filtering on `currency` first makes `sum(number)` meaningful, because `number` ignores the currency.

### Transactions paid by credit card

```sql
SELECT DISTINCT date, description
WHERE 'Liabilities:CreditCard' IN other_accounts AND account ~ '^Expenses'
ORDER BY date DESC
```

`other_accounts` holds the accounts of the other postings in each transaction, so this finds expenses paid from the card. `DISTINCT` lists each transaction once even if it has several expense postings.

### Postings without a payee

```sql
SELECT date, narration, account, position
WHERE payee IS NULL AND account IN ('Expenses:Misc', 'Expenses:Uncategorized')
ORDER BY date DESC
```

Uses `IS NULL` and a list after `IN`.

### Invoices recorded in metadata

```sql
SELECT date, payee, entry_meta('invoice') AS invoice, position
WHERE entry_meta('invoice') IS NOT NULL AND leaf(account) = 'Consulting'
ORDER BY date
```

Lists postings to accounts named `...:Consulting` whose transaction has an `invoice` metadata entry.

### Portfolio value at the end of a year

```sql
SELECT account, value(sum(position), 2024-12-31) AS market_value
WHERE account ~ '^Assets:Investments' AND date <= 2024-12-31
GROUP BY account
ORDER BY account
```

Holdings as of 31 December 2024, valued at the prices in effect on that date, in each holding's cost currency.

### Income and expenses as positive numbers

```sql
SELECT root(account, 1) AS type, sum(possign(position, account)) AS total
WHERE account ~ '^(Income|Expenses)' AND year = 2024
GROUP BY type
```

Income is negative in the ledger. `possign` flips the sign of each income posting before the sum, so both totals read as positive numbers.

### Income statement for a year

```sql
SELECT account, sum(position) AS total
FROM OPEN ON 2024-01-01 CLOSE ON 2025-01-01
WHERE account ~ '^(Income|Expenses)'
GROUP BY account
ORDER BY account
```

`OPEN ON` moves the income and expenses of earlier years into equity, and `CLOSE ON` drops everything from 2025 on, so the totals cover 2024 only.

### Balance sheet

```sql
BALANCES FROM CLOSE ON 2025-01-01 CLEAR
WHERE account ~ '^(Assets|Liabilities|Equity)'
```

The balances of the assets, liabilities and equity accounts at the start of 2025, sorted by account type. `CLEAR` moves all income and expenses into `Equity:Earnings:Current`, so the equity accounts include the earnings.

### Holdings at cost

```sql
BALANCES AT cost WHERE account ~ '^Assets'
```

The book value of every asset account, in the cost currency of its holdings.

### Journal of an account

```sql
JOURNAL 'Assets:Bank:Checking' FROM OPEN ON 2024-01-01 CLOSE ON 2025-01-01
```

Every posting to the account in 2024, with the running balance. The first row is the opening balance on 31 December 2023, so the balance column shows the real balance of the account.

### Balance at the end of each day

```sql
SELECT date, last(balance) AS closing
WHERE account = 'Assets:Bank:Checking'
GROUP BY date
ORDER BY date
```

One row per day with postings, with the account's balance at the end of the day. The result has a date and an inventory column, so the Explore page draws it as a line chart.

## Differences from BQL and beanquery

### Not available yet

- **`PRINT`**, which is rejected with an error.
- **Subqueries** after `FROM`, the double-quoted table names of beanquery (`FROM "prices"`), and its one-row table `FROM #`.
- **The beanquery columns** `posting_flag`, `filename`, `lineno`, `location`, `entry`, `accounts` and `type` of the postings table, and `lineno` of `#entries`: Zhang does not keep line numbers.
- **Subscripts**, such as `meta['name']`. Use `meta('name')`.
- **Operators `BETWEEN` and `%`**, and beanquery's quoted identifiers.
- **Functions not listed on this page**, such as `round`, `safediv`, `has_account`, `grep`, `subst`, `upper`, `lower`, `joinstr`, `findfirst`, `parse_date` and the conversion functions `int`, `decimal` and `date(date)`. Calling one is an error.


### Behaving differently

- **`SELECT *` includes `account`.** beanquery expands `*` to `date, flag, payee, narration, position`. Zhang adds `account` before `position`, because a posting is hard to read without its account.
- **Standard three-valued logic.** In beanquery, `NOT NULL` is `TRUE`, so `NOT (payee = 'x')` keeps postings without a payee, and `NULL AND FALSE` is `NULL`. In Zhang, `NOT NULL` is `NULL` and `NULL AND FALSE` is `FALSE`, as in SQL.
- **One-element lists work.** `payee IN ('Amazon')` works in Zhang. beanquery reads `('Amazon')` as a parenthesized string and needs `('Amazon',)`.
- **Comments** start with `--`. beanquery's `;` line comments and `/* */` block comments are not supported. A single `;` is only allowed at the end of the query.
- **Regular expressions** use Rust syntax, which has no look-around or back-references.
- **Booking methods.** Accounts that use `STRICT`, `AVERAGE`, `AVERAGE_ONLY` or `NONE` are booked FIFO for now, and an ambiguous `STRICT` match is not an error. See [Lot booking](#lot-booking).
- **Limits.** Queries are limited in length, nesting depth, regular-expression size, execution time and result size. See [Limits](#limits).
- **Errors carry a position.** Every query error reports the line and column where it was found, whenever it can be located.
- **Exact decimals throughout.** Numbers are arbitrary-precision decimals, and amounts are never stored with a fixed number of decimal places.
- **Column names of `BALANCES` and `JOURNAL`** are those of the equivalent `SELECT`: `sum(position)`, `sum(cost(position))` and `maxwidth(payee, 48)`. beanquery names them `SUM((position))`, `SUM(cost(position))` and `MAXWIDTH(payee, 48)`.
- **The running balance.** `balance` cannot be used in `FROM` or `WHERE`, and it adds up exactly the rows that pass them. beanquery updates its balance each time it evaluates the column, so in a `WHERE` clause it would count the rows it tests rather than the rows it keeps.
- **`account_sortkey`** of a name whose first component is not an account type returns a key that sorts after all the types. beanquery raises an error.
- **`date_bin` lays its bins from the origin.** With a stride of months or years, beanquery adds each stride to the previous bin, so bins from a month end drift (`01-31`, `02-28`, `03-28`, ...), and it puts a date exactly on a bin boundary other than the origin into the previous bin: `date_bin('1 month', 2000-02-01, 2000-01-01)` is `2000-01-01` there. In Zhang the bins are `origin + k × stride` (`01-31`, `02-28`, `03-31`, ...) and a date on a boundary starts its bin (`2000-02-01`). A zero stride, or a stride text that `interval()` cannot read, is `NULL`; beanquery fails.
- **`interval()` accepts weeks**, seven days each. beanquery returns `NULL` for them.
- **`NULL` arguments.** A `NULL` literal is accepted wherever a value is, and a function given `NULL` returns `NULL`: `date_add(NULL, 1)` is `NULL`. beanquery types `NULL` apart and rejects such a call.
- **Interval arithmetic.** `interval - interval` is an interval; beanquery declares it a date. `interval - date` is an error; beanquery accepts it and fails while running.
- **Interval comparison.** Intervals can be compared with `=`, `!=` and `IN`, which beanquery rejects, by their months and days, so `GROUP BY` and `DISTINCT` treat `interval('1 year') + interval('-1 month')` and `interval('11 months')` as one value (beanquery keeps them apart). Ordering them, which beanquery fails on while running, is an error when the query is checked.
- **Dates are years 1 to 9999.** A date function or date arithmetic whose result falls outside gives `NULL`; beanquery raises an error.
- **`OFFSET`** is a Zhang extension; beanquery has only `LIMIT`.
- **Parameters.** The `JOURNAL` pattern, the `OPEN ON` and `CLOSE ON` dates, and `LIMIT` and `OFFSET` can be [parameters](#parameters). beanquery only accepts literals there.
- **The `FROM` expression filters after the period clauses.** This is what beanquery does. In BQL v2, the expression chose the transactions before `OPEN`, `CLOSE` and `CLEAR` were applied.
- **Equity accounts.** An `account_previous_*` or `account_current_*` option whose value is not a valid account name is ignored, and the default account is used.
- **The last entry of a period**, which dates the `T` transactions of `CLEAR` and the `C` transaction of a bare `CLOSE`, ignores Zhang's budget directives, which Beancount does not have.
- **Columns in `HAVING`.** beanquery reads a column used outside an aggregate function in `HAVING` from an arbitrary posting. Zhang reads a group key as the value of each group and rejects any other column.
- **`HAVING` must be a boolean expression.** beanquery also accepts other values and keeps the groups for which they are not zero or empty, as in `HAVING sum(number)`. Zhang rejects them, as it does in `WHERE`; write `HAVING sum(number) != 0`.
- **`PIVOT BY` with `NULL` values.** beanquery fails when the values of a pivot target mix `NULL` with others. Zhang sorts `NULL` first and names its column `NULL`.
- **`PIVOT BY` without grouping** is an error in Zhang. beanquery fails while it runs such a query.
- **CSV export of a pivot with missing cells.** beanquery fails to numberify the empty cells of a pivoted amount or inventory column. Zhang leaves them empty.
- **Metadata is text.** beanquery's `meta` columns are dictionaries that also hold `filename` and `lineno`. In Zhang, `meta` is the text `key: "value", ...` of the directive's own metadata (of the posting's in the postings table), and `open.meta` and `close.meta` likewise. The structured form is the Zhang `metas` type, see [Structured metadata](#structured-metadata).
- **`open` and `close` of `#accounts` read as their date** when used without an attribute. In beanquery they are the whole directives.
- **`entry_meta()` and `any_meta()` work on every table**, like `meta()`. beanquery only accepts them on the postings table.
- **`#entries` holds Zhang's directives.** It has Zhang's budget directives, and a `balance ... with pad` is one `balance` entry followed by its padding transaction, where beancount has a `pad` and a `balance` entry. The `id` of an entry is Zhang's id, not beancount's hash.
- **`discrepancy`, `actual` and `passed` follow Zhang's balance checks.** Like beancount, Zhang measures the balance from the sum of the postings of the account and its sub-accounts, and an assertion moves no balance. An assertion without a `~` tolerance must match exactly, where beancount allows a tolerance inferred from the decimals of the asserted amount.
- **`#documents` also lists the documents of transactions**, after the `document` directives: the values of the `document` metadata of transactions and postings, which beancount does not treat as documents.
- **CSV export keeps exact numbers.** `bean-query` pads numbers for alignment (`" 600.00"`), rounds numberified numbers to each currency's display precision (`360.03` instead of `360.03016`), and writes some numbers with an exponent (`1E+3`). Zhang does none of this.
