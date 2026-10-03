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
- It covers the account and all its sub-accounts, as in Beancount: `balance Assets:Bank 100 CNY` passes when
  `Assets:Bank:Checking` holds 60 CNY and `Assets:Bank:Savings` holds 40 CNY.
- **It must match exactly.** `1520.00 CNY` passes only when the balance is exactly 1520 CNY. Only an explicit
  tolerance written with `~` allows a difference: `1520.00 ~ 0.01 CNY` passes for any balance from 1519.99 to
  1520.01. Zhang never infers a tolerance, from the decimals of the amount or from an option.
- An assertion only checks. Passing or failing, it changes no balance: an account always holds the sum of its
  postings, and every balance, report and journal shows that sum. A failing assertion is reported as an
  [`AccountBalanceCheckError`](/user-guide/error-code/#accountbalancecheckerror).
- The journal lists every assertion with the asserted amount, the balance it was checked against, and whether it
  passed.

:::caution[Difference from Beancount]
Beancount infers a tolerance for an assertion without `~` from the decimals of its amount: `balance ... 1520.00 CNY`
passes there for any balance from 1519.99 to 1520.01 (one unit of the last decimal place, scaled by the
`tolerance_multiplier` option). Zhang does not, so a Beancount ledger that passes `bean-check` only thanks to that
inferred tolerance reports the assertion as failed in Zhang. To fix it, write the exact amount the account holds, or
give the tolerance you accept explicitly with `~`.
:::

### Pads

To correct a balance on purpose, pad the account from another one. A `pad` directive, as in Beancount, pads the
account for its next balance assertions; a `balance ... with pad` pads its own assertion:

```zhang
2024-01-01 pad Assets:Bank:Checking Equity:Opening-Balances
2024-02-01 balance Assets:Bank:Checking 1000.00 CNY

2024-03-01 balance Assets:Bank:Savings 500.00 CNY with pad Equity:Opening-Balances
```

Zhang adds a padding transaction (flag `P`) that moves the difference between the asserted amount and the
account's balance there from the pad account, so the assertion holds. The difference is measured from the sum of
the postings of the account and its sub-accounts: an earlier assertion, even a failing one, does not count. The
padding goes to the asserted account itself, also when it is a parent account. A pad brings the account to exactly
the asserted amount: it pads even a difference within an explicit `~` tolerance, where Beancount pads nothing. An
account already at the asserted amount gets no padding transaction.

- A `pad` serves the next `balance` of its own account in each commodity, until the next `pad` of that account.
- Its padding transaction is dated on the `pad`, as in Beancount, so the balances between the `pad` and the
  assertion include it.
- A `balance` on the day of the `pad` comes before it, as Beancount orders a day, and is not padded.
- The `pad` and the `balance` it serves may be in different files of the ledger.
- A `pad` that pads nothing, because no later assertion of its account needs it, is reported as an
  [`UnusedPad`](/user-guide/error-code/#unusedpad) error, as in Beancount.
- A `balance ... with pad` pads its own assertion, dated on it, and is never reported unused.
- Padding a commodity the account or one of its sub-accounts holds at cost is reported as a
  [`PadWithCost`](/user-guide/error-code/#padwithcost) error on the assertion, as in Beancount. The padding is
  booked without a cost.

The `balance ... with pad` is still an assertion, listed in the journal with the others. It is checked once every
balance entry of its time is booked, and fails with an
[`AccountBalanceCheckError`](/user-guide/error-code/#accountbalancecheckerror) instead of holding silently when the
padding cannot bring the total to the asserted amount:

- when the pad of a sub-account at the same time changes the total afterwards. Write the balances of sub-accounts
  before those of their parents; the batch balance tool does so;
- when it pads from the asserted account itself or from one of its sub-accounts: that padding moves units within
  the total it asserts, so it never changes it. Beancount fails such a pad too.

In a Beancount ledger, a pad of an account and a balance of one of its sub-accounts in the same batch fail
`bean-check` whichever is written first: Beancount lets the sub-account's balance use up the parent's pad (see the
differences below). The batch balance tool warns about it; check the parent without a pad instead.

Zhang differs from Beancount in how pads are sized and paired:

- A pad brings the account to exactly the asserted amount, also within an explicit `~` tolerance, where Beancount
  pads nothing (see above).
- Only an assertion on the padded account itself uses the `pad`. Beancount also lets an assertion on a sub-account
  use up the `pad` of its parent account, which then pads nothing for the parent's own assertion.
- A pad is sized from the balance with every padding before it. Beancount sizes the pad of a parent account without
  the padding of its sub-accounts, so the parent's assertion fails there by that padding.

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
