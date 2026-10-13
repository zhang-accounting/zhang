# zhang-mcp: design

`zhang-mcp` is an [MCP](https://modelcontextprotocol.io/) server for a zhang ledger, written as one Lua script
(`plugins/mcp/mcp.lua`) on zhang's Lua plugin runtime. It follows #698: the core stays generic, the business
organisation of the API lives in the plugin, and the plugin only maps tools onto BQL that the query engine runs and
onto the host's generic write capability. This document condenses the two design rounds (PRD, 2026-10-13, which
produced the wasm crate of PR #799; PRD2, 2026-10-13, which replaced it with this script and added writes) to their
decisions, the rules the implementation was checked against, the tool catalogue, the write flow and the deviations.
The user guide is `docs/src/content/docs/guides/mcp.md`.

Specification references: `S` = https://modelcontextprotocol.io/specification/2026-07-28 (the current revision),
`L` = https://modelcontextprotocol.io/specification/2025-11-25 (the legacy revision today's clients speak), `schema` =
https://raw.githubusercontent.com/modelcontextprotocol/modelcontextprotocol/main/schema/2026-07-28/schema.ts.

## 1. Decisions

The owner's (PRD2 §1):

| # | Decision | Where it binds |
| --- | --- | --- |
| D1 | No wasm authored by users: plugins are Lua scripts; the execution boundary stays Extism/wasm (the interpreter is the only wasm, embedded in zhang). | one file, `mcp.lua`, bundled in zhang as `plugin "zhang:mcp"` (or copied into the ledger); nothing to build, nothing to install |
| D2 | Lua, **and** writes. | §5 |
| D3 | Amount arithmetic stays exact; never Lua floats; decimals travel as strings. | every amount in a tool argument or result is a string; a number is refused before the host sees it |
| D4 | The MCP server is a Lua plugin replacing #799; it reuses #799's language-independent assets (BQL texts, tool catalogue, transport and era rules, review fixes, e2e sessions) and the parameterised-query host functions. | §2-4 |
| D5 | Writes are a generic, capability-gated host path (`allowed_writes`), not MCP-specific. | §5: the plugin calls `zhang.write.preview`/`zhang.write.commit`, the host validates, chooses the file, appends and reloads |
| D6 | Standing rulings: ABI v1 additive only; business logic in the engine; nothing silently changes a balance or drops user text; no implicit account auto-open. | §4 (BQL verbatim), §5 (strings, explicit `open_account`) |

The plugin's own (carried from #799 where still true):

| # | Decision | Why |
| --- | --- | --- |
| P1 | Name `zhang-mcp`; the MCP endpoint is `POST /api/plugins/zhang-mcp/mcp`; `GET /api/plugins/zhang-mcp/` answers a JSON description with the endpoint path; `GET /api/plugins/zhang-mcp/queries` lists the BQL (for the parity test); other paths are `404`. | The Settings page links to a router plugin's root: a user who opens it sees how to connect. The hidden `queries` route lets a Rust test compare texts without parsing Lua. |
| P2 | One JSON object per request, never SSE, never a session. | A plugin call is one buffered request/response; both revisions allow `application/json` answers (§2). |
| P3 | Dual-era in one plugin: the legacy `initialize` lifecycle (2025-03-26, 2025-06-18, 2025-11-25) and the stateless 2026-07-28 revision. | Today's clients open with `initialize`; the current revision needs per-request `_meta` and header validation. A fresh Lua state per call makes statelessness the only option, and the right one. |
| P4 | Twelve read tools; every BQL with a built-in equivalent is copied verbatim from `zhang-server/src/builtin.rs` and pinned by a parity test; the others are written in the same style. User input is only bound as a parameter. | The MCP answers are the app's answers; the owner's rule for routes (BQL plus a thin mapping) applies to plugins. |
| P5 | Tool results: `structuredContent` with the query API's cell encoding, plus the same JSON in a text block, plus an `outputSchema`. Tool execution errors are `isError: true` with a text block only. | S/server/tools: structured content SHOULD be mirrored as text; with an `outputSchema` the structured result MUST conform, so an error object must not be sent as `structuredContent`. |
| P6 | Authentication is zhang's; no OAuth. `Origin`: absent → allowed; listed in `allowed_origins` → allowed; equal to the request's own `Host` → allowed only when that host is loopback; `null` or anything else → `403`. | S/basic/authorization makes authorization optional; the DNS rebinding rule of both transports, with "same host" safe for loopback names only. |
| P7 | `limit` defaults to 200; one over `max_rows` (default 1000) is a tool error naming the cap; tools with `LIMIT :limit` fetch one row more than the limit so `truncated` is exact. | Keeps a result within a model's context; the host's value cap stays the hard bound. |
| P8 | Three write tools, hidden unless both the plugin's `allow_writes` and the host's `allowed_writes` grant are on; `add_transaction` is two-step (preview token), `open_account` one-shot. | §5. |
| P9 | Every failure inside a tool, a host error included, is an `isError` result; nothing a tool does becomes an HTTP 500. | A model can act on a tool error; a 500 is a transport error it cannot read. |
| P10 | Resources, prompts, SSE, elicitation, rate limiting: out of scope. | Same as #799; elicitation needs server→client requests this transport cannot carry. |

## 2. Transport (`mcp.lua` section `transport`)

| Request | Response | Rule |
| --- | --- | --- |
| `Origin` present and not allowed (P6) | `403`, JSON-RPC error without `id` | S transport "Security & Endpoint": "If the `Origin` header is present and invalid, servers MUST respond with HTTP 403 Forbidden"; the same in L. |
| `GET`, `DELETE`, any non-POST on `/mcp` | `405`, `Allow: POST` | S "Earlier Streamable HTTP Revisions"; L "Listening for Messages from the Server" 3. |
| body over 1 MiB | `413`, `-32600` without id | the plugin's own bound. |
| body not JSON | `400`, `-32700` | JSON-RPC 2.0. |
| array (batch), no `"jsonrpc": "2.0"`, `id: null`, a numeric id that is not an integer within 64 bits, params not an object, neither request nor notification nor response | `400`, `-32600` (with the id when it was readable) | S/basic "Requests": the ID MUST NOT be null, "a string or integer ID"; batching was removed in 2025-06-18. |
| notification, or a response posted by the client | `202`, empty body | S "Sending Messages" 5; L 4. |
| request | `200 application/json` with the result, or an error as §3 says | S 6 / L 5. |
| `Accept` | not checked | both revisions constrain the client only. |

Every answer carries `Cache-Control: no-store` and `X-Content-Type-Options: nosniff`, and `Content-Type: application/json`
except the `202` (the host adds its default `text/plain; charset=utf-8` there, see §7). A transport-level error omits
the `id` member rather than sending `null` (the 2026-07-28 schema's `JSONRPCErrorResponse` has `id?: RequestId`).

## 3. Eras (`mcp.lua` section `era`)

Detection: `initialize` is always legacy; otherwise a request is modern when
`params._meta["io.modelcontextprotocol/protocolVersion"]` is present or the `MCP-Protocol-Version` header is
`2026-07-28`; everything else is legacy. Stateless in both.

**Legacy.** No `MCP-Protocol-Version` → served as 2025-03-26; present and unsupported → `400` + `-32022` with
`data.supported`. `initialize` → `{protocolVersion, capabilities: {tools: {}}, serverInfo, instructions}`, the
requested version echoed when it is a supported legacy one, else `2025-11-25` (`2026-07-28` in `initialize` is
answered with `2025-11-25`: `initialize` means legacy semantics). `ping` → `{}`. `tools/list`, `tools/call` as §4;
any other method → `200` with `-32601`. No `Mcp-Session-Id`.

**Modern.** Validation in this order, each failure a `400`: (1) `_meta` protocol version missing → `-32602`;
(2) `MCP-Protocol-Version` header missing or different → `-32020`; (3) version unsupported → `-32022`;
(4) `Mcp-Method` missing or different → `-32020`; (5) for `tools/call`, `prompts/get` (`params.name`) and
`resources/read` (`params.uri`): `Mcp-Name` missing or different after decoding the `=?base64?…?=` sentinel →
`-32020`; (6) `_meta` client capabilities missing or not an object → `-32602`. Then `server/discover` →
`{supportedVersions, capabilities: {tools: {}}, instructions, ttlMs: 3600000, cacheScope: "private"}` (the
instructions name the ledger), `tools/list` with `ttlMs: 3600000, cacheScope: "public"`, `tools/call` as §4, any other
method → `404` + `-32601`. Every result gets `resultType: "complete"` and `_meta["io.modelcontextprotocol/serverInfo"]`.

HTTP status of JSON-RPC errors: `400`/`404` where the specification names them; errors a handler produces (unknown
tool, malformed call) are `200` with the error body, in both eras. Codes used: `-32700`, `-32600`, `-32601`,
`-32602`, `-32020`, `-32022`.

## 4. Tools (`mcp.lua` sections `catalogue`, `queries`, `shape`, `read tools`, `write tools`)

Conventions: names of `[A-Za-z0-9_]`; `inputSchema` generated from the argument declarations with
`additionalProperties: false`; dates `YYYY-MM-DD`; `limit` default 200, capped by `max_rows`. A rows tool answers
`{columns: [{name, type}], rows: [[cell…]], row_count, truncated}` with the cells of `POST /api/query`, as text and as
`structuredContent`, described by its `outputSchema`.

Errors: an unknown tool, or a call that is not a `CallToolRequest` (`params.name` missing or not a string,
`arguments` present but not an object) → `-32602`. An argument that fails the tool's own schema (unknown or missing
argument, wrong type, a date not `YYYY-MM-DD`, `limit` under 1 or over `max_rows`, an unknown `group_by`, `from`
after `to`, a numeric amount, an empty `bql`) → `isError: true` with a text naming the argument and what it expects
(S/server/tools "Error Handling", SEP-1303). A failing query, a refused write or an unavailable host function →
`isError: true` with `query error at line L, column C: …`, `<kind>: <message>` or the write tool's own sentence, and no
`structuredContent` (P5).

| Tool | Arguments | Runs | Parity | Annotations |
| --- | --- | --- | --- | --- |
| `ledger_info` | — | `zhang.ledger_info` + `zhang.now` → `{title, operating_currency, timezone, now, today}` | — | read-only |
| `schema` | — | `zhang.query_schema` | the payload of `GET /api/query/schema` | read-only |
| `query` | `bql` (required), `limit` | `zhang.query(bql)`; rows cut at `limit` | — | read-only |
| `accounts` | `prefix`, `status` (`open`/`closed`/`all`, default `open`) | `accounts.list` (no prefix) or `accounts.subtree`, `:date`/`:time` = now; rows whose engine-computed `status` is not the asked one are dropped | builtin | read-only |
| `balances` | `account` | `accounts.balances` or `accounts.subtree_balances`, `:operating_currency` from `ledger_info` | builtin | read-only |
| `net_worth` | `date` (default today), `currency` (default operating) | `report.net_worth` | builtin | read-only |
| `spending` | `account` (default `Expenses`), `from`, `to` (required), `group_by` (`account`/`month`/`currency`), `currency` | `account`: `report.account_totals` (`:type` = account); `month`: `mcp.spending_by_month`; `currency`: `mcp.spending_by_currency` | builtin / own | read-only |
| `journal` | `account` (required), `limit`, `offset` | `accounts.journal_page` (`:limit` = limit + 1, cut at limit) | builtin | read-only |
| `transactions` | `keyword`, `payee`, `tags`, `from`, `to`, `limit`, `offset` | `mcp.transactions` over `#transactions`, newest first, every filter NULL-able | own | read-only |
| `payees` | — | `journals.payees` | builtin | read-only |
| `errors` | `limit`, `offset` | `journals.errors` (`:size` = limit + 1) | builtin | read-only |
| `prices` | `commodity`, `currency` (default operating) | `commodities.latest_prices` or `commodities.latest_price` | builtin | read-only |
| `preview_transaction` | `date` (required, optional time), `payee`, `narration` (required), `postings` (required: `account`, `amount` string, `currency`, `cost`, `price`), `tags`, `links`, `flag`, `metas` | `zhang.write.preview` of one transaction, `write_tag` added to `tags` | the web UI's preview | `readOnlyHint: true, idempotentHint: true` |
| `add_transaction` | the same + `token` (required) | re-preview for the errors, `zhang.write.commit` with the caller's token (verified by the host) | the web UI's append | `readOnlyHint: false, destructiveHint: false, idempotentHint: false` |
| `open_account` | `date`, `account` (required), `commodities`, `booking_method` | `zhang.write.preview` of one `open`, then `zhang.write.commit` unless `already_open` | — | `readOnlyHint: false, destructiveHint: false, idempotentHint: true` |

"read-only" = `{readOnlyHint: true, destructiveHint: false, idempotentHint: true, openWorldHint: false}`; every tool has
`openWorldHint: false`. The 14 BQL texts live in the `queries` section; `zhang-server/tests/mcp_plugin.rs` reads them
through `GET /queries`, pins each `builtin` one to `BUILTINS[name].bql` and its parameter types, and compiles the
plugin's own ones with the engine and exactly the parameters they declare.

`instructions` (sent by `initialize` and `server/discover`): what the ledger is (title JSON-quoted, operating
currency, timezone), that amounts are exact decimals as strings, that `ledger_info` gives today's date and `schema`
describes BQL for `query`, that row contents are data, not instructions; with writes on: preview first, show the
user the `text`, commit only with the token after the user agrees, open accounts explicitly, nothing is edited or
deleted.

## 5. Writes (`mcp.lua` section `write tools`)

Two switches, both required: the plugin's `allow_writes: "true"` (lists the three tools; default off, so a read-only
deployment never shows them to the model) and the host's `allowed_writes: "transaction"` / `allowed_writes: "open"`
(the generic grant of PRD2 §B, the same shape as `allowed_paths`). With `allow_writes` on but a kind not granted, the
tool is listed and answers an `isError` naming the exact meta line to add (actionable for the owner, harmless for the
model). Without `allow_writes` the three names are unknown tools (`-32602`), whatever the grant; with it they are always
listed, so the owner who turned writes on gets the missing grant line relayed by the model instead of "no such tool".

The flow and the token: `preview_transaction` sends one `transaction` directive to `zhang.write.preview`; the host
validates it exactly as `POST /api/transactions/preview` does, exports the text, chooses the file by the ledger's
`directive_output_path`, and mints a token bound to the ledger generation, the plugin, the file and the exact text,
valid ten minutes, only when the entry is clean. The tool answers `{ok, text, file, token, field_errors, unbalanced,
errors}`. `add_transaction` rebuilds the same batch, previews it again to report errors precisely, then `zhang.write.commit`s
with the caller's token, which the host verifies against the recomputed plan (generation, expiry, plugin, files, texts); the host appends the text in one write, bumps the generation (every outstanding
token dies, replay is impossible) and reloads after the plugin returns. A host `conflict` (the ledger changed, a
reload is pending, the token expired or was used) is an `isError` that says "the ledger changed, run
preview_transaction again". `open_account` is one-shot (an `open` changes no balance): preview, then commit with the preview's token. A ledger
keeps one `open` per account, and the host lets a plugin open only an account the ledger never opened: the same
commodity set answers `recorded: false, already_open: true` with `opened_on` and a message telling the user to move
that open's date in the file to record earlier transactions (the plugin reads the date through `accounts.subtree`);
another commodity set is the host's field error naming the existing open, and a closed account the host's
`account_closed` error ("was closed on <date>: a plugin does not reopen a closed account"), both relayed as the tool's error. The instructions tell
the model to date a new open on or before the earliest transaction that needs it, not today.

Confirmation design (end-user-safe default): (1) the client's approval prompt, driven by the annotations
(`readOnlyHint: false` on `add_transaction` and `open_account`; Claude Code and Claude Desktop prompt for non-read-only
tools); (2) the token: the model cannot commit what it did not preview, or on a ledger that changed since;
(3) the `instructions` tell the model to show the preview's `text` before committing; (4) the two server-side
switches. MCP elicitation is not used: it needs server→client requests, which the one-JSON-per-POST transport cannot
carry. Provenance: `write_tag` (default `mcp`, `""` disables) is added by the plugin to the batch the user sees, not
injected by the host (D6). No auto-open: a hallucinated account name never creates an account silently; the preview
reports `AccountDoesNotExist`, the model calls `open_account` (approved by the user), then previews again.

Amounts: strings in the schema, passed through untouched to the host's `{number, commodity}`; a number is refused
with "must be a decimal string such as \"4.50\", never a number: decimals stay exact". A posting without `amount` is
the inferred-amount posting; one without `currency` takes the operating currency.

## 6. Configuration, limits, security

```zhang
option "features.plugin" "true"
plugin "zhang:mcp"                                ; the bundled copy; a copy in the ledger is declared by its path
  timeout: "20s"                                  ; the host's: how long one request may run (default 60s)
  allowed_origins: "https://ledger.example.com"   ; comma-separated or repeated; absent = loopback same-host only
  max_rows: "500"                                 ; the cap of every tool's limit; default 1000
  allow_writes: "true"                            ; list the write tools; default false
  write_tag: "mcp"                                ; the tag add_transaction adds; "" disables
  allowed_writes: "transaction"                   ; the host's grant
  allowed_writes: "open"
```

Distribution (PRD2 §F, D-E): the script is bundled in the zhang binary and declared as `plugin "zhang:mcp"`. A
`zhang:` module resolves to an entry of the host's small table of bundled plugins (`zhang-core/src/plugin/bundled.rs`:
name → the repository file, embedded with `include_bytes!` under `plugin_runtime`) instead of a file read through the
ledger's data source, so the install is the same two lines on a local disk and on remote storage (the owner's ledger
is on R2 through opendal, where copying a file in is the painful step). The bundled copy is this file unchanged: the
same identity (`zhang-mcp`), the same metas, the same grant (`allowed_writes` on the directive; bundling grants
nothing); it is no `ExtraInput` of the load and its version moves with the binary. Messages and `zhang.script_name`
name it `zhang:mcp`. A copy declared by its path (`plugin "plugins/mcp.lua"`) keeps working and is the way to
customise it; the release asset `zhang-mcp-<version>.lua` stays for that.

Repeated keys are read from `zhang.plugin` (the directive as JSON), so `allowed_origins` and `allowed_writes` may be
repeated; the flat config is the fallback. Bounds: the plugin `timeout`; 10 s per BQL and the host's value cap; the
64 MiB Lua budget per call; `limit`/`max_rows`; request bodies ≤ 1 MiB; one transaction or one `open` per write
call; the host's ≤ 50 directives / ≤ 64 KiB per batch; token TTL 10 min; one commit per reload cycle.

Security: read tools are read-only by construction; writes are additive only, validated like the web UI's, into a
file the ledger's policy chooses, bound by the token to the previewed text, tagged, and logged by the host. The
credential never enters the sandbox; the plugin declares no `allowed_hosts` and no `allowed_paths`. Injection: every
argument is bound as a typed parameter; the `query` tool runs arbitrary BQL by design in the read-only, bounded
engine. Prompt injection: payees, narrations and metadata reach the model as data; the instructions say so; the
ledger title inside them is JSON-quoted. `Origin` (P6), `no-store`, `nosniff`. Rate limiting: none beyond one commit
per reload (owner decision).

## 7. Deviations

From PRD2:

- `preview_transaction` lifts the single entry's fields (`text`, `field_errors`, `unbalanced`, `errors`) to the top
  level of its result beside `ok`, `file`, `token` (PRD2 §C.2 said "the host's preview object"): one transaction per
  call, so the model never indexes `entries[0]` to find the text it must show.
- `add_transaction` previews again only to name the ledger's errors precisely; the token itself is verified by the
  host alone on commit (a token embeds its expiry in seconds, so two previews of the same text in different seconds
  mint different tokens: a plugin-side comparison would refuse any token older than a second, the normal case for a
  model). The host's `conflict` is turned into a distinct remedy: an expired token ("the preview expired, run
  preview_transaction again"), other arguments ("the arguments differ from the transaction this token was previewed
  for"), a token the server never issued, or a changed ledger ("the ledger changed since the preview").
- A posting without `currency` is in the operating currency (unstated in PRD2; the web UI does the same).
- `date` of a write tool accepts an optional `HH:MM:SS`.

From #799 (the wasm crate this script replaces), besides the language:

- A numeric request id must be an integer within the signed 64-bit range (Lua integers); #799 also accepted
  `u64::MAX`. Such ids do not occur in practice and are refused with `-32600`.
- The empty `202` carries the host's default `content-type: text/plain; charset=utf-8`: a router response without a
  content type gets one from the host, and a plugin cannot suppress it (#799's open nit, unfixable plugin-side).
- Fixed from #799's open nits: `params` that are not an object still echo the request id; `limit: 5.0` is accepted as
  5; an empty `bql` is refused in the plugin; `limit + 1` saturates; `_meta` client capabilities must be an object.
- The parity test reads the BQL through `GET /queries` instead of linking the crate.
- Unit tests of the Rust crate (44) are not ported one-to-one: the e2e sessions carry their assertions where they
  mattered (framing, Origin, eras, schema errors, truncation), and the rest was Rust-side mechanics.

Carried from #799's first round (still true): endpoint at `/api/plugins/zhang-mcp/mcp` with a description page at
`/`; `net_worth` and `spending` by account reuse `report.net_worth` and `report.account_totals`; `transactions` reads
`#transactions` and has `keyword` and `offset`; `spending` requires `from` and `to`; `accounts` applies `status` on
the engine's column; `LIMIT`-bound tools fetch one extra row; a legacy request with an unsupported version header gets
`-32022` with the supported list; error responses carry the request id whenever it was parsed and omit it (not `null`)
for transport-level errors; handler-level JSON-RPC errors are HTTP 200 in both eras; schema failures are `isError`
results; `server/discover` is `cacheScope: "private"`; the same-host `Origin` rule applies to loopback hosts only.

## 8. Follow-ups

- Resources (`zhang://schema`, `zhang://errors`) and prompts (`monthly-review`, `bql-help`).
- A body-size cap per plugin in the host (the 1 MiB check runs after the host buffered the body).
- Rate limiting in the host, if a shared deployment needs it.
- More write kinds (`balance`, `note`) once the host's capability grows; editing stays out by design.
