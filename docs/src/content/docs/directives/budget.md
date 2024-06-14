---
title: Budget
description: This is a page in my Starlight-powered site
---

Zhang's budget system is built based on the YNAB model, which is essentially zero-based budgeting. The budget system
introduces 4 instructions and 1 account-related configuration for Zhang.

The budget system and account will not participate in the core system's balance calculation logic. For example,
allocating a portion of the amount to the budget account will not reduce the balance of a certain asset account.

## Instruction Set

### Create a Budget Account

```zhang
{DATE} budget {BUDGET_NAME} {CURRENCY}
```

#### Meta Configuration

- `alias`: (Optional) Provides a more semantic description for display on the page, as BUDGET_NAME can only be in
  English and underscores.
- `category`: (Optional) Used to group budget accounts on the frontend page.

### Add Amount to Budget Account

```zhang
{DATE} budget-add {BUDGET_NAME} {AMOUNT} {CURRENCY}
```

### Budget Transfer

```zhang
{DATE} budget-transfer {FROM_BUDGET_NAME} {TO_BUDGET_NAME} {AMOUNT} {CURRENCY}
```

The budget system has a scenario where we need to adjust the budget amount according to actual situations, and we need
to transfer part of the amount from one budget account to another.

This requires the use of the budget transfer instruction.

For example:

```zhang
// In December 2023, I want to save some money from my food budget to buy a gift for my girlfriend
2023-12-01 budget-transfer Diet GirlFriendGift 200 CNY
```

### Close Budget Account

```zhang
{DATE} budget-close {BUDGET_NAME}
```

### Bind Consumption Account

```zhang
{DATE} open {ACCOUNT_NAME} {COMMODITY}
budget: {BUDGET_NAME}
```

To enable the budget system to correctly calculate the **consumed amount** and **available amount**, we need to bind the
consumption account to the budget account.

This can be done by using the `budget` meta when setting up the consumption
account.

For example:

```zhang
// Bind the lunch account to the Diet budget
1970-01-01 open Expenses:Lunch CNY
budget: Diet
```

## Beancount Compatibility

To ensure that Beancount users can also use the budget system, we have made the instructions compatible with Beancount.
The specific syntax can be referenced as follows:

- Create a Budget Account
    - Zhang: `{DATE} budget {BUDGET_NAME} {CURRENCY}`
    - Beancount: `{DATE} custom budget {BUDGET_NAME} {CURRENCY}`
- Add Amount to Budget Account
    - Zhang: `{DATE} budget-add {BUDGET_NAME} {AMOUNT} {CURRENCY}`
    - Beancount: `{DATE} custom budget-add {BUDGET_NAME} {AMOUNT} {CURRENCY}`
- Budget Transfer
    - Zhang: `{DATE} budget-transfer {FROM_BUDGET_NAME} {TO_BUDGET_NAME} {AMOUNT} {CURRENCY}`
    - Beancount: `{DATE} custom budget-transfer {FROM_BUDGET_NAME} {TO_BUDGET_NAME} {AMOUNT} {CURRENCY}`
- Close Budget Account
    - Zhang: `{DATE} budget-close {BUDGET_NAME}`
    - Beancount: `{DATE} custom budget-close {BUDGET_NAME}`