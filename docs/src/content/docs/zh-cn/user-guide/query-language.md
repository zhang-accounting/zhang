---
title: 查询语言
description: 张记账兼容 BQL 的查询语言参考，包括语法、postings 表、类型、函数、HTTP API 以及与 Beancount 查询语言的差异。
---

张记账提供了一门小型查询语言，用来临时回答关于账本的各种问题。它支持 [Beancount 查询语言（BQL）](https://beancount.github.io/docs/beancount_query_language/)的一个子集，为 Beancount 或 Fava 编写的大多数 `SELECT` 查询无需修改即可使用。当 BQL v2 与其后继者 [beanquery](https://github.com/beancount/beanquery) 行为不一致时，张记账以 beanquery 为准，本页末尾列出的少数有意为之的差异除外。

查询直接在张记账已加载到内存中的账本上执行。查询是只读的，所有运算都使用精确的十进制数，金额永远不会经过浮点数转换。

:::caution[早期版本]
本页描述的是查询语言的第一个版本（[#434](https://github.com/zhang-accounting/zhang/issues/434) 的第一阶段）：只支持在单一的 `postings` 表上执行 `SELECT` 查询。尚未支持的功能见[与 BQL 和 beanquery 的差异](#与-bql-和-beanquery-的差异)。
:::

## 运行查询

### 在网页界面中

打开 `/explore` 的 **查询** 页面（英文界面中为 **Query**）。

- 在编辑器中输入查询。只有点击运行按钮或按下 <kbd>Ctrl</kbd>+<kbd>Enter</kbd>（macOS 上为 <kbd>Cmd</kbd>+<kbd>Enter</kbd>）时才会执行，输入过程中不会自动执行。
- 结果以表格展示，每个单元格按其[类型](#类型)渲染。库存（inventory）单元格每行显示一个持仓。
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
  [;]

target    = expression [AS name]
group_key = expression | target name | target number
order_key = expression | target name | target number
```

- 各子句必须按上面的顺序出现。除 `SELECT` 外都是可选的，末尾可以有一个 `;`。
- 关键字、列名和函数名都不区分大小写：`SELECT account`、`select ACCOUNT` 和 `Select Account` 是同一个查询。
- 名字由 ASCII 字母、数字和下划线组成，不能以数字开头。`SELECT`、`DISTINCT`、`FROM`、`WHERE`、`GROUP`、`BY`、`ORDER`、`ASC`、`DESC`、`LIMIT`、`AS`、`AND`、`OR`、`NOT`、`IN`、`IS`、`NULL`、`TRUE`、`FALSE`、`HAVING` 和 `PIVOT` 是保留字，不能用作列名。
- 词法单元之间的空格和换行没有意义，一个查询可以分成多行书写。
- `--` 开始一段注释，直到行尾。

### 查询的执行过程

1. `FROM` 和 `WHERE` 决定哪些分录参与计算。
2. 如果查询使用了[聚合函数](#聚合函数)或带有 `GROUP BY` 子句，被选中的分录会被分组，每组产生一行；否则每条分录产生一行。
3. `ORDER BY` 对结果行排序。
4. `DISTINCT` 去除重复的行，每组重复只保留第一行。
5. `LIMIT` 保留前面的若干行，丢弃其余的行。

### SELECT

目标（target）是要为每一行计算的表达式，用逗号分隔。

- `SELECT *` 是 `SELECT date, flag, payee, narration, account, position` 的简写。
- `AS name` 为目标命名。这个名字可以在 `GROUP BY` 和 `ORDER BY` 中使用，但不能在 `WHERE` 中使用。
- 结果列有别名时以别名命名；否则以目标的原文命名（去掉首尾空格），与书写完全一致，例如 `account`、`ACCOUNT`、`sum(position)` 或 `sum( position )`。`SELECT *` 的各列使用上面列出的小写名字。
- `SELECT DISTINCT` 会去掉与之前某一行完全相同的行，只比较被选择的值。

### FROM

在第一阶段中，`FROM` 后面跟的是表达式而不是表名。它和 `WHERE` 一样过滤分录；当两者同时存在时，分录必须同时满足两个条件：

```sql
SELECT account, sum(position)
FROM year = 2024
WHERE account ~ '^Expenses'
GROUP BY account
```

等价于 `... WHERE (year = 2024) AND (account ~ '^Expenses') ...`。这与 beanquery 的行为一致，在 `FROM` 中做过滤的 BQL 查询因此可以继续使用。

查询总是读取 `postings` 表。在 `FROM` 中写表名（如 `FROM postings` 或 `FROM #postings`）会报错，时间段子句 `OPEN ON`、`CLOSE ON` 和 `CLEAR` 也会报错，替代写法见[变通方法](#变通方法)。

### WHERE

`WHERE` 保留条件为 `TRUE` 的分录。条件结果为 `NULL` 时视为不成立，该分录会被丢弃（见 [NULL 与三值逻辑](#null-与三值逻辑)）。`WHERE` 和 `FROM` 的条件必须是布尔表达式，两者中都不能使用聚合函数。

### GROUP BY

只要有一个目标使用了[聚合函数](#聚合函数)，或者查询带有 `GROUP BY` 子句，查询就是聚合查询。此时分录会被分组，每组产生一行。

- 分组键可以是表达式、目标名（别名，或者 `account` 这类目标的原文），或目标在 `SELECT` 列表中的序号（从 1 开始）。`GROUP BY 1, 2` 表示按前两个目标分组。名字的匹配不区分大小写。
- 分组键不必出现在 `SELECT` 中。`SELECT sum(position) GROUP BY account` 为每个账户返回一个不带标签的合计。
- **没有 `GROUP BY` 时**，聚合查询按所有非聚合的目标分组。`SELECT account, sum(position)` 等价于 `SELECT account, sum(position) GROUP BY account`。如果所有目标都是聚合函数，所有匹配的分录组成一个分组。
- **有 `GROUP BY` 时**，每个在聚合函数之外读取了列的目标都必须是分组键。`SELECT account, payee, count(*) GROUP BY account` 会报错，因为 `payee` 没有被分组。不使用任何列的目标（例如常量）不需要分组。
- 在聚合查询中，`ORDER BY` 里的非聚合表达式也必须是分组键。
- `set` 类型（如 `tags`）和 `inventory` 类型的值不能作为分组键。
- 分组键本身不能包含聚合函数，聚合函数也不能嵌套（`sum(count(*))` 会报错）。
- 同一个目标不能把聚合函数和聚合函数之外的列混在一起，即使这个列已经被分组。`number - sum(number)` 和 `possign(sum(position), account)` 都会报错，后者应写成 `sum(possign(position, account))`。只由聚合函数和常量构成的表达式，例如 `sum(number) / 12` 或 `units(sum(position))`，本身就是聚合表达式，可以正常使用。
- 没有匹配任何分录的聚合查询不返回任何行。即使是 `SELECT count(*) WHERE FALSE`，结果也是空的，而不是一行 `0`。

```sql
SELECT root(account, 2) AS category, sum(position) AS total
WHERE account ~ '^Expenses'
GROUP BY category
```

### ORDER BY

- 排序键和分组键一样，可以是表达式、目标名或目标序号。没有被选择的表达式也可以用来排序，只是不会出现在结果中。
- 每个排序键有自己的方向：`ASC`（升序，默认）或 `DESC`（降序）。在 `ORDER BY 1 DESC, 2` 中，第一个键降序，第二个键升序。
- 先按第一个键比较，第一个键相同时才比较第二个键，依此类推。所有键都相同的行保持原来的顺序。
- `NULL` 比其他任何值都小：升序时排在最前，降序时排在最后。
- 各类型的值如何排序见[排序与比较](#排序与比较)。
- 没有 `ORDER BY` 时，普通查询按账本顺序返回分录：先按日期和时间，再按它们在文件中出现的顺序。聚合查询按各分组第一条分录在账本中出现的顺序返回。

### LIMIT

`LIMIT n` 在排序和 `DISTINCT` 之后保留前 `n` 行。`n` 必须是非负整数字面量。`LIMIT 0` 不返回任何行。

## 字面量

| 字面量 | 示例 | 类型 |
|--------|------|------|
| 字符串 | `'Food'`、`"USD"` | `str` |
| 整数 | `0`、`42` | `int` |
| 小数 | `3.14`、`0.5`、`.5` | `decimal` |
| 日期 | `2024-01-31` | `date` |
| 布尔值 | `TRUE`、`FALSE` | `bool` |
| 空值 | `NULL` | `null` |

- 字符串可以用单引号或双引号。与标准 SQL 不同，`"USD"` 是字符串而不是列名。
- 字符串在下一个同类引号处结束，没有转义序列。如果字符串中需要包含引号，请用另一种引号把它括起来：`"Joe's Diner"` 或 `'say "hi"'`。
- 日期写作 `YYYY-MM-DD`，**不加**引号。`2024-13-01` 这样无效的日期会报错。
- 带引号的字符串与日期比较时会被当作日期读取，所以 `date >= '2024-01-01'` 也可以使用。此时如果字符串不是有效的 `YYYY-MM-DD` 日期，会报错。
- 整数是 64 位的，更大的整数字面量会成为 `decimal`。不支持 `1e3` 这样的指数写法。
- `TRUE`、`FALSE` 和 `NULL` 是关键字，不区分大小写。

## 运算符

按优先级从高到低：

| 优先级 | 运算符 | 含义 |
|--------|--------|------|
| 1 | `-x` `+x` | 一元负号、一元正号 |
| 2 | `*` `/` | 乘、除 |
| 3 | `+` `-` | 加、减 |
| 4 | `=` `==` `!=` `<>` `<` `<=` `>` `>=` | 比较 |
| 4 | `~` `!~` `?~` | 正则匹配 |
| 4 | `IN` `NOT IN` | 成员测试 |
| 4 | `IS NULL` `IS NOT NULL` | 空值测试 |
| 5 | `NOT` | 逻辑非 |
| 6 | `AND` | 逻辑与 |
| 7 | `OR` | 逻辑或 |

- 可以用括号显式分组：`(a OR b) AND c`。
- 由于 `NOT` 的优先级低于比较运算，`NOT account ~ '^Assets'` 的含义是 `NOT (account ~ '^Assets')`。
- 第 4 级的运算符不能连用：`a = b = c` 会报错。

### 算术运算

| 表达式 | 结果 | 说明 |
|--------|------|------|
| `int` `+ - *` `int` | `int` | 溢出时报错。 |
| `int` `/` `int` | `decimal` | `7 / 2` 等于 `3.5`。 |
| `int` 或 `decimal` `+ - * /` `int` 或 `decimal` | `decimal` | |
| `date` `+` `int`、`int` `+` `date`、`date` `-` `int` | `date` | 加上或减去天数：`2024-01-31 + 1` 等于 `2024-02-01`。 |
| `date` `-` `date` | `int` | 两个日期相差的天数。 |
| `str` `+` `str` | `str` | 拼接字符串。 |
| `amount` `*` 数值、数值 `*` `amount` | `amount` | 例如 `units(position) * 2`。 |
| `amount` `/` 数值 | `amount` | |
| `amount` `+ -` `amount` | `amount` | 两个金额的货币必须相同，否则查询失败。 |

- 加、减、乘都是精确的。乘积保留两个操作数的全部小数位：`1000.00 * 1` 等于 `1000.00`。
- 结果不超过 28 位有效数字时，除法是精确的；否则像 Beancount 一样四舍六入五成双，保留 28 位有效数字：`1 / 3` 等于 `0.3333333333333333333333333333`。
- 除以零得到 `NULL`，而不是报错。
- 一元负号可用于 `int`、`decimal`、`amount`、`position` 和 `inventory`。
- 不支持取余运算 `%`。

### 比较运算

- `=`（或 `==`）和 `!=`（或 `<>`）比较两个同类型的值。`int` 和 `decimal` 之间可以按数值比较：`1 = 1.00` 为 `TRUE`。
- `<`、`<=`、`>` 和 `>=` 只能用于 `bool`、`int`、`decimal`、`str` 和 `date`。比较金额时，请用 [`number`](#金额与数值) 取出数值再比较。
- 字符串比较区分大小写，按字符编码比较，所以 `'B' < 'a'`。只有 `~` 和 `!~` 忽略大小写。

### 正则匹配

| 运算符 | 含义 |
|--------|------|
| `text ~ pattern` | 当 `pattern` 匹配 `text` 的**任意一部分**（忽略大小写）时为 `TRUE`。 |
| `text !~ pattern` | 与 `~` 相反。 |
| `text ?~ pattern` | 与 `~` 相同，但区分大小写。 |

| 表达式 | 结果 |
|--------|------|
| `'Expenses:Food:Dining' ~ 'food'` | `TRUE`，部分匹配且忽略大小写 |
| `'Expenses:Food:Dining' ~ '^Food'` | `FALSE`，因为 `^` 锚定在开头 |
| `'Expenses:Food:Dining' ~ 'Dining$'` | `TRUE` |
| `'Expenses:Food:Dining' !~ '^Income'` | `TRUE` |
| `'Expenses:Food:Dining' ?~ 'food'` | `FALSE`，因为 `?~` 区分大小写 |

- 需要完整匹配时，请用 `^` 和 `$` 锚定模式。
- 模式使用 Rust [`regex`](https://docs.rs/regex/latest/regex/#syntax) 库的语法。它与 Python 的语法很接近，但不支持环视（`(?=...)`、`(?!...)`）和反向引用。
- 无效的模式会报错，错误位置指向该模式。
- 两边都必须是字符串。任一边为 `NULL` 时结果为 `NULL`，`~` 和 `!~` 都是如此。

### 成员测试

`IN` 测试一个值是否属于某个集合或列表：

```sql
WHERE 'trip-new-york' IN tags
WHERE 'Assets:Cash' NOT IN other_accounts
WHERE account IN ('Assets:Cash', 'Assets:Bank:Checking')
WHERE year IN (2023, 2024)
WHERE payee IN ('Amazon')
```

- 右侧可以是 `set` 类型的值（例如 `tags`、`links` 或 `other_accounts` 列），也可以是括号中的表达式列表。列表可以只有一个元素。
- 右侧是集合时，左侧必须是字符串。把集合放在括号里的 `'x' IN (tags)` 同样是测试集合成员。
- 右侧是列表时，每个元素都必须能像 `=` 那样与左侧比较。
- `NOT IN` 与 `IN` 相反。
- 左侧为 `NULL` 时结果为 `NULL`。如果在含有 `NULL` 的列表中没有找到该值，结果也是 `NULL`，与标准 SQL 一致。

### NULL 与三值逻辑

缺失的值为 `NULL`，例如没有收款方的交易的 `payee`，或者没有按成本持有的分录的成本。

- 算术运算、比较、`~`、`!~`、`?~`、`IN`、`NOT IN` 和函数调用只要有一个操作数为 `NULL`，结果就是 `NULL`。例外的是 `IS NULL`、`IS NOT NULL`、`AND`、`OR` 和聚合函数。
- `AND`、`OR` 和 `NOT` 遵循标准 SQL 的三值逻辑：

| 表达式 | 结果 |
|--------|------|
| `TRUE AND NULL` | `NULL` |
| `FALSE AND NULL`、`NULL AND FALSE` | `FALSE` |
| `TRUE OR NULL`、`NULL OR TRUE` | `TRUE` |
| `FALSE OR NULL` | `NULL` |
| `NOT NULL` | `NULL` |

- `WHERE` 和 `FROM` 把 `NULL` 视为不成立，相应的分录会被丢弃。
- 请用 `IS NULL` 和 `IS NOT NULL` 判断缺失值。`payee = NULL` 的结果永远是 `NULL`，不会是 `TRUE`。
- `payee != 'Shop'` 和 `NOT (payee = 'Shop')` 都会丢弃 `payee` 为 `NULL` 的分录。如需保留，请写成 `payee IS NULL OR payee != 'Shop'`。

## postings 表

第一阶段只有一张表 `postings`。每笔交易的每条分录对应一行，交易的字段会重复出现在它的每条分录上。

- **包含：**所有交易（无论标记是什么），以及张记账为 `balance ... with pad ...` 指令生成的补齐交易。补齐交易的标记为 `P`，收款方为 `Balance Pad`，描述形如 `pad Assets:Bank to Equity:Opening`。
- **不包含：**余额断言，以及所有非交易指令，例如 `open`、`close`、`price`、`note`、`document` 和预算指令。
- 没有写金额的分录，使用张记账在平衡交易时推断出的金额。
- 减少持仓但没有给出成本数值的分录（例如 `-7 AAPL {}`）会与该账户中尚未平仓的批次匹配：先进先出；如果账户使用 `LIFO` 记账方法，则后进先出。如果它减少了多个批次，就会像 Beancount 一样，每个批次产生一行，每行带有对应批次的成本。
- 各行按账本顺序排列：先按日期和时间，再按交易在文件中出现的顺序。

### 列

| 列 | 类型 | 说明 |
|----|------|------|
| `date` | `date` | 交易日期。如果交易带有时间，时间部分会被舍去。 |
| `year` | `int` | `date` 的年份。 |
| `month` | `int` | `date` 的月份，1 到 12。 |
| `day` | `int` | `date` 在当月的日，1 到 31。 |
| `flag` | `str` | 交易的标记：`*`（没有写标记时也是它）、`!`、表示补齐的 `P`，或自定义标记。 |
| `payee` | `str` | 交易的收款方，没有则为 `NULL`。交易头只有一个字符串时，该字符串是描述，收款方为 `NULL`。 |
| `narration` | `str` | 交易的描述，没有则为 `NULL`。 |
| `description` | `str` | 用 `" \| "` 连接收款方和描述。缺失或为空的部分会被省略，所以两者都缺失时为 `''`。 |
| `tags` | `set` | 交易的标签，不含开头的 `#`。 |
| `links` | `set` | 交易的链接，不含开头的 `^`。 |
| `id` | `str` | 张记账为交易生成的标识符，是一个 UUID。同一交易的所有分录共享这个值。 |
| `account` | `str` | 分录的账户。 |
| `number` | `decimal` | 分录的单位数量。 |
| `currency` | `str` | 单位的货币（商品）。 |
| `position` | `position` | 分录的单位及其成本批次（如果有）。 |
| `cost_number` | `decimal` | 单位成本，未按成本持有则为 `NULL`。用 `{{...}}` 写出的总成本会除以单位数量。 |
| `cost_currency` | `str` | 成本的货币，或 `NULL`。 |
| `cost_date` | `date` | 成本批次的日期，未按成本持有则为 `NULL`。没有明确写出日期的批次使用其交易的日期。 |
| `cost_label` | `str` | 成本批次的标签。未按成本持有时为 `''`（空字符串），批次没有标签时为 `NULL`。 |
| `price` | `amount` | 用 `@` 写出的单价，没有则为 `NULL`。用 `@@` 写出的总价会除以单位数量。 |
| `weight` | `amount` | 分录在交易平衡中所占的金额：按成本持有时为单位数量乘以单位成本；否则如果有价格，为单位数量乘以价格；否则为单位本身。 |
| `other_accounts` | `set` | 同一交易中其他分录的账户。 |

## 类型

| 类型 | 说明 | 示例 |
|------|------|------|
| `null` | `NULL` 字面量的类型。其他所有类型也都可以为 `NULL`。 | `NULL` |
| `bool` | `TRUE` 或 `FALSE`。 | `TRUE` |
| `int` | 64 位整数。 | `2024` |
| `decimal` | 任意精度的精确十进制数。 | `12.50` |
| `str` | 文本。 | `'Expenses:Food'` |
| `date` | 日历日期。 | `2024-01-31` |
| `set` | 无序的字符串集合，用于 `tags`、`links` 和 `other_accounts`。 | `{'trip', 'food'}` |
| `amount` | 带货币的十进制数，即金额。 | `12.50 USD` |
| `position` | 持仓：单位（一个金额）加上可选的成本批次。成本批次包含单位成本的数值和货币，还可以有日期和标签。 | `10 VTI {120.00 USD, 2024-01-02, "lot-a"}` |
| `inventory` | 库存：由多个持仓组成，可以包含任意多种货币和成本批次。 | `-30.00 USD, 10 VTI {120.00 USD}` |

持仓如何合并为库存：

- `sum(position)` 把所有持仓加到同一个库存中。
- 货币相同且成本批次（数值、货币、日期和标签）相同的持仓会合并，数量相加。成本批次不同的持仓保持独立，因此以不同价格买入的持仓仍然分开显示。
- 数量变为零的持仓会被移除，所以余额为零的账户得到一个空库存。

`int` 与 `decimal` 一起运算，或者传给需要 `decimal` 的函数时，会被转换为 `decimal`。除此之外，只有[字面量](#字面量)中提到的字符串转日期规则，不会发生其他隐式转换。在 `position`、`amount` 和 `inventory` 之间转换请使用[估值函数](#估值函数)。

### 排序与比较

`ORDER BY`、`min` 和 `max` 按以下规则对同类型的值排序。比较运算符 `<`、`<=`、`>` 和 `>=` 使用同样的顺序，但只接受列表中的前四种类型和 `bool`。

- `int` 和 `decimal`：按数值。
- `str`：按字符编码，因此区分大小写。
- `date`：按时间先后。
- `bool`：`FALSE` 排在 `TRUE` 之前。
- `set`：按排序后的元素逐个比较。
- `amount`：先按货币，再按数值。
- `position`：Beancount 的持仓顺序。`USD`、`EUR`、`JPY`、`CAD`、`GBP`、`AUD`、`NZD` 和 `CHF` 按此顺序排在最前，其他货币随后，货币名较短的在前。再依次按成本数值、成本货币和单位数量排序。
- `inventory`：把其中的持仓按持仓顺序排列后逐个比较。因此只含一种货币的库存按数值排序，[按收款方统计支出](#按收款方统计支出)示例中的 `ORDER BY total DESC` 正是依赖这一点。

## 函数

下面每个表格中，每个重载占一行，签名与 `GET /api/query/schema` 返回的完全一致。`any` 表示任意类型的参数。需要 `decimal` 的地方也接受 `int` 参数。函数名不区分大小写。

任一参数为 `NULL` 时，标量函数直接返回 `NULL`，不会执行函数本身。聚合函数则跳过 `NULL` 值。

### 聚合函数

聚合函数把一个分组中所有分录的值合并成一个值，见 [GROUP BY](#group-by)。

| 签名 | 说明 |
|------|------|
| `count(*) -> int` | 分组中的分录数。 |
| `count(any) -> int` | 分组中参数不为 `NULL` 的分录数。 |
| `sum(int) -> int` | 整数之和。 |
| `sum(decimal) -> decimal` | 数值之和。 |
| `sum(amount) -> inventory` | 各金额之和，按货币分开。 |
| `sum(position) -> inventory` | 各持仓之和，成本批次按[类型](#类型)中描述的方式合并。 |
| `sum(inventory) -> inventory` | 各库存之和。 |
| `first(any) -> any` | 按账本顺序，分组中第一个非 `NULL` 的值，类型与参数相同。 |
| `last(any) -> any` | 按账本顺序，分组中最后一个非 `NULL` 的值，类型与参数相同。 |
| `min(any) -> any` | 按[排序与比较](#排序与比较)中的顺序，最小的非 `NULL` 值。 |
| `max(any) -> any` | 最大的非 `NULL` 值。 |

- `first` 和 `last` 按账本顺序计算，`ORDER BY` 不会改变哪条分录算作第一条。
- 如果一个分组的值全部为 `NULL`，`sum` 的结果为 `0`（或空库存），`first`、`last`、`min` 和 `max` 的结果为 `NULL`。

### 估值函数

| 签名 | 说明 |
|------|------|
| `units(position) -> amount` | 持仓的单位，不含成本。 |
| `units(inventory) -> inventory` | 每个持仓的单位，不含成本。同一货币的不同成本批次合并为一个持仓。 |
| `cost(position) -> amount` | 持仓的总成本（单位数量乘以单位成本），以成本货币计。未按成本持有的持仓返回其单位。 |
| `cost(inventory) -> inventory` | 对每个持仓应用 `cost` 后按货币求和。 |
| `convert(amount, str) -> amount` | 按最新价格把金额换算为第二个参数指定的货币。 |
| `convert(amount, str, date) -> amount` | 同上，但使用不晚于该日期的最新价格。 |
| `convert(position, str) -> amount` | 把持仓的单位换算为指定货币。成本不会被当作价格使用，但成本货币可以作为中间步骤（见下文）。 |
| `convert(position, str, date) -> amount` | 同上，但使用不晚于该日期的最新价格。 |
| `convert(inventory, str) -> inventory` | 把每个持仓换算为指定货币后求和。 |
| `convert(inventory, str, date) -> inventory` | 同上，但使用不晚于该日期的最新价格。 |
| `value(position) -> amount` | 按最新价格计算的持仓市值，以成本货币计。未按成本持有或没有价格的持仓返回其单位。 |
| `value(position, date) -> amount` | 同上，但使用不晚于该日期的最新价格。 |
| `value(inventory) -> inventory` | 对每个持仓应用 `value` 后求和。 |
| `value(inventory, date) -> inventory` | 同上，但使用不晚于该日期的最新价格。 |
| `getprice(str, str) -> decimal` | 第一种货币一个单位以第二种货币计的最新价格，例如 `getprice('VTI', 'USD')`，没有价格时为 `NULL`。货币名会转换为大写。 |
| `getprice(str, str, date) -> decimal` | 同上，但使用不晚于该日期的最新价格。 |

价格的查找规则：

- 价格来自账本中的 `price` 指令。
- 提供了 `date` 参数时，使用日期不晚于该日期的最新价格；未提供时，使用账本中最新的价格，即使它的日期在未来。
- 同一货币对在同一天有多个价格时，以账本中最后一个为准。
- 价格可以双向使用。`price VTI 120 USD` 既可以按 120 把 VTI 换算为 USD，也可以按 1/120 把 USD 换算为 VTI。如果一个货币对在两个方向上都有报价，报价点较少的方向会被取倒数，合并到另一个方向中。
- `convert` 先查找从单位货币到目标货币的价格。如果没有，而持仓是按成本持有的，就通过成本货币分两步换算：先从单位货币换算为成本货币，再从成本货币换算为目标货币。例如 `10 VTI {100 EUR}` 可以借助 `VTI`/`EUR` 价格和 `EUR`/`USD` 价格换算为 USD。
- 找不到价格时，值保持不变：仍是原来的货币，不会被丢弃，也不会变成零。因此 `convert` 或 `value` 之后的库存仍可能包含多种货币。
- 把一个值换算为它本身的货币，会原样返回。
- 与价格相乘的结果需要超过 28 位有效数字时，会像 Beancount 一样舍入到 28 位。

### 金额与数值

| 签名 | 说明 |
|------|------|
| `number(amount) -> decimal` | 金额的数值。 |
| `currency(amount) -> str` | 金额的货币。 |
| `commodity(amount) -> str` | 与 `currency` 相同。 |
| `only(str, inventory) -> amount` | 库存中某一种货币的单位总数，例如 `only('USD', sum(position))`。库存中没有该货币时为该货币的 `0`。 |
| `filter_currency(position, str) -> position` | 持仓的单位是该货币时返回该持仓，否则返回 `NULL`。 |
| `filter_currency(inventory, str) -> inventory` | 库存中单位为该货币的持仓。 |
| `abs(int) -> int` | 绝对值。 |
| `abs(decimal) -> decimal` | 绝对值。 |
| `abs(amount) -> amount` | 数值取绝对值后的金额。 |
| `abs(position) -> position` | 单位取绝对值后的持仓，成本保持不变。 |
| `abs(inventory) -> inventory` | 对每个持仓应用 `abs`。 |
| `neg(int) -> int` | 取相反数，与一元负号相同。 |
| `neg(decimal) -> decimal` | 取相反数。 |
| `neg(amount) -> amount` | 取相反数后的金额。 |
| `neg(position) -> position` | 单位取相反数后的持仓，成本保持不变。 |
| `neg(inventory) -> inventory` | 每个持仓都取相反数。 |
| `possign(decimal, str) -> decimal` | 除非第二个参数给出的账户属于 `Assets` 或 `Expenses`，否则把第一个参数的符号取反。这样收入、负债和权益的金额都显示为正数。 |
| `possign(amount, str) -> amount` | 对金额做同样的处理。 |
| `possign(position, str) -> position` | 对持仓做同样的处理。 |
| `possign(inventory, str) -> inventory` | 对库存做同样的处理。 |

### 账户函数

| 签名 | 说明 | 示例 |
|------|------|------|
| `root(str) -> str` | 账户名的第一段。 | `root('Expenses:Food:Dining')` 为 `'Expenses'` |
| `root(str, int) -> str` | 账户名的前 `n` 段。如果账户只有 `n` 段或更少，则原样返回。 | `root('Expenses:Food:Dining', 2)` 为 `'Expenses:Food'` |
| `parent(str) -> str` | 去掉最后一段后的账户名。顶级账户的结果为 `''`。 | `parent('Expenses:Food:Dining')` 为 `'Expenses:Food'` |
| `leaf(str) -> str` | 账户名的最后一段。 | `leaf('Expenses:Food:Dining')` 为 `'Dining'` |

### 日期函数

| 签名 | 说明 | 示例 |
|------|------|------|
| `year(date) -> int` | 年份。 | `year(2024-05-17)` 为 `2024` |
| `month(date) -> int` | 月份，1 到 12。 | `month(2024-05-17)` 为 `5` |
| `day(date) -> int` | 当月的日。 | `day(2024-05-17)` 为 `17` |
| `quarter(date) -> str` | 年份和季度，以文本表示。 | `quarter(2024-05-17)` 为 `'2024-Q2'` |
| `weekday(date) -> str` | 星期几的三字母英文缩写。 | `weekday(2024-01-05)` 为 `'Fri'` |
| `yearmonth(date) -> date` | 该日期所在月份的第一天。 | `yearmonth(2024-05-17)` 为 `2024-05-01` |
| `today() -> date` | 账本时区（`timezone` 选项）中的当前日期。 | |

`year`、`month` 和 `day` 列是简写：`year` 等同于 `year(date)`。

### 元数据函数

| 签名 | 说明 |
|------|------|
| `meta(str) -> str` | 分录上某个元数据键的值，未设置则为 `NULL`。 |
| `entry_meta(str) -> str` | 交易上某个元数据键的值，未设置则为 `NULL`。 |
| `any_meta(str) -> str` | 先在分录上查找某个元数据键，找不到再查交易；都没有则为 `NULL`。 |

元数据的值总是以文本形式返回。

:::note
张记账目前还不按分录保存元数据：交易内的每一行元数据，包括缩进在某条分录下面的行，都存放在交易上。在这一点改变之前（见 [#434](https://github.com/zhang-accounting/zhang/issues/434)），`meta` 总是返回 `NULL`。请使用 `entry_meta` 或 `any_meta` 读取元数据。
:::

### 字符串函数

| 签名 | 说明 | 示例 |
|------|------|------|
| `str(any) -> str` | 任意值的文本形式。布尔值为 `TRUE` 和 `FALSE`，集合用 `, ` 连接，库存写在括号中。 | `str(2024-01-31)` 为 `'2024-01-31'` |
| `length(str) -> int` | 字符串中的字符数。 | `length('Food')` 为 `4` |
| `length(set) -> int` | 集合中的元素个数。 | `length(tags)` |

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

- `columns` 按顺序列出结果列。每列有 `name` 和 `type`，`type` 是 `null`、`bool`、`int`、`decimal`、`str`、`date`、`set`、`amount`、`position` 和 `inventory` 之一。
- `rows` 是行的列表。每行是一个列表，每列一个单元格，顺序与 `columns` 相同。

### 单元格编码

| 类型 | JSON | 示例 |
|------|------|------|
| 任何 `NULL` | `null` | `null` |
| `bool` | 布尔值 | `true` |
| `int` | 数字 | `2024` |
| `decimal` | 字符串 | `"1520.35"` |
| `str` | 字符串 | `"Expenses:Food"` |
| `date` | 字符串，`YYYY-MM-DD` | `"2024-01-31"` |
| `set` | 排好序的字符串数组 | `["food", "trip"]` |
| `amount` | 对象 | `{"number": "12.50", "currency": "USD"}` |
| `position` | 对象 | `{"units": {"number": "10", "currency": "VTI"}, "cost": {"number": "120.00", "currency": "USD", "date": "2024-01-02", "label": null}}` |
| `inventory` | 对象 | `{"positions": [ ...持仓... ]}` |

- 十进制数（包括金额和成本中的 `number` 字段）以字符串形式发送，以免损失精度。它们不使用指数写法，并保留小数位（`"12.50"`）。请用十进制数库解析，而不要解析为浮点数。
- 持仓未按成本持有时 `cost` 为 `null`；否则包含单位成本的 `number` 和 `currency`，以及成本批次的 `date` 和 `label`，后两者都可能为 `null`。
- 库存中的持仓按单位货币排序，再按成本排序，没有成本的持仓排在最前。空库存为 `{"positions": []}`。

### 错误

无法解析、类型检查或执行的查询返回 HTTP 状态码 400。与成功响应不同，响应体没有包在 `data` 中。例如 `SELECT acount, position` 会返回：

```json
{
  "message": "unknown column 'acount'",
  "line": 1,
  "column": 8
}
```

- `line` 和 `column` 给出问题在查询文本中的位置，都从 1 开始。
- `column` 按 Unicode 字符而不是字节计数，一个汉字或带重音的字母算作一列。
- 错误包括语法错误、未知的列或函数、参数类型错误、不合法的 `GROUP BY` 用法、无效的正则表达式，以及不支持的语句或子句。
- 少数在计算各行时发现的错误（例如 `sum` 中的整数溢出）没有位置信息，此时 `line` 和 `column` 为 `null`。

### Schema

`GET /api/query/schema` 描述 `postings` 表和每个函数重载。查询页面的参考面板就是根据它生成的。

```json
{
  "data": {
    "columns": [
      { "name": "date", "type": "date", "description": "Date of the transaction." }
    ],
    "functions": [
      { "name": "count", "signature": "count(*) -> int", "description": "Number of rows." }
    ]
  }
}
```

- `columns` 每列一项，顺序与[列](#列)表格相同。
- `functions` 每个重载一项：先是聚合函数，然后是标量函数。`signature` 的写法与本页表格相同。

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

### 以正数显示收入和支出

```sql
SELECT root(account, 1) AS type, sum(possign(position, account)) AS total
WHERE account ~ '^(Income|Expenses)' AND year = 2024
GROUP BY type
```

收入在账本中是负数。`possign` 在求和之前把每条收入分录的符号取反，所以两个合计都显示为正数。

## 与 BQL 和 beanquery 的差异

### 尚未支持

- **`SELECT` 以外的语句：**`BALANCES`、`JOURNAL` 和 `PRINT` 会报错。
- **`FROM` 中的时间段子句：**不支持 `OPEN ON`、`CLOSE ON` 和 `CLEAR`，`FROM` 后面也不能跟表名或子查询。
- **`HAVING` 和 `PIVOT BY`。**
- **其他表：**只有 `postings` 表，没有 entries、prices、accounts、commodities、documents 或 balances 等表。
- **累计余额 `balance` 列**，以及 beanquery 的 `posting_flag`、`filename`、`lineno`、`location`、`meta`、`entry`、`accounts` 和 `type` 列。
- **`query` 指令：**尚不能列出或执行保存在账本中的查询（`2024-01-01 query "name" "SELECT ..."`）。
- **`BETWEEN` 和 `%` 运算符**，以及 beanquery 的带引号标识符。
- **本页未列出的函数**，例如 `round`、`safediv`、`account_sortkey`、`has_account`、`open_date`、`close_date`、`open_meta`、`currency_meta`、`grep`、`subst`、`upper`、`lower`、`joinstr`、`findfirst`，类型转换函数 `int`、`decimal` 和 `date`，以及 `date_*` 系列函数。调用它们会报错。

[#434](https://github.com/zhang-accounting/zhang/issues/434) 中的路线图计划在下一阶段支持 `BALANCES`、`JOURNAL`、`OPEN`/`CLOSE`/`CLEAR`、`query` 指令和 CSV 导出，之后再支持 `HAVING`、`PIVOT BY` 和更多的表。

### 行为不同之处

- **`SELECT *` 包含 `account`。**beanquery 把 `*` 展开为 `date, flag, payee, narration, position`。张记账在 `position` 之前加入了 `account`，因为没有账户的分录很难看懂。
- **标准的三值逻辑。**在 beanquery 中，`NOT NULL` 为 `TRUE`，所以 `NOT (payee = 'x')` 会保留没有收款方的分录；`NULL AND FALSE` 为 `NULL`。在张记账中，与 SQL 一样，`NOT NULL` 为 `NULL`，`NULL AND FALSE` 为 `FALSE`。
- **`?~` 的模式在右侧。**在 beanquery 中，`?~` 的模式是左操作数。在张记账中，`?~` 只是 `~` 区分大小写的版本。
- **单元素列表可以使用。**`payee IN ('Amazon')` 在张记账中可以正常使用。beanquery 会把 `('Amazon')` 当作带括号的字符串，必须写成 `('Amazon',)`。
- **`meta()` 目前总是返回 `NULL`**，因为张记账还不保存分录级的元数据，见[元数据函数](#元数据函数)。
- **注释**以 `--` 开头。不支持 beanquery 的 `;` 行注释和 `/* */` 块注释。`;` 只能出现在查询末尾。
- **正则表达式**使用 Rust 语法，不支持环视和反向引用。
- **错误带有位置信息。**只要能定位，每个查询错误都会给出出错的行和列。
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
