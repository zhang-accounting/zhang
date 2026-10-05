---
title: Lots and Cost Basis
description: "Track investments with costs and prices: lot syntax, booking methods, realized gains and rounding."
sidebar:
  order: 3
---

When you buy shares, fund units or a cryptocurrency, you hold units of one commodity (`AAPL`) that cost you an amount of another (`USD`). Zhang keeps each purchase as a **lot**: the units, their cost per unit and the date you acquired them. When you sell, Zhang takes the units from your lots, so it knows the cost of what you sold and the gain you realized.

This guide follows a brokerage account through three purchases and a sale.

## Set up

```zhang
option "operating_currency" "USD"

1970-01-01 commodity AAPL

2024-01-01 open Assets:Broker:Cash USD
2024-01-01 open Assets:Broker:AAPL AAPL
2024-01-01 open Income:Broker:Gains USD
2024-01-01 open Expenses:Broker:Fees USD
2024-01-01 open Equity:Opening-Balances

2024-01-02 balance Assets:Broker:Cash 10000 USD with pad Equity:Opening-Balances
```

Every commodity you hold must be declared with [`commodity`](/reference/directives/commodity/). The operating currency, here `USD`, is declared by the option.

## Buy at a cost

Write the cost of the units in braces:

```zhang
2024-01-10 * "Buy 10 AAPL"
  Assets:Broker:AAPL 10 AAPL {185.00 USD}
  Assets:Broker:Cash -1850.00 USD

2024-03-15 * "Buy 5 AAPL for 860 USD in total"
  Assets:Broker:AAPL 5 AAPL {{860.00 USD}}
  Assets:Broker:Cash

2024-05-20 * "Buy 5 AAPL"
  Assets:Broker:AAPL 5 AAPL {190.00 USD}
  Assets:Broker:Cash -950.00 USD
```

- `{185.00 USD}` is the cost of one unit. `{{860.00 USD}}` is the cost of all the units of the posting: Zhang keeps the lot at 860 / 5 = 172.00 USD per unit. A compound cost, `{185.00 # 5.00 USD}`, is a cost per unit plus a total, such as a commission: for 10 units Zhang keeps the lot at 185.00 + 5.00 / 10 = 185.50 USD per unit, as Beancount does.
- A posting with a cost weighs its units times the cost, `10 × 185.00 = 1,850.00 USD`, and the transaction balances in USD.
- The lot is acquired on the date of the transaction. To give it another date, add it after the cost: `{185.00 USD, 2024-01-09}`. A label can follow too, `{185.00 USD, 2024-01-09, "first"}`. The parts of a cost come in any order, and each can stand alone: `{2024-01-09}` or `{"first"}` is a cost too.
- An empty `{}` on a purchase infers the cost from the other postings' amounts and opens a lot on the purchase date. For example, `3 AAPL {}` balanced by `-600 USD` has a cost of `200 USD` per share. Only one number can be missing: combining an unspecified cost with an implicit cash amount, or with another unspecified cost, is rejected with [`TransactionCannotInferTradeAmount`](/reference/error-codes/#transactioncannotinfertradeamount).

The account now holds three lots: 10 AAPL at 185.00 USD, 5 at 172.00 and 5 at 190.00.

## Prices are not costs

`@` gives the price of one unit, and `@@` the price of all the units. A posting with a price weighs its units converted at that price, but **no lot is kept**: the units are held without a cost. That is what you want for a currency exchange:

```zhang
2024-02-01 * "Exchange"
  Assets:Bank:USD 100 USD @ 7.20 CNY
  Assets:Bank:CNY -720.00 CNY
```

On a posting that also has a cost, the price is only information, such as the price you sold at: the posting weighs at its cost. Neither `@` nor `@@` adds to a commodity's price history. Record market prices with [`price`](/reference/directives/price/) directives:

```zhang
2024-08-01 price AAPL 220.00 USD
```

## Sell and realize a gain

Sell with an empty cost, `{}`. Zhang picks the lots by the account's [booking method](#choose-a-booking-method), `FIFO` by default:

```zhang
2024-08-01 * "Sell 12 AAPL"
  Assets:Broker:AAPL -12 AAPL {} @ 220.00 USD
  Assets:Broker:Cash 2639.00 USD
  Expenses:Broker:Fees 1.00 USD
  Income:Broker:Gains
```

- With `FIFO` the sale takes the oldest lots first: all 10 AAPL at 185.00 and 2 of the 5 at 172.00. The sold units weigh what they cost, `10 × 185.00 + 2 × 172.00 = 2,194.00 USD`. The price `@ 220.00 USD` is not part of the weight.
- The cash you received and the fee add up to 2,640.00 USD. The difference with the cost, 446.00 USD, is the realized gain. Leave the income posting without an amount and Zhang fills in `-446.00 USD`. You can also write the gain yourself; the transaction must then balance.
- The account keeps 3 AAPL at 172.00 USD and 5 at 190.00 USD.

To sell from particular lots, write their cost instead of `{}`:

- `{172.00 USD}` takes from the lots held at 172.00 USD, whatever their date.
- `{172.00 USD, 2024-03-15}` takes only from the lot acquired on 15 March 2024.
- `{2024-03-15}` alone takes only from the lots acquired on 15 March 2024, whatever their cost.
- `{172.00 USD, "first"}`, or `{"first"}` without the cost (Zhang also reads the older `{, "first"}`), takes only from the lot labelled `first`. A label written on a purchase, such as `{172.00 USD, "first"}`, names its lot: lots that differ only by label are kept apart, and a purchase without a label never adds to a labelled lot.

Beancount's merge-cost marker, `{*}`, is read but not supported: the sale is reported with [`CostMergingNotSupported`](/reference/error-codes/#costmergingnotsupported) and books like `{}`.

### Selling more than you hold

A sale written with `{}` must be covered by matching cost lots. Otherwise Zhang reports [`NoEnoughCommodityLot`](/reference/error-codes/#noenoughcommoditylot) and [`TransactionCannotInferTradeAmount`](/reference/error-codes/#transactioncannotinfertradeamount), leaves the entire transaction out of the ledger and keeps the previous holdings. This also applies when the account only holds units without cost.

If you write an explicit cost on a sale that exceeds the matching lots, Zhang reports `NoEnoughCommodityLot` and keeps the remainder as a short lot at that cost. A later positive posting with `{}` can cover that short and retains its cost basis.

## Choose a booking method

The booking method decides which lots a sale with `{}`, or with a cost that matches several lots, takes from:

| Method | Takes from |
|---|---|
| `FIFO` | the lot with the oldest acquisition date first. The default. |
| `LIFO` | the lot with the newest acquisition date first. |
| `STRICT` | a single matching lot. A sale matching several lots must take all of them in full; otherwise it is ambiguous. |

Lots with the same acquisition date go in the order they were created, reversed for `LIFO`.

Set the method of one account with `booking_method` on its `open`, or of the whole ledger with the `default_booking_method` option:

```zhang
option "default_booking_method" "LIFO"

2024-01-01 open Assets:Broker:Retirement AAPL
  booking_method: "STRICT"
```

In a beancount ledger, the method can also follow the commodities of the `open`: `2024-01-01 open Assets:Broker:AAPL AAPL "STRICT"`. `NONE`, `AVERAGE` and `AVERAGE_ONLY` are not implemented: an account or option using one of them gets an [`UnsupportedBookingMethod`](/reference/error-codes/#unsupportedbookingmethod) error and books with the default method. See [Booking method](/reference/directives/account/#booking-method) and [`default_booking_method`](/reference/directives/options/#default_booking_method).

### STRICT and ambiguous sales

On a `STRICT` account holding 10 AAPL at 185.00 USD and 5 at 172.00 USD, a sale of `-6 AAPL {}` matches both lots without taking all of them. Zhang reports an [`AmbiguousLotMatch`](/reference/error-codes/#ambiguouslotmatch) error that lists the matching lots:

```text
10 AAPL {185.00 USD, 2024-01-10}, 5 AAPL {172.00 USD, 2024-03-15}
```

It still books the sale like `FIFO`, so the numbers stay usable. Say which lot you mean, for example `-6 AAPL {185.00 USD}`, and the error goes away.

## Rounding

Zhang computes with exact decimals and keeps every digit you write. Amounts, costs and prices, and their sums, are never rounded. Neither are their products, unless one needs more than 28 significant digits, which Beancount rounds too.

Only a division can produce more digits, such as the per-unit cost of a total cost. Zhang divides as Beancount does, to 28 significant digits: `{{1000 USD}}` for 3 units is 333.3333333333333333333333333 USD per unit, the cost the lot keeps and queries show. When Zhang fills in a missing amount whose exact value has more than 20 decimals, or that comes from such a divided cost, it rounds that amount at the larger of the commodity's precision and the most decimals the transaction writes in that commodity, with the commodity's rounding:

```zhang
2024-05-16 * "Buy 3 AAPL for 1000 USD in total"
  Assets:Broker:AAPL 3 AAPL {{1000 USD}}
  Assets:Broker:Cash -1000 USD

2024-05-17 * "Sell one share at cost"
  Assets:Broker:AAPL -1 AAPL {}
  Assets:Broker:Cash

2024-05-18 * "Sell the other two at cost"
  Assets:Broker:AAPL -2 AAPL {}
  Assets:Broker:Cash
```

The cash postings become `333.33 USD` and `666.67 USD`: together exactly the 1,000 USD you paid.

A transaction balances when, in each commodity, the sum of its weights rounds to zero at the commodity's precision. The precision is the `precision` metadata of the [`commodity`](/reference/directives/commodity/#precision) directive, 2 decimals without one. So a commodity with finer amounts, such as a cryptocurrency, needs a `precision` of its own. [Balance assertions](/guides/balances/) are not rounded: they hold only on the exact amount, or within the `~` tolerance you write.

## See your lots

- The **Commodities** page lists every commodity. Open one, such as `AAPL`, to see its **Lots** tab: each lot held in an `Assets` or `Liabilities` account, with its account, acquisition date, cost and amount.
- In [queries](/guides/querying/), `position` shows a posting's units with their cost, `cost()` gives the book value and `value()` the market value at the latest `price`:

  ```sql
  SELECT account, sum(position) AS holding, cost(sum(position)) AS book_value, value(sum(position)) AS market_value
  WHERE account ~ '^Assets:Broker:AAPL'
  GROUP BY account
  ```

  After the sale above this gives one row for `Assets:Broker:AAPL`: the holding `3 AAPL {172.00 USD, 2024-03-15}` and `5 AAPL {190.00 USD, 2024-05-20}`, a book value of `1466.00 USD` and a market value of `1760.00 USD`.
