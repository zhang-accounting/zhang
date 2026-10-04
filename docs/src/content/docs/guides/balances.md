---
title: Balances and Padding
description: Check account balances with balance assertions and fill gaps with padding, from a day-to-day bookkeeping point of view.
sidebar:
  order: 2
---

A balance assertion states what an account held at a moment, as your bank statement or your wallet says. Zhang checks it against the postings of the account. Asserting balances regularly catches typos, forgotten and duplicated transactions while they are still easy to find.

Padding is the other half: it fills in an amount you cannot or do not want to account for in detail, such as the money an account held before you started your ledger.

The exact syntax of both is in [Balance](/reference/directives/balance/), and the details of padding in [Padding with `with pad`](/reference/directives/balance/#padding-with-with-pad).

## Assert a balance

Your bank statement for January ends with a balance of 16,643.60 CNY:

```zhang
2024-01-01 open Assets:Bank:Checking CNY
2024-01-01 open Income:Salary CNY
2024-01-01 open Expenses:Food CNY
2024-01-01 open Equity:Opening-Balances CNY

; the 5,000 CNY the account held before the ledger starts, see below
2024-01-01 balance Assets:Bank:Checking 5000 CNY with pad Equity:Opening-Balances

2024-01-05 * "ACME Corp" "January salary"
  Assets:Bank:Checking 12000 CNY
  Income:Salary

2024-01-20 * "Supermarket"
  Assets:Bank:Checking -356.40 CNY
  Expenses:Food

2024-02-01 balance Assets:Bank:Checking 16643.60 CNY
```

- An assertion is checked at the start of its date: it counts everything dated before it and nothing dated that day. To check a statement that ends on 31 January, date the assertion 1 February.
- With a time of day, `2024-01-31 23:59:59 balance …`, it counts the entries dated earlier that day too. An entry without a time counts as `00:00:00`, and an assertion comes before any other entry with the same date and time.
- The balance of an account includes its sub-accounts: `balance Assets:Bank …` checks `Assets:Bank`, `Assets:Bank:Checking` and every other account under `Assets:Bank` together.
- An assertion checks one commodity. Write one line per commodity for an account that holds several. A commodity the account does not hold counts as zero.

### Exact, unless you allow a tolerance

An assertion holds only when the balance equals the amount exactly. `16643.604 CNY` does not hold for a balance of `16643.60 CNY`, and neither does `16643.6` for `16643.604`.

When your source rounds, for example an app that shows a fund to two decimals while the units have more, write the tolerance you accept after `~`:

```zhang
2024-02-01 balance Assets:Bank:Checking 16643.60 ~ 0.01 CNY
```

It holds when the balance is within 0.01 CNY of 16,643.60 CNY. Zhang never derives a tolerance from the number of decimals you write, and no option loosens assertions: `default_balance_tolerance_precision`, despite its name, does not.

## Start from an existing balance

Your bank account already held money when you started the ledger. Instead of recording its whole history, pad it to the balance it had:

```zhang
2024-01-01 open Assets:Bank:Checking CNY
2024-01-01 open Equity:Opening-Balances CNY

2024-01-01 balance Assets:Bank:Checking 5000 CNY with pad Equity:Opening-Balances
```

`with pad Equity:Opening-Balances` asks Zhang to add a transaction that brings the account to 5,000 CNY, taking the difference from `Equity:Opening-Balances`:

- The padding transaction has the flag `P`, the payee `Balance Pad` and the narration `pad Assets:Bank:Checking to Equity:Opening-Balances`. It is dated on the assertion's date and comes before that day's other entries, so the account holds 5,000 CNY at the start of the day. The Journals page lists it as a **Pad**.
- It is sized from the balance the postings give at that point. If the account already holds the amount, Zhang adds nothing.
- The assertion is then checked like any other, against the padded balance.
- Padding is not limited to opening balances. It also closes a gap you will never reconstruct, such as a wallet after a month of small cash spending:

```zhang
2024-01-01 balance Assets:Cash 200 CNY with pad Equity:Opening-Balances

2024-01-10 * "Bakery"
  Assets:Cash -18 CNY
  Expenses:Food

; counted 150 CNY: the other 32 CNY went on things not worth recording
2024-02-01 balance Assets:Cash 150 CNY with pad Expenses:Misc
```

Here Zhang moves 32 CNY from `Assets:Cash` to `Expenses:Misc` on 1 February.

### Beancount's `pad`

A beancount ledger writes the pad and the assertion as two directives:

```beancount title="main.bean"
2024-01-01 pad Assets:Bank:Checking Equity:Opening-Balances
2024-01-02 balance Assets:Bank:Checking 1000.00 USD
```

Zhang pairs them the way beancount does: a `pad` serves the next `balance` of its account in each commodity, up to the account's next `pad`. Both must be in the same file. A `balance` on the same day as the `pad` is not padded, and a `pad` that no `balance` follows does nothing.

Each pair works like `balance … with pad`, so the padding transaction is dated on the day of the `balance`, here 2 January, while beancount dates it on the day of the `pad`. The balance on and after 2 January is the same. The `pad` directive only exists in beancount files; in a Zhang file, write `with pad`.

## When an assertion fails

A failing assertion changes nothing in your books: the account keeps the balance its postings give, and later assertions and pads are measured from that balance too. Zhang only reports it:

- The ledger's error list on the Overview page shows an [`AccountBalanceCheckError`](/reference/error-codes/#accountbalancecheckerror) with the name of the account.
- On the Journals page the assertion is marked **Check failed**. Its preview shows the asserted balance, the accumulated balance, the difference and the tolerance.
- On the account's page, the **Journals** tab shows the assertion as a row with the amount it asserted next to the running balance.

To find the cause:

1. Look at the difference. It often equals one transaction: a forgotten one, one recorded twice, or one with the wrong sign (twice its amount).
2. List the postings of the account with their running balance and compare them with the statement, line by line:

   ```sql
   JOURNAL 'Assets:Bank:Checking' FROM date >= 2024-01-01 AND date < 2024-02-01
   ```

3. Narrow the period down with more assertions, for example one per statement or per week. The first one that fails tells you where to look.
4. List every failing assertion with the difference, the balance the postings give minus the asserted amount, on the [Query](/guides/querying/) page:

   ```sql
   SELECT date, account, amount, discrepancy FROM #balances WHERE discrepancy IS NOT NULL
   ```

When you have found and fixed the transactions, the assertion holds again. If you decide the difference is not worth finding, turn the assertion into `balance … with pad` to an expense account, which records the difference on purpose.

## Assert balances in the web UI

- On an account's page, the **Balance check** tab lists the commodities the account holds. Enter the actual balance of one and select **Check balance**. To pad the difference, pick an account under **Pad from** and select **Pad & check**.
- **Tools** → **Batch balance** does the same for many accounts at once. Rows left empty are skipped.

The web UI dates these assertions with the current date and time, so they count everything recorded up to now, today's entries included. It writes them to the file the [`directive_output_path`](/guides/recording-transactions/#where-new-entries-are-written) option names.

## Plugins, pads and assertions

Zhang processes a ledger in this order: first the [plugins](/guides/plugins/), in the order they are declared, then the check that every account is open, then the pads, then the assertions. So a transaction a plugin adds is part of the balance a pad fills up to and an assertion checks. A plugin sees your `balance … with pad` directives, but not the padding transactions, which do not exist yet when it runs.
