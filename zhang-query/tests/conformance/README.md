# BQL conformance fixtures (beanquery oracle)

This directory holds an independent conformance suite for zhang's native
BQL-compatible query engine (issue #434, Phase 1). The expected results were
produced by the official Python **beanquery**, not by zhang, so they are the
reference the engine is cross-validated against.

- `cases/NNN_<name>.json`: one fixture per query (60 cases).
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
`case(...)` entry to `CASES`. Fixtures are numbered by their position in the
list.

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

- `kind`:
  - `engine`: pure query semantics over postings that both tools agree on.
    These cases must pass.
  - `ledger-dependent`: the result also depends on booking (lot matching, cost
    dates), the price map, or data the Store does not keep (`@` price). These
    cases must pass too. A failure is triaged first, because it may be a
    ledger-processing difference rather than an engine bug. It is tolerated
    only through an explicit allow-list entry with a reason (see "Running the
    harness").
- `ordered`: `true` only when the query's `ORDER BY` fully determines the row
  order.
- `expect`: `"rows"` or `"error"`. Error fixtures have `"columns": []` and
  `"rows": []`, plus an `error_class` taken from the type of beanquery's
  exception:
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
- `FAIL`: any other difference. This includes unlisted ledger-dependent
  cases, an error of the wrong class, and functions the engine lacks.

Only `FAIL` makes the test fail. An allow-list entry for a case that passes is
reported as a stale entry.

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
- Phase 2/3 features: `FROM … OPEN/CLOSE/CLEAR`, `BALANCES`, `JOURNAL`,
  `HAVING`, `PIVOT BY`, other tables, and the running `balance` column.

## Cases

60 cases: 47 `engine` with rows, 6 `ledger-dependent`, and 7 errors (`engine`).

| # | Case | Area | Kind | Ordered | Expect |
|---|---|---|---|---|---|
| 001 | `literals` | select | engine | no | 1 rows |
| 002 | `select_star` | select | engine | yes | 12 rows |
| 003 | `entry_columns` | select | engine | yes | 12 rows |
| 004 | `posting_columns` | select | engine | yes | 19 rows |
| 005 | `description_without_payee` | select | engine | no | 7 rows |
| 006 | `tags_links_sets` | select | engine | no | 4 rows |
| 007 | `id_groups_postings_by_transaction` | select | engine | no | 24 rows |
| 008 | `cost_columns_without_cost` | select | engine | no | 1 rows |
| 009 | `cost_columns_on_lots` | select | ledger-dependent | yes | 8 rows |
| 010 | `regex_case_insensitive` | where | engine | yes | 2 rows |
| 011 | `regex_not_match` | where | engine | yes | 2 rows |
| 012 | `tag_filter` | where | engine | yes | 42 rows |
| 013 | `in_string_list_and_not_in_set` | where | engine | yes | 2 rows |
| 014 | `in_numeric_list` | where | engine | yes | 2 rows |
| 015 | `other_accounts_membership` | where | engine | yes | 6 rows |
| 016 | `boolean_precedence` | where | engine | yes | 4 rows |
| 017 | `date_range_bare_literals` | where | engine | yes | 4 rows |
| 018 | `decimal_comparison` | where | engine | yes | 5 rows |
| 019 | `string_comparison` | where | engine | yes | 4 rows |
| 020 | `is_null_is_not_null` | null | engine | yes | 2 rows |
| 021 | `null_logic_standard` | null | engine | no | 1 rows |
| 022 | `null_logic_beanquery_quirks` | null | engine | no | 1 rows |
| 023 | `not_vs_not_equal_on_null` | null | engine | yes | 2 rows |
| 024 | `order_by_nulls_first_asc` | null | engine | yes | 2 rows |
| 025 | `order_by_nulls_last_desc` | null | engine | yes | 2 rows |
| 026 | `decimal_arithmetic_on_columns` | arithmetic | engine | yes | 9 rows |
| 027 | `literal_arithmetic_and_precedence` | arithmetic | engine | yes | 3 rows |
| 028 | `count_variants` | aggregate | engine | no | 1 rows |
| 029 | `sum_over_int_decimal_amount_position` | aggregate | engine | no | 1 rows |
| 030 | `min_max_first_last_grouped` | aggregate | engine | yes | 4 rows |
| 031 | `aggregate_over_no_rows` | aggregate | engine | no | 0 rows |
| 032 | `sum_position_empty_inventory` | aggregate | engine | yes | 2 rows |
| 033 | `sum_position_by_root_account` | aggregate | engine | yes | 4 rows |
| 034 | `sum_position_with_lots` | aggregate | ledger-dependent | yes | 2 rows |
| 035 | `implicit_group_by` | group-order | engine | no | 4 rows |
| 036 | `group_by_unselected_expression` | group-order | engine | no | 3 rows |
| 037 | `group_by_index_order_by_index_desc` | group-order | engine | yes | 6 rows |
| 038 | `monthly_expenses_by_category` | group-order | engine | yes | 73 rows |
| 039 | `payee_totals_top_n` | group-order | engine | yes | 10 rows |
| 040 | `order_by_mixed_directions` | group-order | engine | yes | 4 rows |
| 041 | `order_by_unselected_expression` | group-order | engine | yes | 8 rows |
| 042 | `distinct_order_limit` | group-order | engine | yes | 5 rows |
| 043 | `from_expression_filter` | from | engine | yes | 4 rows |
| 044 | `from_posting_level_expression` | from | engine | yes | 4 rows |
| 045 | `account_functions` | function | engine | yes | 2 rows |
| 046 | `date_functions` | function | engine | yes | 12 rows |
| 047 | `str_and_length` | function | engine | yes | 12 rows |
| 048 | `meta_missing_key` | function | engine | no | 1 rows |
| 049 | `units_cost_value_without_cost` | function | engine | yes | 9 rows |
| 050 | `holdings_cost_and_market` | valuation | ledger-dependent | yes | 4 rows |
| 051 | `convert_at_date` | valuation | ledger-dependent | yes | 5 rows |
| 052 | `value_latest_and_at_date` | valuation | ledger-dependent | yes | 2 rows |
| 053 | `convert_without_price_keeps_units` | valuation | ledger-dependent | yes | 2 rows |
| 054 | `error_syntax` | error | engine | no | error |
| 055 | `error_unknown_column` | error | engine | no | error |
| 056 | `error_unknown_function` | error | engine | no | error |
| 057 | `error_ungrouped_column` | error | engine | no | error |
| 058 | `error_mixed_aggregate_in_expression` | error | engine | no | error |
| 059 | `error_aggregate_in_where` | error | engine | no | error |
| 060 | `error_type_mismatch` | error | engine | no | error |
