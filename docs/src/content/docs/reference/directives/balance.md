---
title: Balance
description: "Reference for the balance directive: balance assertions, explicit tolerances and padding with balance … with pad or the Beancount pad directive."
sidebar:
  order: 5
---

A `balance` directive asserts how much of one commodity an account holds at a point in time. Zhang compares the
assertion with the sum of the account's postings and reports an error when they differ. With `with pad`, the directive
also corrects the balance: Zhang adds a padding transaction from another account, so that the assertion holds.

## Syntax

```text
YYYY-MM-DD [HH:MM[:SS]] balance <Account> <Number> [~ <Tolerance>] <Commodity>
YYYY-MM-DD [HH:MM[:SS]] balance <Account> <Number> <Commodity> with pad <PadAccount>
```

| Part | Required | Description |
|---|---|---|
| Date and time | yes | The date, optionally followed by a time of day (`10:30` or `10:30:15`). See [When it is checked](#when-it-is-checked). |
| `<Account>` | yes | The account to check. The check covers the account and all its sub-accounts. |
| `<Number>` | yes | The amount the account should hold. It can be an expression, such as `1000 - 35.5`. |
| `~ <Tolerance>` | no | The largest difference that still passes. Without it, the amount must match exactly. Ignored with `with pad`. |
| `<Commodity>` | yes | The commodity to check. Other commodities in the account are not checked. |
| `with pad <PadAccount>` | no | Pad the difference from this account, usually an `Equity` account. |

Metadata lines can follow the directive.

## Examples

```zhang
2024-01-01 open Assets:Bank:Checking CNY
2024-01-01 open Expenses:Food CNY
2024-01-01 open Equity:Opening-Balances

; set the opening balance
2024-01-01 balance Assets:Bank:Checking 1000.00 CNY with pad Equity:Opening-Balances

2024-01-05 * "Cafe" "lunch"
  Assets:Bank:Checking -35.50 CNY
  Expenses:Food

; check the account against the bank statement
2024-01-31 balance Assets:Bank:Checking 964.50 CNY
```

An assertion with a tolerance passes for any balance from 964.49 to 964.51:

```zhang
2024-01-31 balance Assets:Bank:Checking 964.50 ~ 0.01 CNY
```

## Behavior

### What is checked

- The balance is the sum of the postings of the account **and all its sub-accounts** before the assertion, in the
  asserted commodity. `balance Assets:Bank 100 CNY` passes when `Assets:Bank:Checking` holds 60 CNY and
  `Assets:Bank:Savings` holds 40 CNY. The asserted account must be open itself, also when it is a parent account.
- The amount must match exactly, unless the assertion gives a tolerance with `~`. `964.50 ~ 0.01 CNY` passes when the
  difference is at most 0.01.
- An assertion only checks. Passing or failing, it changes no balance: an account always holds the sum of its
  postings, and every balance, report and journal shows that sum. A failing assertion is reported as an
  [`AccountBalanceCheckError`](/reference/error-codes/#accountbalancecheckerror).
- The journals of the web UI list every assertion with the asserted amount, the balance it was checked against, and
  whether it passed. In queries, assertions are the rows of
  [`#balances`](/reference/query-language/#prices-balances-notes-events-documents-and-commodities).

### When it is checked

Zhang orders the directives by date and time. Within one date and time, `open` and `commodity` come first, then
balance assertions and padding transactions, then everything else in file order. So:

- An assertion without a time is checked at the start of its day, before the transactions of that day. To check the
  balance after them, date the assertion on the next day or give it a time.
- An assertion with a time is checked after the transactions of the same day with an earlier time. A transaction
  without a time counts as midnight.

```zhang
2024-02-01 10:00 * "Cafe" "coffee"
  Assets:Bank:Checking -5.00 CNY
  Expenses:Food

2024-02-01 09:00 balance Assets:Bank:Checking 964.50 CNY
2024-02-01 12:00 balance Assets:Bank:Checking 959.50 CNY
```

### Padding with `with pad`

`balance <Account> <amount> with pad <PadAccount>` brings the account to the asserted amount:

1. Zhang computes the difference between the asserted amount and the account's balance at that point: the sum of the
   postings of the account and its sub-accounts, padding transactions before it included. An earlier assertion, even
   a failing one, does not count.
2. If the difference is not zero, Zhang adds a padding transaction at the same date and time. Its flag is `P`, its
   payee `Balance Pad` and its narration `pad <Account> to <PadAccount>`. It posts the difference to `<Account>`
   itself, also when that is a parent account, and the opposite amount to `<PadAccount>`.
3. An account already at the asserted amount gets no padding transaction.

The padding transaction is an ordinary transaction from then on: it appears in the journals, moves both accounts'
balances, and is a row with flag `P` in queries. Plugins run before padding, so a pad is sized after every transaction
a plugin adds.

A `balance … with pad` is still an assertion. It is checked once every balance entry of its date and time is booked,
and fails with an [`AccountBalanceCheckError`](/reference/error-codes/#accountbalancecheckerror) when the padding
cannot bring the total to the asserted amount:

- when the pad of a sub-account at the same date and time changes the total afterwards. Write the balances of
  sub-accounts before those of their parents. The batch balance tool of the web UI does so.
- when it pads from the asserted account itself or from one of its sub-accounts: that padding moves units within the
  total it asserts, so it never changes it. Beancount fails such a pad too.

A `~ tolerance` written on a `balance … with pad` is ignored: the padding makes the balance exact.

### From the web UI

The balance form on an account page and the batch balance tool write `balance` or `balance … with pad` directives
dated now, into the file that the [`directive_output_path`](/reference/directives/options/#directive_output_path)
option selects.

## Errors

| Error | When |
|---|---|
| [`AccountBalanceCheckError`](/reference/error-codes/#accountbalancecheckerror) | The balance differs from the asserted amount by more than the tolerance, or at all without one. |
| [`AccountDoesNotExist`](/reference/error-codes/#accountdoesnotexist) | The account, or the pad account, is not open at that date. The check and the padding still happen. |
| [`AccountClosed`](/reference/error-codes/#accountclosed) | The account, or the pad account, is already closed at that point. |
| [`CommodityDoesNotDefine`](/reference/error-codes/#commoditydoesnotdefine) | A padding transaction uses a commodity that is not defined. The error points at the `balance … with pad`. |

## Beancount compatibility

In a Beancount file (`.bean`, `.bc` or `.beancount`), `with pad` does not exist. Write Beancount's `pad` directive
instead:

```beancount
2024-01-01 pad Assets:Bank:Checking Equity:Opening-Balances
2024-01-02 balance Assets:Bank:Checking 1000.00 CNY
```

Zhang turns each `pad` and the `balance` it serves into one `balance … with pad`. A `pad` serves the next `balance` of
its own account in each commodity, until the account's next `pad`. A `balance` on the day of the `pad` comes before it
and is not padded. In a zhang file (`.zhang`), there is no `pad` directive: use `with pad`. When Zhang writes a
`balance … with pad` into a Beancount file, it writes a `pad` dated the day before and the `balance`.

Zhang differs from Beancount here:

- The padding transaction is dated on the `balance` it serves, where Beancount dates it on the `pad`, and the `pad`
  and its `balance` must be in the same file.
- Only an assertion on the padded account itself uses the `pad`. Beancount also lets an assertion on a sub-account
  use up the `pad` of its parent account, which then pads nothing for the parent's own assertion. So in a Beancount
  ledger, a pad of an account and a balance of one of its sub-accounts written at the same time fail `bean-check`.
  The batch balance tool warns about it; check the parent without a pad instead.
- A pad is sized from the balance with every padding before it. Beancount sizes the pad of a parent account without
  the padding of its sub-accounts, so the parent's assertion fails there by that padding.
- Beancount infers a tolerance from the number of decimals of the asserted amount. Zhang does not: an assertion
  without `~` must match exactly. Add `~ 0.01`, or the tolerance you want, where you relied on Beancount's.
- A time of day is written as `time: "HH:MM:SS"` metadata in a Beancount file.

## Related

- [Balances](/guides/balances/): using balance assertions and pads in practice.
- [Transaction](/reference/directives/transaction/): how padding transactions and other transactions are booked.
