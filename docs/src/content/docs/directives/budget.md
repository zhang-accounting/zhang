---
title: Budget Directives Guide
description: Comprehensive guide on leveraging budget directives within Zhang Accounting, including examples and FAQs.
---

## Introduction to Budget Directives

Zhang Accounting's budget system is inspired by the YNAB (You Need A Budget) model, which is fundamentally a zero-based budgeting approach. This system introduces several directives and configurations related to budget accounts, which do not impact the core system's balance calculations. For instance, allocating funds to a budget account does not affect the balance of any asset account.

### Key Directives

#### Creating a Budget Account

To establish a budget account, use the following directive:

```zhang
{DATE} budget {BUDGET_NAME} {CURRENCY}
```

##### Meta Configuration Options

- `alias`: (Optional) Provides a more descriptive name for display purposes, as BUDGET_NAME is limited to English and underscores.
- `category`: (Optional) Facilitates grouping of budget accounts on the frontend for better organization.

#### Adding Funds to a Budget Account

Increase the budget by adding funds:

```zhang
{DATE} budget-add {BUDGET_NAME} {AMOUNT} {CURRENCY}
```

#### Transferring Budget Funds

To reallocate funds between budget accounts:

```zhang
{DATE} budget-transfer {FROM_BUDGET_NAME} {TO_BUDGET_NAME} {AMOUNT} {CURRENCY}
```

Example scenario:

```zhang
// To save for a gift, reallocating funds from the food budget in December 2023
2023-12-01 budget-transfer Diet GirlFriendGift 200 CNY
```

#### Closing a Budget Account

When a budget account is no longer needed:

```zhang
{DATE} budget-close {BUDGET_NAME}
```

#### Linking Expense Accounts to Budgets

Linking an expense account to a budget allows for tracking against the budget:

```zhang
{DATE} open {ACCOUNT_NAME} {COMMODITY}
budget: {BUDGET_NAME}
```

Example:

```zhang
// Linking the lunch expense account to the Diet budget
1970-01-01 open Expenses:Lunch CNY
budget: Diet
```

### Beancount Compatibility

For Beancount users, the budget system directives are compatible and can be used as follows:

- Creating a Budget Account
    - Zhang: `{DATE} budget {BUDGET_NAME} {CURRENCY}`
    - Beancount: `{DATE} custom budget {BUDGET_NAME} {CURRENCY}`
- Adding Funds to Budget Account
    - Zhang: `{DATE} budget-add {BUDGET_NAME} {AMOUNT} {CURRENCY}`
    - Beancount: `{DATE} custom budget-add {BUDGET_NAME} {AMOUNT} {CURRENCY}`
- Transferring Budget Funds
    - Zhang: `{DATE} budget-transfer {FROM_BUDGET_NAME} {TO_BUDGET_NAME} {AMOUNT} {CURRENCY}`
    - Beancount: `{DATE} custom budget-transfer {FROM_BUDGET_NAME} {TO_BUDGET_NAME} {AMOUNT} {CURRENCY}`
- Closing a Budget Account
    - Zhang: `{DATE} budget-close {BUDGET_NAME}`
    - Beancount: `{DATE} custom budget-close {BUDGET_NAME}`

## Frequently Asked Questions (FAQs)

### How do I adjust my budget mid-month?

You can use the `budget-transfer` directive to reallocate funds between budget accounts as needed.

### What happens if I overspend in a category?

Overspending in a category will not automatically affect your account balances. However, it's recommended to adjust your budget to reflect actual spending and plan accordingly.

### Can I have multiple currencies in my budget?

Each budget account is tied to a single currency. To manage budgets in multiple currencies, create separate budget accounts for each currency.

### How do I close a budget account?

Use the `budget-close` directive with the name of the budget account you wish to close. This will prevent any further transactions from being allocated to this budget.

For more detailed examples and advanced configurations, refer to the official documentation and Zhang Accounting's community forums.
