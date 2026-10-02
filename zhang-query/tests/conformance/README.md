# BQL conformance fixtures (beanquery oracle)

This directory holds an independent conformance suite for zhang's native
BQL-compatible query engine (issue #434, Phases 1 and 2). The expected results
were produced by the official Python **beanquery**, not by zhang, so they are
the reference the engine is cross-validated against.

- `cases/NNN_<name>.json`: one fixture per query (100 cases: 001–060 for
  Phase 1, 061–100 for Phase 2).
- `generate.py`: the generator. It holds the case list and writes the fixtures.
- Oracle versions used: **beancount 3.2.3, beanquery 0.2.0** (Python 3.9).

## Shared ledger

All cases run against one ledger:

```
integration-tests/fava-demo-ledger/main.zhang
```

This is the Fava/Beancount example ledger already in the repository (2015-01-01
to 2017-09-12). It was chosen over a fresh copy of beancount's
`examples/example.beancount` because it is already in the repo, it is the
ledger the engine's golden tests target, and both tools load it cleanly:

| Tool | Command | Result |
|---|---|---|
| beancount / beanquery | `loader.load_file("integration-tests/fava-demo-ledger/main.zhang")` | 0 errors; 1044 transactions, 3209 postings, 846 prices |
| zhang | `zhang serve integration-tests/fava-demo-ledger --endpoint main.zhang --port 18970 --no-report`, then `curl 'localhost:18970/api/errors?page=1&size=10'` | `{"total_count":0,…,"records":[]}` |

The two tools agree on the ledger. Every one of beancount's 1044 transactions
has an identical counterpart in zhang's Store (same date, payee, narration,
tags, links, and posting accounts, units and cost numbers), and both have the
same 846 prices. There are two known differences, and both matter for the engine:

1. **Synthetic balance-check transactions.** zhang's Store has 82 extra
   `BalanceCheck` transactions, one for each `balance` directive (payee
   `"Balance Check"`, one zero-amount posting on `Assets:US:BofA:Checking`,
   `Liabilities:US:Chase:Slate` or `Assets:US:Federal:PreTax401k`).
   beancount's `postings` table contains real transactions only. The engine
   should exclude these rows, or results such as `count(*)` over those accounts
   will differ. The generator checks that no `engine` case is sensitive to them
   (see below).
2. **Fields missing from the Store posting.** `PostingDomain` has no `@` price,
   no cost date, no cost label and no posting metadata. Cases that use them are
   marked `ledger-dependent`.

The ledger has no `pad` directives, no links, no transaction or posting
metadata, no `!` flags, no cost labels and no implicit (amount-less) postings.
It has 4 tags (`trip-*`) and 5 postings with an `@` price. Features that depend
on the missing data are therefore only partly covered (see "Not covered").

## Regenerating

```sh
python3 -m venv /tmp/bqvenv
/tmp/bqvenv/bin/pip install beancount==3.2.3 beanquery==0.2.0
cd zhang-query/tests/conformance
/tmp/bqvenv/bin/python generate.py                 # rewrite cases/*.json (removes stale files)
/tmp/bqvenv/bin/python generate.py --check         # fail if cases/*.json are out of date
/tmp/bqvenv/bin/python generate.py --check --table # also print the case table below
```

`generate.py [LEDGER]` defaults to the shared ledger. It uses only the
standard library plus beancount and beanquery. To add a case, append a
`case(...)` entry to `CASES`, or `case2(...)` for a Phase 2 case. Fixtures are
numbered by their position in the list, so append new cases at the end.

Before writing anything, the generator checks every case:

- **Determinism.** It re-runs the query over a perturbed ledger, with entries
  reversed within each date and postings reversed within each transaction. An
  ordered case must return the same sequence and an unordered case the same
  multiset. This rejects results that depend on tie-breaking, such as
  `ORDER BY date` with several rows on one day, `LIMIT` cut-offs inside a tie,
  or `first()`/`last()` over rows on the same day.
- **Classification.** It re-runs the query over the ledger plus one synthetic
  zero-amount transaction per `balance` directive, which mimics zhang's Store.
  An `engine` case that changes is rejected. A `ledger-dependent` case that
  changes gets an automatic note.
- **Shape.** A query with `LIMIT` must be `ordered`, and each fixture has at
  most 200 rows.
- **CSV rounding.** For a csv case, beanquery's numberify step rounds numbers
  to the ledger's display precision (inferred by beancount per currency). A
  case whose cells change under that rounding is rejected, so no fixture
  depends on it.

Determinism matters most for Phase 2: the running `balance` column and
`JOURNAL` depend on the order of rows within a day, so those cases select
windows with at most one selected posting per day.

## Fixture format

```json
{
  "name": "sum_position_by_root_account",
  "query": "SELECT ...",
  "kind": "engine",
  "ordered": true,
  "expect": "rows",
  "columns": [{"name": "root", "type": "str"}, {"name": "balance", "type": "inventory"}],
  "rows": [["Equity", {"positions": [...]}]],
  "notes": "free text, may be empty"
}
```

A Phase 2 fixture adds `"phase": 2` after `query`. A csv fixture has
`"expect": "csv"`, empty `columns` and `rows`, and a `csv` field:

```json
{
  "name": "csv_inventory_sums",
  "query": "SELECT account, sum(position) AS balance WHERE ... GROUP BY account ORDER BY account",
  "phase": 2,
  "kind": "engine",
  "ordered": true,
  "expect": "csv",
  "columns": [],
  "rows": [],
  "csv": [
    "account,balance (VACHR),balance (USD)",
    "Assets:US:Hoogle:Vacation, -13,",
    "Liabilities:US:Chase:Slate,,-2703.29"
  ],
  "notes": "..."
}
```

- `kind`:
  - `engine`: pure query semantics over postings that both tools agree on.
    These cases must pass.
  - `ledger-dependent`: the result also depends on booking (lot matching, cost
    dates), the price map, or data the Store does not keep (`@` price). These
    cases must pass too. A failure is triaged first, because it may be a
    ledger-processing difference rather than an engine bug. It is tolerated
    only through an explicit allow-list entry with a reason (see "Running the
    harness").
- `phase`: absent for the Phase 1 fixtures, `2` for the Phase 2 ones. The
  harness keeps Phase 2 fixtures non-fatal until the Phase 2 features land
  (see "Running the harness").
- `ordered`: `true` only when the row order is fully determined: by the
  query's `ORDER BY`, by `BALANCES` (which sorts by account), or by ledger
  order for a `JOURNAL` whose rows all have different dates.
- `expect`: `"rows"`, `"error"` or `"csv"`. Error and csv fixtures have
  `"columns": []` and `"rows": []`. A csv fixture holds the expected CSV in
  `csv` (see "CSV fixtures"). An error fixture has an `error_class` taken
  from the type of beanquery's exception:
  - `"syntax"` for a `ParseError`.
  - `"compile"` for a `CompilationError`: unknown column or function, type
    mismatches and grouping errors.

  The generator refuses any other exception, because beanquery crashing is
  not an expectation. Messages and positions are not compared.
- Every fixture contains `notes`. It is `""` when there is nothing to note.
- Files are UTF-8 JSON with one row per line. `query` is exactly what zhang
  should run.

### Type mapping (beanquery datatype → fixture `type`)

| beanquery datatype | fixture type |
|---|---|
| `NoneType` (the `NULL` literal) | `null` |
| `bool` | `bool` |
| `int` | `int` |
| `Decimal` | `decimal` |
| `str` | `str` |
| `datetime.date` | `date` |
| `set`, `frozenset`, `list`, `typing.Set[str]` (`tags`, `links`, `other_accounts`) | `set` |
| `Amount` | `amount` |
| `Position` | `position` |
| `Inventory` | `inventory` |
| `object` (`meta()` / `entry_meta()`, dynamically typed) | `str` (zhang metadata values are strings) |

### Cell encoding

This is the same as the HTTP API.

| type | JSON |
|---|---|
| any NULL | `null` |
| bool | `true` / `false` |
| int | number |
| decimal | string, beanquery's digits without exponent notation (`"4.00"`, `"3.5"`) |
| str | string |
| date | `"YYYY-MM-DD"` |
| set | array of strings, sorted |
| amount | `{"number": "<decimal>", "currency": "USD"}` |
| position | `{"units": <amount>, "cost": null \| {"number": "<decimal>", "currency": "USD", "date": "YYYY-MM-DD" \| null, "label": <string> \| null}}` |
| inventory | `{"positions": [<position>, ...]}`, sorted by `units.currency`, then positions without cost first, then `cost.number`, `cost.currency`, `cost.date`, `cost.label`. An empty inventory is `{"positions": []}`. |

## CSV fixtures

A csv fixture holds beanquery's CSV output for the query, produced like
`bean-query -m -f csv` (numberify, then the CSV writer). Every csv fixture is
byte-identical to that command's output. The `csv` field is that output split
into lines: beanquery ends each line with CRLF, and no cell contains a line
break.

numberify turns each amount, position and inventory column into one decimal
column per currency:

- The column is named `<name> (<currency>)`, for example `balance (USD)`.
  Other columns keep their name.
- The currency columns are ordered by the number of rows that hold the
  currency, most first. Ties are ordered by currency name, descending, so
  `VHT, VEA, ITOT, GLD` (case 090) and `VACHR` before `IRAUSD` (case 091).
- An amount or position cell is its number when its currency matches the
  column, and empty otherwise. The cost of a position is dropped, so only the
  units remain.
- An inventory cell is the sum of the units of that currency over all lots.
  It is empty when the currency is absent or sums to zero, so an empty
  inventory is a row of empty cells.
- A column with no currency at all, for example an amount column that is NULL
  on every row, produces no CSV column (case 093).
- numberify rounds numbers to the display precision of their currency. The
  generator rejects cases where this changes a value.

The CSV writer then renders the cells:

- NULL and the empty string are both an empty cell.
- A set is its elements, sorted and joined by `,`. The writer quotes cells
  that contain `,`.
- Booleans are `TRUE` and `FALSE`, dates are `YYYY-MM-DD`.
- Decimals are padded with spaces to align on the decimal point (`  91.45`).

`csv` fixtures alias every computed column, so the header names do not depend
on how an engine names unaliased expressions.

## Comparison rules for the harness

1. **Columns:** compare the count and each column's `type` by position.
   Column `name`s come from beanquery (e.g. `sum(position)`, `count(*)`, or the
   `AS` alias) and are advisory only, since zhang may name unaliased
   expressions differently.
2. **Rows:** if `ordered` is true, compare the sequences; otherwise compare
   multisets of rows.
3. **Decimals** (decimal cells, amount/position/cost numbers) are compared
   numerically, so `"4.00"` equals `"4"` and trailing zeros don't matter.
4. **Inventories** are compared as multisets of positions. A position is equal
   when units (numerically), currency and cost (number numerically, currency,
   date, label) are equal. Positions with zero units should not appear.
5. **Sets** are compared as sets (fixtures are already sorted).
6. **NULL** matches only NULL. `""` is not NULL. beanquery returns `''` in some
   places, see the quirks below.
7. **Errors:** for `expect: "error"`, the engine must reject the query with an
   error of the fixture's `error_class`. The harness maps zhang's
   `QueryErrorKind` as follows: `Parse` to `syntax`, `Compile` to `compile`,
   and `Eval` to `runtime`. No fixture expects `runtime`.
8. **CSV:** for `expect: "csv"`, the engine's result goes through zhang's
   CSV export (`zhang_query::export::to_csv`). Both CSV texts are parsed
   (`,` separators, `"` quoting with `""` escapes, CRLF or LF line ends).
   Each cell is trimmed. Cells that are plain numbers (`-?digits[.digits]`)
   are compared numerically, and every other cell is compared as exact text.
   The header row must match cell by cell, including the ` (<currency>)`
   suffixes and the column order. The data rows are then compared like
   typed rows: as a sequence when `ordered`, otherwise as a multiset.

## Running the harness

`zhang-query/tests/conformance.rs` runs every fixture against the engine on the
shared ledger and applies the rules above:

```sh
cargo test -p zhang-query --test conformance
```

It prints a table to stderr with one status per case:

- `PASS`: the engine matches the oracle.
- `ACCEPTED`: the engine differs from the oracle exactly as documented in
  `ACCEPTED_DEVIATIONS` in that file. Most entries give the exact rows the
  engine must return instead.
- `LEDGER-DEP`: a `ledger-dependent` case differs and is listed, with a
  reason, in `LEDGER_DEPENDENT_ALLOWED`. The list is empty today.
- `PENDING-PHASE2`: a `"phase": 2` fixture while the temporary gate
  `PHASE2_FEATURES_LANDED` in the harness is `false`. The case still runs,
  and the detail column shows the status it would get (`would PASS`,
  `would FAIL: ...`). A summary line counts the pending cases that would
  fail.
- `FAIL`: any other difference. This includes unlisted ledger-dependent
  cases, an error of the wrong class, and functions the engine lacks.

Only `FAIL` makes the test fail. Phase 1 fixtures are always strict. An
allow-list entry for a case that passes is reported as a stale entry.

During the Phase 2 integration, set `PHASE2_FEATURES_LANDED` to `true` and
replace the body of `engine_csv()` with
`Some(zhang_query::export::to_csv(result))`. Until then, csv cases report the
export as missing. A second test, `csv_comparison_rules`, checks the CSV
comparison itself on the csv fixtures: an equivalent rewrite (trimmed,
normalized, fully quoted, LF line ends) must compare equal, and a changed
number or header must not.

To check that the gate catches regressions, run it over a mutated copy of the
fixtures:

```sh
ZHANG_QUERY_CONFORMANCE_CASES=/tmp/mutated-cases cargo test -p zhang-query --test conformance
```

## beanquery semantics captured by the fixtures

These come straight from the oracle. Some of them differ from SQL or from the
Phase 1 decisions, and are worth deciding on explicitly.

- `~` and `!~` are case-insensitive `re.search`: a partial match unless
  anchored (cases 010, 011). This matches the Phase 1 decision. beanquery's
  `?~` operator, by contrast, is case-sensitive, and its operands are reversed:
  the pattern is on the left.
- **`SELECT *`** in beanquery 0.2.0 expands to `date, flag, payee, narration,
  position`, with no `account`. Case 002 deliberately follows the Phase 1 spec
  (`… narration, account, position`) and was generated from the equivalent
  explicit column list (see its `notes`).
- **Implicit GROUP BY:** aggregates mixed with plain columns and no `GROUP BY`
  are accepted and grouped by the non-aggregate targets (035). With an explicit
  `GROUP BY`, every non-aggregate target must be covered, otherwise it is an
  error (057). `GROUP BY` may use an expression that is not selected (036).
- An aggregate query that matches **no rows returns zero rows**, not one row
  with `0`/NULL (031).
- `sum(amount)` and `sum(position)` produce an `Inventory`; `sum(int)` stays
  `int`; `sum(decimal)` stays `decimal`. `count(x)` counts non-NULL values.
- NULL logic: `TRUE AND NULL` = NULL, `FALSE AND NULL` = FALSE, `TRUE OR NULL`
  = TRUE, `NULL OR FALSE` = NULL (021). There are two quirks that differ from
  SQL (022, 023):
  - `NOT NULL` is **TRUE**, so `NOT (payee = 'x')` keeps NULL payees, while
    `payee != 'x'` drops them.
  - `NULL AND FALSE` is **NULL**, because AND returns at the first NULL.
- NULL sorts **first** with `ASC` and **last** with `DESC` (024, 025). Each
  `ORDER BY` key has its own direction (040).
- `DISTINCT` is applied before `LIMIT` (042).
- `FROM <expr>` is a per-posting row filter combined with `WHERE` by AND. It
  can use posting columns such as `account` (043, 044).
- `/` always yields a decimal (`7 / 2` = `3.5`). Division by zero yields NULL.
  Other int-with-int arithmetic stays int (027). Unary minus works on
  int/decimal only, and `-position` is a compile error in beanquery.
- `x IN (a, b)` takes a parenthesized list. A one-element `(a)` is just a
  parenthesized scalar, and beanquery then fails at runtime. Write `(a,)`, or
  avoid it. Int and decimal values compare numerically in `=` and `IN` (014,
  018).
- `cost_label` is `''`, not NULL, for postings without a cost. It is NULL for
  a cost without a label (008, 009).
- `parent()` of a top-level account is `''`, not NULL. `root(a, n)` with `n`
  past the depth returns `a` (045).
- `description` joins payee and narration with `' | '` and skips NULL or
  empty parts (003, 005).
- `quarter(date)` returns `'YYYY-Qn'`. `str(TRUE)` returns `'TRUE'`.
  `str(decimal)` keeps the exponent (`'4.00'`) (046, 047).
- Without a cost, `cost(position)` and `value(position)` return the units.
  `convert()` without an applicable price returns the amount unchanged (049,
  053).
- `ORDER BY` an inventory uses beancount's position sort key. For
  single-currency inventories that is the units number (039).

### Phase 2

- **`BALANCES [AT f] [FROM …] [WHERE …]`** is `SELECT account,
  sum(f(position)) GROUP BY account ORDER BY account_sortkey(account)`. The
  order is by account type (Assets, Liabilities, Equity, Income, Expenses),
  then by name, so it is not alphabetical (061, 062). Every account with
  postings is listed, even when its balance is the empty inventory. `AT units`
  merges lots and `AT cost` gives book values with exact products, not
  rounded (063, 064). `AT f` with an unknown `f` is a compile error (100).
  beanquery names the column `SUM((position))` (names are advisory).
- **`JOURNAL 'regex' [AT f] [FROM …]`** is `SELECT date, flag,
  maxwidth(payee, 48), maxwidth(narration, 80), account, f(position),
  f(balance) WHERE account ~ "regex"`, with no `ORDER BY`, so rows come in
  ledger order (067–071). There is no `WHERE`, `ORDER BY` or `LIMIT` (095);
  use `FROM <expr>` to filter. The pattern must be a string literal (094).
  It uses the case-insensitive partial-match `~`. `AT cost` gives an amount
  column and an inventory column (070).
- **`maxwidth(s, n)`** is Python's `textwrap.shorten(s, width=n)`. It
  collapses runs of whitespace and strips leading and trailing whitespace,
  so the ledger's `'Eating out '` is returned as `'Eating out'` (069). Text
  still longer than `n` is cut at a word boundary and gets `' [...]'`.
  Nothing in this ledger is long enough to be cut: the longest payee has 25
  characters and the longest narration 48. NULL stays NULL.
- **The running `balance` column** is the running sum (an inventory) of
  `position` over the rows that pass `FROM` and `WHERE`, in ledger order. It
  starts empty, so it ignores postings outside the filter (072, 068). It is
  computed before `ORDER BY` and `LIMIT`, so with `ORDER BY date DESC` the
  first row carries the final balance (073). It is one sum over all selected
  rows, not one per account (074, 068). It keeps lots (075). Rows on the same
  day follow source order, so an engine whose same-day order differs gets
  different intermediate balances. The fixtures avoid such days.
- **`FROM [expr] [OPEN ON d] [CLOSE [ON d]] [CLEAR]`.** The modifiers take
  bare date literals and have this fixed order (096–098). `CLOSE` before
  `OPEN` is a compile error, and equal dates are allowed (099). The
  modifiers rewrite the whole ledger first, then the expression filters the
  postings (088). The synthetic transactions all have a NULL payee and use
  beancount's default equity accounts:
  - **`OPEN ON d`** inserts a conversions transaction dated `d - 1` on
    `Equity:Conversions:Previous` and transfers every income and expense
    balance before `d` to `Equity:Earnings:Previous`. It then replaces all
    transactions before `d` by one transaction per account, with flag **`S`**,
    dated `d - 1` and narrated `Opening balance for '<account>'
    (Summarization)`. That transaction has one posting per position (lot) of
    the balance and, for each, a counterpart at cost on
    `Equity:Opening-Balances`. Income and expense accounts get no `S` rows,
    so they start the period at zero (076–078).
  - **`CLOSE ON d`** drops everything dated `d` or later, so it is exclusive
    (080). It then adds a conversions transaction with flag **`C`**, dated
    `d - 1`, on `Equity:Conversions:Current`. That transaction holds minus
    the cost-basis total of all remaining postings (`cost(position)`, or the
    units without a cost). On this ledger that is only a rounding residual
    of the lot purchases, such as `-0.01346 USD` (065, 079). Its narration
    `Conversion for (<inventory>)` uses beancount's inventory formatting and
    is not covered.
  - **`CLOSE` without a date** truncates nothing. It only appends that
    conversions transaction, dated like the last entry of the ledger (081).
    Here that is 2017-09-08, which is both the last transaction and the last
    price.
  - **`CLEAR`** transfers each income and expense balance to
    `Equity:Earnings:Current`, using flag **`T`** and the narration `Transfer
    balance for '<account>' (Transfer balance)`. It adds one posting per
    position plus a counterpart (082–084, 087). The transfers are dated like
    the last remaining entry. After `CLOSE ON d` that is the conversions
    transaction on `d - 1`, as in the balance sheet at 2017-01-01 (085).
  - The counterpart postings of an `S` or `T` transaction carry the
    narration of the account they balance (087). Case 086 counts every
    synthetic flag for `OPEN ON 2016-01-01 CLOSE ON 2017-01-01 CLEAR`:
    `S` 90, `C` 1, `T` 64.
  - zhang stores balance assertions as transactions with flag `C`, which is
    also beancount's conversions flag. They are not rows of the postings
    table, but an implementation that uses flags to tell them apart must not
    confuse the two.

## Not covered

These are deliberately out of scope or not exercisable on this ledger:

- `today()`, because it is not deterministic.
- Non-NULL `meta()`/`entry_meta()` values, links, `!` flags and cost labels.
  The ledger has none, and beancount's automatic `filename`/`lineno` metadata
  has no zhang equivalent.
- Division that doesn't terminate (`1 / 3`). beanquery uses Python's 28-digit
  decimal context, and zhang's precision is a separate decision.
- `str()` of amounts, positions and inventories, and ordering by
  multi-currency inventories. Both depend on beancount-specific formatting and
  sort order.
- The narration of the `CLOSE` conversions transaction, and `maxwidth()`
  truncation (no text in the ledger is long enough).
- CSV output without numberify (`bean-query -f csv`). It renders amounts and
  inventories as padded text and drops cost dates, so it is not a useful
  target.
- Phase 3 features: `HAVING`, `PIVOT BY`, and other tables.

## Cases

100 cases:

- Phase 1 (001–060), 60 cases: 47 `engine` with rows, 6 `ledger-dependent`, and 7 errors (`engine`).
- Phase 2 (061–100), 40 cases: 13 `engine` and 15 `ledger-dependent` with rows, 3 `engine` and 2
  `ledger-dependent` csv cases, and 7 errors (`engine`). By area: `balances` 6, `journal` 5,
  `balance-column` 4, `period` 13, `csv` 5, `error` 7.

| # | Case | Phase | Area | Kind | Ordered | Expect |
|---|---|---|---|---|---|---|
| 001 | `literals` | 1 | select | engine | no | 1 rows |
| 002 | `select_star` | 1 | select | engine | yes | 12 rows |
| 003 | `entry_columns` | 1 | select | engine | yes | 12 rows |
| 004 | `posting_columns` | 1 | select | engine | yes | 19 rows |
| 005 | `description_without_payee` | 1 | select | engine | no | 7 rows |
| 006 | `tags_links_sets` | 1 | select | engine | no | 4 rows |
| 007 | `id_groups_postings_by_transaction` | 1 | select | engine | no | 24 rows |
| 008 | `cost_columns_without_cost` | 1 | select | engine | no | 1 rows |
| 009 | `cost_columns_on_lots` | 1 | select | ledger-dependent | yes | 8 rows |
| 010 | `regex_case_insensitive` | 1 | where | engine | yes | 2 rows |
| 011 | `regex_not_match` | 1 | where | engine | yes | 2 rows |
| 012 | `tag_filter` | 1 | where | engine | yes | 42 rows |
| 013 | `in_string_list_and_not_in_set` | 1 | where | engine | yes | 2 rows |
| 014 | `in_numeric_list` | 1 | where | engine | yes | 2 rows |
| 015 | `other_accounts_membership` | 1 | where | engine | yes | 6 rows |
| 016 | `boolean_precedence` | 1 | where | engine | yes | 4 rows |
| 017 | `date_range_bare_literals` | 1 | where | engine | yes | 4 rows |
| 018 | `decimal_comparison` | 1 | where | engine | yes | 5 rows |
| 019 | `string_comparison` | 1 | where | engine | yes | 4 rows |
| 020 | `is_null_is_not_null` | 1 | null | engine | yes | 2 rows |
| 021 | `null_logic_standard` | 1 | null | engine | no | 1 rows |
| 022 | `null_logic_beanquery_quirks` | 1 | null | engine | no | 1 rows |
| 023 | `not_vs_not_equal_on_null` | 1 | null | engine | yes | 2 rows |
| 024 | `order_by_nulls_first_asc` | 1 | null | engine | yes | 2 rows |
| 025 | `order_by_nulls_last_desc` | 1 | null | engine | yes | 2 rows |
| 026 | `decimal_arithmetic_on_columns` | 1 | arithmetic | engine | yes | 9 rows |
| 027 | `literal_arithmetic_and_precedence` | 1 | arithmetic | engine | yes | 3 rows |
| 028 | `count_variants` | 1 | aggregate | engine | no | 1 rows |
| 029 | `sum_over_int_decimal_amount_position` | 1 | aggregate | engine | no | 1 rows |
| 030 | `min_max_first_last_grouped` | 1 | aggregate | engine | yes | 4 rows |
| 031 | `aggregate_over_no_rows` | 1 | aggregate | engine | no | 0 rows |
| 032 | `sum_position_empty_inventory` | 1 | aggregate | engine | yes | 2 rows |
| 033 | `sum_position_by_root_account` | 1 | aggregate | engine | yes | 4 rows |
| 034 | `sum_position_with_lots` | 1 | aggregate | ledger-dependent | yes | 2 rows |
| 035 | `implicit_group_by` | 1 | group-order | engine | no | 4 rows |
| 036 | `group_by_unselected_expression` | 1 | group-order | engine | no | 3 rows |
| 037 | `group_by_index_order_by_index_desc` | 1 | group-order | engine | yes | 6 rows |
| 038 | `monthly_expenses_by_category` | 1 | group-order | engine | yes | 73 rows |
| 039 | `payee_totals_top_n` | 1 | group-order | engine | yes | 10 rows |
| 040 | `order_by_mixed_directions` | 1 | group-order | engine | yes | 4 rows |
| 041 | `order_by_unselected_expression` | 1 | group-order | engine | yes | 8 rows |
| 042 | `distinct_order_limit` | 1 | group-order | engine | yes | 5 rows |
| 043 | `from_expression_filter` | 1 | from | engine | yes | 4 rows |
| 044 | `from_posting_level_expression` | 1 | from | engine | yes | 4 rows |
| 045 | `account_functions` | 1 | function | engine | yes | 2 rows |
| 046 | `date_functions` | 1 | function | engine | yes | 12 rows |
| 047 | `str_and_length` | 1 | function | engine | yes | 12 rows |
| 048 | `meta_missing_key` | 1 | function | engine | no | 1 rows |
| 049 | `units_cost_value_without_cost` | 1 | function | engine | yes | 9 rows |
| 050 | `holdings_cost_and_market` | 1 | valuation | ledger-dependent | yes | 4 rows |
| 051 | `convert_at_date` | 1 | valuation | ledger-dependent | yes | 5 rows |
| 052 | `value_latest_and_at_date` | 1 | valuation | ledger-dependent | yes | 2 rows |
| 053 | `convert_without_price_keeps_units` | 1 | valuation | ledger-dependent | yes | 2 rows |
| 054 | `error_syntax` | 1 | error | engine | no | error |
| 055 | `error_unknown_column` | 1 | error | engine | no | error |
| 056 | `error_unknown_function` | 1 | error | engine | no | error |
| 057 | `error_ungrouped_column` | 1 | error | engine | no | error |
| 058 | `error_mixed_aggregate_in_expression` | 1 | error | engine | no | error |
| 059 | `error_aggregate_in_where` | 1 | error | engine | no | error |
| 060 | `error_type_mismatch` | 1 | error | engine | no | error |
| 061 | `balances_plain` | 2 | balances | ledger-dependent | yes | 58 rows |
| 062 | `balances_where_account_type_order` | 2 | balances | engine | yes | 6 rows |
| 063 | `balances_at_units` | 2 | balances | engine | yes | 11 rows |
| 064 | `balances_at_cost` | 2 | balances | ledger-dependent | yes | 8 rows |
| 065 | `balances_from_close_on` | 2 | balances | ledger-dependent | yes | 32 rows |
| 066 | `balances_at_cost_open_close_where` | 2 | balances | ledger-dependent | yes | 9 rows |
| 067 | `journal_account_regex` | 2 | journal | engine | yes | 13 rows |
| 068 | `journal_several_accounts_and_currencies` | 2 | journal | engine | yes | 9 rows |
| 069 | `journal_maxwidth_trims_narration` | 2 | journal | engine | yes | 10 rows |
| 070 | `journal_at_cost` | 2 | journal | ledger-dependent | yes | 8 rows |
| 071 | `journal_open_close_summary_row` | 2 | journal | ledger-dependent | yes | 10 rows |
| 072 | `balance_running_total` | 2 | balance-column | ledger-dependent | yes | 15 rows |
| 073 | `balance_computed_before_order_by` | 2 | balance-column | ledger-dependent | yes | 5 rows |
| 074 | `balance_spans_all_selected_accounts` | 2 | balance-column | engine | yes | 6 rows |
| 075 | `balance_with_lots` | 2 | balance-column | ledger-dependent | yes | 9 rows |
| 076 | `open_on_summarization_rows` | 2 | period | ledger-dependent | yes | 12 rows |
| 077 | `open_on_only_summaries_before_date` | 2 | period | ledger-dependent | no | 1 rows |
| 078 | `income_statement_2016` | 2 | period | engine | no | 32 rows |
| 079 | `close_on_conversion_entry` | 2 | period | ledger-dependent | yes | 1 rows |
| 080 | `close_on_truncates_before_date` | 2 | period | engine | no | 1 rows |
| 081 | `close_without_date` | 2 | period | ledger-dependent | yes | 1 rows |
| 082 | `clear_transfers` | 2 | period | engine | yes | 3 rows |
| 083 | `clear_zeroes_income_statement` | 2 | period | engine | yes | 3 rows |
| 084 | `clear_in_journal` | 2 | period | engine | yes | 14 rows |
| 085 | `balance_sheet_2017_with_clear` | 2 | period | ledger-dependent | yes | 16 rows |
| 086 | `open_close_clear_flags` | 2 | period | ledger-dependent | yes | 4 rows |
| 087 | `summarization_narrations` | 2 | period | engine | yes | 4 rows |
| 088 | `from_expression_with_open_close` | 2 | period | engine | yes | 2 rows |
| 089 | `csv_inventory_sums` | 2 | csv | engine | yes | csv, 5 rows |
| 090 | `csv_holdings_with_cost` | 2 | csv | ledger-dependent | yes | csv, 4 rows |
| 091 | `csv_amount_columns_mixed_currencies` | 2 | csv | engine | yes | csv, 18 rows |
| 092 | `csv_positions_cost_and_price` | 2 | csv | ledger-dependent | yes | csv, 8 rows |
| 093 | `csv_sets_nulls_bools` | 2 | csv | engine | yes | csv, 10 rows |
| 094 | `error_journal_non_string` | 2 | error | engine | no | error |
| 095 | `error_journal_where_clause` | 2 | error | engine | no | error |
| 096 | `error_open_on_non_date` | 2 | error | engine | no | error |
| 097 | `error_open_without_on` | 2 | error | engine | no | error |
| 098 | `error_clear_before_close` | 2 | error | engine | no | error |
| 099 | `error_close_date_before_open_date` | 2 | error | engine | no | error |
| 100 | `error_balances_unknown_summary_function` | 2 | error | engine | no | error |
