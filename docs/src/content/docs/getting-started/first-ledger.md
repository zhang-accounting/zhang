---
title: Your First Ledger
description: Write a minimal ledger, start Zhang on it, tour the web UI and record a transaction from the browser.
sidebar:
  order: 3
---

This page walks through a small ledger: a bank account with an opening balance, a salary, some spending and a balance check. You need Zhang installed, see [Installation](/getting-started/installation/).

## Write the ledger

Create a folder, for example `~/ledger`, and a file named `main.zhang` in it:

```zhang
option "title" "My Ledger"
option "operating_currency" "USD"

1970-01-01 commodity USD
  precision: 2

2024-01-01 open Assets:Bank:Checking USD
2024-01-01 open Liabilities:CreditCard USD
2024-01-01 open Income:Salary USD
2024-01-01 open Expenses:Food USD
2024-01-01 open Expenses:Rent USD
2024-01-01 open Equity:Opening-Balances USD

2024-01-01 * "Opening balance"
  Assets:Bank:Checking     2500.00 USD
  Equity:Opening-Balances

2024-01-05 * "ACME Corp" "January salary"
  Assets:Bank:Checking     3000.00 USD
  Income:Salary

2024-01-06 * "Landlord" "January rent"
  Assets:Bank:Checking    -1200.00 USD
  Expenses:Rent

2024-01-08 * "Corner Cafe" "Lunch"
  Liabilities:CreditCard    -12.50 USD
  Expenses:Food

2024-01-31 balance Assets:Bank:Checking 4300.00 USD
```

What each part does:

- The [options](/reference/directives/options/) name the ledger, shown in the web UI, and set the currency used for totals and reports.
- The [`commodity`](/reference/directives/commodity/) directive declares `USD`, shown with two decimals.
- The [`open`](/reference/directives/account/) directives create the accounts. The `USD` after each name, which is optional, notes the commodity the account holds.
- The first transaction brings the money that was already in the bank into the books. It comes from `Equity:Opening-Balances`, the usual place for starting balances.
- The other [transactions](/reference/directives/transaction/) record a salary, the rent and a lunch paid by credit card. Every transaction here leaves the amount of its last posting out: Zhang fills it in so that the postings sum to zero.
- The [`balance`](/reference/directives/balance/) directive asserts that the bank account holds exactly 4300.00 USD on January 31: 2500 + 3000 − 1200.

## Start Zhang

```shell
zhang serve ~/ledger
```

or with Docker:

```shell
docker run --name zhang -v "$HOME/ledger:/data" -p "8000:8000" kilerd/zhang:latest
```

Open `http://localhost:8000` in a browser. If the command stops right away, the ledger could not be loaded, for example because of a typo in the syntax: run it again with `RUST_LOG=info zhang serve ~/ledger` to see the file and the line.

## A tour of the web UI

The web UI has these pages:

- **Overview**: a summary of the last 30 days, the net worth and cash flow charts, and the health of the ledger (the errors found while loading it).
- **Journals**: the transactions and balance checks of the ledger, newest first, with a search and filters by tag and link. Select an entry to see its postings, metadata and documents, or to edit a transaction.
- **Report**: income, expenses and net worth over a period you choose, with the income and expenses broken down by account.
- **Balance sheet** (**Accounts** on small screens): every account with its balance. Open an account to see its postings, documents and balance history, with those of its sub-accounts, and to record a balance check.
- **Budget**: your [budgets](/guides/budgets/) month by month: what you assigned, what you spent and what is left.
- **Commodities**: the currencies and assets of the ledger, with holdings, lots and price history.
- **Documents**: the receipts and statements attached to accounts and transactions. You can upload new ones, see [Documents](/guides/documents/).
- **Raw Editing**: edit the ledger files in the browser. Saving a file reloads the ledger.
- **Query**: run queries in a [BQL-compatible language](/reference/query-language/), see [Querying](/guides/querying/).
- **Tools**: utilities, such as checking or padding the balances of many accounts at once.
- **Settings**: language and theme, the ledger's title, operating currency and options, the Zhang version, the loaded plugins, a link to the API documentation, and your passkeys when [passkey sign-in](/deployment/authentication/#passkeys) is enabled.

The navigation also has a **New transaction** button, a button to reload the ledger, and the list of accounts.

## Record a transaction in the web UI

Choose **New transaction** and fill in the form: the date (today by default), a payee such as `Corner Cafe`, a narration such as `Coffee`, and the postings. For a coffee paid by credit card, enter `Liabilities:CreditCard` with the amount `-4.50 USD`, and `Expenses:Food` with no amount. Choose **Create**.

Zhang writes the transaction to a file named after its month, `data/2024/02.zhang` for a date in February 2024, and adds an `include` for that file to `main.zhang` the first time:

```zhang
2024-02-03 12:30:00 * "Corner Cafe" "Coffee"
  Liabilities:CreditCard -4.50 USD
  Expenses:Food
```

The date is written with the time of day, in the ledger's timezone. The path of the new file comes from the [`directive_output_path`](/reference/directives/options/) option, see [Recording Transactions](/guides/recording-transactions/). The ledger reloads and the transaction appears in **Journals**.

## Where errors appear

Change the balance assertion to `4200.00 USD` and save the file. Zhang reloads the ledger (if it does not, use the reload button and see [when files are reloaded](/deployment/data-sources/local/#when-zhang-reloads)), and an error appears: the error count is shown next to **Overview** in the navigation, and the Overview page lists the errors. Select one to see the file it comes from and the source of the entry. Here it is an [`AccountBalanceCheckError`](/reference/error-codes/#accountbalancecheckerror): the account holds 4300.00 USD, not 4200.00 USD. [Error Codes](/reference/error-codes/) explains each error and how to fix it.

These errors do not stop Zhang: the rest of the ledger is still shown. A syntax error is different, because Zhang cannot read the file at all:

- At startup, `zhang serve` exits with an error.
- While the server is running, the reload fails and the web UI keeps showing the ledger as it was before the change, without an error in the list. The reason is only written to the log (`docker logs zhang` with Docker). Fix the file and save it again.

## Next steps

- [Recording Transactions](/guides/recording-transactions/) covers tags, links, metadata and where new entries are written.
- [Balances](/guides/balances/) explains balance assertions and padding.
- [Local File System](/deployment/data-sources/local/) explains when Zhang reloads the files you edit.
