---
title: Router Plugins
description: How a WASM plugin serves its own HTTP endpoints under /api/plugins/{name}, the request and response JSON it exchanges with Zhang, and the zhang_query host function it reads the ledger with.
---

A plugin that declares the `Router` type answers HTTP requests itself, so it can serve a custom report page, a chart's data or a small API next to Zhang's own. Like every plugin it is a WebAssembly module built with [Extism](https://extism.org/), and plugins must be enabled with `option "features.plugin" "true"`.

:::caution[Early version]
Router plugins can read the ledger but not change it. A full plugin guide comes later.
:::

## Route

A router plugin serves `/api/plugins/{name}` and every path below it, for any HTTP method. `{name}` is the name the plugin's `name` export returns, the one the Settings page and `GET /api/plugins` list. The Settings page links to the route of each router plugin.

- `/api/plugins/report` and `/api/plugins/report/` reach the plugin with the path `/`.
- `/api/plugins/report/by-month?year=2024` reaches it with the path `/by-month` and the query `{"year": ["2024"]}`.

The routes sit behind the same [authentication](/installation/3-authentication/) as the rest of the API: with sign-in enabled, a request needs a session (or the Basic header a script sends), whatever its method. When two router plugins have the same name, the first one declared serves the route and Zhang logs a warning.

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

## Reading the ledger: `zhang_query`

A router plugin reads the ledger through host functions in the `extism:host/user` namespace. Each one returns JSON, either `{"Ok": value}` or `{"Err": {"kind": "...", "message": "..."}}`, and reports problems as values instead of failing the plugin.

- `zhang_query(bql)` runs a read-only [query](/user-guide/query-language/) and returns what `POST /api/query` returns in `data`: `{"columns": [{"name", "type"}], "rows": [[...]]}`. It has the same time and result size limits. A query that fails gives the kind `query`, with `message`, `line` and `column`.
- `zhang_ledger_info()` returns `{"title": "...", "operating_currency": "CNY", "timezone": "Asia/Shanghai"}`.

In Rust with the Extism PDK:

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

Every request runs in a fresh instance of the plugin, so nothing is kept between requests. The plugin gets the same config, `allowed_hosts` and `timeout` as its processor: a request running longer than 60 seconds, or the `timeout` meta of its `plugin` directive, is stopped. While it runs the ledger does not reload.

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
