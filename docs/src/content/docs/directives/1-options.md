---
title: Options
description: Configure your ledger settings with various options
---

# Options

Options are used to configure various aspects of your ledger. They can be set using the `option` directive in your ledger file.

## Available Options

### Operating Currency

The default currency used for transactions.

```beancount
option "operating_currency" "CNY"
```

- **Default**: `CNY`
- **Example values**: `USD`, `EUR`, `JPY`

### Default Rounding

Specifies how numbers should be rounded in calculations.

```beancount
option "default_rounding" "round_down"
```

- **Default**: `round_down`
- **Available values**:
  - `round_down`: Round down to the nearest decimal place
  - `round_up`: Round up to the nearest decimal place
  - `round_half_up`: Round to the nearest decimal place, rounding up when exactly halfway

### Default Balance Tolerance Precision

Sets the precision for balance tolerance checks.

```beancount
option "default_balance_tolerance_precision" "2"
```

- **Default**: `2`
- **Example**: `2` means 0.01 precision
- **Usage**: Used to determine how precise balance checks should be

### Default Commodity Precision

Sets the precision for commodity amounts.

```beancount
option "default_commodity_precision" "2"
```

- **Default**: `2`
- **Example**: `2` means 0.01 precision
- **Usage**: Used to determine how precise commodity amounts should be displayed

### Default Booking Method

Specifies the default method for booking transactions.

```beancount
option "default_booking_method" "FIFO"
```

- **Default**: `FIFO`
- **Available values**:
  - `FIFO`
  - `STRICT`
  - `LIFO`
  - `AVERAGE`
  - `AVERAGE_ONLY`
  - `NONE`
  
### Timezone

Sets the timezone for date and time calculations.

```beancount
option "timezone" "Asia/Shanghai"
```

- **Default**: System timezone
- **Example values**: `Asia/Shanghai`, `America/New_York`, `Europe/London`
- **Usage**: Affects how dates and times are interpreted in your ledger


### Directive Output Path

Defines the file path pattern for storing new directives. Uses Python's [Jinja2](https://jinja.palletsprojects.com/) template engine for path formatting.

```beancount
option "directive_output_path" "data/{{year}}/{{month_str}}.zhang"
```

- **Default**: `data/{{year}}/{{month_str}}.zhang`
- **Available placeholders**:
  - `{type}`: directive type
  - `{year}`: Current year (e.g., `2023`)
  - `{month}`: Current month (e.g., `1` for January)
  - `{month_str}`: Current month (e.g., `01` for January)
  - `{day}`: Current day (e.g., `5`)
  - `{day_str}`: Current day (e.g., `05`)
- **Usage**: When adding new transactions or other directives, they will be stored in files according to this pattern

#### Example Path Patterns

```beancount
; Store by month (default)
option "directive_output_path" "data/{{year}}/{{month}}.zhang"

; Store by day
option "directive_output_path" "data/{{year}}/{{month}}/{{day}}.zhang"

; Store by transaction type in separate files
option "directive_output_path" "data/{{year}}/{{type}}-{{month}}.zhang"

; Store everything in a single file
option "directive_output_path" "data/ledger.zhang"
```

## Usage Examples

Here's an example of setting multiple options in your ledger file:

```beancount
option "operating_currency" "USD"
option "default_rounding" "round_half_up"
option "default_balance_tolerance_precision" "2"
option "default_commodity_precision" "2"
option "default_booking_method" "FIFO"
option "timezone" "America/New_York"
```

## Notes

- Options should be set at the beginning of your ledger file
- Each option can only be set once
- Some options may affect how transactions are processed and displayed
- Custom options can be added for specific needs, but they won't affect the core functionality 