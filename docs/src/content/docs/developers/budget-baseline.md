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
The numbers of [After #840/#841](#after-840841), taken on 2026-11-21 on `main` at `1e8bf3e6`, are the baseline for G1 and P4: the budget plugin will run on that server. The sections before it are the first baseline, taken with #836 on `main` at `9c156c7b`, kept as they were.
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

# the load and the writes of one ledger only, to repeat them
ZHANG_BUDGET_BASELINE_ONLY=writes cargo run --release -p zhang --example budget_baseline -- 10000tx-30b-60m

# the digest of a ledger, to compare with the table of the ledgers
cd /tmp/budget-ledgers/100000tx-30b-60m && LC_ALL=C sha256sum *.zhang | sha256sum
```

A ledger prints its figures as a table on stderr and as a `BASELINE {...}` line of JSON on stdout, with every sample in the order it was taken (up to 200 of them), the machine and the load average before and after. `ZHANG_BUDGET_BASELINE_OUT` writes the same JSON to `<dir>/<ledger>.json`. `ZHANG_BUDGET_BASELINE_KEEP` writes each ledger, as generated, to `<dir>/<ledger>/`, which `zhang serve` can open too. `ZHANG_BUDGET_BASELINE_SOURCE=local` reads the files with `LocalFileSystemDataSource` instead of the data source of `zhang serve`, for comparison. `ZHANG_BUDGET_BASELINE_ONLY=writes` measures the load and the writes only. `ZHANG_BUDGET_BASELINE_IDLE=<seconds>` reads the resident memory again after that long idle at the end of the run. The bench uses the allocator of the binary: mimalloc, or the system's with `--no-default-features`.

Run the whole bench at least twice, and compare the plugin with the native implementation in the same session, alternating them.
