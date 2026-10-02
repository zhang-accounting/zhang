---
title: Query Language
description: Reference for Zhang's BQL-compatible query language, covering syntax, the postings table, types, functions, the HTTP API and the differences from Beancount's query language.
---

Zhang can answer ad-hoc questions about your ledger with a small query language. It understands a subset of the [Beancount Query Language (BQL)](https://beancount.github.io/docs/beancount_query_language/), so most `SELECT` queries written for Beancount or Fava work without changes. When BQL v2 and its successor [beanquery](https://github.com/beancount/beanquery) disagree, Zhang does what beanquery does.

Queries run directly against the ledger that Zhang has already loaded into memory. They are read-only, and all arithmetic uses exact decimals, so amounts never pass through floating point.

:::caution[Early version]
This page covers the first version of the query language (Phase 1 of [#434](https://github.com/zhang-accounting/zhang/issues/434)): `SELECT` queries over a single `postings` table. [Differences from BQL and beanquery](#differences-from-bql-and-beanquery) lists what is not available yet.
:::

## Running a query

### In the web UI

Open the **Query** page (**查询** in the Chinese interface) at `/explore`.

- Type a query in the editor. It runs only when you click the run button or press <kbd>Ctrl</kbd>+<kbd>Enter</kbd> (<kbd>Cmd</kbd>+<kbd>Enter</kbd> on macOS), never while you are typing.
- Results are shown as a table, and each cell is rendered according to its [type](#types). An inventory cell shows one position per line.
- If the query has an error, the editor highlights the line and column where it was found.
- The examples menu inserts ready-made queries, and the reference panel lists every column and function.

### Over HTTP

Send the query to `POST /api/query`. See [HTTP API](#http-api) for the request and response format.

## A first query

```sql
SELECT date, payee, account, position
WHERE account ~ '^Expenses:Food'
ORDER BY date DESC
LIMIT 10
```

This returns the ten most recent postings to `Expenses:Food` and its sub-accounts.

- Each row is one posting. `date` and `payee` come from the transaction, while `account` and `position` come from the posting itself.
- `~` matches a regular expression anywhere in the text and ignores case. The `^` anchors the pattern to the start of the account name. Note that `'^Expenses:Food'` also matches `Expenses:Foodstuff`. Use `'^Expenses:Food(:|$)'` to match only the account and its children.
- `ORDER BY date DESC` puts the newest postings first, and `LIMIT 10` keeps the first ten rows.

## Query syntax

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

- The clauses must appear in the order shown. All of them except `SELECT` are optional.
- Keywords are case-insensitive: `select`, `SELECT` and `Select` are the same keyword.
- Spaces and line breaks between tokens are not significant, so a query can span several lines.
- Column and function names are written in lowercase, as they appear in the tables on this page.

### How a query is evaluated

1. `FROM` and `WHERE` choose which postings take part.
2. If the query uses an [aggregate function](#aggregate-functions), the chosen postings are grouped and each group produces one row. Otherwise each posting produces one row.
3. `ORDER BY` sorts the rows.
4. `DISTINCT` removes duplicate rows.
5. `LIMIT` keeps the first rows and drops the rest.

### SELECT

The targets are the expressions to compute for each row, separated by commas.

- `SELECT *` is short for `SELECT date, flag, payee, narration, account, position`.
- `AS name` gives a target a name. The name can be used in `GROUP BY` and `ORDER BY`, but not in `WHERE`.
- Each result column is named after its alias if it has one, otherwise after the column it reads (for a bare column such as `account`), otherwise after the expression as written (for example `sum(position)`).
- `SELECT DISTINCT` removes rows that are identical to an earlier row. Only the selected values are compared.

<!-- TODO(lead): confirm the default column name for a non-column target. beanquery uses the expression's source text, stripped of surrounding whitespace (compiler.get_target_name). Is the text normalised (spacing, keyword case) in zhang-query? -->

### FROM

In Phase 1, `FROM` is followed by an expression, not a table name. It filters postings exactly like `WHERE`, and when a query has both, a posting must satisfy both:

```sql
SELECT account, sum(position)
FROM year = 2024
WHERE account ~ '^Expenses'
GROUP BY account
```

is the same as `... WHERE (year = 2024) AND (account ~ '^Expenses') ...`. This matches beanquery, and it means BQL queries that filter in `FROM` keep working. The period clauses `OPEN ON`, `CLOSE ON` and `CLEAR` are not supported yet; see [Workarounds](#workarounds).

<!-- TODO(lead): beanquery treats `FROM <identifier>` as a table name when the identifier is not a column (e.g. `FROM postings`, `FROM #postings`). Does zhang-query accept `FROM postings` as a no-op, or reject it as an unknown column? -->

### WHERE

`WHERE` keeps the postings for which the condition is `TRUE`. A condition that evaluates to `NULL` counts as not true, so the posting is dropped (see [NULL](#null)). Aggregate functions are not allowed in `WHERE` or `FROM`.

### GROUP BY

A query becomes an aggregate query when one of its targets uses an [aggregate function](#aggregate-functions). Postings are then grouped and each group produces one row.

- A group key can be an expression, the name given to a target with `AS`, or a target's position in the `SELECT` list, counting from 1. `GROUP BY 1, 2` groups by the first two targets.
- When aggregates are present, every target that is not an aggregate must be grouped, that is, it must appear in `GROUP BY` or be referred to there by name or number.
- A group key cannot itself contain an aggregate function, and aggregate functions cannot be nested (`sum(count(*))` is an error).
- A target that combines an aggregate with a column outside of it, such as `number - sum(number)`, is an error. Expressions built only from aggregates and constants, such as `sum(number) / 12` or `units(sum(position))`, are aggregates themselves and are fine.
- If every target is an aggregate and there is no `GROUP BY`, all matching postings form a single group.

<!-- TODO(lead): confirm the GROUP BY rule against beanquery's actual behaviour (compiler.py, _compile_group_by and _select):
  1. Without GROUP BY, beanquery does NOT reject a mix of aggregate and non-aggregate targets. With SUPPORT_IMPLICIT_GROUPBY = True it silently groups by all non-aggregate targets. The rule written above ("must be grouped") would turn such queries into errors.
  2. With GROUP BY, the set of non-aggregate targets must equal the set of group keys exactly. A GROUP BY expression that is not selected is added as a hidden key and is allowed.
  3. A non-aggregate ORDER BY expression that is not selected becomes a hidden target and must therefore also be grouped.
  4. beanquery rejects GROUP BY on unhashable types (set columns such as tags, inventories). What does zhang-query do?
  5. beanquery returns zero rows (not one row with count 0) when an all-aggregate query matches no postings.
-->

```sql
SELECT root(account, 2) AS category, sum(position) AS total
WHERE account ~ '^Expenses'
GROUP BY category
```

### ORDER BY

- An order key can be an expression, a target name or a target number, just like a group key.
- Each key has its own direction: `ASC` (ascending, the default) or `DESC` (descending). In `ORDER BY 1 DESC, 2`, the first key is descending and the second ascending.
- Rows are compared by the first key. The second key only breaks ties, and so on.
- `NULL` sorts lower than any other value. It comes first in ascending order and last in descending order.
- See [Ordering and comparison](#ordering-and-comparison) for how values of each type are ordered.
- Without `ORDER BY`, a plain query returns postings in ledger order (by date). The order of groups in an aggregate query is not specified. Add `ORDER BY` whenever the order matters.

<!-- TODO(lead): confirm the default row order without ORDER BY (store order of postings? groups in first-seen order like beanquery?). -->

### LIMIT

`LIMIT n` keeps the first `n` rows, after sorting and `DISTINCT`. `n` must be a non-negative integer literal.

## Literals

| Literal | Examples | Type |
|---------|----------|------|
| String | `'Food'`, `"USD"` | `str` |
| Integer | `0`, `42` | `int` |
| Decimal | `3.14`, `0.5` | `decimal` |
| Date | `2024-01-31` | `date` |
| Boolean | `TRUE`, `FALSE` | `bool` |
| Null | `NULL` | `null` |

- Strings can use single or double quotes. Unlike standard SQL, `"USD"` is a string, not a column name. To put a quote character inside a string, wrap the string in the other kind of quote: `"Joe's Diner"`.
- Dates are written **without** quotes, in `YYYY-MM-DD` form. `'2024-01-31'` with quotes is a string, not a date.
- There are no negative number literals. Write `-5`, which applies unary minus to `5`.
- `TRUE`, `FALSE` and `NULL` are keywords and are case-insensitive.

<!-- TODO(lead): string escapes are unspecified. beanquery allows '' inside single-quoted strings and has no escapes in double-quoted ones. Does zhang-query support any escape sequence? Comments (beanquery: /* */ and ; to end of line) and a trailing ';' are also unspecified. -->

## Operators

From highest to lowest precedence:

| Precedence | Operators | Meaning |
|------------|-----------|---------|
| 1 | `-x` | unary minus |
| 2 | `*` `/` | multiplication, division |
| 3 | `+` `-` | addition, subtraction |
| 4 | `=` `!=` `<` `<=` `>` `>=` | comparison |
| 4 | `~` `!~` | regular-expression match and non-match |
| 4 | `IN` `NOT IN` | membership |
| 4 | `IS NULL` `IS NOT NULL` | null test |
| 5 | `NOT` | logical negation |
| 6 | `AND` | logical and |
| 7 | `OR` | logical or |

Use parentheses to group explicitly: `(a OR b) AND c`. Because `NOT` binds more loosely than comparisons, `NOT account ~ '^Assets'` means `NOT (account ~ '^Assets')`.

### Arithmetic

`+`, `-`, `*` and `/` work on `int` and `decimal` values.

- `int` with `int` gives `int` for `+`, `-` and `*`.
- If either side is a `decimal`, the result is a `decimal`.
- `/` always produces a `decimal`, even for two integers: `7 / 2` is `3.5`.
- Dividing by zero produces `NULL` instead of an error.
- Addition, subtraction and multiplication of decimals are exact and never round.

<!-- TODO(lead): BigDecimal division needs a precision/scale limit for non-terminating results (1 / 3). What does zhang-query use? beanquery uses Python's decimal context (28 significant digits). Also unspecified: date +/- int, date - date, str + str (all supported by beanquery), and % (modulo). -->

### Comparison

`=`, `!=`, `<`, `<=`, `>` and `>=` compare two values of the same type. `int` and `decimal` can be compared with each other. String comparison is case-sensitive. Only `~` ignores case.

### Regular-expression match

`text ~ pattern` is `TRUE` when the regular expression `pattern` matches **any part** of `text`, ignoring case. `text !~ pattern` is the opposite.

| Expression | Result |
|------------|--------|
| `'Expenses:Food:Dining' ~ 'food'` | `TRUE`, a partial match that ignores case |
| `'Expenses:Food:Dining' ~ '^Food'` | `FALSE`, because `^` anchors to the start |
| `'Expenses:Food:Dining' ~ 'Dining$'` | `TRUE` |
| `'Expenses:Food:Dining' !~ '^Income'` | `TRUE` |

Anchor the pattern with `^` and `$` when you need a full match.

<!-- TODO(lead): regex dialect. If zhang-query uses Rust's `regex` crate, look-around and back-references are unavailable, unlike Python `re` in beanquery. Confirm, and confirm that an invalid pattern is a 400 error with a position. -->

### Membership

`IN` tests whether a value belongs to a set or a list:

```sql
WHERE 'trip-new-york' IN tags
WHERE 'Assets:Cash' NOT IN other_accounts
WHERE account IN ('Assets:Cash', 'Assets:Bank:Checking')
WHERE year IN (2023, 2024)
```

The right-hand side is either a `set` value, such as the `tags`, `links` or `other_accounts` columns, or a list of literals in parentheses. `NOT IN` is the opposite of `IN`.

<!-- TODO(lead): beanquery requires a comma in a literal list (a one-element list is written ('x',)); is `x IN ('a')` accepted by zhang-query? -->

### NULL

A value is `NULL` when it is missing, for example the payee of a transaction that has none, or the cost of a posting that is not held at cost.

- Arithmetic, comparisons, `~`, `!~`, `IN` and `NOT IN` with a `NULL` operand produce `NULL`.
- `WHERE` and `FROM` treat `NULL` as not true, so the posting is dropped.
- Test for missing values with `IS NULL` and `IS NOT NULL`. `payee = NULL` is always `NULL`, never `TRUE`.
- Note that `payee != 'Shop'` drops postings whose payee is `NULL`. Write `payee IS NULL OR payee != 'Shop'` to keep them.

<!-- TODO(lead): beanquery quirks to confirm: NOT is null-safe (NOT NULL evaluates to TRUE), and AND returns NULL as soon as an operand is NULL, even if a later operand is FALSE. Only the WHERE behaviour (NULL counts as false) is documented above. -->

## The postings table

Phase 1 has a single table, `postings`. It has one row for every posting of every transaction, with the transaction's fields repeated on each of its postings.

- **Included:** all transactions whatever their flag, and the padding transactions that Zhang creates for `balance ... with pad ...` directives. These have the flag `P`.
- **Not included:** balance assertions, and directives that are not transactions, such as `open`, `close`, `price`, `note`, `document` and budget directives.
- A posting written without an amount has the amount that Zhang inferred for it when it balanced the transaction.

### Columns

| Column | Type | Description |
|--------|------|-------------|
| `date` | `date` | Date of the transaction. The time of day, if any, is dropped. |
| `year` | `int` | Year of `date`. |
| `month` | `int` | Month of `date`, from 1 to 12. |
| `day` | `int` | Day of the month of `date`, from 1 to 31. |
| `flag` | `str` | Flag of the transaction, such as `*`, `!` or `P`. |
| `payee` | `str` | Payee of the transaction, or `NULL` if it has none. |
| `narration` | `str` | Narration of the transaction, or `NULL` if it has none. |
| `description` | `str` | Payee and narration joined with `" \| "`. If one of them is missing, only the other is used. |
| `tags` | `set` | Tags of the transaction, without the leading `#`. |
| `links` | `set` | Links of the transaction, without the leading `^`. |
| `id` | `str` | Identifier of the transaction. All postings of a transaction share it. |
| `account` | `str` | Account of the posting. |
| `number` | `decimal` | Number of units of the posting. |
| `currency` | `str` | Currency (commodity) of the units. |
| `position` | `position` | Units of the posting together with its cost lot, if any. |
| `cost_number` | `decimal` | Cost per unit, or `NULL` if the posting is not held at cost. |
| `cost_currency` | `str` | Currency of the cost, or `NULL`. |
| `cost_date` | `date` | Date of the cost lot, or `NULL`. |
| `cost_label` | `str` | Label of the cost lot, or `NULL`. |
| `price` | `amount` | Price per unit written with `@`, or `NULL` if there is none. |
| `weight` | `amount` | Amount that the posting contributes to balancing its transaction: the total cost if the posting is held at cost, otherwise units times price if it has a price, otherwise the units. |
| `other_accounts` | `set` | Accounts of the other postings in the same transaction. |

<!-- TODO(lead): column details to confirm against the implementation:
  - id: zhang's transaction UUID? (beanquery: a hash of the entry)
  - date: date part in the ledger timezone (zhang transactions can carry a time)
  - narration: NULL or '' when missing? (Beancount always has a narration string)
  - cost_date: NULL when not written, or the transaction date as Beancount fills in?
  - cost_label: NULL when missing? (beanquery returns '' for postings without cost)
  - price: per-unit for `@@` total prices too (total / units)?
  - cost_number for `{{ }}` total cost: per-unit (total / units)?
  - transactions without a flag: what does `flag` return?
  - zhang's rich columns not exposed in Phase 1: posting_flag, filename, lineno, location, balance, meta (as a dict), accounts, type.
-->

## Types

| Type | Description | Example |
|------|-------------|---------|
| `null` | A missing value. | `NULL` |
| `bool` | `TRUE` or `FALSE`. | `TRUE` |
| `int` | A whole number. | `2024` |
| `decimal` | An exact decimal number of any precision. | `12.50` |
| `str` | Text. | `'Expenses:Food'` |
| `date` | A calendar date. | `2024-01-31` |
| `set` | An unordered set of strings, used by `tags`, `links` and `other_accounts`. | `{'trip', 'food'}` |
| `amount` | A decimal number with a currency. | `12.50 USD` |
| `position` | Units (an amount) plus an optional cost lot. The lot has a per-unit cost number and currency, and optionally a date and a label. | `10 VTI {120.00 USD, 2024-01-02, "lot-a"}` |
| `inventory` | A collection of positions in any number of currencies and lots. | `-30.00 USD, 10 VTI {120.00 USD}` |

How positions combine into an inventory:

- `sum(position)` adds every position into one inventory.
- Positions with the same currency and the same cost lot are merged by adding their numbers. Positions with different lots stay separate, so holdings bought at different prices remain distinct.
- A position whose number becomes zero is removed, so an account that nets to zero gives an empty inventory.

An `int` is converted to `decimal` when it is combined with a `decimal`. No other conversions happen implicitly. Use the [valuation functions](#valuation-functions) to move between `position`, `amount` and `inventory`.

### Ordering and comparison

Values of the same type are ordered as follows. This applies to `ORDER BY`, `min`, `max` and the comparison operators.

- `int` and `decimal` by numeric value, `str` by character code (so case matters), `date` chronologically, and `FALSE` before `TRUE`.
- `amount` by currency first and then by number.
- `position` and `inventory` by their positions, in the same way. An inventory that holds a single currency therefore sorts by its number, which is what `ORDER BY total DESC` relies on in the [spending by payee](#spending-by-payee) example.

<!-- TODO(lead): ordering of amount / position / inventory is modelled on Beancount (Amount: (currency, number); Inventory: compare sorted position lists; Position sort key starts with a currency-order table). Confirm what zhang-query implements, and whether these types are allowed in < / > at all. -->

## Functions

`x` stands for an argument of the type named in the signature. Arguments in square brackets are optional. Unless stated otherwise, a function returns `NULL` when an argument is `NULL`.

### Aggregate functions

An aggregate function turns the values of all postings in a group into a single value. See [GROUP BY](#group-by).

| Function | Returns | Description |
|----------|---------|-------------|
| `count(*)` | `int` | Number of postings in the group. |
| `count(x)` | `int` | Number of postings in the group for which `x` is not `NULL`. |
| `sum(x: decimal)` | `decimal` | Sum of the values. `NULL` values are skipped. |
| `sum(x: amount)` | `inventory` | Sum of the amounts, kept separate per currency. |
| `sum(x: position)` | `inventory` | Sum of the positions, with lots merged as described in [Types](#types). |
| `sum(x: inventory)` | `inventory` | Sum of the inventories. |
| `first(x)` | type of `x` | Value of `x` for the first posting of the group, in ledger order. |
| `last(x)` | type of `x` | Value of `x` for the last posting of the group, in ledger order. |
| `min(x)` | type of `x` | Smallest non-`NULL` value. |
| `max(x)` | type of `x` | Largest non-`NULL` value. |

`first` and `last` follow ledger order. `ORDER BY` does not change which posting is first.

<!-- TODO(lead): sum(int) (beanquery: int) and sum over no non-NULL values (beanquery: 0 for decimal, empty inventory) are unspecified. beanquery's first() keeps looking until it sees a non-NULL value, while last() keeps the final value even if it is NULL. -->

### Valuation functions

| Function | Returns | Description |
|----------|---------|-------------|
| `units(x: position)` | `amount` | The units of the position, without the cost. |
| `units(x: inventory)` | `inventory` | The units of every position, without costs. Lots of the same currency merge into one position. |
| `cost(x: position)` | `amount` | Total cost of the position (units times per-unit cost), in the cost currency. A position not held at cost returns its units. |
| `cost(x: inventory)` | `inventory` | `cost` applied to every position, then summed. |
| `convert(x: amount, currency: str[, date: date])` | `amount` | `x` converted into `currency`. |
| `convert(x: position, currency: str[, date: date])` | `amount` | The units of `x` converted into `currency`. The cost is ignored. |
| `convert(x: inventory, currency: str[, date: date])` | `inventory` | Every position converted into `currency`, then summed. |
| `value(x: position[, date: date])` | `amount` | Market value of the position in its cost currency. A position not held at cost returns its units. |
| `value(x: inventory[, date: date])` | `inventory` | `value` applied to every position, then summed. |

How prices are found:

- Prices come from the `price` directives in your ledger.
- With a `date` argument, the function uses the latest price dated on or before that date. Without one, it uses the latest price in the ledger.
- A price can be used in either direction. `price VTI 120 USD` converts VTI into USD at 120, and USD into VTI at 1/120.
- If no price is found, the value is left unchanged. It keeps its original currency and is not dropped or set to zero, so an inventory can still contain several currencies after `convert` or `value`.
- Converting a value into its own currency returns it unchanged.

<!-- TODO(lead): valuation details to confirm:
  - with no date: the latest price overall (beanquery), or the latest on or before today?
  - if both a direct and an inverse price exist, which wins: the direct one, or the more recent one?
  - beanquery's convert(position, ccy) also tries a two-step conversion through the cost/price currency when no direct rate exists. Not documented here.
  - are implied prices from `@` / `{}` postings in the price map, or only `price` directives? In zhang-core today only `price` directives populate Store.prices.
-->

### Account functions

| Function | Returns | Description | Example |
|----------|---------|-------------|---------|
| `root(account: str, n: int)` | `str` | The first `n` components of the account name. If the account has `n` components or fewer, it is returned whole. | `root('Expenses:Food:Dining', 2)` is `'Expenses:Food'` |
| `parent(account: str)` | `str` | The account name without its last component. | `parent('Expenses:Food:Dining')` is `'Expenses:Food'` |
| `leaf(account: str)` | `str` | The last component of the account name. | `leaf('Expenses:Food:Dining')` is `'Dining'` |

<!-- TODO(lead): root(account) with one argument (beanquery: n defaults to 1) and parent() of a top-level account (beanquery: '') are unspecified. -->

### Date functions

| Function | Returns | Description | Example |
|----------|---------|-------------|---------|
| `year(d: date)` | `int` | Year. | `year(2024-05-17)` is `2024` |
| `month(d: date)` | `int` | Month, from 1 to 12. | `month(2024-05-17)` is `5` |
| `quarter(d: date)` | `str` | Year and quarter. | `quarter(2024-05-17)` is `'2024-Q2'` |
| `day(d: date)` | `int` | Day of the month. | `day(2024-05-17)` is `17` |
| `today()` | `date` | The current date. | |

The `year`, `month` and `day` columns are shortcuts: `year` is the same as `year(date)`.

<!-- TODO(lead): quarter() follows beanquery, which returns a 'YYYY-Qn' string. Confirm it is not an int 1-4. Also confirm today()'s timezone (ledger `timezone` option or server local time). -->

### Metadata functions

| Function | Returns | Description |
|----------|---------|-------------|
| `meta(key: str)` | `str` | Value of the metadata `key` on the posting, or `NULL` if it is not set. |
| `entry_meta(key: str)` | `str` | Value of the metadata `key` on the transaction, or `NULL` if it is not set. |

Zhang stores every metadata line inside a transaction on the transaction itself, including lines indented under a posting. Use `entry_meta` to read them.

<!-- TODO(lead): zhang's AST has no per-posting metadata (Posting has no meta; every key-value line in a transaction goes to Transaction.meta). Decide whether meta(key) always returns NULL, falls back to the transaction metadata (like beanquery's any_meta), or is an alias of entry_meta, then update the paragraph above. Also confirm that values are always str (zhang stores metadata values as strings). -->

### Other functions

| Function | Returns | Description | Example |
|----------|---------|-------------|---------|
| `str(x)` | `str` | Text form of any value. | `str(2024-01-31)` is `'2024-01-31'` |
| `length(x: str)` | `int` | Number of characters in the string. | `length('Food')` is `4` |
| `length(x: set)` | `int` | Number of elements in the set. | `length(tags)` |

<!-- TODO(lead): confirm the text form produced by str() for bool (beanquery: 'TRUE'/'FALSE'), decimal (scale kept?), amount ('12.50 USD'?), position and inventory. -->

## HTTP API

The Explore page uses the same HTTP endpoints, and you can call them from scripts. If [basic authentication](/installation/3-basic_auth/) is enabled, send the same credentials as for the rest of the API.

### Run a query

`POST /api/query` with a JSON body:

```shell
curl -X POST http://localhost:8000/api/query \
  -H 'Content-Type: application/json' \
  -d '{"query": "SELECT account, sum(position) WHERE account ~ \"^Assets:Bank\" GROUP BY account"}'
```

A successful response has HTTP status 200:

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

- `columns` lists the result columns in order, each with a `name` and one of the [type](#types) names.
- `rows` is a list of rows. Each row is a list with one cell per column, in the same order.

### Cell encoding

| Type | JSON | Example |
|------|------|---------|
| `null` | `null` | `null` |
| `bool` | boolean | `true` |
| `int` | number | `2024` |
| `decimal` | string | `"1520.35"` |
| `str` | string | `"Expenses:Food"` |
| `date` | string, `YYYY-MM-DD` | `"2024-01-31"` |
| `set` | array of strings | `["trip", "food"]` |
| `amount` | object | `{"number": "12.50", "currency": "USD"}` |
| `position` | object | `{"units": {"number": "10", "currency": "VTI"}, "cost": {"number": "120.00", "currency": "USD", "date": "2024-01-02", "label": null}}` |
| `inventory` | object | `{"positions": [ ...positions... ]}` |

- Decimal numbers, including the `number` fields of amounts and costs, are sent as strings so that no precision is lost. Parse them with a decimal library rather than as floating point.
- In a position, `cost` is `null` when the position is not held at cost. Otherwise it holds the per-unit cost `number` and `currency`, and the lot's `date` and `label`, each of which can be `null`.
- An empty inventory is `{"positions": []}`.
- Do not rely on the order of elements in a `set`.

<!-- TODO(lead): confirm the type name strings used in `columns[].type` (str/bool/int/decimal/date/set/amount/position/inventory/null), the column type of meta()/entry_meta() results, whether int is a JSON number, and whether set elements / inventory positions are sorted. The issue comment says "decimals and amounts are encoded as strings", while this page follows the brief: amount = {number, currency}. -->

### Errors

A query that cannot be parsed or run returns HTTP status 400. For example, `SELECT acount, position` gives:

```json
{
  "message": "unknown column 'acount'",
  "line": 1,
  "column": 8
}
```

- `line` and `column` give the position of the problem in the query text. Both start at 1.
- `column` counts characters, not bytes, so a Chinese character or an accented letter counts as one column.
- Errors include syntax errors, unknown columns or functions, arguments of the wrong type, and invalid `GROUP BY` usage.

<!-- TODO(lead): confirm (a) the exact message text (the example message is illustrative), (b) whether every 400 error carries a line/column or they can be null, (c) "character" = Unicode scalar value (not UTF-16 code unit; an emoji counts once), (d) whether the 400 body is wrapped in {"data": ...} like success responses. -->

### Schema

`GET /api/query/schema` returns the columns of the `postings` table with their types, and the available functions with their signatures. The reference panel on the Explore page is built from this response.

<!-- TODO(lead): document the exact JSON shape of /api/query/schema once the implementation is final. -->

## Examples

### Monthly expenses by category

```sql
SELECT year, month, root(account, 2), sum(position) WHERE account ~ "^Expenses" GROUP BY 1, 2, 3 ORDER BY 1, 2, 3
```

One row per month and second-level expense account (such as `Expenses:Food`), with the total spent. `GROUP BY 1, 2, 3` and `ORDER BY 1, 2, 3` refer to the first three targets by number.

### Spending by payee

```sql
SELECT payee, sum(cost(position)) AS total WHERE account ~ "^Expenses" GROUP BY payee ORDER BY total DESC LIMIT 20
```

The 20 payees you spent the most with. `cost(position)` values purchases made at cost by what you paid for them, and `ORDER BY total DESC` sorts by the alias.

### Postings with a tag

```sql
SELECT date, payee, account, position WHERE 'trip-new-york' IN tags
```

Every posting of every transaction tagged `#trip-new-york`.

### Holdings at cost and market value

```sql
SELECT account, units(sum(position)) AS qty, cost(sum(position)) AS book, convert(units(sum(position)), "USD") AS market WHERE account ~ "^Assets:Trading" GROUP BY account
```

For each trading account: the quantity held, its book value (what it cost), and its market value in USD at the latest prices. Holdings without a USD price stay in their own currency in the `market` column.

### Recent postings

```sql
SELECT date, payee, account, position ORDER BY date DESC LIMIT 20
```

The 20 most recent postings across all accounts.

### Expenses in a date range, excluding taxes

```sql
SELECT account, sum(position) AS total
WHERE account ~ '^Expenses' AND account !~ ':Taxes(:|$)'
  AND date >= 2024-01-01 AND date < 2024-04-01
GROUP BY account
ORDER BY account
```

Uses `!~` to exclude accounts and bare date literals for the range.

### Quarterly totals with a count

```sql
SELECT quarter(date) AS q, count(*) AS postings, sum(number) AS total, sum(number) / count(*) AS average
WHERE account ~ '^Expenses:Food' AND currency = 'USD'
GROUP BY q
ORDER BY q
```

Postings, total and average amount per quarter. Filtering on `currency` first makes `sum(number)` meaningful, because `number` ignores the currency.

### Transactions paid by credit card

```sql
SELECT DISTINCT date, description
WHERE 'Liabilities:CreditCard' IN other_accounts AND account ~ '^Expenses'
ORDER BY date DESC
```

`other_accounts` holds the accounts of the other postings in each transaction, so this finds expenses paid from the card. `DISTINCT` lists each transaction once even if it has several expense postings.

### Postings without a payee

```sql
SELECT date, narration, account, position
WHERE payee IS NULL AND account IN ('Expenses:Misc', 'Expenses:Uncategorized')
ORDER BY date DESC
```

Uses `IS NULL` and a list after `IN`.

### Invoices recorded in metadata

```sql
SELECT date, payee, entry_meta('invoice') AS invoice, position
WHERE entry_meta('invoice') IS NOT NULL AND leaf(account) = 'Consulting'
ORDER BY date
```

Lists postings to accounts named `...:Consulting` whose transaction has an `invoice` metadata entry.

### Portfolio value at the end of a year

```sql
SELECT account, value(sum(position), 2024-12-31) AS market_value
WHERE account ~ '^Assets:Investments' AND date <= 2024-12-31
GROUP BY account
ORDER BY account
```

Holdings as of 31 December 2024, valued at the prices in effect on that date, in each holding's cost currency.

## Differences from BQL and beanquery

### Not available yet

- **Statements other than `SELECT`:** `BALANCES`, `JOURNAL` and `PRINT` are not supported.
- **Period clauses in `FROM`:** `OPEN ON`, `CLOSE ON` and `CLEAR` are not supported, and neither are table names or subqueries after `FROM`.
- **`HAVING` and `PIVOT BY`.**
- **Other tables:** only `postings` exists. There are no entries, prices, accounts, commodities, documents or balances tables.
- **The running `balance` column.**
- **The `query` directive:** queries saved in the ledger (`2024-01-01 query "name" "SELECT ..."`) are not listed or run yet.
- **Functions and operators not listed on this page**, such as `getprice`, `abs`, `round`, `any_meta`, `has_account`, the case-sensitive match `?~`, `BETWEEN` and `%`. Using one is an error.

The roadmap in [#434](https://github.com/zhang-accounting/zhang/issues/434) schedules `BALANCES`, `JOURNAL`, `OPEN`/`CLOSE`/`CLEAR`, the `query` directive and CSV export for the next phase, and `HAVING`, `PIVOT BY` and more tables after that.

### Behaving differently

- **Errors carry a position.** Every query error reports the line and column where it was found.
- **`SELECT *` includes `account`.** beanquery expands `*` to `date, flag, payee, narration, position`. Zhang adds `account` before `position`.
- **Metadata belongs to the transaction.** Zhang has no separate posting metadata, so use `entry_meta`. See [Metadata functions](#metadata-functions).
- **Exact decimals throughout.** Numbers are arbitrary-precision decimals, and amounts are never stored with a fixed number of decimal places.

<!-- TODO(lead): the SELECT * difference follows the brief (date, flag, payee, narration, account, position). Both BQL v2 and beanquery use `date flag payee narration position` (no account). Keep the difference or align with beanquery? -->

### Workarounds

Until `OPEN ON` and `CLOSE ON` arrive, filter on `date` instead.

An income statement for 2024:

```sql
SELECT account, sum(position)
WHERE account ~ '^(Income|Expenses)' AND date >= 2024-01-01 AND date < 2025-01-01
GROUP BY 1
ORDER BY 1
```

Balances of assets and liabilities at the start of 1 April 2025:

```sql
SELECT account, sum(position)
WHERE account ~ '^(Assets|Liabilities)' AND date < 2025-04-01
GROUP BY account
ORDER BY account
```

Unlike `CLOSE ON ... CLEAR`, these queries do not move income and expenses into equity, so they are only equivalent for the accounts they select.
