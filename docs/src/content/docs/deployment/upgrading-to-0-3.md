---
title: Upgrading from 0.2 to 0.3
description: What changes for a ledger, its plugins and the scripts that use the API when you upgrade Zhang from 0.2 to 0.3, and what to do about it.
---

Zhang 0.3 checks a ledger the way Beancount does, lets the server decide what plugins may do, and retires the API endpoints the old web UI used. A ledger that loaded without errors on 0.2 can report errors on 0.3, a plugin can lose its network access, and a script can find an endpoint gone. Nothing is converted: your files stay as they are, and you fix what the new release reports.

This page lists what to check. The [changelog](https://github.com/zhang-accounting/zhang/blob/main/CHANGELOG.md) lists every change, with the pull request behind it. How to replace the program itself is in [Upgrading](/deployment/upgrading/).

## Before you upgrade

1. Back up the ledger folder, or commit it if it is a Git repository. Include `.zhang/passkeys.json` if you use [passkeys](/deployment/authentication/#passkeys).
2. Note which plugins the ledger loads (`plugin` directives) and what they need: network access, a long `timeout`, files of the ledger. The server now limits all three, see [Plugins](#plugins). If the ledger has budgets, keep plugins on: budgets are a plugin in 0.3.
3. If you use the Docker tag `kilerd/zhang:snapshot`, switch to `kilerd/zhang:latest` or a version tag such as `kilerd/zhang:0.3.0`: images are published for releases only, and `snapshot` is no longer updated. To keep the compiled code of plugins when the container is re-created, mount a volume of its own at `/root/.cache/zhang`, see [Docker](/getting-started/installation/#docker).

## Plugins

The server, not the ledger, now has the last word on what plugins may do. By default plugins run read-only, without network and for at most 30 seconds a call. Each `plugin` directive that asks for more gets a [`PluginCapabilityReduced`](/reference/error-codes/#plugincapabilityreduced) error naming the setting that would allow it:

| The plugin needs | Allow it with |
|---|---|
| the hosts in its `allowed_hosts` | `ZHANG_PLUGIN_ALLOWED_HOSTS` |
| a `timeout` above 30 seconds | `ZHANG_PLUGIN_MAX_TIMEOUT` |
| to write entries (`allowed_writes`) | `zhang serve --plugins write` or `ZHANG_PLUGINS=write` |
| more than 256 MiB of memory | `ZHANG_PLUGIN_MAX_MEMORY` |

`--plugins off` (`ZHANG_PLUGINS=off`) loads no plugin at all. [What the server allows](/guides/plugins/#what-the-server-allows) describes each setting. A plugin's HTTP requests also no longer follow redirects, ignore the proxy settings of the environment, and never reach a loopback, private or link-local address.

**`ZHANG_PLUGINS=off` turns budgets off too.** Budgets are interpreted by the official plugin `zhang:budget`, which every ledger has, without a `plugin` directive and whatever its `features.plugin` option says, and which the operator's switch stops as it stops every plugin. A server run with `--plugins off` or `ZHANG_PLUGINS=off` logs that budgets are off when it starts; the budget pages and queries answer that the budget tables are unavailable, and the error list shows one [`ExtensionUnavailable`](/reference/error-codes/#extensionunavailable) error. The query function `account_budgets()` still answers, as it reads only the `budget` metadata of the accounts' `open` directives. Budgets need `ZHANG_PLUGINS` `read`, the default, or `write`.

**Budgets are a plugin.** `zhang:budget` is a Lua script bundled with Zhang, and since 0.3 the only implementation of budgets: Zhang's native budget code is gone. Every ledger runs it, at every load and so after every write. The budget pages, the budget tables, the built-in queries and the budget errors answer as described above. What you can see as the operator:

- **First start.** Every ledger now runs a plugin, so the first start compiles the plugin runtime, and some memory while it lasts, given back once it is compiled: with an empty [cache of compiled code](/guides/plugins/#compiled-code), a start takes about 0.5 to 0.7 seconds at 10,000 transactions, and about 0.3 seconds with it. Later starts read the code from the cache, and a reload, so every write, reuses it. Without the cache, such as in a re-created container without its cache volume, every start compiles it again, so mount the cache volume of a container.
- **Memory.** The server holds more for the plugin runtime and the budgets' analysis: about 10 to 20 MiB with a ledger of 10,000 transactions, about 40 to 60 MiB with 100,000. Look at a container's memory limit after the upgrade.
- **Logs.** Every load logs the analysis at the `info` level, such as `analyzer zhang:budget analysed the ledger in 19.182292ms: 0 diagnostics`, or, for a ledger without budgets, `analyzer zhang:budget has nothing to read in the ledger: its tables are empty`.
- **Settings.** The **Settings** page and `GET /api/plugins` list `zhang:budget`, of the type `Analyzer`.
- **Limits.** It runs within what the server allows every plugin: a ledger whose budgets need more time than `ZHANG_PLUGIN_MAX_TIMEOUT` or more memory than `ZHANG_PLUGIN_MAX_MEMORY` loads without its budget pages and without budget errors, with one [`PluginAnalysisFailed`](/reference/error-codes/#pluginanalysisfailed) error saying which to raise. Of the default 256 MiB, the script's Lua memory is 224 MiB; a budget per category and month, 35 categories over ten years, needs about 50 to 60 MiB.
- **No other implementation.** Where `zhang:budget` does not run, budgets are not interpreted, and the error list says so once with an [`ExtensionUnavailable`](/reference/error-codes/#extensionunavailable) error: with plugins off (`ZHANG_PLUGINS=off`), in the online playground, and in a program that loads ledgers with `zhang-core` without the bundled script and an analysis host (see [Project Structure](/developers/project-structure/)). The budget tables and queries stay listed there and answer that they are unavailable.
- **Known differences from the budgets of 0.2,** measured before the release: a write and the budget or errors page after it take 35 to 50 ms longer at 10,000 transactions, and a ledger of thousands of closed budgets loads about 2.5 times slower (3,600 closed budgets: 1.2 seconds rather than 0.5). A ledger of a budget per category and month loads 10 to 20% slower (4,200 budgets: 0.53 seconds rather than 0.47).

**Plugins keep their compiled code in a cache of Zhang's own**, in the subdirectory `zhang-wasmtime-cache` of `~/.cache/zhang`, or of the directory [`ZHANG_PLUGIN_CACHE_DIR`](/guides/plugins/#compiled-code) sets, about 128 MiB at most. The cache in wasmtime's default directory (`~/.cache/wasmtime` on Linux) is no longer used, and you can delete it unless another program uses wasmtime; `EXTISM_CACHE_CONFIG` and wasmtime's configuration file are no longer read. As the cache holds code the server runs, Zhang uses it only in directories that belong to the user running it and that no other user can write to, and never in the ledger's folder. A directory Zhang cannot write to, such as on a read-only file system, or one it refuses, no longer stops every plugin from loading: Zhang runs them without the cache and logs a warning when it starts. If you set `EXTISM_CACHE_CONFIG` to move wasmtime's cache, set `ZHANG_PLUGIN_CACHE_DIR` instead; the cache's size is fixed at about 128 MiB.

**What counts toward a plugin's memory.** `ZHANG_PLUGIN_MAX_MEMORY` (256 MiB by default) bounds everything one instance of a plugin holds, from the moment it is built: its memory and its tables, and what its start function or initialisation takes, which also run within the plugin's `timeout`. A module that needs more fails to load, with an error naming the setting. A [Lua script](/guides/lua-plugins/#limits) gets the limit less 32 MiB that Zhang keeps for the host, 224 MiB by default, and raising `ZHANG_PLUGIN_MAX_MEMORY` raises it too. A plugin that reports more than 1000 errors in a run gets one more error saying how many were dropped.

Plugin directives themselves change too:

- **A module must be inside the ledger's folder** and be named `.wasm`, `.wat` or `.lua` (or have no extension). A path outside the folder, `../` or a symlink that leads out stops the load. Move the module into the ledger and refer to it by its relative path.
- **Processors and mappers see booked postings**: amounts the ledger leaves out are filled in, and costs are resolved per lot. A plugin that expects the transactions as written needs `stage: "raw"` on its `plugin` directive.
- **Plugins run in the order of their `plugin` directives**, where 0.2 ran all processors before all mappers. Reorder the directives if one plugin relies on another.

[Router plugins](/guides/router-plugins/) are new in 0.3, so no 0.2 ledger has one; their pages are sandboxed and their requests limited from the start.

## Your ledger

Start 0.3, open the web UI and look at the error list on the **Overview** page. These are the errors and behaviour changes a 0.2 ledger is most likely to meet; [Error Codes](/reference/error-codes/) explains how to fix each error.

- **Account lifecycle.** A posting, `pad` or `balance … with pad` on an account that is not open yet is reported as [`AccountDoesNotExist`](/reference/error-codes/#accountdoesnotexist), and one after the account's `close` as [`AccountClosed`](/reference/error-codes/#accountclosed). A typo in an account name shows up this way. A `close` without a time takes effect at the end of its day.
- **Commodities of an account.** An account opened with commodities, such as `open Assets:Bank USD`, accepts only those: a posting or balance assertion in another commodity is reported as [`CommodityNotAllowed`](/reference/error-codes/#commoditynotallowed), as Beancount does. Add the commodity to the `open`, or leave the list out.
- **Balance assertions no longer move balances.** A failing [balance assertion](/reference/directives/balance/) is still reported as [`AccountBalanceCheckError`](/reference/error-codes/#accountbalancecheckerror), but balances, reports and pads follow the postings. Fix the postings, or correct the balance on purpose with a `pad` or `balance … with pad`.
- **An assertion covers sub-accounts.** A balance assertion on `Assets:Bank` includes `Assets:Bank:Checking` and the other sub-accounts, as in Beancount. Re-check assertions on parent accounts.
- **Assertion times in Beancount ledgers.** In a `.bean` ledger a `balance` is checked at the start of its date, whatever its `time` metadata; a [`BalanceTimeIgnored`](/reference/error-codes/#balancetimeignored) notice marks each assertion whose result that changes. Date it the next day, or remove its `time`.
- **Transactions balance per commodity**, at the commodity's precision. A transaction that mixes commodities or prices without balancing is reported as [`UnbalancedTransaction`](/reference/error-codes/#unbalancedtransaction).
- **Lots are booked as in Beancount.** Review the holdings and gains of accounts held at cost: a reduction that no lot covers rejects the transaction ([`NoEnoughCommodityLot`](/reference/error-codes/#noenoughcommoditylot), [`TransactionCannotInferTradeAmount`](/reference/error-codes/#transactioncannotinfertradeamount)), and an account booked `STRICT` reports [`AmbiguousLotMatch`](/reference/error-codes/#ambiguouslotmatch) when a reduction matches several lots. See [Lots and cost basis](/guides/lots-and-cost-basis/).
- **Booking methods.** An account that uses an unsupported booking method (`NONE`, `AVERAGE`, `AVERAGE_ONLY`) reports [`UnsupportedBookingMethod`](/reference/error-codes/#unsupportedbookingmethod), and an invalid one [`ParseInvalidMeta`](/reference/error-codes/#parseinvalidmeta); the account loads with the default method instead of stopping the load. See [booking methods](/reference/directives/account/).
- **Comments in `.zhang` files.** A posting can carry its own flag, so a line that starts with `*` is no longer a comment and fails to parse. Start comment lines with `;`.
- **Includes.** An `include` that matches no file, or points outside the ledger, is reported as [`IncludeNotFound`](/reference/error-codes/#includenotfound) and the rest of the ledger loads.
- **Pads and budgets.** A pad that pads nothing reports [`UnusedPad`](/reference/error-codes/#unusedpad), a posting after a budget's `budget-close` no longer counts ([`BudgetClosed`](/reference/error-codes/#budgetclosed)), and a second `budget` of the same name reports [`DefineDuplicatedBudget`](/reference/error-codes/#defineduplicatedbudget).
- **New entries in Beancount ledgers** go to `.bean` files: the default `directive_output_path` is `data/{{year}}/{{month_str}}.{{ext}}`.
- **Read-only files.** Zhang saves a local file by writing a temporary file next to it and renaming it over the file. A file Zhang may not write is refused with HTTP 403 instead of being half-written; a hard-linked file is still written in place.

## Budget figures

The **Budget** page of 0.3 can show other figures than 0.2 for the same ledger. The ledger is not at fault: 0.2 computed them wrongly, and 0.3 computes them as the [budgets guide](/guides/budgets/) describes.

- **Activity in another commodity is converted.** 0.2 added up the numbers of a budget's postings whatever their commodity, so 300 USD spent from a CNY budget counted as 300 CNY. 0.3 converts each posting to the budget's commodity at the price in effect on its date. With `2025-03-31 price USD 7.0 CNY`, a CNY budget that spent 3,190 CNY and 300 USD in April showed 3,490 CNY of activity on 0.2 and shows 5,290 CNY on 0.3: the 300 USD count as 2,100 CNY. Its available amount, and what it carries over into May, are 1,800 CNY lower. A `budget-add` or `budget-transfer` in another commodity is converted the same way. An amount that no price converts is left out and reported as [`BudgetCommodityMismatch`](/reference/error-codes/#budgetcommoditymismatch): add a `price` to count it.
- **`closed` is per month.** 0.2 showed a budget as closed in every month as soon as the ledger had a `budget-close` for it, whatever its date. 0.3 shows it closed from the month of its first `budget-close` on: a budget closed on 2025-05-02 is open in April (`budgets.month` for `2025-04-01` answers `closed` `false`) and closed from May. Postings after the close no longer count, see **Pads and budgets** above.
- **A posting counts toward the budgets of its account's `open` in effect at its date.** An account closed and opened again with other `budget` metadata no longer counts its whole history toward both budgets.

The ledger needs no change. If you compare with figures noted under 0.2, expect budgets that spend in more than one commodity, and the months before a budget's close, to differ.

## Scripts and API clients

- **Removed endpoints.** The typed read endpoints of the old web UI are gone: `GET /api/store`, `/api/budgets…`, `/api/documents`, `/api/accounts/{account}/documents`, `/api/accounts/{account}/balances`, `/api/errors`, `/api/statistic/…` and `/api/for-new-transaction`. Run the built-in queries instead: [Retired endpoints](/reference/builtin-queries/#retired-endpoints) gives the query for each. `POST /api/sql` is gone with the SQL Explore page; write the query in [BQL](/reference/query-language/) and send it to `POST /api/query`.
- **Write responses.** `POST /api/transactions` and `PUT /api/transactions/{id}` answer `{"data": {"id": …, "version": …}}` instead of `"Ok"` and `null`, once the ledger has been reloaded.
- **Metadata order.** Metadata keeps the order it was written or sent in; don't rely on the journal's `metas` being sorted by key.
- **Error ids.** Every error has an id of its own: link an error to its transaction through its `txn_id` meta, not through its id.
- **Paging.** `page` starts at 1 and `size` is 1 to 1000; `page=0` is a 400.
- **Account journals** include the account's sub-accounts.
- **Credentials.** An unauthenticated request is answered 401 without a `WWW-Authenticate` challenge: send the password as an HTTP Basic header, or a session token as `Authorization: Bearer`, with every request. See [Authentication](/deployment/authentication/).
- **Errors** are JSON `{"message": …}`, and an unknown `/api` path is a JSON 404.
- **CLI.** `zhang parse` and `zhang export` are removed; `zhang serve` and `zhang update` remain.

## Sign-in

The browser's password popup is replaced by a login page, with passkeys. Sign-ins last 30 days; set `ZHANG_SESSION_SECRET` so that they survive a restart of the server. The mobile app signs in through the web UI, see [Mobile app sign-in](/deployment/authentication/#mobile-app-sign-in); `ZHANG_APP_RETURN_SCHEMES` ignores web schemes such as `https`.

## Stopping the server and its log

- **`zhang serve` stops gracefully on SIGTERM**, the signal `docker stop`, Podman, Kubernetes and systemd send, as on Ctrl+C: it takes no new connection, lets the requests in progress finish (a write is never cut off halfway) and exits with code 0, within about 7 seconds. `docker stop` no longer waits its 10 seconds and kills the container with code 137. If you set `--stop-signal SIGINT` or a `KillSignal=` to work around that, you can remove it. A second signal exits at once, with code 130 for SIGINT and 143 for SIGTERM.
- **The log is quieter.** At `info` it no longer carries the internals of dependencies (`wasmtime_wasi`, `gotcha`, `tracing::span`), which show warnings and errors only unless the filter names them, such as `RUST_LOG=info,wasmtime_wasi=trace`. `ZHANG_LOG` now applies to every line. Lines are coloured only when the output is a terminal, so `docker logs` and journald get plain text; `RUST_LOG_STYLE=always` brings the colours back.

## Memory

The `zhang` binary of 0.3 allocates memory with [mimalloc](https://github.com/microsoft/mimalloc) instead of the system's allocator. After a reload, which every write and every change of a ledger file causes, the memory of the ledger it replaced goes back to the system instead of staying with the server, so a server that writes often holds far less than with the system's allocator; right after it loads a small ledger, it may hold a few tens of MiB more. If you set a memory limit for Zhang, such as a container's, look at its memory under 0.3 after a few writes before you change the limit. A build from source with `--no-default-features` uses the system's allocator.

A plugin compiles on one thread of its own, so its first compile, before the cache holds its code, takes about 60 MiB for a Lua plugin whatever the number of CPUs, and gives it back; a container's memory limit does not need to grow with the host's CPUs.

## After upgrading

Check the version on the **Settings** page, work through the error list on the **Overview** page, and open a few account and report pages you know well to compare their figures with what you expect.
