---
title: Account
description: This is a page in my Starlight-powered site
---

## Defining an Account

```zhang
{DATE} open {ACCOUNT_NAME} {COMMODITY1} {COMMODITY2}
```

When defining an account, you can specify account details through meta:

- `alias` displays an alias on the web page
- `booking_method` is used for complex investment systems, with optional
  values `STRICT`, `FIFO`, `LIFO`, `AVERAGE`, `AVERAGE_ONLY`, `NONE`

### booking_method

Beancount users can directly define the booking method after the commodity in the account definition using double
quotes. For example:

```beancount
1970-01-01 open Assets:Card CNY "NONE"
```