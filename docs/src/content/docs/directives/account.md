---
title: Account Directives
description: Comprehensive guide on utilizing account directives within Zhang Accounting.
---

## Account Directives Overview

Account directives are fundamental in Zhang Accounting, allowing users to define and manage various accounts for their financial transactions. This section delves into the account directive, its usage, and provides examples to illustrate its application in different scenarios.

### Defining an Account

To define an account, use the following syntax:

```zhang
{DATE} open {ACCOUNT_NAME} {COMMODITY1} {COMMODITY2}
```

The `open` directive is used to initiate an account, specifying the commodities it deals with. Here are additional details you can specify through meta configurations:

- `alias`: Assigns a more descriptive name for display purposes.
- `booking_method`: Specifies the method used for handling complex investment scenarios. Options include `STRICT`, `FIFO`, `LIFO`, `AVERAGE`, `AVERAGE_ONLY`, `NONE`.

#### Booking Method

The booking method is crucial for accounts involved in investments, affecting how transactions are recorded and reported. Beancount users can directly define the booking method after the commodity in the account definition using double quotes, as shown below:

```beancount
1970-01-01 open Assets:Card CNY "NONE"
```

### Examples of Account Usage

#### Example 1: Opening a Savings Account

```zhang
2023-01-01 open Assets:Savings USD
```

This example demonstrates how to open a savings account that deals with USD.

#### Example 2: Defining an Investment Account with Alias

```zhang
2023-01-01 open Investments:Stocks USD
  alias: "Tech Stocks"
  booking_method: "FIFO"
```

Here, an investment account for stocks is created, with an alias for easier identification and a FIFO booking method for managing stock transactions.

### Meta Configurations Explained

Meta configurations offer additional customization for accounts, enhancing their functionality and reporting capabilities. Understanding these configurations allows for more precise account management and reporting.

- `alias`: Provides a user-friendly name for the account, useful for reports and tracking.
- `booking_method`: Determines how transactions are processed, especially important for investment and trading accounts.

By leveraging account directives and meta configurations effectively, users can tailor Zhang Accounting to their specific financial tracking and reporting needs.
