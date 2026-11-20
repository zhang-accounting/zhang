---
title: Plugin Platform Design (PRD2)
description: "A reconstruction of PRD2, the design document the plugin pull requests #798 to #805 cite: its decisions, its sections and stack rows, what each pull request delivered or changed, what is still open, and the review outcomes of 2026-11-20."
sidebar:
  order: 4
---

:::caution[Reconstruction, for the owner's review]
PRD2 was never committed: it is not in the repository, an issue or a branch. This page rebuilds it from what cites it,
in the same numbering (§1, §A to §G, rows D-A to D-E, items such as G.11, decisions D1 to D6), so that the references
in the pull requests resolve. Everything here comes from the [sources](#source-index). Where they say nothing, or
disagree, the page says so; it adds no requirement of its own. Section titles in square brackets are inferred from the
items cited under them.
:::

**Status legend.** *Delivered (#n)*: implemented in pull request #n. *Deviated (#n)*: implemented differently, with
the reason the pull request gives. *Open*: not implemented. *Paused*: open, and held by the [review of
2026-11-20](#review-outcomes-2026-11-20). *Unknown*: cited without enough to reconstruct it. As of 2026-11-20 every
pull request of the stack is open, so "delivered" means "in the open pull request", not "on `main`".

## Purpose and scope

PRD2 is dated 2026-10-13. It is the second design round of the plugin platform behind the MCP server. The first
round ("PRD") produced [#799](https://github.com/zhang-accounting/zhang/pull/799), the MCP server as a Rust wasm
crate. PRD2 "replaced it with this script and added writes" ([DESIGN.md][design]). Its scope, as the pull requests
cite it:

- Lua scripts as router plugins, on the existing Extism/wasm boundary (§A);
- reuse of compiled modules (§A.5, row D-A);
- a generic write capability for router plugins, gated by the plugin and the ledger owner (§B);
- the MCP server rewritten as a Lua plugin with write tools (§C);
- distribution of that plugin inside the zhang binary (§F, G.11);
- the stack of pull requests that delivers all of this (§D).

Out of scope, as the sources state it: Lua processors and mappers (§A.4); write kinds other than `transaction` and
`open`, rate limiting and atomicity across several files (§B.4, §G.8, §G.12); MCP resources, prompts, SSE and
elicitation (DESIGN.md P10).

### Relation to #698

[RFC #698](https://github.com/zhang-accounting/zhang/issues/698) (2026-10-05) makes Budget the first complete official
plugin and keeps the core minimal: the core and host provide bookkeeping, exact amounts, generic queries and
controlled extension points, HTTP authentication, generic routing, and controlled persistence and reload
(受控持久化与重载); the business rules live in plugins. The execution boundary stays Extism/wasm, Lua is the first
script language, and amounts in plugins must never fall back to Lua or Python floats. PRD2 covers part of #698's
phase P1:

| #698 P1 item | Covered by |
|---|---|
| Parameterised BQL | [#798](https://github.com/zhang-accounting/zhang/pull/798), opened before PRD2 and reused by D4. It cites #698 P1, not PRD2. |
| Named queries, a query compile cache, typed reads of amounts, dates and metadata | Not addressed. |
| Adopt the verified compile reuse and script loading, keeping instances, contexts and permissions isolated | §A.5 / D-A ([#801][pr801]), §A / D-B ([#802][pr802]). |
| A read-only analysis / derived-data entry point, per-plugin derived snapshots, derived query tables with a schema, limits on derived computation, diagnostics mapping | Not addressed. |
| An exact-decimal ABI / SDK for plugins | Not addressed. It is the reason §A.4 keeps Lua out of processors and mappers. |

The MCP server is not a #698 item: #804 calls it "the MCP server the owner asked for without putting it into the
core". The sources do not say whether §B was meant as #698's "controlled persistence".

### History

- [#421](https://github.com/zhang-accounting/zhang/issues/421) / [#425](https://github.com/zhang-accounting/zhang/issues/425)
  built the stage pipeline, declarative capabilities and Router plugins. The owner's revised Decision 10 of #425 kept
  Router plugins read-only: "Writing to the ledger (e.g. a bank webhook that records a transaction) is out of scope
  for v1 and would need its own explicit capability." §B is that capability. #425 also listed "caching compiled
  modules across router requests" as a follow-up (D-A).
- The spike [`spike/extism-script-runtimes`][spike] (2026-10-05) ran Lua and Python interpreters as Extism modules.
  [PERFORMANCE.md][perf]: compiling the module costs 176 ms for Lua and 3,724 ms for Python, while an instance built
  from compiled code costs 0.16 ms and 1.08 ms; a JSON round trip of 10,000 transactions takes seconds, while a BQL
  summary over them takes 4 to 5 ms with a reused instance. Its conclusions: reuse compilation first; keep the
  plugin boundary to small requests and aggregated results; keep computation primitives and accounting invariants in
  Rust. COMPILE_CACHE.md (quoted in #698): a 10k-transaction BQL router call went from 35.72 ms to 8.89 ms median on
  Lua, 582.55 ms to 10.28 ms on Python.
- The pull requests also refer to working files that were not committed: paths under `/tmp/mcp/v2/` (#802, #804),
  "RULES" (#803), "the stack plan" (#801) and "the lead's brief" (#805).

## Decisions (§1)

[DESIGN.md][design] lists "the owner's (PRD2 §1)" decisions. No pull request body cites D1 to D6; the rationale
column quotes the sources where they give one.

| # | Decision | Rationale in the sources | Status |
|---|---|---|---|
| D1 | No wasm authored by users: plugins are Lua scripts; the execution boundary stays Extism/wasm, and the interpreter is the only wasm, embedded in zhang. | "Nothing to compile, nothing to install" (#802). #698: Lua is the first script language; the interface is not bound to a language. | Delivered for routers (#802). The stack keeps user-written wasm plugins working and extends the Rust SDK (#803); the sources do not say whether D1 means to retire them. |
| D2 | Lua, and writes. | Not stated. | Delivered (#802, #803, #804). |
| D3 | Amount arithmetic stays exact; never Lua floats; decimals travel as strings. | Lua has no decimal type and "would read `1234.56789012345678` as a floating-point number and write it back changed" (Lua guide, #802). "Owner ruling: floats never enter a write" (#803). | Delivered: routers only see decimal strings (#802); a JSON number in a write batch is refused (#803); MCP amounts are strings (#804). |
| D4 | The MCP server is a Lua plugin replacing #799, reusing #799's language-independent assets (BQL texts, tool catalogue, transport and era rules, review fixes, e2e sessions) and the parameterised-query host functions. | Not stated beyond the decision. | Delivered (#804); #799 closed on 2026-10-13. |
| D5 | Writes are a generic, capability-gated host path (`allowed_writes`), not MCP-specific. | "Any router plugin, wasm or Lua"; "this PR has value on its own" (#803). | Delivered (#803). |
| D6 | Standing rulings: ABI v1 additive only; business logic in the engine; nothing silently changes a balance or drops user text; no implicit account auto-open. | ABI additive dates from #425 Decision 1 ("every plugin compiled today keeps working unchanged"). "Business logic in the query engine, routes = BQL plus a thin mapping" (#798). No auto-open: "a hallucinated account name never creates an account silently" (DESIGN.md §5). | Delivered: the ABI stays at version 1 throughout the stack; the MCP plugin's built-in BQL is copied verbatim and pinned by a parity test; `open_account` is explicit. |

The numbers D1 to D6 are PRD2's own. #425 numbers its decisions 1 to 17 and #458 has its own D6; they are different
lists.

### Rulings made during implementation

| Ruling | By | Effect | Source |
|---|---|---|---|
| Floats never enter a write. | owner | A JSON number anywhere in a batch is `invalid`. | #803 |
| A module path without an extension keeps loading as wasm. | lead | Backward compatible; deviates from §A.1. | #802 |
| The `include` of a new file is written before the file. | lead | Overrides §B.3.6 and §G.8. | #803 |
| The release asset `zhang-mcp-<version>.lua` stays; the Docker image does not carry the script. | not stated ("Distribution ruling") | The Docker copy added in `d1c96246` is removed. | [`4a5e6e1e`][c-4a5e6e1e] |
| No rate limiting beyond one commit per reload. | owner | §G.12 stays out of scope (if G.12 is rate limiting, see below). | DESIGN.md §6 |

## §A [Lua router plugins]

"A `plugin "x.lua"` directive runs a Lua script as a router plugin (PRD2 §A)" (#802). Row D-B.

| Item | Intent | Status |
|---|---|---|
| A.1 | The kind of a module comes from its extension: `.lua` is a script, `.wasm` and `.wat` are modules; an unknown extension is refused before anything is read. | Delivered (#802). Deviated: a path without an extension still loads as wasm ("lead ruling: backward compatible"). #802 lists this as a deviation, which suggests §A.1 refused it; the sources do not say so outright. |
| A.2 | The interpreter is one wasm module, vendored with its checksum and rebuilt reproducibly from pinned sources. | Delivered (#802): Lua 5.4.8 and dkjson 2.11, 416,356 bytes, `tools/lua-runtime`, CI job `lua-runtime`; rebuilt at 420,406 bytes in #803. Deviated: the CI job runs on every pull request instead of a weekly schedule (it takes 27 s); the spike's binaryen and python-pdk pins are dropped as unused. #802 lists both as deviations without a section number; §A.2 is inferred. |
| A.3 | The Lua API: a module table `{name, version, types, router}`, `zhang.*` bridges to the host functions, `json`, parameter constructors; `zhang.read_file` / `zhang.list_dir` under `allowed_paths`; a 64 MiB memory budget per call, "the first memory bound in the plugin host" (§A.3, §G.3). | Delivered (#802). Deviated: `types` is optional and defaults to `{"Router"}` ("a 20-line plugin should not need it"); the file functions always raise `denied` because "a router has no file access (host rule, unchanged)". Open: a generic `memory` capability for every plugin (#802 follow-up). |
| A.4 | Lua scripts are router plugins only in this round; `Processor` and `Mapper` are refused, because directive amounts cross as JSON numbers. | Delivered (#802). Open: Lua processors and mappers with the exact-decimal bridge of #698 P1. |
| A.5 | The spike's compile cache, as its own pull request (D-A); the manifest is the cache key, so a script edit recompiles once per reload. | Delivered (#801, #802). Deviated: §A.5 "describes one `Arc<CompiledPlugin>` per registered plugin"; #801 keeps a pool of up to four idle compiled plugins and leases one per concurrent call, because "shared code would let one request's timeout fail a concurrent request of the same plugin". #801 compiled twice per load; #802 (`13a10dd4`) and #803 register with the full settings, so a load compiles once. |
| A.6 | A Lua fixture suite through the wasm test harness. | Delivered (#802): 21 tests (WAT twins, sandbox, JSON round trip, memory budget, syntax and shape errors, name collision, file access denied). |

#802 lists two more deviations without a section: a failed non-query host call raises `<kind>: <message>` instead of
the bare message, "so a `pcall` caller can tell `denied` from `not_found`"; a non-UTF-8 `body` is sent as base64
instead of being refused, "so a Lua router can serve an image".

## §B [The generic plugin write capability]

"Implements PRD2 §B (the generic plugin write capability), stack item D-C (§D)" (#803).

| Item | Intent | Status |
|---|---|---|
| B.1 | The grant: a multi-valued `allowed_writes` meta on the `plugin` directive (`transaction`, `open`); the plugin declares what it writes; both are needed. | Delivered (#803): the optional `capabilities` export (`writes` in a Lua module table), `writes_requested` and `writes_granted` in `GET /api/plugins`. |
| B.2 | Preview: the host validates a batch as `POST /api/transactions/preview` does, exports the text, chooses the file by `directive_output_path` and mints a token (HMAC over generation, plugin, expiry and plan; 10 minutes) only when every entry is clean. Bounds: 50 directives, 64 KiB of text, 1 MiB of input. | Delivered (#803). Deviated: (1) §B.2 "shows one `file` per batch"; each entry names its own file because a batch can span months. (2) §B.2 "lists `conflict` for commits only"; a preview also answers `conflict` when the ledger is stale or a write is pending, "so a plugin never holds a token it cannot use". (3) §B.2 "says an `open` with a different commodity set 'is an error'"; it is a field error on `commodities`, because "the ledger would accept the re-open; the refusal is the plugin path's". (5) A transaction's `datetime` may be a bare date, "which §B.2's example relies on"; the API accepts it too. |
| B.3 | Commit: re-plan, verify the token under a lock, append in one write, move the generation on, reload; one commit per reload cycle; error kinds `unavailable`, `denied`, `invalid_input`, `invalid`, `conflict`, `io`. | Delivered (#803). Changed in review: a failed write is recorded once and names what landed (`4fad54f0`); the commit marks the write pending itself instead of the route handler (#803 review item P7); the generation moves only when the reloaded content differs (`cc71b6ea`, `36aa0001`). Deviated (4, section not stated): an `open`'s `booking_method` is validated at preview. Deviated (6, section not stated): the registering instance gets the full manifest; "the PRD left registration as it was". |
| B.3.6 | Writes are atomic per file, and a new data file is written before the `include` of it. | Atomic save delivered (#803), and extended to `zhang serve`'s local-disk data source in [`35bab424`][c-35bab424] (temporary file plus rename, links and permissions kept). Deviated: the `include` is written first ("§B.3.6 and §G.8 ... are overridden by the lead"), because "a dangling `include` is visible and an un-included file is a silent duplicate on retry". |
| B.4 | [Write kinds beyond `transaction` and `open`] | Out of scope (#803 lists "`balance`/`note`/`pad` kinds, rate limiting, a write journal" against "§B.4, §G.8, §G.12" without pairing them; the kinds are most likely §B.4). Paused. |
| B.5 | [An audit log line for writes] | Partial: a plugin commit logs one line (#803). Open: "An audit line for API writes (`wrote` still logs nothing), per §B.5." |

#803's deviation list is numbered 1, 2, 7, 3, 4, 5, 6; the numbers above follow it.

## §C [zhang-mcp as a Lua plugin]

"Replaces #799 ... (PRD2 §C, §D row D-D)" (#804). The MCP server is `plugins/mcp/mcp.lua`, 12 read tools and 3
write tools.

| Item | Intent | Status |
|---|---|---|
| §C | The MCP server as one Lua script with #799's tools, BQL, transport and eras (D4). | Delivered (#804): 1,204 lines, "4 over the ≤ 1,200 target"; the sources do not say where that target is set. |
| C.2 | With `allow_writes` on, the three write tools are always listed; an ungranted kind answers an error naming the `allowed_writes` line to add. | Delivered after review ([`4618a2e2`][c-4618a2e2], "PRD2 C.2"). The first version (`d1c96246`) listed them only when a grant also existed. |
| C.2 | `preview_transaction` answers "the host's preview object". | Deviated (#804, DESIGN.md §7): the single entry's fields are lifted to the top level, "so the model never indexes `entries[0]` for the text it must show". |

Deviations "from PRD2" that #804 lists without a section: `add_transaction` leaves token verification to the host
alone (a token "embeds its expiry in seconds", so comparing previews refused any token older than a second); a posting
without `currency` uses the operating currency ("unstated in PRD2"); a write tool's `date` accepts an optional
`HH:MM:SS`.

## §D [Stack plan]

§D lists the pull requests of "the Lua/MCP stack" as rows. [#798](https://github.com/zhang-accounting/zhang/pull/798)
is below the stack: #801 calls D-A "the bottom of the Lua/MCP stack".

| Row | Pull request | Content | Status |
|---|---|---|---|
| — | [#798](https://github.com/zhang-accounting/zhang/pull/798) | `zhang_query_params` and `zhang_query_schema` for router plugins; decimal parameters bounded like ledger literals. | Delivered. Not a PRD2 row. |
| D-A | [#801][pr801] | Compile reuse (§A.5). | Delivered. Deviated: §D names it `reuse compiled wasm modules across calls`; the stack plan's title `reuse compiled plugin modules` is used. |
| D-B | [#802][pr802] | Lua router plugins (§A). | Delivered. |
| D-C | [#803][pr803] | Capability-gated writes (§B). | Delivered. |
| D-D | [#804][pr804] | zhang-mcp as a Lua plugin with write tools (§C). | Delivered. |
| D-E | [#805][pr805] | Bundling as `plugin "zhang:mcp"` (§F, G.11). | Delivered. Deviated: §D row D-E and the lead's brief ask for a `bundled` field in `GET /api/plugins`; it is not added, because it would change an exact-JSON listing test the pull request may not edit. Open (optional): `bundled: true` and a Settings badge. |

## §E

Unknown: no source cites §E, and its content is not stated in the sources.

## §F [Distribution]

| Item | Status |
|---|---|
| The MCP plugin ships inside zhang and a ledger declares `plugin "zhang:mcp"`, with no file to copy, on every data source; bundling grants nothing. | Delivered (#805). The case it serves: a ledger on R2 through opendal, where copying a file in "is the painful step" (DESIGN.md §6). |
| A release asset `zhang-mcp-<version>.lua` for customising the script. | Delivered (#804, `release.yml`). |
| A copy of the script in the Docker image. | Added in `d1c96246`, removed by the "Distribution ruling" in `4a5e6e1e`. Whether §F asked for it is not stated. |
| If zhang-core is ever published, `mcp.lua` must move into the crate. | Open, conditional (#805). |

## §G [Limits, risks and deferred items]

Only these items are cited. G.1, G.2, G.4 to G.7 and G.9 are unknown: the sources do not cite them.

| Item | Intent | Status |
|---|---|---|
| G.3 | A memory bound in the plugin host. | The Lua 64 MiB budget is "the first memory bound" (#802). Open: a generic `memory` capability. |
| G.8 | A new data file before its `include`; atomicity across two files. | Ordering overridden by the lead (see B.3.6). "A write journal for two-file atomicity" is out of scope (#803); the error of a partial write names what landed. |
| G.10 | `zhang.http` for Lua under `allowed_hosts`. | Open: "≈40 lines over `extism_http_request`" (#802). Paused. |
| G.11 | Bundling, done only once D-D is green. | Delivered (#805: "done because D-D is green"). #804 calls D-E "required"; whether G.11 made it conditional or required is not consistent in the sources. |
| G.12 | [Rate limiting, or the write journal] | Out of scope (#803). The pairing of #803's three items with §B.4, §G.8 and §G.12 is not stated; since §G.8 is tied to file ordering elsewhere, G.12 is most likely rate limiting. |

## Contradictions and gaps

1. **When the MCP write tools are listed.** #804's summary and DESIGN.md P8 say the write tools are "hidden unless
   both the plugin's `allow_writes` and the ledger's `allowed_writes` grant are on". #804's write-flow section,
   DESIGN.md §5, commit `4618a2e2` (citing PRD2 C.2) and the tests say they are listed whenever `allow_writes` is on.
   The code on the stack head follows the second; the summary and P8 are stale.
2. **D1 and user-written wasm.** D1 says "no wasm authored by users", while the stack keeps and extends wasm plugins
   and the Rust SDK. The sources do not say whether D1 limits official plugins only.
3. **G.11, conditional or required.** #805 says bundling was "done because D-D is green"; #804 calls D-E "required".
4. **Unpaired references.** #803 lists three out-of-scope items against §B.4, §G.8 and §G.12 without pairing them.
5. **Sections without a number.** #802's error-format and base64 deviations, #803's deviation 4, and #804's three
   deviations from PRD2 name no section.
6. **§E** and most of §G are never cited.
7. **The generation.** #803 first bumped the generation on every reload; the review made it content-based. The
   sources do not say what §B.2 or §B.3 specified.

## Review outcomes (2026-11-20)

The owner reviewed the stack on 2026-11-20 and decided:

1. **Security baseline on `main` first.** A security baseline lands on `main` before any pull request of the stack
   merges. Its content is not described on this page.
2. **An operator ceiling, `ZHANG_PLUGINS`**, with the values `off`, `read` and `write`, default `read`. The deployer
   sets what plugins may do at most, whatever a ledger grants: at the default, a ledger's `allowed_writes` grant does
   not turn writes on.
3. **Fix, then merge the stack in order**: #798, #801, #802, #803, #804, #805.
4. **Pause new plugin surface** until the #698 "Analyzer / derived-table" extension point and a `zhang.decimal` API
   are designed. Paused: `zhang.http` (§G.10), more write kinds (§B.4) and MCP resources (#804 follow-up). The
   `zhang.decimal(v)` of #802 is a query-parameter constructor, not the exact-decimal API meant here.

## Source index

| Item | Source |
|---|---|
| Date, the two rounds, §1, D1 to D6, P8, §F, C.2 deviation, rate limiting | [plugins/mcp/DESIGN.md][design] (§1, §5, §6, §7) on the stack head |
| #698 scope, P1 items, spike figures | [#698](https://github.com/zhang-accounting/zhang/issues/698) |
| Router read-only, writes need their own capability, compiled-module follow-up | [#425](https://github.com/zhang-accounting/zhang/issues/425) (comments of 2026-10-02 and 2026-10-03), [#421](https://github.com/zhang-accounting/zhang/issues/421) |
| Compile and JSON costs | [PERFORMANCE.md][perf] on [`spike/extism-script-runtimes`][spike] |
| §A, A.1 to A.6, G.3, G.10 | [#802][pr802] (body: What, Design, The interpreter module, Tests, Deviation, Follow-ups); commits [`6cdb848b`][c-6cdb848b] ("PRD2 A.4"), [`47c5d139`][c-47c5d139] ("A.6"), [`13a10dd4`][c-13a10dd4] |
| A.5, row D-A | [#801][pr801] (body: What and why, Design, Deviation from PRD2, Follow-ups) |
| §B, B.1 to B.5, G.8, G.12, row D-C, owner and lead rulings | [#803][pr803] (body: What, Design decisions, After the fresh-eyes review, Deviation from PRD2, Follow-ups); commits [`ece2c1f3`][c-ece2c1f3], [`4fad54f0`][c-4fad54f0], [`cc71b6ea`][c-cc71b6ea], [`36aa0001`][c-36aa0001], [`35bab424`][c-35bab424] |
| §C, C.2, row D-D | [#804][pr804] (body: What it is, The write flow, Deviations, Follow-ups); commits [`d1c96246`][c-d1c96246], [`4618a2e2`][c-4618a2e2] ("PRD2 C.2"), [`4a5e6e1e`][c-4a5e6e1e] (distribution ruling) |
| Row D-E, §F, G.11 | [#805][pr805] (body: first line, Design, Deviations) |
| #798 below the stack, D4's host functions | [#798](https://github.com/zhang-accounting/zhang/pull/798), [#801][pr801], [#804][pr804] |
| First round (PRD) | [#799](https://github.com/zhang-accounting/zhang/pull/799) ("Deviations from the PRD", closing comment) |
| Review outcomes | The owner's review of 2026-11-20 |

None of the six pull requests has comments or reviews on GitHub; the deviations come from their bodies and commit
messages.

[design]: https://github.com/zhang-accounting/zhang/blob/e47a36b7bf18229348c92b92c61d86033dfd703c/plugins/mcp/DESIGN.md
[spike]: https://github.com/zhang-accounting/zhang/tree/fcfc90583c762ab0e51bea5d63a294408cd354f8/experiments/extism-scripts
[perf]: https://github.com/zhang-accounting/zhang/blob/fcfc90583c762ab0e51bea5d63a294408cd354f8/experiments/extism-scripts/PERFORMANCE.md
[pr801]: https://github.com/zhang-accounting/zhang/pull/801
[pr802]: https://github.com/zhang-accounting/zhang/pull/802
[pr803]: https://github.com/zhang-accounting/zhang/pull/803
[pr804]: https://github.com/zhang-accounting/zhang/pull/804
[pr805]: https://github.com/zhang-accounting/zhang/pull/805
[c-4a5e6e1e]: https://github.com/zhang-accounting/zhang/commit/4a5e6e1edd0621d9b3946dcfd36af2c4e6ab0be9
[c-35bab424]: https://github.com/zhang-accounting/zhang/commit/35bab424a4189d53b29e25219baf55f1cb15df68
[c-4618a2e2]: https://github.com/zhang-accounting/zhang/commit/4618a2e27250c802c4256cf61580f460d7daa9d9
[c-d1c96246]: https://github.com/zhang-accounting/zhang/commit/d1c9624642ca3524bba3871567eae167743a3b50
[c-6cdb848b]: https://github.com/zhang-accounting/zhang/commit/6cdb848ba606feeb79c54b104af9b0202993cefa
[c-47c5d139]: https://github.com/zhang-accounting/zhang/commit/47c5d139fe7b812ad16507583c0ae91b3e02b7ac
[c-13a10dd4]: https://github.com/zhang-accounting/zhang/commit/13a10dd4804b7f33fb2e8f9a8a481ac9079a3138
[c-ece2c1f3]: https://github.com/zhang-accounting/zhang/commit/ece2c1f3dd2a8d89629e701223eb102822d8eb12
[c-4fad54f0]: https://github.com/zhang-accounting/zhang/commit/4fad54f026db159234e3673f10fcceb52204de22
[c-cc71b6ea]: https://github.com/zhang-accounting/zhang/commit/cc71b6ea39080fa27f47349622ace0eb2708839f
[c-36aa0001]: https://github.com/zhang-accounting/zhang/commit/36aa0001ad28b687daed218a32493f687bb22fe5
