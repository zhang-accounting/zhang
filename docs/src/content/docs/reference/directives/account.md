---
title: Account
description: Reference for the open and close directives, which start and end the life of an account.
sidebar:
  order: 2
---

An `open` directive creates an account and makes it usable from its date. A `close` directive ends its use. Every
account a transaction posts to must be opened first.

## Syntax

```text
YYYY-MM-DD [HH:MM[:SS]] open <Account> [<Commodity>[, <Commodity> …]]
  [<key>: <value>]
YYYY-MM-DD [HH:MM[:SS]] close <Account>
```

| Part | Required | Description |
|---|---|---|
| Date and time | yes | When the account opens or closes, optionally with a time of day. |
| `<Account>` | yes | The account name, such as `Assets:Bank:Checking`. |
| `<Commodity>, …` | no | The only commodities the account may hold, separated by commas. Each must be defined. Leave it out to allow any commodity. |
| `<key>: <value>` | no | Metadata lines. See [Metadata](#metadata) for the keys Zhang reads. |

**Account names** start with one of the five account types, `Assets`, `Liabilities`, `Equity`, `Income` or
`Expenses`, followed by one or more parts separated by `:`. A part is any text without spaces, quotes, colons,
parentheses or commas, so `Expenses:Food:餐饮` is a valid name.

## Examples

```zhang
2024-01-01 commodity USD
2024-01-01 budget Food CNY

2024-01-01 open Assets:Bank:Checking CNY
  alias: "Main checking"
2024-01-01 open Assets:Broker USD
  booking_method: "STRICT"
2024-01-01 open Expenses:Food CNY
  budget: Food
2024-01-01 open Equity:Opening-Balances

2024-12-31 close Assets:Broker
```

## Metadata

| Key | Effect |
|---|---|
| `alias` | A display name for the account. The account list shows it, with the account's full name under it. |
| `booking_method` | The booking method of the account: see [Booking method](#booking-method). |
| `budget` | Links the account to a [budget](/reference/directives/budget/#linking-accounts): its postings count as the budget's activity. Repeat the key to link several budgets. |

Other metadata is kept. In queries, `#accounts` gives the metadata of the `open` directive as `open.meta`.

### Booking method

The booking method decides which lot a reduction, for example `-5 AAPL {}`, takes its units from. Without the
metadata, the account uses the [`default_booking_method`](/reference/directives/options/#default_booking_method)
option (`FIFO` unless set).

Like Beancount, a reduction matches the lots its cost names, and what the cost leaves out matches anything:
`{100 USD}` matches the lots held at 100 USD whatever their acquisition date, `{100 USD, 2024-01-01}` only the lot
acquired on that date, and `{}` every lot held at cost. A lot's acquisition date is the date in its cost, or else the
date of the transaction that opened it.

- `STRICT`: Beancount's default. A reduction must match a single lot, or reduce every lot it matches in full.
  Otherwise the ledger reports an [`AmbiguousLotMatch`](/reference/error-codes/#ambiguouslotmatch) error and books the
  reduction like `FIFO` among the matching lots.
- `FIFO`: first in, first out: the matching lot with the oldest acquisition date first.
- `LIFO`: last in, first out: the matching lot with the newest acquisition date first.

`AVERAGE`, `AVERAGE_ONLY` and `NONE` are not implemented yet. An account using one of them, or a value that is not a
booking method, gets an error on its `open` directive
([`UnsupportedBookingMethod`](/reference/error-codes/#unsupportedbookingmethod) or
[`ParseInvalidMeta`](/reference/error-codes/#parseinvalidmeta)) and books with the default booking method. The ledger
still loads. See [Lots and cost basis](/guides/lots-and-cost-basis/) for working with lots.

## Behavior

### When an account is active

One rule decides whether an account is active, for every directive that uses it and for the web UI:

- An account is active from its `open`. Within a date and time, `open` comes before every other directive, so a
  transaction on the day of the `open` is fine.
- It stays active through its `close`. A `close` with only a date closes the account at the end of that day (24:00),
  so everything dated that day may still use it. In a zhang file, a `close` with a time closes it at that time: a
  directive at that time may still use it, and one later that day may not. In a Beancount file, the `time` metadata
  of a `close` is plain metadata, as in Beancount: the account stays active through the whole day.
- An `open` after a `close` opens the account again.
- A directive that uses an account that was never opened, or that is opened only later, reports
  [`AccountDoesNotExist`](/reference/error-codes/#accountdoesnotexist).
- What books on the account after its close reports [`AccountClosed`](/reference/error-codes/#accountclosed): a
  posting, a [`pad`, or a `balance … with pad`](/reference/directives/balance/).
- What only records may follow the close, as in Beancount: a plain `balance`, such as one asserting that the closed
  account is empty, a [`document`](/reference/directives/document/), such as a final statement, and a
  [`note`](/reference/directives/note-and-event/).
- Each error is reported once per account and directive, and the directive still counts: the transaction is booked
  and the document is listed.
- Opening an account does not open its parent. `Assets:Bank:Checking` can be used without opening `Assets:Bank`, but a
  [balance assertion](/reference/directives/balance/) on `Assets:Bank` needs `Assets:Bank` to be open.

```zhang
2024-01-01 open Assets:Wallet
2024-03-31 close Assets:Wallet
2024-03-31 18:00 * "Last coffee"     ; fine: the account is active through March 31
  Assets:Wallet -3 CNY
  Expenses:Coffee
2024-04-01 * "Too late"              ; AccountClosed
  Assets:Wallet -3 CNY
  Expenses:Coffee
2024-04-02 balance Assets:Wallet -6 CNY  ; fine: a balance only records
```

The account list shows an account as closed from the moment its close takes effect, by the ledger's clock, and as open
again after a later `open`.

### Commodities

The commodities listed on `open` must be [defined](/reference/directives/commodity/) before it; an undefined one
reports [`CommodityDoesNotDefine`](/reference/error-codes/#commoditydoesnotdefine) on the `open`. On the same date,
write the `commodity` directive above the `open`. Queries read the list as `open.currencies` in `#accounts`.

As in Beancount, a list restricts the account to the commodities in it: a posting, a
[balance assertion](/reference/directives/balance/) or a padding in another commodity reports
[`CommodityNotAllowed`](/reference/error-codes/#commoditynotallowed) once for each posting as written, with the
`account_name` and `commodity` metas. The ledger still loads and the transaction is still booked.

- Only the units of a posting are checked, not its cost or its price, so an account opened with `AAPL` can buy
  `AAPL {90 EUR}`.
- An `open` without a list allows any commodity. Only the account itself is restricted, not its sub-accounts.
- An account that is opened again follows the list of its latest `open` before the directive.

### Closing

- `close` checks the account's own balance in every commodity, without its sub-accounts. A balance that is not zero
  reports [`CloseNonZeroAccount`](/reference/error-codes/#closenonzeroaccount). The account is closed anyway.
- Closing an account that was never opened reports `AccountDoesNotExist`, and closing a closed account reports
  `AccountClosed`. The first `close` stands.
- A closed account keeps its balances and history. The account list marks it as closed and can hide it.
- A plain `balance`, a [`document`](/reference/directives/document/) and a
  [`note`](/reference/directives/note-and-event/) may follow the `close` without an error.

## Errors

| Error | When |
|---|---|
| [`CommodityDoesNotDefine`](/reference/error-codes/#commoditydoesnotdefine) | A commodity listed on `open` is not defined. |
| [`CommodityNotAllowed`](/reference/error-codes/#commoditynotallowed) | A posting, balance assertion or padding is in a commodity the account's `open` does not list. |
| [`ParseInvalidMeta`](/reference/error-codes/#parseinvalidmeta) | `booking_method` is not a booking method. |
| [`UnsupportedBookingMethod`](/reference/error-codes/#unsupportedbookingmethod) | `booking_method` is `AVERAGE`, `AVERAGE_ONLY` or `NONE`. |
| [`CloseNonZeroAccount`](/reference/error-codes/#closenonzeroaccount) | The account holds something when it is closed. |
| [`AccountDoesNotExist`](/reference/error-codes/#accountdoesnotexist) | A directive uses an account that is not open yet, or closes one that was never opened. |
| [`AccountClosed`](/reference/error-codes/#accountclosed) | A posting, a `pad` or a `balance … with pad` uses an account after its close, or a closed account is closed again. |

## Beancount compatibility

- Beancount writes the booking method as a quoted string after the commodities. Zhang reads it in a Beancount file
  and stores it as the `booking_method` metadata. In a zhang file, write the metadata instead: a string after the
  commodities is a syntax error.

  ```beancount
  2024-01-01 open Assets:Broker USD "FIFO"
  ```

- Both report a posting in a commodity that the `open` does not list, and a balance assertion in one. For a sale booked
  against several lots, Beancount reports one error for each lot, and Zhang one for the posting as written. Zhang lets
  an account be opened again, with the list of its latest `open`; Beancount reports the second `open` as an error.
- Beancount's default booking method is `STRICT`; Zhang's is `FIFO`.
- `CloseNonZeroAccount` is Zhang's own check: Beancount closes such an account without an error.
- Both keep an account active through the day of a `close`: Beancount sorts a `close` after everything else of its
  day. Beancount knows no times, and in a Beancount file Zhang keeps the `time` metadata of a `close` as plain
  metadata too.
- Both accept a `balance`, a `document` and a `note` after the `close`, and report a posting or a `pad`.

## Related

- [Balance](/reference/directives/balance/): balance assertions and pads.
- [Lots and cost basis](/guides/lots-and-cost-basis/): booking methods in practice.
- [Budget](/reference/directives/budget/): linking expense accounts to budgets.
