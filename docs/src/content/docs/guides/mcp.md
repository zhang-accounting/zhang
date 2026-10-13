---
title: MCP Server
description: Let an AI assistant such as Claude read your ledger, and record entries you approve, through the zhang-mcp plugin, a Lua script with MCP tools for balances, spending, journals, BQL and transactions.
sidebar:
  order: 10
---

[MCP](https://modelcontextprotocol.io/) (the Model Context Protocol) is how AI assistants such as Claude Code, Claude.ai and other LLM clients call tools. `zhang-mcp` is Zhang's MCP server, shipped as a [Lua plugin](/guides/lua-plugins/): one file, `mcp.lua`, that you copy into your ledger. Once declared, an assistant can answer questions like "how much did I spend on food last month" or "what is my net worth" by running the same queries the web UI runs, and, if you allow it, record an entry you describe after showing you exactly what it will write.

Reading is the default and needs nothing else. Writing is off until you turn on two switches (below); even then the plugin can only append entries, never edit or delete anything, and a well-behaved client asks you before each one.

## Install

1. Get `mcp.lua`: `zhang-mcp-<version>.lua` from the [release](https://github.com/zhang-accounting/zhang/releases) of the Zhang you run, or `plugins/mcp/mcp.lua` from the repository at that version. Put it into your ledger directory, for example as `plugins/mcp.lua`; the script is read through your ledger's storage, so for a ledger on S3, WebDAV or GitHub, upload it there too.
2. Enable plugins and declare it:

   ```zhang
   option "features.plugin" "true"

   plugin "plugins/mcp.lua"
     timeout: "20s"
   ```

3. Reload. The **Settings** page lists `zhang-mcp` with an **Open** link, and `GET /api/plugins/zhang-mcp/` describes the server and where its endpoint is.

:::note[Planned: a bundled plugin]
A bundled `plugin "zhang:mcp"` is planned, which removes step 1; the settings, the tools and everything below stay the same.
:::

The MCP endpoint is `POST <your zhang url>/api/plugins/zhang-mcp/mcp`, for example `http://localhost:8000/api/plugins/zhang-mcp/mcp`.

The settings of the `plugin` directive:

| Metadata | Meaning | Default |
|---|---|---|
| `timeout` | how long one request may run, see [Using Plugins](/guides/plugins/#grant-capabilities) | 60 seconds; `"20s"` is plenty |
| `allowed_origins` | browser origins (`scheme://host[:port]`) allowed to call the endpoint, comma-separated (`"https://ledger.example.com, http://localhost:5173"`) or the key repeated. Without it, only a page served by Zhang itself on `localhost` or `127.0.0.1` may call it; a Zhang behind a domain or a reverse proxy lists that address here | none |
| `max_rows` | the most rows any tool returns | `"1000"` |
| `allow_writes` | `"true"` lists the three write tools (`preview_transaction`, `add_transaction`, `open_account`); see [Recording entries](#recording-entries) | `"false"` |
| `write_tag` | the tag `add_transaction` puts on every entry it records, so you can find them later; `""` disables it | `"mcp"` |
| `allowed_writes` | Zhang's own grant, one line per kind: `"transaction"` and `"open"`. Without it the write tools cannot write, whatever `allow_writes` says | none |

The plugin needs the Zhang it ships with, or newer: it is a Lua script and, for writes, uses the plugin write capability of that release. On an older Zhang the ledger fails to load with an error naming the script or a missing host function.

## Connect a client

With [sign-in enabled](/deployment/authentication/), MCP clients send the credentials as an `Authorization` header with every request. Zhang does not use OAuth, so configure the header when you add the server. The simplest credential is the HTTP Basic form of your username and password:

```shell
printf 'admin:admin888' | base64    # → YWRtaW46YWRtaW44ODg=
```

### Claude Code

```shell
claude mcp add --transport http zhang http://localhost:8000/api/plugins/zhang-mcp/mcp \
  --header "Authorization: Basic YWRtaW46YWRtaW44ODg="
```

Then, in a Claude Code session, ask away: the tools appear as `mcp__zhang__*`. `claude mcp list` shows the connection, and `claude mcp remove zhang` removes it. Without sign-in (a Zhang on localhost with no `--auth`), leave the `--header` out.

Instead of the password, a session token works as `--header "Authorization: Bearer <token>"`: the value of the `zhang_session` cookie of a signed-in browser, or the token the [mobile app sign-in](/deployment/authentication/#mobile-app-sign-in) returns. Sessions expire after 30 days.

### MCP Inspector

The [Inspector](https://github.com/modelcontextprotocol/inspector) is the reference client for trying a server by hand:

```shell
npx @modelcontextprotocol/inspector --cli http://localhost:8000/api/plugins/zhang-mcp/mcp \
  --transport http --header "Authorization: Basic YWRtaW46YWRtaW44ODg=" --method tools/list

npx @modelcontextprotocol/inspector --cli http://localhost:8000/api/plugins/zhang-mcp/mcp \
  --transport http --header "Authorization: Basic YWRtaW46YWRtaW44ODg=" \
  --method tools/call --tool-name spending --tool-arg from=2026-09-01 --tool-arg to=2026-09-30
```

### Claude.ai and other clients

Any client speaking Streamable HTTP takes the same URL and header. A hosted client such as Claude.ai reaches the server from its provider's network, so Zhang must be reachable from the internet, over HTTPS, with sign-in enabled, and the client must let you set a fixed `Authorization` header (check its connector settings; Zhang offers no OAuth flow). Clients that only speak stdio need a stdio-to-HTTP bridge such as `mcp-remote`.

The server speaks both the current MCP revision (2026-07-28) and the earlier ones that today's clients use (2025-03-26 to 2025-11-25). It never streams and keeps no session, so it works through any reverse proxy.

## What to ask

- "What is my net worth?"
- "How much did I spend on food in September, by month?"
- "Show the last ten entries of Assets:Bank:Checking."
- "Find the transactions with Landlord this year."
- "Which entries have errors?"
- "What is the latest price of AAPL?"
- "Run `SELECT account, sum(position) WHERE under(account, 'Expenses') AND date >= 2026-01-01 GROUP BY account`."
- With writes on: "Record 4.50 CNY for a coffee at Blue Bottle today, paid in cash."

The assistant is told what the ledger is, that amounts are exact decimals, to read the schema before writing BQL, and, when writing is on, to show you the preview before recording anything, so it can combine the tools on its own.

## The tools

| Tool | Answers | Runs |
|---|---|---|
| `ledger_info` | the title, operating currency, timezone, and today's date and time by the ledger's clock | the ledger's options and clock |
| `schema` | the tables, columns and functions of BQL | what `GET /api/query/schema` returns |
| `query` | the rows of a BQL query, `limit` of them | the query, as the Query page runs it |
| `accounts` | the accounts with their open and close dates, alias and status; `prefix` for a subtree, `status` for closed ones | `accounts.list` / `accounts.subtree` |
| `balances` | every account's balance per currency, valued in the operating currency at today's prices; `account` for a subtree | `accounts.balances` / `accounts.subtree_balances` |
| `net_worth` | the net worth at the end of a day (default today): units per currency and the value in one currency | `report.net_worth`, the figure of the report page |
| `spending` | what a subtree (default `Expenses`) took from `from` to `to`, grouped by account, by month or by currency | `report.account_totals` and two queries of the plugin's own |
| `journal` | the postings of an account and its sub-accounts with the running balance, paged | `accounts.journal_page` |
| `transactions` | the transactions matching a keyword, a payee, tags and a date range, newest first | a query of the plugin's own over `#transactions` |
| `payees` | every payee, sorted | `journals.payees` |
| `errors` | the ledger's errors with their source text, paged | `journals.errors` |
| `prices` | the latest price of each commodity (or one) in a currency, as the valuations use it | `commodities.latest_prices` / `commodities.latest_price` |
| `preview_transaction` | the exact text a transaction would be written as, the file, the errors, and a `token` when it is clean; writes nothing | the same validation as the web UI's preview |
| `add_transaction` | appends the transaction previewed with that `token` | the same append as the web UI |
| `open_account` | appends an `open` directive, in one call; nothing when the account is already open with those commodities | the same append |

The named queries are the [built-in queries](/reference/builtin-queries/) of the app, with their text copied as is, so an answer here is the answer the web UI shows. Every result is returned both as text and as structured JSON with the cell encoding of `POST /api/query`: decimals and dates are strings, amounts are `{number, currency}`. A query that fails comes back as a tool error naming the line and column, which the assistant uses to correct itself. The three write tools exist only when writing is on.

## Recording entries

Two switches, both needed, both in the `plugin` directive:

```zhang
plugin "plugins/mcp.lua"
  timeout: "20s"
  allow_writes: "true"              ; the plugin lists its write tools
  allowed_writes: "transaction"     ; Zhang lets this plugin append transactions
  allowed_writes: "open"            ; ... and open directives
```

`allow_writes` is the plugin's own setting: without it the write tools are not even listed, so a read-only deployment never shows them to the model. `allowed_writes` is Zhang's grant, the same kind of line as `allowed_paths` or `allowed_hosts`: without it the tools are listed (when `allow_writes` is on) but every call answers with the exact line to add. Grant it only to a copy of `mcp.lua` you trust: see the [capabilities](/reference/directives/plugin/) of the `plugin` directive.

How a transaction gets recorded:

1. The assistant calls `preview_transaction` with the date, payee, narration and postings (amounts as decimal strings such as `"4.50"`; one posting may leave its amount out, as in the web UI; a posting without a currency is in the operating currency). Zhang validates it exactly as the web UI's preview does and answers the text as it will be written, the file it will go to (chosen by [`directive_output_path`](/guides/recording-transactions/#where-new-entries-are-written)), any error, and, only when everything is clean, a `token`.
2. The assistant shows you that text and asks. This is what the instructions tell it to do, and clients such as Claude Code and Claude Desktop also ask you themselves before a tool that is not read-only runs.
3. When you agree, it calls `add_transaction` with the same arguments and the token. Zhang checks that the token is the one it minted for exactly that text on the ledger as it is now, appends the text, and reloads. The entry carries the tag `#mcp` (`write_tag`), so `transactions` with `tags: ["mcp"]`, or the journal's tag filter, finds everything an assistant recorded.

The token is valid for ten minutes and for one ledger state: anything written in between (by you, by the web UI or by another assistant) makes it stale, and `add_transaction` answers "the ledger changed, run `preview_transaction` again". The assistant then previews again and shows you the new text. A token cannot be reused: once an entry is in, the same token is refused.

Accounts are never created on the fly. When the preview reports `AccountDoesNotExist`, the assistant is told to ask you, then open the account explicitly with `open_account` (which a client also asks you to approve) and preview again. `open_account` writes nothing when the account is already open with the same commodities.

## Limits

- `limit` defaults to 200 rows; asking for more than `max_rows` (1000) is answered with an error naming the cap. A result cut at the limit says `truncated: true`; the assistant can raise `limit`, narrow the query, or, for `journal`, `transactions` and `errors`, page on with `offset`.
- Each query has the 10 second time limit and the result size limit of the Query page; a request as a whole has the plugin's `timeout`.
- Request bodies over 1 MiB are refused.
- A write is one transaction, or one `open`, per call; a preview token lives ten minutes; one entry is committed per reload cycle (the next one waits for the reload, which takes a moment).

## Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| `401 {"message": "unauthorized"}` | sign-in is enabled and the client sends no valid `Authorization` header | add the `--header` as above; check the base64 of `user:password` |
| `403` on every request | the client is a browser page whose origin is neither Zhang's own `localhost` address nor listed; a Zhang behind a reverse proxy that rewrites `Host`, or reached by a domain or LAN address, needs the listing too | add the page's origin to `allowed_origins` |
| `405` | the client opened a GET stream (older clients may try it once, then continue with POST) | nothing: the server offers no stream |
| `504 {"message": "plugin zhang-mcp ran longer than its timeout"}` | a query ran longer than the plugin's `timeout` | raise `timeout`, or narrow the query |
| the ledger does not load, naming `mcp.lua` or a host function | the Zhang is older than the plugin | upgrade Zhang, or use the plugin of your Zhang's release |
| the client says the server needs OAuth | it got a `401` before any header was configured | configure the header first; Zhang's `401` carries no OAuth metadata |
| the assistant says it has no tool to record a transaction | `allow_writes` is off, or no `allowed_writes` line grants anything | add the lines of [Recording entries](#recording-entries) and reload |
| `add_transaction` answers that the plugin may not write transactions | `allow_writes` is on but the grant is missing | the message names the `allowed_writes` line to add |
| `add_transaction` answers that the ledger changed | something was written since the preview, or the token is older than ten minutes, or it was already used | let the assistant preview again and show you the new text |

## Security

- Everything a tool returns, payees, narrations and metadata included, is sent to the assistant's model. Only connect a client whose provider you trust with your ledger.
- With sign-in disabled, anyone who can reach the port can read the ledger through this endpoint, as through the rest of the API, and, with writes on, append to it. Enable sign-in, or bind Zhang to localhost.
- Writes are additive only: nothing can be edited or deleted through this server, every entry is validated like one typed in the web UI, and the file is chosen by your ledger's own policy. The token ties a commit to the exact text that was previewed; the tag marks the entries; Zhang's log records every commit.
- The plugin cannot reach the network and cannot read files; it holds no credentials (Zhang never passes the `Authorization` header to a plugin).
- A web page cannot call the endpoint with your browser's session: the `Origin` check refuses every page except one served by Zhang itself on `localhost` and those you listed in `allowed_origins`, so a malicious site cannot reach a Zhang on your machine through DNS rebinding.
