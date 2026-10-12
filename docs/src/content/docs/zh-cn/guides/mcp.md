---
title: MCP 服务器
description: 通过 zhang-mcp 插件让 Claude 等 AI 助手读取你的账本：一个提供余额、支出、流水和 BQL 等只读工具的 MCP 服务器。
sidebar:
  order: 9
---

[MCP](https://modelcontextprotocol.io/)（Model Context Protocol）是 Claude Code、Claude.ai 等 AI 助手和 LLM 客户端调用工具的协议。`zhang-mcp` 是张记账的 MCP 服务器，以 [Router 插件](/zh-cn/guides/router-plugins/)的形式提供：在账本中声明之后，助手就能通过运行和网页界面相同的查询来回答"上个月我在吃饭上花了多少"或"我的净资产是多少"这样的问题。

这个插件是只读的。它不能添加或修改条目，只通过查询引擎读取账本，并受和查询页面相同的时间和大小限制。

## 安装

1. 从你所运行的张记账版本的 [release](https://github.com/zhang-accounting/zhang/releases) 下载 `zhang-mcp-<version>.wasm`，放进账本目录，例如 `plugins/zhang-mcp.wasm`。模块通过账本的存储读取，所以存放在 S3、WebDAV 或 GitHub 上的账本也要把它上传到那里。
2. 启用插件并声明它：

   ```zhang
   option "features.plugin" "true"

   plugin "plugins/zhang-mcp.wasm"
     timeout: "20s"
   ```

3. 重新加载。**设置**页面会列出 `zhang-mcp` 并附有**打开**链接；`GET /api/plugins/zhang-mcp/` 会描述这个服务器及其端点所在。

MCP 端点是 `POST <你的张记账地址>/api/plugins/zhang-mcp/mcp`，例如 `http://localhost:8000/api/plugins/zhang-mcp/mcp`。

`plugin` 指令的设置：

| 元数据 | 含义 | 默认值 |
|---|---|---|
| `timeout` | 一个请求最多可以运行多久，见[插件](/zh-cn/guides/plugins/#授予能力) | 60 秒；`"20s"` 足够 |
| `allowed_origins` | 除张记账自己的地址之外，允许调用端点的浏览器来源（origin），用逗号分隔（`"https://ledger.example.com, http://localhost:5173"`）或重复该键 | 只有张记账自己的地址 |
| `max_rows` | 任何工具最多返回的行数 | `"1000"` |

这个插件需要带有 `zhang_query_params` 宿主函数的张记账（随它一起发布的版本或更新）。在更老的张记账上，账本会加载失败，并在错误中指出这个导入。

## 连接客户端

[启用登录](/zh-cn/deployment/authentication/)后，MCP 客户端在每个请求中以 `Authorization` 请求头发送凭据。张记账不使用 OAuth，所以要在添加服务器时就配置好这个请求头。最简单的凭据是用户名和密码的 HTTP Basic 形式：

```shell
printf 'admin:admin888' | base64    # → YWRtaW46YWRtaW44ODg=
```

### Claude Code

```shell
claude mcp add --transport http zhang http://localhost:8000/api/plugins/zhang-mcp/mcp \
  --header "Authorization: Basic YWRtaW46YWRtaW44ODg="
```

然后在 Claude Code 会话中直接提问：工具以 `mcp__zhang__*` 的名字出现。`claude mcp list` 显示连接状态，`claude mcp remove zhang` 移除它。没有启用登录时（本机上没有 `--auth` 的张记账），省略 `--header` 即可。

除了密码，会话令牌也可以用作 `--header "Authorization: Bearer <token>"`：已登录浏览器的 `zhang_session` cookie 的值，或[移动应用登录](/zh-cn/deployment/authentication/#移动端登录)返回的令牌。会话 30 天后过期。

### MCP Inspector

[Inspector](https://github.com/modelcontextprotocol/inspector) 是手动试用服务器的参考客户端：

```shell
npx @modelcontextprotocol/inspector --cli http://localhost:8000/api/plugins/zhang-mcp/mcp \
  --transport http --header "Authorization: Basic YWRtaW46YWRtaW44ODg=" --method tools/list

npx @modelcontextprotocol/inspector --cli http://localhost:8000/api/plugins/zhang-mcp/mcp \
  --transport http --header "Authorization: Basic YWRtaW46YWRtaW44ODg=" \
  --method tools/call --tool-name spending --tool-arg from=2026-09-01 --tool-arg to=2026-09-30
```

### Claude.ai 和其他客户端

Claude.ai 的自定义连接器，以及任何使用 Streamable HTTP 的客户端，都使用同样的 URL 和请求头。Claude.ai 从 Anthropic 的网络访问服务器，所以张记账必须能从互联网通过 HTTPS 访问，并启用登录。只支持 stdio 的客户端需要一个 stdio 到 HTTP 的桥接，例如 `mcp-remote`。

服务器同时支持当前的 MCP 修订版（2026-07-28）和现今客户端使用的早期版本（2025-03-26 到 2025-11-25）。它从不使用流式响应，也不保存会话，因此可以经过任何反向代理。

## 可以问什么

- "我的净资产是多少？"
- "九月我在吃饭上花了多少，按月分？"
- "显示 Assets:Bank:Checking 最近的十条记录。"
- "找出今年和 Landlord 的交易。"
- "哪些条目有错误？"
- "AAPL 的最新价格是多少？"
- "运行 `SELECT account, sum(position) WHERE under(account, 'Expenses') AND date >= 2026-01-01 GROUP BY account`。"

助手会被告知这个账本是什么、金额是精确的小数、写 BQL 之前要先读 schema，所以它能自己组合这些工具。

## 工具

| 工具 | 回答 | 运行 |
|---|---|---|
| `ledger_info` | 标题、记账本位币、时区，以及按账本时钟的今天日期和时间 | 账本的选项和时钟 |
| `schema` | BQL 的表、列和函数 | `GET /api/query/schema` 返回的内容 |
| `query` | 一条 BQL 查询的行，最多 `limit` 行 | 和查询页面一样运行该查询 |
| `accounts` | 账户及其开户、关户日期、别名和状态；`prefix` 取一棵子树，`status` 查看已关闭的 | `accounts.list` / `accounts.subtree` |
| `balances` | 每个账户每种货币的余额，按今天的价格折算成记账本位币；`account` 取一棵子树 | `accounts.balances` / `accounts.subtree_balances` |
| `net_worth` | 某一天（默认今天）结束时的净资产：每种货币的数量和按一种货币折算的价值 | `report.net_worth`，报表页面的数字 |
| `spending` | 一棵子树（默认 `Expenses`）从 `from` 到 `to` 的支出，按账户、按月或按货币分组 | `report.account_totals` 和插件自己的两条查询 |
| `journal` | 一个账户及其子账户的分录，附每条之后的余额，分页 | `accounts.journal_page` |
| `transactions` | 匹配关键字、收款人、标签和日期范围的交易，最新在前 | 插件自己对 `#transactions` 的查询 |
| `payees` | 所有收款人，排序 | `journals.payees` |
| `errors` | 账本的错误及其源文本，分页 | `journals.errors` |
| `prices` | 每种（或一种）商品在某货币中的最新价格，即估值所用的价格 | `commodities.latest_prices` / `commodities.latest_price` |

这些具名查询就是应用的[内置查询](/zh-cn/reference/builtin-queries/)，原文照抄，所以这里的答案就是网页界面显示的答案。每个结果同时以文本和结构化 JSON 返回，采用 `POST /api/query` 的单元格编码：小数和日期是字符串，金额是 `{number, currency}`。失败的查询会以工具错误返回，并指出行和列，助手据此自行纠正。

## 限制

- `limit` 默认 200 行，上限由 `max_rows`（1000）决定。被截断的结果会标明 `truncated: true`；助手可以用 `offset` 翻页或缩小查询。
- 每条查询有和查询页面相同的 10 秒时间限制和结果大小限制；整个请求受插件的 `timeout` 限制。
- 超过 1 MiB 的请求体会被拒绝。

## 排错

| 现象 | 原因 | 处理 |
|---|---|---|
| `401 {"message": "unauthorized"}` | 启用了登录，而客户端没有发送有效的 `Authorization` 请求头 | 按上文加上 `--header`；检查 `user:password` 的 base64 |
| 每个请求都 `403` | 客户端是一个来源不是张记账自己地址的浏览器页面 | 把它列入 `allowed_origins` |
| `405` | 客户端尝试打开 GET 流（较老的客户端可能尝试一次，然后继续用 POST） | 无需处理：服务器不提供流 |
| `504 {"message": "plugin zhang-mcp ran longer than its timeout"}` | 一条查询运行超过了插件的 `timeout` | 提高 `timeout`，或缩小查询 |
| 账本加载失败，错误提到 `zhang_query_params` | 张记账比插件旧 | 升级张记账，或使用你的张记账版本自带的插件 |
| 客户端说服务器需要 OAuth | 在配置请求头之前它收到了 `401` | 先配置请求头；张记账的 `401` 不带 OAuth 元数据 |

## 安全

- 工具返回的一切，包括收款人、摘要和元数据，都会发送给助手的模型。只连接你信任其提供方处理你账本的客户端。
- 未启用登录时，任何能访问该端口的人都能通过这个端点读取账本，就像通过 API 的其他部分一样。请启用登录，或把张记账绑定到 localhost。
- 插件不能写入、不能访问网络、不能读取文件；它不持有凭据（张记账从不把 `Authorization` 请求头传给插件）。
- 其他来源的网页不能凭你浏览器的会话调用这个端点：除非你列出了那个来源，`Origin` 检查会拒绝它。
