---
title: Balance
description: "Reference for the balance directive: balance assertions, explicit tolerances and padding with balance … with pad or the pad directive."
sidebar:
  order: 5
---

A `balance` directive asserts how much of one commodity an account holds at a point in time. Zhang compares the
assertion with the sum of the account's postings and reports an error when they differ. With `with pad`, the directive
also corrects the balance: Zhang adds a padding transaction from another account, so that the assertion holds. A
`pad` directive, as in Beancount, does the same for the next assertions of its account.

## Syntax

```text
YYYY-MM-DD [HH:MM[:SS]] balance <Account> <Number> [~ <Tolerance>] <Commodity>
YYYY-MM-DD [HH:MM[:SS]] balance <Account> <Number> <Commodity> with pad <PadAccount>
YYYY-MM-DD [HH:MM[:SS]] pad <Account> <PadAccount>
```

| Part | Required | Description |
|---|---|---|
| Date and time | yes | The date, optionally followed by a time of day (`10:30` or `10:30:15`). See [When it is checked](#when-it-is-checked). |
| `<Account>` | yes | The account to check. The check covers the account and all its sub-accounts. |
| `<Number>` | yes | The amount the account should hold. It can be an expression, such as `1000 - 35.5`. |
| `~ <Tolerance>` | no | The largest difference that still passes. Without it, the amount must match exactly. Ignored with `with pad`. |
| `<Commodity>` | yes | The commodity to check. Other commodities in the account are not checked. |
| `with pad <PadAccount>` | no | Pad the difference from this account, usually an `Equity` account. |

Metadata lines can follow the directive. A `pad` pads `<Account>` from `<PadAccount>` for its next assertions: see
[The `pad` directive](#the-pad-directive).

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
- In a Beancount file, Zhang ignores the time of a `balance` (its `time` metadata), as Beancount does: the assertion
  is checked at the start of its date. Where transactions of the account on that day before that time, which
  earlier versions of Zhang counted, change what it checks, the assertion is listed with a
  [`BalanceTimeIgnored`](/reference/error-codes/#balancetimeignored) notice.

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

### The `pad` directive

A `pad` directive pads its account for its next balance assertions, from the pad account, as in Beancount. It works in
Zhang files and Beancount files alike:

```zhang
2024-01-01 pad Assets:Bank:Checking Equity:Opening-Balances
2024-02-01 balance Assets:Bank:Checking 1000.00 CNY
```

- A `pad` serves the first `balance` of its own account in each commodity on a later day than the `pad`, until a
  later `pad` of that account. Days are compared, not times, as Beancount knows no times.
- A `balance` on the day of the `pad` comes before it, as Beancount orders a day, and is not padded, whatever their
  times: in a Zhang file with times, a `pad` comes after the last balance of its day. In a Beancount file, Zhang
  ignores the `time` metadata of a `pad`, and the `pad`s of a day keep the order of their lines.
- Its padding transaction is dated on the `pad` and comes right after it, as in Beancount, so the balances between
  the `pad` and the assertion include it. When the `pad` comes after a later balance of its day, the padding is dated
  at the time of that balance. A `pad` that pads several commodities adds one padding transaction for each.
- A pad is sized when the assertion it serves comes, with the padding of every assertion padded before it. The
  padding of a `pad` dated earlier that serves a later assertion is not counted: when the `pad` of a sub-account
  comes before the `pad` of its parent but serves a later balance, the parent is padded without it, and its balance
  fails by that padding, as in Beancount.
- The `pad` and the `balance` it serves may be in different files of the ledger.
- A `pad` that pads nothing, because no later assertion of its account needs it, is reported as an
  [`UnusedPad`](/reference/error-codes/#unusedpad) error, as in Beancount.
- A `balance … with pad` pads its own assertion and is never reported unused. A `pad` before it serves it first.
- Padding a commodity the account or one of its sub-accounts holds at cost, such as shares bought `{100 USD}`, is
  reported as a [`PadWithCost`](/reference/error-codes/#padwithcost) error on the assertion, as in Beancount. The
  padding is booked without a cost.

### From the web UI

The balance form on an account page and the batch balance tool write into the file that the
[`directive_output_path`](/reference/directives/options/#directive_output_path) option selects.

- In a Zhang file, they write a `balance` or a `balance … with pad` dated now.
- In a Beancount file, they write so that Beancount reads the balance as Zhang does, and nothing is padded but what
  you asked for, when you asked:
  - "My balance now" is a `balance` dated tomorrow: the start of tomorrow is the end of today, after every
    transaction of today. A transaction you add later today is not in the amount you asserted, so it makes that
    balance fail, as in Beancount.
  - Checking again replaces that balance of tomorrow in your file instead of adding a second one. It asserts exactly
    the new amount: a `~` tolerance it had goes, and its metadata and comment stay as you wrote them. A balance you
    wrote more than once is changed everywhere. The web UI tells you which balances it replaced, with the amount and
    tolerance each had.
  - With a pad, the difference between your amount and what the account and its sub-accounts hold now is written as
    a padding transaction (flag `P`, payee `Balance Pad`) dated now, before the `balance`. Nothing is written when
    there is no difference. A padding transaction written earlier stays, and a new pad books the difference from
    it. No `pad` directive is written: it would pad the next balance of every commodity of the account, and absorb a
    transaction you add later today.
  - A `pad` you wrote yourself would still pad a balance written after it, in a commodity it never served. Such a
    balance is refused, and the message names the file and the date of that `pad`: edit the file, and add a balance
    of that commodity on the day after the `pad`, right after it.

In both kinds of files, these are refused, as they could only be reported once written:

- a balance of an account that is closed or not open, or padded from one;
- a pad from the account itself or one of its sub-accounts: the padding would move units within the total the
  balance asserts, which it never changes;
- a pad of a commodity the account or one of its sub-accounts holds at cost on that day, which would book units
  without a cost. Record them with their cost instead, as a purchase or a sale.

A refused request writes nothing, and the web UI shows why. A request also writes nothing when a file it would change
in place was edited since Zhang loaded it: it answers that the file changed, and trying again works on the ledger
reloaded. When the files cannot be loaded, say after saving a mistake in the file editor, every write but the file
editor's is refused until you fix them there.

## Errors

| Error | When |
|---|---|
| [`AccountBalanceCheckError`](/reference/error-codes/#accountbalancecheckerror) | The balance differs from the asserted amount by more than the tolerance, or at all without one. |
| [`UnusedPad`](/reference/error-codes/#unusedpad) | A `pad` pads nothing: no later assertion of its account needs it. |
| [`PadWithCost`](/reference/error-codes/#padwithcost) | A pad pads a commodity the account or a sub-account holds at cost. The error points at the assertion. |
| [`BalanceTimeIgnored`](/reference/error-codes/#balancetimeignored) | A notice: in a Beancount file, the time of an assertion is ignored, and that changes what it checks. |
| [`AccountDoesNotExist`](/reference/error-codes/#accountdoesnotexist) | The account, or the pad account, is not open at that date. The check and the padding still happen. |
| [`AccountClosed`](/reference/error-codes/#accountclosed) | The account, or the pad account, is already closed at that point. |
| [`CommodityDoesNotDefine`](/reference/error-codes/#commoditydoesnotdefine) | A padding transaction uses a commodity that is not defined. The error points at the `balance … with pad`. |

## Beancount compatibility

In a Beancount file (`.bean`, `.bc` or `.beancount`), `with pad` does not exist: write the `pad` directive, which
Zhang reads as Beancount does (see [The `pad` directive](#the-pad-directive)). A time of day is written as
`time: "HH:MM:SS"` metadata in a Beancount file, and Zhang ignores it on `balance` and `pad`, as Beancount does.

Zhang differs from Beancount here:

- A pad brings the account to exactly the asserted amount, also within an explicit `~` tolerance, where Beancount
  pads nothing.
- Only an assertion on the padded account itself uses the `pad`. Beancount also lets an assertion on a sub-account
  use up the `pad` of its parent account, which then pads nothing for the parent's own assertion.
- A pad is sized with the padding of every assertion padded before it. Beancount sizes the pad of a parent account
  without the padding of its sub-accounts, so the parent's assertion fails there by that padding.
- Zhang orders the directives of a day by their time, then by file, in the order the ledger includes the files, then
  by line. Beancount orders them by line, whatever their file, and knows no times. Of two `pad`s of an account on one
  day in different files, the one Zhang orders last pads, which may not be the one Beancount uses.
- Padding a commodity held at cost is one `PadWithCost` error for the assertion it serves. Beancount reports it once
  for each lot held at cost.
- Beancount infers a tolerance from the number of decimals of the asserted amount. Zhang does not: an assertion
  without `~` must match exactly. Add `~ 0.01`, or the tolerance you want, where you relied on Beancount's.

## Related

- [Balances](/guides/balances/): using balance assertions and pads in practice.
- [Transaction](/reference/directives/transaction/): how padding transactions and other transactions are booked.
