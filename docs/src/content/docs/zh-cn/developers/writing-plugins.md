---
title: 编写插件
description: 如何用 Rust SDK 为张记账编写 WASM 插件，以及插件 ABI v1 参考：plugin 指令及其能力、阶段顺序、配置键、宿主函数、确定性、custom 指令配置和 router 插件。
sidebar:
  order: 1
---

插件是一个 WebAssembly 模块，张记账在加载账本时，或者在 HTTP 请求到达它时运行它。用插件可以：

- **转换**账本：添加、修改或删除指令，例如生成周期性交易、拆分支出或给条目打标签；
- **校验**账本：在账本的错误列表中报告问题，而不做任何修改；
- **提供**页面和数据：响应 `/api/plugins/{name}` 下的 HTTP 请求，例如一份自定义报表。

插件基于 [Extism](https://extism.org/) 构建，所以任何有 Extism plug-in kit 的语言都可以使用。本指南使用 Rust SDK `zhang-plugin-sdk`，并为其他所有人记录了它底层的 ABI。

:::caution[信任]
插件能看到你的整个账本。只声明你信任的插件，并且只授予每个插件它所需要的能力。
:::

## 启用插件

插件默认关闭。在账本中开启它们：

```zhang
option "features.plugin" "true"
```

`features.plugins` 也可以。没有这个选项时，`plugin` 指令会被忽略，它们的模块也从不会被读取。

## `plugin` 指令

```zhang
plugin "plugins/guard.wasm" "strict"
  threshold: "500 CNY"
  allowed_paths: "guard"
  timeout: "10s"
```

- 第一个值是模块。张记账通过账本的数据源读取它，所以路径相对于账本根目录，并且对存放在 S3、WebDAV 或 GitHub 上的账本同样适用。
- 之后的值是**位置参数**（这里是 `"strict"`）。Beancount 的 `plugin "module" "config"` 就是这样传递它的配置字符串的。
- 元数据行包含张记账授予插件的**能力**（见下文）以及插件自己的**配置**（这里是 `threshold`）。

以 `zhang.` 开头的元数据键保留给张记账设置的值。

### 能力

除非它的指令授予，插件在自己的内存之外什么也做不了。

| 元数据 | 授予 | 默认值 |
|---|---|---|
| `allowed_hosts` | 向这些主机发起 HTTP 请求。多个主机时重复这个键。 | 完全不能联网 |
| `allowed_paths` | 对账本中这些文件和目录的只读访问。多个时重复这个键。 | 不能访问文件 |
| `timeout` | 对插件的一次调用最多可以运行多久：整数秒（`"90"`），或者带单位 `ms`、`s`、`m` 或 `h` 的数字（`"500ms"`、`"2m"`），最长一天 | 60 秒 |
| `seed` | 不授予任何东西；任意文本，混入插件的[种子](#确定性) | 无 |
| `stage` | 插件的 processor 和 mapper 在哪个阶段运行：`"booked"`，在张记账完成记账之后；或 `"raw"`，在记账之前，看到的是按原样写下的交易（见[阶段顺序约定](#阶段顺序约定)） | `"booked"` |

无效的 `timeout`、`allowed_paths` 或 `stage` 值会在该指令上报告为 [`ParseInvalidMeta`](/zh-cn/reference/error-codes/#parseinvalidmeta) 错误，插件则使用默认值。

#### `allowed_paths`

```zhang
plugin "plugins/receipts.wasm"
  allowed_paths: "documents"
  allowed_paths: "statements/2024.csv"
```

- 每个值是一个文件或目录，相对于账本根目录，用 `/` 分隔。`"."` 授予整个根目录。
- 目录会授予其下的所有内容，按路径组成部分逐段比较：`documents` 不会授予 `documents-private/`。
- **点规则：** 在授予的路径之下，隐藏名称（以 `.` 开头的名称）只有在某个值明确写出它时才可读取。所以 `"."` 不会暴露 `.git/config`、`.env` 或 `.cache/`，而 `".config"` 授予 `.config/…`，`"documents/.receipts"` 恰好授予那个目录。列出目录时不包含隐藏条目。
- 空值、绝对路径，或者含有 `..` 组成部分、NUL 字符或反斜杠的值，不授予任何东西。
- 访问是**只读**的，并且账本根目录之外的任何内容都无法访问：在本地磁盘上，符号链接只有在仍处于授予范围内时才会被跟随。单个文件最大 16 MiB，一次列出目录最多 10 000 个条目。
- 只有 processor 或 mapper 能读取文件。插件读取的每个文件或目录都会被记录下来，当其中任何一个发生变化、出现或消失时，`zhang serve` 会重新加载本地账本。

:::danger[同时授予文件和网络]
同时授予 `allowed_paths` 和 `allowed_hosts`，插件就能把它读到的内容发送到你的机器之外。插件本来就会收到整个账本，所以只有对你在这两方面都信任的插件，才同时授予文件和主机。
:::

## 插件类型

插件在它的 `supported_type` 导出函数中声明自己是什么，并且可以同时是多种类型。

| 类型 | 导出函数 | 何时运行 | 输入 → 输出 |
|---|---|---|---|
| `Processor` | `processor` | 每次加载一次 | 整个指令流 → 新的指令流 |
| `Mapper` | `mapper` | 每条指令一次，整个指令流共用一个插件实例 | 一条指令 → 替换它的指令（零条、它自己或多条） |
| `Router` | `router` | 每个 HTTP 请求一次，每次都在新的实例中 | 请求 → 响应 |

同时是 processor 和 mapper 的插件会先运行 processor。张记账不认识的类型会被忽略并给出警告，所以为更新版本的张记账编写的插件仍然可以加载。

## 阶段顺序约定

张记账加载账本时，指令流按以下顺序依次经过这些阶段：

1. **声明了 `stage: "raw"` 的插件**，按照它们的 `plugin` 指令声明的顺序；
2. **记账**：对每笔交易记账，就像 Beancount 在运行插件之前先记账一样。没有写金额的记账行得到由其他记账行推算出的金额；成本变成它所匹配批次的单位成本和取得日期；跨多个批次的卖出变成每个批次一行。张记账无法记账的交易（例如有两行没写金额）保持原样；
3. **你的其他插件**，按照它们的 `plugin` 指令声明的顺序；
4. **再次记账**，仅当第 3 步有插件运行时：插件添加或改动的内容按真实的批次记账，后面的步骤看到的指令流与张记账据以构建账本的完全一致。对已记账的交易再记账不会有任何改变，所以什么都没改的插件在这里没有任何开销；
5. **活跃账户**：报告对未开设账户的记账行；
6. **补齐**：为每条 `pad` 和 `balance … with pad` 添加其断言所需的补齐交易（标记为 `P`）；
7. **余额检查**：检查每条余额断言。这一阶段不做任何记账：不成立的断言是一条错误。

然后由最终的指令流构建账本：各阶段留下的未记账内容（插件添加的、带有没写金额记账行的交易，补齐交易）在这里补记，报告记账错误，并检查每笔交易是否平衡。

插件能看到的内容：

- **完整的指令流**，按日期排序：没有日期的指令（`option`、`plugin`、`include`、注释）在最前；同一日期内，先是 `open` 和 `commodity`，然后是余额指令，最后是其他所有指令。张记账在每个阶段之后都会重新排序，所以插件可以按任意日期顺序返回指令；同一日期、同一种类的指令保持插件返回它们的顺序，也就是它们在当天的顺序。
- 所有类型的指令，包括 `custom`、`option` 和 `plugin` 指令。选项和插件在各阶段运行之前就已应用，所以插件添加的 `option` 或 `plugin` 指令不起作用。
- **记账后的交易**，与 Beancount 插件看到的一样：每个记账行都有金额，每个成本都有单位成本数字和日期，跨多个批次的卖出是多行，每个批次一行。被记账改动过的记账行带有 [`written`](#导出函数) 字段，记录用户原本写下的内容；请原样传递它。声明了 `stage: "raw"` 的插件看到的则是**按原样写下**的交易：没有写金额的记账行此时还没有金额，成本也还没有与批次匹配。在记账之前改写记账行的插件（例如补全账户或金额的插件）应选择它。
- **记账后的行保持不变。** 每一行记录的是你的插件运行之前记账所匹配到的批次。插件如果改动了指令流，使这一行本可以匹配到别的批次（例如插入了一笔更早卖出该批次的交易），张记账也不会重新为它记账：构建账本时张记账会对最终的指令流再记账一次，把不再成立的部分作为该交易上的错误报告（卖空的批次报告为 [`NoEnoughCommodityLot`](/zh-cn/reference/error-codes/#noenoughcommoditylot)），并保留这一行，就像 Beancount 保留插件返回的内容一样。以总成本买入的行（例如 3 个单位的 `{{1000 USD}}`）的单位成本是精确的商，可能很长；原样写下的总成本在该行的 `written.cost` 中。
- `balance` 指令本身，但看不到补齐阶段创建的补齐交易（标记为 `P`）：补齐阶段在插件之后运行。
- **看不到 `pad` 指令。** ABI v1 早于 [`pad` 指令](/zh-cn/reference/directives/balance/#pad-指令)，基于较旧 `zhang-ast` 构建的插件无法读取它。所以张记账在调用插件之前会把每条 `pad` 放到一边。由某条 `pad` 补齐的 `balance` 会以张记账支持 `pad` 之前的样子，即带有该 `pad` 填充账户的 `balance … with pad`，交给插件：插件看到的指令流与以前 Beancount 账本给它的一样。插件返回的内容以插件为准，张记账只在 `pad` 仍然补齐插件所返回内容的地方把它放回：
  - 如果一条 `pad` 的所有 `balance … with pad` 都以同一账户、同一填充账户的 `balance … with pad` 返回，这条 `pad` 会以该账户和填充账户放回，所以插件可以重命名二者之一或更换填充账户。这些余额会变回带着原有容差的 `balance`，并保留插件对它们做的其他所有修改。什么都不改的插件会原样拿回它收到的指令流；
  - 如果插件丢弃了一条 `pad` 的某条 `balance … with pad`、把它变成了普通的 `balance`，或者给了它与其他几条不同的账户或填充账户，这条 `pad` 就会被略去；放回后会补齐它原本所代表之外的余额的 `pad` 也会被略去（例如插件把某条余额移到了另一天，或者在某条余额之前添加了该账户的余额）。此时插件返回的每条 `balance … with pad` 都像 `balance … with pad` 一样补齐它自己的断言；
  - 不补齐任何余额的 `pad` 对插件不可见，插件无法修改或丢弃它。它会原样放回，并且仍然必须不补齐任何余额：放回后如果会补齐某条余额，它就会被略去。

  被略去的 `pad` 不补齐任何金额，会报告为 [`UnusedPad`](/zh-cn/reference/error-codes/#unusedpad) 错误，与放回后不补齐任何金额的 `pad` 一样。

  放回的 `pad` 只补齐它原本所代表的余额。插件返回的其他任何 `balance`，例如它添加的，或者它变成普通 `balance` 的，都不会被它看不到的 `pad` 补齐，而是按原样检查。

  插件目前还看不到 `pad`；向插件开放它是今后 ABI 的工作。

**为什么插件在补齐和余额检查之前运行。** 先运行插件，意味着补齐金额是在插件添加的所有交易之后才计算的，所以账户最终总是等于你写下的金额。Beancount 在插件之前运行 `pad`，在插件之后检查 `balance`，因此在 Beancount 中，插件向被补齐的账户添加的交易会使断言不成立。对于一个正常工作的 Beancount 账本，两种方式的补齐金额相同。只有检查补齐交易本身的插件才会注意到差别，而且它仍然能看到 `balance` 指令。

## 使用 Rust SDK 快速上手

`zhang-plugin-sdk` 位于张记账仓库中，与张记账一起发布版本；它还没有发布到 crates.io。请把它固定到你所运行的张记账发布版：指令以 `zhang-ast` 的 JSON 结构跨越边界传递，基于较旧 `zhang-ast` 构建的插件无法读取较新张记账新增的指令类型。张记账不会把 ABI v1 之后新增的类型（`pad` 指令）交给 v1 插件。

1. 创建一个库 crate，并把它设为 `cdylib`：

   ```toml
   # Cargo.toml
   [package]
   name = "large-expense"
   version = "0.1.0"
   edition = "2021"

   [lib]
   crate-type = ["cdylib"]

   [dependencies]
   zhang-plugin-sdk = { git = "https://github.com/zhang-accounting/zhang", tag = "vX.Y.Z" }
   ```

2. 编写插件。`plugin!` 会导出 `name`、`version`、`supported_type`（根据你提供的处理函数推导）以及各个处理函数，并替你处理 JSON：

   ```rust
   // src/lib.rs
   use zhang_plugin_sdk::config::Config;
   use zhang_plugin_sdk::{custom, errors, plugin, Directive, Error, Stream};

   const NAME: &str = "large-expense";

   plugin! {
       name: NAME,
       version: env!("CARGO_PKG_VERSION"),
       processor: process,
   }

   fn process(stream: Stream) -> Result<Stream, Error> {
       // 扁平配置、指令的元数据，以及 `custom "large-expense" …` 指令
       let config = Config::load().with_custom(custom::entries(NAME, &stream));
       for directive in &stream {
           let Directive::Transaction(txn) = &directive.data else { continue };
           let date = txn.date.naive_date();
           let Some(threshold) = config.resolve("threshold", date, Some(&txn.meta)) else { continue };
           let threshold = threshold.amount(0)?;
           for posting in &txn.postings {
               if let Some(units) = &posting.units {
                   if units.commodity == threshold.commodity && units.number > threshold.number {
                       errors::emit_error_at(&directive.span, format!("{} is a large expense", posting.account.name()), [("rule", "threshold")]);
                   }
               }
           }
       }
       Ok(stream)
   }
   ```

3. 为 `wasm32-unknown-unknown` 构建它：

   ```sh
   rustup target add wasm32-unknown-unknown
   cargo build --release --target wasm32-unknown-unknown
   cp target/wasm32-unknown-unknown/release/large_expense.wasm ~/ledger/plugins/
   ```

4. 在账本中声明它：

   ```zhang
   option "features.plugin" "true"
   plugin "plugins/large_expense.wasm"
     threshold: "500 CNY"

   2024-07-01 custom "large-expense" "threshold" 300 CNY
   ```

SDK 提供的内容：

| 模块 | 用途 |
|---|---|
| `plugin!` | 导出函数，以及带类型的处理函数：`fn(Stream) -> Result<Stream, Error>`、`fn(Spanned<Directive>) -> Result<Stream, Error>`、`fn(Request) -> Result<Response, Error>` |
| `config` | `Config::load()`、`get`（扁平键）、`abi`、`plugin`（参数和多值元数据）、`meta`、`option`、`seed`、`resolve`，以及用来解析数字、金额、日期、布尔值和账户的 `Values` |
| `custom` | 用于 [`custom` 配置](#custom-指令中的配置)的 `entries(name, &stream)` 和 `latest(…)` |
| `clock` | `now()`、`today()`、`rng()`、`rng_for(&directive)` |
| `fs` | `read_file`、`read_to_string`、`list_dir` |
| `errors` | `emit_error(message)`、`emit_error_at(span, message, metas)` |
| `prices` | 用于[汇率](#汇率)的 `PriceMap::from_stream(&stream)`、`rate(base, quote, date)`、`convert(amount, target, date)` |
| `router` | `Request`、`Response`、`query(bql)`、`ledger_info()` |

在原生 target 上 SDK 依然可以编译：`plugin!` 不导出任何东西，宿主函数返回 `unavailable`，所以 `cargo test` 可以在没有张记账的情况下运行你的插件逻辑，并用 `Config::from_map(...)` 代替宿主提供的配置。[`zhang-plugin-sdk/examples`](https://github.com/zhang-accounting/zhang/tree/main/zhang-plugin-sdk/examples) 中有三个完整的插件，两个 processor（`guard`，以及展示记账后视图的 `lots`）和一个 router；张记账自己的测试会构建并运行它们。

## 确定性

账本每次加载的结果应当相同。张记账无法强制保证这一点，所以这是一项约定：

- **从张记账读取时间**，使用 `zhang_now` 宿主函数（SDK 中为 `clock::now()` / `clock::today()`）。张记账每次加载只读取一次时钟，所以每个插件看到的都是同一时刻，使用账本的时区。读取时间会让账本依赖于日期，`zhang serve` 会在午夜重新加载这样的账本。
- **从种子派生随机数**，即 `zhang.seed` 配置（SDK 中为 `clock::rng()` 和 `clock::rng_for(&directive)`）。种子只取决于插件的指令：它的模块路径、它在同一模块的指令中的位置，以及它的 `seed` 元数据，所以生成的 id 和链接在每次重新加载时都保持不变。`rng_for` 还会混入指令的文本，所以文件其他部分发生变化时，id 不会改变。
- **为 `wasm32-unknown-unknown` 构建。** 为 WASI 构建的插件可以读取宿主真实的时钟和熵源，张记账无法拦截；这样的插件不可复现。

## `custom` 指令中的配置

随时间变化的配置应当放在账本中，写成带日期的 `custom` 指令，其第一个值是插件的名称：

```zhang
2024-01-01 custom "large-expense" "threshold" 100 CNY
2024-07-01 custom "large-expense" "threshold" "150 CNY"
```

日期为 2024-03-05 的条目看到的是 2024-01-01 的阈值；日期为 2024-08-01 的条目看到的是 2024-07-01 的阈值。值保持为字符串：`100 CNY` 会作为 `"100"` 和 `"CNY"` 两个值传入，SDK 的 `Values::amount` 两种形式都能读取。

`Config::resolve(key, date, entry_meta)` 按以下顺序查找设置，第一个包含该键的来源胜出：

1. 正在处理的条目的元数据；
2. 日期在该条目当天或之前的最新一条 `custom "<plugin>" "<key>" …`（同一天有多条时取最后一条）；
3. 插件的 `plugin` 指令的元数据；
4. 同名的账本选项。

只有 processor 能看到整个指令流，所以只有 processor 能读取 `custom` 配置；mapper 每次只能看到一条指令。

## 汇率

张记账从不为插件预先计算价格。需要汇率的 processor 可以用 SDK 的 `PriceMap`，从它收到的指令流中构建汇率：

```rust
use zhang_plugin_sdk::prices::PriceMap;

let prices = PriceMap::from_stream(&stream);
let rate = prices.rate("USD", "CNY", date);              // Option<BigDecimal>
let value = prices.convert(&posting_units, "CNY", date); // Option<Amount>
```

它给出的汇率与张记账查询引擎在 `convert`、`value` 和 `getprice` 中使用的相同，所以插件的估值与张记账一致。规则与 Beancount 相同：

- 某一天的汇率是**该日或之前最新的一条 `price`**。第一条价格之前没有汇率。
- 同一货币对**在同一天的价格会相互替换**：指令流中的最后一条胜出。
- 自身没有价格的货币对使用反方向货币对汇率的**倒数** `1 / rate`；价格为零时没有倒数，会被跳过。
- **两个方向都有报价**的货币对共用一条合并后的历史。保留价格较多的那个方向，另一个方向取倒数后并入其中，所以汇率是两个方向中最新的报价。同一天有两个方向的报价时，报价较少的方向胜出。
- 商品对自身的汇率为 1。

**精度：** 来自 `price` 指令的汇率是精确的。能除尽的倒数也是精确的（`1 / 8 = 0.125`）；除不尽的倒数按银行家舍入法（half-even）保留 28 位有效数字，与 Beancount 的 decimal 上下文和张记账的查询引擎相同（`1 / 7 = 0.1428571428571428571428571429`）。`convert` 只有在乘积超过 28 位有效数字时才会将其舍入到 28 位。

**隐含价格：** `PriceMap::from_stream_with_implicit` 还会采用记账行上写的价格，即单价 `@` 和总价 `@@`，与 Beancount 的 `implicit_prices` 插件相同。张记账本身不使用这些价格，所以它们添加的汇率与张记账显示的不同；这是为从 Beancount 移植过来的插件提供的可选功能。没有写数量的记账行会被跳过：只有以 `stage: "raw"` 运行的插件才会看到这样的记账行，它此时还没有价格。

## 报告错误

插件有两种方式表示出了问题：

- 用 `zhang_emit_error`（`errors::emit_error` / `emit_error_at`）**报告**。问题会成为账本错误列表中的一个 [`PluginError`](/zh-cn/reference/error-codes/#pluginerror)，挂在你传入其 span 的指令上（或者插件自己的指令上），带有元数据 `plugin`、`message` 以及你自己的元数据。账本仍然会加载。校验类插件就是这样工作的。
- 让调用**失败**：从处理函数返回错误（或者 trap、panic）。processor 或 mapper 失败会中止整个加载，调用运行超过它的 `timeout` 也一样。这种方式用于用户必须先修复、否则账本就没有意义的问题，例如无效的插件配置。

## Router 插件

router 插件为 `/api/plugins/{name}` 及其下的所有路径提供服务，支持任意 HTTP 方法：

```rust
use zhang_plugin_sdk::router::{self, Request, Response};
use zhang_plugin_sdk::{plugin, Error};

plugin! {
    name: "summary",
    version: env!("CARGO_PKG_VERSION"),
    router: route,
}

fn route(request: Request) -> Result<Response, Error> {
    match request.path.as_str() {
        "/balances" => Response::json(&router::query("SELECT account, sum(position) AS balance GROUP BY account")?),
        _ => Ok(Response::text("not found").with_status(404)),
    }
}
```

- **路由：** `{name}` 是插件的名称；请求的 `path` 是路由之下的部分。
- **身份认证：** 这些路由位于张记账自己的登录保护之后，凭据请求头永远不会传给插件。
- **只读：** router 只能通过 `zhang_query`（BQL）和 `zhang_ledger_info` 读取账本；没有任何宿主函数能修改账本。每个请求都在新的实例中运行，运行期间账本不会重新加载。
- **安全：** router 的页面由张记账自己的地址提供，所以这类页面上的脚本可以用你的会话调用张记账的 API，包括会修改账本文件的接口。放进 HTML 的所有内容都要转义。

[Router 插件](/zh-cn/guides/router-plugins/)介绍了请求和响应的 JSON、错误状态码以及更多细节。

## ABI v1 参考

本节说明跨越边界传递的内容，供不使用 Rust SDK 编写的插件参考。所有变化都是增量的：字段永远不会被移除或改为必填，所以为较旧张记账编译的插件可以继续工作。

### 导出函数

输入和输出是 Extism plug-in 的输入和输出，格式为 JSON。

| 导出函数 | 输入 | 输出 |
|---|---|---|
| `name` | 无 | 插件的名称，一个 JSON 字符串：`"guard"` |
| `version` | 无 | 它的版本，一个 JSON 字符串：`"0.1.0"` |
| `supported_type` | 无 | 由 `"Processor"`、`"Mapper"`、`"Router"` 组成的 JSON 数组 |
| `processor` | 指令流：一个指令数组 | 新的指令流 |
| `mapper` | 一条指令 | 一个指令数组 |
| `router` | 请求 | 响应 |

一条指令是 `zhang-ast` 中 `Spanned<Directive>` 的 serde JSON，并且是 ABI v1 认识的类型：张记账从不把 `pad` 指令交给插件（参见[阶段顺序约定](#阶段顺序约定)）。例如：

```json
{"data": {"Comment": {"content": "; a note"}}, "span": {"start": 0, "end": 8, "content": "; a note", "filename": "/ledger/main.zhang"}}
```

导出函数通过返回非零代码并附带 Extism 错误来表示失败；对于 processor 或 mapper，这会中止加载。

记账行可以带有 `written` 字段：当记账改变了这个记账行时，它记录用户原本写下的内容，形如 `{"index": 0, "units": …, "cost": …}`（`index` 是原记账行在交易中的位置，一笔减持被拆到多个批次时，拆出的几行相邻并共用同一个 `index`；没写金额的记账行 `units` 为 `null`；`cost` 是原样写下的成本说明）。它只是辅助信息：张记账从不用它计算余额、批次或错误，只用它把流水和导出按原样显示。张记账没有改动的记账行不带这个字段，所以其序列化结果与该字段出现之前完全相同；基于旧版 `zhang-ast` 构建的插件会丢掉它，影响的只是这些行的显示方式。请原样传递它；插件改动了某个记账行的数量或成本时，应去掉它的 `written` 字段，这样流水显示的就是插件写入的内容。一笔拆分出的几行必须保持相邻、属于同一账户并保留各自的 `written`，张记账才会按原样显示和导出这个记账行；被挪开的行，或者两个记账行共用同一个 `index`，会按记账后的样子显示和导出，所以不会丢失任何内容。

### 配置

张记账为每个插件实例提供一个字符串到字符串的 Extism 配置：

| 键 | 值 |
|---|---|
| 账本选项的键 | 该选项的值 |
| `plugin` 指令的元数据键 | 它的值，重复的键取最后一个；优先于同名的选项。`allowed_hosts` 永远不会以这种方式传递 |
| `zhang.abi` | ABI 版本，`"1"` |
| `zhang.plugin` | 按原样写下的指令，格式为 JSON：`{"module": "…", "args": ["…"], "meta": {"key": ["value", …]}}`，包含每个元数据键的所有值（包括 `allowed_hosts`） |
| `zhang.seed` | 插件的种子，一个十进制的 `u64` |

没有设置 `zhang.abi` 的张记账早于 ABI v1：它不设置任何 `zhang.*` 键，也不链接下面的任何宿主函数。

### 宿主函数

它们位于 `extism:host/user` 命名空间中。参数和结果都是 Extism 内存块的 `i64` 偏移量。每个有返回值的函数都返回 JSON：`{"Ok": value}` 或 `{"Err": {"kind": "…", "message": "…"}}`，并且都不会 trap。

| 函数 | 参数 | `Ok` 值 | 错误类型 |
|---|---|---|---|
| `zhang_emit_error` | `{"message": "…", "span": {…}?, "metas": {"k": "v"}?}` | 无返回值 | 无：无法读取的参数本身会被报告 |
| `zhang_now` | 无 | `{"now": "2024-03-16T00:30:00+08:00", "today": "2024-03-16", "timezone": "Asia/Shanghai"}` | 目前没有；但仍要处理 `Err` |
| `zhang_read_file` | 路径，UTF-8 | `{"content": "…", "encoding": "utf8" \| "base64"}` | `denied`、`not_found`、`too_large`、`unsupported`、`invalid` |
| `zhang_list_dir` | 路径，UTF-8 | `{"entries": [{"name": "…", "kind": "file" \| "dir"}]}`，按名称排序 | 同 `zhang_read_file` |
| `zhang_query` | BQL 文本 | `{"columns": [{"name": "…", "type": "…"}], "rows": [[…]]}` | `query`（带 `line` 和 `column`）、`invalid_input`、`unavailable` |
| `zhang_ledger_info` | 无 | `{"title": "…" \| null, "operating_currency": "CNY", "timezone": "Asia/Shanghai"}` | `unavailable` |

它们在哪里可用：

- `zhang_emit_error` 在 processor 或 mapper 中把问题报告到错误列表。插件注册期间（`name`、`version`、`supported_type`）报告的内容会被丢弃，在 router 中则只会写入日志。
- `zhang_now` 在任何地方都可用。在 router 中，它为每个请求重新读取时钟；在注册期间调用它不会让账本依赖于日期。
- `zhang_read_file` 和 `zhang_list_dir` 在 processor 或 mapper 之外，以及对 `allowed_paths` 未授予的任何路径，都返回 `denied`。
- `zhang_query` 和 `zhang_ledger_info` 在 router 请求之外返回 `unavailable`。

### 最低版本要求

**导入某个宿主函数，会使插件在没有该函数的张记账上无法加载**，报 `unknown import` 错误，即使插件从未调用它。Rust SDK 只会导入你的插件实际调用的宿主函数。

| 功能 | 张记账版本 |
|---|---|
| `processor` 和 `mapper` 导出函数、扁平配置（选项和元数据）、`allowed_hosts` | 0.2.0 |
| 真正提供服务的 `router` 导出函数、`zhang.abi`、`zhang.plugin`、`zhang.seed`、上面所有宿主函数、`allowed_paths`、`timeout`、`seed`、`features.plugins`、忽略未知插件类型 | 0.2.0 之后的发布版 |
