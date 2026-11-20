---
title: Router 插件
description: WASM 插件如何在 /api/plugins/{name} 下提供自己的 HTTP 接口，它与张记账交换的请求和响应 JSON，以及它用来读取账本的 zhang_query 宿主函数。
sidebar:
  order: 8
---

声明了 `Router` 类型的插件会自己响应 HTTP 请求，因此可以在张记账自身的接口旁边提供自定义报表页面、图表数据或一个小型 API。和所有插件一样，它是用 [Extism](https://extism.org/) 构建的 WebAssembly 模块。Router 插件可以读取账本，但不能修改它。

## 使用 Router 插件

像其他插件一样启用插件并声明 Router 插件，见[插件](/zh-cn/guides/plugins/)：

```zhang
option "features.plugin" "true"

plugin "plugins/report.wasm"
```

**设置**页面会列出它，并附有指向其页面的**打开**链接。本页其余部分面向插件作者：请求如何到达插件，以及插件如何响应。

:::note[编写插件]
[编写插件](/zh-cn/developers/writing-plugins/)介绍了所有插件类型、`plugin` 指令及其能力，以及 Rust SDK，SDK 的 `router` 模块封装了本页的全部内容。
:::

## 路由

Router 插件服务 `/api/plugins/{name}` 及其下的所有路径，接受任何 HTTP 方法。`{name}` 是插件的 `name` 导出函数返回的名称，也就是设置页面和 `GET /api/plugins` 列出的名称。设置页面链接到每个 Router 插件的路由。

- `/api/plugins/report` 和 `/api/plugins/report/` 以路径 `/` 到达插件。
- `/api/plugins/report/by-month?year=2024` 以路径 `/by-month` 和查询参数 `{"year": ["2024"]}` 到达插件。

这些路由与 API 的其余部分使用同样的[身份认证](/zh-cn/deployment/authentication/)：启用登录后，无论使用什么方法，请求都需要会话（或脚本发送的 Basic 请求头）。当两个 Router 插件同名时，由先声明的那个提供该路由，张记账会记录一条警告。

## `router` 导出函数

插件导出一个名为 `router` 的函数。它的输入是 JSON 形式的请求：

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

- `path` 是插件路由之下的部分，保留发送时的百分号编码。
- `query` 把每个键映射到它的所有值，并保持顺序。
- 请求头名称为小写，重复请求头的值用 `, ` 连接。只有这些请求头会传给插件：`accept`、`accept-language`、`content-type`、`host`、`if-modified-since`、`if-none-match`、`origin` 和 `user-agent`，以及供 [MCP](https://modelcontextprotocol.io/) 端点使用的 `last-event-id`、`mcp-method`、`mcp-name`、`mcp-protocol-version` 和 `mcp-session-id`。其他请求头都会被丢弃：你的会话凭证（`authorization`、`cookie`），以及张记账前面的代理添加的身份请求头，例如 `cf-access-jwt-assertion`、`x-auth-request-*`、`tailscale-user-*` 和 `x-forwarded-*`，这样插件就无法把它们发往别处。
- 请求体是有效的 UTF-8 时，`body_encoding` 为 `utf8`，否则为 `base64`。请求体最大 4 MiB（见[限制](#限制)）。

它的输出是 JSON 形式的响应。每个字段都是可选的：

```json
{
  "status": 200,
  "headers": {"content-type": "text/html; charset=utf-8"},
  "body": "<h1>Monthly report</h1>",
  "body_encoding": "utf8"
}
```

- `status` 默认为 `200`。
- 没有 `content-type` 头时，响应的类型是 `text/plain; charset=utf-8`。任何内容类型都可以，因此插件可以返回 JSON、HTML 页面，或者配合 `"body_encoding": "base64"` 返回图片。
- 张记账只发送插件的这些响应头：`content-type`、`cache-control`、`etag`、`last-modified`、`content-disposition`、`content-language`、`vary`，以及停留在张记账内的 `location`：相对 URL，例如 `/api/plugins/report/by-month`、`by-month` 或 `?year=2024`。其他响应头都会被丢弃，例如 `set-cookie`、`content-security-policy`、`access-control-*`、`clear-site-data` 或 `refresh`，带协议或主机的 `location`（`https://…`、`//…`）也会被丢弃。响应仍然会发送，只是不带这些头。
- 张记账给每个响应加上[沙箱](#页面在沙箱中运行)的响应头。

## 页面在沙箱中运行

Router 插件的页面由张记账自己的地址提供，所以无论插件返回什么，张记账都会把它发送的每个响应放进沙箱：

```http
Content-Security-Policy: sandbox allow-scripts allow-forms allow-popups allow-downloads
X-Content-Type-Options: nosniff
```

浏览器以一个不透明的源（opaque origin）而不是张记账的源来显示这样的页面。页面中的脚本可以运行，但不是以张记账的身份：它们读不到张记账的 Cookie 和存储，浏览器也把它们发往张记账的请求视为来自其他站点，所以启用登录后这些请求不带会话，无法使用张记账的 API。插件的页面无法以你的身份登录、改写你的账本文件，也无法为自己创建移动端登录。

对插件作者来说，这意味着：

- 把页面的脚本、样式和图片放进页面本身（内联，或者用 `data:` URL）。启用登录后，页面无法再从张记账加载其他内容，包括插件自己的路由，它的脚本也无法获取插件的 JSON。
- 链接以及 `method="get"` 的表单会让整个页面跳转到插件的另一个页面，可以正常使用。链接可以打开新标签页（新标签页同样在沙箱中），也可以下载它指向的内容，例如带 `content-disposition: attachment` 的 CSV 导出。
- `X-Content-Type-Options: nosniff` 让浏览器不会把文本或 JSON 当作 HTML 或脚本：HTML 页面请以 `text/html` 发送。

## 限制

无论 `plugin` 指令请求什么，服务器都会限制每个 Router 插件。运维者在启动 `zhang serve` 时用环境变量设置这些限制：

| 限制 | 默认值 | 设置方式 | 达到时 |
|---|---|---|---|
| 请求体大小 | 4 MiB | `ZHANG_PLUGIN_MAX_BODY` | 413，插件不会运行 |
| 一个插件同时处理的请求数 | 4 | `ZHANG_PLUGIN_MAX_CONCURRENCY` | 立即返回 503，带 `Retry-After: 1` |
| 一个请求的运行时间 | 插件的 `timeout`，最多 30 秒 | `timeout` 元数据，可以更短 | 504 |
| 一个插件实例的内存 | 256 MiB | `ZHANG_PLUGIN_MAX_MEMORY` | 500 |

大小写作 `4MiB`、`512KiB` 或字节数。插件运行期间，请求一直持有账本的读锁；当一次账本写入在等待这把锁时，其他请求都得不到服务：所以即使插件的 `timeout` 更长，一个请求最多也只能运行 30 秒。

## 读取账本：`zhang_query`

Router 插件通过 `extism:host/user` 命名空间中的宿主函数读取账本。每个函数都返回 JSON，要么是 `{"Ok": value}`，要么是 `{"Err": {"kind": "...", "message": "..."}}`，并把问题作为值报告，而不是让插件失败。

- `zhang_query(bql)` 运行一条只读的[查询](/zh-cn/reference/query-language/)，返回 `POST /api/query` 在 `data` 中返回的内容：`{"columns": [{"name", "type"}], "rows": [[...]]}`。它有同样的时间和结果大小限制。失败的查询给出 kind `query`，以及 `message`、`line` 和 `column`。
- `zhang_ledger_info()` 返回 `{"title": "...", "operating_currency": "CNY", "timezone": "Asia/Shanghai"}`；账本没有设置 `title` 选项时，`title` 为 `null`。

使用普通 Extism PDK 的 Rust 代码如下（[Rust SDK](/zh-cn/developers/writing-plugins/#router-插件) 把它封装为 `router::query`）：

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

这些函数只在 `router` 运行期间响应。同时也是 `Processor` 或 `Mapper` 的插件仍能加载，但在那些阶段调用它们会得到 kind `unavailable`。

`zhang_emit_error` 在 router 中也可以使用，但请求没有可以添加错误的错误列表：张记账只把插件在那里报告的内容写入日志。要把问题告诉调用方，请在响应中返回它。

## 隔离与错误

每个请求都在插件的一个全新实例中运行，因此请求之间不保留任何状态。插件得到的配置和 `allowed_hosts` 与它作为 processor 运行时相同：运行超过其 `plugin` 指令的 `timeout` 元数据（最多 30 秒）的请求会被终止。无论 `allowed_paths` 如何，它都不能读取文件：在 router 中，`zhang_read_file` 和 `zhang_list_dir` 会返回 `denied`。它运行期间，账本不会重新加载。

插件无法响应时，张记账以 JSON `{"message": "..."}` 作为响应，并在日志中记录详情：

| 状态码 | 含义 |
|---|---|
| 404 | 没有该名称的 Router 插件。 |
| 413 | 请求体超过了限制。 |
| 501 | 插件声明了 `Router`，但没有导出 `router`。 |
| 502 | 插件返回的内容不是有效的响应。 |
| 503 | 插件已经在处理它能同时处理的最多请求数。请在 `Retry-After` 秒之后重试。 |
| 504 | 插件运行超过了超时时间。 |
| 500 | 插件触发了 trap、返回了错误、内存不足或无法加载。 |

:::danger[只安装你信任的插件]
沙箱让插件的页面碰不到你的会话，但 Router 插件仍然能读取你的整个账本，并决定它的页面显示什么。页面还可以把它显示的内容发往其他网站，例如作为一张图片的地址：发出这个请求的是你的浏览器而不是插件，所以 `allowed_hosts` 限制不了它。
:::
