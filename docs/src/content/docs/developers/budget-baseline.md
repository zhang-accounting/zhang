---
title: Budget Baseline
description: The recorded performance of the native budget implementation on 10,000 and 100,000 transactions (load and reload, the budget pages' requests, writes and the reloads they cause, concurrent readers, memory, data volume), which the budget plugin of RFC #698 is measured against, and how to reproduce it.
sidebar:
  order: 4
---

[RFC #698](https://github.com/zhang-accounting/zhang/issues/698) moves budgets out of the core and the query engine into an official WASM plugin. Its phase P4 compares the plugin with the native implementation on the same ledgers and the same requests. This page is the baseline of phase P0 it compares against: how long the native implementation, served as `zhang serve` serves a local ledger, takes to load and reload a ledger with budgets and without, to answer the budget pages' requests, alone and eight at once, and to write a transaction, how much memory it uses, and how much data the pages and the budget computation move. It also says how the numbers were taken and how to take them again.

:::caution[Noisy numbers]
The numbers were taken on a build machine shared with other jobs. They are given for two runs, and they vary between runs and within one. Compare the plugin with the native implementation by running both in the same session, as [Reproduce](#reproduce) shows, rather than with this page alone.
:::

:::note[The comparison point]
The official comparison point of G1 and P4 is [End to end](#end-to-end), in [its results with mimalloc](#results-with-mimalloc-the-comparison-point), taken on 2026-11-21 on `main` at `19e68cfc` in valid runs: the time of a write, a reload or a start together with the first budget page or errors page after it, to which the budget plugin may add at most max(5%, 50 ms), and its memory, to which it may add at most max(10%, 40 MiB). Its results before mimalloc, taken on `main` at `fa8e8abb`, are kept as history. The per-request figures of [After #840/#841](#after-840841), taken on `main` at `1e8bf3e6`, describe the same server request by request. The sections before them are the first baseline, taken with #836 on `main` at `9c156c7b`, kept as they were.
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

The bench is `zhang-cli/examples/budget_baseline.rs`, an example of the `zhang` package rather than a test, run with `cargo run --release` in the release profile zhang is shipped with (`opt-level = "s"`); a debug build refuses to run. It runs each ledger in a process of its own, itself again, so the peak memory it reads for a ledger is that ledger's.

**The serve path.** The ledger is served as `zhang serve` serves a local ledger. It is read through `OpendalDataSource` with the `Fs` service, built by the same `OpendalDataSource::from_env` the binary calls. That data source is part of the `zhang` binary, not of a library, so the example compiles the binary's `src/opendal.rs` as a module of its own, beside a stand-in for the two fields of the CLI options it reads (the ledger's root and main file); a test target could not, as it would compile the unit tests that file holds too. The bench starts what `zhang serve` starts: the reload listener (`start_reload_listener`) and the file watcher (`start_fs_event_listener`), and the app with its middleware (`create_server_app`), whose router it sends the requests to, in the process and without a socket. The one difference from `zhang serve` is the clock: the ledger is loaded with a fixed one, noon of 2025-12-31 in the ledger's timezone (`Asia/Shanghai`), so that today, the current month and the months of `#budgets` do not depend on the day of the run.

For each ledger, the bench:

1. Generates the ledger into a temporary directory, checks its digest, and resets the peak resident set size of the process (`5` written to `/proc/self/clear_refs`).
2. Loads it on a blocking thread, as `zhang serve` loads: the **load** time.
3. Reloads it once, untimed, then 20 times through the reload listener (`ReloadSender::reload_and_wait`), as the file watcher asks for a reload: the listener takes the ledger's write lock, loads the files again, computes the fingerprint of the new ledger, replaces the old one and drops it, and answers. These are the **reload** times, from asking to the answer. The server logs each reload's own duration; the two agree within 0.15 ms at the p50.
4. With budgets, reloads it 25 more times, each time followed by one request, the kinds of request in turn, so each kind has 5 **cold** requests: the first query of a loaded ledger also builds the query engine's cache of that ledger (its postings, entries and prices).
5. Sends 3 requests of each kind to warm up, then 30 rounds of one request of each kind, the order of the kinds rotating by one from round to round: the **warm** requests.
6. Runs the queries of [Data volume](#data-volume).
7. Writes 22 transactions through `POST /api/transactions`, 2 to warm up and 20 timed, one at a time: the **writes**. After each answer, a reader sends `budgets.month` request after request until the file watcher's reload of the write is over; the longest of them is the **reader stall**. The bench then waits a second for any further reload before the next write.
8. Runs 8 **concurrent readers**, each sending the kinds of request in turn: first 10 rounds of every kind each while nothing is written, then without stopping while 10 more transactions are written, each once the file watcher's reload of the one before is done.

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

**A write costs two reloads.** `POST /api/transactions` appends the transaction to `data/2025/12.zhang`, the file of the default `directive_output_path` option (the first write creates it and adds its `include` to `main.zhang`), then reloads the ledger in place, synchronously, under the ledger's write lock, before it answers. The file watcher sees the file written and asks the reload listener for a second reload of the same files. It waits 500 ms after the first file event and 100 ms more after each further one; the events of a write include the opens of the files its own reload reads, so a batch held 7 to 20 events (380 of the 540 events of the 40 batches of the timed writes were opens). The second reload therefore starts about 2.4 s after the write's first event: after the answer with 10,000 transactions, and just after it with 100,000, whose own reload is about as long. Every write went through exactly two reloads. The ledger grows by one transaction a write, 32 in a run.

Memory is read from `/proc/self/status`: `VmHWM`, the peak resident set size, and `VmRSS`, the resident set size, in MiB.

## Machine

| | |
|---|---|
| CPU | AMD EPYC-Turin, 96 CPUs on the node (`nproc`), a limit of 64 CPUs for the build pod (cgroup `cpu.max`); the node is shared with other build jobs |
| Memory | 251 GiB on the node, a limit of 128 GiB for the pod; glibc's allocator, as the Linux release builds |
| Kernel | Linux 7.0.0-38-generic |
| Toolchain | rustc 1.100.0 (0ef1bc386 2026-11-09) |
| Code | `main` at `9c156c7b`, with this bench |
| Load average of the node | 7.4 to 16.8 over one minute during the two runs |

## Results

Two runs of the whole bench, A and B, one after the other. A cell gives A / B. Times are in milliseconds.

### Load and reload

| Ledger | Load | Reload p50 | Reload p95 | Reload min | Reload max |
|---|---:|---:|---:|---:|---:|
| 10,000 transactions, with budgets | 188 / 194 | 181 / 198 | 188 / 234 | 170 / 180 | 225 / 240 |
| 10,000 transactions, without budgets | 202 / 177 | 193 / 178 | 206 / 184 | 174 / 164 | 211 / 189 |
| 100,000 transactions, with budgets | 1,651 / 1,771 | 2,238 / 2,252 | 2,389 / 2,521 | 1,962 / 1,962 | 2,415 / 2,642 |
| 100,000 transactions, without budgets | 1,668 / 1,742 | 2,133 / 2,092 | 2,315 / 2,180 | 1,905 / 1,916 | 2,338 / 2,221 |

- With 100,000 transactions, a reload took about 500 ms longer than the first load. Besides loading, it computes the fingerprint of the new ledger and frees the old one; the bench does not time these apart.
- With 100,000 transactions, the reloads with budgets were 5 to 8% slower at the p50; with 10,000, the difference is within the spread of the reloads.

### Requests

The warm requests are 30 samples per run. The cold ones are 5 per kind and run, given as the minimum, the median and the maximum of the 10.

10,000 transactions:

| Request | Min | p50 | p95 | Max | Cold, min / median / max | Response bytes |
|---|---:|---:|---:|---:|---:|---:|
| `month` | 3.85 / 3.88 | 4.03 / 4.96 | 4.57 / 9.19 | 4.65 / 9.27 | 15 / 20 / 32 | 4,917 |
| `month_first` | 2.70 / 2.77 | 2.88 / 3.90 | 3.37 / 7.99 | 3.40 / 8.10 | 12 / 16 / 21 | 4,510 |
| `detail` | 3.25 / 3.31 | 3.38 / 4.48 | 3.74 / 8.78 | 3.77 / 8.80 | 12 / 16 / 31 | 835 |
| `interval` | 4.62 / 4.89 | 4.92 / 5.17 | 5.22 / 6.90 | 5.33 / 7.04 | 14 / 17 / 35 | 6,375 |
| `bql_year` | 3.53 / 3.53 | 3.69 / 4.82 | 4.18 / 8.74 | 4.37 / 8.75 | 12 / 19 / 30 | 54,917 |

100,000 transactions:

| Request | Min | p50 | p95 | Max | Cold, min / median / max | Response bytes |
|---|---:|---:|---:|---:|---:|---:|
| `month` | 66.8 / 65.5 | 72.5 / 76.0 | 89.0 / 91.1 | 89.8 / 91.2 | 148 / 165 / 185 | 5,107 |
| `month_first` | 63.9 / 66.6 | 70.8 / 76.2 | 87.2 / 89.6 | 88.0 / 90.2 | 161 / 167 / 182 | 4,697 |
| `detail` | 66.7 / 63.4 | 73.8 / 78.2 | 90.3 / 91.5 | 90.4 / 91.9 | 161 / 168 / 188 | 838 |
| `interval` | 44.1 / 44.9 | 45.5 / 45.5 | 48.0 / 47.7 | 48.8 / 47.8 | 131 / 137 / 146 | 68,018 |
| `bql_year` | 66.4 / 71.6 | 75.3 / 77.2 | 89.5 / 90.7 | 90.4 / 90.8 | 147 / 165 / 186 | 57,209 |

### Writes

20 timed writes per run, on the ledgers with budgets. The answer of a write is 131 bytes, the new transaction's id and version.

| | 10,000: min | p50 | p95 | max | 100,000: min | p50 | p95 | max |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| Write, with its own reload | 192 / 190 | 213 / 206 | 233 / 234 | 244 / 248 | 1,984 / 1,915 | 2,192 / 2,317 | 2,403 / 3,315 | 2,451 / 5,402 |
| The file watcher's reload | 180 / 173 | 193 / 189 | 211 / 211 | 222 / 239 | 1,940 / 1,906 | 2,185 / 2,197 | 2,416 / 5,167 | 2,448 / 6,068 |
| From the answer to the watcher's reload | 2,187 / 2,189 | 2,221 / 2,229 | 2,242 / 2,248 | 2,250 / 2,252 | 176 / 0 | 398 / 308 | 622 / 489 | 640 / 552 |
| Reader stall: the longest `budgets.month` while the watcher reloads | 195 / 190 | 210 / 209 | 237 / 229 | 241 / 262 | 2,107 / 2,265 | 2,357 / 2,422 | 2,602 / 5,626 | 2,610 / 6,724 |

### Concurrent readers

8 readers, each sending the five kinds of request in turn. Times in milliseconds; while writing, 10 transactions were written over 29 s (10,000 transactions) and 49 s (100,000).

| | Reads | Min | p50 | p95 | Max |
|---|---:|---:|---:|---:|---:|
| 10,000 transactions, nothing written | 400 / 400 | 2.8 / 2.7 | 4.4 / 4.4 | 7.6 / 6.6 | 15.4 / 17.3 |
| 10,000 transactions, while writing | 36,251 / 38,375 | 2.6 / 2.6 | 5.2 / 4.9 | 8.9 / 8.8 | 255.6 / 260.8 |
| 10,000 transactions, the writes meanwhile | 10 / 10 | 207.6 / 208.9 | 226.8 / 223.2 | 248.4 / 258.4 | 248.4 / 258.4 |
| 100,000 transactions, nothing written | 400 / 400 | 43.2 / 43.3 | 69.3 / 68.6 | 90.9 / 94.7 | 221.1 / 237.0 |
| 100,000 transactions, while writing | 643 / 560 | 40.3 / 42.3 | 71.3 / 75.8 | 2,420.6 / 2,453.5 | 2,525.5 / 2,516.7 |
| 100,000 transactions, the writes meanwhile | 10 / 10 | 1,965.2 / 1,872.9 | 2,244.9 / 2,293.0 | 2,490.2 / 2,436.2 | 2,490.2 / 2,436.2 |

### Memory

In MiB, read at the end of each step of the method.

| Ledger | RSS before the load | Peak of the load | RSS after the load | Peak after the reloads | Peak after the requests | Peak after the data volume queries | Peak after the writes | Peak at the end |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| 10,000 transactions, with budgets | 16 / 16 | 94 / 94 | 94 / 94 | 205 / 200 | 217 / 213 | 222 / 224 | 274 / 274 | 598 / 566 |
| 10,000 transactions, without budgets | 16 / 16 | 104 / 105 | 104 / 105 | 181 / 182 | | | | 181 / 182 |
| 100,000 transactions, with budgets | 39 / 39 | 674 / 674 | 646 / 646 | 1,319 / 1,320 | 1,380 / 1,361 | 1,380 / 1,361 | 1,781 / 1,728 | 3,544 / 3,532 |
| 100,000 transactions, without budgets | 39 / 39 | 671 / 672 | 638 / 638 | 1,312 / 1,312 | | | | 1,312 / 1,312 |

- A loaded ledger of 100,000 transactions takes 640 MiB, with budgets or without. A reload holds the old ledger until the new one is loaded, so the peak after the reloads is about twice that.
- Read with `LocalFileSystemDataSource` instead (`ZHANG_BUDGET_BASELINE_SOURCE=local`, one run), the same ledger took 448 MiB after its load and peaked at 850 MiB after the reloads: the data source of `zhang serve` costs about 200 MiB of resident memory per loaded ledger here. That run loaded the ledger in 1.85 s and reloaded it in 2.06 s at the p50.
- The writes, and then the concurrent readers, raise the peak further, to 3.5 GiB with 100,000 transactions, and the process keeps it: reloads and queries run on many threads. This bench does not break that memory down further.
- With 10,000 transactions, the ledger without budgets measured 10 MiB more after its load, in every run. That is the memory allocator, not budget state: with 100,000 transactions, the two are within 8 MiB.

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
| the same: ms | 18 / 18 | 182 / 191 |
| Sums per account, month, currency and cost currency: rows | 3,024 | 5,877 |
| the same: response bytes | 181,973 | 355,571 |
| the same: ms | 11 / 12 | 87 / 93 |

The two reads are `POST /api/query` with `SELECT date, time, account, units(position) AS units, cost_number, cost_currency WHERE length(account_budgets(account, date)) > 0`, and with `SELECT account, yearmonth(date) AS month, currency, cost_currency, sum(number) AS number WHERE length(account_budgets(account, date)) > 0 GROUP BY account, month, currency, cost_currency`. The response bytes of the pages' requests are in the [Requests](#requests) tables.

## What the numbers say

For the design of phase P1 and the comparison of phase P4:

- **Every `#budgets` request computes every budget over every posting.** With 100,000 transactions, `month_first` costs as much as `month` (p50 71 to 76 ms against 73 to 76 ms): `WHERE date <= :month` stops the generation of the later months, but the activity of every budget is first added up over all the postings of its accounts. `detail` costs as much as the whole list (74 to 78 ms), because `name = :name` filters the rows once every budget is computed. Nothing of `#budgets` is kept between requests.
- **The budget requests grow faster than the ledger.** From 10,000 to 100,000 transactions, the load takes 9 times longer, a reload or a write 10 to 12 times, `interval` 9 times, but the p50 of the `#budgets` requests 15 to 25 times.
- **The first request after a load pays for the query cache:** 131 to 188 ms cold against 46 to 78 ms warm at the p50 with 100,000 transactions, 12 to 35 ms against 3 to 5 ms with 10,000.
- **A write costs two reloads.** With 100,000 transactions a write answers after 2.2 to 2.3 s (p50), its own reload included, and the file watcher's reload of the same files follows 0 to 0.6 s later and holds the write lock for another 2.2 s: a reader that comes then waits 2.4 s (p50), and while transactions were written, 8 concurrent readers saw a p95 of 2.4 s against 91 to 95 ms without writes. With 10,000 transactions the watcher's reload comes 2.2 s after the answer and stalls a reader 0.2 s. A budget plugin whose derived data is computed at load time adds its time to both reloads of every write; one computed by the first query adds it to the first budget request after each reload.
- **The months follow the clock.** `#budgets` has a row per budget and month through the latest of the budget's last directive, the last transaction and the current month: 1,785 rows here. A ledger whose last transaction is old still has every month up to today, and a query without a date bound, such as `SELECT count(*) FROM #budgets`, generates them all.
- **The pages need little, the computation reads much.** The pages' answers are 0.8 KB for a budget's detail, 5 KB for a month's list, 6 to 68 KB for a budget's month and 55 to 57 KB for a year in BQL. Reading the postings of the linked accounts row by row is 10.2 MB of JSON with 100,000 transactions; sums per account, month and currency are 29 times smaller. Such sums are not enough on their own: a `budget-close` with a time, a reopened account and the order of the load decide within a day.
- **The spread is in the `#budgets` requests.** Within a run with 100,000 transactions, the 30 samples of a `#budgets` request spread over 19 to 28 ms from the fastest to the slowest, and their p95 is 13 to 17 ms above their p50; those of `interval`, timed in the same rounds, spread over 3 to 5 ms.

## After #840/#841

Measured on 2026-11-21 on `main` at `1e8bf3e6`, with the bench as this section describes it. Besides #840 (a reload loads outside the ledger's write lock) and #841 (the file watcher's reload after a write loads nothing when the files hold what the ledger was loaded from), `main` has by then #842 (fixes of the budget contract), #843 to #845 (the budget tables served by a native provider whose snapshot of a ledger is built by its first budget query and kept), #846 (the reload events and the file watcher hardened) and #847. The ledgers, their digests and the counts of [Data volume](#data-volume) are unchanged.

**These are the baseline for G1 and P4 from now on: the budget plugin will run on this server.** The sections above remain the record of the first baseline (#836, `main` at `9c156c7b`); compare with them only to see what #840 to #847 changed.

### What each number covers now

- **Load**: as before, the first load on a blocking thread, through `OpendalDataSource`, with the fixed clock.
- **Reload, back to back**: `reload_and_wait` through the reload listener, 20 in a row. A reload takes the write cycle (it waits for the writes and reloads before it), waits for the ledger the reload before it replaced to be freed, loads the files and computes the fingerprint holding no lock (readers read the ledger served meanwhile), takes the ledger's write lock only to swap the new ledger in (it waits for the readers in flight), and tells the readers. The replaced ledger is freed afterwards on a blocking thread: not in its own reload's time, but in the next one's when that one comes at once, as here. Before, the whole reload held the write lock, and the old ledger was dropped in it.
- **Reload, spaced**: the same, 20 times, each after a pause of 1 s, so that no freeing is left to wait for: the load, the fingerprint and the swap.
- **Cold and warm requests**: the same timers. The first budget query of a loaded ledger now also builds the budget provider's snapshot of it, which the later ones read.
- **Write**: `POST /api/transactions`, as before, but its load runs with no lock held and only the swap takes the write lock; the ledger it replaced is freed afterwards. **A write now costs one load**: in all 40 timed writes the reload the file watcher asked for afterwards loaded nothing.
- **Reader during the write** (new): a reader sends `budgets.month` back to back while the write runs; the longest. It includes the first read after the swap, which builds the budget snapshot of the new ledger.
- **The file watcher's turn**: the watcher ignores reads now, and gathers events until none came for 500 ms (2 s at most); a write gave it a batch of 6 events (the file created, written, its metadata, renamed). It asks the reload listener for a reload that loads only when a file changed; the listener takes the write cycle, reads the ledger's files again and compares their SHA-256 with what the load read, and loads nothing. *Turn*: from the write's answer to that skip. *Check*: from the watcher's ask to the skip, waiting for the write cycle included. Writers wait for the check, readers do not.
- **Reader during the watcher's turn** (the "reader stall" of before): the longest `budgets.month` sent back to back from the answer until the turn is over.
- **Concurrent readers**: the same 8 readers; while writing, each write waits for the watcher's turn of the one before.

One run (A) of the whole bench, with the node's load average at 1.9 to 15.6 over one minute. Times in milliseconds.

### Load and reload

| Ledger | Load | Back to back p50 | Back to back p95 | Spaced p50 | Spaced p95 | Spaced min – max |
|---|---:|---:|---:|---:|---:|---:|
| 10,000 transactions, with budgets | 192 | 162 | 171 | 163 | 244 | 145 – 252 |
| 10,000 transactions, without budgets | 147 | 156 | 163 | 142 | 184 | 137 – 189 |
| 100,000 transactions, with budgets | 1,606 | 1,947 | 2,001 | 1,832 | 1,883 | 1,725 – 1,889 |
| 100,000 transactions, without budgets | 1,588 | 1,913 | 1,965 | 1,799 | 1,823 | 1,671 – 1,834 |

### Requests

30 warm samples; the cold ones are 5 per kind, given as minimum / median / maximum.

| Request | 10k p50 | 10k p95 | 10k cold | 100k min | 100k p50 | 100k p95 | 100k max | 100k cold | Bytes 10k / 100k |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| `month` | 2.34 | 3.40 | 42 / 46 / 61 | 1.8 | 3.8 | 4.4 | 4.7 | 360 / 393 / 501 | 4,917 / 5,107 |
| `month_first` | 0.29 | 0.59 | 39 / 47 / 56 | 0.2 | 0.3 | 0.9 | 0.9 | 343 / 363 / 394 | 4,510 / 4,697 |
| `detail` | 0.37 | 0.72 | 37 / 50 / 56 | 0.4 | 0.4 | 0.8 | 0.9 | 290 / 372 / 499 | 835 / 838 |
| `interval` | 5.34 | 6.94 | 36 / 47 / 58 | 60.8 | 63.7 | 65.8 | 67.2 | 283 / 358 / 529 | 6,375 / 68,018 |
| `bql_year` | 1.40 | 1.66 | 41 / 53 / 61 | 1.2 | 2.2 | 2.9 | 2.9 | 336 / 417 / 491 | 54,917 / 57,209 |

### Writes

20 timed writes, on the ledgers with budgets.

| | 10k min | p50 | p95 | max | 100k min | p50 | p95 | max |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| Write | 155 | 176 | 200 | 232 | 1,624 | 1,798 | 1,878 | 1,917 |
| Reader during the write | 2.8 | 4.5 | 42.7 | 50.5 | 137 | 227 | 458 | 467 |
| The watcher's turn, from the answer | 340 | 372 | 388 | 391 | 58 | 135 | 252 | 255 |
| The watcher's check | 32 | 36 | 40 | 42 | 58 | 135 | 252 | 255 |
| Reader during the watcher's turn | 1.2 | 4.6 | 41.7 | 41.9 | 1.1 | 2.4 | 4.3 | 6.0 |

With 100,000 transactions the watcher's events come in while the write still loads, so its turn starts as the write answers and is the check alone.

**Noise of the 10,000-transaction write.** The writes alone (`ZHANG_BUDGET_BASELINE_ONLY=writes`) were run 5 more times, each in a process of its own, one after the other, with the node's load average under 2:

| Run | 1 | 2 | 3 | 4 | 5 |
|---|---:|---:|---:|---:|---:|
| p50 | 204 | 172 | 187 | 184 | 193 |
| p95 | 230 | 208 | 228 | 222 | 245 |
| min – max | 153 – 236 | 149 – 221 | 158 – 235 | 158 – 228 | 159 – 247 |

The p50 of a run moved between 172 and 204 ms (median 187), and within a run the 20 writes spread over 70 to 88 ms, their p95 25 to 52 ms above their p50. A difference of less than about 30 ms between two p50s of the 10,000-transaction write is within this noise: compare writes over several runs, alternating the builds.

### Concurrent readers

8 readers, each sending the five kinds of request in turn; while writing, 10 transactions over 5.5 s (10k) and 20.8 s (100k).

| | Reads | Min | p50 | p95 | Max |
|---|---:|---:|---:|---:|---:|
| 10,000 transactions, nothing written | 400 | 0.19 | 1.23 | 4.89 | 12.15 |
| 10,000 transactions, while writing | 26,657 | 0.16 | 1.18 | 4.55 | 46.78 |
| 10,000 transactions, the writes meanwhile | 10 | 165.8 | 177.4 | 191.2 | 191.2 |
| 100,000 transactions, nothing written | 400 | 0.17 | 1.70 | 48.36 | 89.25 |
| 100,000 transactions, while writing | 15,649 | 0.14 | 1.54 | 43.50 | 330.39 |
| 100,000 transactions, the writes meanwhile | 10 | 1,679 | 1,904 | 2,168 | 2,168 |

### Memory

In MiB, read at the end of each step.

| Ledger | RSS before the load | Peak of the load | RSS after the load | Peak after the reloads | Peak after the requests | Peak after the data volume queries | Peak after the writes | Peak at the end |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| 10,000 transactions, with budgets | 16 | 94 | 94 | 206 | 279 | 288 | 285 | 496 |
| 10,000 transactions, without budgets | 16 | 104 | 104 | 179 | | | | 179 |
| 100,000 transactions, with budgets | 39 | 675 | 645 | 1,318 | 1,839 | 1,839 | 2,197 | 3,933 |
| 100,000 transactions, without budgets | 39 | 671 | 638 | 1,312 | | | | 1,312 |

The data volume queries answered as before (same rows and bytes) in 22 and 16 ms (10k) and 268 and 128 ms (100k).

### What changed

Against the first baseline (its two runs, A / B):

| ms | 10k before | 10k now | 100k before | 100k now |
|---|---:|---:|---:|---:|
| reload p50 (now back to back / spaced) | 181 / 198 | 162 / 163 | 2,238 / 2,252 | 1,947 / 1,832 |
| `budgets.month` p50, warm | 4.03 / 4.96 | 2.34 | 72.5 / 76.0 | 3.8 |
| `budgets.month`, cold median | 20 | 46 | 165 | 393 |
| `interval` p50 | 4.92 / 5.17 | 5.34 | 45.5 / 45.5 | 63.7 |
| write p50 | 213 / 206 | 176 (172 – 204 over 5 runs) | 2,192 / 2,317 | 1,798 |
| loads per write | 2 | 1 | 2 | 1 |
| reader while the watcher works, p50 | 210 / 209 | 4.6 | 2,357 / 2,422 | 2.4 |
| 8 readers while writing, p95 | 8.9 / 8.8 | 4.55 | 2,421 / 2,454 | 43.5 |
| RSS after the load (MiB) | 94 | 94 | 646 | 645 |
| peak at the end (MiB) | 598 / 566 | 496 | 3,544 / 3,532 | 3,933 |

- **Readers no longer wait for loads.** With 100,000 transactions, a reader during the watcher's turn took 2.4 ms at the p50 instead of 2.4 s, and 8 readers while writing saw a p95 of 43.5 ms instead of 2.4 s. What is left on a read is the swap, and the first budget request after it, which builds the budget snapshot of the new ledger: a reader during a write waited up to 467 ms.
- **A write costs one load**, as #841 intended: 1.8 s instead of 2.2 to 2.3 s with 100,000 transactions. With 10,000 the difference is within the noise of the write.
- **The budget requests trade the first one for the others.** A warm `#budgets` request reads the snapshot and takes 0.3 to 4 ms instead of 3 to 78 ms; the first budget request of every loaded ledger builds it, which costs about as much as one request cost before (+0.4% of instructions with 100,000 transactions). The server loads the ledger on every write and every edit, so the budget page's first answer after an edit is the cold one.
- **The cold requests and `interval` are slower because of the allocator, not of the budget code.** These numbers were taken with glibc's allocator. Since #840 the ledger a reload replaced is freed on a blocking thread while the requests on the new one run: under glibc that makes the first request after a reload two to three times slower (283 to 529 ms instead of about 160 ms with 100,000 transactions) and leaves allocation-heavy queries such as `interval` about 45% slower afterwards. The same bench on `main` before the budget snapshot (`3d0f9652`) gave the same figures, and so did the instruction counts of the requests (`interval` 636 M instructions at `67c82375`, 624 M on `main`). Freeing the replaced ledger before the request, or another allocator, takes the effect away ([The allocator](#the-allocator)).
- **Memory**: a loaded ledger takes what it took, and the budget snapshot adds about 7 MiB to one of 100,000 transactions. The peak at the end of the run (3.5 to 5.4 GiB between runs) is memory glibc keeps after the reloads freed it ([The allocator](#the-allocator)).

### The allocator

`zhang` uses mimalloc since this comparison; built with `--no-default-features`, it uses the system's allocator. The four allocators below ran the bench of this section on `main` at `fa8e8abb`, two runs each, alternating, with `ZHANG_BUDGET_BASELINE_IDLE=60`, which reads the resident memory again after 60 s idle at the end of the run. Two values are the two runs; a range is the two runs' values.

| | glibc | glibc + `malloc_trim` | mimalloc | jemalloc |
|---|---:|---:|---:|---:|
| 10k: `interval` p50 (ms) | 4.3 / 4.5 | 4.9 / 4.3 | 5.3 / 5.8 | 4.8 / 4.2 |
| 10k: cold requests, min / median / max (ms) | 17 / 43 / 56 | 21 / 44 / 59 | 16 / 20 / 54 | 10 / 13 / 26 |
| 10k: write p50 (ms) | 179 / 170 | 213 / 222 | 165 / 164 | 169 / 153 |
| 10k: reader during a write, p95 (ms) | 45 / 47 | 54 / 55 | 28 / 19 | 25 / 26 |
| 10k: RSS after the load / peak / at the end / 60 s idle later (MiB) | 100 / 555–670 / 555–666 / 533–556 | 103 / 237–320 / 221–278 / 187–204 | 125 / 326–360 / 326–360 / 213–220 | 116 / 366–395 / 311–344 / 97–99 |
| 100k: `interval` p50 (ms) | 41.9 / 66.7 | 68.5 / 70.2 | 44.6 / 47.8 | 45.9 / 46.3 |
| 100k: cold requests, min / median / max (ms) | 171 / 389 / 1,714 | 265 / 399 / 558 | 136 / 156 / 190 | 126 / 139 / 179 |
| 100k: write p50 (ms) | 1,962 / 1,772 | 1,826 / 2,216 | 1,377 / 1,379 | 1,408 / 1,399 |
| 100k: reader during a write, p95 (ms) | 396 / 367 | 399 / 463 | 164 / 163 | 171 / 153 |
| 100k: 8 readers while writing, p95 (ms) | 68.5 / 42.1 | 42.8 / 70.1 | 44.5 / 44.0 | 49.7 / 51.2 |
| 100k: RSS after the load / peak / at the end / 60 s idle later (MiB) | 645 / 4,447–5,424 / 4,372–5,380 / 4,130–5,018 | 645 / 1,659–1,847 / 1,391–1,442 / 822–882 | 417 / 1,403–1,441 / 1,092–1,198 / 505–518 | 624 / 1,391–1,400 / 851–886 / 414 |
| `zhang` binary, stripped (Linux x86_64) | 45,521,264 B | + 0 | + 109,208 B | + 252,376 B |
| Release targets | all | glibc only (a no-op elsewhere) | all: Linux gnu (x86_64, aarch64), macOS, Windows (MSVC), FreeBSD | not Windows (MSVC); aarch64 builds need `JEMALLOC_SYS_WITH_LG_PAGE=16` for 16 KiB and 64 KiB page kernels |

- **glibc keeps what the reloads freed**: 4.1 to 5.0 GiB after 60 s idle with 100,000 transactions, for a ledger of 645 MiB. It is free memory, not live data: `malloc_trim` after each freed ledger gives most of it back (822 to 882 MiB), but makes the first requests and `interval` no faster.
- **mimalloc and jemalloc fix both**: the first request after a reload is about as fast as in a fresh process (136 to 190 ms), a write is a quarter faster (loading is), and the peak is 1.4 GiB instead of 4.4 to 5.4. After 60 s idle mimalloc holds 505 to 518 MiB, jemalloc 414 (97 to 99 MiB with 10,000 transactions, against 213 to 220 for mimalloc).
- **mimalloc is the default**: jemalloc returns a little more memory, but it does not build for Windows (MSVC), and its page size is fixed at build time, so an aarch64 build for 4 KiB pages fails on the 16 KiB and 64 KiB page kernels some ARM hosts run. mimalloc builds for every release target, reads the page size at run time, and holds less memory after loading a large ledger (417 MiB, against 624 for jemalloc and 645 for glibc).

## End to end

**The official comparison point of G1 and P4 is [the results with mimalloc](#results-with-mimalloc-the-comparison-point)**, measured on 2026-11-21 on `main` at `19e68cfc` (with #858) with the bench's end-to-end mode (`ZHANG_BUDGET_BASELINE_ONLY=e2e`), in valid runs only. What the owner of RFC #698 accepts is end to end: the time of a load or a write together with the first budget page or errors page after it. With the budget plugin, each of the times below may grow by at most **max(5%, 50 ms)**, at 10,000 and at 100,000 transactions: at 10,000 transactions that is 50 ms, at 100,000 about 5%. Whether a build stays within it is decided by [the paired rule](#comparing-a-build-with-the-baseline), not by these tables: G1 needs all 12 timers to pass. Its memory may grow by at most **max(10%, 40 MiB)**, judged on the same runs (step 6 of the rule).

:::note[History]
The [results before mimalloc](#results-before-mimalloc-history), taken on `main` at `fa8e8abb` under glibc's allocator, at a load average of 5.4 to 38.0, before the bench loaded as `zhang serve` loads (with the analysis host) and checked its runs' conditions, are kept as a record. Their bands are informational, not acceptance thresholds.
:::

### The timers

Each timer runs 2 rounds to warm up, then 20 timed rounds, one at a time, with no other request meanwhile, on the serve path of the sections above (`OpendalDataSource`, the reload listener and the file watcher running, the fixed clock). A round is timed from sending the write, asking for the reload or starting the load, until the page's answer is read whole.

| Timer | Starts with | Then reads |
|---|---|---|
| (a) write → budgets | `POST /api/transactions`: groceries on `Expenses:Food:Groceries`, paid by card, on 2025-12-30 | `budgets.month` for 2025-12 |
| (b) write → errors | `POST /api/transactions`: a purchase in GBP at an `@` price in USD on `Expenses:Travel:Japan`, paid by card (the `Japan` budget is in JPY, and no price converts GBP to JPY, so zhang reports a `BudgetCommodityMismatch`) | `journals.errors`, as the app's home page asks for it: `size` 100, `offset` 0, `count_total` |
| (c) reload → budgets | an edit of the ledger on disk (the same groceries appended to the month's file, not timed), then a reload through the reload listener, waited for (`ReloadSender::reload_and_wait`, what `POST /api/reload` runs) | `budgets.month` |
| (d) reload → errors | the same with the GBP purchase | `journals.errors` |
| (e) start → budgets | a load of the files on a blocking thread, as `zhang serve` loads at start, and a new app on it (`create_server_app` and its router, without a reload listener or a file watcher) | `budgets.month` |
| (f) start → errors | the same | `journals.errors` |

- **The page answers from the ledger with the change.** A write answers once the ledger with its transaction is swapped in (#818), and a waited-for reload once its ledger is. Each round checks it: the generation of the served ledger moved on by one, and the page's figure moved by what the change adds, the activity of `Groceries` in the month by the amount written, or the number of errors by one. A start checks that its page gives the figure of the ledger served.
- **Loaded as `zhang serve` loads.** Every load, the first, a start's, the reload listener's and a write's, runs with the server's load options (`zhang_server::analysis::load_options`: the host of the analyzers' analyses, the default plugins, the operator's plugin policy, here the default one), with the fixed clock instead of the system's; a plugin's analysis is therefore timed with the load it runs in. A run records the analyzers of the ledger and what each published after the first load and at the end (`zhang:budget: ok`, `failed (…)`, `skipped`), and the providers of the tables `GET /api/query/schema` lists.
- **What it covers.** A write's or a reload's own time (the append, the load outside the write lock, the swap), then the page: the first query of a loaded ledger builds the query engine's cache of it and, for `budgets.month`, the budget snapshot, while the replaced ledger is being freed on another thread. A start covers a load and the app's construction, not the process start nor the reading of cold files (the files are in the OS cache).
- **The file watcher.** It sees every write and edit; it gathers events until none came for 500 ms, then asks for a reload that loads nothing, as the files hold what the ledger was loaded from. The bench waits for that turn of the watcher, and 1 s more, before the next round. With 100,000 transactions the watcher's check may run while the page of (a) or (b) is read: readers do not wait for it, but it shares the machine, as under `zhang serve`. (c) and (d) ask for the reload right after the edit: an edit that only the watcher reloads adds its quiet period, at least 500 ms, before the reload.
- **Rounds the watcher overlaps.** The bench tells from the server's log when the watcher gathered a batch of changes, asked for a reload, or had one skipped, and whether a reload that was not the round's own started. A round in which any of that happened before the round's own write or reload started (or at all, in a start) may have waited for the watcher: it is **excluded** and run again, up to twice the rounds in all, so that each timer keeps 20 clean rounds. A round in which it happened only once the round's own reload had started is the watcher looking at the round's own write or edit, as under `zhang serve`: it is **kept**. The report gives both counts per timer, and the watcher's state of every timed round. In sample runs on `main`, no round overlapped with 10,000 transactions, and with 100,000 every round of (a) to (d) did, during (the watcher's batch comes 500 ms after the write's or edit's file events, while the round's own load still runs). (The results before mimalloc were taken before these flags.)
- **A valid run.** A run is pinned with `taskset` to the CPUs `ZHANG_BUDGET_BASELINE_CPUS` names (`8-15` by default; `none` pins nothing), server and client alike, as they are one process, and serves with `ZHANG_BUDGET_BASELINE_WORKERS` worker threads (8). It is **invalid** when a build ran in the machine when it started or when it ended (a `rustc`, `cargo`, `cc`, `ld` or similar process other than the bench and the `cargo run` that started it, zombies aside), when the cgroup it runs in was throttled during it (`cpu.stat`, `throttled_usec` moved), or when tasks of that cgroup waited for a CPU 2% of its time or more (`cpu.pressure`, the `some` total); a run whose cgroup files cannot be read is invalid too. The load average is only a warning, above half the machine's CPUs: in a pod, `/proc/loadavg` is the node's. The report gives every figure and the reasons of an invalid run.
- The ledger grows by one transaction a round of (a) to (d), 88 in a run.

### Results with mimalloc: the comparison point

Measured on 2026-11-21 on `main` at `19e68cfc` (mimalloc, #858; zhang:budget as a default plugin, #863), with the bench at `00008aab` of this branch, in one `kbuild --bench` window. The e2e mode ran 12 times with 10,000 transactions (6 pairs of the same build, the self-comparison below) and 3 times with 100,000, each in a process of its own, one after the other, pinned to CPUs 8-15 with 8 worker threads. **Every run was valid**, at its first attempt: no build ran at its start or end, its cgroup was never throttled, and its tasks waited for a CPU 0.012% to 0.021% of the time; the node's load average, advisory only, was 8 to 22 over one minute. No analyzer published a snapshot (the native implementation; the tables of `zhang:budget` come from its native provider), the plugin level was `read`, the allocator mimalloc. No round had to be excluded for the file watcher; with 100,000 transactions every round of (a) to (d) had the watcher during its own reload, as expected. *p50* and *p95* are the medians of the runs' p50 and p95; the bands are the lowest and the highest of the runs' p50 and p95; *min – max* is over every sample of every run; *trigger + page* splits the median p50 into the write, reload or start and the page after it. *Allowed* is what the plugin may add: max(5% of the p50, 50 ms). Times in milliseconds.

10,000 transactions (12 runs):

| Timer | p50 | p95 | Run p50s | Run p95s | Min – max | Trigger + page | Allowed |
|---|---:|---:|---:|---:|---:|---:|---:|
| (a) write → budgets | 164.0 | 173.6 | 160.7 – 166.5 | 165.4 – 187.2 | 151.4 – 270.2 | 148.7 + 14.6 | 50 |
| (b) write → errors | 155.6 | 167.5 | 153.4 – 159.6 | 158.0 – 176.4 | 145.2 – 180.9 | 155.4 + 0.3 | 50 |
| (c) reload → budgets | 154.6 | 162.7 | 150.4 – 157.5 | 155.4 – 192.3 | 143.3 – 193.2 | 140.0 + 14.4 | 50 |
| (d) reload → errors | 145.9 | 158.6 | 142.3 – 153.0 | 149.4 – 165.0 | 136.8 – 168.1 | 145.5 + 0.4 | 50 |
| (e) start → budgets | 155.7 | 163.3 | 149.2 – 162.4 | 159.8 – 179.4 | 139.6 – 188.9 | 140.8 + 15.1 | 50 |
| (f) start → errors | 140.8 | 148.1 | 132.1 – 145.3 | 142.7 – 154.0 | 129.4 – 167.7 | 140.1 + 0.6 | 50 |

100,000 transactions (3 runs):

| Timer | p50 | p95 | Run p50s | Run p95s | Min – max | Trigger + page | Allowed |
|---|---:|---:|---:|---:|---:|---:|---:|
| (a) write → budgets | 1,564 | 1,615 | 1,548 – 1,576 | 1,614 – 1,616 | 1,478 – 1,627 | 1,411 + 151 | 78 |
| (b) write → errors | 1,413 | 1,445 | 1,403 – 1,422 | 1,442 – 1,469 | 1,357 – 1,489 | 1,413 + 0.4 | 71 |
| (c) reload → budgets | 1,520 | 1,550 | 1,512 – 1,526 | 1,548 – 1,568 | 1,477 – 1,579 | 1,367 + 146 | 76 |
| (d) reload → errors | 1,353 | 1,386 | 1,352 – 1,358 | 1,373 – 1,400 | 1,325 – 1,409 | 1,353 + 0.5 | 68 |
| (e) start → budgets | 1,475 | 1,499 | 1,467 – 1,487 | 1,489 – 1,525 | 1,439 – 1,546 | 1,323 + 150 | 74 |
| (f) start → errors | 1,334 | 1,370 | 1,332 – 1,348 | 1,361 – 1,375 | 1,298 – 1,378 | 1,334 + 0.7 | 67 |

- With valid runs, the runs' p50s spread over 6 to 13 ms with 10,000 transactions and 6 to 28 ms with 100,000, well within the allowances; the earlier bands (25 to 200 ms) were the shared machine's.
- Against the results before mimalloc, the p50s are 13 to 31 ms lower with 10,000 transactions and 25 to 29% lower with 100,000; the first budget page after a change still adds about 15 ms and 150 ms.
- **A self-comparison** of this build, the 12 runs at 10,000 transactions as 6 pairs, by `budget_compare`: PASS for (a), (b), (d) and (f), with median differences of −0.4 to +5.4 ms and 95% intervals within [−2.8, +8.8] ms; INCONCLUSIVE for (c) and (e), which need 10 pairs; no FAIL. With `--expect-analyzer zhang:budget` the comparison is refused, as no candidate run shows that analyzer publishing.

### Results before mimalloc (history)

Measured on 2026-11-21 on `main` at `fa8e8abb`, under glibc's allocator. The e2e mode ran 5 times with 10,000 transactions and 3 times with 100,000, each in a process of its own, one after the other, with the node's load average at 5.4 to 38.0 over one minute. *p50* and *p95* are the medians of the runs' p50 and p95; the bands are the lowest and the highest of the runs' p50 and p95; *min – max* is over every sample of every run; *trigger + page* splits the median p50 into the write, reload or start and the page after it. *Allowed* is what the plugin may add: max(5% of the p50, 50 ms). Times in milliseconds.

10,000 transactions:

| Timer | p50 | p95 | Run p50s | Run p95s | Min – max | Trigger + page | Allowed |
|---|---:|---:|---:|---:|---:|---:|---:|
| (a) write → budgets | 192 | 283 | 183 – 208 | 215 – 405 | 168 – 406 | 175 + 16.9 | 50 |
| (b) write → errors | 184 | 231 | 170 – 207 | 189 – 349 | 155 – 431 | 184 + 0.3 | 50 |
| (c) reload → budgets | 183 | 234 | 173 – 229 | 205 – 339 | 156 – 340 | 166 + 15.4 | 50 |
| (d) reload → errors | 159 | 196 | 157 – 165 | 174 – 206 | 144 – 441 | 159 + 0.4 | 50 |
| (e) start → budgets | 176 | 219 | 169 – 269 | 184 – 337 | 161 – 343 | 162 + 14.4 | 50 |
| (f) start → errors | 172 | 188 | 164 – 181 | 169 – 224 | 154 – 250 | 172 + 0.6 | 50 |

100,000 transactions:

| Timer | p50 | p95 | Run p50s | Run p95s | Min – max | Trigger + page | Allowed |
|---|---:|---:|---:|---:|---:|---:|---:|
| (a) write → budgets | 2,091 | 2,452 | 1,999 – 2,107 | 2,238 – 4,776 | 1,780 – 6,536 | 1,832 + 185 | 105 |
| (b) write → errors | 1,885 | 1,962 | 1,811 – 1,887 | 1,924 – 2,446 | 1,712 – 2,931 | 1,857 + 36 | 94 |
| (c) reload → budgets | 2,105 | 2,328 | 1,910 – 2,107 | 1,950 – 2,557 | 1,854 – 2,673 | 1,912 + 166 | 105 |
| (d) reload → errors | 1,834 | 1,933 | 1,821 – 1,846 | 1,887 – 1,939 | 1,634 – 2,172 | 1,834 + 0.6 | 92 |
| (e) start → budgets | 2,040 | 2,077 | 1,960 – 2,053 | 2,029 – 2,085 | 1,927 – 2,095 | 1,877 + 154 | 102 |
| (f) start → errors | 1,882 | 1,959 | 1,832 – 1,911 | 1,885 – 1,960 | 1,794 – 1,980 | 1,881 + 1.0 | 94 |

- The time is the load's: the first budget page after it adds 14 to 17 ms with 10,000 transactions and 154 to 185 ms with 100,000 (the query engine's cache and the budget snapshot of the new ledger); the errors page adds under 2 ms, and with 100,000 transactions 36 ms after a write.
- **Noise against the allowance.** The runs' p50s moved by up to 25 ms for (a) and 100 ms for (e) with 10,000 transactions, and by up to 200 ms for (c) with 100,000; the p95s moved more. That is as wide as the allowance, or wider, so neither a single run against these tables nor a comparison with their bands can tell a regression: a build is compared with the native implementation in pairs, as the next section says.
- The full bench (`cargo run --release -p zhang --example budget_baseline`) also runs these timers, after its other steps; its process has by then grown (to 0.9 GiB with 10,000 transactions and 4.3 GiB with 100,000) and its times are noisier, as (e) shows (290 ms at 10,000 in that run). The comparison point is the e2e mode.

### Comparing a build with the baseline

A build (the budget plugin) is compared with the native implementation run against run, on the same machine, in one session, with valid runs only:

1. Run the e2e mode of each build in **pairs**, alternating them (native, candidate, native, candidate, …, or in a random order within each pair), or the two runs of a pair at the same time, as [the fast procedure of P4](#the-fast-procedure-of-p4) does, each run writing its report to a directory of its own:
   ```sh
   ZHANG_BUDGET_BASELINE_ONLY=e2e ZHANG_BUDGET_BASELINE_OUT=/tmp/cmp/native/01 \
     cargo run --release -p zhang --example budget_baseline -- 10000tx-30b-60m      # on the native build
   ZHANG_BUDGET_BASELINE_ONLY=e2e ZHANG_BUDGET_BASELINE_OUT=/tmp/cmp/candidate/01 \
     cargo run --release -p zhang --example budget_baseline -- 10000tx-30b-60m      # on the candidate build
   # then 02, 03, ...
   ```
2. For each timer, take the difference of the two runs' p50 in each pair (candidate − native), and an interval of the **median difference** from the order statistics: with `n` pairs sorted, `[d(k), d(n+1−k)]` covers the median with probability `1 − 2·P(Binomial(n, ½) < k)`, whatever the distribution of the differences. Take the largest `k` with a coverage of at least **95%**:

   | Pairs | 5 | 6 | 7 | 8 | 9 | 10 | 11 | 12 | 13 | 14 | 15 |
   |---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
   | `k` | – | 1 | 1 | 1 | 2 | 2 | 2 | 3 | 3 | 3 | 4 |
   | coverage | – | 96.9% | 98.4% | 99.2% | 96.1% | 97.9% | 98.8% | 96.1% | 97.8% | 98.7% | 96.5% |

   No interval reaches 95% with 5 pairs; with 6 to 8 it is from the lowest to the highest difference.
3. With the allowance max(5% of the median native p50, 50 ms): the timer **passes** when the interval's upper end is below the allowance, and **fails** when its lower end is above it. Otherwise it is **inconclusive**: add pairs, up to 15, and report it inconclusive if it still is. **False PASS ≤ 5% per timer, optional stopping included**: a timer whose true median difference is the allowance passes in at most 4.1% of comparisons that start looking at 5 pairs, and 2.8% of those that start at 10.
4. A verdict needs at least **10 pairs** for (c) and (e) with 10,000 transactions and for (a) and (c) with 100,000, whose run p50s spread wider than the allowance in the results before mimalloc, and at least **5 pairs** for the others (so 6, the first with an interval); 10 also for any timer whose native runs in the comparison spread wider than the allowance.
5. **G1 needs all 12 timers to pass**: the six timers with 10,000 and with 100,000 transactions. A single FAIL among the 12 is a reason to run that timer again (new pairs), not an immediate rejection.
6. **Memory** is judged on the same runs, per ledger: each figure of a report's `memory_mib` (the resident memory after the load, the peaks of the load, after the reloads and at the end, the resident memory at the end), the median of the candidate's runs against the median of the native runs, may grow by at most **max(10%, 40 MiB)**. The floor of 40 MiB is the owner's decision of 2026-11-22 (RFC #698, decision 16), like the floor of 50 ms of the time: the plugin runtime holds memory of its own whatever the size of the ledger, which 10% alone counted against a ledger of 10,000 transactions (10% of its 156 MiB peak through a reload is 15.6 MiB). At 100,000 transactions the 10% decides. `budget_compare` judges it after the timers.
7. **The named-query p95** is judged on runs of the `requests` mode, per ledger and kind of warm request (the six of [Requests](#requests)): in each pair, the ratio of the candidate's p95 to the native p95, and the interval of the median ratio as in step 2. The request **passes** when the interval's upper end is at most **1.1**, **fails** when its lower end is above it, and is **inconclusive** otherwise. It is judged on **60 pairs**, all run before the judgement (no pairs are added: a false PASS is then at most 1.4%), of runs of **200 rounds**. A `requests` run takes a few seconds, and most of these requests answer in under a millisecond, whose p95 moves by 10 to 50% from one process to the next whatever the rounds: with 15 pairs of 30 rounds, as G1 and P3 had, the interval of a sub-millisecond request is wider than the bar. `budget_compare` prints G1's reading beside it, the ratio of the medians of the runs' p95, to compare with G1 and P3.

`budget_compare` applies the rule to the reports:

```sh
cargo run --release -p zhang --example budget_compare -- /tmp/cmp/native /tmp/cmp/candidate --expect-analyzer zhang:budget
```

It reads every report under each directory in the order of their paths and pairs the runs of each ledger by that order, the runs with end-to-end timers for the timers and the memory, the runs with warm requests for the p95. It prints each side's setup (the build, the commit, the mode, the analyzers and what they published, the table providers, the ledger, the number of CPUs, the worker threads and the rounds of warm requests) and **refuses** the comparison when the runs of a side disagree on any of them, when the sides ran in another mode, on another number of CPUs or worker threads, other rounds or ledgers, or when a candidate run does not show the analyzer `--expect-analyzer` names publishing (`zhang:budget: ok`) after its first load and at its end. It skips a pair with an invalid run and says why (`--include-invalid` keeps them, for a diagnosis only, never for a verdict). Per timer it prints the pairs, the median native p50, the median difference, its interval and coverage, the allowance, each side's rounds excluded and kept for the file watcher, and the verdict, with how many more pairs an inconclusive timer needs; a timer that a run measured with fewer than its 20 clean rounds is **refused**. Then, per memory figure, each side's median, the difference and what is allowed; per kind of warm request, each side's median p95, G1's ratio, the median ratio and its interval, and the verdict; and how far apart the two runs of a pair started. It exits with a failure when a timer, a memory figure or a p95 fails or a comparison is refused. `--summary <file>` writes the verdicts as JSON too.

On the build pod, run timed comparisons in a bench window, which waits until no build runs and holds every other build off until the command ends (one bench at a time): build the examples first, then run the built binaries under the window, so that it holds nobody off while compiling.

```sh
kbuild <lane> <worktree> 'CARGO_TARGET_DIR=/cache/target/<lane> cargo build --release -p zhang --examples'
kbuild --bench <lane> --no-sync 'ZHANG_BUDGET_BASELINE_ONLY=e2e ZHANG_BUDGET_BASELINE_OUT=/tmp/cmp/native/01 \
  /cache/target/<lane>/release/examples/budget_baseline 10000tx-30b-60m'
```

### The fast procedure of P4

P4 judges three bars at 10,000 and 100,000 transactions, each with the pairs of the rule: the 12 end-to-end timers (steps 1 to 5), the memory of the same runs (step 6) and the named-query p95 (step 7). Run one after the other, as G1's were, the runs take hours: an `e2e` run takes about 2.4 minutes with 10,000 transactions and 4.5 with 100,000, and a full run, which G1 and P3 took the p95 from, about 4 and 8.5: 10 pairs of each at both sizes are about 2.3 hours of `e2e` runs and 4.2 hours of full runs, and P3's p95 measurement, mostly one run after the other, took about 4 hours. The fast procedure runs only what each bar needs, and runs the pairs at the same time.

**A mode for each bar.** `ZHANG_BUDGET_BASELINE_ONLY=e2e` runs the timers, and its reports hold the memory step 6 judges: their figures at the end come after the timers, so the memory is not measured apart. `ZHANG_BUDGET_BASELINE_ONLY=requests` runs the warm requests alone: the load, the reload every mode starts with (not timed), 3 requests of each kind to warm up, then the timed rounds, 30 as in the full mode unless `ZHANG_BUDGET_BASELINE_ROUNDS` says otherwise (`budget_pairs` asks for 200); nothing else runs in the process before them. With 200 rounds, a `requests` run takes about 2 seconds with 10,000 transactions and 11 with 100,000. Each report records its `mode`, and `budget_compare` refuses to pair runs of different modes or rounds.

**The runner.** `budget_pairs` (`zhang-cli/examples/budget_pairs.rs`) runs the pairs of every mode and ledger and judges them:

- **A pair's two runs run at the same time**, each pinned to a set of 8 CPUs of its own, with 8 worker threads. `--cpus 16-63` (the default) is cut into 6 sets, so 3 pairs run at once, 48 CPUs in all, below the build pod's quota of 64. The two sides swap the sets of their slot from pair to pair, so that no side keeps the faster set.
- **Every mode and ledger gets its valid pairs**: `--pairs` of the `e2e` mode (10 by default), `--requests-pairs` of the `requests` mode (60, each of `--rounds` 200). A pair with a run the bench marked invalid (a build ran, the cgroup was throttled, its tasks waited for a CPU for 2% of the time), a timer with fewer than its 20 clean rounds, or a run that failed, is moved to `<out>/invalid/` and run again, up to `--max-invalid` times.
- **Then it runs `budget_compare`** on what it collected (`<out>/native`, `<out>/candidate`, one directory per mode, ledger and pair) and, while a verdict of the timers is inconclusive and its ledger has fewer than `--most-pairs` valid `e2e` pairs (15), runs at least 3 more pairs and judges again, as step 3 allows; the p95 gets no more than its 60. It prints the judgement and keeps it in `<out>/compare.txt` and `<out>/summary.json`, records every pair it ran in `<out>/runs.jsonl` (the CPUs of each run, its exit, its validity), and exits with the status of `budget_compare`. Started again with the same `--out`, it keeps the valid pairs it finds and runs only what is missing.

The whole P4 set, in one bench window, with one binary for both sides told apart by their environment (`ZHANG_BUDGET_IMPL` in a build with the `budget-switch` feature), or with two binaries (`--candidate <path>`):

```sh
kbuild <lane> <worktree> 'CARGO_TARGET_DIR=/cache/target/<lane> cargo build --release -p zhang --examples --features budget-switch'
kbuild <lane> --no-sync 'mkdir -p /cache/<lane>-bin && cp /cache/target/<lane>/release/examples/budget_{baseline,compare,pairs} /cache/<lane>-bin/'
kbuild --bench <lane> --no-sync '/cache/<lane>-bin/budget_pairs --out /cache/<lane>-p4 \
  --native /cache/<lane>-bin/budget_baseline --env ZHANG_BUDGET_BASELINE_COMMIT=<commit> \
  --native-env ZHANG_BUDGET_IMPL=native --candidate-env ZHANG_BUDGET_IMPL=plugin \
  -- --expect-analyzer zhang:budget'
```

The defaults are the P4 set: `--ledgers 10000tx-30b-60m,100000tx-30b-60m --modes e2e,requests --pairs 10 --most-pairs 15 --requests-pairs 60 --rounds 200 --cpus 16-63 --cpus-per-side 8`. Copy the binaries out of the target directory first, so that a build of the lane cannot replace them during the run. A run that outlives the terminal can be started detached in the pod (`nohup setsid`), behind a script that takes the bench window as `kbuild --bench` does.

**Why pairing at the same time is sound.** The rule compares the two runs of a pair with each other, never a run with this page's tables: what it needs is that both runs of a pair meet the same machine. Run one after the other, they do not quite: the VM's memory has slow periods of 0.2 to 2 seconds (G1 found them, #870) and the node's other tenants come and go, so the second run of a pair can meet another machine than the first. Two runs started together go through the same seconds, the same slow periods and the same neighbours, and the difference of their p50s or the ratio of their p95s cancels what they share. Each run keeps what made a run trustworthy: CPUs of its own (a run never waits for the other's CPUs, which the validity check confirms: no throttling, under 2% of CPU pressure), its own process, ledger directory and file watcher, and the same 20 clean rounds. Swapping the sets between the sides from pair to pair keeps a difference between two sets of CPUs out of the median.

**Its limits.**

- **The runs share the machine.** Up to six runs at once share the memory bandwidth and the L3 caches (the vCPUs of two sets may sit on the same CCD), and the pairs of the other slots load them more. A run's absolute times are therefore not comparable with the sequential results of this page, or with runs at another concurrency: compare only the paired differences and ratios, in the same session. In the self-comparisons below, the timers' native p50s came out between the same and 9% higher than in two runs of the same binary made one at a time with nothing else running.
- **The two sides of a pair disturb each other a little.** A side that moves more memory slows the other side by a little, which shrinks a difference that is mostly memory traffic. When a verdict lands close to its bar, confirm it with pairs run one at a time (`--cpus 16-31` runs a single pair at once, its two sides still simultaneous), or one after the other as in step 1.
- **The validity check reads the pod's cgroup, which all the runs share.** Throttling or CPU pressure caused by any run invalidates every run that overlapped it: the check is stricter, not looser. The memory figures are each process's own and are not affected.

**Self-comparison.** `main` (`362a602f`, with this tooling) against itself, one binary on both sides, in one bench window on the build pod on 2026-11-23, `budget_pairs` with its defaults: 10 + 10 `e2e` pairs and 60 + 60 `requests` pairs of 200 rounds, at 10,000 and 100,000 transactions, in **28 minutes** (24 for the `e2e` pairs, 4 for the `requests` pairs). One run after the other, the `e2e` runs alone would take about 2.3 hours. Every run was valid at its first attempt: never throttled, CPU pressure 0.017% at most, the two runs of each pair started within 0.1 s of each other.

- **The timers**: all 12 PASS on 10 pairs, so none needed more, with median differences of −0.4 to +3.1 ms at 10,000 transactions and −9.0 to +6.0 ms at 100,000. Their intervals are 3.3 to 14.8 ms wide at 10,000 transactions, against 9.8 to 60.1 ms for 10 of G1's sequential pairs under the same rule, and 15.7 to 64.4 ms at 100,000, against 20.3 to 77.2. The write and reload timers at 10,000 transactions, which fault their memory in afresh and so meet the VM's slow periods, are 2 to 18 times narrower; the start timers are as wide as before. At 100,000 the write timers are narrower, and the reload and start timers as wide or, in one of two such self-comparisons, up to twice as wide: six loads of 100,000 transactions at once share the memory bandwidth (the first limit). Every upper end stayed below half its allowance.
- **The memory**: every figure within 2.6 MiB, but the resident memory at the end at 10,000 transactions (−6.9 MiB), the noisiest figure: PASS.
- **The p95**: 10 of the 12 requests PASS. `month_first` at 100,000 transactions ([0.94, 1.12]) and `errors` at 10,000 ([0.95, 1.11]), whose p95s are 0.34 and 0.08 ms, stay inconclusive; nothing fails. An earlier self-comparison with 15 pairs of 30 rounds, as G1 and P3 measured the p95, left 6 of the 12 inconclusive, with intervals about as wide as P3's sequential pairs (up to [0.99, 1.80]) but for `month` and `interval`: pairing at the same time does not narrow the p95 of a sub-millisecond request, which moves from process to process; more pairs and more rounds do.

### The errors page among the requests

`journals.errors` (as the app sends it, the first 100 errors with their total) is now one of the kinds of request the warm and cold requests time; the concurrent readers keep sending the five budget kinds, as before. In one full run on `fa8e8abb`:

| | 10,000 transactions | 100,000 transactions |
|---|---:|---:|
| warm, min / p50 / p95 / max | 0.10 / 0.32 / 0.54 / 0.55 | 0.17 / 0.78 / 1.71 / 1.79 |
| cold, min – max of 5 | 0.30 – 0.38 | 1.03 – 1.61 |
| response bytes (the ledger's 2 errors) | 600 | 606 |

## Reproduce

In a checkout of the commit to measure, with nothing else running if possible:

```sh
# the machine
nproc; lscpu | grep 'Model name'; free -g; uname -r; rustc -V; git rev-parse HEAD

# the four ledgers, each in a process of its own, one after the other
cargo run --release -p zhang --example budget_baseline

# one ledger
cargo run --release -p zhang --example budget_baseline -- 100000tx-30b-60m

# the four, keeping the reports as JSON and the generated ledgers
ZHANG_BUDGET_BASELINE_OUT=/tmp/budget-baseline ZHANG_BUDGET_BASELINE_KEEP=/tmp/budget-ledgers \
  cargo run --release -p zhang --example budget_baseline

# the load and the end-to-end timers of one ledger only, the G1/P4 comparison point
ZHANG_BUDGET_BASELINE_ONLY=e2e cargo run --release -p zhang --example budget_baseline -- 100000tx-30b-60m

# two sets of such runs compared by the paired rule (on the build pod, run the runs in a bench window:
# kbuild --bench, see "Comparing a build with the baseline")
cargo run --release -p zhang --example budget_compare -- /tmp/cmp/native /tmp/cmp/candidate --expect-analyzer zhang:budget

# the load and the warm requests of one ledger only, the named-query p95 of P4
ZHANG_BUDGET_BASELINE_ONLY=requests cargo run --release -p zhang --example budget_baseline -- 100000tx-30b-60m

# the whole P4 set, its pairs run two sides at a time, then judged (in a bench window on the build pod: see
# "The fast procedure of P4")
cargo build --release -p zhang --examples
target/release/examples/budget_pairs --out /tmp/p4 --native target/release/examples/budget_baseline \
  --native-env ZHANG_BUDGET_IMPL=native --candidate-env ZHANG_BUDGET_IMPL=plugin -- --expect-analyzer zhang:budget

# the load and the writes of one ledger only, to repeat them
ZHANG_BUDGET_BASELINE_ONLY=writes cargo run --release -p zhang --example budget_baseline -- 10000tx-30b-60m

# the digest of a ledger, to compare with the table of the ledgers
cd /tmp/budget-ledgers/100000tx-30b-60m && LC_ALL=C sha256sum *.zhang | sha256sum
```

A ledger prints its figures as a table on stderr and as a `BASELINE {...}` line of JSON on stdout, with every sample in the order it was taken (up to 200 of them), the machine and the load average before and after. `ZHANG_BUDGET_BASELINE_OUT` writes the same JSON to `<dir>/<ledger>.json`. `ZHANG_BUDGET_BASELINE_KEEP` writes each ledger, as generated, to `<dir>/<ledger>/`, which `zhang serve` can open too. `ZHANG_BUDGET_BASELINE_SOURCE=local` reads the files with `LocalFileSystemDataSource` instead of the data source of `zhang serve`, for comparison. `ZHANG_BUDGET_BASELINE_ONLY=writes` measures the load and the writes only, `ZHANG_BUDGET_BASELINE_ONLY=e2e` the load and the end-to-end timers only, `ZHANG_BUDGET_BASELINE_ONLY=requests` the load and the warm requests only; another value is refused. `ZHANG_BUDGET_BASELINE_ROUNDS=<n>` sets the rounds of the warm requests (30). Every run writes its report to `ZHANG_BUDGET_BASELINE_OUT` when it is set, a single ledger's too. A run is pinned to `ZHANG_BUDGET_BASELINE_CPUS` (`8-15`) with `ZHANG_BUDGET_BASELINE_WORKERS` worker threads (8), records the commit `ZHANG_BUDGET_BASELINE_COMMIT` names, and is marked invalid when a build, throttling or CPU pressure disturbed it. `ZHANG_BUDGET_BASELINE_IDLE=<seconds>` reads the resident memory again after that long idle at the end of the run. The bench uses the allocator of the binary: mimalloc, or the system's with `--no-default-features`.

Run the whole bench at least twice, and compare the plugin with the native implementation in the same session, alternating them.
