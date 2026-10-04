---
title: Transaction
description: Reference for transactions, their postings, amounts, costs and prices, and the metadata of each.
sidebar:
  order: 4
---

A transaction moves amounts between accounts. It is a header line with the date, an optional flag, the payee and the
narration, followed by one indented line per posting. The amounts of its postings must add up to zero in each
commodity.

## Syntax

```text
YYYY-MM-DD [HH:MM[:SS]] [<Flag>] ["<Payee>"] ["<Narration>"] [#tag …] [^link …]
  [<key>: <value>]
  [<Flag>] <Account> [<Amount>] [<Cost>] [@ <Price> | @@ <TotalPrice>] [; <comment>]
    [<key>: <value>]
  [<Flag>] <Account> …
```

| Part | Required | Description |
|---|---|---|
| Date and time | yes | The date, optionally followed by a time of day (`10:30` or `10:30:15`) in the ledger's [timezone](/reference/directives/options/#timezone). |
| `<Flag>` | no | On the header, `*` for a completed transaction, `!` for one to check. See [Flags](#flags). Before the account of a posting, the posting's own flag. See [Posting flags](#posting-flags). |
| `"<Payee>"`, `"<Narration>"` | no | Quoted strings. See [Payee and narration](#payee-and-narration). |
| `#tag`, `^link` | no | Tags and links, in any order, at the end of the header line. |
| `<key>: <value>` | no | Metadata of the transaction, or of the posting above it. See [Metadata](#metadata). |
| `<Account>` | yes | The account of the posting. |
| `<Amount>` | no | A number and a commodity, such as `-35.50 CNY`. One posting may leave it out. |
| `<Cost>` | no | The cost of the units: `{…}` per unit or `{{…}}` in total. See [Costs and prices](#costs-and-prices). |
| `@ <Price>`, `@@ <TotalPrice>` | no | The price the units were converted at, per unit or in total. |

The postings and metadata lines follow the header without blank lines: an empty line ends the transaction. An indented
line starting with `;`, `#` or `//` is a comment, and a posting can end with a `; comment`. `*` does not start a
comment: an indented line such as `* Assets:Cash -10 CNY` is a [flagged posting](#posting-flags), as in Beancount. In a
Beancount file, `# Assets:Cash -10 CNY` is a flagged posting too, as Beancount reads it.

## Examples

```zhang
2024-01-01 open Assets:Cash CNY
2024-01-01 open Expenses:Food CNY

2024-01-02 * "Cafe" "lunch" #work ^trip-2024
  Assets:Cash -35.50 CNY
  Expenses:Food 35.50 CNY

2024-01-03 12:30 * "Bakery" "bread"
  Assets:Cash -12 CNY
  Expenses:Food
```

## Header

### Flags

| Flag | Meaning |
|---|---|
| `*` | A completed transaction. `txn` is the same flag, as in Beancount. |
| `!` | A transaction to check. |
| `P` | A padding transaction, which [`balance … with pad`](/reference/directives/balance/#padding-with-with-pad) or a [`pad`](/reference/directives/balance/#the-pad-directive) adds, or which the web UI writes for a pad in a Beancount file. Zhang orders transactions written with `P` like balance assertions: after `open` and `commodity`, before the other entries of their date and time. The padding transactions of a `pad` stay right after it. |
| another uppercase letter, `#`, `&`, `?` or `%` | A flag of your own, kept as written. `C` is an ordinary flag too. These are the other flags Beancount accepts. |

A transaction without a flag is completed (`*`).

### Payee and narration

The header needs a flag or at least one quoted string.

| Header | Payee | Narration |
|---|---|---|
| `* "Cafe" "lunch"` | `Cafe` | `lunch` |
| `* "lunch"` | none | `lunch` |
| `"Cafe" "lunch"` | `Cafe` | `lunch` |
| `"Cafe"` | `Cafe` | none |

With a flag, a single string is the narration. Without a flag, it is the payee.

## Postings

### Posting flags

A posting can have a flag of its own, written before its account with at least one space or tab in between, as in
Beancount. Importers and Fava use it to mark a posting to check:

```zhang
2024-01-10 * "Lunch"
  ! Assets:Cash -10 CNY
  Expenses:Food
```

A posting flag is any Beancount flag except `#`: `*`, `!`, `&`, `?`, `%` or an uppercase letter. In a zhang file it
cannot be `#`: an indented line starting with `#` is a comment there, so `# Assets:Cash -10 CNY` stays a comment and
books nothing. In a Beancount file a posting flag can also be `#`, as in Beancount, where that line is a posting
flagged `#`. `txn` is a flag of the header only. The space after the flag is required.

:::caution[`*` is not a comment in a zhang file]
The comment prefixes of a zhang file are `;`, `#` and `//`. `*` is a flag, not a comment prefix: a line starting with
`*` that is neither a posting nor a directive is a parse error, never a comment. Earlier versions of Zhang read such a
line as a comment. A `.zhang` file that used `*` for comments, such as org-mode headings like `* Banking`, must switch
them to `;` or `#`, otherwise it fails to load with a parse error at that line. `.bean` files are not affected: a line
starting with `*` outside a transaction is an org-mode heading there, which Zhang skips as Beancount does.
:::

A posting flag changes nothing in how the posting is booked. Zhang keeps it:

- When Zhang writes the transaction, the flag is written back before the account. Editing a transaction in the web UI
  keeps the flag of each posting it edits, though the form does not show it.
- In [queries](/reference/query-language/#columns), the `posting_flag` column holds it, and is `NULL` for a posting
  without a flag, as in beanquery. The `flag` column is the transaction's.

### Amounts

An amount is a number followed by a commodity, such as `-1,234.50 CNY`. The number can have `,` or `_` between its
digits, and can be an expression with `+`, `-`, `*`, `/` and parentheses: `(120 + 35) / 2 CNY` is `77.5 CNY`. The
commodity must be [defined](/reference/directives/commodity/) at the transaction's date.

### Elided amounts

One posting of a transaction may leave out its amount. Zhang gives it the amount that balances the transaction:

- The weights of the other postings (see [How a transaction balances](#how-a-transaction-balances)) must leave
  exactly one commodity unbalanced. The posting without an amount gets the opposite of that remainder.
- If the other postings already balance and are all in one commodity, it gets zero of that commodity. The journal
  keeps the posting as you wrote it.
- The inferred amount is exact. Only an amount that a division leaves with more than 20 decimals, such as a total cost
  spread over 3 units, is rounded, with the commodity's `rounding`, at the larger of its precision and the most
  decimals written in the transaction in that commodity.

When there is nothing to infer, or several postings leave out their amount, or the others leave several commodities
unbalanced, the transaction is reported and **not booked**: it changes no balance until you fix it. See
[Errors](#errors).

### Costs and prices

| Posting | Weight |
|---|---|
| `Assets:Wallet 100 USD` | `100 USD` |
| `Assets:Wallet 100 USD @ 7.10 CNY` | `710 CNY`: the units at the price per unit |
| `Assets:Wallet 100 USD @@ 710 CNY` | `710 CNY`: the total price |
| `Assets:Broker 10 AAPL {185 USD}` | `1850 USD`: the units at the cost per unit |
| `Assets:Broker 3 AAPL {{1000 USD}}` | `1000 USD`: the total cost |
| `Assets:Broker -5 AAPL {}` | the cost of the lots the reduction takes |

A cost can also give the acquisition date and a label of a lot, as in `{185 USD, 2024-01-02, "lot-a"}`. A posting with
a cost and a price, such as `-5 AAPL {185 USD} @ 200 USD`, weighs its cost; the price only records what it was sold
for. How lots are created, matched and reduced is explained in [Lots and cost basis](/guides/lots-and-cost-basis/)
and [Booking method](/reference/directives/account/#booking-method).

### How a transaction balances

Like in Beancount, each commodity must balance on its own. A posting contributes its weight: its units, its units at
the price, or its units at cost (see the table above). Zhang adds up the weights of each commodity, rounds the sum to
the commodity's [precision](/reference/directives/commodity/#precision) with its
[rounding](/reference/directives/commodity/#rounding), and the result must be zero.

With the default precision of 2, a transaction off by `0.004 CNY` balances, and one off by `0.006 CNY` does not. One
off by exactly `0.005 CNY` balances with `RoundDown`, the default, and not with `RoundUp`.

A transaction that does not balance is reported as
[`UnbalancedTransaction`](/reference/error-codes/#unbalancedtransaction) and still booked, so the account balances
follow what you wrote.

## Metadata

Metadata lines are `key: value` pairs. A transaction has its own metadata, and each posting can have metadata of its
own:

```zhang
2024-01-02 * "Cafe" "lunch"
  invoice: "2024-001"
  Assets:Cash -10 CNY
    receipt: "r-17"
  Expenses:Food 10 CNY
    category: "meals"
```

Here `invoice` belongs to the transaction, `receipt` to the `Assets:Cash` posting and `category` to the
`Expenses:Food` posting.

A value is a quoted string, or a single word without spaces, quotes, colons, parentheses or commas, such as `123`,
`2024-01-01` or `TRUE`. Write an account name as a value in quotes. Every value is kept as text.

### Which lines belong to a posting

In a zhang file (`.zhang`), a metadata line belongs to the posting above it **only when it is indented deeper than
that posting's line**. Every other metadata line belongs to the transaction, wherever it is: before the postings,
between them or after them.

```zhang
2024-01-02 * "Cafe" "lunch"
  Assets:Cash -10 CNY
    receipt: "r-17"      ; deeper than the posting: the posting's
  Expenses:Food 10 CNY
  invoice: "2024-001"    ; at the postings' indentation: the transaction's
```

Older versions of Zhang wrote transaction metadata after the postings, at the same indentation as the postings, so
with this rule existing zhang ledgers keep their meaning. When you indent with tabs, a tab counts up to the next
multiple of four columns.

In a Beancount file (`.bean`, `.bc` or `.beancount`), Zhang follows Beancount instead: metadata before the first
posting belongs to the transaction, and **every metadata line after a posting belongs to that posting, however it is
indented**. This is how Beancount and Fava read the file.

### How Zhang writes metadata

When Zhang writes a transaction, for example when you create or edit one in the web UI, it writes the transaction's
metadata right after the header, then each posting followed by its own metadata, indented one level deeper than the posting:

```zhang
2024-01-02 * "Cafe" "lunch"
  invoice: "2024-001"
  Assets:Cash -10 CNY
    receipt: "r-17"
  Expenses:Food 10 CNY
```

Zhang reads this layout the same way in both file formats, and so do Beancount and Fava. A key that is not a single
word, such as `"my key"`, is written in quotes. Beancount has no quoted keys, so in a Beancount ledger the web UI only
takes a new key that Beancount can read.

### Using metadata

- Zhang's API returns the metadata of each posting with the posting, next to the transaction's own metadata, and takes
  both when a transaction is created or edited.
- In [queries](/reference/query-language/#metadata-functions), `meta('key')` reads the posting's metadata,
  `entry_meta('key')` the transaction's, and `any_meta('key')` the posting's and then the transaction's. The `meta`
  column of the postings table holds the posting's metadata as text.
- A `document` metadata entry links a file to the transaction, whether it is written on the transaction or on one of
  its postings. See [Document](/reference/directives/document/).

### Plugins

WASM plugins receive and return transactions with the metadata of each posting in the `meta` field of the posting. A
plugin built against an older version of Zhang, from before posting metadata, still works, but it reads and writes
back every directive it is given, so **every** transaction that passes through it loses the metadata of its postings,
not only the ones it changes. Rebuild such a plugin to keep it.

A plugin can also set the `flag` of a posting. In a zhang ledger, a posting flag that a zhang file cannot hold, `#`,
is left out when Zhang writes the transaction, for example after an edit in the web UI: the posting is written
without it, so that it stays a posting rather than a comment. A `*` flag is written, as in a Beancount file.

## Errors

| Error | When | Booked? |
|---|---|---|
| [`UnbalancedTransaction`](/reference/error-codes/#unbalancedtransaction) | A commodity does not balance. | yes |
| [`CommodityDoesNotDefine`](/reference/error-codes/#commoditydoesnotdefine) | The transaction balances in a commodity that is not defined. | yes |
| [`AccountDoesNotExist`](/reference/error-codes/#accountdoesnotexist), [`AccountClosed`](/reference/error-codes/#accountclosed) | A posting's account is not open at the transaction's date. | yes |
| [`NoEnoughCommodityLot`](/reference/error-codes/#noenoughcommoditylot), [`AmbiguousLotMatch`](/reference/error-codes/#ambiguouslotmatch) | A reduction at cost does not find its lots. | yes |
| [`BudgetDoesNotExist`](/reference/error-codes/#budgetdoesnotexist) | A posting's account is linked to a budget that is not defined. | yes |
| [`TransactionCannotInferTradeAmount`](/reference/error-codes/#transactioncannotinfertradeamount) | The elided amount has nothing to be inferred from. | no |
| [`TransactionHasMultipleImplicitPosting`](/reference/error-codes/#transactionhasmultipleimplicitposting) | More than one posting leaves out its amount. | no |
| [`TransactionExplicitPostingHaveMultipleCommodity`](/reference/error-codes/#transactionexplicitpostinghavemultiplecommodity) | The other postings leave several commodities unbalanced. | no |

## Beancount compatibility

- A time of day is written as `time: "HH:MM:SS"` metadata in a Beancount file. Zhang writes it that way too. Zhang
  ignores it on `balance` and `pad`, as Beancount does.
- Beancount requires a flag or `txn` on every transaction. A header without one, such as `2024-01-02 "Cafe" "lunch"`,
  only works in Zhang.
- A flag in front of a posting, such as `! Assets:Cash -10 CNY` or `* Assets:Cash -10 CNY`, is read as in Beancount.
  See [Posting flags](#posting-flags). `# Assets:Cash -10 CNY` is a flagged posting in a Beancount file and a comment
  in a zhang file. Beancount also reads a flag without the space, as in `!Assets:Cash -10 CNY`; Zhang reports that
  line as an error.
- A line starting with `*` outside a transaction, an org-mode heading, is skipped in a Beancount file, as Beancount
  does. In a zhang file it is a parse error: use `;` or `#` for comments there.
- Beancount's `pushtag` / `poptag` and `pushmeta` / `popmeta` work in Beancount files only. Zhang applies them while
  it reads the file.
- Posting metadata follows Beancount's rule in Beancount files, as described in
  [Which lines belong to a posting](#which-lines-belong-to-a-posting).

## Related

- [Recording transactions](/guides/recording-transactions/): writing transactions in files and in the web UI.
- [Lots and cost basis](/guides/lots-and-cost-basis/): costs, lots and booking.
- [Balance](/reference/directives/balance/): checking the result against your statements.
