---
title: Router Plugins
description: How a WASM plugin serves its own HTTP endpoints under /api/plugins/{name}, the request and response JSON it exchanges with Zhang, and the host functions it reads the ledger with, zhang_query and zhang_query_params.
sidebar:
  order: 8
---

A plugin that declares the `Router` type answers HTTP requests itself, so it can serve a custom report page, a chart's data or a small API next to Zhang's own. Like every plugin it is a WebAssembly module built with [Extism](https://extism.org/), or a Lua script that Zhang runs in its built-in interpreter ([Lua Plugins](/guides/lua-plugins/) has the Lua side of everything on this page). Router plugins can read the ledger but not change it.

## Use a router plugin

Enable plugins and declare the router plugin like any other, as [Using Plugins](/guides/plugins/) describes:

```zhang
option "features.plugin" "true"

plugin "plugins/report.wasm"
```

The **Settings** page lists it with an **Open** link to its pages. The rest of this page is for plugin authors: how requests reach the plugin and what it answers.

:::note[Writing plugins]
[Writing Plugins](/developers/writing-plugins/) covers every plugin type, the `plugin` directive and its capabilities, and the Rust SDK, whose `router` module wraps everything on this page.
:::

## Route

A router plugin serves `/api/plugins/{name}` and every path below it, for any HTTP method. `{name}` is the name the plugin's `name` export returns, the one the Settings page and `GET /api/plugins` list. The Settings page links to the route of each router plugin.

- `/api/plugins/report` and `/api/plugins/report/` reach the plugin with the path `/`.
- `/api/plugins/report/by-month?year=2024` reaches it with the path `/by-month` and the query `{"year": ["2024"]}`.

The routes sit behind the same [authentication](/deployment/authentication/) as the rest of the API: with sign-in enabled, a request needs a session (or the Basic header a script sends), whatever its method. When two router plugins have the same name, the first one declared serves the route and Zhang logs a warning.

## The `router` export

The plugin exports a function named `router`. Its input is the request as JSON:

```json
{
  "method": "GET",
  "path": "/by-month",
  "query": {"year": ["2024"]},
  "headers": {"accept": "text/html"},
  "body": "",
  "body_encoding": "utf8"
}
```

- `path` is the part below the plugin's route, with its percent-encoding as sent.
- `query` maps each key to all its values, in order.
- Header names are lower case, and the values of a repeated header are joined with `, `. The credential headers `authorization`, `proxy-authorization` and `cookie`, which carry your session, are never passed to a plugin.
- `body_encoding` is `utf8` when the body is valid UTF-8, and `base64` otherwise.

Its output is the response as JSON. Every field is optional:

```json
{
  "status": 200,
  "headers": {"content-type": "text/html; charset=utf-8"},
  "body": "<h1>Monthly report</h1>",
  "body_encoding": "utf8"
}
```

- `status` defaults to `200`.
- Without a `content-type` header the response is `text/plain; charset=utf-8`. Any content type works, so a plugin can return JSON, an HTML page or, with `"body_encoding": "base64"`, an image.
- Zhang sets `content-length`, `transfer-encoding` and `connection` itself and ignores them in the plugin's headers.

## Reading the ledger: `zhang_query` and `zhang_query_params`

A router plugin reads the ledger through host functions in the `extism:host/user` namespace. Each one returns JSON, either `{"Ok": value}` or `{"Err": {"kind": "...", "message": "..."}}`, and reports problems as values instead of failing the plugin.

- `zhang_query(bql)` runs a read-only [query](/reference/query-language/) and returns what `POST /api/query` returns in `data`: `{"columns": [{"name", "type"}], "rows": [[...]]}`. It has the same time and result size limits. A query that fails gives the kind `query`, with `message`, `line` and `column`.
- `zhang_query_params(input)` runs a query with named parameters bound, and returns the same. Bind what a user typed instead of writing it into the query text. The input is JSON:

  ```json
  {"bql": "SELECT date, narration WHERE date >= :from AND account ~ :re",
   "params": {"from": {"type": "date", "value": "2024-01-01"}, "re": {"type": "str", "value": null}}}
  ```

  `from` binds `:from` (positional `$1` parameters cannot be bound), and the query is compiled against the parameters' types. `type` is `bool`, `int`, `decimal`, `str`, `date` or `set`, and `value` is what the [built-in queries](/reference/builtin-queries/) take: a boolean, an integer, a number or a string such as `"12.50"`, a string, a string `YYYY-MM-DD`, or a list of strings; `null` is NULL for any type. A parameter of another type or shape is a `query` error without a position, e.g. `parameter :from must be a date string YYYY-MM-DD or null`; a parameter the query uses but the input does not bind is a `query` error at its position.
- `zhang_query_schema()` returns the tables, columns and functions a query can use, exactly what `GET /api/query/schema` returns in `data`, for a plugin that composes queries or describes them.
- `zhang_ledger_info()` returns `{"title": "...", "operating_currency": "CNY", "timezone": "Asia/Shanghai"}`, with a `title` of `null` when the ledger sets no `title` option.

In Rust with the plain Extism PDK (the [Rust SDK](/developers/writing-plugins/#router-plugins) wraps these as `router::query`, `router::query_with_params` and `router::schema`):

```rust
use extism_pdk::*;

#[host_fn]
extern "ExtismHost" {
    fn zhang_query(bql: String) -> String;
}

#[plugin_fn]
pub fn router(request: String) -> FnResult<String> {
    let rows = unsafe { zhang_query("SELECT account, sum(position) GROUP BY account".to_owned())? };
    Ok(serde_json::json!({
        "headers": {"content-type": "application/json"},
        "body": rows,
    })
    .to_string())
}
```

These functions answer only while `router` runs. A plugin that is also a `Processor` or `Mapper` still loads, and there they return the kind `unavailable`.

`zhang_emit_error` works in a router too, but a request has no error list to add to: Zhang only logs what the plugin reports there. To tell the caller about a problem, return it in the response.

## Isolation and errors

Every request runs in a fresh instance of the plugin, so nothing is kept between requests. The plugin gets the same config, `allowed_hosts` and `timeout` as its processor: a request running longer than 60 seconds, or the `timeout` meta of its `plugin` directive, is stopped. It cannot read files, whatever its `allowed_paths`: `zhang_read_file` and `zhang_list_dir` answer `denied` in a router. While it runs the ledger does not reload.

When the plugin cannot answer, Zhang responds with JSON `{"message": "..."}` and logs the details:

| Status | Meaning |
|---|---|
| 404 | No router plugin has that name. |
| 501 | The plugin declares `Router` but does not export `router`. |
| 502 | The plugin returned something that is not a valid response. |
| 504 | The plugin ran longer than its timeout. |
| 500 | The plugin trapped, returned an error or could not be loaded. |

:::danger[Only install plugins you trust]
A router plugin's pages are served from Zhang's own address. A script on such a page can call Zhang's API, including the endpoints that change your ledger files, with your browser's session.
:::
