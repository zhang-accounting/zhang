---
title: Error Code Guide
description: A detailed guide on understanding and resolving common error codes in Zhang Accounting.
---

# Error Code Guide

This guide provides comprehensive explanations and solutions for common error codes encountered in Zhang Accounting. Understanding these error codes will help you troubleshoot and resolve issues more efficiently.

## UnbalancedTransaction

This error indicates that a transaction is unbalanced, meaning the sum of each posting does not equal zero. This is often due to discrepancies in the amounts or currencies used in the postings.

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

**Solution:** Verify and correct all transactions affecting the account to ensure the balance check aligns with the actual account balance.

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
1970-01-01 * "Payee" "Transaction for defined account"
    Assets:DefinedAccount  -100 USD
    Expenses:Misc  100 USD
```

**Solution:** Define the account using the `open` directive before referencing it in transactions or other operations.

## AccountClosed

Occurs when attempting to perform operations on a closed account. Ensure the account is open or reopen it before performing transactions.

**Example of Error:**
```zhang
1970-01-01 close Assets:ClosedAccount
1970-01-02 * "Payee" "Transaction for closed account"
    Assets:ClosedAccount  -100 USD
    Expenses:Misc  100 USD
```

**Correct Case:**
```zhang
1970-01-01 open Assets:ReopenedAccount
1970-01-02 * "Payee" "Transaction for reopened account"
    Assets:ReopenedAccount  -100 USD
    Expenses:Misc  100 USD
```

**Solution:** Reopen the account using the `open` directive if necessary before conducting transactions.

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
