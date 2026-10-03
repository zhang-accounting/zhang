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
It decides which lot a reduction (for example `-5 AAPL {}`) takes its units from. Without the meta, the account uses
the `default_booking_method` option (`FIFO` unless set).

Like Beancount, a reduction matches the lots its cost names, and what the cost leaves out matches anything:
`{100 USD}` matches the lots held at 100 USD whatever their acquisition date, `{100 USD, 2024-01-01}` only the lot
acquired on that date, and `{}` every lot held at cost. A lot's acquisition date is the date in its cost, or else the
date of the transaction that opened it.

Supported methods:
- `STRICT`: Beancount's default. A reduction must match a single lot, or reduce every lot it matches in full.
  Otherwise the ledger reports an [`AmbiguousLotMatch`](/user-guide/error-code/#ambiguouslotmatch) error and books the
  reduction like `FIFO` among the matching lots.
- `FIFO`: First In First Out: the matching lot with the oldest acquisition date first.
- `LIFO`: Last In First Out: the matching lot with the newest acquisition date first.

`AVERAGE`, `AVERAGE_ONLY` and `NONE` are not implemented yet. An account using one of them, or a value that is not a
booking method, gets an error on its `open` directive
([`UnsupportedBookingMethod`](/user-guide/error-code/#unsupportedbookingmethod) or
[`ParseInvalidMeta`](/user-guide/error-code/#parseinvalidmeta)) and books with the default booking method. The ledger
still loads.

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

## Balance Assertions and Pads

### Balance Assertions

A `balance` directive asserts what an account holds in one commodity:

```zhang
2024-01-31 balance Assets:Bank:Checking 1520.00 CNY
2024-01-31 balance Assets:Bank:Checking 1520.00 ~ 0.01 CNY
```

- The assertion is checked at the start of its date (at its time, if it has one), against the sum of the account's
  postings before it. Transactions of the same day come after it, as in Beancount.
- It checks the postings of the account itself, not those of its sub-accounts.
- It must match exactly, unless it gives a tolerance with `~`: `1520.00 ~ 0.01 CNY` passes for any balance from
  1519.99 to 1520.01.
- An assertion only checks. Passing or failing, it changes no balance: an account always holds the sum of its
  postings, and every balance, report and journal shows that sum. A failing assertion is reported as an
  [`AccountBalanceCheckError`](/user-guide/error-code/#accountbalancecheckerror).
- The journal lists every assertion with the asserted amount, the balance it was checked against, and whether it
  passed.

### Pads

To correct a balance on purpose, add `with pad` and the account to pad from:

```zhang
2024-01-01 balance Assets:Bank:Checking 1000.00 CNY with pad Equity:Opening-Balances
```

Zhang adds a padding transaction (flag `P`) that moves the difference between the asserted amount and the
account's balance there from the pad account, so the assertion holds. The difference is measured from the sum of
the postings: an earlier assertion, even a failing one, does not count. An account already at the asserted amount
gets no padding transaction.

In a Beancount ledger, a `pad` directive serves the next `balance` of its account in each commodity, as in
Beancount. A `balance` on the day of the `pad` comes before it and is not padded. The padding transaction is dated
on the `balance` it serves, where Beancount dates it on the `pad`, and the `pad` and its `balance` must be in the
same file.

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
  booking_method: "STRICT"
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
