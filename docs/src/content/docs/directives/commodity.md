---
title: Understanding the Commodity Directive
description: A detailed guide on the commodity directive, including examples and use cases.
---

The commodity directive plays a crucial role in Zhang Accounting by defining new commodities. This guide aims to provide a comprehensive understanding of the commodity directive, its significance, and how to effectively utilize it with examples and use cases.

## Introduction to the Commodity Directive

To define a new commodity, the commodity directive is used as follows:

```zhang
{DATE} commodity {COMMODITY_NAME}
```

This directive allows for the introduction of new commodities into the accounting system, which can then be used in transactions and other directives.

## Supported Meta Configurations

The commodity directive supports several meta configurations that provide additional details about the commodity. Understanding these configurations is essential for accurate and effective commodity management.

### Precision

The `precision` meta configuration specifies the decimal precision to be preserved for the commodity. It is particularly important for currencies and financial instruments where precision is critical.

- Default value: `2` (common for most currencies)
- Example:

```zhang {2}
1970-01-01 commodity CNY
  precision: 2
```

### Prefix

The `prefix` meta configuration defines a label that appears in front of the decimal value. This is commonly used to denote currency symbols.

- Example:

```zhang {2}
1970-01-01 commodity USD
  prefix: "$"
```

### Suffix

The `suffix` meta configuration specifies a label that appears behind the decimal value. It is useful for commodities like cryptocurrencies where a suffix is preferred over a prefix.

- Default value: Commodity name (if not specified)
- Example:

```zhang {2}
1970-01-01 commodity MY_BTC
  suffix: "BTC"
```

### Rounding

The `rounding` meta configuration determines the decimal place to which values are rounded. It supports two options:

- `RoundUp` (default): Rounds down for values 0-4.
- `RoundDown`: Rounds up for values 5-9.

- Example:

```zhang {2}
1970-01-01 commodity MY_BTC
  rounding: "RoundDown"
```

### Group

The `group` meta configuration is used to categorize commodities into different groups, such as "Fiat currencies" and "Crypto currencies". This categorization is primarily used for organizational purposes in the frontend.

- Example:

```zhang {2,5,8,11}
1970-01-01 commodity CNY
  group: "Fiat currencies"

1970-01-01 commodity USD
  group: "Fiat currencies"

1970-01-01 commodity BTC
  group: "Crypto currencies"

1970-01-01 commodity ETH
  group: "Crypto currencies"
```

## Use Cases and Examples

The commodity directive's flexibility allows for a wide range of applications, from defining traditional currencies to incorporating modern cryptocurrencies into your accounting system. By leveraging the supported meta configurations, users can tailor the directive to meet their specific needs, ensuring accurate and efficient financial tracking and reporting.
