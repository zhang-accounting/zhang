# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.0] - Unreleased

<!--
Release prep. Before tagging:
- refresh: `git-cliff --unreleased --tag v0.3.0` lists the raw commits since v0.2.0; fold the new ones in below
  (the budget migration, #698, complete on main with #913, is in "Budget as an official plugin" and under
  Breaking changes › Budgets). Folded in up to #913;
- replace "Unreleased" above with the release date, and `main` in the [0.3.0] link at the end with `v0.3.0`;
- the workspace version in Cargo.toml must equal the tag.
-->

The first release since 0.2.0 gathers more than 350 changes. Zhang now checks a ledger the way beancount does, has its
own BQL query engine, serves Lua and WebAssembly router plugins that may write within limits the server sets, bundles
an MCP server, and lets the mobile app sign in; budgets become the official plugin `zhang:budget`. **Ledgers that
loaded cleanly on 0.2 can report new errors, plugins lose network access by default, budgets stop when plugins are
turned off, a ledger without a time zone option falls back to UTC instead of UTC+8, and several API endpoints are
gone: read [Breaking changes](#breaking-changes) and [Upgrading from 0.2](#upgrading-from-02) before you upgrade.**
The documentation has the same guide:
<https://zhang-accounting.kilerd.me/deployment/upgrading-to-0-3/>.

### Breaking changes

#### Ledger checks and booking

- A failing `balance` assertion is still reported as `AccountBalanceCheckError`, but it no longer moves the account to
  the asserted amount: balances, reports and pads follow the postings, and a `C`-flagged transaction is an ordinary
  transaction (#485, #508)
- A balance assertion, and the pad that serves it, covers the account and all its sub-accounts, as in beancount
  (#485, #508)
- In a beancount ledger a `balance` is checked at the start of its date, whatever its `time` metadata; where that
  changes the result, a `BalanceTimeIgnored` notice says so (#508)
- A posting, `pad` or `balance … with pad` on an account that is not open yet reports `AccountDoesNotExist`, and one
  after its `close` reports `AccountClosed`. A date-only `close` takes effect at the end of its day, and an `open`
  after a `close` reopens the account (#456, #689)
- Transactions are booked as a whole, then balanced per commodity at the commodity's precision: imbalances across
  commodities or prices report `UnbalancedTransaction` (#451)
- Lots are matched as in beancount: an undated cost spec reduces lots of any date, FIFO and LIFO pick by acquisition
  date, a label selects its lots (`{, "a"}`), `{{T}}` is spread over the absolute units, covering a short keeps the
  cost of the remaining lot, a `{}` augmentation infers its cost, and a `{}` reduction that no lot covers rejects the
  transaction (`NoEnoughCommodityLot`, `TransactionCannotInferTradeAmount`) (#451, #605, #637, #653, #655)
- `STRICT` booking is enforced (`AmbiguousLotMatch`). `NONE`, `AVERAGE` and `AVERAGE_ONLY` report
  `UnsupportedBookingMethod`, an invalid method reports `ParseInvalidMeta`, and the account loads with the default
  method instead of stopping the load (#448)
- A posting can carry its own flag (`! Assets:Cash -10 USD`), so in a `.zhang` file a line that starts with `*` is no
  longer a comment (#623)
- New errors an existing ledger may show: `CommodityNotAllowed` for a commodity an `open` does not list (#683),
  `IncludeNotFound` (#682), `BudgetClosed` and `BudgetCommodityMismatch` (#684), `UnusedPad` and `PadWithCost` (#508),
  `DefineDuplicatedBudget` (#842)

#### Time zone

- **A ledger without `option "timezone"` takes its time zone from `TZ` first, and falls back to UTC instead of
  UTC+8.**
  - The order is now: the IANA name in the `TZ` environment variable (`docker run -e TZ=Europe/Berlin`), then the
    system's time zone, then UTC. The fallback logs how to set the time zone. `TZ` is read once, when the server
    starts: changing it needs a restart.
  - Before, `TZ` was ignored, and the fallback was `Asia/Hong_Kong`, which every Docker install got.
  - A `TZ` that is not an IANA name, such as `UTC0`, is ignored with a warning. The option still wins.
  - An invalid option now really uses the default time zone. (#915)

#### Plugins

- The server, not the ledger, has the last word on what plugins may do (`zhang serve --plugins off|read|write` /
  `ZHANG_PLUGINS`, `ZHANG_PLUGIN_ALLOWED_HOSTS`, `ZHANG_PLUGIN_MAX_TIMEOUT`). By default plugins run read-only, with
  no network and for at most 30 seconds a call: a plugin whose directive grants `allowed_hosts` has no network until
  `ZHANG_PLUGIN_ALLOWED_HOSTS` allows its hosts, and a longer `timeout` is lowered. Each such directive gets a
  `PluginCapabilityReduced` error naming the setting; with plugins off, each gets `PluginsDisabled`. (#830)
- Plugins off turns budgets off too: budgets are interpreted by the official plugin `zhang:budget`, which every ledger
  has, and `--plugins off` / `ZHANG_PLUGINS=off` stops it with the others. The budget pages and queries then answer
  that the budget tables are unavailable, the ledger shows one `ExtensionUnavailable` error, and `zhang serve` logs it
  when it starts. Budgets need `ZHANG_PLUGINS` `read`, the default, or `write`. (#863, #867)
- Compiled plugin code is cached in Zhang's own directory, `zhang-wasmtime-cache` under `ZHANG_PLUGIN_CACHE_DIR`
  (default `$XDG_CACHE_HOME/zhang`, else `~/.cache/zhang`), instead of wasmtime's `~/.cache/wasmtime`.
  **`EXTISM_CACHE_CONFIG` and wasmtime's configuration file are no longer read.** (#868)
- A plugin's HTTP requests no longer follow redirects, no longer use the environment's HTTP proxy, and never go to a
  loopback, private, link-local, CGNAT or other non-public address, whatever `allowed_hosts` allows. (#830)
- Processors and mappers see booked postings (interpolated amounts filled in, costs resolved per lot); a `plugin`
  directive with `stage: "raw"` keeps the transactions as written. Plugins run in the order they are declared and
  see the `option` and `plugin` directives too (#427, #638)
- A plugin module outside the ledger's directory (an absolute path elsewhere, `../`, a symlink out) stops the load
  (#776), and a module must be `.wasm`, `.wat`, `.lua` or have no extension (#802)

#### Budgets

- Budgets are computed by the bundled Lua plugin `zhang:budget`, the only implementation of budgets (#698, #905):
  `zhang serve` runs it for every ledger as an analyzer, at every load and so after every write, within
  `ZHANG_PLUGIN_MAX_TIMEOUT` and `ZHANG_PLUGIN_MAX_MEMORY` (its Lua memory is the limit less 32 MiB, 224 MiB by
  default). A ledger whose budgets need more loads without its budget pages and without budget errors, with one
  `PluginAnalysisFailed` saying which limit to raise
- A build without the plugin runtime (the `zhang-wasm` playground) and a program loading ledgers with `zhang-core`
  without the bundled script and an analysis host report `ExtensionUnavailable` on a ledger with budgets, and
  interpret none (#905)
- What an operator sees of it (#905): the first start compiles the plugin runtime, about half a second, kept in the
  cache of compiled code for the next starts (mount the cache volume of a container); the server holds about 10 to
  20 MiB more with 10,000 transactions, 40 to 60 MiB with 100,000; every load logs
  `analyzer zhang:budget analysed the ledger in …` at `info`; the Settings page and `GET /api/plugins` list
  `zhang:budget` as an `Analyzer`
- Known differences from the native budgets of 0.2, measured in P4 (#905): the server holds more memory (about 10 to
  20 MiB with 10,000 transactions, 40 to 60 MiB with 100,000, within P4's bar); the first start takes about 0.7 s with
  an empty cache of compiled code and about 0.3 s with it, at 10,000 transactions; a write, and the budget or errors
  page after it, takes 35 to 50 ms longer at 10,000 transactions; and a ledger of many closed budgets loads about
  2.5 times slower (3,600 closed budgets: 1.2 s rather than 0.5 s)
- The native budget code is removed (#911, #912): the bundled script `zhang:budget` is the only implementation of
  budgets, with no fallback. Where it does not run (plugins off, the playground, an embedder without it), the budget
  tables, the `budgets` column and the `budgets.*` queries stay listed and answer that they are unavailable. The
  budget fuzzer and the parity tests compare the script with the SDK's `examples/budget` analyzer. For programs that
  embed the crates: `LoadOptions::native_budgets`, `zhang_server::analysis::{BudgetImpl, load_options_for}` (use
  `load_options`), `Ledger::{account_budgets, budget_accounts}` (use `open_meta_values_at` / `open_meta_accounts` with
  `"budget"`) and the `budget-switch` features are gone. The budgets guide, query reference, plugin development and
  upgrading pages describe the plugin alone
- Budget figures (`assigned`, `added`, `activity` and `available`, in `#budgets` and on the budget pages) keep the
  larger scale of what they add up, zeros included, as Python's decimal and beancount do. For example, a budget with
  `0.00` available that is assigned `500` reads `500.00` instead of `500`. Values are unchanged (#914)

#### Query

- `sum()` of decimals keeps the largest scale of its terms, zeros included, as Python's decimal and beancount do:
  `0.00` and `500` sum to `500.00` instead of `500`. Values are unchanged. Binary `+` and `-` don't follow this rule
  yet (see the query reference) (#914)

#### HTTP API

- `POST /api/transactions` and `PUT /api/transactions/{id}` answer `{"data": {"id", "version"}}`, read from the
  reloaded ledger, instead of `"Ok"` and `null` (#818)
- Metadata keeps the order it was written or sent in: the journal's and the written files' `metas` are no longer
  sorted by key (#821)
- Error ids are unique per error: an error at a transaction no longer has the transaction's id, it names the
  transaction in its `txn_id` meta (#833)
- Removed endpoints, replaced by the built-in queries (`POST /api/query/builtins/{name}`, see "Retired endpoints" in
  the built-in queries reference): `GET /api/store` (#606), `GET /api/budgets`, `GET /api/budgets/{name}`,
  `GET /api/budgets/{name}/interval/{year}/{month}` (#754), `GET /api/documents`,
  `GET /api/accounts/{account}/documents`, `GET /api/accounts/{account}/balances` (#756), `GET /api/errors` (#757),
  `GET /api/statistic/summary`, `GET /api/statistic/{type}` (#758), `GET /api/for-new-transaction` (#760); and
  `POST /api/sql` (#433), replaced by BQL through `POST /api/query`
- An unauthenticated `/api` request gets a JSON 401 without `WWW-Authenticate`, so a client must send its credentials
  up front instead of waiting for a challenge (#466)
- One paging rule for `/api/journals` and `/api/accounts/{account}/journals`: `page` starts at 1, `size` is 1 to
  1000 (default 100), anything else is a 400 (#713)
- An account's journal covers its sub-accounts, with a running balance of the subtree (#513); a balance assertion row
  has one shape in both journals, and its `account_after` is the checked balance (#718)
- Every error body is `{"message"}` (query errors add `line` and `column`), and an unknown `/api` path is a JSON 404
  instead of the web UI's page (#715, #755)

#### Removed

- The DuckDB SQL Explore page and `POST /api/sql`; the BQL Query page at the same `/explore` path replaces them
  (#433, #511)
- `zhang parse` (it did nothing) and `zhang export` (it panicked); `zhang serve` and `zhang update` remain (#742)
- The Python binding, which was never built nor published (#742)
- `greet()` and `ParseResult.store()` of the `zhang-wasm` package; `ParseResult.lots()` lists the lots (#603, #737,
  #751)

#### Packaging

- Docker images are published for version tags only (`latest` and `<version>`): `kilerd/zhang:snapshot` and
  `kilerd/zhang:<commit>` are no longer updated (#453)

### Upgrading from 0.2

1. Back up the ledger folder, `.zhang/passkeys.json` included once you use passkeys.
2. Plugins: if one needs the network, allow its hosts with `ZHANG_PLUGIN_ALLOWED_HOSTS`; if one needs more than 30
   seconds a call, raise `ZHANG_PLUGIN_MAX_TIMEOUT`; to let plugins write, start the server with `--plugins write`
   (`ZHANG_PLUGINS=write`). Move plugin modules into the ledger's folder and give them a `.wasm` or `.wat` name. Add
   `stage: "raw"` to a processor or mapper that expects transactions as written, and check the order of the `plugin`
   directives. Keep `ZHANG_PLUGINS` at `read` or `write` if the ledger has budgets. If you set `EXTISM_CACHE_CONFIG`,
   set `ZHANG_PLUGIN_CACHE_DIR` instead; `~/.cache/wasmtime` can go unless another program uses it.
3. Docker: switch `kilerd/zhang:snapshot` to `latest` or `0.3.0`. To keep compiled plugins across re-created
   containers, mount a volume of its own (not the ledger's) at `/root/.cache/zhang`. `docker stop` now stops Zhang
   gracefully, so a `--stop-signal SIGINT` workaround can go.
4. Start 0.3.0 and open the error list on the Overview page: work through the new errors listed above
   ([error codes](https://zhang-accounting.kilerd.me/reference/error-codes/) explains each). In `.zhang` files,
   rewrite comment lines that start with `*` with `;`.
5. Budgets: expect other figures than on 0.2, with no change to the ledger. A budget that spends in another
   commodity shows more activity and less available, as 0.2 added 300 USD as 300 CNY; a budget closed later in the
   ledger is no longer `closed` in the months before its close (see [Budgets](#budgets)).
6. Scripts and API clients: read `data.id` and `data.version` from a create or an update; move the removed endpoints
   to the built-in queries and SQL to BQL; send `page` ≥ 1; send credentials with every request; link an error to
   its transaction through `metas.txn_id`; don't expect `metas` sorted by key.
7. Set `ZHANG_SESSION_SECRET` to keep sign-ins across restarts, and `ZHANG_APP_RETURN_SCHEMES` if the app signs in
   with a scheme other than `zhang-app`.
8. A ledger without `option "timezone"` that relied on the UTC+8 fallback (the Docker image) is in UTC now. Write
   `option "timezone" "Asia/Shanghai"` (your zone) in the ledger, or pass `-e TZ=Asia/Shanghai` to the container.

### Budget as an official plugin

<!-- RFC #698 is complete on main (#913): P0, P1.1–P1.6, the G1 gate (2026-11-23), P3.1–P3.3, P4 (accepted by
the owner on 2026-11-24) and P5. What it breaks is under Breaking changes › Budgets. -->

Budgets moved out of the core into the official plugin `zhang:budget`, the bundled Lua script `budget.lua`, built on
the analyzer and extension-table API (RFC #698):

- `zhang:budget` is a default plugin: every ledger has it, without a `plugin` directive and whatever its
  `features.plugin` option says, and only the server's `ZHANG_PLUGINS=off` stops it (see
  [Breaking changes](#breaking-changes)). The query function `account_budgets()` still answers with plugins off, as
  it reads only the accounts' `budget` metadata (#863, #867)
- A `plugin "zhang:budget"` directive is allowed and loads no second copy. `zhang:budget` has no options yet: an
  argument or a meta on that directive, or a second such directive, reports a `PluginError` saying it configures
  nothing (#863, #869)
- The five `budgets.*` built-in queries are named queries of `zhang:budget`, listed where they always were and with
  the same text (#859)
- The budget behaviour is frozen as a contract suite and benchmarked (#835, #836, #851, #856), and the contract's
  known issues are fixed: a timed budget directive round-trips through a `.bean` file, budget names are exported
  quoted, a second `budget` of a name reports `DefineDuplicatedBudget`, and the budget queries' `month` must be the
  first day of a month (#842)
- The budget tables are extension tables, and the query schema names their provider (`provided_by: "zhang:budget"`)
  (#843, #844, #845)
- The `Analyzer` plugin type, in the SDK and in Lua; the SDK's `examples/budget` computes zhang's budgets as an
  analyzer and passes the contract (#848, #849, #850, #864)
- The bundled script `plugins/budget/budget.lua` computes the budgets as the native code does: `#budget_definitions`,
  `#budget_events`, `#budgets`, `postings.budgets`, the five `budgets.*` named queries and every budget error, with
  the same ids, places and order. It passes #835's contract byte for byte and the `budget_reference` oracle, and a
  differential fuzzer that compares it with the native code (daylight-saving changes, decimals of up to 28 places and
  budget boundary days included) finds no difference on 10,000 generated ledgers (#886, #887, #895)
- Its activity queries are a few per budget commodity, whatever the number of budgets, closed ones included, so a
  ledger with thousands of closed budgets stays within an analysis's 10,000 queries, and a budget's own commodity is
  summed exactly, without `convert()`'s 28-digit rounding (#891, #898)
- A budget of an account whose opens name many budgets, such as a budget per category and month, is read from the
  account's months: 4,200 monthly budgets take 6 queries instead of 48 and load in about 0.57 s rather than 6.3 s,
  with 48 to 60 MiB of Lua memory, and the months of closed budgets are classed from the first aggregation (#908,
  #909)
- P4 measured the script against the native budget code, paired, at 10,000 and 100,000 transactions (#894, #905,
  #910): every memory figure and all 12 named-query p95s pass, as do the concurrent readers, the Docker smoke test
  under `--memory 512m` and the upgrade canary from 0.2.0; 9 of the 12 end-to-end timers pass, and three of the errors
  page stay inconclusive, which the owner accepted on 2026-11-24 (`reload → errors` +38.5 ms and `start → errors`
  +41.8 ms against 50 ms at 10,000 transactions, `reload → errors` +58.0 ms against 66 ms at 100,000). Ledgers heavy
  in closed budgets load about 2.5 times slower (3,600 closed budgets: 1.2 s rather than 0.5 s); monthly ones 10 to
  20% slower (4,200 budgets: 0.53 s rather than 0.47 s)
- The native budget code is gone (#911, #912, #913); see [Breaking changes](#breaking-changes)

### Plugins

- Router plugins: a plugin is served at `/api/plugins/{name}` and every path below it, for any method, behind
  sign-in. It gets the request as JSON and answers `{status, headers, body, body_encoding}`; the Settings page links
  to each router (#468)
- Lua plugins: `plugin "report.lua"` runs the script in an embedded Lua 5.4, with a `zhang` table (queries, built-in
  queries, schema, ledger info, clock, config, errors, writes, conversions) and a `json` codec. Scripts have no `io`,
  `os`, `require` nor network, and are routers or analyzers (#802, #838, #864)
- `zhang.decimal` gives Lua scripts exact decimals: add, subtract and compare values, strings and integers (a sum
  keeps the larger scale of the two); `*`, `/`, floats and texts of more than 1000 digits raise an error. A decimal
  goes into JSON as its text, so into a cell, an amount, a query parameter or a write (#864)
- `zhang.convert` (Lua) converts a batch of amounts with the ledger's prices, as `zhang_convert` does (#864)
- Plugin writes: a router plugin may append `transaction` and `open` entries through `zhang_write_preview` and
  `zhang_write_commit` when the plugin declares `writes`, its directive grants `allowed_writes`, and the server runs
  with `--plugins write`; at most 50 directives and 64 KiB a commit, through a preview token that a change of the
  ledger voids (#803, #826, #847)
- Two router plugins of one name: the first declared serves the route, and the other gets a `PluginError` naming both
  modules (#803)
- Host functions `zhang_query_params` and `zhang_query_schema` (#798), `zhang_query_builtin` (a built-in query by
  name, #804) and `zhang_convert` (exact conversions with the ledger's prices, #849)
- `allowed_paths` gives processors and mappers read-only access to ledger files (`zhang_read_file`,
  `zhang_list_dir`), and a local ledger reloads when such a file changes (#461, #477)
- `zhang_now` gives every plugin of a load the same clock in the ledger's timezone and `zhang.seed` a stable seed per
  directive; a ledger whose plugin read the clock reloads at local midnight (#461, #469)
- `zhang_emit_error` lets a plugin report `PluginError` entries into the error list (#463)
- A `timeout` meta (`"90"`, `"500ms"`, `"2m"`) bounds every call into a plugin, within the server's ceiling (#465,
  #830)
- Plugin arguments and multi-valued meta reach the plugin as config (`zhang.abi`, `zhang.plugin`), with the effective
  grants `zhang.allowed_hosts`, `zhang.timeout_ms`, `zhang.allowed_writes` and `zhang.max_memory` (#462, #803, #830,
  #901)
- Unknown plugin types are ignored with a warning, `option "features.plugins"` works as an alias, a missing module
  stops the load with `plugin module not found: <path>`, and a plugin makes no HTTP request while it registers (#460,
  #682, #833)
- `GET /api/plugins` adds the effective `capabilities.allowed_hosts`, the `route`, `writes_requested` and
  `writes_granted` (#468, #803, #830)
- Compiled code is kept in a cache of Zhang's own, `zhang-wasmtime-cache` under `ZHANG_PLUGIN_CACHE_DIR` (default
  `$XDG_CACHE_HOME/zhang`, else `~/.cache/zhang`; set it to nothing to turn the cache off), about 128 MiB at most, so
  a start reads plugins back instead of compiling them. `zhang serve` logs where the cache is, or why there is none
  (#868)

#### Security and limits

- Sandbox every router plugin response (`Content-Security-Policy: sandbox allow-scripts allow-forms allow-popups
  allow-downloads`, `X-Content-Type-Options: nosniff`) and send only an allowlist of its headers (`content-type`,
  `cache-control`, `etag`, `last-modified`, `content-disposition`, `content-language`, `vary`, `allow`, and a
  `location` on Zhang's own origin), with a `cache-control` that is always `private`. A plugin's page could run
  scripts with the owner's session and set cookies on Zhang's origin. (#827)
- Forward only an allowlist of request headers to router plugins, so proxy identity headers (`cf-access-jwt-assertion`,
  `x-auth-request-*`, `tailscale-user-*`, `x-forwarded-*`) no longer reach them. (#827)
- Limit router plugin requests: a body of 4 MiB (`ZHANG_PLUGIN_MAX_BODY`, 413), 4 requests at once per plugin
  (`ZHANG_PLUGIN_MAX_CONCURRENCY`; up to 16 more wait up to 5 seconds, then 503 with `Retry-After`) and the plugin's
  `timeout` per request, at most `ZHANG_PLUGIN_MAX_TIMEOUT` (30 seconds by default; 504 past it). (#827, #830)
- Limit the memory of every plugin instance to 256 MiB (`ZHANG_PLUGIN_MAX_MEMORY`). An invalid `ZHANG_PLUGIN_*` value
  stops `zhang serve` with an error naming it. (#827)
- Escape control characters in the plugin text Zhang logs, so a plugin cannot forge log lines. (#827)
- Patch the vendored extism with a bounds check on every read of plugin memory: a plugin could forge the length of a
  memory block and crash the server (SIGSEGV/SIGBUS) through extism's own host functions or the output and error of
  any call. The patched extism also counts the memory a plugin starts with toward `ZHANG_PLUGIN_MAX_MEMORY`, which it
  left out, so a plugin could use about twice the limit. (#830)
- Resolve a plugin's HTTP request once, refuse non-public addresses and connect to the checked address, so DNS
  rebinding and redirects cannot reach the machine, the private network or the cloud metadata service. (#830)
- Update wasmtime from 43 to 49.0.7 by vendoring extism 1.41.0, with zhang's patches carried over: on aarch64 a plugin
  with a `br_table` of more than ~256k targets could make Cranelift branch to the wrong code and escape the sandbox
  (GHSA-mghf-wjfp-p5xm). Plugins now get WASI from `wasmtime-wasi` instead of the removed `wasi-common`. (#852)
- Trust the compiled-code cache only where no one else can change it: on Linux and macOS its directory and
  `zhang-wasmtime-cache` must belong to the user running Zhang and be writable by that user only, the subdirectory
  must not be a symbolic link, and a directory in or around the served ledger's folder is refused. Wasmtime's clean-up
  only ever sees `zhang-wasmtime-cache`. A refused directory, or one that cannot be created or written to, turns the
  cache off with one warning, and plugins still load. (#868)
- A Lua script's memory follows `ZHANG_PLUGIN_MAX_MEMORY`: the instance's limit less 32 MiB for the host (a quarter of
  a limit under 128 MiB), so 224 MiB by default. The cap is enforced where the interpreter allocates, so a script runs
  out of Lua memory, with an error naming the limit, before its instance traps; an analysis that fails so has the
  reason `memory`, which the interpreter's exit code tells and a script cannot fake (#901)
- A module's start function, a reactor's `_initialize` and a WASI module's constructors run within the plugin's
  `timeout`: a module whose initialisation never ends fails the load at its timeout, where it hung that load and every
  reload after it (#903)
- A module's initialisation counts toward `ZHANG_PLUGIN_MAX_MEMORY` as a call does: its initial memory, Extism's
  kernel and its tables (8 bytes an element) count from the start, so a start function or a `table.grow` can no longer
  take memory past the limit (a table took 1 GiB under a 32 MiB limit). The main module is instantiated once per
  instance instead of twice, and one that imports the kernel's memory is refused (#904)
- What a plugin instance keeps in the host is bounded: at most 1000 reported errors, 1 MiB in all, per run (one more
  `PluginError` says how many were dropped), and at most 10,000 recorded file inputs and 1 MiB of their paths (past
  that, each `allowed_paths` grant counts as a directory) (#906)
- Editing a transaction a plugin generated, or uploading a document to it, is a 400 instead of a panic or a write
  into the main file (#664)

### MCP server (`zhang:mcp`)

- `plugin "zhang:mcp"` turns on the bundled MCP server at `POST /api/plugins/zhang-mcp/mcp` (MCP 2025-03-26 to
  2025-11-25 and the stateless 2026-07-28 revision), with 12 read tools: ledger_info, schema, query, accounts,
  balances, net_worth, spending, journal, transactions, payees, errors and prices (#804, #805)
- The write tools `preview_transaction`, `add_transaction` and `open_account` are listed only when the plugin's
  directive grants `allowed_writes` and the server runs with `--plugins write`; their entries are tagged `#mcp`
  (#804)
- Settings: `allowed_origins` (loopback only by default), `max_rows`, `write_tag` and `timeout` (#804)
- A `zhang:<name>` module loads a plugin built into the binary; each release also attaches `zhang-mcp-<version>.lua`
  for running your own copy (#805)
- The plugin reports the Zhang release it ships with as its version (0.3.0), in `GET /api/plugins` and the MCP
  `serverInfo`

### Plugin SDK

- New Rust crate `zhang-plugin-sdk` (a git dependency, not on crates.io): the `plugin!` macro and the `config`,
  `custom`, `clock`, `fs`, `errors` and `router` modules, with the guard and summary examples and the Writing Plugins
  guide (#480)
- `prices::PriceMap` with zhang-query's conversion rules (#483), `SparseRealization` for booked balances of chosen
  accounts and the `balances` and `lots` examples (#638, #651), `router::query_with_params`, `router::schema` and
  `router::query_builtin` (#798, #804)

### Analyzers and extension tables

The `Analyzer` plugin type is stable from 0.3.0: `analysis_abi` 1 (the manifest, the records, the output,
`zhang_convert`, the SDK's `analysis` module, Lua analyzers) only grows by additions, such as a field a reader ignores
or another optional form, and any other change brings a new `analysis_abi` version. An analyzer that uses an addition
needs the release that brought it.

- New `Analyzer` plugin type (a WebAssembly module that exports `manifest` and `analyze`, or a Lua script): it runs on
  every load after booking and the checks, publishes diagnostics (`PluginError` with a `kind`) and tables that queries
  read as `#<ns>_<table>` and `postings.<ns>_<column>`. An analyzer reads no files, has no network, runs at most
  10,000 queries, and a failing one leaves the ledger loaded with one `PluginAnalysisFailed` error. The analyses of a
  load share `ZHANG_PLUGIN_MAX_TIMEOUT`: each runs at most its own timeout and what the analyses before it left (#848,
  #849, #855, #866)
- The SDK writes analyzers (`plugin!` with `analyzer:` and `manifest:`, `analysis::convert`) (#850)
- Lua analyzers: a script with `types = { "Analyzer" }` (alone or with `"Router"`) declares its manifest as fields of
  its module table and returns its output from `analyze`; while it analyses it may run `zhang.query`,
  `zhang.query_params`, `zhang.query_schema`, `zhang.ledger_info`, `zhang.now` and `zhang.convert`. A raised error
  fails that analysis only, with the script's line in its `PluginAnalysisFailed` (#864)
- Named queries: an analyzer's manifest may declare `queries`, BQL with typed parameters, named after its namespace
  (`shop.item`). They are checked when the ledger's catalog is built, then listed and run like the built-in queries
  (`GET /api/query/builtins`, `POST /api/query/builtins/{name}`, a router's `zhang_query_builtin`). The namespaces of
  zhang's own built-in queries are reserved: an analyzer that declares a query in one is reported on its directive
  (#859, #869)
- An analyzer's output is at most 32 MiB of JSON and 1,000,000 values, and a decimal in it (an amount's number too)
  has at most 1000 digits written out in full, before and after its point together, the bound of `zhang_convert`:
  `1e1000` is refused (#849, #876)
- Every ledger has its own query catalog, and `GET /api/query/schema` describes the served ledger's tables, with
  `provided_by` on the tables an extension provides (#839, #845, #848)
- An analyzer's output may list a record table's rows as their cells in column order, write an amount as
  `["12.50", "CNY"]`, and leave out of a series fact the cells it carries over; these shorter forms need
  zhang ≥ 0.3.0. **A series fact without `$meta` now takes the previous fact's metadata** (`"$meta": {}` for none).
  Its input names the ledger's last transaction (`last_transaction`), and a record at a time a daylight-saving change
  skips has its `written_time`. The SDK's `Output::tables` rows are `serde_json::Value`s, a breaking change for SDK
  analyzers (#885)

### Query language

- A native BQL engine (beanquery-compatible, exact decimals) replaces the SQL Explore page: `SELECT [DISTINCT] … FROM
  … WHERE … GROUP BY … HAVING … PIVOT BY … ORDER BY … LIMIT … OFFSET`, `BALANCES` and `JOURNAL`, accounting periods
  (`FROM … OPEN ON … CLOSE ON … CLEAR`) (#439, #450, #467, #484)
- Tables: `#entries`, `#transactions`, `#prices`, `#balances`, `#notes`, `#events`, `#documents`, `#accounts`,
  `#commodities`, and zhang's `#budgets`, `#budget_events`, `#budget_definitions` and `#errors` (#467, #484, #503,
  #748)
- Functions: beanquery's date functions, `open_meta`, `commodity_meta`, `open_date`, `close_date`, `meta_values`;
  zhang's `icontains`, `any_icontains`, `intersects`, `under`, `set()`, `CASE WHEN … END`, `least`/`greatest`,
  `account_status`, `now()`, `open_meta_values(account, key, date[, time])` (`account_budgets()` is a deprecated
  alias) and `open_meta_has(account, key, value, date[, time])`, a cheap `value IN open_meta_values(…)` (#484, #490,
  #503, #504, #689, #759, #837, #862)
- `unnest(set)` groups by each element of a set, such as `SELECT unnest(tags) AS tag, count(*) GROUP BY tag`: a row
  counts in the group of each element, and in none when the set is empty or NULL. It is allowed once per query, as a
  whole target or `GROUP BY` item (#873)
- The Query page runs on Run or Ctrl/Cmd+Enter, with examples, a schema and function reference, saved queries (the
  `query` directive), CSV export and charts of the results; "Open query" buttons across the app open the query
  behind a figure; the editor highlights the keywords the server's parser reads, and charts format numbers in the
  app's language (#439, #450, #467, #490, #552, #732, #743, #762)
- `POST /api/query` answers `{columns, rows}` (`total` with `count_total`); a query is bounded to 64 KiB, 10 seconds
  and 1,000,000 result values (`ZHANG_QUERY_MAX_RESULT_VALUES`) (#439, #450)
- `seq` numbers the entries in the order zhang processes the ledger (#502)
- Fixes: an aggregate over no rows returns one row (#656), non-ASCII currencies sort consistently (#680), a document
  of a split posting is listed once (#686), text is read as a date by `date()`'s rule (#705), `open.booking` shows
  the method zhang books with (#708), the directive tables agree with the postings (#706, #709)

### HTTP API

- `DELETE /api/transactions/{id}` removes a transaction; it requires `If-Match: <version>` (428 without it, 412 when
  the version is stale). Transactions carry a `version` in the journals, and `PUT` and its preview take an optional
  `If-Match` (#818)
- Every write answers once the ledger has been reloaded, so the next read sees it (#818)
- `POST /api/transactions` takes an `Idempotency-Key` header: the transaction is written with the key as its
  `idempotency-key` metadata, and a create sent again with the key writes nothing and answers the transaction that
  has it, with `Idempotency-Replayed: true`, after a reload or a restart of the server too (#831)
- Posting flags round-trip (`flag` in requests, `written.flag` in the journal) (#821), a request `datetime` may be a
  date alone, which writes no time (`has_time: false`) (#821, #726), and an edit keeps a posting's cost, price and
  comment (#671)
- `POST /api/transactions/preview` and `POST /api/transactions/{id}/preview` answer the text and the checks of a
  request without writing (#710)
- Built-in queries over HTTP: `GET /api/query/builtins`, `POST /api/query/builtins/{name}`, a batch at
  `POST /api/query/builtins` and the filled-in text at `POST /api/query/builtins/{name}/text`. The budget queries'
  `month` must be the first day of a month (400 otherwise) (#490, #503, #753, #842)
- The file editor's `GET /api/files/{path}` answers a `sha256`, and a save with `expected_sha256` over a file changed
  since is a 409 (#667)
- Numbers are written in plain notation (`0.0000001`, not `1E-7`) (#695); a write refuses numbers of more than 1000
  digits and says why an amount expression has no value (#782, #820)
- Fixes: base64 paths that contain a slash (#507), a commodity's latest price (#666), a remote ledger's documents when
  their cache cannot be written (#678), a document's extension and MIME type (#690)

### Authentication

- A login page replaces the browser's Basic-auth popup, with passkey sign-in (`--passkey` / `ZHANG_PASSKEY`, stored
  in `.zhang/passkeys.json`). Sessions last 30 days; `ZHANG_SESSION_SECRET` keeps them across restarts. The Basic
  header still works for scripts, and failed sign-ins are rate-limited (429 with `Retry-After`) (#466)
- App sign-in: a signed-in browser hands a single-use code (60 seconds) to the app, which exchanges it for a bearer
  session (`POST /api/auth/app/code`, `POST /api/auth/app/exchange`, `Authorization: Bearer`) (#786)
- The handoff takes PKCE (S256) and a `state`, and the login page asks for consent ("Sign in to the app as …?") before
  it issues a code (#819)
- `ZHANG_APP_RETURN_SCHEMES` ignores, with a warning, `http`, `https` and the other web schemes (#819)
- The web UI's pages are sent with `X-Frame-Options: DENY` and `Content-Security-Policy: frame-ancestors 'none'`
  (#819)

### Ledger processing

- `pad` is supported in both formats, across files, with `UnusedPad` and `PadWithCost` notices (#438, #508)
- Metadata under a posting belongs to the posting (`metas` on journal postings and requests, `meta()`, `any_meta()`)
  (#471)
- A day's entries have one order for checks, booking and queries (#438, #717); valuations use the latest price on or
  before the date (#459); `operating_currency` takes the precision options in any order (#644, #648)
- Every beancount cost form parses (`{*}` reports `CostMergingNotSupported`), and lot costs are divided in
  beancount's 28-digit context (#681, #687)
- Cases that panicked are reported instead: an undefined account budget, daylight-saving gaps and overlaps, a division
  by zero in an amount, a number out of range (#454, #455, #650, #716, #781)
- Errors give their location as a line and a column (#669)
- Every ledger file is decoded strictly as UTF-8, and a leading byte order mark is skipped and kept (#663, #688); the
  ledger's format follows the main file's extension (`UnknownLedgerFormat` otherwise) (#697)

### Beancount compatibility and export

- Budgets in `.bean` files are written as `custom "budget" …` entries that bean-check accepts; the old form is still
  read (#668)
- Export quotes what beancount would not read bare: metadata values such as `budget: "Food"`, arithmetic, `#旅行` or
  `1,5`, and metadata keys that are not bare words; strings escape only `"`, `\` and control characters (#457, #842)
- Metadata values that beancount writes unquoted (accounts, amounts, grouped numbers) are read and round-trip (#659)
- A directive dated with a time, budget directives included, is exported as its date plus `time` metadata and read
  back at that time (#767, #842)
- A beancount `document` path is relative to its file (`DocumentPathRelativeToRoot`, `DocumentNotFound`) (#508)
- Writes refuse names that would not read back, by beancount's rules on a beancount ledger (#457)
- Known limitation: tags and links are written bare, as the ledger has them. Beancount reads only ASCII tags and links
  (letters, digits, `-`, `_`, `/`, `.`), so a ledger whose transactions carry a non-ASCII tag such as `#旅行` gives
  output beancount does not read. A write to a beancount ledger refuses a new such tag, but keeps one the ledger
  already uses

### Data sources and writes

- Local saves are atomic: a temporary file is renamed over the file, a hard-linked file is written in place, and a
  file Zhang may not write (read-only, say) is a 403 `the storage refused to write <file>` (#824)
- An append writes each file once, the main file's `include` first, after reading every file; a failed write takes
  the include out again (#825, #832)
- New entries go to `data/{{year}}/{{month_str}}.{{ext}}` by default, so a beancount ledger gets `.bean` files
  (`{{ext}}` placeholder) (#478, #571)
- An include that matches nothing or points outside the ledger is reported as `IncludeNotFound` and the rest loads;
  include patterns are anchored and `*` stays within one path part (#649, #665, #682, #691)
- On a beancount ledger, a balance check writes a `balance` dated the next day, a reconcile also writes the padding
  transaction, and `/api/info` reports the `format` (#508)
- The server counts the content changes of the served ledger (a generation), which voids plugin write previews made
  before a change (#826, #847)

### Budgets

- Budget figures change on the same ledger (#503): activity in another commodity is converted to the budget's
  commodity at its date, where 0.2 added the numbers whatever their commodity (300 USD spent from a CNY budget counted
  as 300 CNY, now as 2,100 CNY at 7.0); `closed` is per month, from the month of the first `budget-close` on, where
  0.2 showed a budget with a `budget-close` as closed in every month; and a posting counts toward the budgets of its
  account's `open` in effect at its date, so a reopened account no longer counts its whole history in both budgets
- Postings after a budget's `budget-close` no longer count (`BudgetClosed`), and an amount no price converts reports
  `BudgetCommodityMismatch` (#684)
- `postings.budgets` lists the budgets a posting counts toward (#714); see also "Budget as an official plugin"

### Server, reloads and CLI

- A failed reload is shown, not only logged: `reload_failure` in `/api/info`, a `ReloadFailed` event, and a 409 from
  `POST /api/reload`; a write to a stale ledger reloads it first and answers 409 if it cannot (#672, #719)
- The watcher reloads a ledger served through a relative or symlinked root (#670), when a plugin module or a missing
  include appears, and at midnight for a ledger that depends on the date (#461); it ignores reads and waits for 500 ms
  of quiet (2 seconds at most), and reloads the whole ledger when its events overflow (#840, #846)
- A slow event-stream client no longer stalls the server; one that would miss a reload is disconnected and fetches
  again when it reconnects (#840, #846)
- The file watcher no longer panics when it cannot be created or loses its watch (the user's inotify instances or
  watches used up, a folder that vanished during the walk), which left edits made outside the server unloaded. It logs
  once why and which setting to raise (`fs.inotify.max_user_instances`, `fs.inotify.max_user_watches`), checks the
  ledger's files every 5 s meanwhile, and tries to watch again with a backoff from 1 s up to 30 s; `GET /api/info` has
  a new `watch_failure` with the reason (#916)
- `zhang serve` and `zhang update` print `error: <reason>` and exit with 1 when they fail (#488, #660); the log
  filter is `ZHANG_LOG`, then `RUST_LOG`, then `info` (#661); `zhang --version` prints the release version (#662)
- `zhang serve` stops gracefully on SIGTERM, the signal Docker, Podman, Kubernetes and systemd send, as on SIGINT: it
  takes no new connection, lets the requests in progress finish for up to 5 s (a write is never cut off halfway), ends
  the event streams and exits with 0, within 7 s in all. A second signal exits at once (130 for SIGINT, 143 for
  SIGTERM). `docker stop` no longer waits its 10 s and kills the container (#892, #896)
- At `info` the log no longer carries the internals of dependencies: `wasmtime_wasi`, `gotcha` and `tracing::span`
  show warnings and errors only, unless the filter names them (`info,wasmtime_wasi=trace`). `ZHANG_LOG` applies to
  every line, and lines are coloured only when the output is a terminal (`RUST_LOG_STYLE=always` or `never` overrides
  it) (#893)

### Web UI

- Rebuilt on React 19, Tailwind 4 and shadcn/Base UI: a sidebar with the accounts and their balances, a bottom tab
  bar on phones, light and dark themes, an English/Chinese switch, an overview with the month's budgets (#449)
- The transaction form edits a posting's cost, price and comment, and offers only the accounts open at its date
  (#673, #693)
- The error dialog opens the editor at the error's line (#675); every open page refreshes when the ledger changes or
  the event stream reconnects (#745, #761)
- Amounts and totals follow the server's figures and the commodity's precision (#704, #711, #721); a commodity's
  price chart has one scale per quote currency (#764); Settings links to the GitHub repository (#657)
- The bundle is split by page and by heavy library, so a first load fetches the shell and one page (#768)

### Performance

- A reload loads the ledger outside the write lock: requests keep being answered from the current ledger until the
  new one is swapped in (#840), and a write costs one load instead of two (#841). In the budget benchmark at 100,000
  transactions, a reader during a reload waited 2.4 ms instead of 2.4 s, and a write took 1.8 s instead of 2.2 s
- The budget tables are computed once per loaded ledger: a warm `budgets.month` query takes 3.8 ms instead of 72.5 ms
  at 100,000 transactions, while the first budget query after each load builds them (283 to 529 ms) (#844)
- Plugins are compiled once per load instead of on every call (#801), and the Lua runtime's JSON codec is C instead
  of dkjson: decoding 1 MB takes 10 ms instead of 111 ms, and the MCP `query` tool answers in a third of the time
  (#838)
- Loading no longer slows down quadratically with the number of postings and metadata entries (#565, #629, #639,
  #641, #650)
- `zhang` allocates with mimalloc (the default feature `mimalloc`; `--no-default-features` keeps the system
  allocator). In the budget benchmark at 100,000 transactions, against the system allocator: the first request after
  a reload is as fast as in a fresh process (median 156 ms instead of 389 ms), loads and writes take about 25% less
  time, and the memory freed after a reload goes back to the system: the peak is 1.4 GiB instead of 4.4–5.4 GiB, and
  the server holds about 510 MiB instead of 4.1–5.0 GiB after a minute idle. A small ledger holds a little more right
  after loading (125 MiB instead of 100 MiB at 10,000 transactions) (#858)
- The first query of a loaded ledger builds the cache of its booked postings with far fewer allocations, and on up to
  4 threads for a ledger of 32,768 transactions or more: 22–25 ms instead of 82 ms at 100,000 transactions (#871,
  #874). Aggregations no longer allocate per row (30 per-budget aggregations: 62–64 ms instead of 79–85 ms), `IN`
  hashes its items with a randomly keyed foldhash, and plain decimals are read without the general parser (#872,
  #875, #879)
- A default plugin is compiled once, and what it declares when it registers is read once, for as long as its module
  and settings stay the same, so a reload or a write does neither again: about 7 ms less for every reload or write of
  a ledger with a Lua default plugin, as the bundled `zhang:budget` is (#863, #884). An analyzer's
  output is read straight into the cells of its tables, without a JSON tree, its input records are freed before the
  call, and the memory a compile freed is given back (#876, #877, #878)
- A plugin compiles on one thread of its own and gives the memory back after: a cold compile of a Lua plugin takes
  about 60 MiB on any machine, where it took about 10 MiB per CPU and kept it (906 MiB on a host showing 96 CPUs, so a
  container limited to 512 MiB was OOM-killed at its first start). A first start with an empty cache compiles the Lua
  interpreter in about 0.4 s instead of 0.06 to 0.12 s (#897)
- A ledger's own plugins reuse their module's compiled code across loads, as default plugins do, so a write no longer
  compiles them again: without a disk cache, a write to a ledger with its own Lua plugin takes about 27 ms instead of
  compiling the Lua interpreter again (0.4 s). Each module keeps one compiled copy idle, a ledger's own module only
  while a loaded ledger runs it, and the server's own modules at most 3 spares between them (#900, #902)
- Converting through the opposite pair of a price inverts the pair's rate once, the first time it is read, instead of
  at every lookup: the budget activity pass is about 20% faster (#883)
- The postings cache of a large ledger is kept in the parts that built it instead of copied into one: a load with the
  budget script holds about 17 MiB less after loading 100,000 transactions (#907)

### Deployment and packaging

- Release binaries of `main` are built once a day and on demand, as workflow artifacts (#785)
- A free Render demo deploys from a read-only S3 bucket (`render.yaml`, `deploy/render/`), and the README and the
  documentation link an online demo with a tour (#652, #654)
- The documentation is reorganized (getting started, guides, deployment, reference, developers) and translated to
  Chinese, with redirects from the old pages (#489)
- The server and CLI crates share the workspace version, so a source build reports 0.3.0
- Each release also has binaries for Windows (x86-64, a `.exe`) and FreeBSD (x86-64), built by the same workflow but
  untested on real machines and community-supported; reports are welcome
- The Docker image's `org.opencontainers.image.source` label points at zhang-accounting/zhang, and the documentation's
  Compose example mounts a volume for the plugin cache (#888, #889)
- Testing and CI: `cargo build --locked`, as the release workflow runs it, works on every commit of `main`, as CI
  fails a pull request whose `Cargo.lock` is out of date (#860, #881); the workflows' token is read-only except in the
  release jobs (#899); a nightly smoke test runs the Docker image under 512 MiB and checks that `docker stop` ends it
  within 5 s (#890); the budget script is fuzzed against the native code in every test run and on 10,000 seeds every
  night (#887, #895); and P4's benchmark runs its pairs side by side (#894)

### Dependencies

- extism 1.41.0 (wasmtime 49.0.7) and patched transitive crates and web packages clear the reported advisories
  (#561, #562, #573, #578, #597, #852, #853, #854); openapi-typescript 7 for the frontend's generated API client
  (#865); reqwest 0.13 (#625); self_update 1.3 (#609)
- Building the web UI from source needs Node 22.22 or newer (Vite 8, TypeScript 6, ESLint 10, i18next 26, React Router
  8) (#568, #576, #579, #583, #588)

The other commits since 0.2.0 are refactors, tests, CI and dependency updates without a visible effect;
`git log v0.2.0..v0.3.0` lists them.

## [0.2.0] - 2026-10-02

### Added

- Add tracing log in directive process trait for better debugging
- Add help text for healthy ledger
- Add typos checker config file
- Add typo checker in develop build
- Add some padding for setting page
- Add some missing i18n
- Add i18n extraction server and some i18n
- Support random tags and links for transactions
- Add testcase for ledger syntax
- Support inconsistent space and optional space for posting
- Add pagination i18n
- Add OpenAPI documentation section in settings
- Add wildcard include directive support and regex dependency
- Add "no results" message to Explore page
- Add s3 data source (#428)

### Changed

- Bump to v0.1.10
- #357 introduce jotai and refactor error fetcher and pagination
- #357 use jotai for all reduce data
- Bump ws from 7.5.9 to 7.5.10 in /frontend
- Bump rustls from 0.22.3 to 0.22.4 (#302)
- Replace wee_alloc with talc (#359)
- Use cfg_if to control feature related code (#361)
- Bump openssl from 0.10.64 to 0.10.66
- Report graph use right Y axis to display income & expense data
- Journal list can be filter by tags and links
- Display tags and link and can be filter by them
- Make fmt happy
- Fmt code
- Use vite as packager
- Restore prettier command
- Integrate with shadcn/ui
- New sidebar and dashboard
- New composed chart for report
- Refactor the accounts page
- Refactor commodity page
- Refactor single commodity page
- Doucment page
- Budget page
- Single budget page
- Report page
- New raw edit page
- Setting page
- New notification
- New transaction model
- Preview model and edit model
- New account line
- Budget line
- Journal table line
- Journal preview model
- Account balance graph
- Dropzone
- Prettier code
- Log scale for graph
- Make error count red
- Grid for homepage
- Elide the lifetimes
- Use stable rust toolchain for clippy
- Porting the change to beancount extension
- Allow integration test to load beancount file
- Use match expr instead of if expr
- Lock python version
- Make prettier happy
- Make prettier happy
- Integrate Gotcha framework and enhance server architecture
- Add detailed documentation for response entities
- Simplify ServerApp configuration and state management
- Integrate OpenAPI TypeScript for type-safe API interactions
- Migrate frontend to use new API types and fetchers
- Update Gotcha framework import and trait name
- Organize imports and clean up code structure
- Simplify Account and Directive creation in route handlers
- Add PWA support and mobile-friendly improvements
- Add responsive drawer and auto-drawer components
- Enhance AutoDrawer component with improved child handling
- Update API request structures and enhance mobile view components
- Make prettier happy
- Rename response structures for consistency and clarity
- Update documentation for directives and enhance structure
- Integrate minijinja for directive output path customization
- Clean up example
- Make fmt happy
- Make prettier happy
- Deploy docs to cf woker
- Update deploy command
- Update CI workflows to use Ubuntu 22.04
- Implement SQL execution feature
- Migrate zhang & beancount text parsers from pest to nom (#414)
- Beancount balance tolerance, txn keyword, cost labels, total cost, pushmeta/popmeta (#416)
- Dedup domain types, unify commodity, decouple ast (+ frontend) (#418)
- Make Windows best-effort so it no longer blocks CI (#417)
- Per-plugin allowed_hosts + config, fix router/panic/perf issues (#420)
- Drop windows test leg, fix clippy on rust 1.99 and fix rust caching (#429)
- Skip release builds and macos tests on pull requests (#430)

### Fixed

- Mul the unit sign to posting's total price
- Journal api should return correct total count given filtered keyword
- Fix all the typo
- No translate for breadcrumb
- Fix ts compile error
- Update frontend dist folder after switching to vite compiler
- Update frontend dist folder after switching to vite compiler
- Use full name of pnpm in git hooks
- Fix the typo of folder
- Display commodities in col
- Correct indent of account tiers
- Fix the sidebar and scroll the content only
- Page click event in pagination
- Update online playground link in README
- Standardize API schema and response structures
- Fix docs ci
- Clippy redundant-reference warnings in python binding (#415)
- Prettier-format generated schemas.ts + auto-format in `api` script (#419)
- Write plugin output back to directives, dedup process paths, reuse module bytes (#426)

### Removed

- Remove self implemented bigdecimal scale round
- Remove useless console log
- Remove redux dependencies
- Remove useless dependency
- Remove done todo
- Remove useless deps
- Remove needs condition for formatter and clippy to speed up the CI
- Remove gap in trx line
- Remove dot and only active dot on hover
- Remove unused imports and simplify route modules
- Remove global.ts and axios dependencies
- Remove example notify

## [0.1.10] - 2024-06-19

### Added

- Add umami trace script (#344)
- Add span info in process error
- Add rust_log env in docker

### Changed

- We support multiple leading zero in amount
- Make fmt happy
- Bump to v0.1.10

### Fixed

- Open directive should have higher priority then balance directive
- Pad info should be removed after being used
- Use real temp dir
- Get the correct unit when processing txn

## [0.1.9] - 2024-06-14

### Added

- Add discord and testflight link in readme.md (#336)
- Add anchor link in error box to navigate to document
- Add some error codes' description

### Changed

- Resolved #265 translate all documents into english
- Upgrade opendal to support non-standard webdav server

### Fixed

- Fix the testflight icon
- Fix the testflight icon (#337)

### Removed

- Remove TransactionDoesNotBalance in locale file
- Remove useless struct

## [0.1.8] - 2024-06-04

### Added

- Add some log for cache logic
- Add custom error
- Add commodity doc
- Add breakpoint for commodity grid for better display on mobile
- Add some skeletons
- Add single account balance history graph
- Add async process method
- Support implicit posting cost
- Add validation rule for fava data
- Add default booking method options

### Changed

- Resolved #316 sort account in txn edit modal
- Resolved #314 close txn modal after submitting
- Make clippy happy
- Make prettier happy
- Return group in commodity api
- Show commodities in groups
- Show page title in html title
- Group hooks
- Use segment control for document list layout switcher
- Make prettier happy
- Upgrade to mantine 7
- Auto complete does not need creatable
- Extract router module
- New chart
- New logic of sequencedDate
- Improve accessibility
- Split once
- Move typescript pkg into dev dep
- Make prettier happy
- Update the account balance graph first commodity color to primary color
- Enhance the render logic
- Publish the docker image with tag
- Make clippy happy
- Using zhang custom error instead of unwrap the error
- Make clippy happy
- Use lightbox to preview the document image
- Only image can be preview
- Implement a new commodity lot record logic
- Handle single cost scenario
- Introduce new error kind for no enough lot record's amount
- Simplify inventory
- Handle implicit cost in posting
- Ignore unbalance txn error for txns which contains implicit cost postings
- Make clippy happy

### Fixed

- Use tokio fs instead std fs to make sure everything runs on async
- Return empty u8 vec if file does not exist
- #310 limit budget percentage to maximum 100%
- Return correct price commodity
- Fix table style
- Cols setting
- Select in account balance line
- Center of the pagination
- Notification system
- Action button color
- Typo commodities
- Size using calc
- Table with border
- Divider label position on the left
- Use absolute route
- Position of budget's progress bar
- Import dropzone style file
- Set min and max for account balance history graph
- Move test source to temp folder for cleaning testing
- Fix the case of multiple transaction date
- Fix the indent of list
- Show cost info in commodity lots page

### Removed

- Remove some console.log
- Remove mut flag in test
- Remove self-implemented github service
- Remove some useless function
- Remove expect to prevent panics
- Remove logging filter
- Remove useless method
- Remove zero amount lot record
- Remove price field in lot's record
- Remove hardcode uuid for reporting

## [0.1.8-beta.3] - 2024-04-27

### Fixed

- Fix the docker image tag

## [0.1.8-beta.2] - 2024-04-27

### Changed

- Make fmt happy
- Handle ctrl+c
- Make document cacheable

### Fixed

- Fix deprecated function

## [0.1.8-beta.1] - 2024-04-25

### Added

- Add translation for DefineDuplicatedBudget error
- Add transition for border color
- Add the mapper plugin handler
- Add router plugin related struct
- Add git sha tag for docker image
- Add plugin feature test
- Add wasm build test in PR flow
- Add plugin feature on docker build
- Add the macro tu enable feature or not
- Add plugin list in frontend setting page
- Add api of updating transaction
- Add edit transaction in frontend
- Add integration test
- Add more build target
- Add libssl in ubuntu runner
- Add pkgconfig env
- Add cross config for aarch64

### Changed

- Need to pass the value type when retrieving the option
- Extract validate method to make code readable
- Migrate update doc from github wiki to docs
- Use new upgrade link
- New UI of statistic box
- New UI of section
- Bump h2 from 0.3.24 to 0.3.26
- Plugin directive support meta.
- Use extism to implement a new plugin system and implement the processor plugin
- Make plugin as module folder
- Create cache folder if not exist
- Should not handle include directives in process method
- Make clippy happy
- Rename the feature as plugin_runtime
- Correct feature name in macro
- Restore the integration test
- Move app state to separate file
- Handle the flag for transaction
- Use From trait instead of Into
- Make clippy happy
- Combine the payee and narration
- Use once cell to detect timezone once
- Make fmt happy
- Use journal item's tags and links directly
- Show is_balanced flag in journal list and render the error
- Passing the options into plugin context
- Use trait to standard the plugin behavior
- Make clippy happy
- Download the frontend build artifact
- Upload the build artifact to github release
- Use podman to build docker image
- Notify example repo to rebuild example image
- Only trigger the CI when push to main
- Set continue on error to true
- Trigger ci when tags are pushed
- Make some dependencies into workspace

### Fixed

- Fix the feature name
- Fix github sha for docker image tag
- Fix docker image tag
- Fix the feature in dockerfile
- Fix the testcase
- Fix the testcase
- Raise unbalanced error given unbalance txn
- Fix the download artifact file

### Removed

- Remove useless import
- Remove plugin directive in integration test
- Remove the arm build
- Remove armv7 target
- Remove musl target
- Remove pkg config path
- Remove i686 target and add cross config for riscv64
- Remove riscv64 build target

## [0.1.7] - 2024-03-28

### Fixed

- Non-existed commodity wont break the process of transaction

## [0.1.6] - 2024-03-28

### Added

- Add MultipleOperatingCurrencyDetect error
- Add MultipleOperatingCurrencyDetect in frontend
- Add some translation and use pnpm
- Add pnpm.lock file

### Changed

- Install pnpm first

### Fixed

- Fix the command of publishing to npm
- Skip the process for those invalid txn
- Fix test code

### Removed

- Remove dbg macro in test code

## [0.1.5] - 2024-03-27

### Changed

- Show parse error msg as human readable format
- Zhang-ast support any str as flag
- Zhang parser and beancount parser support any upper char as the txn flag

## [0.1.4] - 2024-03-26

### Changed

- Zhang parser support comma and underline symbol for number
- Beancount parser support comma and underline symbol for number
- Make fmt and clippy happy
- Release wasm when release zhang
- Temp disable coverage

### Fixed

- Fix wasm version update script

## [0.1.3] - 2024-03-26

### Added

- Add sha in create or update github content request
- Add number expression support in beancount parser
- Add some comment for beancount number expr parsing & make clippy happy
- Add github datasource document
- Add code highlight for github datasource
- Add i18n for documentation
- Add discord link

### Changed

- Move example dockerfile into docker folder
- Move example docker image snapshot build
- Trigger example build after snapshot build is done
- You can navigate to account page in document page
- Impl github backend
- Update versions of actions, replace actions-rs/toolchain
- Run cargo fmt
- Ignore clippy::empty_docs warning explicitly
- Bump versions of multiple dependencies
- Run cargo fmt & clippy on nightly toolchain
- Use setup-node action to ensure caching
- Run coverage test on stable rust toolchain
- Make clippy happy
- #260 use starlight as document generate tool
- Migrate the arithmetic logic from beancount parser to zhang parser
- Make clippy happy
- Make fmt haapy

### Fixed

- Fix example Dockerfile
- Should quote document path to make sure we can parse files whose name contains space
- Refresh document list after document is uploaded
- Return correct file extension and render it in document page
- Show documents in datetime desc order
- Fix cargo clippy warnings
- Fix deprecation in base64 crate
- Correct inpropriate use of BigDecimal in parser test
- Fix options of setup-node action
- Http 0.2 and 1.1 dependency
- Fix logo path
- Fix link of documentation

## [0.1.2] - 2024-02-28

### Changed

- Return store in wasm
- Update zhang-wasm version to 0.1.0
- Make clippy happy
- Update readme and add playground link

### Fixed

- Handle empty space line for zhang and beancount

### Removed

- Remove Instant now access, cause it's not working in wasm
- Remove debug code

## [0.1.1] - 2024-02-26

### Added

- Add active style for navbar
- Add account filter
- Add border for account table
- Add postings into txn domain

### Changed

- Use checkbox for hide closed account
- Journal should filter by keyword now
- Account filter ignore case
- Journal list support search
- Use postings in txn directly

### Fixed

- Txn filter should ignore case

### Removed

- Remove useless import

## [0.1.0] - 2024-02-21

### Added

- Add Local file system data source

### Changed

- Introduce new DataType and DataSource, impl ZhangDataType
- Make clippy happy
- Rename variables from transformer like to data_source
- Beauty the style of new version

### Fixed

- Impl DataSource for Opendal
- Test cases
- Impl opendal data source

### Removed

- Remove exporter trait
- Remove transform.rs
- Remove database relative files
- Remove test data source

## [0.1.0-beta.6] - 2024-01-21

### Fixed

- Version file generate script

## [0.1.0-beta.5] - 2024-01-21

### Changed

- Update icon for frontend
- Load version from standard file
- Set build version in release action

### Fixed

- Pin package version to 0.1.0
- Version fo packages

### Removed

- Remove useless variable

## [0.1.0-beta.4] - 2024-01-16

### Changed

- Group journal item by day

### Fixed

- Txn's attachment path should be trimmed prefix

## [0.1.0-beta.3] - 2024-01-04

### Added

- Add document for booking method
- Add mutate in useEffect deps

### Changed

- Beancount support booking_method
- Trigger ledger reload when new transaction is added
- Reload info_for_new_transaction api when ledger is reloaded
- Report a random generated uuid

### Fixed

- #214 support trailing space at the end of line

## [0.1.0-beta.2] - 2024-01-01

### Added

- Add trx sequence
- Add payee test
- Add accounts test
- Add example file for testing
- Add DocumentType test
- Add account balance test
- Add refresh button in journal and account page
- Add python binding
- Add ledger parse methods
- Add ledger options and test python file
- Add account access method
- Add account repr for pythong binding
- Add commodity domain
- Add amount and postings domain
- Add bin name in Dockerfile
- Add doc structure
- Add doc build step
- Add logo
- Add zhang perfix for libs
- Add store api
- Add integration-tests structure
- Add integration test framework
- Add start:demo command
- Add example deployment docker file
- Add budget store and budget directive logic
- Add budget integration test files
- Add budget list api
- Add translation for budget nav
- Add progress bar in budget item
- Add budget change by txn
- Add border for journal table
- Add arrow left and right icon to jump to previous or next month
- Add key for iterable budget item to re-render component for refresh the progress bar
- Add budget info api
- Add game-expense related accounts test
- Add budget beancount parser
- Add web basic auth
- Add auth_credential for integration test
- Add multiple postings test
- Add logo
- Add reload api
- Add documentations
- Add info for enabling the frontend feature
- Add inline comment test

### Changed

- Isolate insert_commodity
- Isolate all open accounts
- Isolate all payees
- Isolate static
- Isolate all account
- Isolate transaction and price query
- Isolate lot relative query
- Format code
- Migrate options into in-memory store
- Migrate commodity into in-memory store
- Migrate lots into in-memory store
- Migrate balance into in-memory store
- Format code
- Reorder the imports
- Make clippy happy
- Move text target to a mod
- Make clippy happy
- Seperate routes into modules
- Update document
- Flatten balance check and pad
- Impl new json path test strategy
- Format code
- Make clippy happy
- Statistic api
- Code format
- Export domains
- Make fmt and clippy happy
- Bump rustix from 0.36.11 to 0.36.17
- Use pathbuf if path does not containt glob char
- Make fmt and cliipy happy
- Doc ci
- Uses to run
- Move command
- Mv command 2
- Zhang id
- Zhang id2
- Update ci permission
- Update overview
- Bump axios from 1.2.0 to 1.6.0 in /frontend
- Bump @babel/traverse from 7.17.9 to 7.23.3 in /frontend
- Update online demo uri
- Update readme.md
- Bump openssl from 0.10.55 to 0.10.60
- Update to 0.1.0-alpha.6
- Implement integration test logic
- Make clippy happy
- Move logo to assets
- Move wasm into bindings
- Move example deployment into folder
- Budget directive parser and exporter
- Budget-add directive parser and exporter
- Budget-transfer directive parser and exporter
- Budget-close directive parser and exporter
- Handle budget add directive logic
- Handle budget transfer directive logic
- Handle budget close directive logic
- Impl budget page
- Budget activity amount coming from txn
- Show search and new transaction btn in two rows
- Can select date in budget page
- Activity amount of budget should be clear when month changes
- Interval event list and api
- Make clippy happy
- Format code
- Custom directive support meta
- Make clippy happy
- Format frontend code
- Format frontend code
- Handle beancount exporter
- Make clippy happy
- Zhang's posting support meta
- Exporter for zhang posting's meta
- Beancount parser support posting meta
- New doc framework
- A working version of involving opendal
- Use beancount parser if it's beancount format
- Extract server app creation function
- Format code
- Trigger reload when file changes
- Make clippy happy
- Inline comment supported for zhang data type
- Inline comment supported for beancount data type
- Make clippy happy
- Use codemirror as editor
- Version 0.1.0-beta.2

### Fixed

- Commodity balance calculation
- Use resolver version 2 for whole workspace
- Combine same price commodity lot
- Frontend route path
- Fix clippy
- Format python test code
- Return owned string for amount currency
- Fix ci
- Show hour in 24 hour format
- Load validation points and print it.
- Fix core path for wasm
- Fix budget assigned_amount integration test
- Use string as hash key
- Should not return non-opened budget in budget list
- Should only return budget related accounts
- Integration test
- Frontend packaging
- Expose inner tuple to fix test
- Fix documentation uri
- Docker build rust version
- Create txn should use post method
- Use empty string if payee is null
- Base image for runtime

### Removed

- Remove sqlx related code and enable wasm
- Remove sqlite feature
- Remove all sqlite and sqlx query
- Remove sqlx in server lib
- Remove async in core lib
- Remove tokio dependency
- Remove useless imports
- Remove useless file
- Remove github docker registry
- Remove serde feature
- Remove useless build script
- Remove leading slash
- Remove commented tests
- Remove dbg test macro
- Remove shadow for section
- Remove useless variable
- Remove useless database parameter
- Remove comment
- Remove async dead blocking task
- Remove useless styles

## [0.1.0-alpha.5] - 2023-08-17

### Changed

- Move text tranformer and export back to core
- Centralize the transformer and exporter for beancount

### Fixed

- Resolved #175 to use hk as default timezone

## [0.1.0-alpha.4] - 2023-06-05

### Added

- Add prettier command
- Add frontend style check

### Changed

- Show non leaf account amount
- Run prettier
- Show multiple commodity in account page
- Make prettier happy
- Upgrade react to version 18 and fix the date range in report page
- Bump to 0.1.0-alpha.4

### Fixed

- Fix the prettier check command
- Show currency symbol after minus mark
- #159 remove txn not balance error for given non-balanced check directive

### Removed

- Remove pointer cursor for non leaf account

## [0.1.0-alpha.3] - 2023-05-31

### Added

- Add update command
- Add update notification

### Changed

- Bump to 0.1.0-alpha.3

### Fixed

- Typo of the exporter

### Removed

- Remove release tag prefix

## [0.1.0-alpha.2] - 2023-05-30

### Added

- Add frontend build and cache

## [0.1.0-alpha.1] - 2023-05-30

### Added

- Add idea folder to git ignore
- Add wasm lib
- Add wasm lib
- Add cli and website
- Add open and plugin parser
- Add custom directive
- Add transaction
- Add comment
- Add transaction tags and links
- Add readme
- Add account and amount
- Add draw of beancount implement
- Add forbid unknown payee configu
- Add beancount exporter
- Add date or datetime info into ast
- Add meta output for text target.
- Add chinese name
- Support date hour
- Add complex sort test
- Add docs.rs badge
- Add development and pr action
- Add development and pr action
- Add balance directive logic
- Add silent init flag
- Add graphql entry for backend server
- Add basic journal page
- Add journal page of frontend
- Add statistic box component.
- Add file listener
- Add statistic dto
- Add append data mutation
- Add empty line between transaction content
- Support multiple postings add append, deletion action
- Add git pre-commit hook
- Add block for section
- Add account check ui
- Add meta query for account
- Add price grip using latest map
- Add price grip for account snapshot
- Add distance for statistic
- Add checkbok for hide closed account
- Add eslint for frontend
- Add test
- Add price test
- Add test
- Add multiple value map test
- Add latest map test
- Add price grip test
- Add account test
- Add amount test
- Add balance pad parse test
- Add inventory test
- Add idx key for loop render
- Add snpshot build action file
- Add snapshot build name
- Add non begin at zero for income and expense axis
- Add cli hint msg
- Add statistic journal list
- Add journal to statistic
- Add status bar in report page
- Add accountType and sign for account
- Add docker file and build as snapshot
- Add run-on to fix snapshot build
- Add journal side preview content for journal page
- Add tags and links resolver
- Add timestamp for transaction
- Add account does not exist error type
- Add modal to show and edit ledger error
- Add filename to span info
- Add error length query for error box title
- Add account closed error type
- Add commodity does not define error
- Add default tolerance precision
- Add default tolerance precision
- Add commodityInventory for single commodity
- Add inventory for commodities
- Add local proxy to suit github codespaces development
- Add single commodity page with lot detail
- Add price history for single commodity
- Add sample i18n code
- Add latest balance time for single account
- Add cargo lock and config for notify
- Add toolchain setting for test job
- Add default commodity info for operating currency
- Add new ui for document line
- Add wechat extractor into tool list
- Add precision col in commodities table
- Add meta modification part
- Add pading on meta line in transaction preview page
- Add close badge for closed account
- Add empty line on beginning of directive
- Add table view for documents
- Add sqlite support MVP
- Add file path in error
- Support serve to specify database location
- Add meta for transactions
- Add database log
- Add span info into transaction table
- Add commodity info for default currency
- Add doucment icon for those have document meta
- Add frontend analyze tool
- Add backend server base url into file update api
- Add balance check record
- Add commodity state
- Add unlimited idle time for memory database
- Add unlimited idle time for memory database
- Add title in info api
- Add loader page
- Add dependencies for fmt and clippy
- Add example docker image build process
- Add readonly for example docker
- Add readonly for example docker
- Add readonly for example docker
- Add simple balance check preview
- Add notification after transaction is created
- Add husky hook
- Add responsive view for setting
- Add missing dependencies
- Add debug trait for error kind
- Add balance check for account
- Add balance pad to account
- Add masked amount for balance page
- Add backend api and reflect on unbalance amount logic
- Add beancount parser
- Add type annotation for tag stack
- Support file with bean extension
- Add test for open directive without commodity
- Add dot env file to git ignore
- Add commodity test
- Add report graph component
- Add account status in account balance table
- Add test for meta
- Add options api and show them as table in setting page
- Add test for geting all options
- Add default option override test
- Add variant of closing non zero account
- Add frontend i18n for error
- Add test for closing account
- Add release yml

### Changed

- Init
- Typo license
- Merge from beancount
- Replace beancount to avaro
- Char instead of string
- Pest parser
- Implement a lot directive
- Implement commodity
- Open directive support meta
- Implement comment
- Update readme.md
- Structure posting meta
- Implement transaction
- Foramt
- Project structure
- #[allow(clippy::upper_case_acronyms)]
- Make cargo fmt and clippy happy
- #2 open directive support meta
- Implement wechat importer
- Resolve #5 date time now support datetime
- Impl text target
- Export meta data
- Convert datetime back to date and time in meta.
- Ledger struct
- Impl text target for all directives
- Separate quote string and unquote string
- Optional config to output result to file
- Resolve #6 lower case rule
- Wechat export unknown payee
- #10 rename to zhang
- Update project badge
- Tasking
- Structure option, plugin, include and comment
- Typo
- Sort directive on loading
- Extract open directive info
- Format badge
- Close directive
- Commodity directive
- Update coveralls badge
- #9 load single file
- Resolve #9 multiple file support
- #11 add snapshot struct
- #11 calculate normal posting data.
- #11 calculate unit given one missing unit posting
- #11 calculate price based posting
- #11 calculate single price based posting
- #11 add daily account snapshot
- #3 remove pad directive
- Resolve #3 balance directive with pad
- Embedded server and frontend for ui
- Update readme.md
- Build frontend after backend build
- Return account snapshot api
- Transaction and balance check dto, and cors layer
- Badge for journal item
- Account page.
- Store entry for reload purpose.
- Reload logic
- Cargo fmt
- Cargo fmt
- Raw editing for files in frontend ui
- Get nearest day snapshot
- Statistic logic
- Single account journal
- New transaction modal data.
- Update git ignore
- Update git ignore
- Date time picker and date only checkbox
- Load account list in modal
- Save button validator
- Refetch relative query once transaction is add
- Load documents from document directive
- Document frontend
- Account document line with extension badge
- Navigate to account in document page.
- Simple error dto
- Update hook
- Format code
- Update axum extension
- Implement account balance check api
- Implement account document
- Implement account document frontend ui
- Account line show currency snapshot summary
- Update statistic to generic from to end timestamp for more generic
- Load option directive
- Extract process logic to Trait Directive Process
- Multiple currency support
- Statistic using from and to
- Customize category instead of hard code category
- Using customize group for statistic bar
- Snapshot rename to inventory
- Record snapshot in detail
- Process price directive
- Reorder liabilities statistic box
- Frontend type
- Separate frontend-build
- Test the doctest as well
- Use nightly toolchain
- Save code coverage as artifact
- Move inventory to separate mod
- Update readme
- Using create-react-app instead of customize parcel builder
- Update pipeline
- Use JournalLine to hide the Journal detail
- Allow statistic to separate into frames by day
- MultiAxis chart implementation
- Homepage statistic
- Update example accounting
- Implement report page
- Report setting
- Report page ui enhance
- Serve a path
- Login ghcr.io
- Rename artefact
- Transaction preview
- Transaction meta support
- Resolve #25 meta support same name key
- Show meta block only when metas is not empty
- Center new transaction button
- #52 publish image to docker hub
- Upload account documents
- #53 parsing before append it into files
- Select component style
- #37 using selectable for payee input
- #49 show transaction document in document list
- #49 hide document meta in meta list
- #49 show document in transaction preview page
- #49 format code to make clippy happy
- Resolve #8 add span info in directive to support raw editing
- Show error msg in graphql
- Show error summary in log
- Error pagination and list rendering
- Format code
- Format code
- Modal of updating directive with error
- Reload ledger and reload error list after file modification
- Check transaction balanced
- Show unbalanced info in journal list and preview page
- Parse cost and price in to posting
- Implement trade_amount
- Implement infer_trade_amount
- Implement is_balance method
- Restore the inventory logic
- Export price to string target
- Resolve #65 add bigdecimal ext to support round down and round up
- Refactor the txn balanced method
- Currency support precision and rounding
- Transaction balance support default and currency specific permission and rounding
- Implement commodities list and price list
- Impl FIFO and FILO strategy
- Record currency info into lots
- Format code
- Impl commodity lots detail
- Filter zero number lots
- Update dockerfile and use 8000
- Copy data folder from builder
- Update dockerfile and use 8000
- Rename docker image name
- I18n support, and switch in setting page
- Format code
- Use matches macro
- I18n the nav items
- Implement journal pagination
- Format code
- Implment Eq for those implemented PartialEq
- Implement Eq for structs
- Implement Eq for structs
- Implement Eq for structs
- Parallel check flow
- Cancel the previous running workflow
- Check if file content match zhang's syntax
- Document list item
- Group document by file path
- Format code
- Watch the entry path for incoming zhang's file candidate, and ignore unrelated files
- Text target support transaction cost
- Update commodity info when processing commodity directive
- New mantine ui
- Refact number representation
- Allow unused parse rule
- Set logarithmic for bar chart
- Show trx summary in vertical center
- Show summary color base on amount sign
- Implement transaction upload logic
- New section box ui implementation
- Align title and date range picker to oneline
- Store hide closed account flag into local storage
- Make clippy happy
- Component with oneline on sm page size
- Placeholder and fullscreen when on mobile
- Replace useState with useListState
- Upload screenshot picture
- Update readme
- Show hierachy account tree
- Align right for the blanace column
- Align last price date to end
- Pretty the link and tag badge
- Update nav icon
- Change system to async
- Handle sqlite process for close and commodities directive
- All directive support database cache now
- Format code
- Make clippy happy
- New account list restful api.
- Using actix-web instead of axum
- Account balance api
- Account journal api
- Get files name api
- Get/update file content api
- New journal api
- Journal item mobile ui
- Big decimal for sqlite type
- File edit and simple statistic
- New transaction api
- Statistic api implementation
- Enable log with default info level
- Wrap api data into wrapper
- Implement statistic api
- Report api
- Report income and expense rank
- Journal pagination
- Error api
- Document preview modal
- Document preview for document table
- Fmt code
- Fmt code
- Update base rust version
- #101 add basic info api
- #101 add log for the version
- #104 text to be translated
- Make clippy happy
- #101 add interval task of version report
- #101 add cli opt to control the report task
- #101 make clippy happ
- Suppress error threshold to 256
- Impl basic table view
- New graph ui and table ui
- Badge for journal type
- Fold useless code
- Show balance check
- Journal only support table view now
- Show journal preview icon when hover
- Fold useless code
- Use borrow trait to accept map key
- #111 add time logging for directive process function
- #111 add instruction for flamegraph
- #67 remove self implemented line info extractor
- Use memory database if path is not present
- Move sql operation into domain
- #108 sse MVP
- #109 only listen modify event
- #108 reload error and currency info when reload
- #108 store account into redux
- #108 show basic info and control by sse
- #108 remove version in nav
- #108 add line clamp for title
- Update example docker builder
- Create application folder
- Update example docker builder
- Update example docker builder
- Non root user
- Non root user
- Update readme.md
- Update screenshot
- Update readme.md
- Impl #113 journal data store in redux
- #116 simple setting page
- Format the code via prettier
- Update dev cargo build
- Update example
- Align three setting into oneline
- #126 extract transformer layout
- #126 make cargo check and clippy happy
- #126 add frontend feature to make bundled frontend as optional
- #126 support inline comment
- #126 support pad directive
- #126 support pushtag and poptag
- #126 separate project into multiple slight lib
- #126 add text transformer and exporter
- Tier display file tree
- How exporter append directive
- #126 extract appendableExporter from exporter, so the server can handle append operation.
- Rename append-directives method's name
- Apply function no need move to exporter
- Make fmt and clippy happy
- New error kind for ast
- Introduce the text file based transformer
- Impl grid document box
- #125 add error notification for new transaction
- Generate fixed id
- Bump openssl from 0.10.47 to 0.10.50
- Use fixed id for journal to make frontend rendering work
- Batch balance ui
- Bump h2 from 0.3.15 to 0.3.17
- Update code coverage step
- Update cache step
- Update cache step
- Update coverage step
- Make clippy happy
- Bump spin from 0.9.3 to 0.9.8
- Resolve #42 support wildcard operator in include directive
- Clean zhang pest file
- Rename parser mode
- Clean up zhang syntax and add some beancount directives
- Handle beancount push tag and pop tag directive
- Try to infer transformer by file extension
- Inline method to trait
- Reorg beancount directive variable name
- Combine pad and balance directive into one
- Make clippy happy
- Implement beancount exporter
- Extract time from meta
- Make clippy happy
- Move sql schema to seperated file
- Extract operations to centrelize database operation
- Return calculated statistic
- Display calculated amount in frontend
- Statistic box style
- Display label behind amount
- Extract metas
- Set max line width to 160
- Move some db operations into operations
- Report graph style and report statistic style
- Colorful statistic box
- Colorful statistic box
- Store error in database
- Centrelize insert option operation and add test
- Make clippy happy
- Make fmt happy
- Store default options into DB
- Extract constants
- Extract insert meta operation
- Commodity should get precision from default options
- Meta precision should have higher priority
- Raise error on close directive
- Make fmt happy
- Extract operation of closing an account
- Extract text enum macro
- Extract account status enum
- Extract meta type enum
- Update code coverage badge
- Modify balance check narration
- Bold payee
- Should amount in single account page as currency format and align right
- Impl balance check and pad UI in line and detail preview
- Bump to 0.1.0-alpha.1

### Fixed

- Fix doc test
- To text trait
- Fix custom to text trait
- Fix
- Fix chinese name
- #9 correct include path
- Parser support no flag
- Fix yarn install command
- Wechat importer and frontend demo
- Frontend uri without extension map to index.html for frontend router
- Git ignore
- Transaction support non-amount posting
- Chinese word would not be escaped
- Refresh document info when reloading.
- New transaction button text
- Update errors when reloading
- Update current snapshot after balance check
- Show current month statistic
- Distance account filter should be start with
- Sort account
- Production do not hard code url prefix
- Disable for oneline
- Fix mkdir nested folder
- Plugin test
- Fix account parent method
- Parser of posting cost time and to text target test
- Parse option directive
- Update frontend build folder
- Fix typescript warning
- Fix bin name
- Fix docker file
- Amount number in statistic bar
- Chart registry all registerables
- Use [u8] as buffer to support binary file
- #37 null payee should return empty string
- Resolve #18 format zero as 0 instead of -0
- Fix test
- #67 use self-implement line col algorithm due to low speed of pest implement
- Null handler for posting unit
- Process instance access issue
- Move hook after condition judgement
- Canonicalize the entry path when storing
- Rounding key of commodity
- Use inferredUnit if unit is not defined
- Resolved #91 manually handle journal page data
- Handle duplicated page info
- Use function date to access old data
- Balance check style
- Report page ui issue
- Group the same summaries for same currency
- Line offset of uploading transaction document
- Full page height for the journal page
- Home graph total sign and enable logarithmic
- Tags and links for single account request
- Empty tags and links
- Empty tags and links on preview page
- Correct date getter from Date instance
- Newline issue in windows
- Chart height
- Check error implementation
- Fix issue regrading single document uploaded as multiple documents
- Document preview issue
- Restore transaction's balance check
- Test for acount open and close directive
- Should update commodity info given operating commodity and commodity directive
- Test for multiple file
- Navbar i18n
- Should distinct the documents
- Use the same db file when reloading
- Only open document preview modal for image document
- Disable user scalable
- #103 remove empty payee
- #104 add payee candidate into select item
- #102 fetch REAL type as string and convert it to bigdecimal
- #104 remove useless code
- #104 use raw state hook
- Use bigdecimal to calculate balance
- Report status group
- #2 set precision as 2 if not presented
- #126 remove comment test
- #126 add frontend feature flag when build release
- Align item in table into right
- Return invalid account error instead of empty given invalid account
- #126 no need to wrap in Data twice
- #126 use text exporter as default exporter
- #126 test case
- Fix Dockerfile
- Square box for images of document preview modal
- Insert document for transaction
- Unit test case
- Infer transformer via file extension
- Fix unit test
- Fix the order of latest account balance and load exporter based on extension
- Balance directive should have higher priority
- Should use sequence to get day's latest account balance
- Clean up beancount parser
- Price's amount should use real type
- Account balance sql
- Datetime col name for account balance and add empty test
- Should be passed given same operating currency and defined commodity

### Removed

- Remove lalrpop parser
- Remove lalrpop
- Remove useless import
- Remove old transaction
- Remove from str for inventory
- Remove useless mod file.
- Remove clippy and fmt warning
- Remove useless import
- Remove dbg!
- Remove pre-commit hook
- Remove build check for typescript
- Remove useless parse function
- Remove useless parse functiono
- Remove useless inventory mod
- Remove useless frontend project
- Remove lock file flag for cargo build
- Remove useless import
- Remove unused import
- Remove todo for report
- Remove useless import
- Remove error log for account balance check
- Remove useless import
- Remove dbg macro
- Remove unuse import
- Remove dbg macro
- Remove musl buld
- Remove musl buld
- Remove useless import
- Remove useless variable
- Remove useless import
- Remove useless import
- Remove useless import
- Remove import
- Remove useless nav item
- Remove test notifications
- Remove uesless import
- Remove useless icon import
- Remove pre commit hook
- Remove dbg statement
- Remove rust hook
- Remove context cache in ledger entity, cuz it is used tempfile as store.
- Remove useless import
- Remove graphql related code
- Remove graphql related dependency
- Remove useless test code
- Remove graphql dependency
- Remove useless import
- Remove codemirror to make it lightweight
- Remove capover file
- Remove jotai and implement with redux
- Remove useless code
- Remove useless process context
- Remove useless load from str
- Remove some unwrap
- Remove useless dependencies
- Remove some done todo
- Remove useless import
- Remove comment
- Remove trailing space in open directive
- Remove filter condition
- Remove useless in-memory options and configs store

[0.3.0]: https://github.com/zhang-accounting/zhang/compare/v0.2.0...main
[0.2.0]: https://github.com/zhang-accounting/zhang/compare/v0.1.10...v0.2.0
[0.1.10]: https://github.com///compare/v0.1.9..v0.1.10
[0.1.9]: https://github.com///compare/v0.1.8..v0.1.9
[0.1.8]: https://github.com///compare/v0.1.8-beta.3..v0.1.8
[0.1.8-beta.3]: https://github.com///compare/v0.1.8-beta.2..v0.1.8-beta.3
[0.1.8-beta.2]: https://github.com///compare/v0.1.8-beta.1..v0.1.8-beta.2
[0.1.8-beta.1]: https://github.com///compare/v0.1.7..v0.1.8-beta.1
[0.1.7]: https://github.com///compare/v0.1.6..v0.1.7
[0.1.6]: https://github.com///compare/v0.1.5..v0.1.6
[0.1.5]: https://github.com///compare/v0.1.4..v0.1.5
[0.1.4]: https://github.com///compare/v0.1.3..v0.1.4
[0.1.3]: https://github.com///compare/v0.1.2..v0.1.3
[0.1.2]: https://github.com///compare/v0.1.1..v0.1.2
[0.1.1]: https://github.com///compare/v0.1.0..v0.1.1
[0.1.0]: https://github.com///compare/v0.1.0-beta.6..v0.1.0
[0.1.0-beta.6]: https://github.com///compare/v0.1.0-beta.5..v0.1.0-beta.6
[0.1.0-beta.5]: https://github.com///compare/v0.1.0-beta.4..v0.1.0-beta.5
[0.1.0-beta.4]: https://github.com///compare/v0.1.0-beta.3..v0.1.0-beta.4
[0.1.0-beta.3]: https://github.com///compare/v0.1.0-beta.2..v0.1.0-beta.3
[0.1.0-beta.2]: https://github.com///compare/v0.1.0-alpha.5..v0.1.0-beta.2
[0.1.0-alpha.5]: https://github.com///compare/v0.1.0-alpha.4..v0.1.0-alpha.5
[0.1.0-alpha.3]: https://github.com///compare/v0.1.0-alpha.2..v0.1.0-alpha.3
[0.1.0-alpha.2]: https://github.com///compare/v0.1.0-alpha.1..v0.1.0-alpha.2

<!-- generated by git-cliff -->
