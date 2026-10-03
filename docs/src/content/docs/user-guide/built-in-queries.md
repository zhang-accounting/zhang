---
title: Built-in queries
description: The named queries behind the pages of Zhang's web UI, with their BQL, so every figure the app shows can also be queried, and varied, on the Explore page.
---

The pages of the web UI compute their figures with named queries in Zhang's [query language](/user-guide/query-language/). This page lists them with their BQL, so that anything the app shows can also be queried on the Explore page, and changed to answer a related question.

- A query reads the values of the page that runs it as parameters: `:account` is the account of an account page, and `:operating_currency` the ledger's `operating_currency` option. To run a query yourself, replace each parameter with a value, such as `'Assets:Bank'` or `'CNY'`.
- `today()` is the current date in the ledger's timezone.

## Accounts

An account page shows the account **and its sub-accounts**, as the account tree does: its journal, its balance history, its total and its documents cover the whole subtree. The page of `Assets:Bank` includes the postings of `Assets:Bank:Checking`, and its journal names the account of each posting. Balances are valued in the operating currency at today's prices with [`convert`](/user-guide/query-language/#valuation-functions), which uses inverse prices and the cost currency of a holding.

### accounts

Every account with an `open` or `close` directive, with its open and close dates and its alias, by name. With `account_balances`, it makes the account list: an account with postings but no `open` directive is listed too.

```sql
SELECT account, open, close, meta('alias') AS alias
FROM #accounts
ORDER BY account
```

### account_balances

The balance of every account that has postings, of its own postings, per currency: the units, their value in the operating currency at today's prices, and the date of the first posting. The account list adds up the rows of an account and of the accounts under it for the balance with sub-accounts.

```sql
SELECT account, currency, sum(number) AS units,
       convert(sum(position), :operating_currency, today()) AS value,
       min(date) AS first_date
GROUP BY account, currency
ORDER BY account, currency
```

### account_subtree

An account and its sub-accounts that have an `open` or `close` directive. The account page reads its dates, status and alias from it.

```sql
SELECT account, open, close, meta('alias') AS alias
FROM #accounts
WHERE under(account, :account)
ORDER BY account
```

### account_subtree_balances

The balance of an account and of each of its sub-accounts, as in `account_balances`. The account page shows the total of its rows, the balance that a `balance` assertion on the account is checked against, and the balance of the account alone.

```sql
SELECT account, currency, sum(number) AS units,
       convert(sum(position), :operating_currency, today()) AS value,
       min(date) AS first_date
WHERE under(account, :account)
GROUP BY account, currency
ORDER BY account, currency
```

### account_journal

The journal of an account page: the postings of the account and its sub-accounts, newest first, one row per posting, each with the [running balance](/user-guide/query-language/#the-running-balance) of the account and its sub-accounts in the posting's currency right after it. The padding transactions of `balance ... with pad` are listed like the others.

```sql
SELECT date, time, timestamp, flag, id, account, payee, narration, currency,
       sum(number) AS units,
       last(only(currency, units(balance))) AS balance
WHERE under(account, :account)
GROUP BY seq, posting_index, date, time, timestamp, flag, id, account, payee, narration, currency
ORDER BY seq DESC, posting_index DESC
```

### account_balance_assertions

The balance assertions on an account, listed in its journal. `actual` is the balance of the account and its sub-accounts that the assertion was checked against: the journal shows each assertion where the running balance is that balance, at the start of its day, after the paddings it includes.

```sql
SELECT date, time, timestamp, id, account, amount, actual, passed, pad
FROM #balances
WHERE account = :account
ORDER BY seq DESC
```

### account_balance_history

The balance history chart of an account page: the balance of the account and its sub-accounts at the end of every day with a posting, per currency, in date order.

```sql
SELECT date, currency, last(only(currency, units(balance))) AS balance
WHERE under(account, :account)
GROUP BY date, currency
ORDER BY date, currency
```

### account_documents

The documents of an account page: the `document` directives of the account and its sub-accounts, in ledger order. `path` is the path of the file relative to the ledger's directory, which the page downloads it with.

```sql
SELECT date, time, account, path
FROM #documents
WHERE source = 'directive' AND under(account, :account)
```
