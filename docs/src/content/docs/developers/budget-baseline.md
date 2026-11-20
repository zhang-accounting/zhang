---
title: Budget Baseline
description: The recorded performance of the native budget implementation on 10,000 and 100,000 transactions (load and reload, the budget pages' requests, a transaction written, memory, data volume), which the budget plugin of RFC #698 is measured against, and how to reproduce it.
sidebar:
  order: 4
---

[RFC #698](https://github.com/zhang-accounting/zhang/issues/698) moves budgets out of the core and the query engine into an official WASM plugin. Its phase P4 compares the plugin with the native implementation on the same ledgers and the same requests. This page is the baseline of phase P0 it compares against: how long the native implementation takes to load and reload a ledger with budgets and without, to answer the budget pages' requests and to write a transaction, how much memory it uses, and how much data the pages and the budget computation move. It also says how the numbers were taken and how to take them again.

:::caution[Noisy numbers]
The numbers were taken on a build machine shared with other jobs. They are given for two runs, and they vary between runs and within one. Compare the plugin with the native implementation by running both in the same session, as [Reproduce](#reproduce) shows, rather than with this page alone.
:::

## The ledgers

`zhang_testkit::budget_ledger` generates a household's ledger from a seed, a number of transactions, a number of budgets and a number of months. The baseline uses seed 698, 60 months (January 2021 to December 2025) and 30 budgets:

- 30 budgets in USD, the operating currency, and in EUR, JPY and CNY, in eight categories, linked to 47 expense accounts by the `budget` metadata of their `open`s. One account is in two budgets, two accounts are closed and opened again with other budgets (in January 2024 and January 2025), and three expense accounts are in no budget.
- Postings in USD, EUR, GBP, JPY and CNY, some with an `@` price, a few of `GOLD` held at cost, and seven prices a month that convert every one of them into the commodity of its budgets: directly, through an inverse rate, or through the cost currency.
- A `budget-add` per open budget and month, some in another commodity than the budget's; two to four `budget-transfer`s a month, some between commodities; one budget defined in April 2022; four `budget-close`s, one of them with a time, each followed by a transfer of what is left.
- A household's routine: salaries, rent, insurance, a subscription, card payments, currency top-ups and shares bought at cost, 602 of the transactions. The others are spending on random days, 30% of them with a time of day.

The ledger without budgets is the same ledger without the budget directives and the `budget` metadata. Both load with two errors, the [`CloseNonZeroAccount`](/reference/error-codes/#closenonzeroaccount) of the two reopened accounts, and with no budget error.

| Ledger | Transactions | Postings | Budget directives | Bytes | SHA-256 |
|---|---:|---:|---:|---:|---|
| `10000tx-30b-60m` | 10,000 | 20,799 | 1,893 | 1,078,224 | `b46b01f46a38a81a4650f480bb141b4f4e2589b5a40d9e3f03d5b0bd7b1907d2` |
| `10000tx-60m-no-budgets` | 10,000 | 20,799 | 0 | 995,780 | `e1cc6c76b2b925d2c12b602b6b725fb76d8f95bacc85382423eb70c554914962` |
| `100000tx-30b-60m` | 100,000 | 208,281 | 1,893 | 9,891,591 | `7f4efc770c0b1bb22c47460e7f32dd91a5f8d72016b03d3165a810b4f92005a7` |
| `100000tx-60m-no-budgets` | 100,000 | 208,281 | 0 | 9,809,147 | `bf4c0b58d72b67b13870540fb0b7c458f089d579453dd9c66fb1987f948e5e07` |

The budget directives are 30 `budget`, 1,680 `budget-add`, 179 `budget-transfer` and 4 `budget-close`. Each ledger is a `main.zhang` and a file per year. Its digest is the SHA-256 of the `sha256sum` listing of its files, which [Reproduce](#reproduce) shows how to check. The tests of `zhang-testkit` check the digests of the two 10,000-transaction ledgers, and the bench checks all four before it measures, so a change of the generator cannot produce numbers for another ledger unnoticed.

## Method

The bench is `zhang-server/tests/budget_baseline.rs`. Its tests are ignored, and skipped in a debug build. It runs in the release profile, the one zhang is shipped with (`opt-level = "s"`). Each ledger is a test of its own, in a binary of its own: nextest runs each test in a process of its own, so the peak memory a test reads is its own.

For each ledger, the bench:

1. Generates the ledger into a temporary directory, checks its digest, and resets the peak resident set size of the process (`5` written to `/proc/self/clear_refs`).
2. Loads it with `Ledger::load_with_clock` and a fixed clock: noon of 2025-12-31 in the ledger's timezone, `Asia/Shanghai`, so today is the ledger's last day and the current month is December 2025. This is the **load** time.
3. Serves it with the server's router, the app `zhang serve` runs with its middleware, in the same process and without a socket (`zhang_testkit::http::app` and `router_of`). It reloads the ledger 8 times as the server does, replacing the ledger once the new one is loaded. These are the **reload** times.
4. With budgets, reloads it 10 more times, each time followed by one request, the kinds of request in turn. These are the **cold** requests: the first query of a loaded ledger also builds the query engine's cache of that ledger (its postings, entries and prices).
5. Sends 3 requests of each kind to warm up, then 30 rounds of one request of each kind, the order of the kinds rotating by one from round to round. These are the **warm** requests.
6. Runs the queries of [Data volume](#data-volume).
7. With budgets, drops that ledger and its router, copies the ledger's files to another temporary directory, loads the copy and serves it with a router of its own, and writes 22 transactions through `POST /api/transactions`: 2 to warm up, then 20 timed. These are the **writes**.

A request is timed from when it is handed to the router until its body is read whole. The request is built before, and the body is not parsed. The percentiles are nearest-rank: of 30 sorted samples, p50 is the 15th and p95 the 29th; of 20, the 10th and the 19th.

The requests are those the budget pages send since the typed budget endpoints were [retired](/reference/builtin-queries/#retired-endpoints), for the budget `Groceries`, which has the most postings (two accounts, one of them shared with `Household`), and the month 2025-12-01:

| Request | What it answers | Sent as |
|---|---|---|
| `month` | the budget list of the current month, once `GET /api/budgets` | `POST /api/query/builtins/budgets.month` with `{"params": {"month": "2025-12-01"}}` |
| `month_first` | the budget list of the ledger's first month | the same with `"month": "2021-01-01"` |
| `detail` | a budget's page, once `GET /api/budgets/{name}` | `POST /api/query/builtins` with `budgets.budget` (`name`) and `budgets.budget_month` (`name`, `month`) |
| `interval` | a budget's month, once `GET /api/budgets/{name}/interval/{year}/{month}` | `POST /api/query/builtins` with `budgets.events` and `budgets.postings` (`name`, `month`) |
| `bql_year` | the budget page's figures of every budget over a year | `POST /api/query` with `SELECT name, date, assigned, activity, available FROM #budgets WHERE year = 2025` |
| `write` | a transaction added in the web UI or the app | `POST /api/transactions` with a purchase on `Expenses:Food:Groceries` paid by card on 2025-12-30 at noon, the amount changing from write to write |

A write appends the transaction to `data/2025/12.zhang`, the file of the default `directive_output_path` option (the first write creates it and adds its `include` to `main.zhang`), then reloads the ledger in place before it answers, holding the ledger's write lock. The reload is synchronous in that path: the write's time includes a whole load of the ledger, and the ledger grows by one transaction a write, from 100,000 to 100,022 transactions. No reload listener takes part, so the bench does not start one. `zhang serve` also watches the ledger's files and reloads again once it sees the file written; that reload comes after the answer and is not in these times.

Memory is read from `/proc/self/status`: `VmHWM`, the peak resident set size, and `VmRSS`, the resident set size, in MiB.

## Machine

| | |
|---|---|
| CPU | AMD EPYC-Turin, 96 CPUs on the node (`nproc`), a limit of 64 CPUs for the build pod (cgroup `cpu.max`); the node is shared with other build jobs |
| Memory | 251 GiB on the node, a limit of 128 GiB for the pod |
| Kernel | Linux 7.0.0-38-generic |
| Toolchain | rustc 1.100.0 (0ef1bc386 2026-11-09), cargo-nextest 0.9.152 |
| Code | `main` at `9c156c7b`, with this bench |
| Load average of the node | 13.5 to 16.5 over one minute during the two runs |

## Results

Two runs of the whole suite, A and B, one after the other. A cell gives A / B. Times are in milliseconds.

### Load and reload

| Ledger | Load | Reload p50 | Reload p95 | Reloads, both runs |
|---|---:|---:|---:|---:|
| 10,000 transactions, with budgets | 166 / 162 | 156 / 156 | 161 / 163 | 147 – 163 |
| 10,000 transactions, without budgets | 158 / 160 | 147 / 146 | 151 / 152 | 142 – 152 |
| 100,000 transactions, with budgets | 1,750 / 1,700 | 1,782 / 1,766 | 1,822 / 1,837 | 1,649 – 1,837 |
| 100,000 transactions, without budgets | 1,678 / 1,634 | 1,763 / 1,706 | 1,813 / 1,754 | 1,601 – 1,813 |

With 10,000 transactions, the reloads with budgets were 9 to 10 ms (6 to 7%) slower in both runs: the 1,893 budget directives are 15% of that ledger's directives. With 100,000 transactions, the difference is within the spread of the reloads.

### Requests

The warm requests are 30 samples per run, the cold ones 2 per run, given as the range of the 4.

10,000 transactions:

| Request | Min | p50 | p95 | Max | Cold | Response bytes |
|---|---:|---:|---:|---:|---:|---:|
| `month` | 3.77 / 3.88 | 4.02 / 4.13 | 5.59 / 5.42 | 5.72 / 7.80 | 21 – 23 | 4,917 |
| `month_first` | 2.64 / 2.63 | 2.83 / 2.90 | 3.36 / 5.11 | 4.30 / 5.44 | 19 – 20 | 4,510 |
| `detail` | 3.25 / 3.24 | 3.36 / 3.36 | 5.91 / 4.64 | 6.91 / 5.09 | 18 – 20 | 835 |
| `interval` | 5.03 / 5.08 | 5.28 / 5.27 | 5.58 / 5.61 | 8.03 / 6.02 | 21 – 24 | 6,375 |
| `bql_year` | 3.40 / 3.35 | 3.64 / 3.65 | 5.41 / 5.92 | 6.07 / 6.32 | 19 – 22 | 54,917 |

100,000 transactions:

| Request | Min | p50 | p95 | Max | Cold | Response bytes |
|---|---:|---:|---:|---:|---:|---:|
| `month` | 58.00 / 53.38 | 67.02 / 66.67 | 75.67 / 74.13 | 78.04 / 75.29 | 191 – 211 | 5,107 |
| `month_first` | 57.52 / 56.87 | 62.34 / 65.75 | 70.19 / 75.41 | 70.39 / 76.19 | 185 – 208 | 4,697 |
| `detail` | 59.18 / 58.59 | 66.03 / 65.35 | 73.22 / 73.75 | 73.23 / 77.46 | 174 – 198 | 838 |
| `interval` | 46.52 / 46.89 | 47.46 / 47.61 | 47.88 / 48.86 | 47.92 / 48.93 | 176 – 185 | 68,018 |
| `bql_year` | 60.83 / 58.11 | 67.54 / 69.67 | 74.26 / 77.27 | 77.35 / 77.72 | 200 – 206 | 57,209 |

### Writes

20 timed writes per run, on the ledgers with budgets.

| Ledger | Min | p50 | p95 | Max | Response bytes |
|---|---:|---:|---:|---:|---:|
| 10,000 transactions | 177.7 / 179.0 | 184.7 / 185.5 | 192.3 / 195.8 | 193.9 / 201.3 | 131 |
| 100,000 transactions | 1,917 / 1,908 | 2,045 / 2,061 | 2,097 / 2,115 | 2,097 / 2,143 | 131 |

A write took a reload's time and about 30 ms more with 10,000 transactions, about 280 ms more with 100,000. Besides the reload, it appends the transaction, computes the fingerprint of the reloaded ledger (a pass over its directives that tells the server whether its content changed) and finds the new transaction to answer with its id and version; the bench does not time these apart.

### Memory

| Ledger | RSS before the load | Peak of the load | RSS after the load | Peak after the reloads | Peak after the requests | Peak after the data volume queries | Peak after the writes |
|---|---:|---:|---:|---:|---:|---:|---:|
| 10,000 transactions, with budgets | 15 / 15 | 73 / 73 | 73 / 73 | 158 / 149 | 160 / 165 | 175 / 180 | 248 / 258 |
| 10,000 transactions, without budgets | 15 / 15 | 86 / 86 | 86 / 86 | 118 / 118 | | | |
| 100,000 transactions, with budgets | 38 / 38 | 463 / 464 | 420 / 420 | 811 / 810 | 928 / 902 | 1,030 / 1,007 | 1,438 / 1,443 |
| 100,000 transactions, without budgets | 38 / 38 | 462 / 461 | 419 / 419 | 800 / 798 | | | |

- A loaded ledger of 100,000 transactions takes 420 MiB, with budgets or without. A reload holds the old ledger until the new one is loaded, so the peak after the reloads is about twice that.
- With budgets, the requests raise the peak by 92 to 117 MiB with 100,000 transactions: the cold requests build the query cache of each reloaded ledger, and the queries need working memory. The data volume queries add about 100 MiB more; the largest answers 10 MB of JSON.
- The writes raised the peak by 410 to 440 MiB with 100,000 transactions, although the ledger of the reads is dropped before them. Their reloads run on the server's blocking threads; this bench does not break that memory down further.
- With 10,000 transactions, the ledger without budgets measured 13 MiB more after its load, in every run. That is the memory allocator, not budget state: with 100,000 transactions, the two are within 3 MiB.

### Data volume

What the budget computation reads, and what a plugin would read across its boundary if it asked the query engine for it. The counts are the same in both runs, the times are A / B.

| | 10,000 transactions | 100,000 transactions |
|---|---:|---:|
| Accounts linked to a budget | 47 | 47 |
| Postings of the linked accounts | 10,007 | 104,083 |
| Postings that count toward a budget (`length(budgets) > 0`) | 10,007 | 104,083 |
| Posting and budget pairs (`sum(length(budgets))`) | 10,660 | 111,265 |
| Rows of `#budgets`, every month of every budget | 1,785 | 1,785 |
| The postings of the linked accounts, one row each: rows | 10,007 | 104,083 |
| the same: response bytes | 978,377 | 10,168,343 |
| the same: ms | 23 / 19 | 229 / 230 |
| Sums per account, month, currency and cost currency: rows | 3,024 | 5,877 |
| the same: response bytes | 181,973 | 355,571 |
| the same: ms | 11 / 11 | 84 / 83 |

The two reads are `POST /api/query` with `SELECT date, time, account, units(position) AS units, cost_number, cost_currency WHERE length(account_budgets(account, date)) > 0`, and with `SELECT account, yearmonth(date) AS month, currency, cost_currency, sum(number) AS number WHERE length(account_budgets(account, date)) > 0 GROUP BY account, month, currency, cost_currency`. The response bytes of the pages' requests are in the [Requests](#requests) tables.

## What the numbers say

For the design of phase P1 and the comparison of phase P4:

- **Every `#budgets` request computes every budget over every posting.** With 100,000 transactions, `month_first` costs as much as `month` (p50 62 to 66 ms against 67 ms): `WHERE date <= :month` stops the generation of the later months, but the activity of every budget is first added up over all the postings of its accounts. `detail` costs as much as the whole list (65 to 66 ms), because `name = :name` filters the rows once every budget is computed. Nothing of `#budgets` is kept between requests.
- **The budget requests grow faster than the ledger.** From 10,000 to 100,000 transactions, the load and the writes take 10.5 to 11.4 times longer and `interval` 9 times, but the p50 of the `#budgets` requests 16 to 23 times.
- **The first request after a load pays for the query cache:** 174 to 211 ms cold against 47 to 70 ms warm with 100,000 transactions, 18 to 24 ms against 3 to 5 ms with 10,000. The server reloads the ledger whenever one of its files changes and on every write, so this is the budget page's first answer after every edit.
- **A write costs a whole reload.** 2.0 to 2.1 s with 100,000 transactions, 185 ms with 10,000, under the ledger's write lock. A budget plugin whose derived data is computed at load time adds its time to every write, and one computed by the first query adds it to the first budget request after the write.
- **The months follow the clock.** `#budgets` has a row per budget and month through the latest of the budget's last directive, the last transaction and the current month: 1,785 rows here. A ledger whose last transaction is old still has every month up to today, and a query without a date bound, such as `SELECT count(*) FROM #budgets`, generates them all.
- **The pages need little, the computation reads much.** The pages' answers are 0.8 KB for a budget's detail, 5 KB for a month's list, 6 to 68 KB for a budget's month and 55 to 57 KB for a year in BQL. Reading the postings of the linked accounts row by row is 10.2 MB of JSON with 100,000 transactions; sums per account, month and currency are 29 times smaller. Such sums are not enough on their own: a `budget-close` with a time, a reopened account and the order of the load decide within a day.
- **The spread is in the `#budgets` requests.** Within a run, the samples of a `#budgets` request with 100,000 transactions spread over 14 to 22 ms, while those of `interval`, timed in the same rounds, stay within about 2 ms.

## Reproduce

In a checkout of the commit to measure, with nothing else running if possible:

```sh
# the machine
nproc; lscpu | grep 'Model name'; free -g; uname -r; rustc -V; git rev-parse HEAD

# the four tests, each in a process of its own, one after the other
cargo nextest run --release -p zhang-server --test budget_baseline --run-ignored only --no-capture

# the same, keeping the reports as JSON and the generated ledgers
ZHANG_BUDGET_BASELINE_OUT=/tmp/budget-baseline ZHANG_BUDGET_BASELINE_KEEP=/tmp/budget-ledgers \
  cargo nextest run --release -p zhang-server --test budget_baseline --run-ignored only --no-capture

# the digest of a ledger, to compare with the table of the ledgers
cd /tmp/budget-ledgers/100000tx-30b-60m && LC_ALL=C sha256sum *.zhang | sha256sum
```

Each test prints its figures as a table and as a `BASELINE {...}` line of JSON, with every sample in the order it was taken, the machine and the load average before and after the test. `ZHANG_BUDGET_BASELINE_OUT` writes the same JSON to `<dir>/<ledger>.json`. `ZHANG_BUDGET_BASELINE_KEEP` writes each ledger, as generated, to `<dir>/<ledger>/`, which `zhang serve` can open too.

Without nextest, run one test per process, for example `cargo test --release -p zhang-server --test budget_baseline -- --ignored --exact budget_baseline_100k_transactions_with_budgets`. A test that finds that another test ran in its process before does not report memory.

Run the whole suite at least twice, and compare the plugin with the native implementation in the same session, alternating them.
