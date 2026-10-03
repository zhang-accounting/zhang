---
title: Budgets
description: Set up a zero-based budget, assign money to categories and follow your spending in the web UI.
sidebar:
  order: 4
---

Zhang's budgets are monthly envelopes, in the style of zero-based budgeting: each month you assign money to a budget, the spending in the expense accounts linked to it is its **activity**, and what is left is **available**. Whatever is left at the end of a month, or overspent, carries over to the next month.

Budgets live beside your accounts: they never change a balance. The full syntax is in [Budget](/reference/directives/budget/).

## Create budgets and link accounts

```zhang
2024-01-01 open Expenses:Groceries CNY
  budget: Food
2024-01-01 open Expenses:Restaurants CNY
  budget: Food
2024-01-01 open Expenses:Fun CNY
  budget: Fun

2024-03-01 budget Food CNY
  alias: "Food and dining"
  category: "Daily"
2024-03-01 budget Fun CNY
  category: "Discretionary"
```

- `budget Food CNY` creates the budget `Food` in CNY. `alias` is the name the web UI shows, and `category` groups budgets on the **Budget** page.
- The `budget` metadata of an `open` links the account to a budget. Several accounts can share a budget, and an account can name several budgets by repeating the key.

## Assign and move money

```zhang
2024-03-01 budget-add Food 2000 CNY
2024-03-01 budget-add Fun 500 CNY

2024-03-05 * "Supermarket"
  Assets:Bank:Checking -420 CNY
  Expenses:Groceries
2024-03-12 * "Hotpot"
  Assets:Bank:Checking -1800 CNY
  Expenses:Restaurants

; the hotpot was expensive: move money from Fun to Food
2024-03-20 budget-transfer Fun Food 300 CNY

2024-04-01 budget-add Food 2000 CNY
2024-04-03 * "Supermarket"
  Assets:Bank:Checking -380 CNY
  Expenses:Groceries
```

- `budget-add` assigns money to a budget in the month of its date.
- `budget-transfer Fun Food 300 CNY` moves 300 CNY of assigned money from `Fun` to `Food` in that month.

This gives:

| Month | Budget | Assigned | Activity | Available |
|---|---|---|---|---|
| March | Food | 2,300 | 2,220 | 80 |
| March | Fun | 200 | 0 | 200 |
| April | Food | 2,080 | 380 | 1,700 |
| April | Fun | 200 | 0 | 200 |

In April, `Food` starts with the 80 CNY left from March, plus the 2,000 CNY assigned. An overspent budget carries its negative amount over in the same way.

## How spending counts

- Every posting to a linked account adds to the activity of the budget in the month of its transaction. A refund, a negative posting, takes activity away.
- A posting counts only from the date of the `budget` directive on. A posting to a linked account before the budget exists is skipped, and reported once per account as [`BudgetDoesNotExist`](/reference/error-codes/#budgetdoesnotexist).
- The amount is added as it is, without converting it. Keep the accounts of a budget in the budget's commodity.

## Close a budget

```zhang
2024-12-31 budget-close Fun
```

`budget-close` marks the budget closed, and the **Budget** page shows it as **Closed**, in every month. Accounts that name it still add their spending to it, so remove their `budget` metadata too.

## Follow your budgets in the web UI

- The **Budget** page shows one month at a time, with the **Assigned**, **Activity** and **Available** amounts of each budget, grouped by category (budgets without one are **Uncategorized**). Use the arrows to change month. **Hide budgets with nothing assigned** hides the budgets with no money assigned that month.
- Select a budget to see its linked accounts and its activity in the month: the money assigned and transferred, and the postings to its accounts.
- The **Overview** page shows this month's budgets and how much of each is used.

The budgets are also available to [queries](/guides/querying/), in the `#budgets` table. See [Budgets](/reference/query-language/#budgets) in the query language reference.

In a beancount ledger, budgets are written as `custom` directives. See [Coming from Beancount](/getting-started/from-beancount/#compatibility).
