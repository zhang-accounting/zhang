---
title: MCP 服务器
description: 通过张记账内置的 MCP 插件 zhang-mcp，让 Claude 等 AI 助手读取你的账本，并记录你认可的条目：提供余额、支出、流水、BQL 和交易等工具。
sidebar:
  order: 10
---

[MCP](https://modelcontextprotocol.io/)（Model Context Protocol）是 Claude Code、Claude.ai 等 AI 助手和 LLM 客户端调用工具的协议。`zhang-mcp` 是张记账的 MCP 服务器，一个内置在张记账中的 [Lua 插件](/zh-cn/guides/lua-plugins/)：在账本里写两行就能启用，无需下载或复制任何文件。声明之后，助手就能通过运行和网页界面相同的查询来回答"上个月我在吃饭上花了多少"或"我的净资产是多少"这样的问题；如果你允许，它还能在向你展示将要写入的确切文本之后，记录你描述的条目。

读取是默认的，不需要别的设置。写入默认关闭，需要打开两个开关（见下文）；即使打开，插件也只能追加条目，永远不能修改或删除任何内容，而且行为良好的客户端会在每一条之前征求你的同意。

## 安装

启用插件并声明内置插件，写在账本的任意文件中：

```zhang
option "features.plugin" "true"

plugin "zhang:mcp"
  timeout: "20s"
```

重新加载。**设置**页面会列出 `zhang-mcp` 并附有**打开**链接；`GET /api/plugins/zhang-mcp/` 会描述这个服务器及其端点所在。

`zhang:mcp` 指的是内置在张记账程序里的那份插件。它不从账本的存储读取任何内容，所以放在本地目录的账本，和放在 S3 或 R2、WebDAV、GitHub 上的账本（往里面复制文件很麻烦）用起来完全一样。它的版本就是随你的张记账发布的版本，升级张记账即升级它。内置不会给它任何额外权限：它能读写的只有它的 `plugin` 指令所允许的内容，和一份脚本文件副本完全相同。想修改脚本，请[运行你自己的副本](#运行你自己的副本)。

MCP 端点是 `POST <你的张记账地址>/api/plugins/zhang-mcp/mcp`，例如 `http://localhost:8000/api/plugins/zhang-mcp/mcp`。

`plugin` 指令的设置：

| 元数据 | 含义 | 默认值 |
|---|---|---|
| `timeout` | 一个请求最多可以运行多久，见[插件](/zh-cn/guides/plugins/#授予能力) | 60 秒；`"20s"` 足够 |
| `allowed_origins` | 允许调用端点的浏览器来源（`scheme://host[:port]`），用逗号分隔（`"https://ledger.example.com, http://localhost:5173"`）或重复该键。不设置时，只有张记账自己在 `localhost` 或 `127.0.0.1` 上提供的页面可以调用；部署在域名或反向代理之后的张记账要在这里列出那个地址 | 无 |
| `max_rows` | 任何工具最多返回的行数 | `"1000"` |
| `allow_writes` | `"true"` 时列出三个写入工具（`preview_transaction`、`add_transaction`、`open_account`），见[记录条目](#记录条目) | `"false"` |
| `write_tag` | `add_transaction` 给每条记录的条目加上的标签，便于日后查找；`""` 关闭 | `"mcp"` |
| `allowed_writes` | 张记账自己的授权，每种一行：`"transaction"` 和 `"open"`。没有它，无论 `allow_writes` 怎么设，写入工具都无法写入 | 无 |

### 运行你自己的副本

想改变插件的行为（它给模型的指令、某个工具、某条查询），就改为运行脚本的一份副本，而不是内置的那份：

1. 获取 `mcp.lua`：从你所运行的张记账版本的 [release](https://github.com/zhang-accounting/zhang/releases) 下载 `zhang-mcp-<version>.lua`，或取该版本仓库中的 `plugins/mcp/mcp.lua`。把它放进账本目录，例如 `plugins/mcp.lua`；脚本通过账本的存储读取，所以存放在 S3、WebDAV 或 GitHub 上的账本也要把它上传到那里。
2. 用它的路径代替 `zhang:mcp` 来声明，设置保持不变：

   ```zhang
   plugin "plugins/mcp.lua"
     timeout: "20s"
   ```

对于本地磁盘上的账本，在 `zhang serve` 下编辑这个文件会重新加载账本。副本需要随它一起发布的张记账或更新的版本：它是一个 Lua 脚本，写入时还依赖该版本的插件写入能力。在更老的张记账上，账本会加载失败，错误中会指出脚本或缺失的宿主函数。在插件内置之前发布的张记账会对 `plugin "zhang:mcp"` 回答 "plugin module not found: zhang:mcp"；在那样的版本上请运行副本。

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

任何使用 Streamable HTTP 的客户端都使用同样的 URL 和请求头。Claude.ai 这类托管客户端从其提供方的网络访问服务器，所以张记账必须能从互联网通过 HTTPS 访问、启用登录，并且客户端要允许你设置一个固定的 `Authorization` 请求头（请查看其连接器设置；张记账不提供 OAuth 流程）。只支持 stdio 的客户端需要一个 stdio 到 HTTP 的桥接，例如 `mcp-remote`。

服务器同时支持当前的 MCP 修订版（2026-07-28）和现今客户端使用的早期版本（2025-03-26 到 2025-11-25）。它从不使用流式响应，也不保存会话，因此可以经过任何反向代理。

## 可以问什么

- "我的净资产是多少？"
- "九月我在吃饭上花了多少，按月分？"
- "显示 Assets:Bank:Checking 最近的十条记录。"
- "找出今年和 Landlord 的交易。"
- "哪些条目有错误？"
- "AAPL 的最新价格是多少？"
- "运行 `SELECT account, sum(position) WHERE under(account, 'Expenses') AND date >= 2026-01-01 GROUP BY account`。"
- 开启写入后："记一笔今天在 Blue Bottle 用现金买咖啡花的 4.50 CNY。"

助手会被告知这个账本是什么、金额是精确的小数、写 BQL 之前要先读 schema，开启写入时还会被告知在记录任何内容之前先把预览展示给你，所以它能自己组合这些工具。

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
| `preview_transaction` | 一笔交易将被写入的确切文本、文件、错误，以及（干净时的）`token`；不写入任何内容 | 和网页界面预览相同的校验 |
| `add_transaction` | 追加用该 `token` 预览过的那笔交易 | 和网页界面相同的追加 |
| `open_account` | 一次调用追加一条 `open` 指令；账户已以相同商品开户时不写入 | 同样的追加 |

这些具名查询就是应用的[内置查询](/zh-cn/reference/builtin-queries/)，原文照抄，所以这里的答案就是网页界面显示的答案。每个结果同时以文本和结构化 JSON 返回，采用 `POST /api/query` 的单元格编码：小数和日期是字符串，金额是 `{number, currency}`。失败的查询会以工具错误返回，并指出行和列，助手据此自行纠正。三个写入工具只在开启写入时存在。

## 记录条目

两个开关，缺一不可，都写在 `plugin` 指令里：

```zhang
plugin "zhang:mcp"
  timeout: "20s"
  allow_writes: "true"              ; 插件列出它的写入工具
  allowed_writes: "transaction"     ; 张记账允许这个插件追加交易
  allowed_writes: "open"            ; ……以及 open 指令
```

`allow_writes` 是插件自己的设置：没有它，写入工具根本不会被列出，所以只读部署永远不会把它们展示给模型。`allowed_writes` 是张记账的授权，和 `allowed_paths`、`allowed_hosts` 是同一类行：没有它，（`allow_writes` 打开时）工具会被列出，但每次调用都会回答应该添加的那一行。内置插件和副本的授权方式相同；只有在你信任副本的代码时才把授权给它：见 `plugin` 指令的[能力](/zh-cn/reference/directives/plugin/)。

一笔交易是这样被记录的：

1. 助手带着日期、收款人、摘要和分录调用 `preview_transaction`（金额是 `"4.50"` 这样的小数字符串；和网页界面一样，可以有一条分录不写金额；没写货币的分录使用记账本位币）。张记账用和网页界面预览完全相同的方式校验它，回答将要写入的文本、将要写入的文件（由 [`directive_output_path`](/zh-cn/guides/recording-transactions/#新条目写入的位置) 决定）、所有错误，以及只有在一切干净时才给出的 `token`。
2. 助手把这段文本展示给你并询问。指令文本要求它这样做；Claude Code、Claude Desktop 等客户端在运行非只读工具之前也会自己向你确认。
3. 你同意后，它用相同的参数和 token 调用 `add_transaction`。张记账检查这个 token 正是它为当前账本状态下这段确切文本签发的那一个，然后追加文本并重新加载。条目带有 `#mcp` 标签（`write_tag`），所以用 `transactions` 加 `tags: ["mcp"]`，或者流水页的标签筛选，就能找到助手记录的全部条目。

token 的有效期是十分钟，并且只对一个账本状态有效：在此期间写入的任何内容（你自己、网页界面或另一个助手写的）都会使它失效，`add_transaction` 会回答"账本已变化，请重新运行 `preview_transaction`"。助手随后会重新预览并把新文本展示给你。token 不能重复使用：条目写入之后，同一个 token 会被拒绝。

账户永远不会被顺手创建。预览报告 `AccountDoesNotExist` 时，助手被要求先问你，然后用 `open_account` 显式开户（客户端同样会请你批准），再重新预览。账户已以相同商品开户时，`open_account` 不写入任何内容。

## 限制

- `limit` 默认 200 行；请求超过 `max_rows`（1000）会得到一个指出上限的错误。被截断的结果会标明 `truncated: true`；助手可以提高 `limit`、缩小查询，或者对 `journal`、`transactions` 和 `errors` 用 `offset` 翻页。
- 每条查询有和查询页面相同的 10 秒时间限制和结果大小限制；整个请求受插件的 `timeout` 限制。
- 超过 1 MiB 的请求体会被拒绝。
- 每次调用写入一笔交易或一条 `open`；预览 token 存活十分钟；每个重新加载周期只提交一条条目（下一条要等重新加载完成，这只需片刻）。

## 排错

| 现象 | 原因 | 处理 |
|---|---|---|
| `401 {"message": "unauthorized"}` | 启用了登录，而客户端没有发送有效的 `Authorization` 请求头 | 按上文加上 `--header`；检查 `user:password` 的 base64 |
| 每个请求都 `403` | 客户端是一个浏览器页面，其来源既不是张记账自己的 `localhost` 地址，也没有被列出；经过会改写 `Host` 的反向代理、或通过域名或局域网地址访问的张记账同样需要列出 | 把该页面的来源加入 `allowed_origins` |
| `405` | 客户端尝试打开 GET 流（较老的客户端可能尝试一次，然后继续用 POST） | 无需处理：服务器不提供流 |
| `504 {"message": "plugin zhang-mcp ran longer than its timeout"}` | 一条查询运行超过了插件的 `timeout` | 提高 `timeout`，或缩小查询 |
| 账本加载失败，错误提到 `mcp.lua` 或某个宿主函数 | 张记账比插件旧 | 升级张记账，或使用你的张记账版本自带的插件 |
| 账本加载失败："plugin module not found: zhang:mcp" | 这个张记账发布于插件内置之前 | 升级张记账，或[运行](#运行你自己的副本)你的张记账版本自带的插件副本 |
| 客户端说服务器需要 OAuth | 在配置请求头之前它收到了 `401` | 先配置请求头；张记账的 `401` 不带 OAuth 元数据 |
| 助手说它没有记录交易的工具 | `allow_writes` 关闭（打开时写入工具总会被列出） | 加上[记录条目](#记录条目)中的那几行并重新加载 |
| 助手说张记账不允许插件写入交易或开户 | `allow_writes` 已打开，但缺少对应种类的 `allowed_writes` 行 | 加上错误信息中指出的那一行并重新加载 |
| `add_transaction` 回答账本已变化 | 预览之后有内容被写入，或 token 超过十分钟，或它已被用过 | 让助手重新预览并把新文本展示给你 |

## 安全

- 工具返回的一切，包括收款人、摘要和元数据，都会发送给助手的模型。只连接你信任其提供方处理你账本的客户端。
- 未启用登录时，任何能访问该端口的人都能通过这个端点读取账本，就像通过 API 的其他部分一样；开启写入后还能向账本追加内容。请启用登录，或把张记账绑定到 localhost。
- 写入只能追加：通过这个服务器无法修改或删除任何内容，每条条目都像在网页界面中输入的一样经过校验，文件由账本自己的策略决定。token 把一次提交绑定到预览过的确切文本；标签标记这些条目；张记账的日志记录每次提交。
- 插件不能访问网络、不能读取文件；它不持有凭据（张记账从不把 `Authorization` 请求头传给插件）。
- 网页不能凭你浏览器的会话调用这个端点：`Origin` 检查只放行张记账自己在 `localhost` 上提供的页面和你在 `allowed_origins` 中列出的来源，所以恶意网站无法通过 DNS 重绑定访问你机器上的张记账。
