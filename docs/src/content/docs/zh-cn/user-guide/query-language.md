---
title: 查询语言
description: 张记账兼容 BQL 的查询语言参考，包括语法、BALANCES 和 JOURNAL 语句、会计期间、postings 表、类型、函数、图表、保存的查询、CSV 导出、HTTP API 以及与 Beancount 查询语言的差异。
---

张记账提供了一门小型查询语言，用来临时回答关于账本的各种问题。它支持 [Beancount 查询语言（BQL）](https://beancount.github.io/docs/beancount_query_language/)的一个子集，为 Beancount 或 Fava 编写的大多数查询无需修改即可使用。当 BQL v2 与其后继者 [beanquery](https://github.com/beancount/beanquery) 行为不一致时，张记账以 beanquery 为准，本页末尾列出的少数有意为之的差异除外。

查询直接在张记账已加载到内存中的账本上执行。查询是只读的，所有运算都使用精确的十进制数，金额永远不会经过浮点数转换。

:::caution[早期版本]
本页描述的是 [#434](https://github.com/zhang-accounting/zhang/issues/434) 第三阶段的查询语言：在 `postings` 表、[其他表](#其他表)和[张记账特有的表](#张记账特有的表)上执行的 `SELECT`、`BALANCES` 和 `JOURNAL` 语句，会计期间子句 `OPEN ON`、`CLOSE` 和 `CLEAR`，保存的查询以及 CSV 导出。尚未支持的功能见[与 BQL 和 beanquery 的差异](#与-bql-和-beanquery-的差异)。
:::

## 运行查询

### 在网页界面中

打开 `/explore` 的 **查询** 页面（英文界面中为 **Query**）。

- 在编辑器中输入查询。只有点击运行按钮或按下 <kbd>Ctrl</kbd>+<kbd>Enter</kbd>（macOS 上为 <kbd>Cmd</kbd>+<kbd>Enter</kbd>）时才会执行，输入过程中不会自动执行。
- 结果以表格展示，每个单元格按其[类型](#类型)渲染。库存（inventory）单元格每行显示一个持仓。
- 由一个标签和一个数值或金额组成的两列结果，还会在表格上方绘制成图表，见[图表](#图表)。
- 查询出错时，编辑器会高亮出错的行和列。
- **示例** 菜单把现成的查询放进编辑器并执行，其中包括损益表、年末余额和账户流水。
- **已保存** 菜单列出账本中保存的查询，见[保存的查询](#保存的查询)。
- **参考** 面板列出了所有列和函数。
- **导出 CSV** 把结果下载为 CSV 文件，见[导出为 CSV](#导出为-csv)。

#### 图表

当结果正好有两列、至少有一行，并且第二列是数值或金额（`int`、`decimal`、`amount`、`position` 或 `inventory`）时，查询页面会把它绘制成图表。图表的种类由第一列决定：

| 第一列 | 图表 |
|--------|------|
| `date` | 折线图，每个日期一个点，按日期升序排列。 |
| `str`，且每个标签看起来都是账户名 | 按账户层级绘制的矩形树图。 |
| 其他 `str` | 柱状图，每个标签一根柱子，按结果中的顺序排列。最多绘制 50 根柱子。 |

- 由不含空格、以 `:` 分隔的若干段组成的标签，看起来就是账户名。至少要有一个标签包含 `:`，空标签和 `NULL` 标签不参与判断。
- 持仓和库存按单位绘制，不计成本。图表每次只显示一种货币。结果中有多种货币时，可以用 **货币** 选择器选择要绘制的货币。默认选中账本的运营货币（`operating_currency` 选项，如果结果中有），否则选中出现在最多行中的货币。
- 标签相同的行会相加，值为 `NULL` 的行会被跳过。在折线图中，在所选货币下没有值的日期按零绘制。
- 在矩形树图中，账户按其各段逐层嵌套，所有账户共有的顶层（例如 `Expenses`）会被省略。矩形的面积表示绝对值，颜色表示值的正负。
- 数值的相加是精确的，只在绘制时才转换为浮点数。
- **图表** 开关可以隐藏或显示图表，浏览器会记住这个设置。

下面的查询把一年的支出绘制成矩形树图：

```sql
SELECT account, sum(position) WHERE account ~ '^Expenses' AND year = 2024 GROUP BY account
```

下面的查询把每月支出绘制成折线图：

```sql
SELECT yearmonth(date) AS month, sum(position) WHERE account ~ '^Expenses' GROUP BY month ORDER BY month
```

#### 保存的查询

用 [`query` 指令](/zh-cn/directives/query/)保存在账本中的查询，会出现在 **已保存** 菜单中：

```zhang
2024-01-01 query "food by payee" "SELECT payee, sum(position) WHERE account ~ '^Expenses:Food' GROUP BY payee"
```

- 每一项显示查询的名字、日期和查询文本的开头。查询按账本顺序列出；同名的查询都会列出，可以通过日期区分。
- 选择一项会把它的查询放进编辑器并执行。
- 无法被当前版本的查询引擎编译的查询依然会列出，标记为 **（无效）**，并显示错误信息。加载账本时不会检查保存的查询，所以无效的查询不会让账本报告错误。
- 每次打开菜单都会重新获取列表，所以打开页面之后才加入账本的查询，无需刷新页面就会出现。
- 查询是账本文件中带引号的字符串。不构成已知转义的反斜杠会原样保留，所以写 `'\d+'` 即可保存正则表达式 `\d+`；写两次的 `'\\d+'` 同样可以使用，并且在 Beancount 中的读取结果也相同。见[转义](/zh-cn/directives/query/#转义)。

#### 导出为 CSV

**导出 CSV** 把编辑器中的查询发送到 [`POST /api/query/csv`](#csv-导出)，并把结果下载为 `query.csv`。查询不需要先执行。金额、持仓和库存会按货币拆分成数值列，因此文件可以直接在电子表格中打开。如果查询有错误，错误的显示方式与执行失败时相同。在电子表格软件中打开导出的文件之前，请先阅读[关于公式的注意事项](#csv-导出)。

### 通过 HTTP

将查询发送到 `POST /api/query`；发送到 `POST /api/query/csv` 则得到 CSV 格式的结果。`GET /api/query/saved` 列出保存的查询。请求和响应格式见 [HTTP API](#http-api)。

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
  [FROM from_clause]
  [WHERE expression]
  [GROUP BY group_key [, group_key ...] [HAVING expression]]
  [ORDER BY order_key [ASC | DESC] [, order_key [ASC | DESC] ...]]
  [PIVOT BY pivot_key, pivot_key]
  [LIMIT count]
  [;]

BALANCES [AT function] [FROM from_clause] [WHERE expression] [;]

JOURNAL ['pattern'] [AT function] [FROM from_clause] [;]

target      = expression [AS name]
group_key   = expression | target name | target number
order_key   = expression | target name | target number
pivot_key   = target name | target number
from_clause = #table | [expression] [OPEN ON date] [CLOSE [ON date]] [CLEAR]
```

- 一个查询就是一条语句：`SELECT`，或者简写形式 [`BALANCES` 和 `JOURNAL`](#balances-与-journal) 之一。不支持 `PRINT`。
- 各子句必须按上面的顺序出现。除开头的关键字外都是可选的，末尾可以有一个 `;`。
- 关键字、列名和函数名都不区分大小写：`SELECT account`、`select ACCOUNT` 和 `Select Account` 是同一个查询。[表名](#其他表)与 beanquery 一样区分大小写。
- 结构化的列的字段用点号读取，中间不能有空格：例如 [`#accounts`](#accounts) 中的 `open.date`。
- 名字由 ASCII 字母、数字和下划线组成，不能以数字开头。`SELECT`、`DISTINCT`、`FROM`、`WHERE`、`GROUP`、`BY`、`ORDER`、`ASC`、`DESC`、`LIMIT`、`AS`、`AND`、`OR`、`NOT`、`IN`、`IS`、`NULL`、`TRUE`、`FALSE`、`HAVING` 和 `PIVOT` 是保留字，不能用作列名。
- 词法单元之间的空格和换行没有意义，一个查询可以分成多行书写。
- `--` 开始一段注释，直到行尾。

### 查询的执行过程

1. `FROM #table` 选择要读取的[表](#其他表)，没有时读取分录。如果 `FROM` 中有[会计期间子句](#会计期间)（`OPEN ON`、`CLOSE` 和 `CLEAR`），它们先改写整个账本的分录。
2. `FROM` 中的表达式和 `WHERE` 子句决定哪些行参与计算。
3. 如果查询读取了[累计余额](#累计余额)，就按账本顺序在被选中的分录上累加。
4. 如果查询使用了[聚合函数](#聚合函数)或带有 `GROUP BY` 子句，被选中的分录会被分组，每组产生一行；否则每条分录产生一行。
5. `HAVING` 丢弃不满足其条件的分组。
6. `ORDER BY` 对结果行排序。
7. `DISTINCT` 去除重复的行，每组重复只保留第一行。
8. `LIMIT` 保留前面的若干行，丢弃其余的行。
9. `PIVOT BY` 把剩下的行转换成一张表，某个目标的每个值占一列。

在读取任何分录之前，张记账会先检查并简化查询。只包含常量的部分（例如 `'^Expenses:' + 'Food'`）在这时就计算一次。依赖账本或当前日期的函数（`today`、`convert`、`value`、`getprice` 和元数据函数）不会被提前计算。因此，常量模式中无效的正则表达式会立即报错并给出位置，即使没有任何分录会与它匹配。

张记账只在第一个读取分录的查询中对账本的分录做一次批次记账，并把结果连同账本的价格一起保留，直到账本重新加载。如果查询没有[会计期间子句](#会计期间)，且其 `FROM` 或 `WHERE` 只可能对某些账户成立，查询就只读取这些账户的分录：`account = 'Assets:Bank'`、`account = :account`、`account IN ('Assets:Bank', 'Assets:Cash')` 和 `account IN :accounts`，可以单独使用、用 `OR` 组合，或作为用 `AND` 连接的条件之一。这只会让这类查询更快，结果（包括[累计余额](#累计余额)）完全相同。

### SELECT

目标（target）是要为每一行计算的表达式，用逗号分隔。

- `SELECT *` 是 `SELECT date, flag, payee, narration, account, position` 的简写。用于其他表时，展开为[该表的列](#其他表)。
- `AS name` 为目标命名。这个名字可以在 `GROUP BY` 和 `ORDER BY` 中使用，但不能在 `WHERE` 中使用。
- 结果列有别名时以别名命名；否则以目标的原文命名（去掉首尾空格），与书写完全一致，例如 `account`、`ACCOUNT`、`sum(position)` 或 `sum( position )`。`SELECT *` 的各列使用上面列出的小写名字。
- `SELECT DISTINCT` 会去掉与之前某一行完全相同的行，只比较被选择的值。

### FROM

`FROM` 后面跟的是一个表、一个表达式、[会计期间子句](#会计期间)，或表达式加会计期间子句。其中的表达式和 `WHERE` 一样过滤分录；当两者同时存在时，分录必须同时满足两个条件：

```sql
SELECT account, sum(position)
FROM year = 2024
WHERE account ~ '^Expenses'
GROUP BY account
```

等价于 `... WHERE (year = 2024) AND (account ~ '^Expenses') ...`。这与 beanquery 的行为一致，在 `FROM` 中做过滤的 BQL 查询因此可以继续使用。

会计期间子句写在表达式之后。它们先改写分录，表达式再过滤改写后的分录：

```sql
SELECT account, sum(position)
FROM OPEN ON 2024-01-01 CLOSE ON 2025-01-01
WHERE account ~ '^(Income|Expenses)'
GROUP BY account
```

`FROM #name` 读取[其他表](#其他表)之一，`FROM #postings` 则显式指定默认的表。表在 `FROM` 中必须单独出现，请用 `WHERE` 过滤它的行。会计期间子句只作用于分录，因此不能跟在表的后面；`BALANCES` 和 `JOURNAL` 总是读取分录。

```sql
SELECT date, account, amount, discrepancy
FROM #balances
WHERE discrepancy IS NOT NULL
```

与 beanquery 一样，不是 postings 表的列名的裸名字也表示一个表：`FROM prices` 就是 `FROM #prices`。未知的表会报错，错误位置指向表名。

### WHERE

`WHERE` 保留条件为 `TRUE` 的分录。条件结果为 `NULL` 时视为不成立，该分录会被丢弃（见 [NULL 与三值逻辑](#null-与三值逻辑)）。`WHERE` 和 `FROM` 的条件必须是布尔表达式，两者中都不能使用聚合函数。

### GROUP BY

只要有一个目标使用了[聚合函数](#聚合函数)，或者查询带有 `GROUP BY` 子句，查询就是聚合查询。此时分录会被分组，每组产生一行。

- 分组键可以是表达式、目标名（别名，或者 `account` 这类目标的原文），或目标在 `SELECT` 列表中的序号（从 1 开始）。`GROUP BY 1, 2` 表示按前两个目标分组。名字的匹配不区分大小写。
- 分组键不必出现在 `SELECT` 中。`SELECT sum(position) GROUP BY account` 为每个账户返回一个不带标签的合计。
- **没有 `GROUP BY` 时**，聚合查询按所有非聚合的目标分组。`SELECT account, sum(position)` 等价于 `SELECT account, sum(position) GROUP BY account`。如果所有目标都是聚合函数，所有匹配的分录组成一个分组。
- **有 `GROUP BY` 时**，每个在聚合函数之外读取了列或调用了[元数据函数](#元数据函数)的目标都必须是分组键。`SELECT account, payee, count(*) GROUP BY account` 会报错，因为 `payee` 没有被分组。两者都不使用的目标（例如常量）不需要分组。
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

### HAVING

`HAVING` 过滤聚合查询的分组，就像 `WHERE` 过滤分录一样。它紧跟在 `GROUP BY` 之后，保留条件为 `TRUE` 的分组；条件为 `FALSE` 或 `NULL` 的分组会被丢弃。

```sql
SELECT root(account, 2) AS category, sum(position) AS total
WHERE account ~ '^Expenses'
GROUP BY category
HAVING sum(number) > 1000
```

- `HAVING` 需要 `GROUP BY` 子句。没有 `GROUP BY` 时是语法错误，即使查询已经[隐式分组](#group-by)。
- 条件必须是使用了[聚合函数](#聚合函数)的布尔表达式。其中的聚合函数不必出现在 `SELECT` 中：即使 `count(*)` 不是目标，`HAVING count(*) > 10` 也可以使用。
- `HAVING` 中的名字和 `WHERE` 中一样，指的是 `postings` 表的列，而不是目标的别名。`HAVING total > 1000` 会报错，应该重复写出表达式，例如 `HAVING sum(number) > 1000`。
- 在聚合函数之外，只能通过分组键读取列：与某个分组键相同的表达式取该分组的值。在 `GROUP BY account HAVING account ~ 'Food' AND count(*) > 10` 中，`account` 是每个分组的账户。其他列必须写在聚合函数之内。
- `LIMIT` 只计算 `HAVING` 保留下来的分组。

### ORDER BY

- 排序键和分组键一样，可以是表达式、目标名或目标序号。没有被选择的表达式也可以用来排序，只是不会出现在结果中。
- 每个排序键有自己的方向：`ASC`（升序，默认）或 `DESC`（降序）。在 `ORDER BY 1 DESC, 2` 中，第一个键降序，第二个键升序。
- 先按第一个键比较，第一个键相同时才比较第二个键，依此类推。所有键都相同的行保持原来的顺序。
- `NULL` 比其他任何值都小：升序时排在最前，降序时排在最后。
- 各类型的值如何排序见[排序与比较](#排序与比较)。
- 没有 `ORDER BY` 时，普通查询按账本顺序返回分录：先按日期和时间，再按它们在文件中出现的顺序。聚合查询按各分组第一条分录在账本中出现的顺序返回。

### LIMIT

`LIMIT n` 在排序和 `DISTINCT` 之后保留前 `n` 行。`n` 必须是非负整数字面量。`LIMIT 0` 不返回任何行。

### PIVOT BY

`PIVOT BY a, b` 把聚合查询的结果转换成一张表：目标 `a` 的每个值占一行，目标 `b` 的每个值占一列。

```sql
SELECT root(account, 2) AS category, year, sum(position) AS total
WHERE account ~ '^Expenses:(Food|Home)'
GROUP BY category, year
PIVOT BY category, year
```

| category/year | 2015 | 2016 | 2017 |
|---|---|---|---|
| Expenses:Food | 6614.67 USD | 6859.72 USD | 4686.42 USD |
| Expenses:Home | 31304.42 USD | 31280.55 USD | 20866.27 USD |

- `PIVOT BY` 写在 `ORDER BY` 之后、`LIMIT` 之前，正好接受两个目标，每个目标用名字或它在 `SELECT` 列表中的序号表示，不能是表达式。
- 查询必须是聚合查询，第二个目标必须是分组键，两个目标不能相同。
- 它最后执行，作用于经过 `HAVING`、`ORDER BY`、`DISTINCT` 和 `LIMIT` 之后剩下的行，只有这些行会产生列。
- 无论 `ORDER BY` 如何，结果行都按 `a` 排序，列按 `b` 的值排序。
- 第一列以两个目标命名为 `a/b`。其余每一列以 `b` 的一个值命名（例如 `2016`），存放该值对应的剩余目标。剩余的目标不止一个时，每个值的每个目标各占一列，命名为 `<值>/<目标>`，例如 `2016/total` 和 `2016/count`。
- 值在列名中的写法：日期为 `2016-01-31`，小数为 `12.50`，布尔值为 `True` 和 `False`，`NULL` 为 `NULL`。
- 与 beanquery 一样，列名不一定唯一：`NULL` 值和字符串 `'NULL'` 都命名为 `NULL`，含有 `/` 的值也可能让 `<值>/<目标>` 形式的列名与另一列相同。列按位置区分，所以不会丢失数据，但按列名查找列的电子表格或程序会看到重名的列。
- 某对 `a` 和 `b` 没有对应的行时，单元格为 `NULL`。如果有多行对应同一对值（查询除 `a` 和 `b` 外还按其他键分组时会出现），由结果顺序中的最后一行填充。
- 各列保持其目标的类型，所以 [CSV 导出](#csv-导出)会按货币拆分透视后的金额和库存列，例如 `2016 (USD)`。

### 参数

张记账在自己的代码中（通过 `zhang-query` crate 的 Rust API）执行查询时，查询中凡是可以写值的地方都可以写参数 `$1`、`$2`、... 或 `:name`，参数的值另行绑定。`JOURNAL` 的模式以及 `OPEN ON` 和 `CLOSE ON` 的日期也可以是参数。HTTP API 不绑定参数，所以通过 HTTP 发送的查询中如果含有参数，会报错 `parameter $1 is not bound`。

## BALANCES 与 JOURNAL

与 beanquery 一样，`BALANCES` 和 `JOURNAL` 是两种常用查询的简写。张记账在执行之前会把它们改写为下面所示的 `SELECT`，所以本页关于 `SELECT` 的所有内容同样适用于它们。

### BALANCES

```text
BALANCES [AT function] [FROM from_clause] [WHERE expression]
```

等价于

```text
SELECT account, sum(function(position))
FROM from_clause
WHERE expression
GROUP BY account, account_sortkey(account)
ORDER BY account_sortkey(account)
```

- 每个有分录的账户一行，给出其持仓之和。分录合计为零的账户也会列出，值为空库存。
- 账户先按类型排序，顺序为 `Assets`、`Liabilities`、`Equity`、`Income`、`Expenses`，再按名字排序，见 [`account_sortkey`](#账户函数)。
- `FROM` 和 `WHERE` 决定哪些分录参与求和。`BALANCES FROM year = 2024` 给出的是各账户在 2024 年内的变化，而不是年末余额；年末余额请使用 [`CLOSE ON`](#会计期间)。
- `BALANCES` 没有 `GROUP BY`、`ORDER BY` 和 `LIMIT` 子句。

```sql
BALANCES WHERE account ~ '^Assets'
```

### JOURNAL

```text
JOURNAL ['pattern'] [AT function] [FROM from_clause]
```

等价于

```text
SELECT date, flag, maxwidth(payee, 48), maxwidth(narration, 80), account,
       function(position), function(balance)
FROM from_clause
WHERE account ~ 'pattern'
```

- 账户与模式匹配的每条分录一行，按账本顺序排列，最后一列是[累计余额](#累计余额)。没有模式时列出所有分录。
- 模式是写在引号中的正则表达式，用 [`~`](#正则匹配) 匹配，因此不区分大小写，并且可以匹配账户名的任意一部分。`JOURNAL 'checking'` 列出名字中含有 `Checking` 的所有账户。
- 累计余额把流水中的每一行都加进去，不论属于哪个账户。如果模式匹配了多个账户，余额就是它们的合计余额。
- 收款方被缩短到 48 个字符，描述被缩短到 80 个字符，见 [`maxwidth`](#字符串函数)。
- `JOURNAL` 没有 `WHERE`、`GROUP BY`、`ORDER BY` 和 `LIMIT` 子句。请用 `FROM` 过滤分录。

```sql
JOURNAL 'Assets:Bank:Checking' FROM year = 2024
```

### AT

`AT function` 把一个函数应用到每个持仓上；在 `JOURNAL` 中，还应用到累计余额上：

| 语句 | 结果列 |
|------|--------|
| `BALANCES` | `account`、`sum(position)` |
| `BALANCES AT cost` | `account`、`sum(cost(position))` |
| `JOURNAL 'Cash'` | `date`、`flag`、`maxwidth(payee, 48)`、`maxwidth(narration, 80)`、`account`、`position`、`balance` |
| `JOURNAL 'Cash' AT units` | `date`、`flag`、`maxwidth(payee, 48)`、`maxwidth(narration, 80)`、`account`、`units(position)`、`units(balance)` |

- 函数名后面不加括号。该函数必须接受单个 `position` 参数；用于 `JOURNAL` 时还必须接受单个 `inventory` 参数。常用的是[估值函数](#估值函数) `units`、`cost` 和 `value`，`abs` 和 `neg` 也可以。
- `AT units` 去掉成本，所以同一货币的各个批次合并为一个持仓。`AT cost` 给出账面价值，`AT value` 给出按最新价格计算的市值。
- 未知的函数，或者没有合适重载的函数，会报错，错误位置指向函数名。
- 如表中所示，结果列的命名与等价的 `SELECT` 相同。

## 会计期间

`FROM` 子句中的 `OPEN ON`、`CLOSE` 和 `CLEAR` 像 Beancount 的报表那样，把账本变成某一个会计期间的账簿。有了它们，损益表和资产负债表都只需要一个查询。

2024 年的损益表：

```sql
SELECT account, sum(position) AS total
FROM OPEN ON 2024-01-01 CLOSE ON 2025-01-01
WHERE account ~ '^(Income|Expenses)'
GROUP BY account
ORDER BY account
```

2025 年初的资产负债表：

```sql
BALANCES FROM CLOSE ON 2025-01-01 CLEAR
WHERE account ~ '^(Assets|Liabilities|Equity)'
```

### 期间子句的语法

```text
FROM [expression] [OPEN ON date] [CLOSE [ON date]] [CLEAR]
```

- 各子句必须按这个顺序出现，每个最多出现一次。`FROM CLEAR OPEN ON 2024-01-01` 会报错。
- `FROM` 后面至少要有表达式或一个子句，也可以两者都有。
- 日期是不带引号的日期字面量（例如 `2024-01-01`），或者一个[参数](#参数)。
- 同时给出两个日期时，`CLOSE` 的日期不能早于 `OPEN` 的日期，两者可以相同。
- 这些子句可以用于 `SELECT`、`BALANCES` 和 `JOURNAL`。

### 各子句的作用

这些子句按 `OPEN`、`CLOSE`、`CLEAR` 的顺序改写账本中的所有分录。之后 `FROM` 中的表达式和 `WHERE` 才选择分录，所以过滤条件不会改变子句的计算结果。例如 `FROM year = 2023 OPEN ON 2024-01-01` 只保留日期为 2023-12-31 的期初余额。

**`OPEN ON d`** 让期间从 `d` 开始，并把 `d` 之前的所有内容替换为期初余额：

1. `d` 之前的转换差额被转入 `Equity:Conversions:Previous`。转换差额是所有分录按成本计价后合计不为零的部分。按价格（`@`）把一种货币兑换为另一种货币的交易会留下这样的差额，批次买入时的舍入也会。
2. `d` 之前收入和支出账户的余额被转入 `Equity:Earnings:Previous`，因此这些账户在期间开始时为零。
3. 仍有余额的每个账户得到一笔汇总交易，标记为 `S`，日期为 `d − 1`。余额中的每个批次对应一条分录，所以持仓保留其成本、成本日期和标签；每条分录在 `Equity:Opening-Balances` 上都有一条按该批次成本计的对应分录。

`d` 当天及之后的分录保持不变。

**`CLOSE ON d`** 让期间在 `d` 之前结束。日期为 `d` 或更晚的分录被丢弃，所以 `d` 当天不属于该期间。如果剩下的分录存在转换差额，会加入一笔标记为 `C`、日期为 `d − 1` 的转换交易，把差额记入 `Equity:Conversions:Current`。这些分录的价格为零，以转换货币（默认为 `NOTHING`）计。

不带日期的 **`CLOSE`** 不丢弃任何分录，只加入上述转换交易，其日期为期间内最后一笔条目的日期。

**`CLEAR`** 把每个收入和支出账户的余额转入 `Equity:Earnings:Current`，每个账户一笔转账交易，标记为 `T`，日期为期间内最后一笔条目的日期。之后收入和支出账户的合计为零，资产负债表因此平衡。

期间内的最后一笔条目，指期间内的分录，以及账本中从 `OPEN` 日期到 `CLOSE` 日期前一天的其他带日期指令（例如价格和余额断言）中，日期最晚的那一个。预算指令不计算在内。

### 合成交易

这些子句加入的交易和其他交易一样，是 postings 表中的行：

| 标记 | 由谁加入 | 日期 | 描述 | 账户 |
|------|----------|------|------|------|
| `S` | `OPEN ON d` | `d − 1` | `Opening balance for '<account>' (Summarization)` | 该账户，以及 `Equity:Opening-Balances` |
| `C` | `CLOSE` | `CLOSE ON d` 时为 `d − 1`，否则为期间内最后一笔条目的日期 | `Conversion for (<inventory>)` | `Equity:Conversions:Current` |
| `T` | `CLEAR` | 期间内最后一笔条目的日期 | `Transfer balance for '<account>' (Transfer balance)` | 该账户，以及 `Equity:Earnings:Current` |

- 它们的 `payee` 为 `NULL`，没有标签、链接和元数据。同一笔交易的所有分录共享一个 `id`。
- 对应分录的描述就是它所属交易的描述，因此其中写的是它所平衡的账户。
- `OPEN` 产生的前期收益和前期转换差额，表现为 `Equity:Earnings:Previous` 和 `Equity:Conversions:Previous` 两个账户的 `S` 交易。
- 它们计入[累计余额](#累计余额)。使用 `OPEN ON` 之后，每个账户的第一行就是它的期初余额。
- 可以按 `flag` 过滤它们。例如 `WHERE flag != 'S'` 会隐藏期初余额。

```sql
SELECT flag, count(*) FROM OPEN ON 2024-01-01 CLOSE ON 2025-01-01 CLEAR WHERE flag IN ('S', 'C', 'T') GROUP BY flag
```

### 权益账户

这些子句记账所用的账户不需要在账本中开立。表格最后一列的 Beancount 选项可以修改它们：

| 账户 | 使用者 | 选项 |
|------|--------|------|
| `Equity:Opening-Balances` | `OPEN ON` | `account_previous_balances` |
| `Equity:Earnings:Previous` | `OPEN ON` | `account_previous_earnings` |
| `Equity:Conversions:Previous` | `OPEN ON` | `account_previous_conversions` |
| `Equity:Earnings:Current` | `CLEAR` | `account_current_earnings` |
| `Equity:Conversions:Current` | `CLOSE` | `account_current_conversions` |

- 选项给出的是名字中 `Equity:` 之后的部分。写了 `option "account_previous_balances" "Opening"` 之后，`OPEN ON` 使用 `Equity:Opening`。
- 不是有效账户名的值（例如空值或含有空格的值）会被忽略，此时使用默认账户。
- `option "conversion_currency" "..."` 设置转换分录零价格所用的货币，默认为 `NOTHING`。

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
| `pattern ?~ text` | 区分大小写的匹配。注意模式写在**前面**，与 beanquery 一致。 |

| 表达式 | 结果 |
|--------|------|
| `'Expenses:Food:Dining' ~ 'food'` | `TRUE`，部分匹配且忽略大小写 |
| `'Expenses:Food:Dining' ~ '^Food'` | `FALSE`，因为 `^` 锚定在开头 |
| `'Expenses:Food:Dining' ~ 'Dining$'` | `TRUE` |
| `'Expenses:Food:Dining' !~ '^Income'` | `TRUE` |
| `'Food' ?~ 'Expenses:Food:Dining'` | `TRUE` |
| `'food' ?~ 'Expenses:Food:Dining'` | `FALSE`，因为 `?~` 区分大小写 |

- 需要完整匹配时，请用 `^` 和 `$` 锚定模式。
- 模式使用 Rust [`regex`](https://docs.rs/regex/latest/regex/#syntax) 库的语法。它与 Python 的语法很接近，但不支持环视（`(?=...)`、`(?!...)`）和反向引用。
- 无效的模式会报错，错误位置指向该模式。编译后超过 1 MiB 的模式（例如 `'a{1000}{1000}'`）也会这样报错。
- 两边都必须是字符串。任一边为 `NULL` 时结果为 `NULL`，三个运算符都是如此。

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

默认的表是 `postings`。每笔交易的每条分录对应一行，交易的字段会重复出现在它的每条分录上。[其他表](#其他表)存放各种指令，[张记账特有的表](#张记账特有的表)存放预算和账本错误。

- **包含：**所有交易（无论标记是什么），以及张记账为 `balance ... with pad ...` 指令生成的补齐交易。补齐交易的标记为 `P`，收款方为 `Balance Pad`，描述形如 `pad Assets:Bank to Equity:Opening`。带有[会计期间子句](#会计期间)的查询还会看到这些子句加入的[合成交易](#合成交易)。
- **不包含：**余额断言，以及所有非交易指令，例如 `open`、`close`、`price`、`note`、`document` 和预算指令。
- 没有写金额的分录，使用张记账在平衡交易时推断出的金额。
- 按成本持有的分录会按[批次记账](#批次记账)中的规则与批次匹配。减少了多个批次的分录会产生多行。
- 各行按账本顺序排列：先按日期和时间，再按交易在文件中出现的顺序。

### 批次记账

按成本持有的分录，其 `position`、`cost_*` 和 `weight` 列取决于它与哪个批次匹配。查询引擎按账户和货币、按账本顺序记账，规则与 Beancount 相同：

- **减仓。**按成本持有、且符号与某个未平仓批次相反的分录（例如卖出之前买入的持仓）会减少批次。成本中写出的字段（成本数值和货币、日期、标签）必须与批次一致，没有写出的字段可以匹配任何批次。`-4 AAPL {100 USD}` 减少以 100 USD 买入的批次，不论日期；`-4 AAPL {100 USD, 2024-01-02}` 只减少 2024-01-02 买入的那一个；`-4 AAPL {100 USD, "a"}` 只减少标签为 `a` 的那一个；`-4 AAPL {}` 可以减少任何批次。
- **在匹配的批次中选择。**匹配的批次按先进先出（FIFO）使用；如果账户的 `booking_method` 为 `LIFO`，则后进先出。跨越多个批次的减仓会拆成多行，每个批次一行，每行带有从该批次取出的单位和该批次的成本。
- **加仓。**其他按成本持有的分录会开立一个新批次，或者加到完全相同的批次上。没有日期的成本（例如 `10 AAPL {100 USD}`）使用其交易的日期，所以按成本持有的分录的 `cost_date` 永远不会是 `NULL`。
- **剩余部分。**如果未平仓的批次不足以覆盖整个减仓，剩余部分按加仓处理。没有成本数值的成本（例如 `{}`）无法开立批次，所以这部分没有成本。

:::note
有两种记账情况，张记账的账本处理与查询引擎的结果仍不一致。对于给出了成本但没有日期的减仓，张记账目前只与该减仓当天日期的批次匹配，否则记录一个错误（[#436](https://github.com/zhang-accounting/zhang/issues/436)）。`STRICT`、`AVERAGE`、`AVERAGE_ONLY` 和 `NONE` 记账方法尚未实现（[#437](https://github.com/zhang-accounting/zhang/issues/437)），查询引擎目前把使用这些方法的账户按 FIFO 记账。
:::

### 列

| 列 | 类型 | 说明 |
|----|------|------|
| `date` | `date` | 交易日期。如果交易带有时间，时间部分会被舍去，可以从 `time` 读取。 |
| `year` | `int` | `date` 的年份。 |
| `month` | `int` | `date` 的月份，1 到 12。 |
| `day` | `int` | `date` 在当月的日，1 到 31。 |
| `flag` | `str` | 交易的标记：`*`（没有写标记时也是它）、`!`、表示补齐的 `P`、表示会计期间子句[合成交易](#合成交易)的 `S`、`C` 或 `T`，或自定义标记。 |
| `payee` | `str` | 交易的收款方，没有则为 `NULL`。交易头只有一个字符串时，该字符串是描述，收款方为 `NULL`。 |
| `narration` | `str` | 交易的描述，没有则为 `''`（空字符串），与 Beancount 一致。 |
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
| `cost_date` | `date` | 成本批次的日期，未按成本持有则为 `NULL`。没有明确写出日期的批次使用开立它的交易的日期。 |
| `cost_label` | `str` | 成本批次的标签。未按成本持有时为 `''`（空字符串），批次没有标签时为 `NULL`。 |
| `price` | `amount` | 用 `@` 写出的单价，没有则为 `NULL`。用 `@@` 写出的总价会除以单位数量。 |
| `weight` | `amount` | 分录在交易平衡中所占的金额：按成本持有时为单位数量乘以单位成本；否则如果有价格，为单位数量乘以价格；否则为单位本身。 |
| `other_accounts` | `set` | 同一交易中其他分录的账户。 |
| `meta` | `str` | 分录的元数据文本：按键排序的 `key: "value"` 对，用 `, ` 分隔；没有元数据时为 `''`。交易自己的元数据用 `entry_meta()` 读取。 |
| `balance` | `inventory` | [累计余额](#累计余额)：截至并包括本行的各行持仓之和。不能用在 `FROM` 或 `WHERE` 中。 |
| `time` | `str` | 交易在账本时区中的时刻，格式为 `HH:MM:SS`。没有写时间的交易为 `00:00:00`。张记账扩展。 |
| `timestamp` | `int` | 交易日期和时间的 Unix 时间，单位为秒。张记账扩展。 |
| `seq` | `int` | 交易在 [`#entries`](#entries) 中的位置，从 0 开始。同一交易的所有分录共享这个值，所以 `ORDER BY seq DESC` 以稳定的顺序把最新的交易排在最前。会计期间子句的[合成交易](#合成交易)为 `NULL`。张记账扩展。 |
| `posting_index` | `int` | 分录在其交易中按书写顺序的位置，从 0 开始。[批次记账](#批次记账)把一条分录拆成每个批次一行时，这些行共享这个值。张记账扩展。 |
| `account_balance` | `inventory` | [账户余额](#账户余额)：本条分录之后该分录所属账户的余额。张记账扩展。 |
| `balanced` | `bool` | 张记账发现交易不平衡（`UnbalancedTransaction` 错误）时为 `FALSE`，否则为 `TRUE`。张记账扩展。 |
| `errors` | `set` | 张记账为该交易记录的错误种类，名称与 [`#errors`](#错误表) 的 `kind` 列相同，例如 `UnbalancedTransaction` 或 `AccountDoesNotExist`。没有错误时为空集合。张记账扩展。 |

### 累计余额

`balance` 列是 `position` 的累计合计，类型为库存。

- 它从空库存开始，按账本顺序累加通过 `FROM` 和 `WHERE` 的各行。被过滤掉的分录不计入。使用 `WHERE account = 'Assets:Bank:Checking' AND year = 2024` 时，余额从 2024 年的第一条分录开始从零累计。如果要从账户的真实余额开始，请改用 [`OPEN ON` 和 `CLOSE ON`](#会计期间) 限定日期：`FROM OPEN ON 2024-01-01 CLOSE ON 2025-01-01 WHERE account = 'Assets:Bank:Checking'`。
- 它是所有行的一个总计，而不是每个账户各一个。如果只想跟踪一个账户，请只选择该账户的分录。
- 它在分组、`ORDER BY`、`DISTINCT` 和 `LIMIT` 之前计算，所以对行排序不会改变它们的余额。使用 `ORDER BY date DESC` 时，第一行带有最终余额。
- 它保留批次，所以以不同成本买入的持仓会显示为多个持仓。`units(balance)` 会把它们合并，`cost(balance)` 给出账面价值。
- 在聚合查询中，它可以用在聚合函数内部。`last(balance)` 是每组最后一条分录之后的余额。
- 它不能用在 `FROM` 或 `WHERE` 中，因为正是这两个子句决定了累加哪些行。这样使用会报错。

```sql
SELECT date, payee, position, balance
WHERE account = 'Assets:Bank:Checking'
ORDER BY date DESC
LIMIT 10
```

这个查询返回该账户最近的十条分录，每条都带有记账之后的余额。第一行显示的就是当前余额。

### 账户余额

`account_balance` 列是本条分录之后，分录所属账户的余额。它和 `balance` 一样是保留批次的库存，但与 `balance` 不同，它不受查询的影响：

- 它按账本顺序累加该账户的所有分录，不论 `FROM`、`WHERE` 和 `LIMIT` 选择了哪些行。即使查询只显示账户的部分分录，每条分录显示的仍是账户真实的余额。
- 每个账户各有一个余额，所以涉及多个账户的查询中，每条分录显示的是它自己账户的余额。
- 使用[会计期间子句](#会计期间)时，它累加这些子句产生的分录，从 `OPEN ON` 加入的期初余额开始。
- 与 `balance` 不同，它可以用在 `WHERE` 中。

```sql
SELECT date, payee, position, account_balance
WHERE account = 'Assets:Bank:Checking' AND year = 2024
ORDER BY seq DESC
```

这个查询按从新到旧列出该账户 2024 年的分录，每条都带有之后的账户余额，其中包括 2024 年之前记入的所有金额。

## 其他表

除了 `postings`，查询还可以用 `FROM #name` 读取下面的表。它们就是 beanquery 的表，列名、类型和行的顺序都与 beanquery 相同：

| 表 | 每一行是 | `SELECT *` |
|----|----------|------------|
| `#entries` | 任意一条指令 | `id, type, filename, date, year, month, day, flag, payee, narration, description, tags, links, meta, accounts` |
| `#transactions` | 一笔交易 | `date, flag, payee, narration, tags, links, accounts` |
| `#prices` | 一条 `price` 指令 | `date, currency, amount` |
| `#balances` | 一条余额断言 | `date, account, amount, tolerance, discrepancy` |
| `#notes` | 一条 `note` 指令 | `date, account, comment, tags, links` |
| `#events` | 一条 `event` 指令 | `date, type, description` |
| `#documents` | 一条 `document` 指令 | `date, account, filename, tags, links` |
| `#accounts` | 一个有 `open` 或 `close` 指令的账户 | `account, open, close` |
| `#commodities` | 一条 `commodity` 指令 | `meta, date, name` |

张记账还有两张自己的表 `#budgets` 和 `#errors`，见[张记账特有的表](#张记账特有的表)。

```sql
SELECT currency, last(amount) AS latest
FROM #prices
WHERE date >= 2024-01-01
GROUP BY currency
ORDER BY currency
```

- **每个表有自己的列。**一个表只有为它列出的列，没有 `postings` 的列。`year`、`month` 和 `day` 只存在于 `#entries` 和 `postings` 中，其他表请使用 [`year(date)`](#日期函数) 等日期函数。所有函数、聚合函数，以及 `GROUP BY`、`HAVING`、`ORDER BY`、`PIVOT BY`、`DISTINCT` 和 `LIMIT` 都可以用于每个表。
- **行的顺序。**没有 `ORDER BY` 时，各行按账本顺序排列：先按日期，再按 beancount 对同一天指令的排序（`open` 最先，然后是余额断言、其他指令，`document` 和 `close` 最后），再按指令在文件中的顺序。
- **元数据。**每个指令表都有一列 `meta`，以文本形式给出指令的元数据：按键排序的 `key: "value"` 对，用 `, ` 分隔；没有元数据时为 `''`。`meta(key)`、`entry_meta(key)` 和 `any_meta(key)` 读取该行指令的某个键（在 `#accounts` 中读取其 `open` 指令）。
- **余额断言不是交易。**张记账把每条余额断言保存为一笔标记为 `C` 的交易。这些交易不会出现在 `#transactions` 和 `#entries` 中，断言在 `#entries` 中是一条 `balance` 记录。加载账本时被张记账拒绝的交易也不会出现。`balance ... with pad` 生成的补齐交易（标记为 `P`）与 beancount 一样算作交易。

### #entries

| 列 | 类型 | 说明 |
|----|------|------|
| `id` | `str` | 指令的唯一 ID。交易的 ID 就是交易本身的 ID，与其分录的 `id` 列相同。 |
| `type` | `str` | 指令的种类，小写：`transaction`、`open`、`close`、`balance`、`price`、`note`、`document`、`event`、`commodity`、`custom` 或 `query`，以及张记账的 `budget`、`budget-add`、`budget-transfer` 和 `budget-close`。`balance ... with pad` 算作 `balance`。 |
| `filename` | `str` | 指令所在的账本文件。 |
| `date`、`year`、`month`、`day` | `date`、`int` | 指令的日期及其各部分。 |
| `flag`、`payee`、`narration`、`description` | `str` | 对交易而言与 `postings` 中的同名列相同；其他指令为 `NULL`。 |
| `tags`、`links` | `set` | 交易、note 或 document 的标签和链接；其他指令为 `NULL`。 |
| `meta` | `str` | 指令的元数据。 |
| `accounts` | `set` | 指令涉及的账户：交易的各分录账户，`open`、`close`、`balance`、`note` 或 `document` 的账户，以及 `balance ... with pad` 的补齐账户。其他指令为空集合。 |
| `seq` | `int` | 指令在 `#entries` 中的位置，从 0 开始，即它按账本顺序的行号。`ORDER BY seq DESC` 把最新的记录排在最前。张记账扩展。 |
| `time`、`timestamp` | `str`、`int` | 指令在账本时区中的时刻（`HH:MM:SS`，没有时间时为 `00:00:00`），以及其日期和时间的 Unix 时间，单位为秒。张记账扩展。 |

### #transactions

| 列 | 类型 | 说明 |
|----|------|------|
| `date` | `date` | 交易日期。 |
| `flag` | `str` | `*`、`!`，补齐交易为 `P`。 |
| `payee` | `str` | 收款方，或 `NULL`。 |
| `narration` | `str` | 描述，没有时为 `''`。 |
| `tags`、`links` | `set` | 标签和链接。 |
| `accounts` | `set` | 各分录的账户。 |
| `meta` | `str` | 交易的元数据。 |
| `id` | `str` | 张记账为交易生成的标识符：与其分录的 `id` 以及它在 `#entries` 中那一行的 `id` 相同。张记账扩展。 |
| `seq`、`time`、`timestamp` | `int`、`str`、`int` | 与 `postings` 中相同：交易在 `#entries` 中的位置、交易的时刻和 Unix 时间。张记账扩展。 |
| `balanced`、`errors` | `bool`、`set` | 与 `postings` 中相同：交易是否平衡，以及为它记录的错误种类。张记账扩展。 |

### #prices、#balances、#notes、#events、#documents 和 #commodities

| 表 | 列 | 类型 | 说明 |
|----|----|------|------|
| `#prices` | `date` | `date` | 价格的日期。 |
| | `currency` | `str` | 被定价的商品。 |
| | `amount` | `amount` | 一单位商品的价格。 |
| `#balances` | `date` | `date` | 断言的日期。 |
| | `account` | `str` | 被断言余额的账户。 |
| | `amount` | `amount` | 断言的余额。 |
| | `tolerance` | `decimal` | 显式给出的容差（`~ 0.01`），或 `NULL`。 |
| | `discrepancy` | `amount` | 断言不成立时为余额减去断言金额；成立时为 `NULL`。`balance ... with pad` 总是成立。 |
| `#notes` | `date`、`account` | `date`、`str` | 备注的日期和账户。 |
| | `comment` | `str` | 备注的内容。 |
| | `tags`、`links` | `set` | 标签和链接。 |
| `#events` | `date` | `date` | 事件的日期。 |
| | `type` | `str` | 事件的种类，例如 `location`。 |
| | `description` | `str` | 事件的值，例如一个城市。 |
| `#documents` | `date`、`account` | `date`、`str` | 文档的日期和账户。 |
| | `filename` | `str` | 文件的路径。与 beancount 一样，相对路径相对于声明它的账本文件所在的目录。 |
| | `tags`、`links` | `set` | 标签和链接。 |
| `#commodities` | `date` | `date` | `commodity` 指令的日期。 |
| | `name` | `str` | 商品，例如 `USD`。 |

这些表都还有一列 `meta`。

### #accounts

`open` 和 `close` 是账户的 `open` 和 `close` 指令，是结构化的值，用点号读取它们的字段：

| 列 | 类型 | 说明 |
|----|------|------|
| `account` | `str` | 账户名。 |
| `open`、`open.date` | `date` | `open` 指令的日期，没有时为 `NULL`。单独使用 `open` 时读作这个日期。 |
| `open.account` | `str` | `open` 指令的账户。 |
| `open.currencies` | `set` | 账户限定的货币；不限定时为 `NULL`。 |
| `open.booking` | `str` | 记账方法，取自 `booking_method` 元数据，或 `NULL`。 |
| `open.meta` | `str` | `open` 指令的元数据。 |
| `close`、`close.date` | `date` | `close` 指令的日期，账户未关闭时为 `NULL`。单独使用 `close` 时读作这个日期。 |
| `close.account`、`close.meta` | `str` | `close` 指令的账户和元数据。 |

```sql
SELECT account, open.date, open.currencies
FROM #accounts
WHERE close IS NULL
ORDER BY account
```

不存在的指令的字段为 `NULL`。未知的字段（例如 `open.datum`）会报错，错误位置指向该字段。

## 张记账特有的表

张记账有两张自己的表，存放 Beancount 中没有的数据：`#budgets` 是[预算](/zh-cn/directives/budget/)的逐月数据，`#errors` 是张记账在账本中发现的问题。它们与[其他表](#其他表)一样用 `FROM #budgets` 和 `FROM #errors` 读取，规则也相同：一个表只有自己的列，例如 `account` 不是 `#budgets` 的列，所有子句和函数都可以用于它。与指令表不同，它们没有 `meta` 列，行的顺序见下面各表的说明。`meta(key)`、`entry_meta(key)` 和 `any_meta(key)` 读取的都是该行自己的元数据。

### 预算表

`#budgets` 中每个预算每个月对应一行，数据与网页界面的预算页面在该月显示的一致。

- 每个预算从其 `budget` 指令所在的月份起，每个月都有一行，直到以下两个月份中较晚的一个：该预算最后一次 `budget-add`、`budget-transfer`、`budget-close` 或支出所在的月份，以及账本中最后一笔交易所在的月份。因此，用 `budget-add` 为未来月份提前安排的预算会显示那个月。价格、事件、备注等其他指令不会延长这些月份。没有预算条目、也没有支出的月份同样有一行，可用金额顺延到这个月，与预算页面一致。这些行只取决于账本，与今天的日期无关：更晚的月份就是该预算最后一行顺延过去、没有任何支出的样子。
- 这些月份是生成的，而不是从账本中读取的，所以每个月份都计入[结果大小限制](#限制)，即使它随后被 `WHERE` 丢弃。如果某笔交易或某条预算指令的日期被误写成遥远的未来，查询会以“结果过大”的错误结束，而不会耗尽内存。错误信息会指出月份最多的预算，以及决定其结束月份的指令，例如 `budget 'food' runs from 2024-01 until 2204-05 because of a transaction dated 2204-05-01 (main.zhang); check that date`。改正日期后即可再次查询该表。
- `assigned`、`activity` 和 `available` 即预算页面上的 Assigned、Activity 和 Available 列。`assigned` 是这个月的起始金额（上个月月底仍可用的金额），加上本月 `budget-add` 和 `budget-transfer` 指令放入的金额（`added`）。`activity` 是预算关联的账户在本月的支出，`available` 即 `assigned - activity`，会顺延到下个月。
- 由于 `assigned` 包含顺延的金额，把多个月的 `assigned` 相加会把同一笔钱算多次。要统计一段时间内一共安排了多少预算，请对 `added` 求和。
- 预算关联的账户，是 `open` 指令中带有指向它的 `budget` 元数据（例如 `budget: food`）的账户。这些账户的分录就是该预算的支出。
- `meta(key)` 读取 `budget` 指令的元数据。
- 各行先按预算名称、再按月份排列。`SELECT *` 是 `SELECT name, date, assigned, activity, available` 的简写。

| 列 | 类型 | 说明 |
|----|------|------|
| `name` | `str` | 预算的名称，即指令中写的名字。 |
| `alias` | `str` | 预算的显示名称，来自 `alias` 元数据，没有则为 `NULL`。 |
| `category` | `str` | 预算页面对预算分组所用的类别，来自 `category` 元数据，没有则为 `NULL`。 |
| `currency` | `str` | 预算使用的商品（货币）。 |
| `date` | `date` | 该月的第一天。 |
| `year` | `int` | 该月所在的年份。 |
| `month` | `int` | 月份，1 到 12。 |
| `assigned` | `amount` | 本月分配给该预算的金额：从上个月顺延的可用金额加上 `added`。 |
| `added` | `amount` | 本月 `budget-add` 和 `budget-transfer` 指令放入该预算的金额。从该预算转出的金额计为负数。 |
| `activity` | `amount` | 预算关联的账户在本月的支出。退款计为负数。 |
| `available` | `amount` | 月底剩余的金额，即 `assigned - activity`。它会顺延到下个月，超支时为负数。 |
| `accounts` | `set` | 其分录计入该预算支出的账户。 |
| `closed` | `bool` | 该预算是否已在本月或更早用 `budget-close` 关闭。 |

按预算页面的分组，查看每个预算还剩多少：

```sql
SELECT category, name, available
FROM #budgets
WHERE date = 2024-06-01
ORDER BY category, name
```

2024 年每个预算安排了多少、花了多少：

```sql
SELECT name, sum(added) AS budgeted, sum(activity) AS spent
FROM #budgets
WHERE year = 2024
GROUP BY name
ORDER BY spent DESC
```

未关闭的预算在哪些月份超支：

```sql
SELECT date, name, available
FROM #budgets
WHERE number(available) < 0 AND NOT closed
```

### 错误表

`#errors` 中每个账本错误对应一行，即网页界面的错误页面列出、`GET /api/errors` 返回的那些问题。

- `kind` 是错误代码，例如 `UnbalancedTransaction`。[错误代码指南](/zh-cn/user-guide/error-code/)解释了每个代码及其修复方法。`message` 是错误页面为它显示的那句话（英文）。
- `file` 是引发错误的指令所在的文件，路径相对于账本目录，与网页界面的文件列表一致。`source` 是该指令的文本。`line` 和 `column` 目前为 `NULL`，因为张记账还不记录行号。
- `date` 是该指令的日期；没有日期的指令（例如 `option`）为 `NULL`。`account` 是错误涉及的账户，只有指明了账户的错误才有，例如 `AccountDoesNotExist`、`AccountClosed` 和 `AccountBalanceCheckError`。
- `meta(key)` 读取张记账为错误记录的其他信息。交易中的错误，`meta('txn_id')` 是该交易的 `id`，与 postings 表中的一致。分录引用了未定义的预算时，有 `meta('budget_name')`。
- 各行先按文件、再按在文件中的位置排列。`SELECT *` 是 `SELECT file, date, kind, account, message` 的简写。

| 列 | 类型 | 说明 |
|----|------|------|
| `kind` | `str` | 错误代码，例如 `UnbalancedTransaction`，即 `GET /api/errors` 中的 `error_type`。 |
| `message` | `str` | 错误页面对该错误的说明。 |
| `file` | `str` | 引发错误的指令所在的文件，路径相对于账本目录；文件不在账本目录中时为完整路径。 |
| `line` | `int` | 指令在文件中的行号。目前总是 `NULL`。 |
| `column` | `int` | 指令在行中的列号。目前总是 `NULL`。 |
| `date` | `date` | 指令的日期，没有日期的指令为 `NULL`。 |
| `account` | `str` | 错误涉及的账户，错误没有指明账户时为 `NULL`。 |
| `source` | `str` | 引发错误的指令的文本。 |

每种错误各有多少：

```sql
SELECT kind, count(*) AS errors
FROM #errors
GROUP BY kind
ORDER BY errors DESC
```

按文件中的顺序列出某个文件的错误：

```sql
SELECT date, kind, account, source
FROM #errors
WHERE file = 'data/2024.zhang'
```

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
| `account_sortkey(str) -> str` | 一个排序键，先按账户类型排序（顺序为 `Assets`、`Liabilities`、`Equity`、`Income`、`Expenses`），再按名字排序。它由类型的序号（`0` 到 `4`）、`-` 和账户名组成。第一段不完全等于这些类型之一的名字得到 `5`，因此排在它们之后。[`BALANCES`](#balances) 按这个键排序。 | `account_sortkey('Expenses:Food')` 为 `'4-Expenses:Food'` |

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

元数据的值总是以文本形式返回。一个键重复出现时，返回它的第一个值。在[其他表](#其他表)上，这三个函数都读取该行指令的元数据。在 `#budgets` 上读取 `budget` 指令的元数据，在 `#errors` 上读取张记账为错误记录的信息。

交易中哪些元数据行属于分录取决于文件格式，见[交易](/zh-cn/directives/transaction/#哪些行属于分录)。例如对于

```zhang
2024-01-02 * "Cafe" "lunch"
  category: "meals"
  Assets:Cash -10 CNY
  Expenses:Food 10 CNY
    category: "food"
```

`Expenses:Food` 分录的 `meta('category')` 为 `'food'`，`entry_meta('category')` 为 `'meals'`，`any_meta('category')` 为 `'food'`；`Assets:Cash` 分录则分别为 `NULL`、`'meals'` 和 `'meals'`。

### 字符串函数

| 签名 | 说明 | 示例 |
|------|------|------|
| `str(any) -> str` | 任意值的文本形式。布尔值为 `TRUE` 和 `FALSE`，集合用 `, ` 连接，库存写在括号中。 | `str(2024-01-31)` 为 `'2024-01-31'` |
| `length(str) -> int` | 字符串中的字符数。 | `length('Food')` 为 `4` |
| `length(set) -> int` | 集合中的元素个数。 | `length(tags)` |
| `maxwidth(str, int) -> str` | 把文本缩短到 `n` 个字符以内，与 Python 的 `textwrap.shorten` 相同，详见下文。 | `maxwidth('Paying the  rent', 12)` 为 `'Paying [...]'` |

`maxwidth(text, n)` 分两步处理：

1. 每一段连续的空白都变成一个空格，并去掉两端的空格。如果此时文本不超过 `n` 个字符，就原样返回：`maxwidth('  Eating out ', 48)` 为 `'Eating out'`。
2. 更长的文本保留尽可能多的完整单词，使其连同占位符 ` [...]` 一起不超过 `n` 个字符，占位符加在末尾。如果连第一个单词都放不下，结果为 `'[...]'`。与 Python 一样，单词也可以在两个字母之间的连字符后断开：`maxwidth('abc-def-ghi jkl', 12)` 为 `'abc- [...]'`。

`n` 至少为 5，即 `[...]` 的长度，更小的宽度会报错。[`JOURNAL`](#journal) 用 `maxwidth` 缩短收款方和描述。

## HTTP API

查询页面使用的就是下面这些 HTTP 接口，你也可以在脚本中调用它们。如果启用了[身份认证](/zh-cn/installation/3-authentication/)，请先登录，或者像调用其他 API 一样，用 HTTP Basic `Authorization` 头携带 `ZHANG_AUTH` 的凭证。

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
- 整数是 64 位的，以 JSON 数字发送。JavaScript 把 JSON 数字读作双精度浮点数，所以超出 ±2^53（9,007,199,254,740,992）的 `int` 单元格在 JavaScript 客户端中会损失精度。计数和日期部分远远达不到这个范围。
- 持仓未按成本持有时 `cost` 为 `null`；否则包含单位成本的 `number` 和 `currency`，以及成本批次的 `date` 和 `label`，后两者都可能为 `null`。
- 库存中的持仓按单位货币排序，再按成本排序，没有成本的持仓排在最前。空库存为 `{"positions": []}`。

### 错误

无法解析、类型检查或执行的查询返回 HTTP 状态码 400。与成功响应不同，响应体没有包在 `data` 中。例如 `SELECT nosuchcolumn, position` 会返回：

```json
{
  "message": "unknown column 'nosuchcolumn'",
  "line": 1,
  "column": 8
}
```

- `line` 和 `column` 给出问题在查询文本中的位置，都从 1 开始。
- `column` 按 Unicode 字符而不是字节计数，一个汉字或带重音的字母算作一列。
- 错误包括语法错误、未知的列或函数、参数类型错误、不合法的 `GROUP BY` 用法、无效的正则表达式、不支持的语句或子句，以及超出下面的[限制](#限制)。
- 有些错误没有位置信息，此时 `line` 和 `column` 为 `null`：查询过长、查询超时、结果过大，以及少数在计算各行时发现的错误（例如 `sum` 中的整数溢出）。

### 限制

这些限制防止查询占用过多的内存或时间。超出任何一项都会返回上面描述的 HTTP 400 错误。

| 限制 | 值 | 错误 |
|------|----|------|
| 查询长度 | 64 KiB（65,536 字节的 UTF-8 文本） | `the query is too long (...)`，没有位置信息。 |
| 嵌套深度 | 64 层 | `the query is nested too deeply (at most 64 levels)`，位置为达到限制的地方。 |
| 单个正则表达式编译后的大小 | 1 MiB | `invalid regular expression: Compiled regex exceeds size limit ...`，位置指向该模式。 |
| 执行时间 | 10 秒 | `the query was stopped because it ran longer than the 10s time limit`，没有位置信息。 |
| 结果大小 | 默认 1,000,000 个值 | `the result is too large: ...`，没有位置信息。 |

- 嵌套深度统计的是相互嵌套的括号、函数调用、`IN` 列表、`NOT` 和一元负号。由 `AND`、`OR`、`+` 或 `*` 连接的长链（例如 `account = 'A' OR account = 'B' OR ...`）不算嵌套，在长度限制以内可以任意长。
- 执行时间包括构建 `postings` 表各行以及应用会计期间子句的时间。查询运行期间会持有账本的读锁，时间限制也限定了持有读锁的时长。
- 结果大小把每个单元格计为一个值，库存中的每个持仓、集合中的每个元素以及文本中的每 64 字节各再计一个值。查询在 `ORDER BY`、`DISTINCT` 和 `LIMIT` 之前收集的行也计算在内，聚合查询在构建过程中的分组，以及 [`#budgets`](#预算表) 生成的月份（每个月份计一个值）同样如此。`PIVOT BY` 生成的表计算所有单元格（包括空单元格），并在构建之前检查。超出限制的查询会报错，错误信息建议用 `FROM` 或 `WHERE` 缩小查询范围，或者加上 `LIMIT`。
- 服务器管理员可以通过环境变量 `ZHANG_QUERY_MAX_RESULT_VALUES` 调高或调低结果大小的限制。
- `LIMIT` 可以让结果保持较小，[累计余额](#累计余额)的计算方式也有帮助：除非查询按 `balance` 排序、分组或去重，否则只为最终出现在结果中的行构建 `balance`。`units(balance)` 和 `cost(balance)`（以及 `JOURNAL ... AT units` 和 `AT cost`）按货币累加，不保留批次。
- [CSV 导出](#csv-导出)同样受这些限制。

### CSV 导出

`POST /api/query/csv` 接受与 `POST /api/query` 相同的 JSON 请求体，并以 CSV 文件的形式返回结果：

```shell
curl -X POST http://localhost:8000/api/query/csv \
  -H 'Content-Type: application/json' \
  -d '{"query": "SELECT account, sum(position) AS total WHERE account ~ \"^Assets:Broker\" GROUP BY account"}'
```

- 响应的内容类型为 `text/csv; charset=utf-8`，并带有 `Content-Disposition: attachment; filename="query.csv"` 头。
- 查询失败时，返回与 `POST /api/query` 相同的 HTTP 400 错误和 JSON 响应体，见[错误](#错误)。

为了方便在电子表格中使用，金额会像 beanquery 的 *numberify* 选项（`bean-query -m`）那样转换为纯数字：

- 名为 `name` 的 `amount`、`position` 或 `inventory` 列，按货币拆分为多个 `decimal` 列，每种货币一列，列名为 `name (CUR)`。没有出现任何货币的列会被省略。
- 由同一列拆分出的各列，按其货币出现的行数从多到少排列；行数相同时按货币名降序排列，所以 `USD` 排在 `EUR` 之前。
- `amount` 把数值写在其货币对应的列中。为零的金额视为缺失：单元格为空，也不计入该列的货币。
- `position` 给出其单位的数值，不含成本。单位为零时写作 `0`。
- `inventory` 给出每种货币的单位合计，把所有批次相加，不计成本。合计为零时单元格为空。
- 其他列保持不变。

例如，对于一个现金账户和一个黄金持仓，上面的查询得到：

```text
account,total (USD),total (GLD)
Assets:Broker:Cash,855.83,
Assets:Broker:GLD,,17
```

文件遵循 RFC 4180：

- 第一条记录是列名。每条记录都以 CRLF 结尾，最后一条也不例外。
- `NULL` 为空字段。布尔值写作 `TRUE` 和 `FALSE`，日期写作 `YYYY-MM-DD`，集合写作排好序、用 `,` 连接的元素。
- 数字是精确的：保留全部数字和小数位，不会补空格、不会舍入，也不会使用指数写法。
- 含有 `,`、`"`、回车或换行的字段会用双引号括起来，其中的 `"` 写成两个。只有一个空字段的行写作 `""`，以免成为空行。

:::caution[电子表格中的公式]
与 beanquery 的 CSV 输出一样，文本按原样写出，不做任何防止公式的处理。以 `=`、`+`、`-` 或 `@` 开头的文本单元格（例如收款方或描述），在电子表格软件中打开文件时可能被当作公式。对于并非由你自己编写文本的账本，导出后请谨慎打开，或者把这些列作为文本导入。
:::

### 列出保存的查询

`GET /api/query/saved` 按账本顺序列出用 [`query` 指令](/zh-cn/directives/query/)保存在账本中的查询。每一项包含 `name`、查询文本 `query`、指令的日期 `date`，以及 `valid` 和 `error`，后两者说明该查询能否被当前的查询引擎编译以及不能编译的原因。响应示例见 [`query` 指令](/zh-cn/directives/query/#http-api)。要执行保存的查询，把它的 `query` 文本发送到 `POST /api/query`。

### Schema

`GET /api/query/schema` 描述每个表和每个函数重载。查询页面的参考面板就是根据它生成的。

```json
{
  "data": {
    "columns": [
      { "name": "date", "type": "date", "description": "Date of the transaction." }
    ],
    "functions": [
      { "name": "count", "signature": "count(*) -> int", "description": "Number of rows.", "aggregate": true }
    ],
    "tables": [
      {
        "name": "postings",
        "description": "One row per posting of every transaction, ...",
        "columns": [{ "name": "date", "type": "date", "description": "Date of the transaction." }]
      }
    ]
  }
}
```

- `columns` 每列一项，共 24 项，顺序与[列](#列)表格相同，最后一项是累计余额 `balance`。
- `tables` 每个表一项，先是 `postings`，然后按[其他表](#其他表)中列出的顺序排列，最后是 `budgets` 和 `errors`。`name` 不带 `#`。`postings` 一项的列与 `columns` 相同；结构化列的字段以 `open.date` 这样的名字列为单独的列。
- `functions` 每个重载一项，共 66 项：先是聚合函数，然后是标量函数，其中包括 `account_sortkey` 和 `maxwidth`。`signature` 的写法与本页表格相同；[聚合函数](#聚合函数)的 `aggregate` 为 `true`，其他函数为 `false`。

## 示例

### 按类别统计每月支出

```sql
SELECT year, month, root(account, 2), sum(position) WHERE account ~ "^Expenses" GROUP BY 1, 2, 3 ORDER BY 1, 2, 3
```

每个月、每个二级支出账户（如 `Expenses:Food`）一行，给出总支出。`GROUP BY 1, 2, 3` 和 `ORDER BY 1, 2, 3` 通过序号引用前三个目标。

### 超过某个金额的支出类别

```sql
SELECT root(account, 2) AS category, sum(position) AS total
WHERE account ~ '^Expenses' AND currency = 'USD'
GROUP BY category
HAVING sum(number) > 1000
ORDER BY category
```

支出超过 1000 USD 的支出类别。`HAVING` 在分组合计之后过滤分组；`WHERE` 做不到，因为它每次只看到一条分录。

### 每月支出，每年一列

```sql
SELECT month, year, sum(position) AS total
WHERE account ~ '^Expenses:Food'
GROUP BY month, year
PIVOT BY month, year
```

每个月一行，每年一列，不同年份的同一个月并排显示。某年某月没有分录时，单元格为空。

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

### 某一年的损益表

```sql
SELECT account, sum(position) AS total
FROM OPEN ON 2024-01-01 CLOSE ON 2025-01-01
WHERE account ~ '^(Income|Expenses)'
GROUP BY account
ORDER BY account
```

`OPEN ON` 把以前各年的收入和支出转入权益，`CLOSE ON` 丢弃 2025 年及以后的所有内容，所以合计只包含 2024 年。

### 资产负债表

```sql
BALANCES FROM CLOSE ON 2025-01-01 CLEAR
WHERE account ~ '^(Assets|Liabilities|Equity)'
```

2025 年初资产、负债和权益账户的余额，按账户类型排序。`CLEAR` 把所有收入和支出转入 `Equity:Earnings:Current`，所以权益账户中包含了收益。

### 按成本计的持仓

```sql
BALANCES AT cost WHERE account ~ '^Assets'
```

每个资产账户的账面价值，以其持仓的成本货币计。

### 账户流水

```sql
JOURNAL 'Assets:Bank:Checking' FROM OPEN ON 2024-01-01 CLOSE ON 2025-01-01
```

该账户在 2024 年的每一条分录，带有累计余额。第一行是 2023 年 12 月 31 日的期初余额，所以余额列显示的是账户的真实余额。

### 每天结束时的余额

```sql
SELECT date, last(balance) AS closing
WHERE account = 'Assets:Bank:Checking'
GROUP BY date
ORDER BY date
```

每个有分录的日期一行，给出该账户当天结束时的余额。结果由一个日期列和一个库存列组成，所以查询页面会把它绘制成折线图。

## 与 BQL 和 beanquery 的差异

### 尚未支持

- **`PRINT`**，使用时会报错。
- `FROM` 后面的**子查询**、beanquery 用双引号写的表名（`FROM "prices"`），以及它的单行表 `FROM #`。
- postings 表中 **beanquery 的列** `posting_flag`、`filename`、`lineno`、`location`、`entry`、`accounts` 和 `type`，以及 `#entries` 的 `lineno`：张记账不保存行号。
- **下标访问**，例如 `meta['name']`。请使用 `meta('name')`。
- **`BETWEEN` 和 `%` 运算符**，以及 beanquery 的带引号标识符。
- **本页未列出的函数**，例如 `round`、`safediv`、`has_account`、`open_date`、`close_date`、`open_meta`、`currency_meta`、`grep`、`subst`、`upper`、`lower`、`joinstr`、`findfirst`，类型转换函数 `int`、`decimal` 和 `date`，以及 `date_*` 系列函数。调用它们会报错。


### 行为不同之处

- **`SELECT *` 包含 `account`。**beanquery 把 `*` 展开为 `date, flag, payee, narration, position`。张记账在 `position` 之前加入了 `account`，因为没有账户的分录很难看懂。
- **标准的三值逻辑。**在 beanquery 中，`NOT NULL` 为 `TRUE`，所以 `NOT (payee = 'x')` 会保留没有收款方的分录；`NULL AND FALSE` 为 `NULL`。在张记账中，与 SQL 一样，`NOT NULL` 为 `NULL`，`NULL AND FALSE` 为 `FALSE`。
- **单元素列表可以使用。**`payee IN ('Amazon')` 在张记账中可以正常使用。beanquery 会把 `('Amazon')` 当作带括号的字符串，必须写成 `('Amazon',)`。
- **注释**以 `--` 开头。不支持 beanquery 的 `;` 行注释和 `/* */` 块注释。`;` 只能出现在查询末尾。
- **正则表达式**使用 Rust 语法，不支持环视和反向引用。
- **记账方法。**使用 `STRICT`、`AVERAGE`、`AVERAGE_ONLY` 或 `NONE` 的账户目前按 FIFO 记账，`STRICT` 下有歧义的匹配也不会报错，见[批次记账](#批次记账)。
- **限制。**查询的长度、嵌套深度、正则表达式大小、执行时间和结果大小都有限制，见[限制](#限制)。
- **错误带有位置信息。**只要能定位，每个查询错误都会给出出错的行和列。
- **全程使用精确小数。**数字是任意精度的十进制数，金额不会以固定的小数位数存储。
- **`BALANCES` 和 `JOURNAL` 的列名**与等价的 `SELECT` 相同：`sum(position)`、`sum(cost(position))` 和 `maxwidth(payee, 48)`。beanquery 把它们命名为 `SUM((position))`、`SUM(cost(position))` 和 `MAXWIDTH(payee, 48)`。
- **累计余额。**`balance` 不能用在 `FROM` 或 `WHERE` 中，它累加的正好是通过这两个子句的行。beanquery 在每次计算该列时更新余额，所以在 `WHERE` 子句中，它累加的是被测试的行，而不是被保留的行。
- **`account_sortkey`** 对第一段不是账户类型的名字，返回排在所有类型之后的键。beanquery 会报错。
- **参数。**`JOURNAL` 的模式以及 `OPEN ON` 和 `CLOSE ON` 的日期可以是[参数](#参数)。beanquery 在这些地方只接受字面量。
- **`FROM` 中的表达式在会计期间子句之后过滤。**这与 beanquery 一致。在 BQL v2 中，该表达式在应用 `OPEN`、`CLOSE` 和 `CLEAR` 之前选择交易。
- **权益账户。**`account_previous_*` 或 `account_current_*` 选项的值如果不是有效的账户名，会被忽略，并使用默认账户。
- **期间内的最后一笔条目**决定 `CLEAR` 的 `T` 交易和不带日期的 `CLOSE` 的 `C` 交易的日期。它不考虑张记账特有的预算指令，Beancount 没有这类指令。
- **`HAVING` 中的列。**对于 `HAVING` 中在聚合函数之外使用的列，beanquery 会从任意一条分录读取。张记账把分组键读作每个分组的值，其他列则会报错。
- **`HAVING` 必须是布尔表达式。**beanquery 也接受其他值，保留值不为零或不为空的分组，例如 `HAVING sum(number)`。张记账与 `WHERE` 一样拒绝这类条件，应写成 `HAVING sum(number) != 0`。
- **`PIVOT BY` 遇到 `NULL` 值。**透视目标的值中既有 `NULL` 又有其他值时，beanquery 会出错。张记账把 `NULL` 排在最前，对应的列命名为 `NULL`。
- **没有分组的 `PIVOT BY`** 在张记账中会报错。beanquery 在执行这类查询时出错。
- **导出缺少单元格的透视结果为 CSV。**beanquery 无法对透视后金额或库存列中的空单元格做 numberify。张记账把它们留空。
- **元数据是文本。**beanquery 的 `meta` 列是字典，其中还有 `filename` 和 `lineno`。张记账的 `meta` 是指令自身元数据的文本 `key: "value", ...`（在 postings 表中是分录自己的元数据），`open.meta` 和 `close.meta` 也是如此。
- **`#accounts` 的 `open` 和 `close` 不带字段时读作日期。**在 beanquery 中它们是整条指令。
- **`entry_meta()` 和 `any_meta()` 与 `meta()` 一样可用于每个表。**beanquery 只在 postings 表上接受它们。
- **`#entries` 包含张记账的指令。**其中有张记账的预算指令；`balance ... with pad` 是一条 `balance` 记录，后面跟着它的补齐交易，而 beancount 中是一条 `pad` 和一条 `balance` 记录。记录的 `id` 是张记账的 ID，不是 beancount 的哈希值。
- **`discrepancy` 遵循张记账的余额检查。**每条余额断言之后，张记账都会把账户调整到断言的金额，因此断言的差额是相对于上一条断言计算的。beancount 在断言不成立或仅在容差内成立时，不会调整账户。
- **CSV 导出保留精确的数字。**`bean-query` 会为对齐而在数字前补空格（`" 600.00"`），把 numberify 后的数字舍入到各货币的显示精度（`360.03` 而不是 `360.03016`），有些数字还会用指数写法（`1E+3`）。张记账都不会这样做。
