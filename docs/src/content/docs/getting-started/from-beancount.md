---
title: Coming from Beancount
description: Serve an existing beancount ledger with Zhang, and what to expect from its beancount support, the behavior differences and the features on either side.
sidebar:
  order: 4
---

Zhang reads beancount ledgers directly. You can point it at your `main.bean` and keep editing your files as before: Zhang shows them in its web UI, and what you record there is written back in beancount syntax.

This page shows how to serve a beancount ledger and lists, in [Compatibility](#compatibility), where Zhang and beancount differ.

## Serve a beancount ledger

Give `zhang serve` the directory of your ledger and the name of its main file:

```shell
zhang serve /path/to/ledger --endpoint main.bean
```

With Docker, mount the directory at `/data` and add `--endpoint` after the image name:

```shell
docker run --name zhang -v "/path/to/ledger:/data" -p "8000:8000" kilerd/zhang:latest --endpoint main.bean
```

- A main file ending in `.bean`, `.bc` or `.beancount` makes Zhang read the whole ledger as beancount: the main file and every file it includes, whatever their extension. A ledger is either beancount or Zhang syntax, never a mix.
- Open `http://localhost:8000`. The **Overview** item in the sidebar shows how many problems Zhang found, and the Overview page lists them. Go through them first: most come from the differences below.
- What you record in the web UI goes to a file with your main file's extension, by default `data/{{year}}/{{month_str}}.bean`, written in beancount syntax. See [Recording Transactions](/guides/recording-transactions/#where-new-entries-are-written).

[Installation](/getting-started/installation/) lists every option of `zhang serve`.

## Posting metadata

Zhang reads the metadata of a transaction in a beancount file the way beancount and Fava do: metadata before the first posting belongs to the transaction, and every metadata line after a posting belongs to that posting, however it is indented.

```beancount
2024-01-02 * "Cafe" "lunch"
  invoice: "2024-001"     ; the transaction's
  Assets:Cash -10 CNY
    receipt: "r-17"       ; the Assets:Cash posting's
  Expenses:Food 10 CNY
  category: "meals"       ; the Expenses:Food posting's
```

Older versions of Zhang wrote a transaction's metadata after its postings. Such a file now reads differently in Zhang: that metadata belongs to the last posting, which is how Fava has always read it. Two keys keep working as before:

- `time`: a `time` after the last posting, at the postings' indentation as older Zhang wrote it, is still the transaction's time of day, unless the transaction has a `time` of its own or another posting has one. A `time` indented deeper than its posting stays the posting's.
- `document`: a document on a posting is a document of its transaction, so documents attached to a transaction stay attached.

When Zhang writes a transaction, for example after you edit it in the web UI, it writes the transaction's metadata before the postings and each posting's metadata right under it, so the file reads the same in Zhang, beancount and Fava. See [Transaction](/reference/directives/transaction/).

## Compatibility

### Directives

| Beancount | In Zhang |
|---|---|
| `open` | Read, with its commodities and booking method (`open Assets:Broker HOOL "FIFO"`). The commodities must be declared, but postings in other commodities are not reported. |
| `close` | Read. Closing an account that still holds something is reported as [`CloseNonZeroAccount`](/reference/error-codes/#closenonzeroaccount). |
| `commodity` | Read, and required: see [Commodities must be declared](#commodities-must-be-declared). |
| transactions | Read, with the flags `*`, `!` and other letters, the `txn` keyword, tags, links, costs (`{}`, `{{}}`, with a date and a label) and prices (`@`, `@@`). |
| `balance` | Read, with an optional `~` tolerance. Exact without one: see [Balance assertions are exact](#balance-assertions-are-exact). |
| `pad` | Read, paired with the `balance` it serves: see [Pads](#pads). |
| `note`, `event` | Read. |
| `document` | Read. The path is relative to the ledger's root directory. See [Documents](/guides/documents/). |
| `price` | Read, for valuations in queries and the Commodities page. |
| `query` | Read: the queries appear in the **Saved** menu of the [Query](/guides/querying/) page. |
| `custom` | Read. `custom budget …` defines [budgets](/guides/budgets/): see [Budgets](#budgets). |
| `option` | Read. Only some options have an effect: see [Options](#options). |
| `plugin` | Python plugins do not run: see [Plugins](#plugins). |
| `include` | Read, including `*` patterns such as `include "2024/*.bean"`. |
| `pushtag`, `poptag`, `pushmeta`, `popmeta` | Read. |

A `time: "HH:MM:SS"` metadata entry gives any directive a time of day.

Zhang cannot read the following. A file using them does not load at all:

- metadata values that are amounts, such as `limit: 10.00 USD`. Quote them: `limit: "10.00 USD"`;
- a cost with only a date or a label, such as `{2024-01-01}` or `{"lot-1"}`. Write the cost first: `{100.00 USD, 2024-01-01}`;
- compound costs, `{100 # 9.95 USD}`, and `{*}`;
- a flag on a posting, such as `  ! Assets:Cash -10 USD`. Move the flag to the transaction.

### Options

Zhang uses these options of a beancount ledger:

- `title`, shown in the web UI;
- `operating_currency`, but only one: Zhang keeps the last one and reports every other as [`MultipleOperatingCurrencyDetect`](/reference/error-codes/#multipleoperatingcurrencydetect);
- `account_previous_balances`, `account_previous_earnings`, `account_previous_conversions`, `account_current_earnings`, `account_current_conversions` and `conversion_currency`, in the [accounting periods](/reference/query-language/#accounting-periods) of queries.

Every other beancount option, such as `booking_method`, `inferred_tolerance_default` or `documents`, is listed on the Settings page and otherwise ignored. Zhang has options of its own, such as `default_booking_method` and `timezone`, which beancount ignores in turn. See [Options](/reference/directives/options/).

### Behavior differences

#### Commodities must be declared

Every commodity a posting weighs in, a `price` names or an `open` lists must have a `commodity` directive. The operating currency is declared by its option. Beancount does not require `commodity` directives; Zhang reports a missing one as [`CommodityDoesNotDefine`](/reference/error-codes/#commoditydoesnotdefine). Add one per commodity, dated before its first use:

```beancount
1970-01-01 commodity HOOL
```

#### Balance assertions are exact

Beancount lets a `balance` pass when the difference is within a tolerance it derives from the number of decimals: `balance Assets:A 10.00 USD` passes on a balance of `10.004 USD`. Zhang does not: an assertion holds only on the exact amount, or within the tolerance you write after `~`, such as `10.00 ~ 0.005 USD`. See [Balances and Padding](/guides/balances/#exact-unless-you-allow-a-tolerance).

#### Transactions balance at the commodity's precision

A transaction balances when, in each commodity, its weights add up to zero once rounded at the commodity's precision: the `precision` metadata of its `commodity` directive, 2 decimals without one. Beancount derives this tolerance from the numbers written in the transaction instead. Give commodities with finer amounts, such as `BTC`, a `precision` of their own. See [Lots and Cost Basis](/guides/lots-and-cost-basis/#rounding).

#### Booking

Zhang books with `FIFO` unless told otherwise, while beancount's default is `STRICT`. For beancount's behavior, add `option "default_booking_method" "STRICT"`; beancount's own `booking_method` option is not read. `NONE`, `AVERAGE` and `AVERAGE_ONLY` are not implemented: an account using one gets an error and books with the default method. Lot labels are read and shown in queries, but Zhang does not use them to choose lots. See [Lots and Cost Basis](/guides/lots-and-cost-basis/#choose-a-booking-method).

#### Pads

Zhang pairs each `pad` with the `balance` entries it serves as beancount does: the next `balance` of the account in each commodity, up to the account's next `pad`, never one on the same day as the `pad`. The differences:

- The padding transaction is dated on the day of the `balance`, not of the `pad`. The balance from that day on is the same as in beancount, but between the two dates the account does not include the padding yet.
- A `pad` serves only a `balance` in the same file.
- A `pad` that serves no `balance` is ignored. Beancount reports it as unused.

See [Balances and Padding](/guides/balances/#beancounts-pad), and [Balance](/reference/directives/balance/#beancount-compatibility) for the cases of parent accounts and sub-accounts.

#### Prices

Only `price` directives give prices. The prices written on postings with `@` and `@@` do not, as they would with beancount's `implicit_prices` plugin.

#### Other checks

- Closing an account that still holds something is reported as an error. Beancount allows it.
- Postings in a commodity that the account's `open` does not list are not reported. Beancount reports them.
- Account names must start with `Assets`, `Liabilities`, `Equity`, `Income` or `Expenses`. The `name_assets`… options, which rename them in beancount, are not read.
- In a quoted string, a backslash that does not start an escape is kept: `"\d"` stays `\d`, where beancount drops the backslash.

### Not supported

#### Plugins

Beancount's plugins are Python code, and Zhang does not run them, including the ones that ship with beancount, such as `auto_accounts` or `implicit_prices`. Without them:

- open every account explicitly, instead of relying on `auto_accounts`;
- check what other plugins did for you, and do it in the ledger or with a [query](/guides/querying/).

`plugin` lines stay harmless as long as the ledger does not enable Zhang's own plugins: with `option "features.plugin" "true"`, Zhang tries to load every `plugin` as a WASM module, and a Python module name makes the whole ledger fail to load. Zhang's plugins are WASM modules: see [Using Plugins](/guides/plugins/).

#### Budgets

Zhang reads `custom budget Food CNY`, `custom budget-add Food 2000 CNY`, `custom budget-transfer Fun Food 300 CNY` and `custom budget-close Food`, with the word `budget` and the budget names unquoted, and writes budgets that way. Beancount itself rejects these lines, because it needs a quoted type and quoted names; a quoted `custom "budget" "Food" CNY` is an ordinary `custom` directive for Zhang. So if you also check the ledger with `bean-check`, it reports the budget lines as errors.

### Zhang-only features

These work in Zhang and have no equivalent in beancount:

- [budgets](/guides/budgets/), as above;
- a time of day on entries, which a beancount ledger carries in `time` metadata;
- Zhang's [WASM plugins](/guides/plugins/) and its [query language](/reference/query-language/) extensions, such as the `#budgets` and `#errors` tables.

A Zhang ledger (`main.zhang`) has more syntax of its own, such as `balance … with pad` and dates with a time; beancount cannot read it. Zhang has no command to convert between the two syntaxes, so keep your ledger in beancount syntax if you want to keep using beancount tools.
