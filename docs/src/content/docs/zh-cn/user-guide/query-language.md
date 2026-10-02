---
title: 查询语言
description: 张记账兼容 BQL 的查询语言参考，包括语法、postings 表、类型、函数、HTTP API 以及与 Beancount 查询语言的差异。
---

<!-- TODO(lead): keep this page in sync with the English page (user-guide/query-language.md). Resolve the TODO(lead) comments there first, then mirror the resulting changes here. -->

张记账提供了一门小型查询语言，用来临时回答关于账本的各种问题。它支持 [Beancount 查询语言（BQL）](https://beancount.github.io/docs/beancount_query_language/)的一个子集，为 Beancount 或 Fava 编写的大多数 `SELECT` 查询无需修改即可使用。当 BQL v2 与其后继者 [beanquery](https://github.com/beancount/beanquery) 行为不一致时，张记账以 beanquery 为准。

查询直接在张记账已加载到内存中的账本上执行。查询是只读的，所有运算都使用精确的十进制数，金额永远不会经过浮点数转换。

:::caution[早期版本]
本页描述的是查询语言的第一个版本（[#434](https://github.com/zhang-accounting/zhang/issues/434) 的第一阶段）：只支持在单一的 `postings` 表上执行 `SELECT` 查询。尚未支持的功能见[与 BQL 和 beanquery 的差异](#与-bql-和-beanquery-的差异)。
:::

## 运行查询

### 在网页界面中

打开 `/explore` 的 **查询** 页面（英文界面中为 **Query**）。

- 在编辑器中输入查询。只有点击运行按钮或按下 <kbd>Ctrl</kbd>+<kbd>Enter</kbd>（macOS 上为 <kbd>Cmd</kbd>+<kbd>Enter</kbd>）时才会执行，输入过程中不会自动执行。
- 结果以表格展示，每个单元格按其[类型](#类型)渲染。库存（Inventory）单元格每行显示一个持仓。
- 查询出错时，编辑器会高亮出错的行和列。
- 示例菜单可以插入现成的查询，参考面板列出了所有列和函数。

### 通过 HTTP

将查询发送到 `POST /api/query`。请求和响应格式见 [HTTP API](#http-api)。

## 第一个查询

```sql
SELECT date, payee, account, position
WHERE account ~ '^Expenses:Food'
ORDER BY date DESC
LIMIT 10
```

这个查询返回 `Expenses:Food` 及其子账户最近的十条分录。

- 每一行是一条分录（posting）。`date` 和 `payee` 来自交易，`account` 和 `position` 来自分录本身。
- `~` 在文本的任意位置匹配正则表达式，并且不区分大小写。`^` 把模式锚定在账户名的开头。注意 `'^Expenses:Food'` 也会匹配 `Expenses:Foodstuff`，如果只想匹配该账户及其子账户，请使用 `'^Expenses:Food(:|$)'`。
- `ORDER BY date DESC` 让最新的分录排在最前，`LIMIT 10` 只保留前十行。

## 查询语法

```text
SELECT [DISTINCT] target [, target ...] | *
  [FROM expression]
  [WHERE expression]
  [GROUP BY group_key [, group_key ...]]
  [ORDER BY order_key [ASC | DESC] [, order_key [ASC | DESC] ...]]
  [LIMIT count]

target    = expression [AS name]
group_key = expression | target name | target number
order_key = expression | target name | target number
```

- 各子句必须按上面的顺序出现。除 `SELECT` 外都是可选的。
- 关键字不区分大小写：`select`、`SELECT` 和 `Select` 是同一个关键字。
- 词法单元之间的空格和换行没有意义，一个查询可以分成多行书写。
- 列名和函数名使用小写，与本页表格中的写法一致。

### 查询的执行过程

1. `FROM` 和 `WHERE` 决定哪些分录参与计算。
2. 如果查询使用了[聚合函数](#聚合函数)，被选中的分录会被分组，每组产生一行；否则每条分录产生一行。
3. `ORDER BY` 对结果行排序。
4. `DISTINCT` 去除重复的行。
5. `LIMIT` 保留前面的若干行，丢弃其余的行。

### SELECT

目标（target）是要为每一行计算的表达式，用逗号分隔。

- `SELECT *` 是 `SELECT date, flag, payee, narration, account, position` 的简写。
- `AS name` 为目标命名。这个名字可以在 `GROUP BY` 和 `ORDER BY` 中使用，但不能在 `WHERE` 中使用。
- 结果列的名字依次取：别名（如果有）；所读取的列名（目标是单独的列时，如 `account`）；否则为表达式的原文（如 `sum(position)`）。
- `SELECT DISTINCT` 会去掉与之前某一行完全相同的行，只比较被选择的值。

### FROM

在第一阶段中，`FROM` 后面跟的是表达式而不是表名。它和 `WHERE` 一样过滤分录；当两者同时存在时，分录必须同时满足两个条件：

```sql
SELECT account, sum(position)
FROM year = 2024
WHERE account ~ '^Expenses'
GROUP BY account
```

等价于 `... WHERE (year = 2024) AND (account ~ '^Expenses') ...`。这与 beanquery 的行为一致，在 `FROM` 中做过滤的 BQL 查询因此可以继续使用。时间段子句 `OPEN ON`、`CLOSE ON` 和 `CLEAR` 暂不支持，替代写法见[变通方法](#变通方法)。

### WHERE

`WHERE` 保留条件为 `TRUE` 的分录。条件结果为 `NULL` 时视为不成立，该分录会被丢弃（见 [NULL](#null)）。`WHERE` 和 `FROM` 中不能使用聚合函数。

### GROUP BY

只要有一个目标使用了[聚合函数](#聚合函数)，查询就是聚合查询。此时分录会被分组，每组产生一行。

- 分组键可以是表达式、通过 `AS` 给目标起的名字，或目标在 `SELECT` 列表中的序号（从 1 开始）。`GROUP BY 1, 2` 表示按前两个目标分组。
- 存在聚合函数时，每个非聚合的目标都必须被分组，也就是说它必须出现在 `GROUP BY` 中，或者在那里通过名字或序号被引用。
- 分组键本身不能包含聚合函数，聚合函数也不能嵌套（`sum(count(*))` 会报错）。
- 如果一个目标把聚合函数和聚合函数之外的列混在一起，例如 `number - sum(number)`，会报错。只由聚合函数和常量构成的表达式，例如 `sum(number) / 12` 或 `units(sum(position))`，本身就是聚合表达式，可以正常使用。
- 如果所有目标都是聚合函数且没有 `GROUP BY`，所有匹配的分录组成一个分组。

<!-- TODO(lead): confirm the GROUP BY rule against beanquery's actual behaviour; see the detailed TODO in the English page (implicit GROUP BY when it is omitted, exact-set check, hidden ORDER BY targets, unhashable types, zero rows for an all-aggregate query with no matches). -->

```sql
SELECT root(account, 2) AS category, sum(position) AS total
WHERE account ~ '^Expenses'
GROUP BY category
```

### ORDER BY

- 排序键和分组键一样，可以是表达式、目标名字或目标序号。
- 每个排序键有自己的方向：`ASC`（升序，默认）或 `DESC`（降序）。在 `ORDER BY 1 DESC, 2` 中，第一个键降序，第二个键升序。
- 先按第一个键比较，第一个键相同时才比较第二个键，依此类推。
- `NULL` 比其他任何值都小：升序时排在最前，降序时排在最后。
- 各类型的值如何排序见[排序与比较](#排序与比较)。
- 没有 `ORDER BY` 时，普通查询按账本顺序（按日期）返回分录；聚合查询中各分组的顺序不确定。只要顺序有意义，就请加上 `ORDER BY`。

### LIMIT

`LIMIT n` 在排序和 `DISTINCT` 之后保留前 `n` 行。`n` 必须是非负整数字面量。

## 字面量

| 字面量 | 示例 | 类型 |
|--------|------|------|
| 字符串 | `'Food'`、`"USD"` | `str` |
| 整数 | `0`、`42` | `int` |
| 小数 | `3.14`、`0.5` | `decimal` |
| 日期 | `2024-01-31` | `date` |
| 布尔值 | `TRUE`、`FALSE` | `bool` |
| 空值 | `NULL` | `null` |

- 字符串可以用单引号或双引号。与标准 SQL 不同，`"USD"` 是字符串而不是列名。如果字符串中需要包含引号，请用另一种引号把它括起来：`"Joe's Diner"`。
- 日期写作 `YYYY-MM-DD`，**不加**引号。加了引号的 `'2024-01-31'` 是字符串，不是日期。
- 没有负数字面量。`-5` 是对 `5` 使用一元负号。
- `TRUE`、`FALSE` 和 `NULL` 是关键字，不区分大小写。

## 运算符

按优先级从高到低：

| 优先级 | 运算符 | 含义 |
|--------|--------|------|
| 1 | `-x` | 一元负号 |
| 2 | `*` `/` | 乘、除 |
| 3 | `+` `-` | 加、减 |
| 4 | `=` `!=` `<` `<=` `>` `>=` | 比较 |
| 4 | `~` `!~` | 正则匹配、正则不匹配 |
| 4 | `IN` `NOT IN` | 成员测试 |
| 4 | `IS NULL` `IS NOT NULL` | 空值测试 |
| 5 | `NOT` | 逻辑非 |
| 6 | `AND` | 逻辑与 |
| 7 | `OR` | 逻辑或 |

可以用括号显式分组：`(a OR b) AND c`。由于 `NOT` 的优先级低于比较运算，`NOT account ~ '^Assets'` 的含义是 `NOT (account ~ '^Assets')`。

### 算术运算

`+`、`-`、`*` 和 `/` 作用于 `int` 和 `decimal`。

- 两个 `int` 做 `+`、`-`、`*` 运算，结果是 `int`。
- 只要有一边是 `decimal`，结果就是 `decimal`。
- `/` 的结果总是 `decimal`，即使两边都是整数：`7 / 2` 等于 `3.5`。
- 除以零得到 `NULL`，而不是报错。
- 小数的加、减、乘都是精确的，不会舍入。

### 比较运算

`=`、`!=`、`<`、`<=`、`>` 和 `>=` 比较两个同类型的值，`int` 和 `decimal` 之间也可以比较。字符串比较区分大小写，只有 `~` 忽略大小写。

### 正则匹配

当正则表达式 `pattern` 匹配 `text` 的**任意一部分**（忽略大小写）时，`text ~ pattern` 为 `TRUE`。`text !~ pattern` 与之相反。

| 表达式 | 结果 |
|--------|------|
| `'Expenses:Food:Dining' ~ 'food'` | `TRUE`，部分匹配且忽略大小写 |
| `'Expenses:Food:Dining' ~ '^Food'` | `FALSE`，因为 `^` 锚定在开头 |
| `'Expenses:Food:Dining' ~ 'Dining$'` | `TRUE` |
| `'Expenses:Food:Dining' !~ '^Income'` | `TRUE` |

需要完整匹配时，请用 `^` 和 `$` 锚定模式。

### 成员测试

`IN` 测试一个值是否属于某个集合或列表：

```sql
WHERE 'trip-new-york' IN tags
WHERE 'Assets:Cash' NOT IN other_accounts
WHERE account IN ('Assets:Cash', 'Assets:Bank:Checking')
WHERE year IN (2023, 2024)
```

右侧可以是 `set` 类型的值（例如 `tags`、`links` 或 `other_accounts` 列），也可以是括号中的字面量列表。`NOT IN` 与 `IN` 相反。

### NULL

缺失的值为 `NULL`，例如没有收款方的交易的 `payee`，或者没有按成本持有的分录的成本。

- 算术运算、比较、`~`、`!~`、`IN` 和 `NOT IN` 只要有一个操作数为 `NULL`，结果就是 `NULL`。
- `WHERE` 和 `FROM` 把 `NULL` 视为不成立，相应的分录会被丢弃。
- 请用 `IS NULL` 和 `IS NOT NULL` 判断缺失值。`payee = NULL` 的结果永远是 `NULL`，不会是 `TRUE`。
- 注意 `payee != 'Shop'` 会丢弃 `payee` 为 `NULL` 的分录。如需保留，请写成 `payee IS NULL OR payee != 'Shop'`。

## postings 表

第一阶段只有一张表 `postings`。每笔交易的每条分录对应一行，交易的字段会重复出现在它的每条分录上。

- **包含：**所有交易（无论标记是什么），以及张记账为 `balance ... with pad ...` 指令生成的补齐交易，这些交易的标记为 `P`。
- **不包含：**余额断言，以及所有非交易指令，例如 `open`、`close`、`price`、`note`、`document` 和预算指令。
- 没有写金额的分录，使用张记账在平衡交易时推断出的金额。

### 列

| 列 | 类型 | 说明 |
|----|------|------|
| `date` | `date` | 交易日期。如果交易带有时间，时间部分会被舍去。 |
| `year` | `int` | `date` 的年份。 |
| `month` | `int` | `date` 的月份，1 到 12。 |
| `day` | `int` | `date` 在当月的日，1 到 31。 |
| `flag` | `str` | 交易的标记，例如 `*`、`!` 或 `P`。 |
| `payee` | `str` | 交易的收款方，没有则为 `NULL`。 |
| `narration` | `str` | 交易的描述，没有则为 `NULL`。 |
| `description` | `str` | 用 `" \| "` 连接收款方和描述。如果缺少其中一个，只使用另一个。 |
| `tags` | `set` | 交易的标签，不含开头的 `#`。 |
| `links` | `set` | 交易的链接，不含开头的 `^`。 |
| `id` | `str` | 交易的标识符，同一交易的所有分录共享这个值。 |
| `account` | `str` | 分录的账户。 |
| `number` | `decimal` | 分录的单位数量。 |
| `currency` | `str` | 单位的货币（商品）。 |
| `position` | `position` | 分录的单位及其成本批次（如果有）。 |
| `cost_number` | `decimal` | 单位成本，未按成本持有则为 `NULL`。 |
| `cost_currency` | `str` | 成本的货币，或 `NULL`。 |
| `cost_date` | `date` | 成本批次的日期，或 `NULL`。 |
| `cost_label` | `str` | 成本批次的标签，或 `NULL`。 |
| `price` | `amount` | 用 `@` 写出的单价，没有则为 `NULL`。 |
| `weight` | `amount` | 分录在交易平衡中所占的金额：按成本持有时为总成本；否则如果有价格，为单位数量乘以价格；否则为单位本身。 |
| `other_accounts` | `set` | 同一交易中其他分录的账户。 |

## 类型

| 类型 | 说明 | 示例 |
|------|------|------|
| `null` | 缺失的值。 | `NULL` |
| `bool` | `TRUE` 或 `FALSE`。 | `TRUE` |
| `int` | 整数。 | `2024` |
| `decimal` | 任意精度的精确十进制数。 | `12.50` |
| `str` | 文本。 | `'Expenses:Food'` |
| `date` | 日历日期。 | `2024-01-31` |
| `set` | 无序的字符串集合，用于 `tags`、`links` 和 `other_accounts`。 | `{'trip', 'food'}` |
| `amount` | 带货币的十进制数，即金额。 | `12.50 USD` |
| `position` | 持仓：单位（一个金额）加上可选的成本批次。成本批次包含单位成本的数值和货币，还可以有日期和标签。 | `10 VTI {120.00 USD, 2024-01-02, "lot-a"}` |
| `inventory` | 库存：由多个持仓组成，可以包含任意多种货币和成本批次。 | `-30.00 USD, 10 VTI {120.00 USD}` |

持仓如何合并为库存：

- `sum(position)` 把所有持仓加到同一个库存中。
- 货币相同且成本批次相同的持仓会合并，数量相加。成本批次不同的持仓保持独立，因此以不同价格买入的持仓仍然分开显示。
- 数量变为零的持仓会被移除，所以余额为零的账户得到一个空库存。

`int` 与 `decimal` 一起运算时会被转换为 `decimal`，此外不会发生其他隐式转换。在 `position`、`amount` 和 `inventory` 之间转换请使用[估值函数](#估值函数)。

### 排序与比较

同类型的值按以下规则排序，适用于 `ORDER BY`、`min`、`max` 和比较运算符。

- `int` 和 `decimal` 按数值排序；`str` 按字符编码排序（因此区分大小写）；`date` 按时间先后排序；`FALSE` 排在 `TRUE` 之前。
- `amount` 先按货币、再按数值排序。
- `position` 和 `inventory` 按其中的持仓以同样的方式排序。因此只含一种货币的库存按数值排序，[按收款方统计支出](#按收款方统计支出)示例中的 `ORDER BY total DESC` 正是依赖这一点。

## 函数

`x` 表示类型为签名中所注明类型的参数，方括号中的参数是可选的。除非另有说明，参数为 `NULL` 时函数返回 `NULL`。

### 聚合函数

聚合函数把一个分组中所有分录的值合并成一个值，见 [GROUP BY](#group-by)。

| 函数 | 返回 | 说明 |
|------|------|------|
| `count(*)` | `int` | 分组中的分录数。 |
| `count(x)` | `int` | 分组中 `x` 不为 `NULL` 的分录数。 |
| `sum(x: decimal)` | `decimal` | 各值之和，跳过 `NULL`。 |
| `sum(x: amount)` | `inventory` | 各金额之和，按货币分开。 |
| `sum(x: position)` | `inventory` | 各持仓之和，成本批次按[类型](#类型)中描述的方式合并。 |
| `sum(x: inventory)` | `inventory` | 各库存之和。 |
| `first(x)` | 与 `x` 相同 | 按账本顺序，分组中第一条分录的 `x`。 |
| `last(x)` | 与 `x` 相同 | 按账本顺序，分组中最后一条分录的 `x`。 |
| `min(x)` | 与 `x` 相同 | 最小的非 `NULL` 值。 |
| `max(x)` | 与 `x` 相同 | 最大的非 `NULL` 值。 |

`first` 和 `last` 按账本顺序计算，`ORDER BY` 不会改变哪条分录算作第一条。

### 估值函数

| 函数 | 返回 | 说明 |
|------|------|------|
| `units(x: position)` | `amount` | 持仓的单位，不含成本。 |
| `units(x: inventory)` | `inventory` | 每个持仓的单位，不含成本。同一货币的不同成本批次合并为一个持仓。 |
| `cost(x: position)` | `amount` | 持仓的总成本（单位数量乘以单位成本），以成本货币计。未按成本持有的持仓返回其单位。 |
| `cost(x: inventory)` | `inventory` | 对每个持仓应用 `cost` 后求和。 |
| `convert(x: amount, currency: str[, date: date])` | `amount` | 把 `x` 换算为 `currency`。 |
| `convert(x: position, currency: str[, date: date])` | `amount` | 把 `x` 的单位换算为 `currency`，忽略成本。 |
| `convert(x: inventory, currency: str[, date: date])` | `inventory` | 把每个持仓换算为 `currency` 后求和。 |
| `value(x: position[, date: date])` | `amount` | 持仓以成本货币计的市值。未按成本持有的持仓返回其单位。 |
| `value(x: inventory[, date: date])` | `inventory` | 对每个持仓应用 `value` 后求和。 |

价格的查找规则：

- 价格来自账本中的 `price` 指令。
- 提供了 `date` 参数时，使用日期不晚于该日期的最新价格；未提供时，使用账本中最新的价格。
- 价格可以双向使用。`price VTI 120 USD` 既可以按 120 把 VTI 换算为 USD，也可以按 1/120 把 USD 换算为 VTI。
- 找不到价格时，值保持不变：仍是原来的货币，不会被丢弃，也不会变成零。因此 `convert` 或 `value` 之后的库存仍可能包含多种货币。
- 把一个值换算为它本身的货币，会原样返回。

### 账户函数

| 函数 | 返回 | 说明 | 示例 |
|------|------|------|------|
| `root(account: str, n: int)` | `str` | 账户名的前 `n` 段。如果账户只有 `n` 段或更少，则原样返回。 | `root('Expenses:Food:Dining', 2)` 为 `'Expenses:Food'` |
| `parent(account: str)` | `str` | 去掉最后一段后的账户名。 | `parent('Expenses:Food:Dining')` 为 `'Expenses:Food'` |
| `leaf(account: str)` | `str` | 账户名的最后一段。 | `leaf('Expenses:Food:Dining')` 为 `'Dining'` |

### 日期函数

| 函数 | 返回 | 说明 | 示例 |
|------|------|------|------|
| `year(d: date)` | `int` | 年份。 | `year(2024-05-17)` 为 `2024` |
| `month(d: date)` | `int` | 月份，1 到 12。 | `month(2024-05-17)` 为 `5` |
| `quarter(d: date)` | `str` | 年份和季度。 | `quarter(2024-05-17)` 为 `'2024-Q2'` |
| `day(d: date)` | `int` | 当月的日。 | `day(2024-05-17)` 为 `17` |
| `today()` | `date` | 当前日期。 | |

`year`、`month` 和 `day` 列是简写：`year` 等同于 `year(date)`。

### 元数据函数

| 函数 | 返回 | 说明 |
|------|------|------|
| `meta(key: str)` | `str` | 分录上元数据 `key` 的值，未设置则为 `NULL`。 |
| `entry_meta(key: str)` | `str` | 交易上元数据 `key` 的值，未设置则为 `NULL`。 |

张记账把交易内的每一行元数据都存放在交易本身上，包括缩进在某条分录下面的行。请使用 `entry_meta` 读取它们。

### 其他函数

| 函数 | 返回 | 说明 | 示例 |
|------|------|------|------|
| `str(x)` | `str` | 任意值的文本形式。 | `str(2024-01-31)` 为 `'2024-01-31'` |
| `length(x: str)` | `int` | 字符串中的字符数。 | `length('Food')` 为 `4` |
| `length(x: set)` | `int` | 集合中的元素个数。 | `length(tags)` |

## HTTP API

查询页面使用的就是下面这些 HTTP 接口，你也可以在脚本中调用它们。如果启用了 [Basic Auth 认证](/zh-cn/installation/3-basic_auth/)，请携带与其他 API 相同的凭证。

### 执行查询

用 JSON 请求体调用 `POST /api/query`：

```shell
curl -X POST http://localhost:8000/api/query \
  -H 'Content-Type: application/json' \
  -d '{"query": "SELECT account, sum(position) WHERE account ~ \"^Assets:Bank\" GROUP BY account"}'
```

成功时返回 HTTP 状态码 200：

```json
{
  "data": {
    "columns": [
      { "name": "account", "type": "str" },
      { "name": "sum(position)", "type": "inventory" }
    ],
    "rows": [
      [
        "Assets:Bank:Checking",
        { "positions": [ { "units": { "number": "1520.35", "currency": "USD" }, "cost": null } ] }
      ]
    ]
  }
}
```

- `columns` 按顺序列出结果列，每列有 `name` 和一个[类型](#类型)名。
- `rows` 是行的列表。每行是一个列表，每列一个单元格，顺序与 `columns` 相同。

### 单元格编码

| 类型 | JSON | 示例 |
|------|------|------|
| `null` | `null` | `null` |
| `bool` | 布尔值 | `true` |
| `int` | 数字 | `2024` |
| `decimal` | 字符串 | `"1520.35"` |
| `str` | 字符串 | `"Expenses:Food"` |
| `date` | 字符串，`YYYY-MM-DD` | `"2024-01-31"` |
| `set` | 字符串数组 | `["trip", "food"]` |
| `amount` | 对象 | `{"number": "12.50", "currency": "USD"}` |
| `position` | 对象 | `{"units": {"number": "10", "currency": "VTI"}, "cost": {"number": "120.00", "currency": "USD", "date": "2024-01-02", "label": null}}` |
| `inventory` | 对象 | `{"positions": [ ...持仓... ]}` |

- 十进制数（包括金额和成本中的 `number` 字段）以字符串形式发送，以免损失精度。请用十进制数库解析，而不要解析为浮点数。
- 持仓未按成本持有时 `cost` 为 `null`；否则包含单位成本的 `number` 和 `currency`，以及成本批次的 `date` 和 `label`，这两者都可能为 `null`。
- 空库存为 `{"positions": []}`。
- 不要依赖 `set` 中元素的顺序。

### 错误

无法解析或执行的查询返回 HTTP 状态码 400。例如 `SELECT acount, position` 会返回：

```json
{
  "message": "unknown column 'acount'",
  "line": 1,
  "column": 8
}
```

- `line` 和 `column` 给出问题在查询文本中的位置，都从 1 开始。
- `column` 按字符而不是字节计数，一个汉字或带重音的字母算作一列。
- 错误包括语法错误、未知的列或函数、参数类型错误，以及不合法的 `GROUP BY` 用法。

### Schema

`GET /api/query/schema` 返回 `postings` 表的列及其类型，以及可用的函数及其签名。查询页面的参考面板就是根据这个响应生成的。

## 示例

### 按类别统计每月支出

```sql
SELECT year, month, root(account, 2), sum(position) WHERE account ~ "^Expenses" GROUP BY 1, 2, 3 ORDER BY 1, 2, 3
```

每个月、每个二级支出账户（如 `Expenses:Food`）一行，给出总支出。`GROUP BY 1, 2, 3` 和 `ORDER BY 1, 2, 3` 通过序号引用前三个目标。

### 按收款方统计支出

```sql
SELECT payee, sum(cost(position)) AS total WHERE account ~ "^Expenses" GROUP BY payee ORDER BY total DESC LIMIT 20
```

支出最多的 20 个收款方。`cost(position)` 让按成本购买的东西以实际付出的金额计算，`ORDER BY total DESC` 按别名排序。

### 按标签筛选分录

```sql
SELECT date, payee, account, position WHERE 'trip-new-york' IN tags
```

所有带有 `#trip-new-york` 标签的交易的每一条分录。

### 持仓的成本与市值

```sql
SELECT account, units(sum(position)) AS qty, cost(sum(position)) AS book, convert(units(sum(position)), "USD") AS market WHERE account ~ "^Assets:Trading" GROUP BY account
```

每个交易账户的持有数量、账面价值（买入成本）以及按最新价格换算的美元市值。没有美元价格的持仓在 `market` 列中保持原来的货币。

### 最近的分录

```sql
SELECT date, payee, account, position ORDER BY date DESC LIMIT 20
```

所有账户中最近的 20 条分录。

### 某段时间的支出（不含税费）

```sql
SELECT account, sum(position) AS total
WHERE account ~ '^Expenses' AND account !~ ':Taxes(:|$)'
  AND date >= 2024-01-01 AND date < 2024-04-01
GROUP BY account
ORDER BY account
```

用 `!~` 排除账户，用不带引号的日期字面量限定时间范围。

### 按季度统计并计数

```sql
SELECT quarter(date) AS q, count(*) AS postings, sum(number) AS total, sum(number) / count(*) AS average
WHERE account ~ '^Expenses:Food' AND currency = 'USD'
GROUP BY q
ORDER BY q
```

每个季度的分录数、总额和平均金额。`number` 不区分货币，所以要先按 `currency` 过滤，`sum(number)` 才有意义。

### 用信用卡支付的交易

```sql
SELECT DISTINCT date, description
WHERE 'Liabilities:CreditCard' IN other_accounts AND account ~ '^Expenses'
ORDER BY date DESC
```

`other_accounts` 是每笔交易中其他分录的账户，因此这个查询找出用信用卡支付的支出。即使一笔交易有多条支出分录，`DISTINCT` 也只列出一次。

### 没有收款方的分录

```sql
SELECT date, narration, account, position
WHERE payee IS NULL AND account IN ('Expenses:Misc', 'Expenses:Uncategorized')
ORDER BY date DESC
```

演示 `IS NULL` 和 `IN` 后面的列表。

### 记录在元数据中的发票

```sql
SELECT date, payee, entry_meta('invoice') AS invoice, position
WHERE entry_meta('invoice') IS NOT NULL AND leaf(account) = 'Consulting'
ORDER BY date
```

列出账户名以 `Consulting` 结尾、且交易带有 `invoice` 元数据的分录。

### 年末的投资组合市值

```sql
SELECT account, value(sum(position), 2024-12-31) AS market_value
WHERE account ~ '^Assets:Investments' AND date <= 2024-12-31
GROUP BY account
ORDER BY account
```

2024 年 12 月 31 日的持仓，按当日有效的价格、以各持仓的成本货币计算市值。

## 与 BQL 和 beanquery 的差异

### 尚未支持

- **`SELECT` 以外的语句：**不支持 `BALANCES`、`JOURNAL` 和 `PRINT`。
- **`FROM` 中的时间段子句：**不支持 `OPEN ON`、`CLOSE ON` 和 `CLEAR`，`FROM` 后面也不能跟表名或子查询。
- **`HAVING` 和 `PIVOT BY`。**
- **其他表：**只有 `postings` 表，没有 entries、prices、accounts、commodities、documents 或 balances 等表。
- **累计余额 `balance` 列。**
- **`query` 指令：**尚不能列出或执行保存在账本中的查询（`2024-01-01 query "name" "SELECT ..."`）。
- **本页未列出的函数和运算符**，例如 `getprice`、`abs`、`round`、`any_meta`、`has_account`、区分大小写的匹配 `?~`、`BETWEEN` 和 `%`。使用它们会报错。

[#434](https://github.com/zhang-accounting/zhang/issues/434) 中的路线图计划在下一阶段支持 `BALANCES`、`JOURNAL`、`OPEN`/`CLOSE`/`CLEAR`、`query` 指令和 CSV 导出，之后再支持 `HAVING`、`PIVOT BY` 和更多的表。

### 行为不同之处

- **错误带有位置信息。**每个查询错误都会给出出错的行和列。
- **`SELECT *` 包含 `account`。**beanquery 把 `*` 展开为 `date, flag, payee, narration, position`，张记账在 `position` 之前加入了 `account`。
- **元数据属于交易。**张记账没有单独的分录元数据，请使用 `entry_meta`，见[元数据函数](#元数据函数)。
- **全程使用精确小数。**数字是任意精度的十进制数，金额不会以固定的小数位数存储。

### 变通方法

在支持 `OPEN ON` 和 `CLOSE ON` 之前，可以改为按 `date` 过滤。

2024 年的损益表：

```sql
SELECT account, sum(position)
WHERE account ~ '^(Income|Expenses)' AND date >= 2024-01-01 AND date < 2025-01-01
GROUP BY 1
ORDER BY 1
```

2025 年 4 月 1 日开始时的资产和负债余额：

```sql
SELECT account, sum(position)
WHERE account ~ '^(Assets|Liabilities)' AND date < 2025-04-01
GROUP BY account
ORDER BY account
```

与 `CLOSE ON ... CLEAR` 不同，这些查询不会把收入和支出结转到权益账户，因此只对所选的账户等价。
