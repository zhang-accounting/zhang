# zhang-mcp: design

`zhang-mcp` is an [MCP](https://modelcontextprotocol.io/) server for a zhang ledger, built as a router plugin
(`plugins/mcp`, crate `zhang-plugin-mcp`). It follows #698: the core stays generic, the business organisation of the
API lives in the plugin, and the plugin only maps tools onto BQL that the query engine runs. This document condenses
the design study (the PRD of 2026-10-13) to its decisions, the specification rules the implementation was checked
against, and the tool catalogue. The user guide is `docs/src/content/docs/guides/mcp.md`.

Specification references: `S` = https://modelcontextprotocol.io/specification/2026-07-28 (the current revision),
`L` = https://modelcontextprotocol.io/specification/2025-11-25 (the legacy revision today's clients speak), `schema` =
https://raw.githubusercontent.com/modelcontextprotocol/modelcontextprotocol/main/schema/2026-07-28/schema.ts. Every
rule below was read on these pages on 2026-10-13.

## 1. Decisions

| # | Decision | Why |
| --- | --- | --- |
| D1 | A router plugin, no MCP code in the core. The plugin imports only `zhang_query_params`, `zhang_query_schema`, `zhang_ledger_info` and `zhang_now` (and `zhang_query` for the `query` tool). | #698: host interfaces stay generic; the parameterised query and the schema are on its P1 list and useful to any plugin. |
| D2 | Plugin name `zhang-mcp`; the MCP endpoint is `POST /api/plugins/zhang-mcp/mcp`; `GET /api/plugins/zhang-mcp/` answers a JSON description with the endpoint path; other paths are `404`. | The Settings page links to a router plugin's root: a user who opens it sees how to connect. (Deviation from PRD, which put the endpoint at the root.) |
| D3 | One JSON object per request, never SSE, never a session (`Mcp-Session-Id` is neither minted nor read, `Last-Event-ID` is ignored). | A plugin call is one buffered request/response; both revisions allow `application/json` answers (§2). |
| D4 | Dual-era in one plugin: the legacy `initialize` lifecycle (2025-03-26, 2025-06-18, 2025-11-25) and the stateless 2026-07-28 revision. | Today's clients open with `initialize`; the current revision needs per-request `_meta` and header validation; a dual-era server serves both (S/basic/versioning, compatibility matrix). |
| D5 | Twelve read-only tools; every BQL with a built-in equivalent is copied verbatim from `zhang-server/src/builtin.rs` and pinned by a parity test; the others are written in the same style. User input is only bound as a parameter. | The MCP answers are the app's answers; the owner's rule for routes (BQL plus a thin mapping) applies to plugins. |
| D6 | Tool results: `structuredContent` with the query API's cell encoding, plus the same JSON in a text block, plus an `outputSchema`. Tool execution errors are `isError: true` with a text block only. | S/server/tools (§4): structured content SHOULD be mirrored as text; with an `outputSchema` the structured result MUST conform, so an error object must not be sent as `structuredContent` (the TypeScript SDK validates it even on errors). |
| D7 | Authentication is zhang's (`Basic`, `Bearer`, cookie); no OAuth, no `WWW-Authenticate`. | S/basic/authorization: authorization is OPTIONAL and custom strategies MAY be used; zhang's credentials are issued for zhang. |
| D8 | `Origin`: absent → allowed; listed in `allowed_origins` (exact full origin) → allowed; the same `host[:port]` as the request's `Host` → allowed only when that host is this machine (`localhost`, 127.0.0.0/8, `[::1]`); `null` or anything else → `403`. | The DNS rebinding rule of both transports (§2). A rebinding attack is a `Host` the attacker's DNS points at this server, so "same host" is only safe for loopback names; a deployment behind a proxy or on a LAN address lists its origin. |
| D9 | `limit` defaults to 200; one over `max_rows` (default 1000) is a tool error naming the cap; tools with `LIMIT :limit` fetch one row more than the limit so `truncated` is exact. | Keeps a result within a model's context; the host's value cap stays the hard bound. |
| D10 | Writes, resources, prompts, SSE, rate limiting, budget tools: out of scope (§7). | Read-only MVP first; writes need a generic host capability (G4). |

## 2. Transport (`src/transport.rs`)

| Request | Response | Rule |
| --- | --- | --- |
| `Origin` present and not allowed (D8) | `403`, JSON-RPC error without `id` | S transport "Security & Endpoint" 1: "If the `Origin` header is present and invalid, servers MUST respond with HTTP 403 Forbidden. The HTTP response body MAY comprise a JSON-RPC error response that has no `id`"; the same text in L/basic/transports "Security Warning". |
| `GET`, `DELETE`, any non-POST on `/mcp` | `405`, `Allow: POST` | S "Earlier Streamable HTTP Revisions": "HTTP GET or DELETE to the MCP endpoint: respond with `405 Method Not Allowed`"; L "Listening for Messages from the Server" 3: "or else return HTTP 405 Method Not Allowed, indicating that the server does not offer an SSE stream". |
| body over 1 MiB | `413`, JSON-RPC `-32600` without id | the plugin's own bound; JSON-RPC requests are small. |
| body not JSON | `400`, `-32700` | JSON-RPC 2.0. |
| array (batch), no `"jsonrpc": "2.0"`, `id: null`, a numeric id that is not an integer within 64 bits (a fraction, an exponent, a 20-digit number), params not an object, neither request nor notification nor response | `400`, `-32600` | S/basic "Requests": "the ID MUST NOT be `null`", "a string or integer ID"; batching was removed in 2025-06-18. An id is echoed exactly as read, which only an integer within 64 bits survives without `arbitrary_precision`. |
| notification, or a response posted by the client | `202`, empty body | S "Sending Messages" 5: "If the body is a JSON-RPC notification: If the server accepts it, the server MUST return HTTP status code `202 Accepted` with no body"; L 4: "If the input is a JSON-RPC response or notification: If the server accepts the input, the server MUST return HTTP status code 202 Accepted with no body". |
| request | `200 application/json` with the result, or an error as §3 says | S 6 / L 5: the server returns "`Content-Type: application/json` (a single JSON object)"; "The client MUST support both". |
| `Accept` | not checked | both revisions constrain the client only. |

Every answer carries `Content-Type: application/json` (not the `202`), `Cache-Control: no-store` and
`X-Content-Type-Options: nosniff`. A transport-level error (403, 405, 413, a body that is not one message) has no
`id` member rather than `"id": null`: the 2026-07-28 schema's `JSONRPCErrorResponse` declares `id?: RequestId`
(`schema.ts` lines 298-302), and the transport pages say "a JSON-RPC error response that has no `id`". A legacy
client's SDK ignores a response it cannot correlate either way.

## 3. Eras (`src/era.rs`)

Detection: `initialize` is always legacy (S/basic/versioning: "An `initialize` request selects legacy semantics");
otherwise a request is modern when `params._meta["io.modelcontextprotocol/protocolVersion"]` is present ("A request
carrying modern per-request `_meta` is served statelessly according to this revision") or when the
`MCP-Protocol-Version` header is `2026-07-28`; everything else is legacy. The server is stateless in both eras.

### Legacy (2025-03-26 … 2025-11-25)

- `MCP-Protocol-Version` absent → served as 2025-03-26 (L "Protocol Version Header": "the server SHOULD assume protocol
  version `2025-03-26`"); present and not supported → `400` ("it MUST respond with `400 Bad Request`"), with `-32022`
  and `data.supported` so that the client learns the list.
- `initialize` → `{protocolVersion, capabilities: {tools: {}}, serverInfo: {name: "zhang-mcp", title, version},
  instructions}`. The requested version is echoed when it is a supported legacy one, else `2025-11-25` (L/basic/lifecycle
  "Version Negotiation": "If the server supports the requested protocol version, it MUST respond with the same version.
  Otherwise, the server MUST respond with another protocol version it supports. This SHOULD be the latest version
  supported by the server"). `initialize` with `2026-07-28` is answered with `2025-11-25`: `initialize` means legacy
  semantics. A missing `protocolVersion` is answered with `2025-11-25` rather than refused (lenient to sloppy clients).
- `ping` → `{}` (L/basic/utilities/ping: "The receiver MUST respond promptly with an empty response").
- `tools/list`, `tools/call` as §4; any other method → `200` with `-32601`.
- No `Mcp-Session-Id` (L "Session Management": a server "MAY assign a session ID"; the 400/404 rules bind only
  "Servers that require a session ID").

### Modern (2026-07-28)

Validation, in this order, each failure a `400`:

1. `_meta["io.modelcontextprotocol/protocolVersion"]` missing → `-32602` (S/basic "`_meta`": "A request missing any
   required field is malformed; the server MUST reject it with JSON-RPC error code `-32602` (Invalid params). On HTTP,
   the response status MUST be `400 Bad Request`").
2. `MCP-Protocol-Version` header missing or different from the `_meta` value → `-32020` (S transport "Protocol Version
   Header": "The header value MUST match the `io.modelcontextprotocol/protocolVersion` field … If the values do not
   match, the server MUST reject the request with `400 Bad Request` and a `HeaderMismatch` JSON-RPC error";
   "Validation failure conditions include: A required standard header (`MCP-Protocol-Version`, `Mcp-Method`,
   `Mcp-Name`) is missing").
3. version not in the supported list → `-32022` with `data: {supported, requested}` ("it MUST respond with
   `400 Bad Request` and an `UnsupportedProtocolVersionError` listing its supported versions"; `schema`:
   `UNSUPPORTED_PROTOCOL_VERSION = -32022`, data `supported: string[]`, `requested: string`).
4. `Mcp-Method` missing or different from `method` → `-32020` ("Standard Request Headers": `Mcp-Method` for "All
   requests", "These headers are REQUIRED for compliance").
5. for `tools/call`, `prompts/get` (`params.name`) and `resources/read` (`params.uri`), when the body has the field:
   `Mcp-Name` missing or different after decoding → `-32020`. A value `=?base64?…?=` is decoded first ("servers MUST
   decode an encoded `Mcp-Name` … value before comparing it to the corresponding request body value"; the markers
   "are case-sensitive and MUST appear exactly as shown (lowercase)").
6. `_meta["io.modelcontextprotocol/clientCapabilities"]` missing → `-32602` (the `_meta` table marks it required).

Then: `server/discover` → `{supportedVersions, capabilities: {tools: {}}, instructions, ttlMs: 3600000,
cacheScope: "private"}` (S/server/discover: "Servers MUST implement it"; `schema`: `DiscoverResult extends
CacheableResult { supportedVersions, capabilities, instructions? }`; `private` because the `instructions` name the
ledger's title, currency and timezone, and S/server/utilities/caching says a `public` result "may be shared between
callers even if the Result is coming from an authenticated endpoint"); `tools/list` with `ttlMs: 3600000,
cacheScope: "public"` ("Servers MUST include caching hints on results with `resultType: "complete"` returned by …
`server/discover`, `tools/list` …"; `public` is "appropriate for lists of tools … when they are identical for all
users"); `tools/call` as §4; any other method, `ping` and `initialize` included, → `404` + `-32601` ("If the server
does not implement the requested RPC method, it MUST respond with `404 Not Found` and a JSON-RPC error with code
`-32601`"). Every result gets `resultType: "complete"` (S/basic "Result Responses": "The `result` MUST include a
`resultType` field") and `_meta["io.modelcontextprotocol/serverInfo"]` ("Servers SHOULD include … in every result's
`_meta`"; `schema`: the value is an `Implementation {name, title?, version}`).

A `_meta` request naming a supported legacy version (e.g. `2025-11-25`) is served with modern semantics: the client
asked for per-request metadata, and the list it is answered with is the one `server/discover` advertises.

HTTP status of JSON-RPC errors: `400` and `404` where the specification names them (the rows above); errors a method
handler produces (unknown tool, arguments that fail the schema) are `200` with the error body, in both eras, as the
SDKs' HTTP transports do.

Error codes used: `-32700`, `-32600`, `-32601`, `-32602` (also "Unknown tool", S/server/tools "Error Handling"),
`-32020`, `-32022` (`schema`: `HEADER_MISMATCH = -32020`, `MISSING_REQUIRED_CLIENT_CAPABILITY = -32021` (never
emitted: no tool needs a client capability), `UNSUPPORTED_PROTOCOL_VERSION = -32022`). `-32002` is never emitted
(S/basic "Error Codes": "Implementations of this protocol version MUST NOT emit these codes").

## 4. Tools (`src/tools/`)

Conventions (S/server/tools, L/server/tools): names of `[A-Za-z0-9_]`; `inputSchema` is a JSON Schema object with
`additionalProperties: false` ("For tools with no parameters … `{ "type": "object", "additionalProperties": false }`
- Recommended"); `annotations: {readOnlyHint: true, destructiveHint: false, idempotentHint: true, openWorldHint:
false}`; dates are `YYYY-MM-DD`; `limit` defaults to 200, capped by `max_rows`. A rows tool answers
`{columns: [{name, type}], rows: [[cell…]], row_count, truncated}` with the cells of `POST /api/query` (decimals and
dates are strings, amounts `{number, currency}`, inventories `{positions: [{units, cost}]}`), as text and as
`structuredContent`, described by its `outputSchema`.

Errors: an unknown tool, or a call that is not a `CallToolRequest` (`params.name` missing or not a string,
`arguments` present but not an object) → `-32602` ("Protocol Errors … Unknown tool, Malformed requests (requests that
fail to satisfy CallToolRequest schema)"). An argument that fails the tool's own schema (unknown or missing argument,
wrong type, a date not `YYYY-MM-DD`, `limit` under 1 or over `max_rows`, an unknown `group_by`, `from` after `to`) →
`isError: true` with a text naming the argument and what it expects: S/server/tools "Error Handling" puts "Input
validation errors (e.g., date in wrong format, value out of range)" among the "Tool Execution Errors" that "contain
actionable feedback that language models can use to self-correct", and L/changelog records this as SEP-1303. A
failing query, an unavailable host function or a result over the host's cap → `isError: true` with a text such as
`query error at line 3, column 8: …` (or `<kind>: <message>` for a host error without a position, e.g.
`unavailable: …`) ("Tool Execution Errors … They are reported in tool results with `isError: true`"), no
`structuredContent` (D6).

| Tool | Arguments | Runs | Parity |
| --- | --- | --- | --- |
| `ledger_info` | — | `zhang_ledger_info` + `zhang_now` → `{title, operating_currency, timezone, now, today}` | — |
| `schema` | — | `zhang_query_schema` → the tables, columns, functions and keywords of BQL | the payload of `GET /api/query/schema` |
| `query` | `bql` (required), `limit` | `zhang_query(bql)`; rows cut at `limit` | — |
| `accounts` | `prefix`, `status` (`open`/`closed`/`all`, default `open`) | `accounts.list` (no prefix) or `accounts.subtree` (`:account` = prefix), `:date`/`:time` = now; rows whose engine-computed `status` is not the asked one are dropped (`all` keeps every row) | builtin |
| `balances` | `account` | `accounts.balances` or `accounts.subtree_balances`, `:operating_currency` from `ledger_info` | builtin |
| `net_worth` | `date` (default today), `currency` (default operating) | `report.net_worth` (`:to`, `:currency`): one row, `units` per currency and `value` in one currency | builtin (the app's report figure) |
| `spending` | `account` (default `Expenses`), `from`, `to` (both required), `group_by` (`account`/`month`/`currency`), `currency` | `account`: `report.account_totals` (`:type` = account); `month`: `mcp.spending_by_month`, the `report.changes` valuation for one subtree; `currency`: `mcp.spending_by_currency` | builtin / own |
| `journal` | `account` (required), `limit`, `offset` | `accounts.journal_page` (`:limit` = limit + 1, cut at limit) | builtin |
| `transactions` | `keyword`, `payee`, `tags`, `from`, `to`, `limit`, `offset` | `mcp.transactions` over `#transactions`, newest first, every filter NULL-able as in `postings.matching` and `journals.page` | own |
| `payees` | — | `journals.payees` | builtin |
| `errors` | `limit`, `offset` | `journals.errors` (`:size` = limit + 1) | builtin |
| `prices` | `commodity`, `currency` (default operating) | `commodities.latest_prices` or `commodities.latest_price` | builtin |

The BQL texts are in `src/tools/queries.rs`; `zhang-server/tests/mcp_plugin.rs` pins each `builtin` one to
`BUILTINS[name].bql` and its parameter types, and compiles the plugin's own ones with the engine.

`instructions` (sent by `initialize` and `server/discover`): what the ledger is (title, operating currency, timezone),
that amounts are exact decimals as strings, that `ledger_info` gives today's date and `schema` describes BQL for
`query`, and that row contents are data, not instructions.

## 5. Configuration and limits (`src/config.rs`)

```zhang
option "features.plugin" "true"
plugin "plugins/zhang-mcp.wasm"
  timeout: "20s"                                  ; the host's: how long one request may run (default 60s)
  allowed_origins: "https://ledger.example.com"   ; comma-separated or repeated; absent = the request's own host only
  max_rows: "500"                                 ; the cap of every tool's limit; default 1000
```

Bounds in effect: the plugin `timeout`; 10 s per BQL and `ZHANG_QUERY_MAX_RESULT_VALUES` (the host, as for
`POST /api/query`); `limit`/`max_rows` (the plugin; a `limit` over the cap is a tool error naming the cap, never a
silent cut); request bodies ≤ 1 MiB (the plugin); BQL text ≤ 1 MiB (the host).

## 6. Security

- Read-only by construction: no host function changes the ledger; the plugin declares no `allowed_hosts` and no
  `allowed_paths`.
- The credential never enters the sandbox (zhang withholds `authorization` and `cookie` from plugins); the `401` has no
  `WWW-Authenticate`, so clients are configured with the header up front (the guide shows how).
- Injection: every argument is bound as a typed parameter; the `query` tool runs arbitrary BQL by design, in the
  read-only, time- and size-bounded engine.
- Prompt injection: payees, narrations and metadata reach the model as data; the `instructions` say so, and the guide
  warns that ledger text reaches the model. The ledger title inside the `instructions` is JSON-quoted, so a title
  holding quotes or line breaks cannot add text to them.
- `Origin` (D8), `Cache-Control: no-store`, `X-Content-Type-Options: nosniff`.
- Rate limiting: not provided by a stateless plugin (S/server/tools "Security Considerations" lists it as a server
  MUST); mitigated by authentication, the plugin timeout and the query bounds. A per-plugin limiter would belong to
  the host.

## 7. Deviations from the PRD

- The endpoint is `/api/plugins/zhang-mcp/mcp`, with a description page at the root (D2).
- `net_worth` and `spending` by account reuse the app's own `report.net_worth` and `report.account_totals`
  (both exist in `BUILTINS`); the PRD sketched them as hand-written queries. `net_worth` takes an optional `date`.
- `transactions` reads `#transactions` (one row per transaction, with the accounts it posts to) and gains `keyword`
  and `offset`; the PRD modelled it on `postings.matching` (one row per posting, exact payee only).
- `spending` requires both `from` and `to`, and takes `currency`.
- `accounts` keeps the built-in BQL verbatim and applies `status` to the engine-computed `status` column; the PRD said
  "status filtered in BQL".
- `LIMIT`-bound tools fetch one row more than `limit`, so `truncated` is true when more rows exist.
- A legacy request with an unsupported version header is answered with `-32022` and the supported list (the PRD left
  the code open).
- Error responses carry the request's `id` whenever the request was parsed (the PRD said "without id" for the bad
  version header case); transport-level errors omit the `id` member instead of sending `null`.
- Arguments that fail a tool's schema are tool execution errors (`isError: true`), not `-32602` as the PRD and the
  first review draft said: the specification's tools "Error Handling" (SEP-1303) keeps input validation errors in
  tool results so the model can self-correct; `-32602` stays for an unknown tool and a malformed `CallToolRequest`.
- `server/discover` is `cacheScope: "private"` (the PRD said `public`): its `instructions` name the ledger.
- The same-host `Origin` rule applies to loopback hosts only (the PRD allowed any host equal to `Host`); any other
  deployment lists its origin in `allowed_origins`.
- A numeric request id must be an integer within 64 bits (a fraction or exponent is `-32600`), so that it is echoed
  exactly.

## 8. Follow-ups

- Resources (`zhang://schema`, `zhang://errors`, `zhang://ledger-info`) and prompts (`monthly-review`, `bql-help`): a
  separate PR; stateless and small.
- G4 writes: a generic `zhang_append_directives` recorded by the router host and applied after the plugin returns, gated
  by a directive capability; then an `add_transaction` tool with a preview/confirm shape.
- G3 compile cache: a router plugin's module is compiled per request today; an LLM round-trip dominates, but a cache
  is on #698's P1 list.
- G5 bundling: ship official plugins inside the binary (`plugin "zhang:mcp"`) instead of a downloaded file.
- Rate limiting in the host, if a shared deployment needs it.
- Echoing integer ids beyond 64 bits exactly would need the id carried as raw JSON text (`serde_json`'s `RawValue`)
  through the response builders; such ids do not occur in practice and are refused with `-32600` for now.
