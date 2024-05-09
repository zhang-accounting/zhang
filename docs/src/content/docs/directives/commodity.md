---
title: Commodity
description: commodity directive
---

commodity directive is to define a new commodity.

```zhang
{DATE} commodity {COMMODITY_NAME}
```

## Supported Meta

### precision

precision is to tell zhang know how much decimal precision should be preserved.

the value is `2` for common currency, and `2` also is the default value if not present.

```zhang {2}
1970-01-01 commodity CNY
  precision: 2
```

### prefix

prefix is the label showed at front of the decimal value. like we will define the currency symbol for common currency

```zhang {2}
1970-01-01 commodity USD
  prefix: "$"
```

### suffix

suffix is the label showed behind the decimal value. the default value will be the commodity name if not present. it's
useful for crypto coins.

```zhang {2}
1970-01-01 commodity MY_BTC
  suffix: "BTC"
```

### rounding

rounding is to decide what decimal place we are rounding to. there are two options:

- `RoundUp` **default** If it is 0, 1, 2, 3, or 4, we keep our last digit the same.
- `RoundDown` If it is 5, 6, 7, 8, or 9, we increase our last digit by 1

```zhang {2}
1970-01-01 commodity MY_BTC
  rounding: "RoundDown"
```

### group

group is the flag used to organize commodities into different groups, like we can organize them into Fiat currencies and
Crypto currencies. it's only used in frontend.

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