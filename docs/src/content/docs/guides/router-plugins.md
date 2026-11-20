---
title: Router Plugins
description: How a WASM plugin serves its own HTTP endpoints under /api/plugins/{name}, the request and response JSON it exchanges with Zhang, and the zhang_query host function it reads the ledger with.
sidebar:
  order: 8
---

A plugin that declares the `Router` type answers HTTP requests itself, so it can serve a custom report page, a chart's data or a small API next to Zhang's own. Like every plugin it is a WebAssembly module built with [Extism](https://extism.org/). Router plugins can read the ledger but not change it.

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
- Header names are lower case, and the values of a repeated header are joined with `, `. Only these headers reach the plugin: `accept`, `accept-language`, `content-type`, `host`, `if-modified-since`, `if-none-match`, `origin` and `user-agent`, and for an [MCP](https://modelcontextprotocol.io/) endpoint `last-event-id`, `mcp-method`, `mcp-name`, `mcp-protocol-version` and `mcp-session-id`. Every other header is dropped: the credentials of your session (`authorization`, `cookie`), and the identity headers a proxy in front of Zhang adds, such as `cf-access-jwt-assertion`, `x-auth-request-*`, `tailscale-user-*` and `x-forwarded-*`, so a plugin cannot send them elsewhere.
- `body_encoding` is `utf8` when the body is valid UTF-8, and `base64` otherwise. The body is at most 4 MiB (see [Limits](#limits)).

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
- Zhang sends only these headers of the plugin: `content-type`, `cache-control`, `etag`, `last-modified`, `content-disposition`, `content-language`, `vary`, `allow` (for a `405`), and `location` when it stays on Zhang: a relative URL such as `/api/plugins/report/by-month`, `by-month` or `?year=2024`. It drops every other header, such as `set-cookie`, `content-security-policy`, `access-control-*`, `clear-site-data` or `refresh`, and a `location` with a scheme or a host (`https://…`, `//…`). The response is still sent, without them.
- Zhang adds the headers of the [sandbox](#pages-are-sandboxed) to every response.

## Pages are sandboxed

A router plugin's pages are served from Zhang's own address, so Zhang sandboxes every response the plugin sends, whatever it is:

```http
Content-Security-Policy: sandbox allow-scripts allow-forms allow-popups allow-downloads
X-Content-Type-Options: nosniff
```

The browser shows such a page with an opaque origin instead of Zhang's. Its scripts run, but not as Zhang: they cannot read Zhang's cookies or storage, and the browser treats their requests to Zhang as coming from another site, so with sign-in enabled they carry no session and cannot use Zhang's API. A plugin's page cannot sign in as you, rewrite your ledger files or create an app login for itself.

For a plugin author this means:

- Put the page's scripts, styles and images into the page itself (inline, or as `data:` URLs). With sign-in enabled, the page cannot load more from Zhang, its own routes included, and its scripts cannot fetch the plugin's JSON.
- Links, and forms with `method="get"`, navigate the whole page to another page of the plugin and keep working. Links can open new tabs, which are sandboxed too, and download what they point at, such as a CSV export with `content-disposition: attachment`.
- `X-Content-Type-Options: nosniff` keeps a browser from reading text or JSON as HTML or script: send an HTML page as `text/html`.

## Limits

The server bounds every router plugin, whatever its `plugin` directive asks for. The operator sets the limits with environment variables when starting `zhang serve`:

| Limit | Default | Set with | When it is reached |
|---|---|---|---|
| Size of a request body | 4 MiB | `ZHANG_PLUGIN_MAX_BODY` | 413, before the plugin runs |
| Requests one plugin handles at once | 4 | `ZHANG_PLUGIN_MAX_CONCURRENCY` | 503 with `Retry-After: 1`, at once |
| Time one request runs | the plugin's `timeout`, at most 30 seconds | the `timeout` meta, for less | 504 |
| Memory of one plugin instance | 256 MiB | `ZHANG_PLUGIN_MAX_MEMORY` | 500 |

Sizes are written as `4MiB`, `512KiB` or a number of bytes. A request holds the ledger's read lock for as long as the plugin runs, and while a write to the ledger waits for that lock, no other request is served: that is why a request may run for 30 seconds at most, even when the plugin's `timeout` is longer.

## Reading the ledger: `zhang_query`

A router plugin reads the ledger through host functions in the `extism:host/user` namespace. Each one returns JSON, either `{"Ok": value}` or `{"Err": {"kind": "...", "message": "..."}}`, and reports problems as values instead of failing the plugin.

- `zhang_query(bql)` runs a read-only [query](/reference/query-language/) and returns what `POST /api/query` returns in `data`: `{"columns": [{"name", "type"}], "rows": [[...]]}`. It has the same time and result size limits. A query that fails gives the kind `query`, with `message`, `line` and `column`.
- `zhang_ledger_info()` returns `{"title": "...", "operating_currency": "CNY", "timezone": "Asia/Shanghai"}`, with a `title` of `null` when the ledger sets no `title` option.

In Rust with the plain Extism PDK (the [Rust SDK](/developers/writing-plugins/#router-plugins) wraps this as `router::query`):

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

Every request runs in a fresh instance of the plugin, so nothing is kept between requests. The plugin gets the same config and `allowed_hosts` as its processor: a request running longer than the `timeout` meta of its `plugin` directive, and at most 30 seconds, is stopped. It cannot read files, whatever its `allowed_paths`: `zhang_read_file` and `zhang_list_dir` answer `denied` in a router. While it runs the ledger does not reload.

When the plugin cannot answer, Zhang responds with JSON `{"message": "..."}` and logs the details:

| Status | Meaning |
|---|---|
| 404 | No router plugin has that name. |
| 413 | The request body is larger than the limit. |
| 501 | The plugin declares `Router` but does not export `router`. |
| 502 | The plugin returned something that is not a valid response. |
| 503 | The plugin already handles as many requests as it may. Try again after the `Retry-After` seconds. |
| 504 | The plugin ran longer than its timeout. |
| 500 | The plugin trapped, returned an error, ran out of memory or could not be loaded. |

:::danger[Only install plugins you trust]
The sandbox keeps a plugin's pages away from your session, but a router plugin still reads your whole ledger and decides what its pages show. A page can also send what it shows to another site, for example as the address of an image: your browser makes that request, not the plugin, so `allowed_hosts` does not limit it.
:::
