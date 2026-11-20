---
title: "RFC #698 P1: Budget as a Plugin"
description: "The P1 design of RFC #698, revised with the owner's decisions: where budget semantics live today, an Analyzer extension point with per-plugin snapshots, plugin-provided query tables, exact decimals for scripts, the budget syntax as a compatibility mapping, the default plugin zhang:budget in Lua, and a twelve-PR plan."
sidebar:
  order: 5
---

:::note[Status: every decision recorded on 2026-11-20]
Nothing on this page is implemented. It answers the questions RFC
[#698](https://github.com/zhang-accounting/zhang/issues/698) leaves to P1, against `main` at `9c156c7b`. File
references are `path:line` at that commit.

The owner recorded every decision of [§11](#11-decisions) on 2026-11-20. The page applies them and the findings of an
independent critique of its first version.
:::

## Summary

1. **The budget syntax stays in the core as a compatibility mapping; every rule leaves it.**
   - The core and the query engine keep the four `budget*` directives, their export, their spans and their Beancount
     `custom` form.
   - They lose `process/budget.rs`, the budget state of the fold, the three budget tables, the `budgets` column and
     the membership rule.
2. **One new plugin type, `Analyzer`.**
   - It runs once per load, read-only, on the final ledger, after booking and the core's validation, before error
     placement and ids.
   - It reads the ledger through pushed records and pulled BQL, not the zhang-ast stream, and returns diagnostics and
     derived data. The host validates them and publishes them atomically as the plugin's snapshot, inside the
     `Ledger` value.
   - Every write reloads the ledger, so analysis also runs on every write. It therefore requires the reload to load
     outside the write lock first (lane I1).
3. **Plugins provide typed query tables of three kinds:** record tables, *series* tables and *label columns* on
   `#postings`. They are declared in a static manifest and served by the host. No plugin code runs while a query or
   an API request runs.
4. **`#budgets` becomes a series table.** The plugin publishes sparse monthly facts. The host fills the missing months
   per query by copying values (`carry`, `zero`, `previous`), within the query's date bound and limits, as
   `Rows::Generated` is bounded today.
5. **`postings.budgets` becomes a label column.** It is a BQL predicate template, with one parameter set per budget.
   No per-posting data crosses the wasm boundary.
6. **Exact decimals are strings on the wire.** Sums and conversions happen in the host first (BQL `sum`, `convert`,
   and a new `zhang_convert`). In Lua, `zhang.decimal` gives exact add, subtract and compare in pure Lua on decimal
   strings, with parity tests. There is no libmpdec: its default context rounds every operation, while Zhang's sums
   and products are exact.
7. **The official plugin is `zhang:budget`, a bundled Lua script, enabled by default.**
   - It needs neither `features.plugin` nor a `plugin` directive.
   - `ZHANG_PLUGINS=off` stops it, with an explicit `ExtensionUnavailable` diagnostic and an upgrade note.
   - It requires the C JSON codec that replaces dkjson (lane I2).
8. **The API contract is the five `budgets.*` named queries, not `/api/budgets*`.** #754 retired those routes. The
   plugin declares the named queries, and neither client changes.
9. **The known issues P0 pinned are fixed on `main` before the migration.** Each fix is its own PR, after the owner
   confirms it is a bug, so the migration itself is a pure move.
10. **The plan is twelve pull requests,** after four prerequisites: I1, I2, P0's fixtures and baseline, and the
    known-issue fixes.
    - The generic mechanisms are proven first by re-hosting today's native budget code on them.
    - The gates are G1 (Lua prototype) and P4, each measured on load p95 and `POST /api/transactions` p95, with
      parity at every step.

## Terms

| Term | Meaning on this page |
| --- | --- |
| Analyzer | The new plugin type: a read-only pass over the final ledger at load time that returns diagnostics and derived data. |
| Snapshot | What one plugin's analysis produced for one loaded ledger: tables, series facts, label parameters, diagnostics, and a key. |
| Catalog | The tables, columns and functions a query compiles against: the core's static tables plus the tables of the ledger's snapshots. |
| Record table | A plugin table whose rows the snapshot holds in full (`#budget_definitions`, `#budget_events`). |
| Series table | A plugin table of periods per partition whose missing periods the host fills at query time (`#budgets`). |
| Label column | A plugin column on `#postings` holding the set of labels whose predicate the row satisfies (`budgets`). |
| Named query | A built-in query (`POST /api/query/builtins/{name}`) declared by a plugin instead of `zhang-server/src/builtin.rs`. |
| Default plugin | A bundled plugin the binary enables without a `plugin` directive (`zhang:budget`). |
| Compat mapping | Core code that keeps legacy syntax, names, wording and error kinds working, with no business rule behind them. |

## 1. Current-state map

### 1.1 Where budget semantics live

| Piece | Where (`path:line`) | What it does | Generic capability it needs after the move |
| --- | --- | --- | --- |
| Directive kinds | `zhang-ast/src/models.rs:36-39`; payloads `zhang-ast/src/data.rs:447,456,464,474` | `Budget`, `BudgetAdd`, `BudgetTransfer`, `BudgetClose`: date (optional time), names, commodity, amount, meta; serde-tagged as `{"Budget": …}` | Kept as syntax only (§5); read by plugins as *records* |
| Close rule | `zhang-ast/src/data.rs:81` `Date::close_precedes` | A close with only a date lasts through its day; one with a time ends at that time; shared with account close | Stays generic in `zhang-ast` |
| Dead helper | `zhang-ast/src/data.rs:87` `as_budget_interval` | Nothing calls it | Delete in P5 |
| Zhang parser | `zhang-core/src/data_type/text/parser.rs:986-989`; bodies `:904,:920,:936,:955` | `budget`, `budget-add`, `budget-transfer`, `budget-close` keywords | Kept (compat mapping) |
| Zhang exporter | `zhang-core/src/data_type/text/exporter.rs:441-467,503-506` | Writes the four back, with meta | Kept |
| Beancount parser | `extensions/beancount/src/parser.rs:123-137`; bodies `:170,:187,:204,:224` | `custom "budget*"` (quoted or legacy unquoted) becomes the four kinds; other shapes stay `Custom` | Kept |
| Beancount exporter | `extensions/beancount/src/lib.rs:110-148`, time rule `:163-168` | The four kinds become quoted `custom` directives (#500) | Kept |
| Budget state of the fold | `zhang-core/src/ledger.rs:128-140` | `defined_budgets`, `foreign_budget_amounts`, reported pairs | Deleted (P5) |
| Fold dispatch | `zhang-core/src/ledger.rs:674,715,732-746,758-759` | Defines and closes budgets, checks adds and transfers, checks postings per transaction, and reports unconverted amounts after the fold, once every price is known | Analyzer at load (§2) |
| Definition and directive checks | `zhang-core/src/process/budget.rs:13,30,37,46` | `DefineDuplicatedBudget`; `BudgetDoesNotExist` for an add, transfer or close of an undefined budget (a transfer checks `from`, then `to`; `&&` short-circuits); the first close wins | Plugin rule over records |
| Foreign amounts | `zhang-core/src/process/budget.rs:98-143` | Keeps non-zero amounts in another commodity (a transfer's `from`, then `to`), then reports `BudgetCommodityMismatch` after the fold where `PriceMap::conversion` (with the cost currency as `via`) finds no rate | Host conversion (`convert`, `zhang_convert`) |
| Posting check | `zhang-core/src/process/transaction.rs:12-52` | Per *written* posting (legs merged), per budget in name order: `BudgetDoesNotExist` or `BudgetClosed` once per (account, budget), else keeps foreign amounts | Plugin rule over BQL aggregates |
| `process/open.rs` | `zhang-core/src/process/open.rs:8-17` | Only the commodity check; it holds no budget state | (correction to the RFC's map) |
| Membership rule | `zhang-core/src/pipeline/active_accounts.rs:18-19,96-97,110-113,131-151` | The `budget` metas of an account's latest `open` at a wall-clock time; all accounts per budget | Generic "open metadata at a time" (§3.6) |
| Ledger accessors | `zhang-core/src/ledger.rs:304-315` | `account_budgets`, `budget_accounts` | Generic accessor, plus an alias |
| Tables | `zhang-query/src/table/budgets.rs:113,121,129`; registry `zhang-query/src/table/mod.rs:177-179` | `#budgets` (generated months), `#budget_definitions`, `#budget_events` | Plugin record and series tables (§3) |
| Counting rule | `zhang-query/src/table/budgets.rs:174-189` `Budget::counts` | Membership at the posting's time, folded after the definition, not closed, convertible | Plugin label predicate (§3.6) |
| Activity and carry-over | `zhang-query/src/table/budgets.rs:343-400,440-451,516-598` | Converts and sums per month, negated on credit-normal accounts; carry-over; series end; recomputed on every query | Plugin facts + host series fill |
| Series bound and error | `zhang-query/src/table/budgets.rs:158-162,319-327,537-564,603-657`; `zhang-query/src/optimizer.rs:224-252` | The end is the latest of the budget's own last month, the last transaction month and the current month, capped by the query's date bound, with fixed tie rules (§3.7); `TooLarge` names the budget and the directive | Generic series kind; budget wording in the compat allowlist (§3.7) |
| Posting column | `zhang-query/src/table/postings.rs:634-642`, `budgets.rs:419-436` | The `budgets` set column, the rule the budget page's posting list uses | Label column |
| Function | `zhang-query/src/functions/scalars.rs:78-79`, `scalars/ledger.rs:48-57` | `account_budgets(account, date)` | Alias of a generic function |
| Record plumbing | `zhang-query/src/table/mod.rs:293-298,311-313,395,452,473` | `Record::Budget*`, their metadata, the per-query fold cache | Generic `Record::Extension` |
| Small mentions | `zhang-query/src/period.rs:387-397` (budget directives don't move a period's last date); `table/entries.rs:154-157` (`type` names); `table/errors.rs:95-100` (messages) | Classification and text | Compat mapping |
| Named queries | `zhang-server/src/builtin.rs:105-114,343-381`; compiled once in a static `LazyLock` at `:443` | `budgets.month`, `budgets.budget`, `budgets.budget_month`, `budgets.events`, `budgets.postings` | Plugin-declared named queries (§8) |
| HTTP | `zhang-server/src/lib.rs:148-151`, `routes/query.rs` | `GET/POST /api/query/builtins`, `POST …/{name}`, `POST …/{name}/text` | Unchanged |
| Error kinds | `zhang-ast/src/error.rs:41-48` | `BudgetDoesNotExist`, `DefineDuplicatedBudget`, `BudgetCommodityMismatch`, `BudgetClosed`; metas `budget_name`, `account_name`, `commodity`, `budget_commodity` | Kept as legacy kinds (§2.6) |
| Web UI | `frontend/src/components/budget/budget-api.ts:13-41`, `budget-rows.ts`, `pages/Budgets.tsx`, `pages/SingleBudget.tsx`; types generated into `frontend/src/api/builtins.ts:393-460` | Runs the five named queries, batched | Unchanged |
| Flutter app | `zhang-app` `BudgetRepository`, `docs/zhang-api.md:199-204` | The same five named queries | Unchanged |
| Docs | `guides/budgets.md`, `reference/directives/budget.md` (+ zh-cn), `reference/builtin-queries.md`, `reference/query-language.md` | User contract | Updated in P5 |

### 1.2 Corrections to the map in #698, and where P0 stands

- **`zhang-server/src/routes/budget.rs` no longer exists.**
  - #754 (`67d04456`) retired `GET /api/budgets`, `/api/budgets/{name}` and `/api/budgets/{name}/interval/{y}/{m}`.
    Both clients now read budgets only through the five named queries, and
    `zhang-server/tests/api/error_bodies.rs:65` pins the 404.
  - The stable surface is therefore the named queries (§8), not an `/api/budgets*` route.
- **The "budget-specific state in `process/open.rs`"** moved to the account lifecycle (`pipeline/active_accounts.rs`).
- **The plugin platform moved on** since the RFC's snapshot `74a9f03b`:
  - #798: parameterised BQL and the schema for routers;
  - #801: compile reuse;
  - #802: Lua routers;
  - #803: gated writes;
  - #804/#805: `zhang:mcp`, bundled, and `zhang_query_builtin`;
  - #826: content generation;
  - #827/#830: limits and the operator ceiling;
  - #833: error ids and `txn_id`.
- **P0 is [#835](https://github.com/zhang-accounting/zhang/pull/835)**, open, `test/budget-contract-p0`.
  - **What it pins.** 19 cases and 21 golden files, answered through the router the server runs:
    - every budget request and the three tables over BQL;
    - `postings.budgets`;
    - the ledger's errors *id-exact in report order* (`error_list`: `Ledger::errors` with id, place, kind, `n`, span
      and every meta);
    - exports and round trips in both formats.
  - **Still to come.** A performance-baseline PR follows from lane P0. It holds load, query and write p95, memory and
    the bench harness.
  - **Its notes for P1.** They are folded in below:
    - mismatches are reported after the fold;
    - a transfer reports `from` before `to`;
    - ledger order, not dates, decides membership;
    - exact scales;
    - conversion per date.

### 1.3 What pushing the stream would cost (indicative)

A throwaway measurement taken while writing this page. It ran on the builder (Linux x86_64, release build) over
generated ledgers with 10 budgets and 20% foreign-currency postings. The harness is not committed (Appendix B), so the
figures are **indicative only**. They motivate decision 9, nothing else. G1 and P4 measure with the harness that P0's
performance-baseline PR commits.

| Ledger | Load, no plugin | Load with a Rust→wasm processor returning the stream unchanged | Same, summing every posting per account and month with `BigDecimal` and `PriceMap::convert` | `#budgets` query today (warm cache) |
| ---: | ---: | ---: | ---: | ---: |
| 10,000 transactions (8.3 MB of stream JSON) | 126 ms | 417 ms | 410 ms | 1.0–1.5 ms |
| 50,000 transactions (41.7 MB) | 662 ms | 2,105 ms | 2,110 ms | 6.7–12 ms |

- **The round trip dominates.** Shipping the stream across the boundary and back added about 290 ms per 10,000
  transactions even in Rust. That includes the second booking pass any booked-slot plugin triggers. The computation
  inside the plugin is noise next to it.
- **Lua is worse.** #698's spike measured the same round trip at 9.2 s per 10,000 transactions in Lua with dkjson, on
  an Apple M4.
- **Conclusion:** an analyzer pulls aggregates and must not receive the stream, whatever its language.

### 1.4 Known issues pinned by P0: fixed before the migration

#835 pins five behaviours as `KNOWN-ISSUE`s, as they are. **Decided:** each one is fixed on `main` *before* the
migration, in its own PR, once the owner confirms it is a bug. That PR updates #835's golden files. The migration (P1
to P5) then reproduces `main` as it is at that point, a pure move. Where a fix changes a rule, the plugin implements the
fixed rule.

| # | Known issue (#835) | Where | Effect on the plugin |
| --- | --- | --- | --- |
| K1 | A timed budget directive does not round-trip through beancount (exported with its time on the date line) | `extensions/beancount/src/lib.rs:163-169` | None: syntax stays in the core |
| K2 | Unquoted `budget:` link metadata is exported unquoted to beancount | `zhang-core/src/data_type/text/exporter.rs:98` | None |
| K3 | `DefineDuplicatedBudget` has no metas | `zhang-core/src/process/budget.rs:15` | The plugin emits the metas the fix defines |
| K4 | Income and Equity postings count negated (a refund adds to activity) | `zhang-query/src/table/budgets.rs:36-37,351` | The plugin's sign rule follows the fix (today it is `possign`'s) |
| K5 | A `month` parameter that is not a first day is accepted | `zhang-server/src/builtin.rs:105-114,364-381` | The named queries move with the fix |

## 2. The Analyzer extension point

### 2.1 Where it runs

```text
server reload (I1: outside the write lock) ─► parse ─► options, plugin registration (defaults first)
      ─► raw processors/mappers ─► BookingStage ─► processors/mappers ─► (re-book) ─► ActiveAccounts ─► Pad
      ─► BalanceCheck ─► Validate ─► fold (ids, outcomes; records where each directive's errors start)
      ─► ANALYZERS, in order (default plugins first)             ◄── new, read-only, final ledger
      ─► merge diagnostics at their places ─► reset `derived`    ◄── §2.4, §2.6
      ─► place_errors (txn_id) ─► identify_errors (ids)
      ─► Ledger value: directives, outcomes, errors, snapshots
      ─► server: fingerprint, generation (#826), swap under the write lock
```

An analyzer cannot change a directive, a balance, an outcome or another plugin's output.

**Every write runs this whole path.** A write reloads the ledger before it answers
(`zhang-server/src/routes/transaction.rs:304`), so the analysis cost is part of every `POST /api/transactions` and
not only of a load.

**Lane I1 is a prerequisite (decided).** It moves the reload's load outside the write lock and swaps the result in
under it. Without it, analysis time would stall every reader during each reload.

### 2.2 The contract

**Declaration.**

- A module declares the type `Analyzer` in `supported_type`. Older hosts ignore an unknown type with a warning
  (`zhang-core/src/plugin/store.rs:149-163`).
- It also declares a static **manifest**, an optional export read once at registration, as `capabilities` is today
  (`store.rs:186-195`). In Lua the manifest is fields of the module table.
- The manifest is static, so the catalog and the named queries are known before any analysis runs, even when one
  fails.

```json
{
  "analysis_abi": 1,
  "namespace": "budget",
  "consumes": ["budget", "budget-add", "budget-transfer", "budget-close"],
  "open_meta": ["budget"],
  "diagnostics": [
    {"kind": "BudgetDoesNotExist"}, {"kind": "DefineDuplicatedBudget"},
    {"kind": "BudgetCommodityMismatch", "phase": "after_fold"}, {"kind": "BudgetClosed"}
  ],
  "tables": [ "… §3.2 …" ],
  "columns": [ "… §3.2 …" ],
  "queries": [ "… §8 …" ]
}
```

**Input of `analyze`.** The host passes the *records* of the consumed kinds, in ledger order, in a versioned shape of
`zhang-shared::plugin_abi::analysis`, never zhang-ast's serde shape:

```json
{"records": [
  {"kind": "budget-add", "seq": 41, "date": "2024-03-01", "time": "00:00:00", "has_time": false,
   "timestamp": 1709251200, "fields": {"name": "Food", "amount": {"number": "500.00", "currency": "CNY"}},
   "meta": {"note": ["groceries"]}, "file": "budget.zhang", "line": 12}
],
 "ledger": {"timezone": "Asia/Shanghai", "operating_currency": "CNY"}}
```

- `seq` is the `seq` of `#entries` and `#postings`: the position in the order Zhang processed the ledger. It is
  comparable with a posting's, and as #835 shows, it decides membership on a shared day.
- The fields of each kind come from the compat mapping (§5).
- A `custom "<type>"` directive is a record of kind `custom:<type>` with positional `values`.
- When the ledger has no record of a consumed kind and no `open` with an `open_meta` key, the host does not call the
  plugin. The snapshot is `skipped` and its tables are empty.

**Host functions while `analyze` runs.** These are the router's read-only ones, bound to the ledger being loaded:

- `zhang_query`, `zhang_query_params`, `zhang_query_schema` and `zhang_ledger_info`;
- `zhang_now`, which records the date as an input, as stages do;
- one new function, `zhang_convert`.

`zhang_convert` converts a batch of `{amount, target, date, via?}` exactly as `PriceMap::conversion` and
`Position::convert` do, answering an amount or `null`. It is the core's generic currency conversion, not a budget
function.

Records and query results together are the RFC's "typed reads of amounts, dates, metadata and source order". Reads
come in batches: one parameterised query per question.

**Refused during analysis.** Until the merge, error placement is not final, so these are refused, with the query error
"not available while plugins analyse the ledger":

- `#errors`;
- the `errors` and `balanced` columns of `#entries`, `#transactions` and `#postings`;
- the `journals.errors` named query.

**What analyzers get.** No files, no network and no writes, whatever their directive grants. They are pure functions of
the ledger.

**Output of `analyze`.** One JSON document, validated as a whole before anything is published:

```json
{"diagnostics": [{"kind": "BudgetClosed", "target": {"seq": 812},
                  "metas": {"budget_name": "Fun", "account_name": "Expenses:Shared"}}],
 "tables":  {"budget_definitions": [ {"name": "Food", "date": "2024-01-01", "…": "…", "$meta": {"alias": ["Groceries"]}} ],
             "budget_events": [ "…" ]},
 "series":  {"budgets": {"partitions": [{"key": {"name": "Food"}, "until": "2024-11-01",
                                         "until_cause": {"seq": 980}, "first_cause": {"seq": 3},
                                         "rows": [ {"date": "2024-01-01", "assigned": {"number": "500.00", "currency": "CNY"}, "…": "…"} ]}]}},
 "labels":  {"budgets": [{"label": "Food", "params": {"accounts": ["Expenses:Food"], "defined_seq": 3,
                                                       "close_date": null, "close_time": null, "commodity": "CNY"}}]}}
```

### 2.3 Lifecycle (#698 open question 1)

**Decided (2026-11-20): split by what each part needs.**

- **At each load, so at each write too:** analysis runs eagerly, producing the diagnostics and every fact that grows
  with the ledger.
- **At query time:** the host alone does the work that grows with time, the months of a series (§3.7), bounded by the
  query.
- **Never** does plugin code run at request time.

| Option | Load diagnostics | Stable snapshot | Resource bound | Verdict |
| --- | --- | --- | --- | --- |
| Analysis at load | Yes: merged before placement and ids, so the journal's per-transaction errors and #833's ids work unchanged | Yes: part of the immutable `Ledger` value | Bounded by the ledger, not by time; paid on every load *and write* | **Decided**, for facts and diagnostics |
| First-query computation | No: the errors page and journal would change after a query ran, and ids would depend on request order | Needs a once-cell and race handling under the read lock | The first reader pays the whole analysis | Rejected for diagnostics |
| Plugin code per query (a `table_rows` export) | Not applicable | The plugin must be handed its state on every call | wasm per request under the read lock, re-serialised every call, recursion risk | Rejected (§3.7) |

The analysis must not depend on the wall clock. Calling `zhang_now` records `ExtraInput::Clock`, so the midnight reload
recomputes the snapshot. Budget never calls it: "the current month" is the query's `today()`, applied by the series
fill.

### 2.4 Snapshots

- **Where.** A new `Ledger::extensions` maps each plugin to an `Arc<Snapshot>`. It is written once inside the load,
  before the `Ledger` value is visible to anyone, and immutable afterwards.
- **Key.** Each snapshot records:
  - `(plugin name, module digest, script digest, config digest, analysis_abi, load id)`;
  - `status: ok | failed{reason} | skipped`.
- **Bound to the generation (#826) by construction.** A snapshot lives and dies with its `Ledger` value, and the
  server swaps that value whole under the write lock. A reader holding a `Ledger` sees only that ledger's snapshots.
  A reload always re-analyses, and no WASM instance outlives the `analyze` call.
- **The fingerprint covers inputs, not analysis output.** #826's fingerprint (`zhang-server/src/lib.rs:597-615`) does
  not hash analysis output. That is sound only because analyzers are deterministic and their inputs are hashed. Two
  gaps:
  - It hashes `module_bytes()`, which for a Lua script is the interpreter, not the script
    (`zhang-server/src/lib.rs:677-678`), so a script edit keeps the generation today.
  - P1.5 adds the script source and the default plugins' modules to the fingerprint.
- **Atomic publish.** The output is decoded, type-checked against the manifest, size-checked and converted to host
  values in full, and only then stored. Any failure stores `failed` with no data, never half a snapshot.
- **The query cache is reset after the merge.** The analyzer's queries build the query engine's per-ledger cache
  (`LedgerCache`, in `derived`, a `OnceLock`). Its `#entries` rows copy each transaction's error kinds from
  `Detail::Transaction { errors }` (`zhang-query/src/table/cache.rs:161-170`), but `place_errors` fills that list only
  after the merge (`zhang-core/src/ledger.rs:408`). So:
  - the core replaces `derived` with a fresh one after the merge;
  - error-dependent tables and columns are refused during analysis (§2.2).

  The first query after a load rebuilds the cache, as today. There is no warm cache to gain.
- **Reuse (optional, P4).** A reload of unchanged content may reuse a snapshot whose digests match. It must be an `ok`
  snapshot: **a failed snapshot is never reused**, so the next load always retries. Reuse does not help writes, whose
  content always changes.

### 2.5 Failure isolation

| Failure | Effect |
| --- | --- |
| Trap, Lua error, timeout, out of memory, invalid or oversized output, an undeclared diagnostic kind, a value of the wrong type | **The ledger loads**, unlike a failing processor, which aborts the load. The snapshot is `failed`, and there is one diagnostic, `PluginAnalysisFailed`, a new generic kind with metas `plugin`, `reason` and `message`. It goes on the plugin's directive, or for a default plugin on the first directive it consumes. The tables answer the query error `table #budgets is unavailable: plugin zhang:budget failed (see the ledger errors)`. Label columns read empty, with the same error once per query. Other plugins are unaffected. |
| A query inside `analyze` fails | The plugin gets the error as a value, as routers do. Uncaught, the row above applies. |
| An analyzer queries another's failed table | That query fails with the "unavailable" error. |

### 2.6 Diagnostics

**Kind.**

- A plugin may emit only the kinds its manifest declares.
- The four legacy budget kinds keep their `ErrorKind` variants, OpenAPI enum and message templates
  (`zhang-query/src/table/errors.rs:95-100`), in a core compat allowlist granted to `zhang:budget` alone.
- Any other plugin's kind becomes `PluginError` with metas `plugin`, `kind` and `message`, so the closed `ErrorKind`
  enum that both clients decode does not grow.

**Target.**

- `{"seq": n}`, `{"txn": "uuid"}` or `{"plugin": true}`.
- The host resolves it to the directive's `SpanInfo`; plugins never send spans.

**Placement, which fixes the report order and with it the ids.** #835 pins `Ledger::errors` id-exact in *report
order*, and ids are `of_error(place, kind, n)` with `n` counted in that order (`identify_errors`,
`zhang-core/src/domains/schemas.rs:90-99`). The host therefore puts each diagnostic where the native fold reports
it:

- **Phase `fold` (default).** During its fold, the core records where each directive's errors start in the error list.
  A diagnostic targeting a directive is inserted at that directive's place, after the fold errors of earlier
  directives and before those of later ones.
  - No directive carries both core fold errors and budget diagnostics: a transaction's fold errors are budget-only,
    and opens, commodities, prices and documents carry no budget error. So placement by target is unambiguous.
  - Within one directive, the plugin's emission order is kept.
- **Phase `after_fold`.** The diagnostic is appended after every fold error, in emission order. This is where all
  `BudgetCommodityMismatch` go, those of postings and of add and transfer amounts alike: the native code keeps them
  during the fold and reports them once every price is known (`zhang-core/src/process/budget.rs:117-143`, called at
  `zhang-core/src/ledger.rs:758`).
- **The native emission order the plugin must reproduce:**
  - directive (`seq`) order;
  - in a transaction, the written postings in order, and for each the budgets of the account's `open` in byte order
    (a `BTreeSet`), each (account, budget) pair reported once, on its first transaction;
  - in a transfer, `from` before `to` (`zhang-core/src/process/budget.rs:38-41`), **not** name order. When `to` sorts
    first, swapping them would swap their ids. #835's `errors-order` case pins this: `from` Zeta gets `#0`, `to`
    Alpha `#1`.
- **`txn_id`.** `place_errors` runs after the merge. So a diagnostic at a transaction gets `txn_id` and appears in
  that transaction's list (`Detail::Transaction { errors }`) as today.
- **Volume.** At most 10,000 diagnostics per plugin and load, then one summary diagnostic.

### 2.7 Resource limits

| Limit | Value (proposal) | Why |
| --- | --- | --- |
| Call time | The plugin's `timeout`, capped by `ZHANG_PLUGIN_MAX_TIMEOUT` (30 s) | After I1 a reload no longer stalls readers, but a write still waits for its reload: analysis time is write latency, gated in G1 and P4 |
| Memory | `ZHANG_PLUGIN_MAX_MEMORY` (256 MiB per instance); Lua's 64 MiB of values | Unchanged |
| Each query | `POST /api/query` limits: 10 s and 1,000,000 values | Unchanged |
| Queries per analysis | 10,000 | Keeps a looping script finite before the timeout |
| Output | 1,000,000 values and 64 MiB of JSON | Bounds the host's copy |
| Series rows at publish | Sparse facts only, within the output cap | Dense months are generated per query (§3.7) |
| Diagnostics | 10,000 | §2.6 |

### 2.8 Next to Processor, Mapper, Router and the operator ceiling

| | Processor / Mapper | **Analyzer** | Router |
| --- | --- | --- | --- |
| When | Load, before validation | Load, after the fold, before placement and ids | Each HTTP request |
| Sees | The zhang-ast stream (ABI v1 view) | Records it consumes; BQL over the final ledger | BQL over the served ledger |
| Changes | The stream, so the ledger | Nothing; it publishes a snapshot | Nothing, or appends through gated writes |
| Failure | Aborts the load | Snapshot `failed`, one diagnostic, the ledger loads | The request fails (500/504) |
| Lock | Load, outside the write lock after I1; a write waits for it | Same | Read lock for the call |
| Files, network | `allowed_paths`, `allowed_hosts` | None | None for files; network as granted |
| Lua | Not yet (needs a full decimal API) | Yes (§4, P1.6) | Yes |

**Operator ceiling.** `ZHANG_PLUGINS=off|read|write` (`zhang-core/src/plugin_policy.rs:39,57,93`) bounds every
plugin, the default ones included (§6). `read` and `write` mean nothing to an analyzer,
which never writes. The timeout and memory ceilings apply to every analyzer.

### 2.9 The embedding seam

`zhang-core` cannot run BQL, because `zhang-query` depends on it. So the core defines the hook and `zhang-query`
implements it:

```rust
// zhang-core
pub trait AnalysisHost: Send + Sync {
    /// run every analyzer of `ledger` (final, read-only); diagnostics with their phase, and snapshots, come back
    fn analyze(&self, ledger: &Ledger, analyzers: &[RegisteredPlugin]) -> AnalysisOutcome;
}
```

- **Installing it.** The host is installed with the plugin policy (`Ledger::load_with_policy` grows a `LoadOptions`)
  and kept by `Ledger::reloader()`.
- **Entry points.** The server, the CLI and the test kit install it in one place.
- **Embedders without it.** These are `bindings/wasm` (the playground) and plain-core tests. They get §6's diagnostic.

## 3. Plugin-provided typed query tables

### 3.1 Kinds

| Kind | Rows | Served by | Budget use |
| --- | --- | --- | --- |
| Record table | In full, in the snapshot, with optional row metadata (`$meta`) for `meta()`, `meta_values()`, `metas` | The host, as `Rows::Records` today | `#budget_definitions`, `#budget_events` |
| Series table | Sparse facts per partition plus a declared fill policy; missing periods generated per query | The host, generalising `Rows::Generated` and `optimizer::date_bound` | `#budgets` |
| Label column | A `Set` column on `#postings`: the labels whose predicate holds; one template in the manifest, parameters per label in the snapshot | The host, evaluating compiled predicates | `postings.budgets` |
| Named query | BQL with typed parameters, in the manifest | The built-in query registry | `budgets.*` (§8) |

In v1 a plugin adds no column to any other core table and no scalar function (§3.6 handles `account_budgets()`).

### 3.2 Registration and schema

```json
{"tables": [
  {"name": "budgets", "kind": "series",
   "description": "One row per budget per month, …",
   "period": {"column": "date", "unit": "month", "derive": {"year": "year", "month": "month"}},
   "partition": ["name"],
   "extend_to_today": true,
   "columns": [
     {"name": "name", "type": "str", "fill": "carry", "description": "…"},
     {"name": "alias", "type": "str", "fill": "carry"}, {"name": "category", "type": "str", "fill": "carry"},
     {"name": "currency", "type": "str", "fill": "carry"},
     {"name": "date", "type": "date"}, {"name": "year", "type": "int"}, {"name": "month", "type": "int"},
     {"name": "assigned", "type": "amount", "fill": {"previous": "available"}},
     {"name": "added", "type": "amount", "fill": "zero"},
     {"name": "activity", "type": "amount", "fill": "zero"},
     {"name": "available", "type": "amount", "fill": "carry"},
     {"name": "accounts", "type": "set", "fill": "carry"},
     {"name": "closed", "type": "bool", "fill": "carry"}],
   "wildcard": ["name", "date", "assigned", "activity", "available"],
   "row_meta": "carry"},
  {"name": "budget_definitions", "kind": "records", "columns": ["…"], "order": ["name"]},
  {"name": "budget_events", "kind": "records", "columns": ["…"], "order": "published"}],
 "columns": [
  {"table": "postings", "name": "budgets", "type": "set", "kind": "labels",
   "description": "The budgets the posting counts toward, …",
   "predicate": "account IN :accounts AND seq > :defined_seq AND :label IN open_meta_values(account, 'budget', date, time) AND (:close_date IS NULL OR (:close_time IS NULL AND date <= :close_date) OR (:close_time IS NOT NULL AND (date < :close_date OR (date = :close_date AND time <= :close_time)))) AND currency(convert(position, :commodity, date)) = :commodity",
   "params": {"accounts": "set", "defined_seq": "int", "close_date": "date", "close_time": "str", "commodity": "str"}}]}
```

- **Types.** Column types are BQL `DataType`s (`str`, `int`, `decimal`, `date`, `bool`, `amount`, `set`, `metas`).
  Values use the query API's cell encoding; an amount is `{number: "<decimal string>", currency}`. A JSON number in a
  `decimal` or `amount` cell is refused.
- **Fill policies.** There are exactly three:
  - `carry`: copy the previous row's value;
  - `zero`: zero in the previous value's commodity;
  - `{"previous": "<column>"}`: the previous row's value of another column of the same type, which is filled with
    `carry` (P1.3), so that every generated period copies the last fact's value of it.

  Period columns are generated. Fills only copy, so the host does no arithmetic.
- **Label predicates.** `:label` is bound to each label's name. The predicate compiles once per catalog with the
  declared parameter types.

### 3.3 Type checking

| When | Check |
| --- | --- |
| Registration (manifest) | Names, types and fill policies (a `previous` column has the same type, filled with `carry`); wildcard ⊆ columns; at most 64 columns per table and 16 label columns per ledger (§3.4). The predicate compiles against `#postings` to `Bool`: row-level, no aggregate, no plugin table. Named queries compile against the catalog with their declared parameters. Any failure fails registration, like a bad `supported_type`. |
| Publish (snapshot) | Every cell has its column's type or is NULL. Partition keys are present and unique. Series rows are strictly increasing and aligned to the unit. Label parameters match their types. Record order is as declared. Any failure: `failed` (§2.5). |
| Query compile | As for core tables: columns resolve against the query's table only, functions type-check, `SELECT *` expands to the wildcard. |

### 3.4 Names and collisions

- **Core names win.** A plugin can never shadow a core table, a `#postings` column or a function.
- **Namespaces.** A third-party plugin's tables, columns and named queries start with its manifest `namespace`
  (`[a-z][a-z0-9_]*`; the plugin name with `-` written `_` by default):
  - tables and columns as `#<namespace>_<name>`;
  - named queries as `<namespace>.<name>`.
- **Conflicts.** Two plugins with one name: the first declared serves it, and the later one is reported on its
  directive. This is the routers' rule (`zhang-core/src/plugin/router.rs:347`).
- **Legacy names.** `#budgets`, `#budget_definitions`, `#budget_events`, `postings.budgets` and the `budgets.*`
  queries form a core allowlist granted to the module `zhang:budget` only. No ledger plugin can claim them, so
  forking the budget rules is out of scope for v1.
- **Declared but failed.** A table whose snapshot `failed` stays in the catalog and answers "unavailable", not
  "unknown table".
- **Limits (P1.3).** An extension table has at most **64 columns**, and a ledger's catalog at most **16 label
  columns** on `#postings`, **shared by all the plugins of the ledger**, default ones included (`zhang:budget` uses
  one). Every column is read through a getter per cell position or label slot, so that reading it costs what reading
  a core column does.
  - **Overflow** is refused at registration, like a conflict. With only native providers (P1.3) the catalog does not
    build, which the tests catch. From P1.4 the plugin that overflows is skipped and reported on its directive, and
    the others keep their tables and columns, as for a name conflict.
  - The plugin guide states both limits with the `Analyzer` type (P1.4).

### 3.5 Compiler integration, the schema API and compile caches

**What is static today.**

- The table registry (`zhang-query/src/table/mod.rs:166-191`) and `Plan.table` are `&'static Table`.
- A column in a compiled expression is `CExpr::Column(&'static ColumnDef)` (`zhang-query/src/compiler.rs:79`).
- The projection is a `u64` bitmask whose bit is the column's position, found by pointer identity
  (`zhang-query/src/projector.rs:55,202-207`). That capped a table at 64 columns until P1.1; `#postings` already has
  36.

**In two steps.**

1. **P1.1, byte-identical.**
   - `TableRef` (`Static(&'static Table)` or later `Extension(Arc<ExtensionTable>)`) and `ColumnRef` (a table plus
     the column's index) replace the static references in `Plan`, `CExpr::Column` and the projector.
   - The projection becomes a bitset indexed by `ColumnRef`, with no pointer identity and no 64-column cap of its
     own. Extension tables get a 64-column cap again in P1.3, for another reason (§3.4: a getter per cell position).
   - `Query::compile_in(&catalog, …)` resolves over a catalog that contains only the core tables, and
     `Query::compile` calls it.
   - Nothing observable changes: answers, `GET /api/query/schema`, the OpenAPI document and the frontend's
     `schemas.ts` all stay byte-identical.
2. **P1.3, with the extension tables.**
   - The catalog gains the ledger's extension tables.
   - A *catalog digest* (a hash of every extension table's schema) is recorded by each compiled query. Running it on
     a ledger with another digest is the error "the ledger's tables changed; compile again", never a wrong read.
   - `GET /api/query/schema` and `zhang_query_schema` become per served ledger. Tables and columns gain
     `provided_by` (`null` for the core). That changes the OpenAPI document and `schemas.ts`, regenerated in the same
     PR.

**Caches.**

- The static `COMPILED` map of built-in queries (`zhang-server/src/builtin.rs:443`) becomes one map per catalog
  digest, holding core and plugin named queries (P1.5). A ledger keeps one digest across reloads, because a manifest
  changes only with its module.
- Router and analysis queries keep compiling per call (`zhang-server/src/routes/plugin_router.rs:373-380`). No LRU:
  G1 measures whether the budget analyzer's one template per budget needs one.

### 3.6 Membership as a generic function, and `account_budgets()`

The label predicate needs the `budget` metas of the `open` in effect at a time. P1.2 generalises
`AccountLifecycle::budgets` to keep each `open`'s metadata, and adds `Ledger::open_meta_values_at(account, key, at)`
and the BQL function `open_meta_values(account, key, date[, time]) -> Set`. `account_budgets(account, date)` stays as
a documented alias of `open_meta_values(account, 'budget', date)`: compat mapping, with no rule behind it.

### 3.7 Date-bounded generation for `#budgets` (#698 open question 2)

**Today:**

- `#budgets` folds every budget and converts and sums the activity *on every query* (`budgets.rs:516-598`);
- it generates each budget's months up to the latest of its last own month, the last transaction month and the
  current month;
- a filter such as `date <= x` or `yearmonth(date) = x` caps that end (`optimizer::date_bound`);
- every generated row is charged to the result budget and the deadline.

**Proposed: the same bound, with the work split in two.**

1. **At load, the plugin publishes per budget only facts.**
   - **Rows:** a row for every month that has something: the definition, an add, a transfer, activity, the close.
   - **Columns:** `assigned` and `available` by running sums across the gaps; `added`, `activity` and `closed`.
   - **`until`:** the latest of the budget's own last month and the ledger's last transaction month.
   - **`until_cause` and `first_cause`:** generic `{seq}` references to the directive that sets that end and to the
     definition.

   Facts are O(budget directives + budget-months with activity), however far apart the dates are.
2. **At query time, the host builds each dense series.**
   - **Range:** from the first fact to `max(until, current month)` when `extend_to_today` is set, capped by the
     query's `date_bound` (unchanged code, generalised to any series table).
   - **A month without a fact** is the previous row under the fill policy: `assigned` takes the previous `available`,
     `added` and `activity` are zero, everything else is carried. That is exactly today's carry-over over an empty
     month.
   - **Cost:** every generated row costs `Limits::row()`.

**The tie rules the plugin and the host must keep** (each one pinned by #835's `months*` cases):

- **A budget's own end.** Among the budget's records of the latest date, the later-folded one names the end: `saw`
  compares with `<=` (`budgets.rs:158-162`).
- **The ledger's last transaction.** It is the latest date, and among equal dates the last folded: `max_by_key`
  returns the last maximum (`budgets.rs:319-327`).
- **The precedence of the end.**
  - The ledger's last transaction replaces the budget's own end only when its month is strictly later.
  - The current month replaces it only when strictly later.
  - The query's bound applies only when strictly earlier (`budgets.rs:537-564`).

  The plugin folds the first two into `until`, `until_cause` and `first_cause`; the host applies the last two.
- **The `TooLarge` error** names the series with the most months, and the first by name among ties
  (`budgets.rs:608`, `rev().max_by_key`). A month with an add, a transfer, activity or the close is a fact; any other
  month is generated.

**Error wording.**

- The generic `TooLarge` message names the table, the partition, its first and last period, and the cause:
  - the directive at `until_cause`, described by its kind;
  - the definition at `first_cause`, when the current month sets the end;
  - the query's bound.
- For `#budgets` the exact current wording ("budget '…' runs from … until …, because of a budget-add dated …") is kept
  verbatim in the compat allowlist (`budgets.rs:603-657`). The contract of
  `zhang-query/tests/engine/zhang_tables.rs:859,901` and #835's `months-and-a-date-typo` hold.

**How predicates and limits reach the plugin:** they do not need to.

- The plugin's work is bounded by the ledger at load. The query's date bound, its result budget and its deadline
  apply where rows are generated, in the host, exactly as now.
- A typo dated 9999 costs one fact row. A page for one month still generates only up to that month.

**Alternatives.**

| Alternative | Why not |
| --- | --- |
| Dense months materialised at load | A typo date materialises thousands of months at every load and every write |
| A plugin export per query with `{today, until, projection, budget, deadline}` | wasm per request under the read lock, the snapshot re-serialised per call, a recursion hazard, page latency tied to the language |
| Window functions and `generate_series` in BQL | Generic, but a large engine project. A possible later replacement for fill policies |

### 3.8 Recursion guard

- **During analysis.** While plugin *P* analyses, its queries compile against the core tables plus the tables of
  analyzers that ran *before* it (default plugins first, then declaration order). A query naming *P*'s own table, or a
  later plugin's, fails with `table #x is not available while zhang:budget analyses the ledger`. This is acyclic by
  construction.
- **At query time.** No plugin code runs, so no table reads itself. A label predicate may not name a plugin table.

## 4. Exact decimals across the boundary (#698 open question 5)

**Zhang's semantics** (`zhang-shared/src/decimal.rs`):

- sums and differences are exact, and so is `mul` (`:7-8,72`);
- `div` rounds to 28 significant digits but keeps the ideal scale of an exact quotient (`:42-62`);
- only rate products use a 28-digit context (`mul_in_context`).

The wire carries decimals as strings in plain notation. #835 pins the results, scale included: an inverse conversion
gives `190.8450704225352112676056338 USD`, and a product keeps its scale (`10355.00 CNY`).

| Option | Exactness | Cost | Verdict |
| --- | --- | --- | --- |
| A. A batched host operation (`zhang.decimal.sum(list)`, `…diff`, `…cmp`) | Exact; the host's own code | One boundary crossing per batch | Alternative to B; same API surface |
| B. **Pure Lua on decimal strings: add, subtract, negate, compare, zero test** | Exact if correct: sign, digits and scale, with the result's scale the larger of the two as `BigDecimal` keeps it, and no `-0` | Microseconds per operation, no crossing | **Decided** for Lua, with parity tests |
| C. libmpdec in the interpreter | Its default context rounds every operation, sums included, at 28 digits; Zhang's sums and `mul` are exact and its `div` has its own scale rule | Native | **Rejected** by the owner |
| D. Rust SDK: re-export `zhang_shared::decimal` | The host's code | Native in wasm | For Rust plugins and the host's fixtures |
| E. Aggregation and conversion in the host (`sum`, `convert`, `possign`, `zhang_convert`) | Exact by construction, scale rules included | One query per aggregate | **First resort**, whatever the language |

**v1.**

- Plugins aggregate and convert through the host (E). Lua plugins do the rest with `zhang.decimal` (B), which has no
  multiplication or division: those stay in the host.
- That covers Budget, which needs only running sums, comparisons and zero tests in the plugin.
- Lua processors and mappers stay out of scope: they would need a full decimal API with Zhang's `div` and
  `mul_in_context`.
- **Parity tests.** A property test drives the interpreter with random decimal strings and compares every result,
  scale and text included, with `BigDecimal` and `zhang_shared::decimal::plain`. The strings cover:
  - 0 to 40 fractional digits, up to 80 digits, both signs;
  - zero in several scales;
  - results that cancel to zero.

```lua
local d = zhang.decimal                           -- api_version 2: pure Lua over decimal strings
local available = d("0")
for _, fact in ipairs(facts) do                   -- strings from the host
  local assigned = available + d(fact.added)      -- exact; scale = the larger of the two
  available = assigned - d(fact.activity)
  row.assigned = { number = tostring(assigned), currency = commodity }   -- plain notation: "1234.50"
end
d("0.10") + d("0.2")     --> 0.30
d("1") == d("1.00")      --> true (by value, as BigDecimal compares)
d("-1.00") + d("1")      --> 0.00 (never -0.00)
d("2") * d("3")          --> error: zhang.decimal has no multiplication; compute in the host (BQL, zhang.convert)
d(0.1)                   --> error: decimal must be a string or an integer, got 0.1
```

## 5. AST and syntax (#698 open question 3)

| Option | Export and spans | Effect on ABI v1 plugins | Verdict |
| --- | --- | --- | --- |
| A. **Keep the four `Directive::Budget*` variants as syntax only.** A compat mapping defines each kind's record fields: `name`, `commodity` / `name`, `amount` / `from`, `to`, `amount` / `name`. | Unchanged | None: processors keep seeing `{"Budget": …}` | **Decided** |
| B. A generic `Directive::Record` replacing the four | Needs per-kind writers in both exporters | Breaking: `AbiV1View` would have to map records back forever, as it hides `pad` (`pipeline/plugin_view.rs:1-33`) | Later, only for a second plugin's syntax |
| C. Map the keywords to `Custom` | `budget-add` would export as `custom "budget-add"`; `Custom` has no amount type | Changes what processors see | Rejected |
| D. Plugin-owned parsing | n/a | n/a | Rejected: plugins load after parsing, and both dialects would need a parser ABI |

**Under A, the variants carry no behaviour after P5:**

- the fold ignores them;
- `#entries` lists them with their `type` names;
- `period.rs` excludes them as extension records;
- the exporters write them as written.

Beancount's `custom "budget*"` forms map onto the same variants, so both dialects reach the plugin as identical
records. K1 and K2 (§1.4) are fixed in the core before the migration. Third-party plugins get typed records of their
own `custom "<type>"` directives through `consumes`.

## 6. Official default plugins (#698 open question 4)

**Mechanism.**

- `zhang:budget` is a bundled plugin (`zhang-core/src/plugin/bundled.rs`, file `plugins/budget/budget.lua`,
  `include_bytes!`) in the reserved `zhang:` namespace of #805, marked *default*. Bundling grants nothing, and an
  analyzer needs no grant.
- A default plugin is registered for every ledger, with no `plugin` directive and no `features.plugin`. It registers
  and analyses first.

**Decided (2026-11-20):** there is no `ZHANG_DEFAULT_PLUGINS`. The operator ceiling of #830 bounds
default plugins too, and `getting-started/installation.md:101` documents `off` as "loads none".

| Situation | Behaviour |
| --- | --- |
| Nothing declared | `zhang:budget` runs; budgets work as today |
| `option "features.plugin"` false or absent | `zhang:budget` still runs: the ledger's own switch never disables budgets |
| `plugin "zhang:budget"` declared, with metas | It configures the default plugin (`timeout`). It is not a second instance and needs no `features.plugin`. |
| `ZHANG_PLUGINS=read` or `write` | `zhang:budget` runs (it never writes) |
| `ZHANG_PLUGINS=off` | No plugin runs, the default one included. `ExtensionUnavailable` is reported, and the budget tables and named queries are absent. The startup log line says budgets are off. The upgrading guide gets a note: "budgets need `ZHANG_PLUGINS` `read` (the default) or `write`". |
| A build without `plugin_runtime`, or an embedder without an `AnalysisHost` (`bindings/wasm`, a plain `zhang-core`) | `ExtensionUnavailable` |
| The analysis fails | `PluginAnalysisFailed`; the tables are "unavailable" (§2.5) |
| A ledger plugin declares a legacy budget name | Refused as a conflict; `zhang:budget` keeps it (§3.4) |

**`ExtensionUnavailable`:**

- **When.** It is reported only when the ledger uses budgets: a budget record exists, or an `open` has `budget`
  metadata.
- **Where.** Once, on the first such directive.
- **Metas.** `plugin: zhang:budget` and a `reason`:
  - `plugins are off (ZHANG_PLUGINS=off)`;
  - `this build has no plugin runtime`.
- **Message.** `Budget directives are not interpreted: {{meta.reason}}`.

So no user switch disables budgets, and the operator's switch that does is never silent.

**Listing.** `GET /api/plugins` gains `bundled`, `default` and `status` (`ok`, `failed`, `skipped`, `off`). This is
the optional item #805 deferred.

## 7. The language of the official Budget plugin

**Decided:** `zhang:budget` is a bundled Lua script. The comparison below records why, and what it costs.

| | Lua (bundled script) | Rust → wasm via the SDK | Native trait compiled in |
| --- | --- | --- | --- |
| Request-time cost | **None**: named queries over host tables. A Lua router call would cost 8.9 ms p50 and 66 ms p95 in #698's data; budget pages never make one. | None | None |
| Load and write cost (estimate: 10,000 transactions; 30 budgets × 10 years with monthly adds, so 3,600 records) | About 120 queries (4 per budget, 0.3–1 ms each, account-scoped), plus 0.9 MB of records in and 1.7 MB of facts and events out. With dkjson (2–3 MB/s), **about 1 s**. With I2's C codec (an estimated ≥ 30 MB/s), **about 0.15 s**. | About 0.1 s (indicative, §1.3) | About 0.05 s |
| Exact decimals | `zhang.decimal` in pure Lua (§4) | `zhang_shared::decimal` | Native |
| Parity risk | A rewrite of about 500 lines; arithmetic and conversion stay host-side | Today's code moves nearly verbatim | Lowest |
| Proves the Extism boundary, in the users' language | Yes | Boundary yes, language no | No |
| D1 ("no wasm authored by users") | Consistent | Keeps the SDK first-class for official code | Sidesteps it |

**Conditions of the decision.**

- **The C JSON codec is required.** Lane I2 is replacing dkjson with it now, keeping every rule of the `json` table.
  It also settles the review's "dkjson is slow" item for `zhang:mcp`.
- **Load-outside-the-lock is a prerequisite** (lane I1, §2.1).
- **Gate G1** (after P1.6): a Lua prototype of the budget analyzer runs on P0's baseline ledgers, measuring:
  - the **added load p95** and the **added `POST /api/transactions` p95**, at 10,000 and 100,000 transactions,
    each at most 5% of P0's baseline or 50 ms, whichever is larger (decision 16);
  - **parity** with #835's golden files.

  **The bar is tight.** The estimate above for a budget-heavy ledger of 10,000 transactions is about 0.15 s, over
  50 ms, so G1 is a real test. The levers before any fallback are fewer, grouped host queries and a compact encoding
  of records and facts. Snapshot reuse is no lever: a write changes the content, so it gets no reuse.

  **If G1 fails,** the fallback is the same contract (manifest, records, output, tables) implemented in Rust compiled
  to wasm through the SDK. Only the plugin changes. That switch needs the owner's go-ahead at the gate.

**D1 and the SDK.**

- D1 (`plugins/mcp/DESIGN.md:21`; PR #823) contradicts the shipped and documented Rust SDK
  (`developers/writing-plugins.md`).
- With a Lua Budget, D1 and the official plugin agree. The ABI stays language-neutral (wasm exports plus JSON).
- The SDK only needs `analyzer:` and `decimal` if it stays (decision 6). It is also how the host's analyzer fixtures
  are most easily written.

## 8. API and frontend

**The contract**, pinned by #835 and unchanged until P5:

- the five named queries:
  - their names and parameters;
  - their columns and types, and row order;
  - their batch form and their `/text` form;
- the three tables and `postings.budgets`, with their columns, descriptions and wildcards (#835's `schema.json`);
- the four error kinds and their metas;
- the generated `frontend/src/api/builtins.ts`.

K5 (the `month` parameter) is fixed before the migration if the owner confirms it.

**How the plugin serves it, without routes.**

- **The queries move.** The five queries leave `zhang-server/src/builtin.rs:343-381` for `zhang:budget`'s manifest,
  text unchanged. This was P2; it is now part of P1.5.

  ```lua
  queries = {
    { name = "budgets.month", description = "Every budget as of a month …",
      bql = BUDGETS_AS_OF_MONTH .. " ORDER BY name", params = { month = "date" } },
    -- budgets.budget, budgets.budget_month, budgets.events, budgets.postings
  }
  ```

- **The registry merges core and plugin queries** per catalog digest.
  - `GET /api/query/builtins` lists both.
  - `POST …/{name}`, the batch form, `/text` and the routers' `zhang_query_builtin` find them unchanged.
- **When the plugin is not loaded** (`ZHANG_PLUGINS=off`, no runtime), the names are absent and a call answers the
  existing 404 `NoSuchBuiltinQuery`.
- **When its analysis failed**, the names exist and answer the 400 query error "table #budgets is unavailable".
- **Generated types.** The test that writes `builtins.ts` (`zhang-server/src/builtin.rs:740-760`) reads the bundled
  manifest, so the file is generated byte-identical from the plugin.
- **No plugin routes or pages.** Fava-style pages stay out of this migration, as the RFC says.

## 9. Risks, alternatives, and what stays in the core

### 9.1 Risks

| Risk | Mitigation |
| --- | --- |
| **Write latency**: every write reloads, and analysis is part of the reload | • G1 and P4 gate on `POST /api/transactions` p95, not only load. <br>• Host-side aggregation keeps the analysis small. <br>• The C JSON codec (I2). |
| **Parity drift**, in report order and ids (placement, transfer `from`/`to`, after-fold mismatches), tie rules, booked legs against written postings, zero foreign amounts, the sign rule (K4) | • #835's golden files, ids and report order included. <br>• The dual run (`native\|plugin\|both`) in P3. <br>• **Differential fuzzing** over generated ledgers: native against plugin on the full `Ledger::errors` (id, kind, span, metas, order) and every table and named query. <br>• The `budget_reference` oracle. |
| **The query cache built during analysis** | `derived` is reset after the merge; error-dependent tables, columns and queries are refused during analysis (§2.4) |
| **The catalog refactor** reaches `CExpr::Column`, the projector's 64-column bitmask and every `&'static Table` | P1.1 is byte-identical by construction and guarded by the equivalence and oracle suites |
| **The fingerprint misses script edits** today (§2.4) | P1.5 hashes script sources and the default plugins' modules |
| **Reusing a bad snapshot** | Only `ok` snapshots are ever reused; a failed one is retried on the next load |
| **The playground** (`bindings/wasm`) stops interpreting budgets | `ExtensionUnavailable` and a playground regression test (P3.3) |
| **Operators surprised** that `ZHANG_PLUGINS=off` now stops budgets | The upgrading note, the startup log line, the `GET /api/plugins` status |
| **Prerequisites in other lanes** (I1, I2, the P0 baseline, known-issue fixes) | Each PR states its prerequisite; P1.1 to P1.3 depend on none of them |
| **ABI creep** | The analysis ABI is its own versioned module, additive only, independent of zhang-ast |

### 9.2 Alternatives considered

| Alternative | Why not |
| --- | --- |
| Move only the API and keep the rules native | Fails #698's acceptance: "核心与查询引擎不再保留第二套预算业务实现" |
| The full zhang-ast stream as analyzer input | §1.3; ties a new surface to zhang-ast's weak versioning |
| Per-posting annotations for `postings.budgets` | O(postings) across the boundary on every load and write |
| Lazy analysis on first query | §2.3 |
| Plugin code per query for generated tables | §3.7 |
| libmpdec for `zhang.decimal` | §4: rounds where Zhang is exact |
| A separate `ZHANG_DEFAULT_PLUGINS` switch | §6: one operator ceiling, as #830 defined it |
| A replacing user script (`replaces`) | Dropped: forking budget rules is out of scope for v1 |

### 9.3 What stays in the core

**Generic, owned by the core:**

- booking, lots and costs; pads and balance assertions; the account lifecycle (open, close, open metadata at a time);
- exact decimals; the price map and conversion (`convert`, `zhang_convert`);
- the query engine with its catalog, series tables, label columns and named-query registry;
- the analyzer host, snapshots and diagnostic placement; the operator policy.

**Compat mapping, with no rule behind it:**

- the four budget directive kinds, their parsing, export, spans and Beancount mapping;
- the four legacy error kinds with their messages;
- the legacy names and the `#budgets` `TooLarge` wording;
- the `account_budgets()` alias.

## 10. Phased plan

### 10.1 Prerequisites, outside this plan

| Item | Owner | Needed by |
| --- | --- | --- |
| **I1** Reload: load outside the write lock, swap under it (decided) | lane I1, in progress | P1.4 |
| **I2** C JSON codec replacing dkjson in the Lua interpreter (decided) | lane I2, in progress | P1.6 |
| **P0** Contract fixtures: [#835](https://github.com/zhang-accounting/zhang/pull/835) | lane P0, open | P1.3 onwards |
| **P0b** Performance baseline: load, query and write p95, memory, committed bench harness | lane P0, next PR | G1, P4 |
| **K1–K5** Known-issue fixes (§1.4), one PR each after the owner confirms it is a bug, each updating #835's golden files | the owner, then a lane | P3.1 |

### 10.2 The twelve pull requests

⛳ marks an owner decision point.

| PR | Scope | Exit criterion |
| --- | --- | --- |
| **P1.0** `docs(developers)` | This page (#834) | ⛳ The owner records the decisions of §11 |
| **P1.1** `refactor(query)` table and column references | `TableRef`, `ColumnRef`, `CExpr::Column(ColumnRef)`, a projection bitset without the 64-column cap, `Query::compile_in` over the core catalog only | Byte-identical: every engine, oracle, API and golden suite, `GET /api/query/schema`, the OpenAPI document and `schemas.ts`; a test with a 65-column table |
| **P1.2** `feat(core)` open metadata at a time | The lifecycle keeps open metas; `open_meta_values`; `account_budgets` as an alias | #835 unchanged; reopen and same-day tests |
| **P1.3** `feat(query)` extension tables | Record tables with row metadata. Series tables with `carry`, `zero` and `previous`, the generic `{seq}` causes, the tie rules, and the `#budgets` wording in the compat allowlist. Label columns with the `:x IN column` fast path. Catalog digest, `provided_by` (OpenAPI and `schemas.ts` regenerated). Today's native budget code re-hosted on them through an in-process provider. | #835 byte-identical; engine budget suites unchanged; tie-rule tests; no slower than P0b |
| **P1.4** `feat(plugin)` Analyzer (after I1) | ABI module (manifest, records, output). `AnalysisHost` hook. Snapshots; never reuse a failed one. Diagnostic placement by phase and target. Reset of `derived`; refusal of error-dependent tables. `zhang_convert`, limits, recursion guard, failure isolation. SDK `analyzer:` and `decimal`. **The lifecycle and isolation tests:** module and config swap, clock change at midnight, concurrent reload and query, timeout, OOM, trap, invalid output, missing plugin. | Every failure mode loads the ledger with one diagnostic and no partial snapshot; an SDK fixture analyzer's diagnostics land at the places and ids the native fold gives |
| **P1.5** `feat(plugin)` default plugins and named queries (absorbs P2) | Plugin-declared named queries per catalog digest. Default bundled plugins and `zhang:budget` (Lua, queries only) serving the five `budgets.*` queries moved from `builtin.rs`, still over native tables. `ZHANG_PLUGINS=off` stops defaults, with `ExtensionUnavailable` and the upgrade note. `GET /api/plugins` fields. Fingerprint hashes script sources and default modules. | #835 and `builtins.ts` byte-identical; the matrix `features.plugin` × `ZHANG_PLUGINS` × runtime present or absent |
| **P1.6** `feat(plugin)` Lua analyzers (after I2) | Interpreter: the `Analyzer` type, the manifest in the module table, `zhang.decimal` (pure Lua, §4), `zhang.convert`, `api_version` 2 | Decimal parity property tests; Lua and MCP tests unchanged; reproducible module |
| ⛳ **G1** gate, not a PR | A Lua budget-analyzer prototype on P0b's ledgers | Added load p95 **and** added `POST /api/transactions` p95 each ≤ max(5% of P0b, 50 ms) at 10,000 and 100,000 transactions; parity with #835. On failure, the Rust→wasm fallback only with the owner's go-ahead |
| **P3.1** `feat(budget)` definitions and events | `budget.lua` publishes `#budget_definitions`, `#budget_events` and the directive diagnostics. Test switch `native\|plugin\|both`. The differential fuzzing harness (seeded generated ledgers; full `Ledger::errors` and every table and named query) | `both` and the fuzzer report no difference |
| **P3.2** `feat(budget)` activity, series and labels | Activity by host BQL per budget, carry-over facts, `#budgets` as a series, `postings.budgets` labels, posting diagnostics, the after-fold mismatches | `both`, the fuzzer and the `budget_reference` oracle report no difference |
| ⛳ **P3.3** `feat(budget)!` switch | The plugin is the only runtime implementation; native code remains only behind the test switch. A playground regression test (`bindings/wasm`: `ExtensionUnavailable` on a budget ledger, unchanged otherwise) | #835 identical, ids and report order included; ⛳ the owner confirms |
| ⛳ **P4** `test(budget)` performance and release | Paired native and plugin runs at 10,000 and 100,000 transactions: many budgets, currencies and months, typo dates, concurrent requests. Load, named-query and write p95; memory. Container first start and upgrade canary. | ⛳ Added load and write p95 each ≤ max(5% of P0b, 50 ms) at 10,000 and 100,000; named-query p95 ≤ 1.1×; memory ≤ +10% |
| **P5** `refactor(core)!` delete native, and docs | Removes `process/budget.rs`, the `Ledger` budget fields, `check_budgets`, `table/budgets.rs`, `Record::Budget*`, `posting_budgets`, `as_budget_interval`, the in-process provider and the switch. Keeps the compat mapping. Docs: budgets guide, query reference, plugin development, upgrading (en and zh-cn). | `rg -i budget` in `zhang-core/src` and `zhang-query/src` finds only the compat mapping; every suite green; the docs build |

## 11. Decisions

| # | Decision | Status | Choice | Consequences |
| --- | --- | --- | --- | --- |
| 1 | Derived data lifecycle (#698 Q1) | **Decided (2026-11-20)** | Analysis at each load (so at each write) for facts and diagnostics; host-side bounded generation per query | Ids and `txn_id` unchanged; no plugin code per request |
| 2 | Bounded `#budgets` generation (Q2) | **Decided (2026-11-20)** | Series table: fill `carry`, `zero`, `previous`; generic `{seq}` causes; the budget wording in the compat allowlist | Typo-date behaviour and messages unchanged |
| 3 | AST and syntax (Q3) | **Decided (2026-11-20)** | Keep the four variants as syntax only, with a record view | No change to parsing, export, spans or ABI v1 |
| 4 | Default plugins and builds without a runtime (Q4) | **Decided (2026-11-20)** | Implicit default `zhang:budget` outside `features.plugin`; `ZHANG_PLUGINS=off` stops it with `ExtensionUnavailable` and an upgrade note; no `ZHANG_DEFAULT_PLUGINS`; no native fallback | One operator ceiling, as #830 defined it; the playground reports `ExtensionUnavailable` |
| 5 | Exact decimals (Q5) | **Decided (2026-11-20)** | No libmpdec. Exact add, subtract and compare in pure Lua on strings, or as a batched host operation, with parity tests; host aggregation first; the SDK re-exports `zhang_shared::decimal` | No multiplication or division in Lua; Lua processors stay out of scope |
| 6 | D1 against the Rust SDK | **Decided (2026-11-20)** | D1 binds users; the SDK stays at ABI parity, with no official plugin on it | PR #823's contradiction resolved by scoping |
| 7 | Language of `zhang:budget` | **Decided (2026-11-20)** | Lua, bundled with `include_bytes!` | Requires I2 and P1.6; G1 confirms |
| 8 | JSON codec | **Decided (2026-11-20)** | The C codec replaces dkjson (lane I2) | Every Lua plugin speeds up; the `json` rules stay |
| 9 | Analyzer data access | **Decided (2026-11-20)** | Pushed records plus pulled BQL and `zhang_convert`; never the stream | Boundary bytes scale with budgets × months |
| 10 | Diagnostic kinds | **Decided (2026-11-20)** | Legacy kinds allowlisted for `zhang:budget`; others `PluginError` with a `kind` meta; placement by phase and target | Closed error enums; report order and ids reproduced |
| 11 | Reload and the write lock | **Decided (2026-11-20)**, prerequisite | Load outside the write lock, swap under it (lane I1), before the analysis stage | Readers no longer stall during reloads; writes still pay for theirs |
| 12 | Legacy names | **Decided (2026-11-20)** | Keep every table, column and query name forever as an allowlist; deprecate only `account_budgets()` in docs | Explore, saved queries and clients keep working |
| 13 | `postings.budgets` | **Decided (2026-11-20)** | Label column from a predicate template | No per-posting data across the boundary |
| 14 | Known issues K1–K5 | **Decided (2026-11-20)** | Fixed on `main` before the migration, one PR each after the owner confirms the bug | The migration is a pure move |
| 15 | Scope cuts | **Decided (2026-11-20)** (lead) | No `replaces`, no router LRU, no release asset for the script, P2 folded into P1.5, P4.2's tests folded into P1.4 | Twelve PRs |
| 16 | Pass bars for G1 and P4 | **Decided (2026-11-20)** | Added load p95 **and** added `POST /api/transactions` p95 each ≤ max(5% of P0b, 50 ms) at 10,000 and 100,000 transactions; named-query p95 ≤ 1.1×; memory ≤ +10% | Stricter than proposed: they decide G1's outcome (a failure needs the owner's go-ahead for the Rust→wasm fallback) and the P5 go |

## Appendix A: the budget analysis, step by step

What `budget.lua`'s `analyze` does. Every step marked *host* is exact in Rust.

1. **Records, in `seq` order.**
   - The first `budget` of a name defines it; another gives `DefineDuplicatedBudget`, with the metas K3's fix
     defines.
   - An add or a close of an undefined budget gives `BudgetDoesNotExist`. A transfer checks `from`, then, only if it
     exists, `to`.
   - The first close wins. The `#budget_events` and `#budget_definitions` rows are built here.
2. **Accounts.** `SELECT seq, accounts, meta_values('budget') FROM #entries WHERE type = 'open'` gives every account
   whose `open` ever names a budget.
3. **Label parameters, per budget:** `{accounts, defined_seq, close_date, close_time, commodity}`.
4. **Activity, per budget**, one *host* query:

   ```text
   SELECT yearmonth(date) AS m, sum(possign(number(convert(position, :commodity, date)), account))
   WHERE <the label predicate> GROUP BY m
   ```

   The sign rule is today's `possign`, unless K4's fix changes it.
5. **Posting diagnostics, per budget**, over written postings (`GROUP BY seq, posting_index`):
   - the first transaction per account before the definition, or at all when the budget is never defined, gives
     `BudgetDoesNotExist` (phase `fold`);
   - the first transaction per account after the close gives `BudgetClosed` (phase `fold`);
   - a foreign amount that `zhang_convert` (with `via` the cost currency) does not convert gives
     `BudgetCommodityMismatch` (phase `after_fold`).
6. **Added amounts.** Add and transfer amounts are converted at their dates through `zhang_convert` (*host*) and
   summed per month with `zhang.decimal`. One that does not convert gives `BudgetCommodityMismatch` (phase
   `after_fold`, `from` before `to`).
7. **Facts.** The definition month, the months with an add, a transfer or activity, and the close month. The running
   sums give `assigned` and `available`.
   - `until` and `until_cause` come from the budget's latest record (the later-folded on a tie) and from
     `SELECT seq, date FROM #transactions ORDER BY date DESC, seq DESC LIMIT 1`.
   - `first_cause` is the definition.
8. **Output.** Diagnostics are emitted in the native order of §2.6:
   - phase `fold`: by `seq`, then written posting, then budget name in byte order;
   - phase `after_fold`: by `seq`, with a transfer's `from` before `to`.

   Then come the tables, the series and the labels.

## Appendix B: how §1.3 was measured

- **Plugins.** Two throwaway `zhang-plugin-sdk` example crates, built for `wasm32-unknown-unknown` in release:
  - a processor returning its input;
  - one that also builds a `PriceMap` and sums every posting per (account, month) in `BigDecimal`.
- **Harness.** A throwaway `zhang-server` integration test, run with `cargo test --release` in the builder pod.
- **Ledgers.** Generated: 10 budgets, one `price`, four transactions a day, every fifth in USD.
- **Measured.** `LocalFileSystemDataSource::load` plus `Ledger::process_with_policy`, with a 4 GiB memory ceiling and
  a 600 s timeout. The default 256 MiB was not tested.
- **Samples.** The median of 5 runs at 10,000 transactions and of 3 at 50,000. Queries: the median of 7 executions;
  the first builds the per-ledger cache.
- **Status.** Not committed, hence indicative. P0b's committed harness is where G1 and P4 measure.
