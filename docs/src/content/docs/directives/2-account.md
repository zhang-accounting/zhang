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

- The assertion is checked at the start of its date, against the sum of the account's postings before it.
  Transactions of the same day come after it, as in Beancount. In a Zhang ledger, an assertion with a time is checked
  at that time. In a Beancount ledger, Zhang ignores its `time` metadata, as Beancount does. Where transactions of the
  account on that day before that time, which earlier versions of Zhang counted, changed what it checks, the assertion
  is listed with a [`BalanceTimeIgnored`](/user-guide/error-code/#balancetimeignored) notice.
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
account's balance at the assertion from the pad account, so the assertion holds. The difference is measured from the
sum of the postings of the account and its sub-accounts, with the padding of every assertion padded before this one:
an earlier assertion, even a failing one, does not count. The padding goes to the asserted account itself, also when
it is a parent account. A pad brings the account to exactly the asserted amount: it pads even a difference within an
explicit `~` tolerance, where Beancount pads nothing. An account already at the asserted amount gets no padding
transaction.

- A `pad` serves the first `balance` of its own account in each commodity on a later day than the `pad`, until a
  later `pad` of that account. Days are compared, not times, as Beancount knows no times.
- A `balance` on the day of the `pad` comes before it, as Beancount orders a day, and is not padded, whatever their
  times: in a Zhang ledger with times, a `pad` comes after the last balance of its day. In a Beancount ledger, Zhang
  ignores the `time` metadata of a `pad`, as Beancount does, and the `pad`s of a day keep the order of their lines.
- Its padding transaction is dated on the `pad` and comes right after it, as in Beancount, so the balances between
  the `pad` and the assertion include it. When the `pad` comes after a later balance of its day, the padding is dated
  at the time of that balance.
- A pad is sized when the assertion it serves comes. The padding of a `pad` that is dated earlier but serves a later
  assertion is not counted: when the `pad` of a sub-account comes before the `pad` of its parent account but serves a
  later balance, the parent is padded without it, and its balance fails by that padding, as in Beancount.
- The `pad` and the `balance` it serves may be in different files of the ledger.
- A `pad` that pads nothing, because no later assertion of its account needs it, is reported as an
  [`UnusedPad`](/user-guide/error-code/#unusedpad) error, as in Beancount.
- A `balance ... with pad` pads its own assertion, dated on it, and is never reported unused. A `pad` before it
  serves it first.
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

In a Beancount ledger, the balances made in the UI or with the batch balance tool are written so that Beancount
reads them as Zhang does, and nothing is padded but what you asked for, when you asked:

- "My balance now" is a `balance` dated tomorrow: the start of tomorrow is the end of today, after every transaction
  of today. A transaction you add later today is not in the amount you asserted, so it makes that balance fail, as in
  Beancount. Check the balance again: a new check of the account and commodity replaces that balance of tomorrow in
  your file instead of adding a second one. It asserts exactly the new amount: a `~` tolerance it had goes, and its
  metadata and comment stay as you wrote them. A balance you wrote more than once is changed everywhere. The UI tells
  you which balances it replaced, with the amount and tolerance each had.
- With a pad, the difference between your amount and what the account and its sub-accounts hold now is booked as a
  padding transaction (flag `P`) dated now, from the pad account, before the `balance`. Nothing is booked when there
  is no difference. A padding transaction booked earlier stays, and a new pad books the difference from it. The UI
  writes no `pad` directive: a `pad` would pad the next balance of every commodity of the account, and would silently
  absorb a transaction you add later today.
- A `pad` you wrote yourself would still pad a balance written after it, in a commodity it never served, and absorb
  the transactions before that balance. Such a balance is refused, and the message names the file and the date of
  that `pad`: edit the file, and add a balance of that commodity on the day after the `pad`, right after it. This
  covers a commodity the account first holds after the `pad`.

In both ledgers, these are refused, as they could only be reported once written:

- a balance of an account that is closed or not open, or padded from one;
- a pad from the account itself or one of its sub-accounts: the padding would move units within the total the
  balance asserts, which it never changes;
- a pad of a commodity the account or one of its sub-accounts holds at cost: it would book units without a cost.
  Record them with their cost instead, as a purchase or a sale.

A refused request writes nothing, and the UI shows why. A request also writes nothing when a file it would change in
place was edited since Zhang loaded it, and Zhang has not reloaded it yet: it answers 409, and the ledger is reloaded,
so trying again works. When the files cannot be loaded, say after saving a mistake in the file editor, every write
but the file editor's answers 409 until you fix them there.

Zhang differs from Beancount in how pads are sized and paired:

- A pad brings the account to exactly the asserted amount, also within an explicit `~` tolerance, where Beancount
  pads nothing (see above).
- Only an assertion on the padded account itself uses the `pad`. Beancount also lets an assertion on a sub-account
  use up the `pad` of its parent account, which then pads nothing for the parent's own assertion.
- A pad is sized with the padding of every assertion padded before it. Beancount sizes the pad of a parent account
  without the padding of its sub-accounts, so the parent's assertion fails there by that padding.
- Zhang orders the directives of a day by their time, then by file, in the order the ledger includes the files, then
  by line. Beancount orders them by line, whatever their file, and knows no times. Of two `pad`s of an account on one
  day in different files, the one Zhang orders last pads, which may not be the one Beancount uses. In a Beancount
  ledger, Zhang ignores the `time` metadata of `balance` and `pad` directives, so within one file the pads of a day
  are ordered as in Beancount.
- Padding a commodity held at cost is one `PadWithCost` error for the assertion it serves. Beancount reports it once
  for each lot held at cost.

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

In a Beancount ledger, the path of a `document` is relative to the directory of the file it is written in, as
Beancount reads it. The documents you upload are written so: one written into `data/2026/10.bean` names
`../../attachments/…`. Earlier versions wrote the path from the ledger's directory into such files, which Beancount
reports as missing and Zhang no longer finds: put `../../` before them.

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
