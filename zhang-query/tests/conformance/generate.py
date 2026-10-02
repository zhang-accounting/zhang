#!/usr/bin/env python3
"""Generate the zhang-query conformance fixtures from the beanquery oracle.

The expected results in ``cases/*.json`` are produced by the official Python
beanquery running over the shared ledger, not by zhang. See README.md.

Usage::

    python generate.py [LEDGER]          # (re)write cases/*.json
    python generate.py --check [LEDGER]  # verify cases/*.json are up to date
    python generate.py --table           # also print the README case table

LEDGER defaults to ``integration-tests/fava-demo-ledger/main.zhang`` at the
repository root. Requires only the standard library plus beancount (3.x) and
beanquery (0.2.x).

Besides running each query, the generator validates every case:

* Determinism: the query is re-run over a perturbed copy of the ledger
  (entries reversed within each date, postings reversed within each
  transaction). Ordered cases must produce the same sequence, unordered cases
  the same multiset. This rejects queries whose result depends on tie-breaking.
* Classification: the query is re-run over a copy of the ledger with one
  synthetic zero-amount transaction per ``balance`` directive, which is how
  zhang's Store represents balance checks. A case marked ``engine`` must not be
  sensitive to these rows; ledger-dependent cases get an automatic note.
* Shape: queries with LIMIT must be ordered, and results stay below MAX_ROWS.
"""

import argparse
import collections
import datetime
import json
import os
import sys
import typing
from decimal import Decimal

import beancount
import beanquery
from beancount import loader
from beancount.core import amount, data, inventory, position
from beanquery.sources import beancount as bq_source

HERE = os.path.dirname(os.path.abspath(__file__))
REPO_ROOT = os.path.abspath(os.path.join(HERE, "..", "..", ".."))
DEFAULT_LEDGER = os.path.join(REPO_ROOT, "integration-tests", "fava-demo-ledger", "main.zhang")
CASES_DIR = os.path.join(HERE, "cases")
MAX_ROWS = 200

ENGINE = "engine"
LEDGER = "ledger-dependent"


def case(area, name, query, kind=ENGINE, ordered=False, expect="rows", notes="", oracle_query=None):
    return dict(area=area, name=name, query=query, kind=kind, ordered=ordered, expect=expect,
                notes=notes, oracle_query=oracle_query)


# ---------------------------------------------------------------------------
# Cases. The fixture number is the 1-based position in this list.
# ---------------------------------------------------------------------------

CASES = [
    # --- projection, literals and columns ----------------------------------
    case("select", "literals",
         "SELECT DISTINCT TRUE AS t, FALSE AS f, NULL AS n, 42 AS i, 2.50 AS d, 'single' AS s1, \"double\" AS s2, "
         "2016-01-01 AS dt WHERE account = 'Expenses:Financial:Fees'",
         notes="Literal types: bool, NULL (type null), int, decimal, single- and double-quoted strings, bare date. "
               "A SELECT without FROM still iterates the postings table, so DISTINCT collapses it to one row."),
    case("select", "select_star",
         "SELECT * WHERE account = 'Expenses:Financial:Fees' AND year = 2016 ORDER BY date",
         ordered=True,
         oracle_query="SELECT date, flag, payee, narration, account, position "
                      "WHERE account = 'Expenses:Financial:Fees' AND year = 2016 ORDER BY date",
         notes="DELIBERATE DEVIATION FROM THE ORACLE: beanquery 0.2.0 expands * to date, flag, payee, narration, "
               "position (no account). The zhang Phase 1 spec expands * to date, flag, payee, narration, account, "
               "position, so the expected rows were generated from the explicit query: SELECT date, flag, payee, "
               "narration, account, position WHERE ... ORDER BY date."),
    case("select", "entry_columns",
         "SELECT date, year, month, day, flag, payee, narration, description "
         "WHERE account = 'Expenses:Home:Phone' AND year = 2016 ORDER BY date",
         ordered=True,
         notes="Transaction-level columns joined onto each posting. Narration is '' (empty string) for these "
               "transactions; description is payee and narration joined by ' | ', skipping NULL/empty parts."),
    case("select", "posting_columns",
         "SELECT date, account, number, currency, position, weight, other_accounts "
         "WHERE 'trip-boston-2017' IN tags AND account ~ '^Expenses' ORDER BY date, account, number",
         ordered=True,
         notes="Posting-level columns. Without a cost, weight equals units. other_accounts is the sorted set of the "
               "other postings' accounts in the same transaction."),
    case("select", "description_without_payee",
         "SELECT DISTINCT payee, narration, description "
         "WHERE account ~ '^Expenses:Home' OR (account = 'Assets:US:Vanguard:Cash' AND payee IS NULL)",
         notes="description = ' | '.join of the non-empty parts: payee only when narration is '', narration only "
               "when payee is NULL."),
    case("select", "tags_links_sets",
         "SELECT DISTINCT tags, links WHERE length(tags) > 0",
         notes="Set-typed columns, encoded as sorted string arrays. The ledger has no links, so links is always []."),
    case("select", "id_groups_postings_by_transaction",
         "SELECT first(date) AS date, first(narration) AS narration, count(*) AS postings "
         "WHERE date >= 2015-01-03 AND date <= 2015-01-20 GROUP BY id",
         notes="id values are implementation-defined (beanquery hashes the entry), so only their grouping behaviour "
               "is tested: all postings of a transaction share one id and different transactions differ."),
    case("select", "cost_columns_without_cost",
         "SELECT DISTINCT cost_number, cost_currency, cost_date, cost_label, price WHERE account ~ '^Expenses:Food'",
         notes="beanquery quirk: cost_label is '' (empty string, not NULL) for postings without a cost; the other "
               "cost columns and price are NULL."),
    case("select", "cost_columns_on_lots",
         "SELECT date, account, units(position) AS units, cost_number, cost_currency, cost_date, cost_label, "
         "price, weight WHERE account = 'Assets:US:ETrade:GLD' ORDER BY date, number",
         kind=LEDGER, ordered=True,
         notes="Depends on booking (lot matching fills cost and cost_date on reductions) and on the @ price, which "
               "the current zhang Store posting does not keep. cost_label is NULL for a cost without label. For a "
               "posting with both cost and price, weight is units x cost."),

    # --- WHERE operators ---------------------------------------------------
    case("where", "regex_case_insensitive",
         "SELECT DISTINCT account WHERE account ~ 'expenses:food:(coffee|alcohol)$' ORDER BY account",
         ordered=True,
         notes="~ is a case-insensitive regex search (partial match unless anchored)."),
    case("where", "regex_not_match",
         "SELECT DISTINCT account WHERE account ~ '^Expenses:Home' AND account !~ 'RENT|phone' ORDER BY account",
         ordered=True,
         notes="!~ is the negated case-insensitive regex search."),
    case("where", "tag_filter",
         "SELECT date, payee, account, position WHERE 'trip-chicago-2016' IN tags ORDER BY date, account, number",
         ordered=True,
         notes="Representative query from #434 (tag filter). ORDER BY uses a column that is not selected."),
    case("where", "in_string_list_and_not_in_set",
         "select account, count(*) as n where account in ('Expenses:Food:Coffee', 'Expenses:Food:Alcohol', "
         "'Expenses:Does:Not:Exist') and 'trip-chicago-2016' not in tags group by account order by account",
         ordered=True,
         notes="Value IN a parenthesized list, NOT IN over the tags set. Keywords are written in lower case."),
    case("where", "in_numeric_list",
         "SELECT year, count(*) AS n WHERE year NOT IN (2016, 2018) AND number IN (4, 8.95) "
         "AND account ~ '^Expenses:Financial' GROUP BY year ORDER BY year",
         ordered=True,
         notes="NOT IN over a list of ints; IN mixes an int literal (4) and a decimal (8.95), and the decimal "
               "column number = 4.00 must match the int 4."),
    case("where", "other_accounts_membership",
         "SELECT account, count(*) AS n WHERE 'Expenses:Financial:Commissions' IN other_accounts "
         "GROUP BY account ORDER BY account",
         ordered=True),
    case("where", "boolean_precedence",
         "SELECT DISTINCT account WHERE account ~ 'Food' AND NOT account ~ 'Rest' "
         "OR account = 'Equity:Opening-Balances' ORDER BY account",
         ordered=True,
         notes="NOT binds tighter than AND, AND tighter than OR."),
    case("where", "date_range_bare_literals",
         "SELECT date, payee, number WHERE account = 'Expenses:Home:Rent' "
         "AND date >= 2016-03-01 AND date < 2016-07-01 ORDER BY date",
         ordered=True),
    case("where", "decimal_comparison",
         "SELECT date, payee, number WHERE account = 'Expenses:Food:Groceries' AND number >= 112 "
         "ORDER BY number DESC, date",
         ordered=True,
         notes="Compares the decimal column number with the int literal 112 numerically."),
    case("where", "string_comparison",
         "SELECT DISTINCT payee WHERE payee < 'B' OR payee >= 'Wine' ORDER BY payee",
         ordered=True,
         notes="Lexicographic (code point) string comparison; NULL payees compare as NULL and are filtered out."),

    # --- NULL semantics ----------------------------------------------------
    case("null", "is_null_is_not_null",
         "SELECT payee IS NULL AS no_payee, payee IS NOT NULL AS has_payee, count(*) AS n "
         "WHERE account = 'Assets:US:Vanguard:Cash' GROUP BY 1, 2 ORDER BY 1",
         ordered=True),
    case("null", "null_logic_standard",
         "SELECT DISTINCT TRUE AND NULL AS a, FALSE AND NULL AS b, TRUE OR NULL AS c, NULL OR FALSE AS d, "
         "NULL IS NULL AS e, NULL IS NOT NULL AS f WHERE account = 'Expenses:Financial:Fees'",
         notes="Agrees with SQL three-valued logic."),
    case("null", "null_logic_beanquery_quirks",
         "SELECT DISTINCT NOT NULL AS not_null, NULL AND FALSE AS null_and_false "
         "WHERE account = 'Expenses:Financial:Fees'",
         notes="beanquery quirks that DIFFER from SQL: NOT NULL is TRUE (SQL: NULL), and AND short-circuits on the "
               "first NULL operand, so NULL AND FALSE is NULL (SQL: FALSE; FALSE AND NULL is FALSE)."),
    case("null", "not_vs_not_equal_on_null",
         "SELECT payee IS NULL AS no_payee, NOT (payee = 'Hoogle') AS not_eq, payee != 'Hoogle' AS ne, "
         "count(*) AS n WHERE account = 'Assets:US:Vanguard:Cash' GROUP BY 1, 2, 3 ORDER BY 1",
         ordered=True,
         notes="beanquery quirk: for a NULL payee, payee != 'Hoogle' is NULL but NOT (payee = 'Hoogle') is TRUE, "
               "so WHERE NOT (x = y) keeps NULL rows while WHERE x != y drops them."),
    case("null", "order_by_nulls_first_asc",
         "SELECT payee, count(*) AS n WHERE account ~ '^Assets:US:(Vanguard|Hoogle)' GROUP BY payee "
         "ORDER BY payee",
         ordered=True,
         notes="NULL sorts before every value in ascending order."),
    case("null", "order_by_nulls_last_desc",
         "SELECT payee, count(*) AS n WHERE account ~ '^Assets:US:(Vanguard|Hoogle)' GROUP BY payee "
         "ORDER BY payee DESC",
         ordered=True,
         notes="NULL sorts after every value in descending order (the ascending order reversed)."),

    # --- arithmetic --------------------------------------------------------
    case("arithmetic", "decimal_arithmetic_on_columns",
         "SELECT date, -number AS neg, number * 2 AS dbl, number / 4 AS quarter_part, number + 1 AS plus_one, "
         "number - 0.5 AS minus_half WHERE account = 'Expenses:Financial:Fees' AND year = 2017 ORDER BY date",
         ordered=True,
         notes="Decimal results; compare numerically (beanquery keeps Python Decimal exponents, e.g. 1.00)."),
    case("arithmetic", "literal_arithmetic_and_precedence",
         "SELECT DISTINCT 1 + 2 * 3 AS a, (1 + 2) * 3 AS b, -(3) AS c, 7 - 10 AS d, 7 / 2 AS e, 10 / 0 AS f, "
         "2.5 * 2 AS g, year - 2000 AS yy WHERE account = 'Expenses:Financial:Fees' ORDER BY yy",
         ordered=True,
         notes="int op int stays int except /, which always yields a decimal (7 / 2 = 3.5). Division by zero "
               "yields NULL instead of an error."),

    # --- aggregates --------------------------------------------------------
    case("aggregate", "count_variants",
         "SELECT count(*) AS n, count(payee) AS with_payee, count(cost_number) AS with_cost "
         "WHERE account ~ '^Assets:US:(ETrade|Vanguard)'",
         notes="count(*) counts rows; count(x) counts non-NULL values."),
    case("aggregate", "sum_over_int_decimal_amount_position",
         "SELECT sum(day) AS days, sum(number) AS total, sum(weight) AS by_weight, sum(position) AS by_position "
         "WHERE account = 'Expenses:Financial:Fees'",
         notes="sum(int) -> int, sum(decimal) -> decimal, sum(amount) and sum(position) -> inventory."),
    case("aggregate", "min_max_first_last_grouped",
         "SELECT account, min(payee) AS min_payee, max(payee) AS max_payee, first(date) AS first_date, "
         "last(date) AS last_date, first(number) AS first_amount, last(number) AS last_amount, "
         "min(number) AS min_amount, max(number) AS max_amount "
         "WHERE account ~ '^Expenses:Food' GROUP BY account ORDER BY account",
         ordered=True,
         notes="min/max over str and decimal; first/last follow the row iteration order (by date, then source "
               "order)."),
    case("aggregate", "aggregate_over_no_rows",
         "SELECT count(*) AS n, sum(number) AS total WHERE account = 'Expenses:Does:Not:Exist'",
         notes="beanquery returns ZERO rows (not one row with 0/NULL) when an aggregate query without GROUP BY "
               "matches no input row. This differs from SQL."),
    case("aggregate", "sum_position_empty_inventory",
         "SELECT account, sum(position) AS balance WHERE account ~ '^Liabilities' GROUP BY account ORDER BY account",
         ordered=True,
         notes="Liabilities:AccountsPayable nets to zero, giving an empty inventory {\"positions\": []}."),
    case("aggregate", "sum_position_by_root_account",
         "SELECT root(account, 1) AS root, sum(position) AS balance "
         "WHERE account ~ '^(Income|Expenses|Equity|Liabilities)' GROUP BY 1 ORDER BY 1",
         ordered=True,
         notes="Multi-currency inventories (USD, IRAUSD, VACHR) without lots."),
    case("aggregate", "sum_position_with_lots",
         "SELECT account, sum(position) AS balance WHERE account ~ '^Assets:US:ETrade:(GLD|VEA)$' "
         "GROUP BY account ORDER BY account",
         kind=LEDGER, ordered=True,
         notes="Inventories keep one position per lot (cost number, currency and date). Depends on booking."),

    # --- grouping, ordering, distinct, limit -------------------------------
    case("group-order", "implicit_group_by",
         "SELECT account, count(*) AS n WHERE account ~ '^Expenses:Food'",
         notes="beanquery accepts aggregates mixed with plain columns WITHOUT a GROUP BY and implicitly groups by "
               "all non-aggregate targets."),
    case("group-order", "group_by_unselected_expression",
         "SELECT count(*) AS n, sum(number) AS total WHERE account ~ '^Expenses:Food' GROUP BY year",
         notes="GROUP BY an expression that is not in the target list (hidden group key)."),
    case("group-order", "group_by_index_order_by_index_desc",
         "SELECT root(account, 2) AS category, count(*) AS n, sum(number) AS total "
         "WHERE account ~ '^Expenses' AND currency = 'USD' GROUP BY 1 ORDER BY 3 DESC",
         ordered=True),
    case("group-order", "monthly_expenses_by_category",
         "SELECT year, month, root(account, 2) AS category, sum(position) AS total "
         "WHERE account ~ \"^Expenses\" AND year = 2016 GROUP BY 1, 2, 3 ORDER BY 1, 2, 3",
         ordered=True,
         notes="Representative query from #434, restricted to 2016 to keep the fixture small."),
    case("group-order", "payee_totals_top_n",
         "SELECT payee, sum(cost(position)) AS total WHERE account ~ \"^Expenses\" AND currency = 'USD' "
         "GROUP BY payee ORDER BY total DESC LIMIT 10",
         ordered=True,
         notes="Representative query from #434 (restricted to USD). ORDER BY an inventory: for single-currency "
               "inventories beanquery orders by the units number. Multi-currency inventory ordering follows "
               "beancount's position sort key and is not covered."),
    case("group-order", "order_by_mixed_directions",
         "SELECT date, account, number WHERE account ~ '^Expenses:Food:(Coffee|Alcohol)' AND year = 2017 "
         "ORDER BY account DESC, date ASC, number DESC",
         ordered=True,
         notes="Each ORDER BY key has its own direction (beanquery semantics)."),
    case("group-order", "order_by_unselected_expression",
         "SELECT date, narration WHERE account = 'Expenses:Home:Rent' AND year = 2017 ORDER BY month DESC",
         ordered=True),
    case("group-order", "distinct_order_limit",
         "SELECT DISTINCT account WHERE account ~ '^Expenses' ORDER BY account LIMIT 5",
         ordered=True,
         notes="DISTINCT is applied before LIMIT."),

    # --- FROM --------------------------------------------------------------
    case("from", "from_expression_filter",
         "SELECT account, count(*) AS n FROM year = 2016 WHERE account ~ '^Expenses:Food' "
         "GROUP BY account ORDER BY account",
         ordered=True,
         notes="The FROM expression is a row filter combined with WHERE by AND."),
    case("from", "from_posting_level_expression",
         "SELECT account, count(*) AS n FROM account ~ '^Expenses:Food' GROUP BY account ORDER BY account",
         ordered=True,
         notes="beanquery (v3) evaluates the FROM expression per posting row, so posting columns such as account "
               "are allowed and filter postings (BQL v2 filtered whole transactions)."),

    # --- scalar functions --------------------------------------------------
    case("function", "account_functions",
         "SELECT DISTINCT account, root(account) AS r1, root(account, 2) AS r2, root(account, 9) AS r9, "
         "parent(account) AS p, leaf(account) AS l, parent(root(account)) AS p_of_root "
         "WHERE account ~ 'BofA|^Equity' ORDER BY account",
         ordered=True,
         notes="root(a) defaults to 1 component; root(a, n) with n beyond the depth returns a; parent of a "
               "top-level account is '' (empty string, not NULL)."),
    case("function", "date_functions",
         "SELECT DISTINCT date, year(date) AS y, month(date) AS m, day(date) AS d, quarter(date) AS q "
         "WHERE account = 'Expenses:Home:Rent' AND year = 2016 ORDER BY date",
         ordered=True,
         notes="quarter() returns a string 'YYYY-Qn'."),
    case("function", "str_and_length",
         "SELECT DISTINCT str(year) AS y, str(date) AS d, str(number) AS n, str(TRUE) AS t, "
         "length(narration) AS ln, length(tags) AS lt, length(other_accounts) AS lo "
         "WHERE account = 'Expenses:Financial:Fees' AND year = 2016 ORDER BY d",
         ordered=True,
         notes="str(bool) is 'TRUE'/'FALSE'; str(decimal) keeps the exponent (4.00). length works on str and sets."),
    case("function", "meta_missing_key",
         "SELECT DISTINCT meta('category') AS m, entry_meta('category') AS em WHERE account ~ '^Expenses:Food'",
         notes="The shared ledger has no posting or transaction metadata, so only the missing-key -> NULL path is "
               "covered. beanquery types meta()/entry_meta() as object; the fixture uses str."),
    case("function", "units_cost_value_without_cost",
         "SELECT date, units(position) AS u, cost(position) AS c, value(position) AS v, weight "
         "WHERE account = 'Expenses:Financial:Fees' AND year = 2017 ORDER BY date",
         ordered=True,
         notes="Without a cost, cost(position) and value(position) both return the units amount."),

    # --- valuation (price map, lots) ---------------------------------------
    case("valuation", "holdings_cost_and_market",
         "SELECT account, units(sum(position)) AS qty, cost(sum(position)) AS book, "
         "convert(units(sum(position)), \"USD\") AS market "
         "WHERE account ~ \"^Assets:US:ETrade:\" AND account != 'Assets:US:ETrade:Cash' "
         "GROUP BY account ORDER BY account",
         kind=LEDGER, ordered=True,
         notes="Representative query from #434. market uses the latest price in the price map."),
    case("valuation", "convert_at_date",
         "SELECT account, convert(sum(position), 'USD', 2016-06-30) AS market_mid_2016 "
         "WHERE account ~ '^Assets:US:ETrade:' AND date <= 2016-06-30 GROUP BY account ORDER BY account",
         kind=LEDGER, ordered=True,
         notes="convert(x, ccy, date) uses the latest price on or before date."),
    case("valuation", "value_latest_and_at_date",
         "SELECT account, value(sum(position)) AS latest, value(sum(position), 2016-12-31) AS at_2016_end "
         "WHERE account ~ '^Assets:US:Vanguard:' AND account != 'Assets:US:Vanguard:Cash' "
         "GROUP BY account ORDER BY account",
         kind=LEDGER, ordered=True,
         notes="value() converts each lot to its cost currency at the market price."),
    case("valuation", "convert_without_price_keeps_units",
         "SELECT currency, count(*) AS n, sum(convert(units(position), 'USD')) AS converted "
         "WHERE account ~ '^Expenses:(Taxes|Vacation)' AND currency != 'USD' GROUP BY currency ORDER BY currency",
         kind=LEDGER, ordered=True,
         notes="With no IRAUSD/VACHR price in the price map, convert() returns the amount unchanged."),

    # --- errors ------------------------------------------------------------
    case("error", "error_syntax", "SELECT account WHERE", expect="error"),
    case("error", "error_unknown_column", "SELECT nosuchcolumn", expect="error"),
    case("error", "error_unknown_function", "SELECT nosuchfunction(account)", expect="error"),
    case("error", "error_ungrouped_column",
         "SELECT account, payee, count(*) GROUP BY account", expect="error",
         notes="With an explicit GROUP BY, every non-aggregate target must be grouped."),
    case("error", "error_mixed_aggregate_in_expression", "SELECT number + sum(number)", expect="error"),
    case("error", "error_aggregate_in_where", "SELECT count(*) WHERE sum(number) > 0", expect="error"),
    case("error", "error_type_mismatch", "SELECT 'a' + 1", expect="error"),
]


# ---------------------------------------------------------------------------
# Oracle plumbing
# ---------------------------------------------------------------------------

def load_ledger(path):
    entries, errors, options = loader.load_file(path)
    if errors:
        for error in errors:
            print(f"ledger error: {error}", file=sys.stderr)
        sys.exit(f"{path}: {len(errors)} beancount load error(s); the shared ledger must load cleanly")
    return entries, options


def connect(entries, options):
    conn = beanquery.Connection()
    bq_source.attach(conn, "beancount:", entries=entries, errors=[], options=options)
    return conn


def perturbed(entries):
    """Reverse entries within each date and postings within each transaction."""
    by_date = collections.OrderedDict()
    for entry in entries:
        by_date.setdefault(entry.date, []).append(entry)
    out = []
    for group in by_date.values():
        for entry in reversed(group):
            if isinstance(entry, data.Transaction):
                entry = entry._replace(postings=list(reversed(entry.postings)))
            out.append(entry)
    return out


def with_synthetic_balance_checks(entries):
    """Add one zero-amount transaction per balance directive, like zhang's Store."""
    out = list(entries)
    for entry in entries:
        if isinstance(entry, data.Balance):
            posting = data.Posting(entry.account, amount.Amount(Decimal(0), entry.amount.currency),
                                   None, None, None, {})
            out.append(data.Transaction({}, entry.date, "B", "Balance Check", entry.account,
                                        frozenset(), frozenset(), [posting]))
    out.sort(key=lambda e: e.date)  # stable: synthetic rows go last within their date
    return out


def run(conn, query):
    cursor = conn.execute(query)
    return list(cursor.description), [tuple(row) for row in cursor.fetchall()]


# ---------------------------------------------------------------------------
# Encoding (see README.md, "Type mapping" and "Cell encoding")
# ---------------------------------------------------------------------------

def column_type(dtype):
    if dtype is type(None):
        return "null"
    if dtype is bool:
        return "bool"
    if dtype is int:
        return "int"
    if dtype is Decimal:
        return "decimal"
    if dtype is str:
        return "str"
    if dtype is datetime.date:
        return "date"
    if dtype in (set, frozenset, list) or typing.get_origin(dtype) in (set, frozenset, list):
        return "set"
    if dtype is amount.Amount:
        return "amount"
    if dtype is position.Position:
        return "position"
    if dtype is inventory.Inventory:
        return "inventory"
    if dtype is object:
        # meta()/entry_meta() are dynamically typed in beanquery; zhang metadata values are strings.
        return "str"
    raise TypeError(f"unmapped beanquery datatype {dtype!r}")


def dec(number):
    return format(number, "f")


def enc_amount(value):
    return {"number": dec(value.number), "currency": value.currency}


def enc_position(value):
    cost = value.cost
    return {
        "units": enc_amount(value.units),
        "cost": None if cost is None else {
            "number": dec(cost.number),
            "currency": cost.currency,
            "date": cost.date.isoformat() if cost.date else None,
            "label": cost.label,
        },
    }


def position_sort_key(pos):
    cost = pos.cost
    if cost is None:
        cost_key = (0, Decimal(0), "", "", "")
    else:
        cost_key = (1, cost.number, cost.currency, cost.date.isoformat() if cost.date else "", cost.label or "")
    return (pos.units.currency, cost_key, pos.units.number)


def enc_cell(value):
    if value is None:
        return None
    if isinstance(value, bool):
        return value
    if isinstance(value, int):
        return value
    if isinstance(value, Decimal):
        return dec(value)
    if isinstance(value, str):
        return value
    if isinstance(value, datetime.date):
        return value.isoformat()
    if isinstance(value, inventory.Inventory):
        return {"positions": [enc_position(p) for p in sorted(value.get_positions(), key=position_sort_key)]}
    if isinstance(value, position.Position):
        return enc_position(value)
    if isinstance(value, amount.Amount):
        return enc_amount(value)
    if isinstance(value, (set, frozenset, list, tuple)):
        return sorted(value)
    raise TypeError(f"unmapped beanquery value {value!r} ({type(value).__name__})")


def encode(description, rows):
    columns = [{"name": col.name, "type": column_type(col.datatype)} for col in description]
    return columns, [[enc_cell(v) for v in row] for row in rows]


def same_result(a_rows, b_rows, ordered):
    if ordered:
        return a_rows == b_rows
    key = lambda row: json.dumps(row, sort_keys=True)
    return sorted(map(key, a_rows)) == sorted(map(key, b_rows))


# ---------------------------------------------------------------------------
# Fixture writing
# ---------------------------------------------------------------------------

def dumps(value):
    return json.dumps(value, ensure_ascii=False)


def render_fixture(fixture):
    lines = ["{"]
    for key in ("name", "query", "kind", "ordered", "expect"):
        lines.append(f"  {dumps(key)}: {dumps(fixture[key])},")
    for key in ("columns", "rows"):
        items = fixture[key]
        if items:
            lines.append(f"  {dumps(key)}: [")
            lines.extend(f"    {dumps(item)}," for item in items[:-1])
            lines.append(f"    {dumps(items[-1])}")
            lines.append("  ],")
        else:
            lines.append(f"  {dumps(key)}: [],")
    lines.append(f"  {dumps('notes')}: {dumps(fixture['notes'])}")
    lines.append("}")
    return "\n".join(lines) + "\n"


def build_fixture(index, spec, conns):
    base, shuffled, synthetic = conns
    query = spec["oracle_query"] or spec["query"]
    problems = []
    notes = spec["notes"]

    if spec["expect"] == "error":
        try:
            run(base, query)
        except Exception as exc:  # beanquery raises ParseError / CompilationError
            detail = f"{type(exc).__name__}: {exc}"
        else:
            raise SystemExit(f"case {spec['name']}: expected an error but beanquery accepted the query")
        columns, rows = [], []
    else:
        description, raw_rows = run(base, query)
        columns, rows = encode(description, raw_rows)
        detail = f"{len(rows)} rows"
        if len(rows) > MAX_ROWS:
            problems.append(f"{len(rows)} rows exceeds MAX_ROWS={MAX_ROWS}")
        if " LIMIT " in f" {spec['query'].upper()} " and not spec["ordered"]:
            problems.append("LIMIT without a determining ORDER BY (mark ordered and order totally)")
        _, shuffled_rows = encode(*run(shuffled, query))
        if not same_result(rows, shuffled_rows, spec["ordered"]):
            problems.append("result depends on tie-breaking between equal sort keys or same-day rows")
        _, synthetic_rows = encode(*run(synthetic, query))
        if not same_result(rows, synthetic_rows, spec["ordered"]):
            if spec["kind"] == ENGINE:
                problems.append("sensitive to zhang's synthetic balance-check postings; mark it ledger-dependent "
                                "or narrow the WHERE clause")
            else:
                extra = ("Also sensitive to whether zhang's synthetic balance-check transactions (zero-amount "
                         "postings) are part of the postings table.")
                notes = f"{notes} {extra}".strip()
    if problems:
        raise SystemExit(f"case {spec['name']}: " + "; ".join(problems))

    fixture = {
        "name": spec["name"],
        "query": spec["query"],
        "kind": spec["kind"],
        "ordered": spec["ordered"],
        "expect": spec["expect"],
        "columns": columns,
        "rows": rows,
        "notes": notes,
    }
    filename = f"{index:03d}_{spec['name']}.json"
    return filename, render_fixture(fixture), detail


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("ledger", nargs="?", default=DEFAULT_LEDGER)
    parser.add_argument("--check", action="store_true", help="verify fixtures instead of writing them")
    parser.add_argument("--table", action="store_true", help="also print the README case table (Markdown)")
    args = parser.parse_args()

    names = [spec["name"] for spec in CASES]
    duplicates = [name for name, count in collections.Counter(names).items() if count > 1]
    if duplicates:
        sys.exit(f"duplicate case names: {duplicates}")

    entries, options = load_ledger(args.ledger)
    conns = (connect(entries, options),
             connect(perturbed(entries), options),
             connect(with_synthetic_balance_checks(entries), options))
    print(f"oracle: beancount {beancount.__version__}, beanquery {beanquery.__version__}; ledger {args.ledger}")

    outputs = {}
    details = []
    for index, spec in enumerate(CASES, start=1):
        filename, text, detail = build_fixture(index, spec, conns)
        outputs[filename] = text
        details.append(detail)
        print(f"  {filename:<55} {spec['kind']:<17} {detail}")

    os.makedirs(CASES_DIR, exist_ok=True)
    existing = {name for name in os.listdir(CASES_DIR) if name.endswith(".json")}
    if args.check:
        stale = sorted(existing - set(outputs))
        changed = []
        for filename, text in outputs.items():
            path = os.path.join(CASES_DIR, filename)
            if not os.path.exists(path) or open(path, encoding="utf-8").read() != text:
                changed.append(filename)
        if stale or changed:
            sys.exit(f"fixtures out of date: changed={changed} stale={stale}")
        print("fixtures are up to date")
    else:
        for filename in existing - set(outputs):
            os.remove(os.path.join(CASES_DIR, filename))
        for filename, text in outputs.items():
            with open(os.path.join(CASES_DIR, filename), "w", encoding="utf-8") as handle:
                handle.write(text)

    by_area = collections.Counter(spec["area"] for spec in CASES)
    by_kind = collections.Counter(spec["kind"] if spec["expect"] == "rows" else "error" for spec in CASES)
    print(f"{len(CASES)} cases; by area: {dict(by_area)}; by kind: {dict(by_kind)}")

    if args.table:
        print("\n| # | Case | Area | Kind | Ordered | Expect |")
        print("|---|---|---|---|---|---|")
        for (index, spec), detail in zip(enumerate(CASES, start=1), details):
            expect = "error" if spec["expect"] == "error" else detail
            print(f"| {index:03d} | `{spec['name']}` | {spec['area']} | {spec['kind']} | "
                  f"{'yes' if spec['ordered'] else 'no'} | {expect} |")


if __name__ == "__main__":
    main()
