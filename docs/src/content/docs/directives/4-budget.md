---
title: Budget
description: A comprehensive guide on using budget directives in Zhang Accounting.
---

# Budget Directives

Zhang Accounting's budget system is inspired by the YNAB (You Need A Budget) model, implementing a zero-based budgeting approach. This guide covers all budget-related directives and their usage.

## Basic Concepts

The budget system allows you to:
- Create budget accounts for different spending categories
- Allocate funds to these accounts
- Track spending against budgets
- Transfer funds between budget accounts

## Available Directives

### Creating a Budget Account

```zhang
{DATE} budget {BUDGET_NAME} {CURRENCY}
```

#### Meta Configurations

- `alias`: Provides a more descriptive name for display purposes
- `category`: Groups budget accounts on the frontend

```zhang
2023-01-01 budget Food CNY
  alias: "Monthly Food Budget"
  category: "Daily Expenses"
```

### Adding Funds to a Budget

```zhang
{DATE} budget-add {BUDGET_NAME} {AMOUNT} {CURRENCY}
```

Example:
```zhang
2023-01-01 budget-add Food 2000 CNY
```

### Transferring Budget Funds

```zhang
{DATE} budget-transfer {FROM_BUDGET_NAME} {TO_BUDGET_NAME} {AMOUNT} {CURRENCY}
```

Example:
```zhang
2023-01-15 budget-transfer Food Entertainment 200 CNY
```

### Closing a Budget Account

```zhang
{DATE} budget-close {BUDGET_NAME}
```

Example:
```zhang
2023-12-31 budget-close Vacation
```

### Linking Expense Accounts

Link an expense account to a budget for tracking:

```zhang
{DATE} open {ACCOUNT_NAME} {COMMODITY}
  budget: {BUDGET_NAME}
```

Example:
```zhang
2023-01-01 open Expenses:Food:Rent CNY
  budget: Food
```

## Common Use Cases

### Monthly Budget Setup

```zhang
; Create budget accounts
2023-01-01 budget Food CNY
  alias: "Monthly Food Budget"
  category: "Daily Expenses"

2023-01-01 budget Transportation CNY
  alias: "Monthly Transport Budget"
  category: "Daily Expenses"

2023-01-01 budget Entertainment CNY
  alias: "Monthly Entertainment Budget"
  category: "Discretionary"

; Allocate initial funds
2023-01-01 budget-add Food 2000 CNY
2023-01-01 budget-add Transportation 1000 CNY
2023-01-01 budget-add Entertainment 500 CNY

; Link expense accounts
2023-01-01 open Expenses:Food:Rent CNY
  budget: Food
2023-01-01 open Expenses:Transportation:Gas CNY
  budget: Transportation
2023-01-01 open Expenses:Entertainment:Movies CNY
  budget: Entertainment
```

### Budget Adjustments

```zhang
; Transfer funds between budgets
2023-01-15 budget-transfer Food Entertainment 200 CNY

; Add more funds to a budget
2023-01-20 budget-add Food 500 CNY
```

## Best Practices

1. **Budget Organization**
   - Use clear, descriptive names for budget accounts
   - Group related budgets using categories
   - Use aliases for better readability

2. **Fund Allocation**
   - Allocate funds at the start of each period
   - Keep track of transfers between budgets
   - Document any significant changes

3. **Account Linking**
   - Link all relevant expense accounts to budgets
   - Use consistent naming between budgets and accounts
   - Review linked accounts periodically

## Beancount Compatibility

Zhang Accounting is fully compatible with Beancount's budget directives. Here's the syntax comparison:

| Zhang | Beancount |
|-------|-----------|
| `{DATE} budget {NAME} {CURRENCY}` | `{DATE} custom budget {NAME} {CURRENCY}` |
| `{DATE} budget-add {NAME} {AMOUNT} {CURRENCY}` | `{DATE} custom budget-add {NAME} {AMOUNT} {CURRENCY}` |
| `{DATE} budget-transfer {FROM} {TO} {AMOUNT} {CURRENCY}` | `{DATE} custom budget-transfer {FROM} {TO} {AMOUNT} {CURRENCY}` |
| `{DATE} budget-close {NAME}` | `{DATE} custom budget-close {NAME}` |

## Frequently Asked Questions

### How do I adjust my budget mid-month?

Use the `budget-transfer` directive to reallocate funds between budget accounts:

```zhang
2023-01-15 budget-transfer Food Entertainment 200 CNY
```

### What happens if I overspend in a category?

Overspending will be tracked but won't affect your actual account balances. You can:
1. Transfer more funds to the budget
2. Adjust future allocations
3. Review spending patterns

### Can I have multiple currencies in my budget?

Each budget account is tied to a single currency. For multiple currencies:
1. Create separate budget accounts for each currency
2. Track them independently
3. Consider exchange rates when planning

### How do I close a budget account?

Use the `budget-close` directive:

```zhang
2023-12-31 budget-close Vacation
```

This prevents further transactions to that budget account.
