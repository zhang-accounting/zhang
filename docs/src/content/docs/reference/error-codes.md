---
title: Error Codes
description: Every error code Zhang reports for a ledger, what triggers it and how to fix it.
sidebar:
  order: 15
---

When Zhang loads a ledger, it reports the problems it finds as errors. The ledger still loads: the errors page of the
web UI lists them, with the directive that caused each one, and so do `GET /api/errors` and the
[`#errors`](/reference/query-language/#errors) query table. They come by file, then by position in the file, and the
errors page shows where each one is as its file and the byte offsets of the directive. Each error has a code, listed
below with the message the errors page shows for it.

Some problems stop the ledger from loading instead; they have no code. See
[When the ledger does not load](#when-the-ledger-does-not-load).

## UnbalancedTransaction

*Transaction is Unbalanced*

The postings of a transaction do not add up to zero in some commodity. As in Beancount, each commodity must balance on
its own. A posting contributes its *weight*:

- its units, for a plain posting;
- its units converted at the price, for `10 USD @ 7 CNY` (70 CNY) or `10 USD @@ 70 CNY`;
- its units at cost, for a posting with a cost. For `{}`, the cost of the lots the posting reduces, so `-15 USD {}`
  against lots `10 USD {10 CNY}` and `10 USD {11 CNY}` (FIFO) weighs `-155 CNY`.

The sum of each commodity is rounded to the commodity's `precision` with its `rounding` before it is compared with
zero, so with precision 2 a difference of `0.004` passes. If a weight commodity is not defined,
[`CommodityDoesNotDefine`](#commoditydoesnotdefine) is reported instead.

```zhang
2024-01-01 open Assets:Cash
2024-01-01 open Expenses:Food

2024-01-02 * "Cafe" "lunch"
  Assets:Cash -10 CNY
  Expenses:Food 12 CNY
```

The transaction is still booked as written. **Fix:** correct the amounts, or leave out the amount of one posting so
Zhang infers it. See [How a transaction balances](/reference/directives/transaction/#how-a-transaction-balances).

## TransactionCannotInferTradeAmount

*Cannot infer the trade amount of the transaction*

One posting leaves out its amount, and there is nothing to infer it from: the other postings have no amounts, or they
balance in several commodities.

```zhang
2024-01-01 open Assets:Cash
2024-01-01 open Expenses:Food

2024-01-02 * "Cafe" "lunch"
  Assets:Cash
```

If the other postings already balance in a single commodity, the posting without an amount gets zero of it (a sale at
cost with an implicit gain books `0 CNY`), so the journal still shows the posting you wrote; Beancount drops such a
posting instead.

The inferred amount is exact: amounts, costs, prices and their products are never rounded. Only a cost that Zhang
has to divide, such as a total cost `{{1000 USD}}` spread over 3 units (333.333… USD each), leaves more than 20
decimals; such an amount is rounded, with the commodity's `rounding`, at the larger of the commodity's `precision` and
the most decimals written in the transaction in that commodity. Selling all 3 units then gives back exactly
`1000 USD`.

The transaction is **not booked**. **Fix:** write the amounts of the other postings, or the amount of this one.

## TransactionHasMultipleImplicitPosting

*Transaction has more than one implicit posting unit*

More than one posting leaves out its amount. As in Beancount, only one posting per transaction may do so.

```zhang
2024-01-01 open Assets:Cash
2024-01-01 open Assets:Card
2024-01-01 open Expenses:Food

2024-01-02 * "Cafe" "lunch"
  Expenses:Food 10 CNY
  Assets:Cash
  Assets:Card
```

The transaction is **not booked**. **Fix:** write the amounts of all postings but one.

## TransactionExplicitPostingHaveMultipleCommodity

*Explicit postings of the transaction use multiple commodities*

One posting leaves out its amount, and the weights of the other postings leave more than one commodity unbalanced,
so Zhang cannot tell which commodity the missing amount is in.

```zhang
2024-01-01 commodity USD
2024-01-01 open Assets:Cash
2024-01-01 open Assets:Card
2024-01-01 open Expenses:Food

2024-01-02 * "Cafe" "lunch"
  Expenses:Food 10 CNY
  Assets:Card -5 USD
  Assets:Cash
```

The posting without an amount is inferred from the weights of the other postings, after their lots are matched. A
`{}` sale that reduces more units than its lots hold leaves a part without cost, which weighs its units: that is a
second commodity, so the transaction also gets this error, after
[`NoEnoughCommodityLot`](#noenoughcommoditylot).

The transaction is **not booked**. **Fix:** write every amount, or convert with a price (`@`) so the weights are in
one commodity.

## AccountBalanceCheckError

*Account does not pass the balance check*

A [`balance`](/reference/directives/balance/) assertion does not match: the account and its sub-accounts hold a
different amount of the commodity, by more than the assertion's `~` tolerance, or at all without one. A
`balance … with pad` also fails when its padding cannot bring the total to the asserted amount.

```zhang
2024-01-01 open Assets:Cash
2024-01-01 open Equity:Opening-Balances

2024-01-01 * "Opening balance"
  Assets:Cash 100 CNY
  Equity:Opening-Balances

2024-01-02 balance Assets:Cash 500 CNY
```

The error's `account_name` meta names the asserted account. A failing assertion only reports this error: it changes
no balance, and the account keeps the sum of its postings everywhere. The journal shows the assertion with the
asserted amount and the actual balance.

**Fix:** find the missing or wrong transaction and correct it. Remember that an assertion without a time is checked
before the transactions of its own day. To set the balance on purpose, use
[`balance … with pad`](/reference/directives/balance/#padding-with-with-pad).

## AccountDoesNotExist

*Account does not exist*

A directive uses an account that has no `open` before it: a posting of a transaction, a `balance` (its account or
its pad account), a `note`, a `document`, or a `close`. The account was never opened, opened only later, or its name
has a typo.

```zhang
2024-01-01 open Expenses:Food

2024-01-02 * "Cafe" "lunch"
  Assets:Csh -10 CNY
  Expenses:Food 10 CNY
```

Each account is reported once per directive, with the account in the error's `account_name` meta. The directive still
takes effect: the transaction is booked, the assertion checked. **Fix:** correct the account name, or add an `open`
dated on or before the first use.

## AccountClosed

*Try to operate a closed account*

A directive uses an account after its `close`: a transaction dated after the day of the `close`, a `balance` or
`document` after it, or a second `close`. As in Beancount, an account stays usable through the whole day of its
`close`, and a `note` may follow the `close` without an error.

```zhang
2024-01-01 open Assets:Old-Card
2024-01-01 open Expenses:Food
2024-03-31 close Assets:Old-Card

2024-04-02 * "Cafe" "lunch"
  Assets:Old-Card -10 CNY
  Expenses:Food 10 CNY
```

Each account is reported once per transaction, and the transaction is still booked. **Fix:** post to another
account, correct the date, or reopen the account with a new `open`.

## CommodityDoesNotDefine

*Try to use a undefined commodity*

A commodity is used before a `commodity` directive defines it:

- a transaction balances in it, as units, price or cost (the error has the transaction's `txn_id` meta);
- an `open` lists it, or a `price` names it (the error has a `commodity_name` meta);
- a padding transaction of `balance … with pad` is in it.

```zhang
2024-01-01 open Assets:Cash
2024-01-01 open Expenses:Food

2024-01-02 * "Cafe" "lunch"
  Assets:Cash -10 USD
  Expenses:Food 10 USD
```

The [operating currency](/reference/directives/options/#operating_currency) is defined by its option, so a ledger
without that option can use `CNY` without a `commodity` directive. **Fix:** add a `commodity` directive dated on or
before the first use. On the same date, write it above an `open` that lists it.

## NoEnoughCommodityLot

*Not enough commodity lots to book this posting*

A posting with a cost reduces more units than the matching lots of the account hold.

```zhang
2024-01-01 commodity USD
2024-01-01 commodity AAPL
2024-01-01 open Assets:Broker
2024-01-01 open Assets:Cash

2024-01-02 * "Buy"
  Assets:Broker 5 AAPL {100 USD}
  Assets:Cash -500 USD

2024-02-01 * "Sell"
  Assets:Broker -10 AAPL {100 USD}
  Assets:Cash 1000 USD
```

The transaction is still booked, and the lot goes negative. **Fix:** check the number of units and the cost the
posting names; a missing purchase is a common cause. See [Lots and cost basis](/guides/lots-and-cost-basis/).

## CloseNonZeroAccount

*Trying to close an account with non zero balance*

A `close` directive closes an account whose own balance, without its sub-accounts, is not zero in some commodity.

```zhang
2024-01-01 open Assets:Old-Card
2024-01-01 open Equity:Opening-Balances

2024-01-02 * "Opening balance"
  Assets:Old-Card 100 CNY
  Equity:Opening-Balances

2024-03-31 close Assets:Old-Card
```

The account is closed anyway. **Fix:** move the remaining amount to another account before the `close`.

## BudgetDoesNotExist

*Budget does not exist*

A directive names a [budget](/reference/directives/budget/) that is not defined at its date:

- a `budget-add`, `budget-transfer` or `budget-close`. The directive is ignored.
- a posting to an account linked to the budget with its `budget` metadata. It is reported once per account and
  budget, with the `account_name` and `budget_name` metas; the posting is not counted toward the budget, and the
  transaction is still booked.

```zhang
2024-01-01 budget-add Travel 500 CNY
```

**Fix:** add the `budget` directive, dated on or before the first use, or correct the budget's name.

## DefineDuplicatedBudget

*Trying to define duplicated budget name*

A `budget` directive names a budget that already exists. The second definition is ignored.

```zhang
2024-01-01 budget Food CNY
2024-02-01 budget Food CNY
```

**Fix:** remove the second `budget` directive, or give it another name.

## MultipleOperatingCurrencyDetect

*Ledger contains multiple operating currency options, which is not recommended in zhang*

The `operating_currency` option is set more than once. Zhang supports a single operating currency; the value read
last is used. Beancount ledgers often set several for Fava.

```zhang
option "operating_currency" "USD"
option "operating_currency" "EUR"
```

**Fix:** keep one `operating_currency` option.

## ParseInvalidMeta

*Directive has an invalid meta value*

A metadata value or option that Zhang reads has a value it does not understand:

- the `booking_method` metadata of an `open`, or the `default_booking_method` option, is not a booking method. The
  account books with the ledger's default booking method, and an invalid option leaves the default at `FIFO`;
- the `timeout` or `allowed_paths` metadata of a [`plugin`](/reference/directives/plugin/#capabilities) directive is
  invalid. The plugin runs with the default for that capability.

```zhang
2024-01-01 open Assets:Cash
  booking_method: "NON_EXIST"
```

The ledger still loads. **Fix:** use a valid value: `STRICT`, `FIFO` or `LIFO` for a booking method; see the
[plugin capabilities](/reference/directives/plugin/#capabilities) for `timeout` and `allowed_paths`.

## UnsupportedBookingMethod

*Booking method is not supported yet, the account uses the default booking method*

An account's `booking_method`, or the `default_booking_method` option, is a booking method Zhang does not implement
yet: `NONE`, `AVERAGE` or `AVERAGE_ONLY`. The error is reported once, on the `open` or `option` directive. The account
books with the ledger's default booking method, and an unsupported option leaves the default at `FIFO`.

```zhang
2024-01-01 open Assets:Broker
  booking_method: "AVERAGE"
```

**Fix:** use one of the supported booking methods: `STRICT`, `FIFO` or `LIFO`.

## AmbiguousLotMatch

*Reduction matches several lots, which is ambiguous under the STRICT booking method*

On an account using the `STRICT` booking method, a reduction matches several lots and does not reduce all of them in
full, so the lot to reduce is ambiguous. This follows Beancount's `STRICT` method. The transaction is still booked,
like `FIFO` among the matching lots, so the ledger keeps its numbers until the ambiguity is resolved. The error's
`matched_lots` meta lists the lots it matched.

```zhang
2024-01-01 commodity USD
2024-01-01 commodity AAPL
2024-01-01 open Assets:Broker
  booking_method: "STRICT"
2024-01-01 open Assets:Cash

2024-01-02 * "Buy"
  Assets:Broker 10 AAPL {100 USD}
  Assets:Cash -1000 USD
2024-02-01 * "Buy"
  Assets:Broker 10 AAPL {110 USD}
  Assets:Cash -1100 USD
2024-03-01 * "Sell"
  Assets:Broker -5 AAPL {}
  Assets:Cash 500 USD
```

**Fix:** name the lot to reduce with its cost and acquisition date, such as `-5 AAPL {100 USD, 2024-01-02}`, reduce
all matching lots at once, or use the `FIFO` or `LIFO` booking method on the account.

## PluginError

*Plugin `<plugin>`: `<message>`*

A WASM plugin declared with a [`plugin`](/reference/directives/plugin/) directive reported a problem, usually a
validator that checks the ledger without changing it. The plugin reports it through the `zhang_emit_error` host
function, and the ledger still loads. The error's `message` meta describes the problem and its `plugin` meta names the
plugin; the plugin may add metas of its own. The error points at the directive the plugin names, or at the plugin's
`plugin` directive when it names none. [Writing Plugins](/developers/writing-plugins/#reporting-errors) shows how a
plugin reports one.

For example, a plugin that requires a payee on every transaction reports `message: "payee is missing"` on this
transaction:

```zhang
2024-01-01 open Assets:Cash
2024-01-01 open Expenses:Food

2024-01-02 * "lunch"
  Assets:Cash -10 CNY
  Expenses:Food 10 CNY
```

**Fix:** do what the `message` meta asks, or change the plugin's settings. A message saying that the plugin called
`zhang_emit_error` with an invalid payload is a bug in the plugin: report it to the plugin's author.

## When the ledger does not load

These problems stop Zhang from loading the ledger. `zhang serve` exits with exit code 1 and logs the reason; set the
environment variable `RUST_LOG=info` to see the log. When the server is already running, a reload that fails keeps
the ledger as it was.

- **A syntax error** in a file. The message names the file, line and column, such as
  `failed to parse zhang file: unexpected input at line 4, column 3`.
- **An invalid value** for the [`default_rounding`](/reference/directives/options/#default_rounding) or
  [`directive_output_path`](/reference/directives/options/#directive_output_path) option, or for the `rounding`
  metadata of a [commodity](/reference/directives/commodity/#rounding). The message is `option value is invalid`.
- **A plugin** whose module is missing or cannot be loaded, or whose call fails or runs past its `timeout`, while
  plugins are enabled. See [Plugin](/reference/directives/plugin/#loading-and-order).
- **An `include` pattern** outside the [limits of wildcards](/reference/directives/include/#wildcards).
