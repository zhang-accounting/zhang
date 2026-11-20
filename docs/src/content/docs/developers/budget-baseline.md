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

# the digest of a ledger, to compare with the table of the ledgers
cd /tmp/budget-ledgers/100000tx-30b-60m && LC_ALL=C sha256sum *.zhang | sha256sum
```

A ledger prints its figures as a table on stderr and as a `BASELINE {...}` line of JSON on stdout, with every sample in the order it was taken (up to 200 of them), the machine and the load average before and after. `ZHANG_BUDGET_BASELINE_OUT` writes the same JSON to `<dir>/<ledger>.json`. `ZHANG_BUDGET_BASELINE_KEEP` writes each ledger, as generated, to `<dir>/<ledger>/`, which `zhang serve` can open too. `ZHANG_BUDGET_BASELINE_SOURCE=local` reads the files with `LocalFileSystemDataSource` instead of the data source of `zhang serve`, for comparison.

Run the whole bench at least twice, and compare the plugin with the native implementation in the same session, alternating them.
