---
title: Introduction
description: What Zhang Accounting is, the core ideas of plain text accounting it builds on, and how this documentation is organized.
sidebar:
  order: 1
---

Zhang (账, "ledger" in Chinese) is a plain text, double-entry accounting tool. You keep your books in text files that you own, and Zhang serves a web UI on top of them. It reads [beancount](https://beancount.github.io/) ledgers directly, so you can point it at an existing beancount ledger without converting anything.

Zhang is for people who want to track their personal or household finances precisely, keep the data in a format they can read, diff and back up, and still have dashboards, reports and a form to record a purchase from the browser. You run it yourself, on your computer or on a server.

## Try the online demo

Open the [online demo](https://zhang-demo.onrender.com/) to explore the web UI before installing Zhang. It uses a
fictional, read-only ledger, so changes to the shared ledger and attachment uploads cannot be saved.

- Start with [Overview](https://zhang-demo.onrender.com/) and [Journals](https://zhang-demo.onrender.com/journals)
  to explore income, expenses, multi-currency transactions, tags and links.
- Open [Commodities](https://zhang-demo.onrender.com/commodities) to inspect investment lots and their costs,
  then [Budget](https://zhang-demo.onrender.com/budgets) to see monthly allocations, spending and carry-over.
- Browse [Documents](https://zhang-demo.onrender.com/documents) for sample receipts and statements. In
  [Query](https://zhang-demo.onrender.com/explore), choose **Saved**, run a saved query and export its result as CSV.

To record your own transactions, continue with [installation](/getting-started/installation/) and
[your first ledger](/getting-started/first-ledger/).

## Core concepts

**Ledger files.** A ledger is one or more text files made of *directives*: entries that start with a date and say what happened, such as opening an account or recording a transaction. Zhang starts from a main file (`main.zhang` by default) and follows its [`include`](/reference/directives/include/) directives into other files. The order of the directives does not matter, Zhang sorts them by date. Files ending in `.bean`, `.beancount` or `.bc` are read as beancount. The others use the Zhang format, which is close to beancount's and adds a few directives, such as [budgets](/reference/directives/budget/).

**Accounts.** Amounts live in accounts with hierarchical names such as `Assets:Bank:Checking`. The first part of the name is one of the five account types:

- `Assets`: what you own, such as cash, bank accounts and investments.
- `Liabilities`: what you owe, such as credit cards and loans.
- `Equity`: where your starting balances come from.
- `Income`: where money comes from, such as a salary or interest.
- `Expenses`: where money goes, such as food or rent.

An account must be [opened](/reference/directives/account/) before it is used, and can be closed later.

**Commodities.** Every amount has a commodity: a currency such as `USD`, or anything else you count, such as shares or air miles. You declare them with the [`commodity`](/reference/directives/commodity/) directive and record their value over time with [`price`](/reference/directives/price/).

**Transactions and postings.** A [transaction](/reference/directives/transaction/) moves amounts between accounts. Each account line of a transaction is a *posting*. The postings of a transaction must sum to zero, which is what double-entry means: money always comes from somewhere and goes somewhere. One posting may leave its amount out, and Zhang fills it in.

**Assertions.** A [`balance`](/reference/directives/balance/) directive states what an account holds on a date, for example the balance printed on a bank statement. Zhang checks it and reports an error when the books disagree. An assertion is exact unless you give it an explicit tolerance.

**The web UI reads the files.** Zhang loads the ledger into memory and serves the web UI from it. What you record in the UI is written back to the files as ordinary directives, and when you edit a file yourself, Zhang reloads the ledger (see [Local File System](/deployment/data-sources/local/) for when this happens). Problems in the books, such as an unbalanced transaction or a failed assertion, appear in the error list of the web UI.

## How these docs are organized

- [Getting Started](/getting-started/installation/): install Zhang, write [your first ledger](/getting-started/first-ledger/), or [bring your beancount ledger](/getting-started/from-beancount/).
- [Guides](/guides/recording-transactions/): everyday tasks, such as recording transactions, checking balances, tracking investments, budgets, documents, queries and plugins.
- [Deployment](/deployment/data-sources/local/): where the ledger is stored (local disk, S3, WebDAV or GitHub), signing in and upgrading.
- [Reference](/reference/directives/options/): every directive and option, the [query language](/reference/query-language/) and the [error codes](/reference/error-codes/).
- [Developers](/developers/writing-plugins/): writing plugins, and finding your way around the source code.
