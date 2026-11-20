---
title: "RFC #698 P1: Budget as a Plugin"
description: "The P1 design of RFC #698: where budget semantics live today, an Analyzer extension point with per-plugin snapshots, plugin-provided query tables, exact decimals for scripts, the budget syntax as a compatibility mapping, default plugins, the language of the official Budget plugin, and a phased plan with the decisions the owner has to take."
sidebar:
  order: 5
---

:::note[Status: a proposal for the owner's decision]
Nothing on this page is implemented. It answers the questions RFC
[#698](https://github.com/zhang-accounting/zhang/issues/698) leaves to P1, against `main` at `9c156c7b`. Every
recommendation is a proposal; the [decisions](#11-decisions-for-the-owner) section lists what the owner picks. File
references are `path:line` at that commit.
:::

## Summary

1. **Keep the budget syntax in the core as a compatibility mapping, and move every rule out of it.** The core and the
   query engine keep the four `budget*` directives, their export, their spans and their Beancount `custom` form. They
   lose `process/budget.rs`, the budget state of the fold, the three budget tables, the `budgets` column and the
   membership rule.
2. **Add one new plugin type, `Analyzer`.** It runs once per load, read-only, on the final ledger, after booking and
   the core's validation and before the error ids are assigned. It reads the ledger through generic BQL, not through
   the zhang-ast stream. It returns diagnostics and derived data. The host validates that output and publishes it
   atomically as the plugin's snapshot, inside the loaded `Ledger` value.
3. **Let plugins provide typed query tables of three kinds:** record tables, *series* tables and *label columns* on
   `#postings`. The plugin declares them in a static manifest, and the host serves them. No plugin code runs while a
   query or an API request runs.
4. **`#budgets` becomes a series table.** The plugin publishes sparse monthly facts. The host fills the missing months
   at query time by copying values under a declared fill policy, bounded by the query's date filter and limits, as
   `Rows::Generated` is bounded today.
5. **`postings.budgets` becomes a label column.** It is a BQL predicate template from the manifest, with one parameter
   set per budget from the snapshot. No per-posting data crosses the wasm boundary.
6. **Exact decimals:** decimal strings on the wire, as now. In Lua, a `zhang.decimal` type backed by libmpdec, the C
   library behind Python's `decimal`, whose default context Zhang already follows. In Rust, the SDK re-exports
   `zhang_shared::decimal`. Sums and currency conversions stay in the host (BQL `sum`, `convert`, and a new
   `zhang_convert`).
7. **The official plugin is `zhang:budget`, a bundled Lua script, enabled by default.** It needs neither
   `features.plugin` nor a `plugin` directive. `ZHANG_PLUGINS=off` does not disable it; a separate operator switch
   does. When budget support is missing, the ledger gets one explicit diagnostic.
8. **Lua is recommended for the official plugin, behind a measurement gate.** It matches D1 and the RFC. Its cost falls
   only on load time, because requests never run plugin code. If the gate fails, the same contract is implemented in
   Rust compiled to wasm.
9. **The API contract to keep is not `/api/budgets*`.** #754 retired those endpoints. The contract is the five
   `budgets.*` built-in queries, their `/text` form, the Explore tables and the four error kinds. The plugin declares
   those named queries, and neither client changes.
10. **P1 lands as seven reviewable pull requests, then P2 to P5 as nine.** First the generic mechanisms are proven by
    re-hosting today's native budget code on them, before any plugin exists. A measurement gate (G1), a parity
    gate (P3.3) and a performance gate (P4.1) stand between the phases.

## Terms

| Term | Meaning on this page |
| --- | --- |
| Analyzer | The new plugin type: a read-only pass over the final ledger at load time that returns diagnostics and derived data. |
| Snapshot | What one plugin's analysis produced for one loaded ledger: tables, series facts, label parameters, diagnostics, and a key. |
| Catalog | The tables, columns and functions a query compiles against: the core's static tables plus the tables of the snapshots of the ledger. |
| Record table | A plugin table whose rows the snapshot holds in full (`#budget_definitions`, `#budget_events`). |
| Series table | A plugin table of periods per partition whose missing periods the host fills at query time (`#budgets`). |
| Label column | A plugin column on `#postings` holding the set of labels whose predicate the row satisfies (`budgets`). |
| Named query | A built-in query (`POST /api/query/builtins/{name}`) declared by a plugin instead of `zhang-server/src/builtin.rs`. |
| Default plugin | A bundled plugin the binary enables without a `plugin` directive (`zhang:budget`). |
| Compat mapping | Core code that keeps legacy syntax, names and error kinds working with no business rule behind them. |

## 1. Current-state map

### 1.1 Where budget semantics live

| Piece | Where (`path:line`) | What it does | Generic capability it needs after the move |
| --- | --- | --- | --- |
| Directive kinds | `zhang-ast/src/models.rs:36-39`; payloads `zhang-ast/src/data.rs:447,456,464,474` | `Budget`, `BudgetAdd`, `BudgetTransfer`, `BudgetClose`, with date (optional time), names, commodity, amount, meta; serde-tagged as `{"Budget": …}` | Kept as syntax only (§5); read by plugins as *records* |
| Close rule | `zhang-ast/src/data.rs:81` `Date::close_precedes` | A close with only a date lasts through its day; one with a time ends at that time. Shared with account close | Stays generic in `zhang-ast` |
| Dead helper | `zhang-ast/src/data.rs:87` `as_budget_interval` | Nothing calls it | Delete in P5 |
| Zhang parser | `zhang-core/src/data_type/text/parser.rs:986-989`; bodies `:904,:920,:936,:955` | `budget`, `budget-add`, `budget-transfer`, `budget-close` keywords | Kept (compat mapping) |
| Zhang exporter | `zhang-core/src/data_type/text/exporter.rs:441-467,503-506` | Writes the four back, with meta | Kept |
| Beancount parser | `extensions/beancount/src/parser.rs:123-137`; bodies `:170,:187,:204,:224` | `custom "budget*"` (quoted or legacy unquoted) becomes the four kinds; other shapes stay `Custom` | Kept |
| Beancount exporter | `extensions/beancount/src/lib.rs:110-148`, time rule `:163-168` | The four kinds become quoted `custom` directives (#500) | Kept |
| Budget state of the fold | `zhang-core/src/ledger.rs:128-140` | `defined_budgets`, `foreign_budget_amounts`, reported pairs | Deleted (P5) |
| Fold dispatch | `zhang-core/src/ledger.rs:674,715,732-746,758-759` | Defines and closes budgets, checks adds and transfers, checks postings per transaction, reports unconverted amounts once every price is known | Analyzer at load (§2) |
| Definition and directive checks | `zhang-core/src/process/budget.rs:13,30,37,46` | `DefineDuplicatedBudget`; `BudgetDoesNotExist` for an add, transfer (from first, then to: `&&` short-circuits) or close of an undefined budget; first close wins | Plugin rule over records |
| Foreign amounts | `zhang-core/src/process/budget.rs:98-143` | Keeps amounts in another commodity (not zero), then reports `BudgetCommodityMismatch` where `PriceMap::conversion` (with the cost currency as `via`) finds no rate | Host conversion (`convert`, `zhang_convert`) |
| Posting check | `zhang-core/src/process/transaction.rs:12-52` | Per *written* posting (legs merged): `BudgetDoesNotExist` or `BudgetClosed` once per (account, budget), else keeps foreign amounts | Plugin rule over BQL aggregates |
| `process/open.rs` | `zhang-core/src/process/open.rs:8-17` | Only the commodity check; it holds no budget state | (correction to the RFC's map) |
| Membership rule | `zhang-core/src/pipeline/active_accounts.rs:18-19,96-97,110-113,131-151` | The `budget` metas of an account's latest `open` at a wall-clock time; all accounts per budget | Generic "open metadata at a time" (§3.6) |
| Ledger accessors | `zhang-core/src/ledger.rs:304-315` | `account_budgets`, `budget_accounts` | Generic accessor, plus an alias |
| Tables | `zhang-query/src/table/budgets.rs:113,121,129`; registry `zhang-query/src/table/mod.rs:177-179` | `#budgets` (generated months), `#budget_definitions`, `#budget_events` | Plugin record and series tables (§3) |
| Counting rule | `zhang-query/src/table/budgets.rs:174-189` `Budget::counts` | Membership at the posting's time, folded after the definition, not closed, convertible | Plugin label predicate (§3.6) |
| Activity and carry-over | `zhang-query/src/table/budgets.rs:343-400,440-451,516-598` | Converts and sums per month, negated on credit-normal accounts; carry-over; series end; per-query recomputation | Plugin facts + host series fill |
| Series bound and error | `zhang-query/src/table/budgets.rs:537-564,603-657`; `zhang-query/src/optimizer.rs:224-252` | The end is the latest of the budget's own last month, the last transaction month and the current month, capped by the query's date bound; `TooLarge` names the budget and the directive | Generic series kind (§3.7) |
| Posting column | `zhang-query/src/table/postings.rs:634-642`, `budgets.rs:419-436` | `budgets` set column, the rule the budget page's posting list uses | Label column |
| Function | `zhang-query/src/functions/scalars.rs:78-79`, `scalars/ledger.rs:48-57` | `account_budgets(account, date)` | Alias of a generic function |
| Record plumbing | `zhang-query/src/table/mod.rs:293-298,311-313,395,452,473` | `Record::Budget*`, their metadata, the per-query fold cache | Generic `Record::Extension` |
| Small mentions | `zhang-query/src/period.rs:387-397` (budget directives don't move a period's last date); `table/entries.rs:154-157` (`type` names); `table/errors.rs:95-100` (messages) | Classification and text | Compat mapping |
| Named queries | `zhang-server/src/builtin.rs:105-114,343-381`; compiled once in a static `LazyLock` at `:443` | `budgets.month`, `budgets.budget`, `budgets.budget_month`, `budgets.events`, `budgets.postings` | Plugin-declared named queries (§8) |
| HTTP | `zhang-server/src/lib.rs:148-151`, `routes/query.rs` | `GET/POST /api/query/builtins`, `POST …/{name}`, `POST …/{name}/text` | Unchanged |
| Error kinds | `zhang-ast/src/error.rs:41-48` | `BudgetDoesNotExist`, `DefineDuplicatedBudget`, `BudgetCommodityMismatch`, `BudgetClosed`, metas `budget_name`, `account_name`, `commodity`, `budget_commodity` | Kept as legacy kinds (§2.6) |
| Web UI | `frontend/src/components/budget/budget-api.ts:13-41`, `budget-rows.ts`, `pages/Budgets.tsx`, `pages/SingleBudget.tsx`; types generated into `frontend/src/api/builtins.ts:393-460` | Runs the five named queries, batched | Unchanged |
| Flutter app | `zhang-app` `BudgetRepository`, `docs/zhang-api.md:199-204` | The same five named queries | Unchanged |
| Docs | `guides/budgets.md`, `reference/directives/budget.md` (+ zh-cn), `reference/builtin-queries.md`, `reference/query-language.md` | User contract | Updated in P5 |

### 1.2 Corrections to the map in #698

- `zhang-server/src/routes/budget.rs` no longer exists. #754 (`67d04456`) retired `GET /api/budgets`,
  `/api/budgets/{name}` and `/api/budgets/{name}/interval/{y}/{m}`, and both clients now read budgets only through
  the five named queries. `zhang-server/tests/api/error_bodies.rs:65` pins the 404. The stable surface P2 must keep is
  therefore the named queries (§8), not an `/api/budgets*` route.
- The "budget-specific state in `process/open.rs`" moved to the account lifecycle
  (`pipeline/active_accounts.rs`).
- The plugin platform has moved on since the RFC's snapshot `74a9f03b`:
  - #798: parameterised BQL and the schema for routers;
  - #801: compile reuse;
  - #802: Lua routers;
  - #803: gated writes;
  - #804/#805: `zhang:mcp`, bundled, and `zhang_query_builtin`;
  - #826: content generation;
  - #827/#830: limits and the operator ceiling;
  - #833: error ids and `txn_id`.

  §2 builds on these. Three gaps remain from the RFC's list: query compile caching for routers (built-ins only are
  cached), typed reads of the extension records, and everything about analyzers.

### 1.3 What the move costs, measured

A throwaway measurement on the builder (Linux x86_64, release build, a generated ledger of 10 budgets, 20%
foreign-currency postings; never committed):

| Ledger | Load, no plugin | Load with a Rust→wasm processor that returns the stream unchanged | Same, summing every posting per account and month with `BigDecimal` and `PriceMap::convert` | `#budgets` query today (warm cache) |
| ---: | ---: | ---: | ---: | ---: |
| 10,000 transactions (8.3 MB of stream JSON) | 126 ms | 417 ms | 410 ms | 1.0–1.5 ms |
| 50,000 transactions (41.7 MB) | 662 ms | 2,105 ms | 2,110 ms | 6.7–12 ms |

- Shipping the stream across the boundary and back adds about 290 ms per 10,000 transactions even in Rust (about
  30 µs each, 2.3 times the load itself). That figure includes the second booking pass any plugin of the booked slot
  triggers. The computation inside the plugin is noise next to it. The spike of #698 measured the same round trip at
  9.2 s for 10,000 transactions in Lua with dkjson, on an Apple M4.
- **Conclusion:** an analyzer must pull aggregates through queries and must not receive the stream. That rules out
  the processor-shaped ABI for this job, whatever the language.

## 2. The Analyzer extension point

### 2.1 Where it runs

```text
parse ─► options, plugin registration
      ─► raw processors/mappers ─► BookingStage ─► processors/mappers ─► (re-book) ─► ActiveAccounts ─► Pad
      ─► BalanceCheck ─► Validate ─► fold (ids, outcomes; no budget logic any more)
      ─► ANALYZERS, in declaration order (default plugins first)     ◄── new, read-only, final ledger
      ─► place_errors (txn_id) ─► identify_errors (ids)              ◄── analyzer diagnostics included
      ─► Ledger value returned: directives, outcomes, errors, snapshots
      ─► server: fingerprint, generation (#826), swap under the write lock
```

An analyzer cannot change a directive, a balance, an outcome or another plugin's output. It sees the ledger exactly as
every query and every page sees it.

### 2.2 The contract

**Declaration.** A module declares the type `Analyzer` in `supported_type`. Older hosts ignore an unknown type with a
warning (`zhang-core/src/plugin/store.rs:149-163`). It also declares a static **manifest**, an optional export read
once while the plugin registers, as `capabilities` is today (`store.rs:186-195`). In Lua the manifest is fields of the
module table. Because it is static, the catalog and the named queries are known before any analysis runs, even when
one fails.

```json
{
  "analysis_abi": 1,
  "namespace": "budget",
  "consumes": ["budget", "budget-add", "budget-transfer", "budget-close"],
  "open_meta": ["budget"],
  "diagnostics": [
    {"kind": "BudgetDoesNotExist"}, {"kind": "DefineDuplicatedBudget"},
    {"kind": "BudgetCommodityMismatch"}, {"kind": "BudgetClosed"}
  ],
  "tables": [ "… §3.2 …" ],
  "columns": [ "… §3.2 …" ],
  "queries": [ "… §8 …" ]
}
```

**Input of `analyze`.** The host passes the *records* of the kinds the plugin consumes, in ledger order, in a
versioned shape defined in `zhang-shared::plugin_abi::analysis`, and never zhang-ast's serde shape:

```json
{"records": [
  {"kind": "budget-add", "seq": 41, "date": "2024-03-01", "time": "00:00:00", "has_time": false,
   "timestamp": 1709251200, "fields": {"name": "Food", "amount": {"number": "500.00", "currency": "CNY"}},
   "meta": {"note": ["groceries"]}, "file": "budget.zhang", "line": 12}
],
 "ledger": {"timezone": "Asia/Shanghai", "operating_currency": "CNY"}}
```

- `seq` is the `seq` of `#entries` and `#postings`: the position in the order Zhang processed the ledger, comparable
  with a posting's.
- The fields of each kind come from the compat mapping (§5).
- A `custom "<type>"` directive is a record of kind `custom:<type>` with positional `values`. This gives third-party
  plugins typed access to their own `custom` config without the zhang-ast stream.
- When the ledger has no record of a consumed kind and no `open` with one of the `open_meta` keys, the host does not
  call the plugin: the snapshot is `skipped` and its tables are empty. A ledger without budgets pays nothing.

**Host functions while `analyze` runs.** These are the router's read-only ones, now bound to the ledger being loaded:

- `zhang_query`, `zhang_query_params`, `zhang_query_schema` and `zhang_ledger_info`;
- `zhang_now`, which records the date as an input, as stages do;
- one new function, `zhang_convert`.

`zhang_convert` converts a batch of `{amount, target, date, via?}` exactly as `PriceMap::conversion` and
`Position::convert` do, answering the converted amount or `null`. It is the core's "generic currency conversion"
from the RFC's responsibility table, not a budget function.

Records and query results together are the RFC's "typed reads of amounts, dates, metadata and source order" (`seq`).
Reads come in batches by construction: one parameterised query per question, paged with `LIMIT` and `OFFSET` when a
result could be large.

Analyzers get no files, no network and no writes, whatever their directive grants. They are pure functions of the
ledger, which is what makes their snapshots reusable (§2.4).

**Output of `analyze`.** One JSON document, validated as a whole before anything is published:

```json
{"diagnostics": [{"kind": "BudgetClosed", "target": {"seq": 812},
                  "metas": {"budget_name": "Fun", "account_name": "Expenses:Shared"}}],
 "tables":  {"budget_definitions": [ {"name": "Food", "date": "2024-01-01", "…": "…", "$meta": {"alias": ["Groceries"]}} ],
             "budget_events": [ "…" ]},
 "series":  {"budgets": {"partitions": [{"key": {"name": "Food"}, "until": "2024-11-01",
                                         "until_cause": {"seq": 980, "label": "a transaction"},
                                         "first_cause": {"seq": 3, "label": "its budget directive"},
                                         "rows": [ {"date": "2024-01-01", "assigned": {"number": "500.00", "currency": "CNY"}, "…": "…"} ]}]}},
 "labels":  {"budgets": [{"label": "Food", "params": {"accounts": ["Expenses:Food"], "defined_seq": 3,
                                                       "close_date": null, "close_time": null, "commodity": "CNY"}}]}}
```

### 2.3 Lifecycle (#698 open question 1)

**Recommendation: both, split by what each part needs.**

- **At load:** analysis runs eagerly, producing the diagnostics and every fact that grows with the ledger.
- **At query time:** the host alone does the work that grows with time, the months of a series (§3.7), bounded by
  the query.
- **Never** does plugin code run on first query.

| Option | Load diagnostics | Stable snapshot | Resource bound | Verdict |
| --- | --- | --- | --- | --- |
| Analysis after load (eager) | Yes: merged before ids and `txn_id`, so the journal's per-transaction errors and #833's ids work unchanged | Yes: part of the immutable `Ledger` value | Bounded by the ledger, not by time | **Recommended**, for facts and diagnostics |
| First-query computation (lazy) | No: the errors page and the journal would change after a query ran, and ids would depend on request order | Needs its own once-cell and race handling under the read lock | A first query pays the whole analysis under a reader's lock | Rejected for diagnostics. Possible later for an analyzer that declares no diagnostics (an owner decision; not needed for Budget) |
| Plugin code per query (a `table_rows` export) | Not applicable | The plugin must be handed its state on every call | wasm under the read lock per request, re-serialised each time, recursion risk | Rejected (§3.7) |
| Both, as recommended | Yes | Yes | The analysis is O(ledger); the series fill is O(rows the query keeps), with the result budget and the deadline | **Recommended** |

The analysis must not depend on the wall clock. Calling `zhang_now` records `ExtraInput::Clock`, so the midnight
reload of `zhang-server/src/lib.rs` recomputes the snapshot. Budget never calls it: "the current month" is the
query's `today()`, applied by the series fill.

### 2.4 Snapshots

- **Where.** A new `Ledger::extensions: ExtensionStore` (a map from plugin id to `Arc<Snapshot>`). It is written once,
  inside the load, before the `Ledger` value exists for anyone else, and immutable afterwards. This is separate from
  `Derived` (`zhang-core/src/derived.rs`), which stays the query engine's cache. That cache is warm after an analysis,
  since the analyzer's queries already built it.
- **Key.** Each snapshot records:
  - `(plugin name, module digest, config digest, analysis_abi, load id)`, where the config digest covers what the
    plugin's Extism config holds: the ledger's options, the directive's metas and, for a script, its source;
  - `status: ok | failed{reason} | skipped{reason}`.
- **Binding to the generation (#826).** A snapshot lives and dies with its `Ledger` value. The server swaps that value
  under the write lock, and the generation counts exactly those swaps that changed content. So a reader holding a
  `Ledger` can only ever see that ledger's snapshots, and no stale-read check is needed.
  - A module or config change changes the digests. It also changes the content fingerprint, which hashes plugin
    modules and options (`zhang-server/src/lib.rs:597-615`).
  - A reload therefore always re-analyses. No WASM instance outlives the `analyze` call.
- **Atomic publish.** The output is decoded, type-checked against the manifest, size-checked and turned into host
  values in full, and only then stored. Any failure stores `failed` with no table data, never half a snapshot.
  Multi-plugin atomicity comes for free: the ledger is swapped whole.
- **Reuse (optional, P4).** A reload of unchanged content can reuse snapshots whose module, config and ABI digests
  match. The input key is the fingerprint of the final ledger *before* analysis, and #826 shows it costs 3.5 ms at
  50,000 transactions. This is valid only because analyzers are pure (§2.2).

### 2.5 Failure isolation

| Failure | Effect |
| --- | --- |
| Trap, Lua error, timeout, out of memory, invalid or oversized output, unknown diagnostic kind, a value of the wrong type | The ledger **loads**; this differs from a processor, whose failure aborts the load (`writing-plugins.md`, "Reporting errors"). The snapshot is `failed`. The plugin's directive gets one diagnostic, `PluginAnalysisFailed`, a new generic kind with metas `plugin`, `reason` and `message`; a default plugin, which has no directive, gets it on the first directive it consumes. Its tables answer the query error `table #budgets is unavailable: plugin zhang:budget failed (see the ledger errors)`, and its label columns read as empty sets with the same error once per query. Other plugins are unaffected. |
| A query inside `analyze` fails | The plugin gets the error as a value (`{"Err": …}`), as routers do. It may handle it; uncaught, the row above applies. |
| An analyzer depends on another's table that failed | That query fails with the "unavailable" error. |

### 2.6 Diagnostics

- **Kind.**
  - A plugin may emit only the kinds its manifest declares.
  - The four legacy budget kinds keep their `ErrorKind` variants, OpenAPI enum and English message templates
    (`zhang-query/src/table/errors.rs:95-100`) in a core compat allowlist granted to `zhang:budget` alone.
  - Any other plugin's kind becomes a `PluginError` with metas `plugin`, `kind` (its own code) and `message`. The
    closed `ErrorKind` enum, which both clients decode, does not grow per plugin.
- **Target.** One of:
  - `{"seq": n}`: a directive, which covers records and transactions alike;
  - `{"txn": "uuid"}`;
  - `{"plugin": true}`: the plugin's directive.

  The host resolves it to the directive's `SpanInfo`, keeping file, offsets and the source text the errors page
  opens. Plugins never send spans; they cannot forge one.
- **Ids (#833).** `identify_errors` (`zhang-core/src/domains/schemas.rs:90-99`) numbers errors by (span, kind string,
  n). Because the analyzer's diagnostics are merged *before* `identify_errors` runs and the legacy kind strings are
  unchanged, ids are identical as long as the plugin emits diagnostics of one kind at one directive in the legacy order.
  - That order: directive order, then posting order within a transaction, then budget name within a posting (the
    fold iterates a `BTreeSet`).
  - Adds and transfers are reported at the end, as `report_unconverted_amounts` does; that matters only within a kind.
  - The plugin sorts by `(seq, posting_index, budget)`, and a P0 fixture pins it.
- **`txn_id`.** `place_errors` runs after the merge, so a diagnostic at a transaction gets `txn_id` and appears in that
  transaction's error list (`Detail::Transaction { errors }` of its `Outcome`) as today.
- **List order.** Analyzer diagnostics come after the fold's, as `BudgetCommodityMismatch` already does. `#errors`
  and the errors page sort by file position (`zhang-query/src/table/errors.rs:49`). At a transaction or budget
  directive no other fold error competes, so the visible order is unchanged; only the internal `Vec` order moves.
- **Volume.** At most 10,000 diagnostics per plugin and load, then one summary diagnostic `N more omitted`.

### 2.7 Resource limits

| Limit | Value (proposal) | Why |
| --- | --- | --- |
| Call time | The plugin's `timeout`, capped by `ZHANG_PLUGIN_MAX_TIMEOUT` (30 s) | A reload holds the server's **write** lock while it loads (`start_reload_listener`, `zhang-server/src/lib.rs:567`), so analysis time stalls readers. P4 measures it, and the owner sets a budget (decision 11). |
| Memory | `ZHANG_PLUGIN_MAX_MEMORY` (256 MiB per instance); Lua's 64 MiB of values | Unchanged |
| Each query | `POST /api/query` limits: 10 s and 1,000,000 values | Unchanged |
| Queries per analysis | 10,000 | Keeps a looping script finite before the timeout |
| Output | 1,000,000 values (the unit of the result budget), and 64 MiB of JSON | Bounds the host's copy |
| Series rows at publish | Sparse facts only, within the output cap | The dense months are generated per query, within that query's limits (§3.7) |
| Diagnostics | 10,000 | §2.6 |

### 2.8 Next to Processor, Mapper, Router and the operator ceiling

| | Processor / Mapper | **Analyzer** | Router |
| --- | --- | --- | --- |
| When | Load, before validation | Load, after the fold, before ids | Each HTTP request |
| Sees | The zhang-ast stream (ABI v1 view) | Records it consumes and BQL over the final ledger | BQL over the served ledger |
| Changes | The stream, so the ledger | Nothing; it publishes a snapshot | Nothing, or appends through gated writes |
| Failure | Aborts the load | Snapshot `failed`, a diagnostic, the ledger loads | The request fails (500/504) |
| Lock | Load (write lock during a reload) | Load (write lock during a reload) | Read lock for the call |
| Files, network | `allowed_paths`, `allowed_hosts` | None | None for files; network as granted |
| Lua | Not yet (needs §4) | Yes (needs §4 and an interpreter update) | Yes |

- **Operator ceiling.** `ZHANG_PLUGINS=off|read|write` (`zhang-core/src/plugin_policy.rs:39,57,93`) keeps bounding
  ledger-declared plugins. `read` and `write` mean nothing to an analyzer, which never writes. Default plugins are
  governed by a separate switch (§6). The timeout and memory ceilings apply to every analyzer.

### 2.9 The embedding seam

`zhang-core` cannot run BQL, because `zhang-query` depends on it. So the core defines the hook and `zhang-query`
implements it:

```rust
// zhang-core
pub trait AnalysisHost: Send + Sync {
    /// run every analyzer of `ledger` (final, read-only); diagnostics and snapshots come back
    fn analyze(&self, ledger: &Ledger, analyzers: &[RegisteredPlugin]) -> AnalysisOutcome;
}
```

- **Installing it.** The host is installed with the plugin policy (`Ledger::load_with_policy` grows a `LoadOptions`)
  and kept by `Ledger::reloader()`. `zhang_query::analysis::Host` implements it, with the catalog of §3.5 and the
  compile cache.
- **Entry points.** The server, the CLI and the test kit install it in one place.
- **Embedders without it.** These are `bindings/wasm` and plain-core tests. Without the host or the plugin runtime,
  the default plugins are unavailable and §6's diagnostic applies.

## 3. Plugin-provided typed query tables

### 3.1 Kinds

| Kind | Rows | Served by | Budget use |
| --- | --- | --- | --- |
| Record table | In full, in the snapshot, with optional row metadata (`$meta`) for `meta()`, `meta_values()`, `metas` | The host, as `Rows::Records` today | `#budget_definitions`, `#budget_events` |
| Series table | Sparse facts per partition, plus a declared fill policy; the missing periods are generated per query | The host, generalising `Rows::Generated` and `optimizer::date_bound` | `#budgets` |
| Label column | A `Set` column on `#postings`: the labels whose predicate holds for the row; one predicate template in the manifest, parameters per label in the snapshot | The host, evaluating compiled predicates | `postings.budgets` |
| Named query | BQL with typed parameters, in the manifest | The built-in query registry | `budgets.*` (§8) |

A plugin cannot add a column to any other core table, nor a scalar function, in v1. Section 3.6 handles
`account_budgets()`.

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
   "row_meta": "carry",
   "too_large_hint": "the #budgets table would generate too many rows ({total} months in all, more than the result size limit): budget '{partition}' runs from {first} until {end}{cause}"},
  {"name": "budget_definitions", "kind": "records", "columns": ["…"], "order": ["name"]},
  {"name": "budget_events", "kind": "records", "columns": ["…"], "order": "published"}],
 "columns": [
  {"table": "postings", "name": "budgets", "type": "set", "kind": "labels",
   "description": "The budgets the posting counts toward, …",
   "predicate": "account IN :accounts AND seq > :defined_seq AND :label IN open_meta_values(account, 'budget', date, time) AND (:close_date IS NULL OR (:close_time IS NULL AND date <= :close_date) OR (:close_time IS NOT NULL AND (date < :close_date OR (date = :close_date AND time <= :close_time)))) AND currency(convert(position, :commodity, date)) = :commodity",
   "params": {"accounts": "set", "defined_seq": "int", "close_date": "date", "close_time": "str", "commodity": "str"}}]}
```

- **Types.** Column types are the BQL `DataType`s (`str`, `int`, `decimal`, `date`, `bool`, `amount`, `set`,
  `metas`).
- **Wire encoding.** Values use the query API's cell encoding: an amount is `{number: "<decimal string>", currency}`.
  A JSON number in a `decimal` or `amount` cell is refused, as writes refuse it today.
- **Fill policies.** A series column's policy is `carry`, `zero` (zero in the previous value's commodity),
  `{"previous": "<column>"}` or `null`. Period columns are generated. Fills only copy values, so the host does no
  arithmetic.
- **Label parameters.** `:label` is bound to each label's name. The predicate is compiled once per catalog with the
  declared parameter types.

### 3.3 Type checking

| When | Check |
| --- | --- |
| Registration (manifest) | Names, types, fill policies (a `previous` column has the same type), wildcard ⊆ columns. The predicate compiles against `#postings` to `Bool`, row-level, with no aggregate and no plugin table. Named queries compile against the catalog with their declared parameters (§3.5). A failure fails registration, like a bad `supported_type`. |
| Publish (snapshot) | Every cell has the column's type or is NULL. Partition keys are present and unique per partition. Series rows are strictly increasing by period within a partition and aligned to the unit. Label params match the declared types. Record order is as declared. Any failure: `failed` (§2.5). |
| Query compile | As for core tables: columns resolve against the query's table only, functions type-check, `SELECT *` expands to the wildcard. |

### 3.4 Names and collisions

- **Core names win.** A plugin can never shadow a core table, a `#postings` column or a function, now or later.
  Future core names are protected by the namespace rule below.
- **Namespaces.**
  - A third-party plugin's tables, columns and named queries start with its manifest `namespace`, which is
    `[a-z][a-z0-9_]*` and defaults to the plugin name with `-` written `_`. Tables and columns are written
    `#<namespace>_<name>`, named queries `<namespace>.<name>`.
  - Two plugins with one namespace, or one name: the first declared serves it, and the later one is reported on its
    directive, the rule routers follow (`zhang-core/src/plugin/router.rs:347`).
- **Legacy names.** `#budgets`, `#budget_definitions`, `#budget_events`, `postings.budgets` and the `budgets.*` named
  queries are in a core allowlist granted to the module `zhang:budget`. They are also granted to a user script that
  replaces it under §6's override rule.
- **Not in the schema.** A plugin table that is declared but whose snapshot `failed` stays in the catalog, so a query
  gets "unavailable" and not "unknown table". A table whose plugin is not loaded at all is unknown, as today.

### 3.5 Compiler integration, the schema API and compile caches

- **Catalog.** Today `Plan.table` is `&'static Table` and the registry is a static slice
  (`zhang-query/src/table/mod.rs:166-191`).
  - P1 introduces a `Catalog`, the core tables plus the extension tables of a ledger's snapshots, and a
    `TableRef = Static(&'static Table) | Extension(Arc<ExtensionTable>)`.
  - `Query::compile_in(&catalog, text, params)` resolves against it; `Query::compile` keeps compiling against the core
    catalog.
  - An `ExtensionTable` holds the schema only. Executing a query reads the rows from the snapshot of the ledger it
    runs on, so a compiled query never captures data.
- **Catalog digest.** A hash of every extension table's schema. A query remembers the digest it was compiled with,
  and running it on a ledger with another digest is the error "the ledger's tables changed; compile again", never a
  wrong read.
- **Built-in queries.** `COMPILED` (`zhang-server/src/builtin.rs:443`) becomes a small cache keyed by catalog digest,
  holding the core and plugin named queries. A ledger usually keeps one digest across reloads, because a manifest
  changes only with its module.
- **Router and analysis queries.** `zhang_query*` compile on every call today
  (`zhang-server/src/routes/plugin_router.rs:373-380`). They get a per-catalog LRU keyed by `(text, parameter
  types)`: the "query compile cache" item of #698 P1. The budget analyzer runs one query template per budget, so this
  matters.
- **`GET /api/query/schema` and `zhang_query_schema`.** These become per served ledger: core plus extension tables.
  Every table and column gains `provided_by` (`null` for the core, `"zhang:budget"` …). The Explore page's editor
  and reference already read this endpoint (`frontend/src/components/query/QueryEditor.tsx`, `QueryReference.tsx`).
- **`EXPLAIN`** names an extension table as `#budgets (zhang:budget)`.

### 3.6 Membership as a generic function, and `account_budgets()`

The label predicate needs the `budget` metas of the `open` in effect at a time. P1 generalises
`AccountLifecycle::budgets` to keep each `open`'s metadata, and adds:

- `Ledger::open_meta_values_at(account, key, at)`;
- the BQL function `open_meta_values(account, key, date[, time]) -> Set`.

`account_budgets(account, date)` stays in the function table as a documented alias of
`open_meta_values(account, 'budget', date)`. The alias is compat mapping, with no budget rule behind it; deprecating
it is decision 12.

### 3.7 Date-bounded generation for `#budgets` (#698 open question 2)

**Today:**

- `#budgets` folds every budget, converts and sums the activity *on every query* (`budgets.rs:516-598`);
- it generates each budget's months up to the latest of its last own month, the last transaction month and the
  current month;
- a filter `date <= x` or `yearmonth(date) = x` caps that end (`optimizer::date_bound`);
- every generated row is charged to the result budget and the deadline.

**Proposed: the same bound, with the work split in two.**

1. **At load,** the plugin publishes per budget only *facts*:
   - a row for every month that has something: the definition month, a month with an add, a transfer, activity or the
     close;
   - each row with `assigned` and `available` computed by the running sum across the gaps, and `added`, `activity` and
     `closed`;
   - the partition's `until`: the latest of its own last month and the ledger's last transaction month;
   - `until_cause` and `first_cause`: the directive whose date sets that end, and the budget's definition, for the
     error message.

   The facts are O(budget directives + budget-months with activity), however far apart the dates are.
2. **At query time,** the host builds the dense series for each partition:
   - from its first fact to `max(until, current month if extend_to_today)`, capped by the query's `date_bound`
     (unchanged code, generalised to any series table);
   - a month without a fact is the previous row filled by the policy: `assigned` takes the previous `available`,
     `added` and `activity` are zero, everything else is carried. This gives exactly today's
     `assigned = available + added` and `available = assigned − activity` over an empty month;
   - every generated row costs `Limits::row()`.

   `TooLarge` keeps today's message from the manifest's `too_large_hint`: the partition, its months, and the cause.
   The cause is one of today's three: the `until_cause` directive, the definition (`first_cause`) when the current
   month sets the end, or the query's own bound. That is the contract of
   `zhang-query/tests/engine/zhang_tables.rs:859,901`.

**How predicates and limits reach the plugin:** they don't need to.

- The plugin's work is bounded by the ledger at load. The query's date bound, its result budget and its deadline
  apply where the rows are generated, in the host, exactly as now.
- A date typo, say a `budget-add` in 9999, costs one fact row. A page asking for one month still generates only up to
  that month (`a_date_typo_far_ahead_leaves_the_budget_pages_working`).

**Alternatives.**

| Alternative | Why not |
| --- | --- |
| Dense months materialised at load | A typo date materialises thousands of months per budget at every load, under the reload's write lock, and the pages break with the analysis instead of only the unbounded query |
| A plugin export called per query with `{today, until, projection, budget, deadline}` | wasm per request under the read lock, the snapshot re-serialised per call, a recursion hazard, and page latency tied to the plugin language |
| Window functions and `generate_series` in BQL | Generic and powerful, but a large engine project. A possible later replacement for fill policies, not a P1 dependency |

### 3.8 Recursion guard

- **During analysis.** While plugin *P* analyses, its queries compile against the core tables plus the tables of
  analyzers that ran *before* it. Analyzers run in declaration order, default plugins first. A query naming *P*'s own
  table, or a later plugin's, fails with `table #x is not available while zhang:budget analyses the ledger`.
  Dependencies are therefore acyclic by construction.
- **At query time.** No plugin code runs, so a table can never read itself. A label predicate may not name a plugin
  table (checked at registration).

## 4. Exact decimals across the boundary (#698 open question 5)

**What exists.**

- Every decimal crosses the boundary as a string in plain notation (`zhang-shared/src/decimal.rs`, "Writing a
  number").
- Lua's `zhang.decimal(v)` is only a query-parameter constructor (`guides/lua-plugins.md`).
- The Rust SDK re-exports `bigdecimal` and computes with `zhang_shared::decimal` (`zhang-plugin-sdk/src/lib.rs:74`).
- Zhang's rounding policy is Python's default decimal context: exact sums and products, division and
  rate-products rounded half-even to 28 significant digits.

| Option | Exactness | Cost per operation | Effort and risk | Verdict |
| --- | --- | --- | --- | --- |
| A. Strings on the wire, arithmetic by host calls (`zhang_decimal(op, a, b)`) | Exact, identical to the host | One boundary crossing plus JSON per operation: microseconds. Fine for hundreds of operations, poor for a processor | Small host code; awkward to use | Not recommended: sums go through BQL instead |
| B. A pure-Lua decimal library | Exact if correct | Tens of µs per operation | Re-implementing scale and division rules in Lua; a parity risk for `div` and `mul_in_context` | Fallback for add/sub/compare only |
| C. **libmpdec in the interpreter, as a `zhang.decimal` userdata** | Exact; *is* Python's decimal, so the 28-digit half-even context matches the rule `zhang_shared::decimal` emulates | Native: well under a µs | Vendor libmpdec (BSD) into `tools/lua-runtime` with the existing reproducible build and CI checksum. The interpreter grows by an estimated 150–300 KB. Parity property tests against `zhang_shared::decimal` | **Recommended for Lua** |
| D. Rust SDK types | `BigDecimal` + `zhang_shared::decimal`: the host's own code | Native in wasm | Exists; add a `decimal` module to the SDK | **Recommended for Rust** |
| E. Avoid arithmetic in plugins: aggregates and conversion in the host | Exact by construction | One query per aggregate | Already there (`sum`, `convert`, `possign`), plus `zhang_convert` (§2.2) | **Recommended as the first resort**, whatever the language |

**Recommended v1.**

- The wire type stays decimal strings, and the rule "a JSON number where a decimal is expected is refused" extends to
  analyzer output.
- Plugins aggregate and convert through the host (E). In Lua they compute the rest with `zhang.decimal` (C); in Rust,
  with `zhang_plugin_sdk::decimal` (D).
- Budget itself needs only running sums, comparisons and zero tests in the plugin.
- `zhang.decimal` is the generic API the review asked for. It is the prerequisite for Lua processors and mappers,
  which this design does not enable.

```lua
local d = zhang.decimal                       -- api_version 2
local available = d("0")
for _, fact in ipairs(facts) do               -- strings from the host
  local assigned = available + d(fact.added)  -- exact; '+' on userdata
  available = assigned - d(fact.activity)
  row.assigned = { number = assigned:plain(), currency = commodity }  -- "1234.50", never 1.2345E+3
end
d("1") / d("3")      --> 0.3333333333333333333333333333 (28 digits, half-even, as zhang_shared::decimal::div)
d(0.1)               --> error: decimal must be a string or an integer, got 0.1
json.encode({x = d("12.50")})  --> {"x":"12.50"}
```

```rust
use zhang_plugin_sdk::decimal::{self, BigDecimal};   // re-exports zhang_shared::decimal
let available: BigDecimal = facts.iter().fold(BigDecimal::from(0), |acc, f| acc + &f.added - &f.activity);
let rate_product = decimal::mul_in_context(&units, &rate);   // the host's rounding, bit for bit
```

## 5. AST and syntax (#698 open question 3)

| Option | Export and spans | Effect on ABI v1 plugins | Effort | Verdict |
| --- | --- | --- | --- | --- |
| A. **Keep the four `Directive::Budget*` variants as syntax only**: a compat mapping in the core defines, per kind, its generic record fields (`name`, `commodity` / `name`, `amount` / `from`, `to`, `amount` / `name`). Analyzers read records (§2.2). | Unchanged: the same parser, exporter, Beancount mapping and spans | None: processors keep seeing `{"Budget": …}` | Small: one mapping function, plus `period.rs` and `entries.rs` asking "is this an extension record" | **Recommended** |
| B. A generic `Directive::Record { kind, date, fields, meta }` replacing the four | Needs a per-kind writer table in both exporters | Breaking: an ABI v1 plugin cannot read a new kind, so `AbiV1View` must map records back to the four legacy shapes forever, as it hides `pad` (`pipeline/plugin_view.rs:1-33`) | Medium; touches the parsers, exporters, OpenAPI and SDK | Later, only if a second business plugin needs first-class syntax |
| C. Map the zhang keywords to `Custom` at parse time | `budget-add Food 10 CNY` would export as `custom "budget-add" …`; `Custom` values have no amount type | Changes what processors see | Small but lossy | Rejected |
| D. Plugin-owned parsing | Plugins register syntax | n/a | Plugins load after parsing (their directives are in the parsed files); parse errors and both dialects would need a parser ABI | Rejected |

**Under A,** the zhang-ast variants carry no behaviour once P5 lands:

- the core's fold ignores them;
- `#entries` still lists them with their `type` names;
- `period.rs` excludes them as extension records;
- the exporters write them as written.

The Beancount `custom "budget*"` forms keep mapping onto the same four variants, so a Beancount ledger and a Zhang
ledger reach the plugin as identical records. Third-party plugins get typed records of their own
`custom "<type>"` directives through `consumes`, the generic route for new syntax. This is what the SDK's `custom`
module does today, minus the stream.

## 6. Official default plugins (#698 open question 4)

**Mechanism.**

- `zhang:budget` is a bundled plugin (`zhang-core/src/plugin/bundled.rs`, file `plugins/budget/budget.lua`) in the
  reserved `zhang:` namespace of #805, and is marked *default*. As for `zhang:mcp`, being bundled grants nothing; an
  analyzer needs no grant anyway.
- A default plugin is registered for every ledger: it needs neither a `plugin` directive nor `features.plugin`. That
  option, and `ZHANG_PLUGINS`, govern only code the ledger brings.
- Default plugins register and analyse first, so their tables are available to the ledger's own analyzers. They are
  bounded by the timeout and memory ceilings.

| Situation | Behaviour |
| --- | --- |
| Normal build, nothing declared | `zhang:budget` runs; budgets work as today |
| `option "features.plugin" "false"`, or absent | `zhang:budget` still runs: the option gates ledger-declared plugins only |
| `plugin "zhang:budget"` declared, with metas | It configures the default plugin (`timeout`, its own settings). It is not a second instance. It needs no `features.plugin`. |
| `ZHANG_PLUGINS=off` | Ledger-declared plugins report `PluginsDisabled`, as today. `zhang:budget` still runs. The startup log line (`PluginPolicy`'s `Display`) says so. |
| `ZHANG_DEFAULT_PLUGINS=off` (new operator switch; `zhang serve --default-plugins off`) | No default plugin runs. The diagnostic below is reported, and the budget tables and named queries are absent. |
| A user script replaces it: `plugin "plugins/budget.lua"` whose manifest declares `replaces: "zhang:budget"` | Needs `features.plugin` and is bounded by `ZHANG_PLUGINS`, as user code. It takes over the legacy names (§3.4) and the default plugin is skipped. This is how a user forks the budget rules, as `mcp.lua` can be forked. |
| Build without `plugin_runtime`, or an embedder without an `AnalysisHost` (`bindings/wasm`, a plain `zhang-core`) | No default plugin can run; the diagnostic below. |
| The analysis fails | §2.5: `PluginAnalysisFailed`; tables "unavailable". |

**The diagnostic when budget support is missing:**

- **When.** It is reported only when the ledger uses budgets, so ledgers without budgets stay clean: a record of a
  consumed kind exists, or an `open` has `budget` metadata.
- **Kind and place.** It is a new generic error kind, `ExtensionUnavailable`, reported once on the first such
  directive.
- **Metas.** `plugin: zhang:budget`, `kinds: budget, budget-add, …` and a `reason`:
  - `default plugins are off (ZHANG_DEFAULT_PLUGINS)`;
  - `this build has no plugin runtime`;
  - `the plugin failed: …`.
- **Message.** `Budget directives are not interpreted: {{meta.reason}}`.
- **Silent failure ruled out.** The rule "a user switch must not silently disable budgets" holds both ways: the
  ledger's switches cannot disable them, and the operator's switch that can is never silent.
- **Listing.** `GET /api/plugins` gains `bundled`, `default` and `status` (`ok`, `failed`, `disabled`). This is the
  optional item #805 deferred. The web UI can hide the Budgets entry when the plugin is absent (P4).

## 7. The language of the official Budget plugin

| | Lua (bundled script) | Rust → wasm via the SDK (bundled module) | Native "first-party plugin" trait compiled in |
| --- | --- | --- | --- |
| Request-time cost | **None.** Pages run named queries over host tables (§3); no plugin code runs. A Lua router call would cost 8.9 ms p50 and 66 ms p95 (#698's compile-reuse data); budget pages never make one. | None | None |
| Load-time cost (estimate: 10,000 transactions; 30 budgets × 10 years with monthly adds, so 3,600 records) | About 120 queries (4 per budget, 0.3–1 ms each, account-scoped), plus about 0.9 MB of records in and 1.7 MB of facts and events out. With dkjson at 2–3 MB/s that is **about 1 s** on a 126 ms load; with a C JSON codec (an estimated ≥ 30 MB/s) **about 0.15 s**. | About 0.1 s: the round trip in §1.3 moved about 57 MB/s; the compute is noise | About 0.05 s |
| Exact decimals | Needs `zhang.decimal` (§4, option C) | Exists (`zhang_shared::decimal`) | Exists |
| Parity risk | A rewrite of about 500 lines. The arithmetic and conversion are host-side (BQL, `zhang_convert`); the plugin keeps running sums and the rules. | Today's `budgets.rs` logic can be moved nearly verbatim | Lowest |
| Proves the Extism boundary and the language-neutral ABI | Yes, with the users' language | Yes, with a language D1 removes from users | **No**: business code in the binary behind a Rust trait; contradicts "执行边界继续使用 Extism / WASM" |
| Users can read, copy and fork it | Yes, like `mcp.lua` (§6's `replaces`) | Needs a Rust toolchain and the wasm target | No |
| Build and distribution | One `.lua` file, `include_bytes!` | A second vendored wasm artifact with a reproducible CI rebuild, like `lua-runtime.wasm` | Plain crate |
| Builds without a plugin runtime | Unavailable (§6 diagnostic) | Unavailable | Would work |
| D1 | Consistent | Keeps the SDK first-class for official code | Sidesteps it |

**Recommendation: Lua, behind gate G1.**

**Why Lua.**

- **The load-time cost.** The analysis-plus-host-tables design moves every per-request cost into the host, so the
  language only affects load time.
- **Alignment.** Lua is what the RFC names as the first script target and what D1 makes the users' language.
- **Dogfooding.** The official plugin exercises the extension point exactly as third parties will, which forces
  `zhang.decimal` and the analyzer bridge to be real.

**Gate G1** (end of P1). A Lua prototype of the budget analyzer runs on P0's baseline ledgers. Proposed pass bar:

- added load time at most 15% of the baseline load or 150 ms, whichever is larger, at 10,000 and at 100,000
  transactions (a budget-heavy ledger of 10,000 transactions cannot meet 15% alone, in any language, per the estimate
  above);
- exact parity on P0's fixtures.

**JSON codec, then fallback.**

1. The estimate above makes dkjson the bottleneck for a budget-heavy ledger. P1.6 therefore replaces it with a C
   codec in the interpreter, keeping every rule of the `json` table. That also settles the review's "dkjson is slow"
   item for every Lua plugin, `zhang:mcp` included.
2. If G1 still fails, implement the identical manifest and contract in Rust via the SDK. Only the plugin changes.

The trade-off: the Lua route puts more work into P1 (decimals, a JSON codec and the analyzer bridge in
the interpreter), and that work benefits every Lua plugin. The Rust route needs a second vendored wasm artifact and
keeps the SDK first-class for official code.

The native trait is rejected as the official form. A native in-process provider is still built in P1.3, but only as
test scaffolding that re-hosts today's code on the generic tables (§10), and it is deleted in P5.

**D1 and Budget.**

- **The conflict.** D1 ("no wasm authored by users: plugins are Lua scripts", `plugins/mcp/DESIGN.md:21`, and PR #823)
  contradicts the shipped and documented Rust SDK (`developers/writing-plugins.md`).
- **Effect on Budget.**
  - With a Lua Budget, D1 and the official plugin agree. The Analyzer ABI needs a Lua bridge first, and the SDK needs
    it only if the SDK stays (decision 6).
  - With a Rust Budget, the SDK becomes load-bearing for official code while D1 tells users it is not their path. Two
    first-class implementations of every new surface follow.
  - Either way the ABI stays language-neutral (wasm exports plus JSON), so the choice is reversible.

## 8. API and frontend (P2)

**The contract** (none of it may change before P5, and P0 pins it):

- the five named queries:
  - their names and parameters (`month` and `name`, required, no server default);
  - their column names and types, and row order;
  - their batch form `POST /api/query/builtins`;
  - their `/text` form, which inlines the parameters into BQL that Explore runs;
- the tables that BQL names: `#budgets`, `#budget_definitions`, `#budget_events`, `postings.budgets`, with their
  columns, descriptions and wildcards;
- the four error kinds and their metas;
- the generated `frontend/src/api/builtins.ts`.

**How the plugin serves it.** It serves it without routes.

- **Named queries in the manifest.** The five queries move from `zhang-server/src/builtin.rs:343-381` into
  `zhang:budget`'s manifest, text unchanged:

  ```lua
  queries = {
    { name = "budgets.month", description = "Every budget as of a month …",
      bql = BUDGETS_AS_OF_MONTH .. " ORDER BY name", params = { month = "date" } },
    -- budgets.budget, budgets.budget_month, budgets.events, budgets.postings
  }
  ```

- **Registry.** The built-in registry merges core and plugin named queries per catalog (§3.5).
  - `GET /api/query/builtins` lists both, with `provided_by`.
  - `POST …/{name}`, the batch form, `/text` and the routers' `zhang_query_builtin` find them unchanged.
- **Unavailable support.** When the plugin is not loaded (default plugins off, no runtime), the names are absent and
  a call is the existing 404 `NoSuchBuiltinQuery`. When its analysis failed, the names exist (the manifest is static)
  and a call answers the 400 query error "table #budgets is unavailable". The clients show their existing error; P4
  adds the Budgets-entry hiding of §6.
- **Frontend types.** The test that writes `builtins.ts` (`zhang-server/src/builtin.rs:740-760`) reads the bundled
  manifests too, so the file is generated, byte-identical, from the plugin.
- **Plugin routes.** None are needed. If a later Budget page needs an endpoint that BQL cannot express, it is a
  `Router` on the same module under `/api/plugins/zhang-budget/…`, with the router rules of #827 (read lock,
  concurrency, body limits). Plugin-provided pages in the Fava style (see #698) stay out of this migration, as the RFC
  says.

## 9. Risks, alternatives, and what stays in the core

### 9.1 Risks

| Risk | Mitigation |
| --- | --- |
| **Parity drift**, in several places: <br>• diagnostic order (so ids); <br>• the per-written-posting check against booked legs (a sale across lots with different cost currencies); <br>• `possign` (root name) against `Account::get_account_sign`; <br>• the transfer short-circuit; <br>• a zero foreign amount | • P0 fixtures pin ids, metas and `txn_id`. <br>• P3 runs native and plugin side by side on every fixture and on `budget_reference`, the independent oracle. <br>• The cases listed are named tests in P3.2. |
| **Load-time regression** under the reload's write lock | • G1 and P4 thresholds. <br>• Snapshot reuse across same-content reloads (§2.4). <br>• A generic follow-up: load outside the write lock and swap under it. |
| **The catalog refactor touches the whole query engine**: `&'static Table` is everywhere | • Done first, behaviour-neutral (P1.1). <br>• Guarded by the optimizer's equivalence tests and the oracle suites. |
| **The interpreter grows**: libmpdec and a C JSON codec | • Vendored with checksums; CI rebuilds byte for byte (`tools/lua-runtime`). <br>• A size budget recorded in `PINS.toml`. |
| **Operators expect `ZHANG_PLUGINS=off` to stop all plugin code** | • The startup log and `GET /api/plugins` say which defaults run. <br>• `ZHANG_DEFAULT_PLUGINS=off` exists. <br>• The upgrading guide covers it. |
| **No-runtime builds** (`bindings/wasm`) lose budget interpretation | Explicit `ExtensionUnavailable`; accepted (decision 4). |
| **A second business plugin needs more**: writes from an analyzer, new syntax, functions | • Out of scope. <br>• `custom` records and label columns are the generic routes. <br>• Recorded as follow-ups, not P1 work. |
| **ABI creep** | • The analysis ABI is its own versioned module (`analysis_abi`), additive only, and independent of zhang-ast. <br>• This answers the review's "weak versioning" point for every new surface. |
| **Plugin BQL evaluated by the host** (label predicates, named queries) | • Compiled at registration. <br>• Run under the query's deadline and result budget, read-only, over the same ledger. <br>• No new data exposure. |

### 9.2 Alternatives considered

| Alternative | Why not |
| --- | --- |
| Move only the API (P2) and keep the rules native | Fails #698's acceptance: "核心与查询引擎不再保留第二套预算业务实现" |
| Analyzer receives the full zhang-ast stream | +30 µs per transaction even in Rust (§1.3), 9 s per 10,000 in Lua with dkjson; ties a new surface to zhang-ast's weak versioning |
| Per-posting annotations output for `postings.budgets` | O(postings) across the boundary on every load; the predicate template is O(budgets) |
| Lazy analysis on first query | §2.3 |
| Plugin code per query for generated tables | §3.7 |
| Native first-party trait as the official form | §7 |
| A generic `Directive::Record` now | §5 B; deferred until a second plugin needs syntax |

### 9.3 What stays in the core

**Generic, owned by the core:**

- booking, lots and costs; pads and balance assertions; the account lifecycle (open, close, open metadata at a time);
- exact decimals (`zhang_shared::decimal`); the price map and conversion (`convert`, `zhang_convert`);
- the query engine with its catalog, series tables, label columns and named-query registry;
- the analyzer host, snapshots and diagnostics plumbing; the operator policy.

**Compat mapping, with no rule behind it:**

- the four budget directive kinds, their parsing, export, spans and Beancount `custom` mapping;
- the four legacy error kinds, their messages and metas;
- the legacy table and query names allowlist;
- the `account_budgets()` alias.

## 10. Phased plan

Each row is one pull request, except the gate G1. The table gives scope, tests and exit criterion; ⛳ marks an owner
decision point.

| PR | Scope | Tests | Exit criterion |
| --- | --- | --- | --- |
| **P1.0** `docs(developers)` | This page | Docs build | ⛳ Owner decides §11 |
| **P1.1** `refactor(query)` catalog | `Catalog`, `TableRef`, `Query::compile_in`; built-in compile cache by catalog digest; per-catalog LRU for `zhang_query*`; schema API per ledger, with `provided_by: null` everywhere | Every engine, oracle, API and golden suite unchanged; digest stability; a compiled query refused on another digest | Behaviour-neutral; `GET /api/query/schema` byte-identical apart from `provided_by` |
| **P1.2** `feat(core)` open metadata at a time | The lifecycle keeps open metas; `open_meta_values(account, key, date[, time])`; `account_budgets` becomes an alias | Reopen and same-day cases; every budget test unchanged | Budgets unchanged; the new function documented |
| **P1.3** `feat(query)` extension tables, proven natively | Record tables with row metadata, series tables (fill policies, generalised `date_bound`, the `TooLarge` hint), label columns (predicate templates, the `:x IN column` fast path). Native budget code re-hosted on them through an in-process provider that publishes the same facts and labels | All budget suites (`zhang_tables.rs`, `data_tables.rs`, golden `budget_*`, `query_zhang_tables.rs`) unchanged; fill and bound unit tests; performance not worse than today on P0's ledgers | The generic mechanisms reproduce today's budgets exactly, before any plugin exists |
| **P1.4** `feat(plugin)` Analyzer | ABI module `plugin_abi::analysis` (manifest, records, output); the `AnalysisHost` hook; snapshot store; diagnostics merge (allowlist, targets, ids, `txn_id`, caps); failure isolation; limits; `zhang_convert`; recursion guard; SDK `analyzer:` and `decimal`; `PluginAnalysisFailed` | wat and SDK fixtures: success, trap, timeout, OOM, invalid output, wrong types, recursion, name collisions, determinism, reload with module or config change | Every failure mode loads the ledger with one diagnostic; no partial snapshot |
| **P1.5** `feat(plugin)` named queries | Plugin-declared queries in the registry; namespaces; `GET /api/query/builtins` `provided_by`; `builtins.ts` generation reads bundled manifests | Registry merge, collisions, `/text`, batch, `zhang_query_builtin` | Core queries unchanged; a fixture plugin's query listed and runnable |
| **P1.6** `feat(plugin)` Lua analyzers | Interpreter: `Analyzer` type, the manifest in the module table, `zhang.decimal` (libmpdec), `zhang.convert`, a C JSON codec behind the unchanged `json` rules, `api_version` 2 | Decimal parity property tests against `zhang_shared::decimal` (10⁵ random cases per operation); JSON rule tests (null, empty tables, sorted keys, numbers); existing Lua and MCP tests unchanged; reproducible-build check | Module checksum reproducible in CI; size and JSON throughput recorded |
| **P1.7** `feat(core)` default plugins | Default bundled plugins, `ZHANG_DEFAULT_PLUGINS`, `replaces`, `ExtensionUnavailable`, the `GET /api/plugins` fields | Matrix: `features.plugin` × `ZHANG_PLUGINS` × default on/off × runtime present/absent × a replacing script | §6's table holds, row by row |
| ⛳ **G1** | Lua budget-analyzer prototype (throwaway) on P0's baseline ledgers | Load-time and parity measurements | Pass: Lua. Fail: C JSON, then the Rust fallback (§7) |
| **P2.1** `feat(budget)` | `plugins/budget/budget.lua` as default `zhang:budget`, declaring the five named queries (moved from `builtin.rs`), still reading the native tables | P0 contract fixtures; `builtins.ts` byte-identical; API and golden suites | Both clients unchanged; the queries are the plugin's |
| **P3.1** `feat(budget)` definitions and events | The analyzer publishes `#budget_definitions` and `#budget_events`, plus directive diagnostics (`DefineDuplicatedBudget`, `BudgetDoesNotExist`, mismatched adds and transfers). Test switch `native\|plugin\|both` | Dual run on every fixture and every `integration-tests/budget-*` ledger: rows, diagnostics, ids identical | `both` reports no difference |
| **P3.2** `feat(budget)` activity, series, labels | Activity by BQL per budget, carry-over facts, `#budgets` as a series table, `postings.budgets` labels, posting diagnostics | Dual run plus the `budget_reference` oracle; the named §9.1 edge cases; typo-date tests; P0 performance smoke | `both` reports no difference |
| ⛳ **P3.3** `feat(budget)!` switch | The plugin is the only runtime implementation; native code remains only behind the test switch | P0 fixtures identical, ids included | ⛳ Owner confirms the switch |
| **P4.1** `test(budget)` performance | Paired native and plugin comparison: 10,000 and 100,000 transactions, many budgets, currencies and months, abnormal dates; first computation, repeat queries, load and reload, concurrent requests; p50/p95, memory, bytes across the boundary | Bench harness from P0 | ⛳ Thresholds from P0 met |
| **P4.2** `test(budget)` isolation and lifecycle | Module or config swap, clock change at midnight, concurrent reload and query, timeout, OOM, trap, invalid output, missing plugin, the operator matrix | Integration tests | No stale or partial budget data is ever readable |
| **P4.3** `ci(release)` | Bundled script in every artifact; release asset `zhang-budget-<version>.lua`; container first start and upgrade canary | Release workflow dry run | Canary green |
| **P5.1** `refactor(core)!` delete native | Removes:<br>• `process/budget.rs`, the budget fields of `Ledger`, `check_budgets`, the dispatch arms;<br>• `zhang-query/src/table/budgets.rs`, `Record::Budget*`, `Dataset::budgets`;<br>• `posting_budgets`, `as_budget_interval`, the native provider and the switch.<br>Keeps the AST, the compat mapping, the legacy kinds and the alias. | Every suite green; P0 fixtures identical | `rg -i budget` in `zhang-core/src` and `zhang-query/src` finds only the compat mapping |
| **P5.2** `docs` | Budgets guide (plugin, version, troubleshooting, `ExtensionUnavailable`), the query-language reference (tables provided by `zhang:budget`), plugin development (Analyzer, tables, `zhang.decimal`), deployment and upgrading (`ZHANG_DEFAULT_PLUGINS`), English and zh-cn | Docs build | Done |

**Alignment with lane P0.** Its baseline page (`developers/budget-baseline.md`) and fixtures are not on a branch yet.
The plan assumes P0 delivers:

1. contract fixtures for the five named queries, the three tables, `postings.budgets`, the diagnostics (kinds, metas,
   spans, ids, `txn_id`) and parse/export round trips, on the existing corpus:
   - `integration-tests/budget-*`;
   - `zhang-server/tests/fixtures/journals/tiny-budget`;
   - the golden suites, `data_tables.rs` and `budget-rows.fixture.json`;
2. baselines: load time, query p50/p95, memory and bytes across the boundary, at 10,000 and 100,000 transactions.

Every PR from P2.1 on must keep (1) byte-identical. G1 and P4.1 compare against (2).

## 11. Decisions for the owner

| # | Decision | Options | Recommendation | Consequences of the recommendation |
| --- | --- | --- | --- | --- |
| 1 | **Derived data lifecycle** (#698 Q1) | a) analysis at load; b) first query; c) load for facts and diagnostics, host-side bounded generation per query | **c** | Diagnostics, ids and `txn_id` work unchanged. Load time grows by the analysis. No plugin code at request time. |
| 2 | **Bounded `#budgets` generation** (#698 Q2) | a) series table with host fill; b) plugin export per query; c) dense months at load | **a** | One generic series kind in the engine. The typo-date behaviour and `TooLarge` message are unchanged. The fill policy is a small declarative surface. |
| 3 | **AST and syntax** (#698 Q3) | a) keep the four variants as syntax only, with a record view; b) generic `Directive::Record`; c) map to `Custom`; d) plugin parsing | **a**, revisit b only for a second plugin's syntax | No change to parsing, export, spans or ABI v1. zhang-ast keeps four data-only types. |
| 4 | **Default plugins and builds without a runtime** (#698 Q4) | a) implicit default `zhang:budget`, outside `features.plugin` and `ZHANG_PLUGINS`, with `ZHANG_DEFAULT_PLUGINS`; b) require `plugin "zhang:budget"` plus `features.plugin`, migrating ledgers; c) keep a native fallback for no-runtime builds | **a**, no native fallback | Budgets work out of the box. The playground (`bindings/wasm`) reports `ExtensionUnavailable` for budget ledgers. Operators get one new switch. |
| 5 | **Exact decimals** (#698 Q5) | a) host arithmetic calls; b) pure-Lua library; c) libmpdec `zhang.decimal`; d) SDK `decimal` | **c + d**, with host aggregation first; b as fallback | The interpreter grows by an estimated 150–300 KB. Lua gets Python-identical decimals. This also unblocks Lua processors and mappers later. |
| 6 | **D1 versus the Rust SDK** | a) D1 binds users; the SDK stays maintained at ABI parity, with no official plugin on it; b) retire the SDK and user wasm; c) drop D1 | **a** | PR #823's contradiction is resolved by scoping. The SDK gets `analyzer:` (cheap; it is how the host's fixtures are written). Documentation says Lua is the recommended path. |
| 7 | **Language of `zhang:budget`** | a) Lua; b) Rust → wasm; c) native trait | **a**, behind gate G1 | Requires P1.6 (decimals, JSON codec, analyzer bridge). A failed G1 costs a rewrite of the plugin only, not of the contract. |
| 8 | **Default-plugin operator policy** | a) `ZHANG_PLUGINS` governs ledger-declared code only, plus `ZHANG_DEFAULT_PLUGINS`; b) `ZHANG_PLUGINS=off` also stops defaults; c) defaults cannot be disabled | **a** | A user switch never silently disables budgets; the operator switch is explicit and diagnosed. |
| 9 | **Analyzer data access** | a) pushed records plus pulled BQL and `zhang_convert`; b) the full zhang-ast stream | **a** | Bytes across the boundary scale with budgets × months, not transactions (§1.3). It needs P1.1's compile cache and the `seq` column. |
| 10 | **Diagnostic kinds of plugins** | a) the legacy kinds allowlisted for `zhang:budget`, other plugins `PluginError` with a `kind` meta; b) an open `ErrorKind::Extension(String)` | **a** | Both clients' error enums stay closed. A third-party kind is a meta, not a type. |
| 11 | **Analysis time budget under the reload write lock** | a) the plugin timeout and ceiling, measured in P4; b) a separate lower ceiling (e.g. 10 s); c) restructure reloads to load outside the lock | **a** now, c as a generic follow-up if P4 shows stalls | No new knob in P1. The risk stays visible in P4.2. |
| 12 | **Legacy names** | a) keep `#budgets`, `#budget_definitions`, `#budget_events`, `postings.budgets`, `budgets.*` and `account_budgets()` forever as an allowlist; b) deprecate them in favour of `#budget_*` namespaced names | **a**; deprecate only `account_budgets()` in docs | Explore queries, saved queries and both clients keep working. |
| 13 | **`postings.budgets`** | a) label column from a predicate template; b) per-posting annotations in the snapshot | **a** | No per-posting data crosses the boundary. One generic, reusable tagging mechanism. |
| 14 | **Pass bars** for G1 and P4 | Proposed: added load ≤ max(15% of the baseline, 150 ms) at 10,000 and 100,000 transactions; named-query p95 ≤ 1.1× the baseline; memory ≤ +10% | Owner sets them | They decide the language fallback and the P5 go. |

## Appendix A: the budget analysis, step by step

This is what `budget.lua`'s `analyze` does. Every arithmetic step marked *host* is exact in Rust.

1. **Records, in `seq` order.**
   - The first `budget` of a name defines it; another gives `DefineDuplicatedBudget`.
   - An `add` or `close` of an undefined budget gives `BudgetDoesNotExist`. A `transfer` checks `from`, then, only if
     it exists, `to`.
   - The first `close` wins. The plugin also builds the `#budget_events` rows and the `#budget_definitions` rows.
2. **Accounts.** `SELECT seq, accounts, meta_values('budget') FROM #entries WHERE type = 'open'` gives every account
   whose `open` ever names a budget.
3. **Per budget, the label parameters:** `{accounts, defined_seq, close_date, close_time, commodity}`. These are also
   the `postings.budgets` labels.
4. **Per budget, the activity:** one query of *host* sums and conversions.

   ```text
   SELECT yearmonth(date) AS m, sum(possign(number(convert(position, :commodity, date)), account))
   WHERE <the label predicate> GROUP BY m
   ```

5. **Per budget, the posting diagnostics, each `GROUP BY seq, posting_index` (written postings):**
   - foreign amounts not converted, through `zhang_convert` with `via` the cost currency, giving
     `BudgetCommodityMismatch`;
   - the first transaction per account before the definition, or at all when the budget is never defined, giving
     `BudgetDoesNotExist`;
   - the first transaction per account after the close, giving `BudgetClosed`.
6. **The added amounts.** The amounts of adds and transfers are converted at their dates through `zhang_convert`
   (*host*) and summed per month with `zhang.decimal`. An amount no price converts gives `BudgetCommodityMismatch`, at
   the directive.
7. **The facts.** The fact months are the definition month, the months with an add, a transfer or activity, and the
   close month. The running sums give `assigned` and `available`. `until` and `until_cause` come from the budget's
   last record and from `SELECT seq, date FROM #transactions ORDER BY date DESC, seq DESC LIMIT 1`; `first_cause` is
   the definition.
8. **Output.** Diagnostics are sorted by `(seq, posting_index, budget)`; then come the tables, the series and the
   labels.

## Appendix B: how §1.3 was measured

- **Plugins.** Two throwaway `zhang-plugin-sdk` example crates, built for `wasm32-unknown-unknown` in release:
  - a processor returning its input;
  - one that also builds a `PriceMap` and sums every posting per (account, month) into a `HashMap` of `BigDecimal`.
- **Harness.** A throwaway `zhang-server` integration test, run with `cargo test --release` on the builder pod.
- **Ledgers.** Generated: 10 budgets, one `price`, transactions four a day, every fifth in USD.
- **Measured.** `LocalFileSystemDataSource::load` plus `Ledger::process_with_policy`, with a 4 GiB memory and 600 s
  ceiling.
- **Samples.** Median of 5 runs at 10,000 transactions and 3 at 50,000. Queries: median of 7 executions; the first
  builds the query engine's per-ledger cache.
- **Memory.** The 4 GiB ceiling was set to be safe. The default 256 MiB is documented to fit a processor over "some
  tens of thousands of transactions" (`zhang-core/src/plugin_policy.rs`); that was not tested here.
- **Not committed.** Neither the crates nor the test is on any branch: prototypes for this design are never
  committed.
