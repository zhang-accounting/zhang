---
title: Options
description: Every option Zhang reads, with its accepted values, default and effect.
sidebar:
  order: 1
---

An `option` directive sets a ledger-wide setting, such as the operating currency or the timezone. This page lists
every option Zhang reads.

## Syntax

```text
option "<Key>" "<Value>"
```

| Part | Required | Description |
|---|---|---|
| `"<Key>"` | yes | The name of the option. |
| `"<Value>"` | yes | Its value, always written as a string. |

An `option` has no date and no metadata. It can be written in any file of the ledger, at any position; the
convention is the top of the main file.

## Example

```zhang
option "title" "Family Ledger"
option "operating_currency" "USD"
option "timezone" "America/New_York"
option "default_booking_method" "FIFO"
option "directive_output_path" "data/{{year}}/{{month_str}}.{{ext}}"
```

## Behavior

- When a key is set more than once, the value read last wins. Zhang reads the main file first, then the files it
  [includes](/reference/directives/include/#files-read-once). `operating_currency` also reports an error when it is
  set twice.
- An option Zhang does not know is kept: the web UI's settings and the HTTP API (`GET /api/options`) list it, and
  [plugins](/reference/directives/plugin/#settings-the-plugin-receives) receive it as a setting. It has no other effect.
- Options are applied before any dated directive, wherever they are written.

## Options

| Key | Value | Default |
|---|---|---|
| [`title`](#title) | any text | none |
| [`operating_currency`](#operating_currency) | a commodity | `CNY` |
| [`timezone`](#timezone) | an IANA timezone name | the system's timezone |
| [`default_booking_method`](#default_booking_method) | `STRICT`, `FIFO` or `LIFO` | `FIFO` |
| [`default_commodity_precision`](#default_commodity_precision) | a whole number | `2` |
| [`default_rounding`](#default_rounding) | `RoundDown` or `RoundUp` | `RoundDown` |
| [`default_balance_tolerance_precision`](#default_balance_tolerance_precision) | a whole number | `2` |
| [`directive_output_path`](#directive_output_path) | a path template | `data/{{year}}/{{month_str}}.{{ext}}` |
| [`features.plugin`](#featuresplugin) | `true` or `false` | `false` |
| [`account_previous_balances` and five more](#accounting-periods-in-queries) | account names, a commodity | Beancount's |

### `title`

The name of the ledger. The web UI shows it in the sidebar and in the browser's tab title.

### `operating_currency`

The commodity the web UI shows totals in: account values, and the figures of the home and report pages. Amounts in
other commodities are converted with [prices](/reference/directives/price/#in-the-web-ui) into it.

- The option defines the commodity itself, so it needs no `commodity` directive. Its precision is the value of
  [`default_balance_tolerance_precision`](#default_balance_tolerance_precision) and its rounding the value of
  [`default_rounding`](#default_rounding), as set before this option. A
  [`commodity`](/reference/directives/commodity/) directive for it replaces that definition.
- Zhang supports a single operating currency. Setting the option a second time reports a
  [`MultipleOperatingCurrencyDetect`](/reference/error-codes/#multipleoperatingcurrencydetect) error on it; the last
  value is used, and every value is defined as a commodity.
- Without the option, the operating currency is `CNY`, and `CNY` is defined.

### `timezone`

The timezone of the ledger, an IANA name such as `Asia/Shanghai`, `Europe/London` or `UTC`.

- Dates and times in the ledger are read in this timezone: a date without a time is midnight there.
- Entries created in the web UI are dated with the current time in this timezone.
- Plugins read the current time in it, and `zhang serve` reloads a ledger that depends on the date at midnight in it.

Without the option, Zhang uses the system's timezone, or `Asia/Hong_Kong` if it cannot detect it. A name that is not a
valid timezone is ignored with a message in the server log, and the system's timezone is used.

### `default_booking_method`

The booking method of accounts whose `open` has no `booking_method` metadata: which lot a reduction such as
`-5 AAPL {}` takes its units from. The values are `STRICT`, `FIFO` and `LIFO`; see
[Booking method](/reference/directives/account/#booking-method) for what each does.

`AVERAGE`, `AVERAGE_ONLY` and `NONE` are not implemented yet: they report an
[`UnsupportedBookingMethod`](/reference/error-codes/#unsupportedbookingmethod) error. Any other value reports a
[`ParseInvalidMeta`](/reference/error-codes/#parseinvalidmeta) error. In both cases the default stays as it was,
`FIFO` unless set before.

### `default_commodity_precision`

The precision of a [`commodity`](/reference/directives/commodity/#precision) without a valid `precision` metadata
entry: how many decimals the web UI shows, and the scale a transaction must balance at. It does not apply to the
commodity that `operating_currency` defines.

### `default_rounding`

The rounding of a [`commodity`](/reference/directives/commodity/#rounding) without a `rounding` metadata entry, and of
the commodity that `operating_currency` defines when it is set before that option.

- `RoundDown`: a 5 in the first dropped decimal rounds down, so `0.005` rounds to `0.00` at precision 2.
- `RoundUp`: a 5 in the first dropped decimal rounds up, so `0.005` rounds to `0.01` at precision 2.

Other digits round to the nearest value in both modes. The value is case-sensitive: any other value, such as
`round_down`, stops the ledger from loading with the message `option value is invalid`.

### `default_balance_tolerance_precision`

Despite its name, this option gives balance assertions no tolerance: they are exact unless they write one with `~`
(see [Balance](/reference/directives/balance/)). It only sets the precision of the commodity that
[`operating_currency`](#operating_currency) defines, and only when it is set before that option. A value that is not
a whole number is ignored. To set the precision of the operating currency, prefer a `commodity` directive with a
`precision` entry.

### `directive_output_path`

Where the web UI writes the entries it creates: transactions, balance assertions, documents and the rest. The value
is a path relative to the ledger root, written as a [Jinja](https://jinja.palletsprojects.com/) template with these
placeholders, taken from the entry's date:

| Placeholder | Value |
|---|---|
| `{{year}}` | the year, such as `2024` |
| `{{month}}` | the month, without a leading zero: `1` to `12` |
| `{{month_str}}` | the month with two digits: `01` to `12` |
| `{{day}}` | the day, without a leading zero |
| `{{day_str}}` | the day with two digits |
| `{{type}}` | the kind of entry, such as `Transaction`, `BalanceCheck`, `BalancePad` or `Document` |
| `{{ext}}` | the extension of the main file, such as `zhang` or `bean`, so new entries are written in the ledger's own format |

With the default, a `main.zhang` ledger writes an entry dated in January 2024 to `data/2024/01.zhang`, and a
`main.bean` ledger to `data/2024/01.bean`. A file that does not exist yet is created, and an `include` of it is
appended to the main file. A value that is not a valid template stops the ledger from loading.

```zhang
; one file per month (the default)
option "directive_output_path" "data/{{year}}/{{month_str}}.{{ext}}"

; one file per day
option "directive_output_path" "data/{{year}}/{{month_str}}/{{day_str}}.{{ext}}"

; one file per kind of entry and year
option "directive_output_path" "data/{{year}}/{{type}}.{{ext}}"

; a single file
option "directive_output_path" "data/ledger.{{ext}}"
```

### `features.plugin`

Turns [plugins](/reference/directives/plugin/) on with `"true"`, in any letter case. Any other value turns them off.
`features.plugins` is the same option under another name; of the two, the one read last wins. Without it, `plugin`
directives are ignored.

### Accounting periods in queries

The `OPEN ON`, `CLOSE ON` and `CLEAR` clauses of a [query](/reference/query-language/#accounting-periods) post to
equity accounts. These Beancount options name them, with Beancount's defaults:

| Key | Account it names | Default |
|---|---|---|
| `account_previous_balances` | the opening balances of `OPEN ON` | `Opening-Balances` |
| `account_previous_earnings` | the earlier earnings of `OPEN ON` | `Earnings:Previous` |
| `account_previous_conversions` | the earlier conversions of `OPEN ON` | `Conversions:Previous` |
| `account_current_earnings` | the earnings of `CLEAR` | `Earnings:Current` |
| `account_current_conversions` | the conversions of `CLOSE ON` | `Conversions:Current` |
| `conversion_currency` | the currency of the zero price of conversion postings | `NOTHING` |

An account option gives the part of the name after `Equity:`. A value that is not a valid account name is ignored.
See [Equity accounts](/reference/query-language/#equity-accounts).

## Errors

| Error | When |
|---|---|
| [`MultipleOperatingCurrencyDetect`](/reference/error-codes/#multipleoperatingcurrencydetect) | `operating_currency` is set more than once. |
| [`UnsupportedBookingMethod`](/reference/error-codes/#unsupportedbookingmethod) | `default_booking_method` is `AVERAGE`, `AVERAGE_ONLY` or `NONE`. |
| [`ParseInvalidMeta`](/reference/error-codes/#parseinvalidmeta) | `default_booking_method` is not a booking method. |

An invalid `default_rounding` or `directive_output_path` is not reported as a ledger error: the ledger does not load.

## Beancount compatibility

- Zhang reads Beancount's `title` and `operating_currency` options. Beancount allows several operating currencies,
  which Fava shows side by side; Zhang supports one and reports the others.
- Beancount's own booking option is `booking_method`, and its default is `STRICT`. Zhang does not read
  `booking_method`: set `default_booking_method`, whose default is `FIFO`.
- Queries read Beancount's `account_previous_*`, `account_current_*` and `conversion_currency` options, as above.
- Every other Beancount option, such as `inferred_tolerance_default`, `documents`, `render_commas` or `name_assets`,
  is kept and has no effect. Account names must start with `Assets`, `Liabilities`, `Equity`, `Income` or `Expenses`
  whatever the `name_*` options say.
- `default_*`, `timezone`, `directive_output_path` and `features.*` are Zhang's own options. Beancount reports them as
  invalid options, so `bean-check` fails on a file that sets them.

## Related

- [Recording transactions](/guides/recording-transactions/): where the web UI writes new entries.
- [Commodity](/reference/directives/commodity/): precision and rounding per commodity.
