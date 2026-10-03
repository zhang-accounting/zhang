---
title: Error Code Guide
description: A detailed guide on understanding and resolving common error codes in Zhang Accounting.
---

# Error Code Guide

This guide provides comprehensive explanations and solutions for common error codes encountered in Zhang Accounting. Understanding these error codes will help you troubleshoot and resolve issues more efficiently.

## UnbalancedTransaction

This error indicates that a transaction is unbalanced, meaning the sum of each posting does not equal zero. This is often due to discrepancies in the amounts or currencies used in the postings.

Like Beancount, each commodity must balance on its own. A posting contributes its *weight*:

- its units, for a plain posting;
- its units converted at the price, for `10 USD @ 7 CNY` (70 CNY) or `10 USD @@ 70 CNY`;
- its units at cost, for a cost posting. For `{}`, the cost of the lots the posting reduces, so
  `-15 USD {}` against lots `10 USD {10 CNY}` and `10 USD {11 CNY}` (FIFO) weighs `-155 CNY`.

The sum of each commodity is rounded at the commodity's `precision` and `rounding` before it is compared with zero.
If a weight commodity is not defined, `CommodityDoesNotDefine` is reported instead.

**Example of Unbalanced Transaction:**
```zhang {2-3}
1970-01-01 "" ""
    Assets:A -10 USD
    Assets:B  10 CNY
```

**Solution:** Ensure all postings within a transaction balance out, using the same currency or properly converting between currencies.

## TransactionCannotInferTradeAmount

Occurs when Zhang Accounting cannot infer the trade amount for a transaction. This can happen if postings lack explicit amounts or if the transaction's context does not allow for an amount to be inferred.

**Example of Error:**
```zhang
1970-01-01 * "Payee" "Buying goods"
    Assets:Cash
    Expenses:Goods  100 USD
```

**Correct Case:**
```zhang
1970-01-01 * "Payee" "Buying goods"
    Assets:Cash  -100 USD
    Expenses:Goods  100 USD
```

If the other postings already balance in a single commodity, the implicit posting gets zero of it (a sale at cost
with an implicit gain books `0 CNY`), so the journal still shows the posting you wrote; Beancount drops such a posting
instead. This error remains when there is nothing to infer from, or when the other postings balance in several
commodities.

The inferred amount is exact: amounts, costs, prices and their products are never rounded. Only a cost that zhang has
to divide, such as a total cost `{{1000 USD}}` spread over 3 units (333.333… USD each), leaves more than 20 decimals;
such an amount is rounded, with the commodity's `rounding`, at the larger of the commodity's `precision` and the most
decimals written in the transaction in that commodity. Selling all 3 units then gives back exactly `1000 USD`.

**Solution:** Make sure to specify amounts for all postings in a transaction or ensure the transaction's context allows for an amount to be inferred.

## TransactionHasMultipleImplicitPosting

Zhang Accounting, similar to Beancount, allows only one implicit posting per transaction to ensure clarity and prevent ambiguity.

**Example of Error:**
```zhang {3-4}
1970-01-01 "" ""
    Assets:A -10 USD
    Assets:B
    Assets:C 
```

**Solution:** Ensure only one posting in a transaction lacks an explicit amount.

## TransactionExplicitPostingHaveMultipleCommodity

This error is triggered when a transaction has postings with multiple non-zero commodity amounts, making it impossible to infer amounts for implicit postings.

The implicit posting is inferred from the weights of the other postings (see `UnbalancedTransaction`), after their lots
are matched. A `{}` sale that reduces more units than its lots hold leaves a part without cost, which weighs its units:
that is a second commodity, so the transaction also gets this error, after `NoEnoughCommodityLot`.

**Example of Error:**
```zhang {2-3}
1970-01-01 "" ""
Assets:A -10 USD
Assets:B 10 CNY
Assets:C
```

**Solution:** Review and adjust the transaction to ensure only one commodity is involved or all postings have explicit amounts.

## AccountBalanceCheckError

Indicates a failure in an account's balance check, possibly due to incorrect balance entries or transactions affecting the account.

**Example of Error:**
```zhang
// Assuming Assets:Checking owns 100 USD

1970-01-01 balance Assets:Checking  500 USD
```

**Correct Case:**
```zhang

// given Assets:Checking owns 100 USD

1970-01-01 balance Assets:Checking  100 USD
```

A failing balance check only reports this error: it changes no balance, and the account keeps the sum of its postings
everywhere. The journal shows the check with the asserted amount and the actual balance.

**Solution:** Verify and correct all transactions affecting the account to ensure the balance check aligns with the actual account balance.
To correct the balance on purpose, use `balance ... with pad` (see [Pads](/directives/2-account/#pads)).

## AccountDoesNotExist

Triggered when operations are performed on an account that has not been defined in the ledger.

**Example of Error:**
```zhang
1970-01-01 * "Payee" "Transaction for undefined account"
    Assets:UndefinedAccount  -100 USD
    Expenses:Misc  100 USD
```

**Correct Case:**
```zhang
1970-01-01 open Assets:DefinedAccount
1970-01-01 open Expenses:Misc
1970-01-01 * "Payee" "Transaction for defined account"
    Assets:DefinedAccount  -100 USD
    Expenses:Misc  100 USD
```

**Solution:** Define the account using the `open` directive before referencing it in transactions or other operations.

Every posting of a transaction is checked: a posting to an account that was never opened, or that is only opened later than the transaction, reports this error once per account and transaction, with the account in its `account_name` meta. A `note` on such an account reports it too. The transaction is still booked, so fix the account name or add the missing `open`.

## AccountClosed

Occurs when attempting to perform operations on a closed account. Ensure the account is open or reopen it before performing transactions.

**Example of Error:**
```zhang
1970-01-01 open Assets:ClosedAccount
1970-01-01 open Expenses:Misc
1970-01-01 close Assets:ClosedAccount
1970-01-02 * "Payee" "Transaction for closed account"
    Assets:ClosedAccount  -100 USD
    Expenses:Misc  100 USD
```

**Correct Case:**
```zhang
1970-01-01 open Assets:ReopenedAccount
1970-01-01 open Expenses:Misc
1970-01-02 * "Payee" "Transaction for reopened account"
    Assets:ReopenedAccount  -100 USD
    Expenses:Misc  100 USD
```

**Solution:** Reopen the account using the `open` directive if necessary before conducting transactions.

Every posting of a transaction is checked, once per account and transaction. As in beancount, an account stays usable through the whole day of its `close`: only a transaction dated after that day reports this error. The transaction is still booked. A `note` may follow the `close` without an error.

## CommodityDoesNotDefine

This error occurs when a commodity used in a transaction or directive is not defined in the ledger.

**Example of Error:**
```zhang
1970-01-01 * "Payee" "Transaction with undefined commodity"
    Assets:Cash  -100 XYZ
    Expenses:Misc  100 XYZ
```

**Correct Case:**
```zhang
1970-01-01 commodity XYZ
1970-01-01 * "Payee" "Transaction with defined commodity"
    Assets:Cash  -100 XYZ
    Expenses:Misc  100 XYZ
```

**Solution:** Define the commodity using the `commodity` directive before using it in transactions or other directives.

## NoEnoughCommodityLot

Indicates there's not enough commodity lot available for a transaction. This can happen when selling or transferring more of a commodity than is available.

**Example of Error:**
```zhang
1970-01-01 * "Payee" "Selling more than available"
    Assets:Stocks  -10 SHARES {100 USD}
    Income:Sales  1000 USD
```


**Correct Case:**
```zhang
1970-01-01 * "Payee" "Selling available amount"
    Assets:Stocks  -5 SHARES {100 USD}
    Income:Sales  500 USD
```

**Solution:** Ensure the commodity lots are sufficient for the transaction or adjust the transaction to match the available lots.

## CloseNonZeroAccount

Triggered when attempting to close an account with a non-zero balance. Accounts must have a zero balance before they can be closed.

**Example of Error:**
```zhang

// Assuming Assets:NonZeroBalanceAccount owns 100 USD

1970-01-01 close Assets:NonZeroBalanceAccount
```


**Correct Case:**
```zhang

1970-01-01 balance Assets:ZeroBalanceAccount  0 USD
1970-01-02 close Assets:ZeroBalanceAccount
```

**Solution:** Balance the account to zero before attempting to close it.

## BudgetDoesNotExist

Occurs when referencing a budget that has not been defined. Ensure the budget is defined before referencing it in transactions or directives.

**Example of Error:**
```zhang
1970-01-01 budget-add NonExistentBudget  500 USD
```

**Correct Case:**
```zhang
1970-01-01 budget ExistingBudget USD
1970-01-02 budget-add ExistingBudget  500 USD
```


**Solution:** Define the budget using the `budget` directive before adding funds or performing other operations.

## DefineDuplicatedBudget

This error occurs when a budget is defined more than once, which can lead to confusion and errors in budget tracking.

**Example of Error:**
```zhang
1970-01-01 budget DuplicateBudget USD
1970-01-02 budget DuplicateBudget USD
```


**Correct Case:**
```zhang
1970-01-01 budget UniqueBudget USD
```

**Solution:** Ensure each budget is uniquely defined and avoid duplicating budget definitions.

## MultipleOperatingCurrencyDetect

Triggered when multiple operating currencies are detected in the ledger. Zhang Accounting requires a single operating currency to be defined.

**Example of Error:**
```zhang
option "operating_currency" "USD"
option "operating_currency" "EUR"
```

**Correct Case:**
```zhang
option "operating_currency" "USD"
```

**Solution:** Define only one operating currency in the ledger options.

## ParseInvalidMeta

Occurs when parsing invalid metadata in directives, which can lead to errors in processing and interpretation.

An invalid `booking_method` on an account, or an invalid `default_booking_method` option, is reported on that `open` or
`option` directive. The ledger still loads: the account books with the ledger's default booking method, and an invalid
option leaves the default at `FIFO`.

**Example of Error:**
```zhang
1970-01-01 open Assets:Cash
    booking_method: "NON_EXIST"
```

**Correct Case:**
```zhang
1970-01-01 open Assets:Cash
    booking_method: "FIFO"
```

**Solution:** Ensure metadata is correctly formatted and valid for the context in which it is used.

## UnsupportedBookingMethod

Triggered when an account's `booking_method`, or the `default_booking_method` option, is a booking method Zhang
Accounting does not implement yet: `NONE`, `AVERAGE` or `AVERAGE_ONLY`. The error is reported once, on the `open` (or
`option`) directive. The ledger still loads: the account books with the ledger's default booking method, and an
unsupported option leaves the default at `FIFO`.

**Example of Error:**
```zhang
1970-01-01 open Assets:Stocks
    booking_method: "AVERAGE"
```

**Correct Case:**
```zhang
1970-01-01 open Assets:Stocks
    booking_method: "STRICT"
```

**Solution:** Use one of the supported booking methods: `STRICT`, `FIFO` or `LIFO`.

## AmbiguousLotMatch

Raised on an account using the `STRICT` booking method when a reduction matches several lots and does not reduce all of
them in full, so the lot to reduce is ambiguous. This follows Beancount's `STRICT` method. The transaction is still
booked, like `FIFO` among the matching lots, so the ledger keeps its numbers until the ambiguity is resolved.

**Example of Error:**
```zhang {11}
1970-01-01 open Assets:Stocks
    booking_method: "STRICT"

2024-01-01 * "buy"
    Assets:Stocks  10 AAPL {100 USD}
    Assets:Cash  -1000 USD
2024-02-01 * "buy"
    Assets:Stocks  10 AAPL {110 USD}
    Assets:Cash  -1100 USD
2024-03-01 * "sell"
    Assets:Stocks  -5 AAPL {}
    Assets:Cash  500 USD
```

**Correct Case:**
```zhang
2024-03-01 * "sell"
    Assets:Stocks  -5 AAPL {100 USD, 2024-01-01}
    Assets:Cash  500 USD
```

**Solution:** Name the lot to reduce with its cost (and acquisition date), reduce all matching lots at once, or use the
`FIFO` or `LIFO` booking method on the account.

## PluginError

Reported by a WASM plugin declared with a `plugin` directive, usually a validator that checks the ledger without
changing it. The plugin reports the problem through the `zhang_emit_error` host function, and the ledger still loads.
The error's `message` meta describes the problem and its `plugin` meta names the plugin; the plugin may add metas of
its own. The error points at the directive the plugin names, or at the plugin's `plugin` directive when it names none.

**Example of Error:**
```zhang {4}
option "features.plugin" "true"
plugin "plugins/require-payee.wasm"

2024-01-02 * "lunch"
    Assets:Cash  -10 CNY
    Expenses:Food  10 CNY
```

Here a plugin that requires a payee on every transaction reports `message: "payee is missing"` on the transaction.

**Correct Case:**
```zhang
2024-01-02 * "Burger Shop" "lunch"
    Assets:Cash  -10 CNY
    Expenses:Food  10 CNY
```

**Solution:** Fix what the `message` meta describes, or change the plugin's configuration. A message saying that the
plugin called `zhang_emit_error` with an invalid payload is a bug in the plugin: report it to the plugin's author.
