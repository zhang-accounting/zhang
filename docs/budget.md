# 预算

zhang 的预算系统是基于 YNAB 的模式构建的，其模式本质是 Zero-based budgeting。预算系统为 zhang 引入了 4 条指令与 1 个账户相关的配置

预算系统与账户并不会参与核心系统的余额计算逻辑里面。比如说分配了一部分金额给预算账户，并不会导致某个资产账户的余额缩减。
## 指令集
### 新建预算账户
```zhang
{DATE} budget {BUDGET_NAME} {CURRENCY}
```
#### meta 配置
 - `alias`: (**可选**) 由于 BUDGET_NAME 只能是英文与下划线，所以`alias` 提供了更加语义化的描述用于页面展示
 - `category`: (**可选**) 用于把预算账户在前端页面分组展示

### 预算账户增加金额
```zhang
{DATE} budget-add {BUDGET_NAME} {AMOUNT} {CURRENCY}
```

### 额度转移
```zhang
{DATE} budget-transfer {FROM_BUDGET_NAME} {TO_BUDGET_NAME} {AMOUNT} {CURRENCY}
```
预算系统存在一种场景：当我们需要根据实际情况调整预算额度的时候，需要从一个预算账户划转部分金额到另外一个预算账户，那么就需要使用额度转移指令。

举一个更加具体的例子：
```zhang
// 2023年12月，我希望吃饭的钱省一点给女朋友买个礼物
2023-12-01 budget-transfer Diet GirlFriendGift 200 CNY
```

### 关闭预算账户
```zhang
{DATE} budget-close {BUDGET_NAME}
```

### 消费账户的绑定
```zhang
{DATE} open {ACCOUNT_NAME} {COMMODITY}
  budget: {BUDGET_NAME}
```
为了使预算系统可以正确的计算**已消耗额度**与**可使用额度**，我们需要把消费账户绑定到预算账户上，所以可以使用 `budget`的 meta 在消费账户建立时绑定上预算账户

```zhang
// 把午餐账户绑定到 Diet 的预算中
1970-01-01 open Expenses:Lunch CNY
  budget: Diet
```


## Beancount 兼容性

为了保证 Beancount 用户也可以使用预算系统，所以我们把指令都在beancount的预算上做了兼容与转移，具体的语法可以参考：
 - 新建预算账户
   - zhang: `{DATE} budget {BUDGET_NAME} {CURRENCY}`
   - beancount: `{DATE} custom budget {BUDGET_NAME} {CURRENCY}`
- 预算账户增加金额
    - zhang: `{DATE} budget-add {BUDGET_NAME} {AMOUNT} {CURRENCY}`
    - beancount: `{DATE} custom budget-add {BUDGET_NAME} {AMOUNT} {CURRENCY}`
- 额度转移
    - zhang: `{DATE} budget-transfer {FROM_BUDGET_NAME} {TO_BUDGET_NAME} {AMOUNT} {CURRENCY}`
    - beancount: `{DATE} custom budget-transfer {FROM_BUDGET_NAME} {TO_BUDGET_NAME} {AMOUNT} {CURRENCY}`
- 关闭预算账户
    - zhang: `{DATE} budget-close {BUDGET_NAME}`
    - beancount: `{DATE} custom budget-close {BUDGET_NAME}`

