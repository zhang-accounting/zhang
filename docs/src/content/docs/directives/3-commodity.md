---
title: Commodity
description: A comprehensive guide on using commodity directives in Zhang Accounting.
---

# Commodity Directives

The commodity directive is used to define new commodities (currencies, stocks, cryptocurrencies, etc.) in your accounting system. This guide covers the syntax, meta configurations, and provides practical examples.

## Basic Syntax

To define a new commodity, use the following syntax:

```zhang
{DATE} commodity {COMMODITY_NAME}
```

## Meta Configurations

### Precision

Specifies the decimal precision to be preserved for the commodity.

```zhang
1970-01-01 commodity CNY
  precision: 2
```

Default value: `2` (common for most currencies)

### Prefix

Defines a label that appears in front of the decimal value.

```zhang
1970-01-01 commodity USD
  prefix: "$"
```

### Suffix

Specifies a label that appears behind the decimal value.

```zhang
1970-01-01 commodity BTC
  suffix: "BTC"
```

Default value: Commodity name (if not specified)

### Rounding

Determines the decimal place to which values are rounded.

Available options:
- `RoundUp` (default): Rounds down for values 0-4
- `RoundDown`: Rounds up for values 5-9

```zhang
1970-01-01 commodity BTC
  rounding: "RoundDown"
```

### Group

Categorizes commodities into different groups for organizational purposes.

```zhang
1970-01-01 commodity CNY
  group: "Fiat currencies"

1970-01-01 commodity BTC
  group: "Crypto currencies"
```

## Common Use Cases

### Fiat Currencies

```zhang
1970-01-01 commodity USD
  prefix: "$"
  precision: 2
  group: "Fiat currencies"

1970-01-01 commodity EUR
  prefix: "€"
  precision: 2
  group: "Fiat currencies"

1970-01-01 commodity JPY
  prefix: "¥"
  precision: 0
  group: "Fiat currencies"
```

### Cryptocurrencies

```zhang
1970-01-01 commodity BTC
  suffix: "BTC"
  precision: 8
  rounding: "RoundDown"
  group: "Crypto currencies"

1970-01-01 commodity ETH
  suffix: "ETH"
  precision: 8
  rounding: "RoundDown"
  group: "Crypto currencies"
```

### Stocks

```zhang
1970-01-01 commodity AAPL
  precision: 2
  group: "Stocks"

1970-01-01 commodity GOOGL
  precision: 2
  group: "Stocks"
```

## Best Practices

1. **Naming Conventions**
   - Use uppercase for commodity names (e.g., `USD`, `BTC`, `AAPL`)
   - Avoid special characters
   - Use consistent naming across your ledger

2. **Precision Settings**
   - Use `2` for most fiat currencies
   - Use `0` for currencies like JPY
   - Use `8` for cryptocurrencies
   - Use `2` for stocks

3. **Group Organization**
   - Group related commodities together
   - Use clear, descriptive group names
   - Keep groups consistent across your ledger

## Beancount Compatibility

Zhang Accounting is fully compatible with Beancount's commodity directives. The syntax is identical:

```beancount
1970-01-01 commodity USD
```

## Examples

### Complete Commodity Setup

```zhang
; Fiat Currencies
1970-01-01 commodity USD
  prefix: "$"
  precision: 2
  group: "Fiat currencies"

1970-01-01 commodity EUR
  prefix: "€"
  precision: 2
  group: "Fiat currencies"

1970-01-01 commodity JPY
  prefix: "¥"
  precision: 0
  group: "Fiat currencies"

; Cryptocurrencies
1970-01-01 commodity BTC
  suffix: "BTC"
  precision: 8
  rounding: "RoundDown"
  group: "Crypto currencies"

1970-01-01 commodity ETH
  suffix: "ETH"
  precision: 8
  rounding: "RoundDown"
  group: "Crypto currencies"

; Stocks
1970-01-01 commodity AAPL
  precision: 2
  group: "Stocks"

1970-01-01 commodity GOOGL
  precision: 2
  group: "Stocks"
```
