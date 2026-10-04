---
title: Commodity
description: Reference for the commodity directive and the metadata Zhang reads from it, such as precision, rounding, prefix and suffix.
sidebar:
  order: 3
---

A `commodity` directive defines a commodity: a currency, a stock, a cryptocurrency or anything else you count. Its
metadata sets how Zhang displays and rounds amounts of it. A transaction, price or `open` that uses a commodity needs
it to be defined first.

## Syntax

```text
YYYY-MM-DD [HH:MM[:SS]] commodity <Name>
  [<key>: <value>]
```

| Part | Required | Description |
|---|---|---|
| Date and time | yes | The date the commodity is defined from. |
| `<Name>` | yes | An ASCII letter, followed by letters, digits, `.`, `_`, `-` or `'`, such as `USD`, `AAPL` or `VBMPX`. |
| `<key>: <value>` | no | Metadata lines. See [Metadata](#metadata). |

## Examples

```zhang
1970-01-01 commodity CNY
  prefix: "¥"
  group: "Fiat currencies"

1970-01-01 commodity JPY
  precision: 0
  prefix: "¥"
  group: "Fiat currencies"

1970-01-01 commodity BTC
  precision: 8
  suffix: " BTC"
  rounding: "RoundUp"
  group: "Crypto currencies"
```

## Metadata

| Key | Value | Default |
|---|---|---|
| [`precision`](#precision) | a whole number of decimals | the [`default_commodity_precision`](/reference/directives/options/#default_commodity_precision) option, else `2` |
| [`rounding`](#rounding) | `RoundDown` or `RoundUp` | the [`default_rounding`](/reference/directives/options/#default_rounding) option, else `RoundDown` |
| `prefix` | text shown before the number in the web UI, such as `$` | none |
| `suffix` | text shown after the number in the web UI | none |
| `group` | the heading the commodities page lists the commodity under | none: the commodity is listed in the default group |

Without a `prefix` and a `suffix`, the web UI shows the commodity's name after the number. Other metadata is kept;
queries read it from `#commodities`.

### Precision

The number of decimals of the commodity. It is used:

- by the web UI, to show amounts of the commodity with that many decimals;
- to check that a transaction balances: the sum of a transaction's weights in the commodity is rounded to the
  precision, with the commodity's rounding, and must then be zero. With precision 2, a transaction off by `0.004`
  balances, one off by `0.006` does not. See [Transaction](/reference/directives/transaction/#how-a-transaction-balances);
- to round an amount Zhang infers for a posting when a division leaves it with more than 20 decimals, such as a total
  cost spread over 3 units.

A value that is not a whole number is ignored, and the default is used.

### Rounding

How amounts of the commodity are rounded to its precision, in the cases above:

- `RoundDown`: a 5 in the first dropped decimal rounds down, so `0.005` rounds to `0.00` at precision 2.
- `RoundUp`: a 5 in the first dropped decimal rounds up, so `0.005` rounds to `0.01` at precision 2.

Other digits round to the nearest value in both modes. The value is case-sensitive: any other value stops the ledger
from loading with the message `option value is invalid`. The web UI rounds the amounts it displays on its own, half
up.

## Behavior

- A commodity is defined from its date on. A transaction, `price` or `open` dated before the definition reports
  [`CommodityDoesNotDefine`](/reference/error-codes/#commoditydoesnotdefine). Within one date, `open` and `commodity`
  keep their file order: write the `commodity` above an `open` that lists it.
- For a transaction, Zhang checks the commodities it balances in: the units of a plain posting, the price commodity
  of a posting with `@`, the cost commodity of a posting with a cost.
- The [operating currency](/reference/directives/options/#operating_currency) is defined by its option, with
  precision 2 by default. A `commodity` directive for it replaces that definition, for example to give it a prefix.
- A second `commodity` directive for the same name replaces the first one entirely: metadata it leaves out goes back
  to the default.
- The commodities page of the web UI lists every commodity by group, with the total held in `Assets` and
  `Liabilities` accounts and its latest price in the operating currency.

## Errors

A `commodity` directive produces no ledger error. An invalid `rounding` stops the ledger from loading.

## Beancount compatibility

The directive has the same syntax in Beancount. Zhang reads the metadata above; metadata written for other tools is
kept and has no effect in Zhang. Zhang also accepts names that Beancount rejects, such as names with lowercase
letters.

## Related

- [Price](/reference/directives/price/): prices between commodities.
- [Options](/reference/directives/options/): the defaults for precision and rounding.
