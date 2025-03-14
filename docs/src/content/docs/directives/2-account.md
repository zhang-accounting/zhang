---
title: Account
description: Comprehensive guide on utilizing account directives within Zhang Accounting.
---

# Account Directives

Account directives are fundamental in Zhang Accounting, allowing users to define and manage various accounts for their financial transactions. This guide covers the syntax, meta configurations, and provides practical examples.

## Basic Syntax

To define an account, use the following syntax:

```zhang
{DATE} open {ACCOUNT_NAME} {COMMODITY1} {COMMODITY2}
```

## Meta Configurations

### Alias

Assigns a more descriptive name for display purposes.

```zhang
2023-01-01 open Assets:Card CNY
  alias: "Credit Card"
```

### Booking Method

Specifies the method used for handling complex investment scenarios. This is particularly important for accounts dealing with investments or trading.

Available options:
- `STRICT`: Strict matching of lots
- `FIFO`: First In First Out
- `LIFO`: Last In First Out
- `AVERAGE`: Average cost basis
- `AVERAGE_ONLY`: Only allow average cost basis
- `NONE`: No specific booking method

```zhang
2023-01-01 open Investments:Stocks USD
  booking_method: "FIFO"
```

## Account Types

### Asset Accounts

For tracking money and valuable items you own.

```zhang
2023-01-01 open Assets:Bank:Checking USD
2023-01-01 open Assets:Cash CNY
2023-01-01 open Assets:Card CNY
  alias: "Credit Card"
```

### Liability Accounts

For tracking money you owe.

```zhang
2023-01-01 open Liabilities:CreditCard USD
  alias: "Main Credit Card"
2023-01-01 open Liabilities:Loans:Car CNY
```

### Equity Accounts

For tracking your net worth and capital.

```zhang
2023-01-01 open Equity:Opening-Balances
2023-01-01 open Equity:Retained-Earnings
```

### Income Accounts

For tracking money you receive.

```zhang
2023-01-01 open Income:Salary USD
2023-01-01 open Income:Investments:Dividends USD
```

### Expense Accounts

For tracking money you spend.

```zhang
2023-01-01 open Expenses:Food:Rent CNY
2023-01-01 open Expenses:Transportation:Gas USD
```

## Best Practices

1. **Account Hierarchy**
   - Use colons (`:`) to create hierarchical account structures
   - Group related accounts together (e.g., `Assets:Bank:Checking`, `Assets:Bank:Savings`)

2. **Naming Conventions**
   - Use clear, descriptive names
   - Avoid special characters except colons and hyphens
   - Use consistent capitalization

3. **Meta Data**
   - Use aliases for better readability in reports
   - Set appropriate booking methods for investment accounts
   - Document any special considerations in comments

## Beancount Compatibility

Zhang Accounting is fully compatible with Beancount's account directives. The syntax is identical:

```beancount
1970-01-01 open Assets:Card CNY "NONE"
```

## Examples

### Complete Account Setup

```zhang
; Asset Accounts
2023-01-01 open Assets:Bank:Checking USD
  alias: "Main Checking"
2023-01-01 open Assets:Bank:Savings USD
  alias: "Emergency Fund"
2023-01-01 open Assets:Cash CNY
  alias: "Wallet"

; Investment Accounts
2023-01-01 open Investments:Stocks USD
  booking_method: "FIFO"
  alias: "Stock Portfolio"
2023-01-01 open Investments:Bonds USD
  booking_method: "AVERAGE"
  alias: "Bond Portfolio"

; Liability Accounts
2023-01-01 open Liabilities:CreditCard USD
  alias: "Main Credit Card"
2023-01-01 open Liabilities:Loans:Car CNY
  alias: "Car Loan"

; Income Accounts
2023-01-01 open Income:Salary USD
  alias: "Monthly Salary"
2023-01-01 open Income:Investments:Dividends USD
  alias: "Investment Income"

; Expense Accounts
2023-01-01 open Expenses:Food:Rent CNY
  alias: "Monthly Rent"
2023-01-01 open Expenses:Transportation:Gas USD
  alias: "Gasoline"
```
