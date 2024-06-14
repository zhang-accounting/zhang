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

## AccountDoesNotExist

## AccountClosed

## CommodityDoesNotDefine

## NoEnoughCommodityLot

## CloseNonZeroAccount

## BudgetDoesNotExist

## DefineDuplicatedBudget

## MultipleOperatingCurrencyDetect

## ParseInvalidMeta