---
title: Querying Your Ledger
description: Use the query page to answer questions about your ledger, with common queries to start from.
sidebar:
  order: 6
---

How much did we spend on food in March? What was in the bank at the end of the quarter? Where did the hotel money go? The **Query** page answers questions like these with a query language compatible with Beancount's (BQL). This guide shows how to use the page and gives queries to start from. Every clause, table and function is described in [Query Language](/reference/query-language/).

## The Query page

Open **Query** from the **More** group of the sidebar.

- Type a query in the editor and select **Run**, or press <kbd>Ctrl</kbd>+<kbd>Enter</kbd> (<kbd>Cmd</kbd>+<kbd>Enter</kbd> on macOS).
- The result is a table. A result with a label column and an amount column is also drawn as a chart: a line over time, a bar chart, or a treemap when the labels are accounts. The **Table** and **Chart** switch chooses what you see.
- **Examples** puts a ready-made query in the editor and runs it.
- **Saved** lists the queries saved in your ledger (see [below](#save-a-query)).
- **Reference** lists the tables, columns and functions. Select one to insert it into the editor.
- If the query has an error, the editor points at the line and column.

Queries are read-only: they never change your ledger.

## Queries to start from

The results below come from this small ledger:

<details>
<summary>The example ledger</summary>

```zhang title="main.zhang"
option "title" "Household"
option "operating_currency" "CNY"

1970-01-01 commodity CNY

2024-01-01 open Assets:Bank:Checking CNY
2024-01-01 open Assets:Cash CNY
2024-01-01 open Liabilities:CreditCard CNY
2024-01-01 open Income:Salary CNY
2024-01-01 open Expenses:Food:Groceries CNY
2024-01-01 open Expenses:Food:Restaurants CNY
2024-01-01 open Expenses:Transport CNY
2024-01-01 open Expenses:Travel CNY
2024-01-01 open Equity:Opening-Balances CNY

2024-01-01 balance Assets:Bank:Checking 5000 CNY with pad Equity:Opening-Balances
2024-01-01 balance Assets:Cash 300 CNY with pad Equity:Opening-Balances

2024-03-01 * "ACME Corp" "March salary"
  Assets:Bank:Checking 12000 CNY
  Income:Salary
2024-03-02 * "Supermarket" "Weekly shopping"
  Liabilities:CreditCard -386.50 CNY
  Expenses:Food:Groceries
2024-03-05 * "Noodle House" "Lunch" #work
  Liabilities:CreditCard -48 CNY
  Expenses:Food:Restaurants
2024-03-09 * "Hotel" "Two nights in Hangzhou" #trip-hangzhou
  Liabilities:CreditCard -840 CNY
  Expenses:Travel
2024-03-10 * "Metro" "Top-up"
  Assets:Cash -100 CNY
  Expenses:Transport
2024-03-16 * "Supermarket" "Weekly shopping"
  Liabilities:CreditCard -412.30 CNY
  Expenses:Food:Groceries
2024-03-25 * "Bank" "Credit card bill"
  Assets:Bank:Checking -1686.80 CNY
  Liabilities:CreditCard
2024-04-01 * "ACME Corp" "April salary"
  Assets:Bank:Checking 12000 CNY
  Income:Salary
2024-04-06 * "Supermarket" "Weekly shopping"
  Liabilities:CreditCard -295.00 CNY
  Expenses:Food:Groceries
```

</details>

Each query reads the **postings**: one row per posting, with the date, payee and narration of its transaction, its `account` and its `position` (the amount). `~` matches a regular expression anywhere in a text and ignores case, and `^` anchors it to the start.

### Spending by category in a month

```sql
SELECT account, sum(position) AS spent
WHERE account ~ '^Expenses' AND year = 2024 AND month = 3
GROUP BY account
ORDER BY account
```

| account | spent |
|---|---|
| Expenses:Food:Groceries | 798.80 CNY |
| Expenses:Food:Restaurants | 48 CNY |
| Expenses:Transport | 100 CNY |
| Expenses:Travel | 840 CNY |

The labels are accounts, so the page also draws this as a treemap.

### Spending per month

```sql
SELECT yearmonth(date) AS month, sum(position) AS spent
WHERE account ~ '^Expenses'
GROUP BY month
ORDER BY month
```

| month | spent |
|---|---|
| 2024-03-01 | 1786.80 CNY |
| 2024-04-01 | 295.00 CNY |

`yearmonth(date)` is the first day of the posting's month. With dates as labels, the chart is a line over time.

### The balance of an account on a date

```sql
SELECT account, sum(position) AS balance
WHERE account ~ '^Assets:Bank:Checking' AND date < 2024-04-01
GROUP BY account
```

| account | balance |
|---|---|
| Assets:Bank:Checking | 15313.20 CNY |

This adds up every posting before 1 April 2024, so it is the balance at the end of March. For the balances of many accounts at once, `BALANCES` with `CLOSE ON` gives the balance sheet at a date:

```sql
BALANCES FROM CLOSE ON 2024-04-01 WHERE account ~ '^(Assets|Liabilities)'
```

| account | sum(position) |
|---|---|
| Assets:Bank:Checking | 15313.20 CNY |
| Assets:Cash | 200 CNY |
| Liabilities:CreditCard | |

An account whose postings add up to nothing, like the credit card after its bill was paid, shows an empty amount.

### Everything bought from one payee

```sql
SELECT date, narration, account, position
WHERE payee = 'Supermarket' AND account ~ '^Expenses'
ORDER BY date
```

| date | narration | account | position |
|---|---|---|---|
| 2024-03-02 | Weekly shopping | Expenses:Food:Groceries | 386.50 CNY |
| 2024-03-16 | Weekly shopping | Expenses:Food:Groceries | 412.30 CNY |
| 2024-04-06 | Weekly shopping | Expenses:Food:Groceries | 295.00 CNY |

To total it per payee instead, group by payee: `SELECT payee, sum(position) WHERE account ~ '^Expenses:Food' GROUP BY payee`.

### An account's journal with its running balance

```sql
JOURNAL 'Liabilities:CreditCard' FROM year = 2024 AND month = 3
```

`JOURNAL` lists the postings of the accounts matching the pattern, with a running balance in the last column:

| date | payee | narration | position | balance |
|---|---|---|---|---|
| 2024-03-02 | Supermarket | Weekly shopping | -386.50 CNY | -386.50 CNY |
| 2024-03-05 | Noodle House | Lunch | -48 CNY | -434.50 CNY |
| 2024-03-09 | Hotel | Two nights in Hangzhou | -840 CNY | -1274.50 CNY |
| 2024-03-16 | Supermarket | Weekly shopping | -412.30 CNY | -1686.80 CNY |
| 2024-03-25 | Bank | Credit card bill | 1686.80 CNY | |

The table leaves out the `flag` and `account` columns of the result, and shortens the names of its payee and narration columns, `maxwidth(payee, 48)` and `maxwidth(narration, 80)`.

### Postings with a tag

```sql
SELECT date, payee, account, position
WHERE 'trip-hangzhou' IN tags
```

| date | payee | account | position |
|---|---|---|---|
| 2024-03-09 | Hotel | Liabilities:CreditCard | -840 CNY |
| 2024-03-09 | Hotel | Expenses:Travel | 840 CNY |

### Large expenses

```sql
SELECT date, payee, account, position
WHERE account ~ '^Expenses' AND number > 400
ORDER BY date
```

| date | payee | account | position |
|---|---|---|---|
| 2024-03-09 | Hotel | Expenses:Travel | 840 CNY |
| 2024-03-16 | Supermarket | Expenses:Food:Groceries | 412.30 CNY |

`number` is the number of the posting's amount, without its commodity.

### More

- [Balances and Padding](/guides/balances/#when-an-assertion-fails) finds failing balance assertions with the `#balances` table.
- [Lots and Cost Basis](/guides/lots-and-cost-basis/#see-your-lots) values holdings at cost and at market prices.
- The [Examples](/reference/query-language/#examples) of the reference include an income statement, a balance sheet and a portfolio valuation.

## Save a query

Save the queries you run often in the ledger, with the [`query`](/reference/directives/query/) directive:

```zhang
2024-03-31 query "food by payee" "SELECT payee, sum(position) AS spent WHERE account ~ '^Expenses:Food' GROUP BY payee ORDER BY payee"
```

It appears in the **Saved** menu of the Query page under its name. Choosing it puts the query in the editor and runs it.

## Export the result

**Export CSV** downloads the result of the query in the editor as `query.csv`. Each amount column becomes one numeric column per commodity, so the file opens cleanly in a spreadsheet:

```text
account,spent (CNY)
Expenses:Food:Groceries,798.80
Expenses:Food:Restaurants,48
Expenses:Transport,100
Expenses:Travel,840
```

Scripts can run queries over HTTP too, with `POST /api/query` and `POST /api/query/csv`. See [HTTP API](/reference/query-language/#http-api).
