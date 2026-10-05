---
title: 记录交易
description: 在账本文件或网页界面中记录交易，了解新条目写入哪个文件、标签和链接的用法，以及如何附加元数据。
sidebar:
  order: 1
---

一笔交易在账户之间转移金额：工资到账、付午饭钱、还信用卡账单。你可以用任意文本编辑器把交易写进账本文件，也可以在网页界面中录入。无论哪种方式，交易最终都以纯文本保存在你的文件中。

本指南使用一个小型家庭账本。完整语法见[交易](/zh-cn/reference/directives/transaction/)。

## 编写交易

```zhang title="data/2024-03.zhang"
2024-03-01 * "ACME Corp" "三月工资"
  Assets:Bank:Checking 12,000.00 CNY
  Income:Salary
```

- 首行包含日期、标记、收款方和摘要。`*` 表示已完成的交易，`!` 表示还需要核对的交易。
- 其下每一个缩进行是一条**记账行**：一个账户和一个金额。
- 一笔交易的记账行在每种商品上都必须相加为零。这里 `Income:Salary` 没有金额，张记账会给它填上使交易平衡的金额：`-12,000.00 CNY`。
- 一笔交易只能有一条记账行省略金额，而且其余记账行的合计必须只有一种商品。否则张记账无法确定该填什么：它会报告错误，并把这笔交易排除在账本之外。
- 金额可以用 `,` 或 `_` 分隔数位，也可以使用简单的算术：`-(2 * 420) CNY` 即 `-840 CNY`。

标记后只有一个字符串时，这个字符串是摘要：

```zhang
2024-03-10 ! "打车"
  Assets:Cash -35 CNY
  Expenses:Travel 35 CNY
```

交易用到的每个账户都必须先用 [`open`](/zh-cn/reference/directives/account/) 开设。每种商品都必须用 [`commodity`](/zh-cn/reference/directives/commodity/) 声明，[主货币](/zh-cn/reference/directives/options/#operating_currency)除外。

## 标签、链接和元数据

```zhang
2024-03-09 * "酒店" "杭州两晚" #trip-hangzhou ^booking-8812
  invoice: "INV-2024-0309"
  Liabilities:CreditCard -840 CNY
  Expenses:Travel
    receipt: "hotel.pdf"
```

- **标签**（`#trip-hangzhou`）按主题归组交易，例如一次旅行或一个项目。**链接**（`^booking-8812`）把同一件事的多笔交易联系在一起，例如一笔预订和它的退款。标签和链接写在摘要之后，顺序不限。
- **元数据**行是 `key: value` 形式的键值对。紧跟在首行下面的行属于交易（`invoice`）。缩进比记账行更深的行属于该记账行（`receipt`）。
- 在流水页面中，选中一个标签或链接即可按它筛选流水。
- 在[查询](/zh-cn/guides/querying/)中，`'trip-hangzhou' IN tags` 选出带该标签的记账行，`entry_meta('invoice')` 读取交易的元数据，`meta('receipt')` 读取记账行的元数据。
- `document` 元数据会把一个文件附加到交易上。见[文档](/zh-cn/guides/documents/)。

## 日期和时间

张记账账本中的日期可以带上一天中的时间，写作 `HH:MM` 或 `HH:MM:SS`：

```zhang
2024-03-02 12:30 * "面馆" "午餐" #work
  Liabilities:CreditCard -48 CNY
  Expenses:Food 48 CNY
```

- 时间使用账本的时区，即 [`timezone` 选项](/zh-cn/reference/directives/options/#timezone)。它默认是运行张记账的机器的时区。
- 条目按日期和时间排序。没有时间的条目视为当天的开始（`00:00:00`）。日期和时间相同的条目保持它们在文件中的顺序。
- 时间对[余额断言](/zh-cn/guides/balances/)最重要，余额断言检查的是它所标注时刻的余额。
- Beancount 的日期没有时间。在 Beancount 账本中，张记账把 `time: "12:30:00"` 元数据读作时间，写入时也用这种方式记录时间。

## 把账本拆分成多个文件

账本变大后，可以把账户和每个时期的交易分别放在单独的文件中，再在主文件中用 [`include`](/zh-cn/reference/directives/include/) 引入它们：

```zhang title="main.zhang"
option "title" "Household"
option "operating_currency" "CNY"

include "accounts.zhang"
include "data/*.zhang"
```

- 相对路径相对于引入它的文件。
- `*` 只在一个目录内匹配：`data/*.zhang` 读取直接位于 `data/` 中的所有 `.zhang` 文件，但不读取 `data/2024/` 中的文件。模式的限制见 [`include`](/zh-cn/reference/directives/include/#通配符)。
- 主文件的扩展名决定账本中每个文件的语法：`main.zhang` 使用张记账语法，`main.bean` 使用 Beancount 语法。

## 在网页界面中记录交易

选择侧边栏顶部的**新建交易**，然后填写：

- **日期**：选择哪一天。新交易会带上当前的时间。
- **收款方**和**摘要**。
- **分录**：每条记账行的账户和金额，金额写作“金额 货币”的形式，例如 `-28 CNY`，读法与账本文件相同：`1,000 CNY` 和 `(10 + 2) / 4 USD` 也可以；只写数字时，货币为[运营货币](/zh-cn/reference/directives/options/#operating_currency)。留空一个金额，张记账会自动填上。
- **分录详情**（点开记账行的详情按钮）：它的**成本**和**价格**，写法与账本文件相同（成本如 `{150 USD}`、`{{1500 USD}}` 或 `{}`，价格如 `@ 6 USD` 或 `@@ 60 USD`，见[批次与成本基础](/zh-cn/guides/lots-and-cost-basis/)），行尾的**注释**，以及它的元数据。
- **元数据**：交易的键值对。

输入时，张记账会按保存后账本检查交易的方式检查它，并在表单中显示结果：

- **预览**显示将写入文件的文本。
- 张记账读不懂的字段（例如成本缺少花括号）会标出原因；修正之前无法保存这笔交易。
- 如果分录不平衡，记账行下方会显示相差多少。每条记账行按账本的方式计算权重，即按成本或价格折算，并按各货币的精度舍入。因此 `10 AAPL {150 USD}` 与 `-1500 USD` 是平衡的。
- 表单还会列出账本会报告的其他错误，例如账户在交易日期已关闭，或卖出时没有可卖的批次。这样的交易仍可保存，就像你可以把它写进文件一样。

表单中没有标签和链接的字段。需要这些时，请在文件中编写交易，例如使用**编辑**页面在浏览器中编辑账本文件。

要修改一笔交易，在流水页面中打开该行的菜单，选择**编辑**。张记账会把修改后的交易写回原处，即它所在的文件。它根据表单重写整笔交易：每条记账行的成本、价格、注释、元数据和[标记](/zh-cn/reference/directives/transaction/#记账行标记)都会保留，未写金额的记账行也保持原样；只有原文的排版和记账行之间（或首行末尾）的注释行不会保留；当编辑会丢弃这类注释行时，张记账会先请你确认。插件生成的交易不能编辑：它不在账本的任何文件中，编辑会被拒绝，不会写入任何内容。

API 也是如此：`PUT /api/transactions/{id}` 为每条记账行接受同样写法的 `cost`、`price` 和 `comment`；省略这些字段时保留该记账行原有的值，传 `null` 则删除。记账行的 `unit` 可以是对象 `{"number": "-28", "commodity": "CNY"}`，也可以是金额的文本，例如 `"-28 CNY"`，读法与表单相同。`POST /api/transactions/preview` 和 `POST /api/transactions/{id}/preview` 接受与创建、更新相同的请求体，返回表单的检查结果，不写入任何内容：

- `text`：将写入的文本。
- `field_errors`：创建或更新会拒绝的每个字段：所属记账行和字段、`kind`（如 `invalid_amount` 或 `beancount_commodity`）及其涉及的 `value`，以及 400 响应的消息。
- `unbalanced`：交易相差多少。
- `errors`：账本会报告的错误。

### 新条目写入的位置

张记账把你在网页界面中记录的内容追加到一个文件中，这个文件由条目的日期和 [`directive_output_path` 选项](/zh-cn/reference/directives/options/#directive_output_path)决定。默认值是 `data/{{year}}/{{month_str}}.{{ext}}`，其中 `ext` 是主文件的扩展名。记在 2024 年 4 月 3 日的一杯咖啡会写入 `data/2024/04.zhang`：

```zhang title="data/2024/04.zhang"
2024-04-03 13:15:00 * "Coffee Lab" "澳白"
  Assets:Cash -28 CNY
  Expenses:Food
```

- 如果账本还没有读取这个文件，张记账还会在主文件末尾加上 `include "data/2024/04.zhang"`。
- 网页界面新增的其他条目也都写到同一个位置：在账户页面或用**批量对账**工具写入的余额断言，以及上传到账户的文档。
- 换一个模板就能改变文件布局。`option "directive_output_path" "data/{{year}}.{{ext}}"` 每年一个文件。模板可以使用 `year`、`month`、`month_str`、`day`、`day_str`、`type`（条目的种类）和 `ext`。
- 在 Beancount 账本中，文件使用主文件的扩展名，例如 `.bean`，并以 Beancount 语法写入，一天中的时间记在 `time` 元数据中。

## 出现问题时

张记账每次加载账本时都会检查它。

- **条目中的问题**会进入账本的错误列表。侧边栏中的**总览**项显示问题的数量，总览页面列出这些问题：选中一个即可查看它所涉及的条目，并在**编辑**页中打开它所在的文件、定位到那一行。常见的问题有交易不平衡、账户未开设和商品未声明。[错误码](/zh-cn/reference/error-codes/)逐一解释了每种错误。
  - 不平衡的交易仍留在账本中，在流水中标记为**不平衡**。
  - 无法补全缺失金额的交易，例如有两条记账行都没有金额，在你修正之前会被排除在账本之外。
- **语法错误**会让整个账本无法加载。张记账启动时遇到语法错误会退出，并给出错误以及发现它的文件、行和列。张记账运行期间出现语法错误时，它会继续提供最后一次成功加载的版本，只在日志中记录错误：网页界面不显示错误。如果你的修改没有出现，请查看终端或 `docker logs`。

## 重新加载

对于本地磁盘上的账本，`zhang serve` 会监视账本目录，在账本的某个文件变化时重新加载，因此你在文本编辑器中的修改一两秒内就会出现：哪些变化会触发重新加载，见[本地文件系统](/zh-cn/deployment/data-sources/local/)。在网页界面中记录内容，或在编辑页面中保存文件，也会重新加载账本。

**重新加载账本**按钮，即侧边栏中账本标题旁的圆形箭头，可以随时重新加载账本。账本存放在 [S3、WebDAV 或 GitHub](/zh-cn/deployment/data-sources/s3/) 上时请使用它：在这些地方，张记账不会察觉其他程序所做的修改。
