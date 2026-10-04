---
title: Price
description: Reference for the price directive, which records the price of a commodity on a date.
sidebar:
  order: 6
---

A `price` directive records what one unit of a commodity is worth in another commodity on a date. Zhang uses prices
only to value holdings: in the totals of the web UI and in the valuation functions of queries. A price never changes
a balance and is not used to check transactions.

## Syntax

```text
YYYY-MM-DD [HH:MM[:SS]] price <Commodity> <Number> <QuoteCommodity>
```

| Part | Required | Description |
|---|---|---|
| Date and time | yes | The date the price applies from, optionally with a time of day. |
| `<Commodity>` | yes | The commodity being priced, such as `USD` or `AAPL`. |
| `<Number>` | yes | The price of one unit. It can be an expression. |
| `<QuoteCommodity>` | yes | The commodity the price is written in. |

Metadata lines can follow the directive.

## Examples

```zhang
option "operating_currency" "CNY"

2024-01-01 commodity USD
2024-01-01 commodity AAPL

2024-01-02 price USD 7.10 CNY
2024-01-02 price AAPL 185.64 USD
2024-01-03 price USD 7.12 CNY
```

## Behavior

- Both commodities must be defined on or before the price's date. Otherwise the price is reported as
  [`CommodityDoesNotDefine`](/reference/error-codes/#commoditydoesnotdefine), and still recorded.
- The price on a date is the latest `price` of the pair dated on or before it. Of several prices of a pair at the same
  date and time, the last one in the ledger wins.
- Only `price` directives are prices. A price written on a posting (`@` or `@@`) converts that posting and nothing
  else: it does not add a price.

### In the web UI

The web UI shows totals in the [operating currency](/reference/directives/options/#operating_currency): on the account
list, and in the figures of the home and report pages.

- An amount in the operating currency counts as it is.
- The figures and charts of the home and report pages convert with the query language's
  [`convert`](/reference/query-language/#valuation-functions): the latest price dated on or before the date of the
  figure (the end of the period, or a chart point's last day), in either direction, and through the cost currency
  for a holding at cost without a price of its own. Their [built-in queries](/reference/builtin-queries/#report)
  show how.
- The account list converts an amount in another commodity with the latest price of **that commodity in the
  operating currency**, dated today or before. Only a price of that exact pair counts there. The opposite pair is
  not inverted, so `price CNY 0.14 USD` does not value USD in CNY, and no conversion goes through a third commodity.
- An amount that no price converts is left out of the total. The amounts per commodity still show it.
- The commodities page shows each commodity's latest price in the operating currency: the latest one in the ledger,
  even if it is dated in the future. A commodity's own page lists all its prices.

### In queries

The query functions `convert`, `value` and `getprice` read the same `price` directives with broader rules: a pair
without prices of its own uses the inverse of the opposite pair, a pair quoted in both directions has one merged
history, and a commodity's price in itself is 1. See
[Valuation functions](/reference/query-language/#valuation-functions).

So the account list and queries can value the same holding differently. With only the price below, a query, and
the home and report pages, convert 10 EUR into 80 CNY, while the account list leaves the EUR out of its CNY totals:

```zhang
option "operating_currency" "CNY"

2024-01-01 commodity EUR
2024-01-02 price CNY 0.125 EUR
```

To have the account list value a commodity, write its prices in the operating currency.

Plugins do not receive precomputed prices. A plugin that needs rates builds them from the `price` directives in the
stream; see [Exchange rates](/developers/writing-plugins/#exchange-rates).

## Errors

| Error | When |
|---|---|
| [`CommodityDoesNotDefine`](/reference/error-codes/#commoditydoesnotdefine) | One of the two commodities is not defined at the price's date. |

## Beancount compatibility

The syntax is the same as Beancount's. In a Beancount file, a time of day is written as `time: "HH:MM:SS"` metadata.
Beancount's `implicit_prices` plugin, which turns the prices of postings into price entries, does not run in Zhang;
the [query functions](/reference/query-language/#valuation-functions) only use `price` directives.

## Related

- [Commodity](/reference/directives/commodity/): defining the commodities a price names.
- [Querying](/guides/querying/): valuing holdings with queries.
