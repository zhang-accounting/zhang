---
title: MCP Server
description: Let an AI assistant such as Claude read your ledger through the zhang-mcp plugin, an MCP server with read-only tools for balances, spending, journals and BQL.
sidebar:
  order: 9
---

[MCP](https://modelcontextprotocol.io/) (the Model Context Protocol) is how AI assistants such as Claude Code, Claude.ai and other LLM clients call tools. `zhang-mcp` is Zhang's MCP server, shipped as a [router plugin](/guides/router-plugins/): once declared in your ledger, an assistant can answer questions like "how much did I spend on food last month" or "what is my net worth" by running the same queries the web UI runs.

The plugin is read-only. It cannot add or change entries, and it reads the ledger only through the query engine, with the same time and size limits as the Query page.

## Install

1. Download `zhang-mcp-<version>.wasm` from the [release](https://github.com/zhang-accounting/zhang/releases) of the Zhang you run, and put it into your ledger directory, for example as `plugins/zhang-mcp.wasm`. The module is read through your ledger's storage, so for a ledger on S3, WebDAV or GitHub, upload it there too.
2. Enable plugins and declare it:

   ```zhang
   option "features.plugin" "true"

   plugin "plugins/zhang-mcp.wasm"
     timeout: "20s"
   ```

3. Reload. The **Settings** page lists `zhang-mcp` with an **Open** link, and `GET /api/plugins/zhang-mcp/` describes the server and where its endpoint is.

The MCP endpoint is `POST <your zhang url>/api/plugins/zhang-mcp/mcp`, for example `http://localhost:8000/api/plugins/zhang-mcp/mcp`.

The settings of the `plugin` directive:

| Metadata | Meaning | Default |
|---|---|---|
| `timeout` | how long one request may run, see [Using Plugins](/guides/plugins/#grant-capabilities) | 60 seconds; `"20s"` is plenty |
| `allowed_origins` | browser origins allowed to call the endpoint besides Zhang's own address, comma-separated (`"https://ledger.example.com, http://localhost:5173"`) or the key repeated | only Zhang's own address |
| `max_rows` | the most rows any tool returns | `"1000"` |

The plugin needs a Zhang with the `zhang_query_params` host function (the release it ships with, or newer). On an older Zhang the ledger fails to load with an error naming that import.

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

Claude.ai's custom connectors, and any client speaking Streamable HTTP, take the same URL and header. Claude.ai reaches the server from Anthropic's network, so Zhang must be reachable from the internet, over HTTPS, with sign-in enabled. Clients that only speak stdio need a stdio-to-HTTP bridge such as `mcp-remote`.

The server speaks both the current MCP revision (2026-07-28) and the earlier ones that today's clients use (2025-03-26 to 2025-11-25). It never streams and keeps no session, so it works through any reverse proxy.

## What to ask

- "What is my net worth?"
- "How much did I spend on food in September, by month?"
- "Show the last ten entries of Assets:Bank:Checking."
- "Find the transactions with Landlord this year."
- "Which entries have errors?"
- "What is the latest price of AAPL?"
- "Run `SELECT account, sum(position) WHERE under(account, 'Expenses') AND date >= 2026-01-01 GROUP BY account`."

The assistant is told what the ledger is, that amounts are exact decimals, and to read the schema before writing BQL, so it can combine the tools on its own.

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

The named queries are the [built-in queries](/reference/builtin-queries/) of the app, with their text copied as is, so an answer here is the answer the web UI shows. Every result is returned both as text and as structured JSON with the cell encoding of `POST /api/query`: decimals and dates are strings, amounts are `{number, currency}`. A query that fails comes back as a tool error naming the line and column, which the assistant uses to correct itself.

## Limits

- `limit` defaults to 200 rows and is capped by `max_rows` (1000). A result cut at the limit says `truncated: true`; the assistant can page with `offset` or narrow the query.
- Each query has the 10 second time limit and the result size limit of the Query page; a request as a whole has the plugin's `timeout`.
- Request bodies over 1 MiB are refused.

## Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| `401 {"message": "unauthorized"}` | sign-in is enabled and the client sends no valid `Authorization` header | add the `--header` as above; check the base64 of `user:password` |
| `403` on every request | the client is a browser page whose origin is not Zhang's own address | list it in `allowed_origins` |
| `405` | the client opened a GET stream (older clients may try it once, then continue with POST) | nothing: the server offers no stream |
| `504 {"message": "plugin zhang-mcp ran longer than its timeout"}` | a query ran longer than the plugin's `timeout` | raise `timeout`, or narrow the query |
| the ledger does not load, naming `zhang_query_params` | the Zhang is older than the plugin | upgrade Zhang, or use the plugin of your Zhang's release |
| the client says the server needs OAuth | it got a `401` before any header was configured | configure the header first; Zhang's `401` carries no OAuth metadata |

## Security

- Everything a tool returns, payees, narrations and metadata included, is sent to the assistant's model. Only connect a client whose provider you trust with your ledger.
- With sign-in disabled, anyone who can reach the port can read the ledger through this endpoint, as through the rest of the API. Enable sign-in, or bind Zhang to localhost.
- The plugin cannot write, cannot reach the network and cannot read files; it holds no credentials (Zhang never passes the `Authorization` header to a plugin).
- A web page cannot call the endpoint with your browser's session from another origin: the `Origin` check refuses it unless you listed that origin.
