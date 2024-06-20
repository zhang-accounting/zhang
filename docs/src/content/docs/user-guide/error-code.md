---
title: Error Code
---

To explain errors

## UnbalancedTransaction

Usually, Zhang cannot determine whether a transaction is balanced, whether the sum of each posting is zero.

```zhang {2-3}
1970-01-01 "" ""
    Assets:A -10 USD
    Assets:B  10 CNY
```

## TransactionCannotInferTradeAmount

This error occurs when a transaction's trade amount cannot be inferred due to missing or ambiguous information. Ensure all postings in a transaction specify an amount or that the transaction's context allows for an amount to be inferred.

**Error Case:**
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

## TransactionHasMultipleImplicitPosting

To ensure normal operation, Zhang is similar to Beancount, only allowing one implicit posting:

```zhang {3-4}
1970-01-01 "" ""
    Assets:A -10 USD
    Assets:B
    Assets:C 
```

## TransactionExplicitPostingHaveMultipleCommodity

After calculating, we find multiple non-zero commodity amounts across postings, making it impossible to infer implicit
postings' amount - a common issue in transactions with three or more postings.

```zhang {2-3}
1970-01-01 "" ""
Assets:A -10 USD
Assets:B 10 CNY
Assets:C
```

## AccountBalanceCheckError

This error occurs when an account's balance check fails, possibly due to incorrect balance entries. Ensure all transactions affecting the account are correctly entered and balanced.

**Error Case:**
```zhang
// given Assets:Checking owns 100 USD

1970-01-01 balance Assets:Checking  500 USD
```

**Correct Case:**
```zhang

// given Assets:Checking owns 100 USD

1970-01-01 balance Assets:Checking  100 USD
```

## AccountDoesNotExist

This error occurs when operations are performed on an account that has not been defined. Ensure the account is defined before referencing it in transactions or directives.

**Error Case:**
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

## AccountClosed

This error occurs when trying to perform operations on a closed account. Ensure the account is open or reopen the account before performing transactions.

**Error Case:**
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

## CommodityDoesNotDefine

This error occurs when a commodity used in a transaction or directive is not defined. Ensure all commodities are defined before use.

**Error Case:**
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

## NoEnoughCommodityLot

This error occurs when there's not enough commodity lot for a transaction. Ensure the commodity lots are sufficient for the transaction.

**Error Case:**
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

## CloseNonZeroAccount

This error occurs when trying to close an account with a non-zero balance. Ensure the account balance is zero before closing.

**Error Case:**
```zhang

// given Assets:NonZeroBalanceAccount owns 100 USD

1970-01-01 close Assets:NonZeroBalanceAccount
```

**Correct Case:**
```zhang

1970-01-01 balance Assets:ZeroBalanceAccount  0 USD
1970-01-02 close Assets:ZeroBalanceAccount
```

## BudgetDoesNotExist

This error occurs when referencing a budget that has not been defined. Ensure the budget is defined before referencing it in transactions or directives.

**Error Case:**
```zhang
1970-01-01 budget-add NonExistentBudget  500 USD
```

**Correct Case:**
```zhang
1970-01-01 budget ExistingBudget USD
1970-01-02 budget-add ExistingBudget  500 USD
```

## DefineDuplicatedBudget

This error occurs when a budget is defined more than once. Ensure each budget is uniquely defined.

**Error Case:**
```zhang
1970-01-01 budget DuplicateBudget USD
1970-01-02 budget DuplicateBudget USD
```

**Correct Case:**
```zhang
1970-01-01 budget UniqueBudget USD
```

## MultipleOperatingCurrencyDetect

This error occurs when multiple operating currencies are detected, which is not allowed. Ensure only one operating currency is defined.

**Error Case:**
```zhang
option "operating_currency" "USD"
option "operating_currency" "EUR"
```

**Correct Case:**
```zhang
option "operating_currency" "USD"
```

## ParseInvalidMeta

This error occurs when parsing invalid metadata in directives. Ensure the metadata is correctly formatted.

**Error Case:**
```zhang
1970-01-01 open Assets:Cash
    booking_method: "NON_EXIST"
```

**Correct Case:**
```zhang
1970-01-01 open Assets:Cash
    booking_method: "FIFO"
```
