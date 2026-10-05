---
title: Budget
description: Reference for the budget, budget-add, budget-transfer and budget-close directives, and for linking accounts to a budget.
sidebar:
  order: 12
---

Zhang's budgets follow the envelope model of YNAB (You Need A Budget): you create a budget for a spending category,
assign money to it each month, and link expense accounts to it. Spending on those accounts is the budget's activity,
and what is left carries over to the next month. Budgets are separate from your accounts: they never change an
account's balance.

## Syntax

```text
YYYY-MM-DD budget <Name> <Commodity>
YYYY-MM-DD budget-add <Name> <Number> <Commodity>
YYYY-MM-DD budget-transfer <FromName> <ToName> <Number> <Commodity>
YYYY-MM-DD budget-close <Name>
```

| Directive | Effect |
|---|---|
| `budget` | Creates the budget `<Name>`, counted in `<Commodity>`. |
| `budget-add` | Assigns an amount to the budget in the month of its date. A negative amount takes money away. |
| `budget-transfer` | Moves an assigned amount from one budget to another in the month of its date. |
| `budget-close` | Closes the budget at the end of its date, or at its time if it has one. |

A budget name is a single word without spaces, quotes, colons, parentheses or commas, such as `Food` or
`Daily-Groceries`. Each directive takes a date, optionally with a time of day, and metadata lines below.

A `budget` directive reads two metadata keys:

| Key | Effect |
|---|---|
| `alias` | A display name for the budget in the web UI. |
| `category` | The group the budget page lists the budget under. |

### Linking accounts

An account counts toward a budget through the `budget` metadata of its `open` directive. Repeat the key to link the
account to several budgets. A posting counts toward the budgets of the account's `open` in effect at its date, so an
account closed and opened again with other `budget` metadata counts toward the new budgets from its reopening on.

```text
YYYY-MM-DD open <Account>
  budget: <Name>
```

## Examples

```zhang
2024-01-01 budget Food CNY
  alias: "Food and groceries"
  category: "Daily"
2024-01-01 budget Fun CNY
  category: "Discretionary"

2024-01-01 open Assets:Bank:Checking CNY
2024-01-01 open Expenses:Groceries CNY
  budget: Food
2024-01-01 open Expenses:Restaurants CNY
  budget: Food
2024-01-01 open Expenses:Movies CNY
  budget: Fun

2024-01-01 budget-add Food 2000 CNY
2024-01-01 budget-add Fun 300 CNY

2024-01-12 * "Supermarket" "weekly shopping"
  Assets:Bank:Checking -420.00 CNY
  Expenses:Groceries

2024-01-20 budget-transfer Fun Food 100 CNY
```

In January, `Food` has 2100 CNY assigned, 420 CNY of activity and 1680 CNY available; `Fun` has 200 CNY available.

## Behavior

Budgets work month by month. For each budget and month, Zhang keeps:

- **assigned**: what `budget-add` and `budget-transfer` put into the budget, plus what was available at the end of
  the previous month;
- **activity**: the postings to the linked accounts in that month. Spending on an expense account increases it, a
  refund decreases it;
- **available**: assigned minus activity. At the start of the next month, it becomes that month's assigned amount, so
  money left over, or overspent, carries over.

Some details:

- A budget exists from the date of its `budget` directive. A posting to a linked account before that date is not
  counted, and is reported once as [`BudgetDoesNotExist`](/reference/error-codes/#budgetdoesnotexist).
- The amounts of `budget-add` and `budget-transfer`, and the postings, are converted to the budget's commodity at
  their date, with the prices of the ledger, the latest on or before that date. An amount is never added as a number
  of another commodity: one that no price converts is left out, and reported as
  [`BudgetCommodityMismatch`](/reference/error-codes/#budgetcommoditymismatch).
- `budget-close` closes the budget. A `budget-close` with only a date leaves the budget open through that whole day,
  and one with a time, such as `2024-03-01 18:00:00 budget-close Food`, closes it at that time. A closed budget takes
  no activity: postings to its accounts after the close do not count toward it, and the first one of each account
  is reported as [`BudgetClosed`](/reference/error-codes/#budgetclosed). Its `budget-add` and `budget-transfer`
  directives still count, so you can move what is left to another budget with a `budget-transfer` after the close.
- The budget page shows a budget as closed from the month of its `budget-close` on, and open in the months before;
  the budget card of the home page leaves it out from that month. A later `budget-close` changes nothing.
- The budget page of the web UI shows the assigned, activity and available amounts of each budget for a month, and
  the events of a budget in a month. In queries, `#budgets` and `#budget_events` hold the same figures; see
  [Zhang-specific tables](/reference/query-language/#zhang-specific-tables).

## Errors

| Error | When |
|---|---|
| [`DefineDuplicatedBudget`](/reference/error-codes/#defineduplicatedbudget) | A `budget` directive names a budget that already exists. The second directive is ignored. |
| [`BudgetDoesNotExist`](/reference/error-codes/#budgetdoesnotexist) | A `budget-add`, `budget-transfer` or `budget-close` names a budget that is not defined at its date; the directive is ignored. Or a posting's account is linked to a budget that is not defined at the posting's date; this is reported once per account and budget, and the transaction is still booked. |
| [`BudgetCommodityMismatch`](/reference/error-codes/#budgetcommoditymismatch) | The amount of a `budget-add` or `budget-transfer`, or a posting to a linked account, is in another commodity than the budget's and no price on or before its date converts it. The amount does not count toward the budget, and the transaction is still booked. |
| [`BudgetClosed`](/reference/error-codes/#budgetclosed) | A posting to a linked account comes after the budget's `budget-close`. It does not count toward the budget; this is reported once per account and budget, and the transaction is still booked. |

## Beancount compatibility

Beancount has no budget directives. In a Beancount file, write them as `custom` directives in the form Beancount
itself accepts: a quoted type, quoted budget names, a quoted commodity for `budget`, and a plain amount for
`budget-add` and `budget-transfer`:

| Zhang | Beancount file |
|---|---|
| `2024-01-01 budget Food CNY` | `2024-01-01 custom "budget" "Food" "CNY"` |
| `2024-01-01 budget-add Food 2000 CNY` | `2024-01-01 custom "budget-add" "Food" 2000 CNY` |
| `2024-01-20 budget-transfer Fun Food 100 CNY` | `2024-01-20 custom "budget-transfer" "Fun" "Food" 100 CNY` |
| `2024-12-31 budget-close Food` | `2024-12-31 custom "budget-close" "Food"` |

Metadata lines follow as usual. `bean-check` and Fava accept these lines as ordinary `custom` entries; Zhang reads
them as budgets and writes budgets this way into Beancount files. The unquoted form earlier versions of Zhang wrote,
`custom budget Food CNY`, is still read, so existing ledgers keep working, but Beancount rejects it (`Invalid token`):
switch to the quoted form when you also check the ledger with `bean-check`. A `custom "budget"` with other values,
such as Fava's own budget entries (`custom "budget" Expenses:Coffee "daily" 4.00 EUR`), stays an ordinary
[`custom`](/reference/directives/custom/) directive.

## Related

- [Budgets](/guides/budgets/): setting up and using budgets.
- [Account](/reference/directives/account/#metadata): the `budget` metadata of an account.
