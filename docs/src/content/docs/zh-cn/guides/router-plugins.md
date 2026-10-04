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
- 请求头名称为小写，重复请求头的值用 `, ` 连接。携带你的会话的凭证请求头 `authorization`、`proxy-authorization` 和 `cookie` 永远不会传给插件。
- 请求体是有效的 UTF-8 时，`body_encoding` 为 `utf8`，否则为 `base64`。

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
- 张记账自己设置 `content-length`、`transfer-encoding` 和 `connection`，并忽略插件响应头中的这些字段。

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

每个请求都在插件的一个全新实例中运行，因此请求之间不保留任何状态。插件得到的配置、`allowed_hosts` 和 `timeout` 与它作为 processor 运行时相同：运行超过 60 秒（或其 `plugin` 指令的 `timeout` 元数据）的请求会被终止。无论 `allowed_paths` 如何，它都不能读取文件：在 router 中，`zhang_read_file` 和 `zhang_list_dir` 会返回 `denied`。它运行期间，账本不会重新加载。

插件无法响应时，张记账以 JSON `{"message": "..."}` 作为响应，并在日志中记录详情：

| 状态码 | 含义 |
|---|---|
| 404 | 没有该名称的 Router 插件。 |
| 501 | 插件声明了 `Router`，但没有导出 `router`。 |
| 502 | 插件返回的内容不是有效的响应。 |
| 504 | 插件运行超过了超时时间。 |
| 500 | 插件触发了 trap、返回了错误或无法加载。 |

:::danger[只安装你信任的插件]
Router 插件的页面由张记账自己的地址提供。这样的页面上的脚本可以凭你浏览器的会话调用张记账的 API，包括修改账本文件的接口。
:::
