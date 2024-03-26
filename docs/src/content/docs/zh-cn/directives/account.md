---
title: Account
description: This is a page in my Starlight-powered site
---

# 账户 Account

## 定义账户

```zhang
{DATE} open {ACCOUNT_NAME} {COMMODITY1} {COMMODITY2}
```

定义账户时可以通过 meta 来指定账户细节：

- `alias` 在 web 页面中显示成别名
- `booking_method` 用于复杂的投资系统，可选值 `STRICT`, `FIFO`, `LIFO`,`AVERAGE`,`AVERAGE_ONLY`,`NONE`

### booking_method

beancount 用户可以在 定义账户的 commodity 后面直接用双引号定义。 例如

```beancount
1970-01-01 open Assets:Card CNY "NONE"
```