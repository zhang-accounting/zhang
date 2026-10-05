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
* CSV (``expect: "csv"``): the result goes through beanquery's numberify step
  and CSV writer, as ``bean-query -m -f csv`` does. The numberify step rounds
  numbers to the ledger's display precision; a case whose cells change under
  that rounding is rejected, so the fixtures never depend on it.
"""

import argparse
import collections
import csv
import datetime
import io
import json
import os
import re
import sys
import typing
from decimal import Decimal

import beancount
import beanquery
from beancount import loader
from beancount.core import amount, data, inventory, position
from beanquery.numberify import numberify_results
from dateutil.relativedelta import relativedelta
from beanquery.query_render import render_csv
from beanquery.sources import beancount as bq_source

HERE = os.path.dirname(os.path.abspath(__file__))
REPO_ROOT = os.path.abspath(os.path.join(HERE, "..", "..", ".."))
DEFAULT_LEDGER = os.path.join(REPO_ROOT, "integration-tests", "fava-demo-ledger", "main.zhang")
CASES_DIR = os.path.join(HERE, "cases")
MAX_ROWS = 200

ENGINE = "engine"
LEDGER = "ledger-dependent"

# beanquery exception type -> fixture `error_class` of an `expect: "error"` case
ERROR_CLASSES = {
    beanquery.ParseError: "syntax",
    beanquery.CompilationError: "compile",
}


def case(area, name, query, kind=ENGINE, ordered=False, expect="rows", notes="", oracle_query=None, phase=1,
         strict_names=False):
    return dict(area=area, name=name, query=query, kind=kind, ordered=ordered, expect=expect,
                notes=notes, oracle_query=oracle_query, phase=phase, strict_names=strict_names)


def case2(area, name, query, **kwargs):
    """A Phase 2 case (issue #434): BALANCES, JOURNAL, the balance column, FROM OPEN/CLOSE/CLEAR, CSV."""
    return case(area, name, query, phase=2, **kwargs)


def case4(area, name, query, **kwargs):
    """A case of the engine-language features of issue #479 (wave 1) that beanquery has: its date functions
    (date_trunc, date_part, date_add, date_diff, date(), interval, date_bin) with the INTERVAL type, and its
    account and commodity directive functions (open_date, close_date, open_meta, commodity_meta)."""
    return case(area, name, query, phase=4, **kwargs)


def case3(area, name, query, **kwargs):
    """A Phase 3 case (issue #434): HAVING, PIVOT BY and FROM #table.

    ``strict_names=True`` makes the harness compare the column names too, for results whose names carry
    meaning: the pivoted columns and the expansion of ``SELECT *`` over a table.
    """
    return case(area, name, query, phase=3, **kwargs)


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

    # =======================================================================
    # Phase 2 (issue #434). Fixtures carry "phase": 2.
    # =======================================================================

    # --- BALANCES ----------------------------------------------------------
    case2("balances", "balances_plain", "BALANCES",
          kind=LEDGER, ordered=True,
          notes="BALANCES [AT f] [FROM ...] [WHERE ...] is SELECT account, sum(f(position)) GROUP BY account "
                "ORDER BY account_sortkey(account). The order is by account type (Assets, Liabilities, Equity, "
                "Income, Expenses), then by name. Every account with postings is listed, even when its balance "
                "nets to the empty inventory (Assets:US:Federal:PreTax401k, Liabilities:AccountsPayable). "
                "beanquery names the second column SUM((position)). Lots are kept, so the result depends on "
                "booking."),
    case2("balances", "balances_where_account_type_order", "BALANCES WHERE account ~ 'Vacation|Opening|Slate|Fees'",
          ordered=True,
          notes="The account_sortkey order is not alphabetical: Liabilities comes before Equity, and Income "
                "before Expenses."),
    case2("balances", "balances_at_units", "BALANCES AT units WHERE account ~ '^Assets'",
          ordered=True,
          notes="AT units sums units(position): lots of one commodity merge into a single position without "
                "cost."),
    case2("balances", "balances_at_cost", "BALANCES AT cost WHERE account ~ '^Assets:US:(ETrade|Vanguard)'",
          kind=LEDGER, ordered=True,
          notes="AT cost sums cost(position), the book value of every lot. Exact products are kept "
                "(49049.66613 USD), not rounded to the currency precision."),
    case2("balances", "balances_from_close_on", "BALANCES FROM CLOSE ON 2015-02-01",
          kind=LEDGER, ordered=True,
          notes="CLOSE ON d keeps the entries dated before d and appends a conversions transaction (flag C) on "
                "Equity:Conversions:Current, which holds minus the cost-basis total of all postings so that the "
                "ledger sums to zero. Here that is the rounding residual of lot purchases, -0.00177 USD."),
    case2("balances", "balances_at_cost_open_close_where",
          "BALANCES AT cost FROM OPEN ON 2017-01-01 CLOSE ON 2017-07-01 WHERE account ~ '^(Assets:US:ETrade|Equity)'",
          kind=LEDGER, ordered=True,
          notes="All three modifiers with AT and WHERE. OPEN moves earlier income and expenses to "
                "Equity:Earnings:Previous and earlier conversions to Equity:Conversions:Previous, then "
                "summarizes every balance against Equity:Opening-Balances. CLOSE adds Equity:Conversions:Current."),

    # --- JOURNAL -----------------------------------------------------------
    case2("journal", "journal_account_regex", "JOURNAL 'expenses:food:coffee'",
          ordered=True,
          notes="JOURNAL 'r' [AT f] [FROM ...] is SELECT date, flag, maxwidth(payee, 48), maxwidth(narration, 80), "
                "account, f(position), f(balance) WHERE account ~ \"r\", with no ORDER BY: rows come in ledger "
                "order (by date, then source order). The pattern is the case-insensitive partial-match ~ "
                "regex. Column names are beanquery's (MAXWIDTH(payee, 48), ...) and advisory only. JOURNAL "
                "takes no WHERE, ORDER BY or LIMIT. Every matched account has one posting per day, so the "
                "order and the running balance do not depend on ties."),
    case2("journal", "journal_several_accounts_and_currencies",
          "JOURNAL '^Expenses:(Vacation|Home:Rent)$' FROM year = 2017",
          ordered=True,
          notes="An anchored regex matching two accounts. The balance column is one running inventory over "
                "all journal rows, not one per account, so it mixes USD and VACHR. The FROM expression "
                "restricts the rows, so the balance starts from zero on the first 2017 row. A NULL payee "
                "stays NULL through maxwidth()."),
    case2("journal", "journal_maxwidth_trims_narration",
          "JOURNAL 'Expenses:Food:Restaurant' FROM year = 2017 AND month = 6",
          ordered=True,
          notes="maxwidth(s, n) is Python's textwrap.shorten(s, width=n): it collapses runs of whitespace, strips "
                "leading and trailing whitespace, and if the text is still longer than n it cuts at a word "
                "boundary and appends ' [...]'. The narration 'Eating out ' (trailing space in the ledger) is "
                "returned as 'Eating out'. No payee is longer than 48 characters and no narration longer than "
                "80 in this ledger, so the truncation itself is not exercised."),
    case2("journal", "journal_at_cost", "JOURNAL 'GLD' AT cost",
          kind=LEDGER, ordered=True,
          notes="AT cost applies cost() to both the position and the running balance: cost(position) is an "
                "amount and cost(balance) an inventory. The sells reduce lots booked by the ledger."),
    case2("journal", "journal_open_close_summary_row",
          "JOURNAL 'Assets:US:BofA:Checking' FROM OPEN ON 2017-06-01 CLOSE ON 2017-07-01",
          kind=LEDGER, ordered=True,
          notes="The first row is the OPEN summarization posting: dated the day before the OPEN date, flag S, "
                "payee NULL, narration \"Opening balance for 'Assets:US:BofA:Checking' (Summarization)\". It "
                "carries the account's balance at the OPEN date, so the running balance continues from it. "
                "CLOSE ON 2017-07-01 drops the rows dated 2017-07-01 and later."),

    # --- running balance column --------------------------------------------
    case2("balance-column", "balance_running_total",
          "SELECT date, position, balance WHERE account ~ 'Checking' AND date >= 2017-06-01 ORDER BY date LIMIT 15",
          kind=LEDGER, ordered=True,
          notes="balance is the running sum (an inventory) of position over the rows that pass FROM and WHERE, "
                "in ledger order. It starts empty, so it does not include postings before the WHERE window. "
                "The window has one Checking posting per day, so the sums do not depend on ties."),
    case2("balance-column", "balance_computed_before_order_by",
          "SELECT date, position, balance WHERE account ~ 'Checking' AND date >= 2017-06-01 "
          "ORDER BY date DESC LIMIT 5",
          kind=LEDGER, ordered=True,
          notes="The running balance is computed in ledger order before ORDER BY and LIMIT: with ORDER BY date "
                "DESC the first row carries the final balance of the whole window, not its own position."),
    case2("balance-column", "balance_spans_all_selected_accounts",
          "SELECT date, account, position, balance WHERE account ~ '^Expenses:Food:(Coffee|Alcohol)$' "
          "AND year = 2016 AND month = 4 ORDER BY date",
          ordered=True,
          notes="One running balance over all selected rows, across accounts."),
    case2("balance-column", "balance_with_lots",
          "SELECT date, units(position) AS units, cost(position) AS cost, balance "
          "WHERE account = 'Assets:US:ETrade:VHT' ORDER BY date",
          kind=LEDGER, ordered=True,
          notes="The running inventory keeps lots: a reduction removes units from the lot it was booked "
                "against. Depends on booking."),

    # --- FROM OPEN / CLOSE / CLEAR -----------------------------------------
    case2("period", "open_on_summarization_rows",
          "SELECT account, count(*) AS n, sum(position) AS total FROM OPEN ON 2016-01-01 "
          "WHERE flag = 'S' AND account !~ ':(RGAGX|VBMPX)$' GROUP BY account ORDER BY account",
          kind=LEDGER, ordered=True,
          notes="OPEN ON d (1) inserts a conversions transaction dated d - 1 on Equity:Conversions:Previous, "
                "(2) transfers each income/expense balance before d to Equity:Earnings:Previous, then "
                "(3) replaces every transaction before d by one summarization transaction per account, dated "
                "d - 1, flag S, with one posting per position (lot) of the account's balance and one "
                "counterpart posting at cost on Equity:Opening-Balances per position. Income and expense "
                "accounts therefore get no S rows. Equity:Opening-Balances also summarizes its own balance, "
                "so it has many S postings. The Vanguard lot accounts are excluded only to keep the fixture "
                "small."),
    case2("period", "open_on_only_summaries_before_date",
          "SELECT flag, count(*) AS n, min(date) AS first, max(date) AS last FROM OPEN ON 2016-01-01 "
          "WHERE date < 2016-01-01 GROUP BY flag",
          kind=LEDGER,
          notes="Before the OPEN date only the S postings remain, all dated 2015-12-31. Their number (90) is "
                "two per summarized position, so it depends on the lots."),
    case2("period", "income_statement_2016",
          "SELECT account, sum(position) FROM OPEN ON 2016-01-01 CLOSE ON 2017-01-01 "
          "WHERE account ~ '^(Income|Expenses)' GROUP BY 1",
          notes="Representative query from #434. OPEN moves earlier income and expenses to equity, so these "
                "accounts start the period at zero and the sums are exactly the 2016 activity."),
    case2("period", "close_on_conversion_entry",
          "SELECT date, flag, payee, account, position FROM CLOSE ON 2016-01-01 WHERE flag = 'C'",
          kind=LEDGER, ordered=True,
          notes="The conversions transaction of CLOSE ON d: dated d - 1, flag C, payee NULL, one posting per "
                "currency on Equity:Conversions:Current with minus the cost-basis total of all postings before "
                "d. Its narration ('Conversion for (...)', beancount's inventory formatting) is deliberately not "
                "selected. Note that zhang stores balance assertions as transactions with flag C too; they are "
                "not rows of the postings table."),
    case2("period", "close_on_truncates_before_date",
          "SELECT count(*) AS n, min(date) AS first, max(date) AS last FROM CLOSE ON 2016-01-01 WHERE flag = '*'",
          notes="CLOSE ON d is exclusive: only postings dated before d remain."),
    case2("period", "close_without_date",
          "SELECT date, flag, payee, account, position FROM CLOSE WHERE flag = 'C'",
          kind=LEDGER, ordered=True,
          notes="CLOSE without a date truncates nothing; it only appends the conversions transaction, dated "
                "like the last entry of the ledger (2017-09-08, the date of the last transaction and of the "
                "last prices)."),
    case2("period", "clear_transfers",
          "SELECT root(account, 1) AS type, count(*) AS n, min(date) AS first, max(date) AS last, "
          "sum(position) AS total FROM CLEAR WHERE flag = 'T' GROUP BY 1 ORDER BY 1",
          ordered=True,
          notes="CLEAR adds one transfer transaction per income/expense account with a non-empty balance: "
                "flag T, payee NULL, dated like the last entry (2017-09-08 here), one posting per position "
                "that zeroes the account and a counterpart on Equity:Earnings:Current."),
    case2("period", "clear_zeroes_income_statement",
          "SELECT root(account, 1) AS type, sum(position) AS total FROM CLEAR "
          "WHERE account ~ '^(Income|Expenses|Equity)' GROUP BY 1 ORDER BY 1",
          ordered=True,
          notes="After CLEAR, Income and Expenses sum to empty inventories and Equity holds the net income."),
    case2("period", "clear_in_journal", "JOURNAL 'Expenses:Food:Coffee' FROM CLEAR",
          ordered=True,
          notes="The T row comes last and brings the running balance back to the empty inventory."),
    case2("period", "balance_sheet_2017_with_clear",
          "SELECT account, cost(sum(position)) AS book FROM CLOSE ON 2017-01-01 CLEAR "
          "WHERE account ~ '^(Assets|Liabilities|Equity)' GROUP BY 1 ORDER BY 1",
          kind=LEDGER, ordered=True,
          notes="Balance sheet at 2017-01-01. CLOSE ON then CLEAR: the transfers are dated like the last remaining "
                "entry, which is the conversions transaction on 2016-12-31. Equity:Earnings:Current holds the net "
                "income and Equity:Conversions:Current the conversion residual."),
    case2("period", "open_close_clear_flags",
          "SELECT flag, count(*) AS n, min(date) AS first, max(date) AS last "
          "FROM OPEN ON 2016-01-01 CLOSE ON 2017-01-01 CLEAR GROUP BY flag ORDER BY flag",
          kind=LEDGER, ordered=True,
          notes="Counts and dates of the synthetic postings per flag: S (summarization, 2015-12-31), C "
                "(conversions, 2016-12-31) and T (transfers, 2016-12-31), next to the real '*' postings."),
    case2("period", "summarization_narrations",
          "SELECT date, flag, payee, narration, account, position FROM OPEN ON 2016-01-01 CLOSE ON 2017-01-01 CLEAR "
          "WHERE flag IN ('S', 'T') AND narration ~ 'Coffee|Checking' ORDER BY flag, account",
          ordered=True,
          notes="Narrations of the synthetic transactions: \"Opening balance for '<account>' (Summarization)\" "
                "and \"Transfer balance for '<account>' (Transfer balance)\". The counterpart posting "
                "(Equity:Opening-Balances, Equity:Earnings:Current) has the narration of the account it "
                "balances. Payee is NULL."),
    case2("period", "from_expression_with_open_close",
          "SELECT date, flag, position, balance FROM account = 'Assets:US:ETrade:Cash' "
          "OPEN ON 2017-01-01 CLOSE ON 2017-03-01 ORDER BY date",
          ordered=True,
          notes="FROM <expr> OPEN ON ... CLOSE ON ...: the modifiers transform the whole ledger and the expression "
                "then filters its postings, so the S row carries the full balance at the OPEN date."),

    # --- CSV export (numberify) --------------------------------------------
    case2("csv", "csv_inventory_sums",
          "SELECT account, sum(position) AS balance WHERE account ~ '^Liabilities|Vacation' "
          "GROUP BY account ORDER BY account",
          expect="csv", ordered=True,
          notes="numberify splits an inventory column into one decimal column per currency, named "
                "'<column> (<currency>)'. The columns are ordered by the number of rows holding the currency "
                "(most first), ties by currency name descending. A currency missing from the row, or summing to "
                "zero, is an empty cell; an empty inventory is all empty cells."),
    case2("csv", "csv_holdings_with_cost",
          "SELECT account, units(sum(position)) AS units, cost(sum(position)) AS book "
          "WHERE account ~ '^Assets:US:ETrade:' AND account != 'Assets:US:ETrade:Cash' GROUP BY account ORDER BY account",
          expect="csv", kind=LEDGER, ordered=True,
          notes="Four commodities with one row each tie on the count, so they are ordered by name descending: "
                "VHT, VEA, ITOT, GLD."),
    case2("csv", "csv_amount_columns_mixed_currencies",
          "SELECT account, units(position) AS units, weight WHERE date = 2017-01-12 AND payee = 'Hoogle' "
          "ORDER BY account",
          expect="csv", ordered=True,
          notes="Amount columns split per currency like inventories: USD (13 rows), then VACHR and IRAUSD "
                "(2 rows each, name descending)."),
    case2("csv", "csv_positions_cost_and_price",
          "SELECT date, account, position, price, weight, cost_number WHERE account ~ 'ETrade:(GLD|Cash)' "
          "AND date >= 2017-02-15 AND date <= 2017-03-19 ORDER BY date, account, number",
          expect="csv", kind=LEDGER, ordered=True,
          notes="A position column keeps only the units number per currency; the cost basis is dropped. "
                "price (an amount) is empty when NULL. cost_number is a plain decimal column."),
    case2("csv", "csv_sets_nulls_bools",
          "SELECT date, payee, narration, tags, other_accounts, payee IS NULL AS no_payee, price, number "
          "WHERE (account = 'Assets:US:ETrade:Cash' AND year = 2017 AND month <= 3) "
          "OR (account = 'Expenses:Food:Coffee' AND year = 2017) ORDER BY date, number",
          expect="csv", ordered=True,
          notes="NULL and the empty string are both an empty cell. A set is its sorted elements joined by ',' "
                "(quoted by the CSV writer when it has several). Booleans are TRUE and FALSE, dates YYYY-MM-DD. "
                "beanquery pads decimals with spaces to align them; cells are compared trimmed. The price column "
                "is NULL on every row, so numberify finds no currency for it and the column DISAPPEARS from the "
                "CSV: an amount, position or inventory column without any currency yields zero CSV columns."),

    # --- Phase 2 errors ----------------------------------------------------
    case2("error", "error_journal_non_string", "JOURNAL 42", expect="error",
          notes="The JOURNAL account pattern must be a string literal."),
    case2("error", "error_journal_where_clause", "JOURNAL 'Checking' WHERE year = 2016", expect="error",
          notes="JOURNAL accepts only an account pattern, AT and FROM; filter with FROM <expr> instead."),
    case2("error", "error_open_on_non_date", "SELECT count(*) FROM OPEN ON 2016", expect="error",
          notes="OPEN ON and CLOSE ON take a bare date literal (YYYY-MM-DD)."),
    case2("error", "error_open_without_on", "SELECT count(*) FROM OPEN 2016-01-01", expect="error"),
    case2("error", "error_clear_before_close", "SELECT count(*) FROM CLEAR CLOSE ON 2017-01-01", expect="error",
          notes="The modifiers have a fixed order: [<expr>] [OPEN ON d] [CLOSE [ON d]] [CLEAR]."),
    case2("error", "error_close_date_before_open_date",
          "SELECT count(*) FROM OPEN ON 2017-01-01 CLOSE ON 2016-01-01", expect="error",
          notes="CLOSE date must follow OPEN date (a compile error, not a syntax error). Equal dates are allowed."),
    case2("error", "error_balances_unknown_summary_function", "BALANCES AT nosuch", expect="error",
          notes="AT f applies f to position; an unknown function is a compile error."),

    # =======================================================================
    # Phase 3 (issue #434). Fixtures carry "phase": 3.
    # =======================================================================

    # --- HAVING ------------------------------------------------------------
    case3("having", "having_on_aggregate",
          "SELECT account, count(*) AS n, sum(number) AS total WHERE account ~ '^Expenses:Food' "
          "GROUP BY account HAVING count(*) > 20 ORDER BY account",
          ordered=True,
          notes="HAVING is part of the GROUP BY clause: GROUP BY ... [HAVING expr] [ORDER BY ...] [PIVOT BY ...] "
                "[LIMIT n]. It keeps the groups whose HAVING value is true. The aggregate may also be a target."),
    case3("having", "having_on_hidden_aggregate",
          "SELECT account WHERE account ~ '^Expenses:(Home|Food)' GROUP BY account HAVING sum(number) > 3000 "
          "ORDER BY account",
          ordered=True,
          notes="The HAVING aggregate does not have to be selected: it is computed per group like a hidden "
                "target and is not part of the output."),
    case3("having", "having_group_key_inside_aggregate",
          "SELECT account, count(*) AS n WHERE account ~ '^Expenses:Food' GROUP BY account "
          "HAVING min(account) ~ 'Coffee|Alcohol' OR count(*) > 300 ORDER BY account",
          ordered=True,
          notes="beanquery requires the HAVING expression to contain an aggregate, so a group key is tested "
                "through an aggregate of it (min(account) is the key itself). A bare group key in HAVING is a "
                "compile error (error_having_bare_group_key)."),
    case3("having", "having_order_by_limit",
          "SELECT payee, count(*) AS n, sum(number) AS total WHERE account ~ '^Expenses' AND currency = 'USD' "
          "GROUP BY payee HAVING count(*) >= 10 ORDER BY total DESC LIMIT 5",
          ordered=True,
          notes="HAVING filters the groups before ORDER BY and LIMIT."),
    case3("having", "having_hidden_group_key",
          "SELECT count(*) AS n, sum(number) AS total WHERE account ~ '^Expenses:Food' GROUP BY year "
          "HAVING sum(number) > 6800",
          notes="GROUP BY a column that is not selected, filtered by HAVING: only the 2016 group passes."),
    case3("having", "having_null_drops_group",
          "SELECT account, count(*) AS n WHERE account ~ '^Assets:US:(Vanguard|Hoogle|ETrade)' GROUP BY account "
          "HAVING max(payee) < 'I' ORDER BY account",
          ordered=True,
          notes="max(payee) is NULL for the accounts whose postings have no payee, so the comparison is NULL "
                "and the group is dropped, like a false HAVING value. Only the two accounts with payee "
                "'Hoogle' remain."),
    case3("error", "error_having_without_group_by",
          "SELECT account, count(*) AS n WHERE account ~ '^Expenses:Food' HAVING count(*) > 100", expect="error",
          notes="HAVING belongs to the GROUP BY clause, so it cannot be combined with an implicit GROUP BY "
                "(aggregates mixed with plain targets and no GROUP BY): a syntax error. Write GROUP BY account "
                "HAVING ... instead."),
    case3("error", "error_having_bare_group_key",
          "SELECT account, count(*) AS n WHERE account ~ '^Expenses:Food' GROUP BY account "
          "HAVING account ~ 'Coffee'", expect="error",
          notes="beanquery rejects a HAVING expression without an aggregate, even when it only uses group keys "
                "(SQL would accept it): 'the HAVING clause must be an aggregate expression'."),
    case3("error", "error_having_target_alias",
          "SELECT account, count(*) AS n WHERE account ~ '^Expenses:Food' GROUP BY account HAVING n > 100",
          expect="error",
          notes="HAVING is compiled against the table's columns, not against the target names, so it cannot "
                "use the alias n: 'column \"n\" not found in table \"postings\"'. Repeat the aggregate instead."),

    # --- PIVOT BY ----------------------------------------------------------
    case3("pivot", "pivot_monthly_by_category",
          "SELECT root(account, 2) AS category, month, sum(number) AS total "
          "WHERE account ~ '^Expenses:(Food|Home|Transport)' AND year = 2016 AND currency = 'USD' "
          "GROUP BY 1, 2 PIVOT BY category, month",
          ordered=True, strict_names=True,
          notes="PIVOT BY a, b runs the query, then makes one row per value of a and one column per value of b. "
                "The first column is named 'a/b' and has a's type. With a single remaining target, the value "
                "columns are named str(value of b) ('1' ... '12'), sorted by the value of b (ints numerically), "
                "with that target's type. A missing (a, b) pair is NULL (Expenses:Transport in November). Rows "
                "are sorted by a, so the result is ordered without ORDER BY."),
    case3("pivot", "pivot_inventory_cells",
          "SELECT year, root(account, 1) AS type, sum(position) AS total WHERE account ~ '^(Income|Expenses)' "
          "GROUP BY 1, 2 PIVOT BY 1, 2",
          ordered=True, strict_names=True,
          notes="PIVOT BY takes target indexes too. The cells keep the type of the pivoted target, here "
                "multi-currency inventories (USD, VACHR, IRAUSD)."),
    case3("pivot", "pivot_several_value_columns",
          "SELECT account, year, count(*) AS n, sum(number) AS total "
          "WHERE account ~ '^Expenses:Food:(Coffee|Alcohol|Groceries)' GROUP BY 1, 2 PIVOT BY account, year",
          ordered=True, strict_names=True,
          notes="With several remaining targets, each value of b gets one column per target, named "
                "'<value>/<target>' and ordered value first, then target order: 2015/n, 2015/total, 2016/n, ... "
                "Expenses:Food:Alcohol has postings only in 2016, so its other cells are NULL."),
    case3("pivot", "pivot_limit_applies_before_pivot",
          "SELECT account, year, count(*) AS n WHERE account ~ '^Expenses:Food' GROUP BY 1, 2 ORDER BY n DESC "
          "PIVOT BY year, account LIMIT 5",
          ordered=True, strict_names=True,
          notes="ORDER BY and LIMIT apply to the query BEFORE the pivot: the 5 largest (account, year) groups are "
                "kept, then pivoted. The pivoted rows are sorted by the first PIVOT BY column (year), whatever "
                "the ORDER BY, and the value columns come only from the rows that survived LIMIT (no "
                "Expenses:Food:Coffee or Alcohol column). The first PIVOT BY column may be any target, here "
                "the second one."),
    case3("pivot", "pivot_implicit_group_by_bool_keys",
          "SELECT account, number > 50 AS big, count(*) AS n WHERE account ~ '^Expenses:Food' "
          "PIVOT BY account, big",
          ordered=True, strict_names=True,
          notes="PIVOT BY works with the implicit GROUP BY (no GROUP BY clause). The value columns are named "
                "with Python's str() of the key, so boolean keys give 'False' and 'True' (not 'FALSE'/'TRUE' "
                "like str(TRUE)), sorted False first."),
    case3("pivot", "pivot_having_over_prices_table",
          "SELECT currency, year(date) AS y, max(number(amount)) AS high FROM #prices WHERE currency ~ '^V' "
          "GROUP BY 1, 2 HAVING count(*) > 40 PIVOT BY currency, y",
          ordered=True, strict_names=True,
          notes="PIVOT BY combined with FROM #prices and HAVING. 2017 has only 36 weekly prices per commodity, "
                "so HAVING drops those groups and no '2017' column is produced: the columns come from the "
                "rows left after HAVING."),
    case3("error", "error_pivot_one_column",
          "SELECT account, year, count(*) AS n WHERE account ~ '^Expenses:Food' GROUP BY 1, 2 PIVOT BY account",
          expect="error",
          notes="PIVOT BY takes exactly two columns; one or three is a syntax error."),
    case3("error", "error_pivot_expression",
          "SELECT account, year, count(*) AS n WHERE account ~ '^Expenses:Food' GROUP BY 1, 2 "
          "PIVOT BY account, year(date)", expect="error",
          notes="Each PIVOT BY column is a target name or a 1-based target index, not an expression."),
    case3("error", "error_pivot_column_not_in_targets",
          "SELECT account, year, count(*) AS n WHERE account ~ '^Expenses:Food' GROUP BY 1, 2 "
          "PIVOT BY account, payee", expect="error",
          notes="A PIVOT BY name must be a target name (or alias); a table column that is not selected is a "
                "compile error."),
    case3("error", "error_pivot_index_out_of_range",
          "SELECT account, year, count(*) AS n WHERE account ~ '^Expenses:Food' GROUP BY 1, 2 PIVOT BY 1, 4",
          expect="error"),
    case3("error", "error_pivot_same_column",
          "SELECT account, year, count(*) AS n WHERE account ~ '^Expenses:Food' GROUP BY 1, 2 PIVOT BY year, 2",
          expect="error",
          notes="Both PIVOT BY columns resolve to the target year (by name and by index): a compile error."),
    case3("error", "error_pivot_second_not_group_key",
          "SELECT account, year, count(*) AS n WHERE account ~ '^Expenses:Food' GROUP BY 1, 2 PIVOT BY account, n",
          expect="error",
          notes="The second PIVOT BY column must be a GROUP BY column, so that its values are unique per row of "
                "the first column; an aggregate target is a compile error."),

    # --- FROM #table -------------------------------------------------------
    case3("table", "entries_portable_columns",
          "SELECT type, date, year, month, day, flag, payee, narration, description, tags, links, accounts "
          "FROM #entries WHERE (year = 2016 AND type IN ('open', 'event')) OR date = 2016-11-17",
          notes="#entries has one row per directive. type is the lower-case directive name ('open', 'event', "
                "'transaction', ...). Transaction columns are NULL for other directives, and so are tags, links "
                "and description (not the empty set or ''); accounts is the set of accounts the directive "
                "refers to (empty for an event). beanquery's SELECT * FROM #entries expands to id, type, "
                "filename, lineno, date, year, month, day, flag, payee, narration, description, tags, links, "
                "meta, accounts; id (a hash), filename (an absolute path) and meta (a dict) are not portable, "
                "so this case selects the other columns explicitly."),
    case3("table", "entries_count_by_type",
          "SELECT type, count(*) AS n, min(date) AS first, max(date) AS last FROM #entries GROUP BY type "
          "ORDER BY type",
          kind=LEDGER, ordered=True,
          notes="One row per directive kind of the ledger. A balance directive is a 'balance' entry, not a "
                "transaction."),
    case3("table", "transactions_select_star",
          "SELECT * FROM #transactions WHERE 'trip-chicago-2016' IN tags AND payee ~ 'a' "
          "ORDER BY date, payee LIMIT 5",
          ordered=True, strict_names=True,
          notes="#transactions has one row per transaction. SELECT * expands to date, flag, payee, narration, "
                "tags, links, accounts (accounts is the set of posting accounts; meta is excluded from *). "
                "The table has no year/month/day, description or posting columns."),
    case3("table", "transactions_count_by_year",
          "SELECT year(date) AS y, count(*) AS n, count(payee) AS with_payee FROM #transactions GROUP BY 1 "
          "ORDER BY 1",
          kind=LEDGER, ordered=True,
          notes="year is not a column of #transactions, so the query uses year(date)."),
    case3("table", "prices_select_star",
          "SELECT * FROM #prices WHERE currency = 'GLD' ORDER BY date DESC LIMIT 5",
          ordered=True, strict_names=True,
          notes="#prices has one row per price directive. SELECT * expands to date, currency, amount (an "
                "amount)."),
    case3("table", "prices_aggregate_per_currency",
          "SELECT currency, count(*) AS n, min(date) AS first, max(date) AS last, min(number(amount)) AS low, "
          "max(number(amount)) AS high, last(amount) AS latest FROM #prices GROUP BY currency ORDER BY currency",
          ordered=True,
          notes="Aggregates over the price directives; last() follows the table order (by date)."),
    case3("table", "balances_select_star",
          "SELECT * FROM #balances WHERE year(date) = 2017 ORDER BY date, account LIMIT 6",
          kind=LEDGER, ordered=True, strict_names=True,
          notes="#balances has one row per balance directive. SELECT * expands to date, account, amount, "
                "tolerance, discrepancy. tolerance is the explicit tolerance of the directive (none in this "
                "ledger) and discrepancy the difference found by the balance check, NULL when the assertion "
                "holds (every assertion here), so the result depends on the ledger's balance checking."),
    case3("table", "balances_aggregate_per_account",
          "SELECT account, count(*) AS n, min(date) AS first, max(date) AS last, sum(amount) AS total "
          "FROM #balances GROUP BY account ORDER BY account",
          ordered=True,
          notes="sum(amount) gives an inventory; the three 0 IRAUSD assertions sum to the empty inventory."),
    case3("table", "events_natural_order",
          "SELECT * FROM #events",
          ordered=True, strict_names=True,
          notes="#events has one row per event directive. SELECT * expands to date, type, description. Without "
                "ORDER BY the rows come in ledger order (by date); the dates are all different, so the order is "
                "fully determined."),
    case3("table", "events_filter_aggregate",
          "SELECT description, count(*) AS n, max(date) AS last FROM #events WHERE type = 'location' "
          "GROUP BY description ORDER BY n DESC, description",
          ordered=True,
          notes="In #events, type is the event type and description its value."),
    case3("table", "notes_select_star",
          "SELECT * FROM #notes ORDER BY date",
          ordered=True, strict_names=True,
          notes="The ledger has no note directives, so only the columns are pinned: SELECT * expands to date, "
                "account, comment, tags, links."),
    case3("table", "documents_select_star",
          "SELECT * FROM #documents ORDER BY date",
          ordered=True, strict_names=True,
          notes="The ledger has no document directives, so only the columns are pinned: SELECT * expands to "
                "date, account, filename, tags, links."),
    case3("table", "accounts_open_close_fields",
          "SELECT account, open.date AS opened, open.currencies AS currencies, close.date AS closed "
          "FROM #accounts WHERE account ~ 'Vanguard|Chase|Opening' ORDER BY account",
          ordered=True,
          notes="#accounts has one row per account with an open or close directive. Its columns are account, "
                "open and close; open and close are the directives themselves (structured values, NULL when "
                "missing), so SELECT * is not portable and this case reads their fields with attribute "
                "access: open.date, open.currencies (the constraint currencies, NULL when the open has none) "
                "and close.date (NULL: the ledger closes no account)."),
    case3("table", "accounts_count_by_type",
          "SELECT root(account, 1) AS type, count(*) AS n FROM #accounts GROUP BY 1 ORDER BY 1",
          ordered=True,
          notes="The 60 opened accounts, including parent accounts that have their own open directive "
                "(Assets:US:BofA)."),
    case3("table", "commodities_filter_order_limit",
          "SELECT name, date FROM #commodities WHERE date < 2005-01-01 ORDER BY date DESC, name LIMIT 5",
          ordered=True,
          notes="#commodities has one row per commodity directive; the currency column is named name. "
                "beanquery's SELECT * expands to meta, date, name, and meta (a dict) is not portable, so this "
                "case selects the columns explicitly."),
    case3("table", "postings_explicit_table",
          "SELECT account, count(*) AS n, sum(position) AS balance FROM #postings WHERE account ~ '^Expenses:Food' "
          "GROUP BY account ORDER BY account",
          ordered=True,
          notes="FROM #postings names the default table explicitly; the result is the same as without FROM."),
    case3("table", "bare_table_name",
          "SELECT currency, count(*) AS n FROM prices WHERE year(date) = 2017 GROUP BY currency ORDER BY currency",
          ordered=True,
          notes="beanquery quirk: a bare FROM name that is not a column of the postings table is resolved as a "
                "table name, so FROM prices is FROM #prices. A name that is also a postings column (FROM "
                "accounts) stays a FROM expression."),
    case3("error", "error_unknown_table", "SELECT * FROM #nosuch", expect="error",
          notes="An unknown table is a compile error ('table \"nosuch\" does not exist'). Table names are "
                "case-sensitive: #Prices is unknown too."),
    case3("error", "error_table_with_open", "SELECT count(*) FROM #postings OPEN ON 2016-01-01", expect="error",
          notes="OPEN, CLOSE and CLEAR belong to the FROM expression form, so they cannot follow a #table: a "
                "syntax error."),
    case3("error", "error_table_with_close_clear", "SELECT count(*) FROM #entries CLOSE ON 2017-01-01 CLEAR",
          expect="error"),
    case3("error", "error_column_not_in_table", "SELECT account FROM #transactions", expect="error",
          notes="Columns are per table: #transactions has no account column."),

    # --- CSV of pivoted and table results ----------------------------------
    case3("csv", "csv_pivot_inventory_date_keys",
          "SELECT root(account, 1) AS type, yearmonth(date) AS month, sum(position) AS total "
          "WHERE account ~ '^(Income|Expenses):' AND year = 2017 AND month <= 3 GROUP BY 1, 2 "
          "PIVOT BY type, month",
          expect="csv", ordered=True,
          notes="Date keys name the pivoted columns as YYYY-MM-DD. numberify then splits each inventory column "
                "per currency, so the header is '2017-01-01 (USD)', ... The currency columns of each pivoted "
                "column follow the usual numberify order (rows holding the currency, then name descending)."),
    case3("csv", "csv_prices_table",
          "SELECT date, currency, amount FROM #prices WHERE date >= 2017-09-01 ORDER BY date, currency",
          expect="csv", ordered=True,
          notes="The amount column of #prices becomes 'amount (USD)'."),
    case3("csv", "csv_balances_select_star",
          "SELECT * FROM #balances WHERE account ~ 'Slate' AND year(date) = 2017 ORDER BY date LIMIT 4",
          expect="csv", kind=LEDGER, ordered=True,
          notes="SELECT * over #balances in CSV: tolerance is a decimal column of empty cells, and discrepancy "
                "is an amount column that is NULL on every row, so numberify drops it (no CSV column)."),

    # --- Phase 4 (issue #479, wave 1): beanquery 0.2.0 date functions ------------------------
    case4("date", "date_trunc_fields_on_ledger_dates",
          "SELECT DISTINCT date, date_trunc('week', date) AS week, date_trunc('month', date) AS month, "
          "date_trunc('quarter', date) AS quarter, date_trunc('year', date) AS year, "
          "date_trunc('decade', date) AS decade, date_trunc('century', date) AS century, "
          "date_trunc('millennium', date) AS millennium "
          "WHERE account ~ '^Expenses' AND year = 2016 AND (day(date) = 1 OR day(date) >= 29) ORDER BY date",
          ordered=True,
          notes="date_trunc(field, date) is the first day of the date's week (Monday, so 2016-03-01 is in the week "
                "of 2016-02-29), month, quarter, year, decade, century (1901, 2001, ...) or millennium (1001, "
                "2001, ...). The dates are the first and last days of months in 2016, a leap year."),
    case4("date", "date_trunc_edges",
          "SELECT DISTINCT date_trunc('week', 2016-01-03) AS sunday, date_trunc('week', 2016-01-04) AS monday, "
          "date_trunc('week', 2016-03-01) AS leap_week, date_trunc('month', 2016-02-29) AS leap_day, "
          "date_trunc('quarter', 2016-12-31) AS q4, date_trunc('decade', 2010-01-01) AS d2010, "
          "date_trunc('century', 2000-12-31) AS c2000, date_trunc('century', 2001-01-01) AS c2001, "
          "date_trunc('millennium', 2000-12-31) AS m2000, date_trunc('day', 2016-01-01) AS unknown, "
          "date_trunc('Week', 2016-01-01) AS upper_case WHERE account = 'Expenses:Financial:Fees'",
          notes="A Sunday belongs to the week that started on the Monday before, across a year boundary. The "
                "century of 2000 starts in 1901 and that of 2001 in 2001. An unknown field ('day') and a field "
                "in another case ('Week') are NULL. One row: DISTINCT over the postings of one account."),
    case4("date", "date_part_fields",
          "SELECT DISTINCT date_part('weekday', 2016-01-03) AS weekday, date_part('dow', 2016-01-04) AS dow, "
          "date_part('isoweekday', 2016-01-03) AS isoweekday, date_part('isodow', 2016-01-04) AS isodow, "
          "date_part('week', 2016-01-03) AS week, date_part('isoyear', 2016-01-03) AS isoyear, "
          "date_part('week', 2015-12-31) AS week_dec, date_part('month', 2016-02-29) AS month, "
          "date_part('quarter', 2016-11-03) AS quarter, date_part('year', 2016-01-03) AS year, "
          "date_part('decade', 2009-05-05) AS decade, date_part('century', 2000-01-03) AS century, "
          "date_part('millennium', 2001-01-03) AS millennium, date_part('epoch', 2016-01-03) AS epoch, "
          "date_part('epoch', 1969-12-31) AS before_epoch, date_part('day', 2016-01-03) AS unknown "
          "WHERE account = 'Expenses:Financial:Fees'",
          notes="date_part(field, date): weekday and dow count from Monday = 0, isoweekday and isodow from Monday "
                "= 1; week and isoyear are ISO 8601 (2016-01-03 is in week 53 of 2015); decade is year // 10, "
                "century and millennium start at year 1; epoch is in seconds and negative before 1970. There is "
                "no 'day' field: NULL."),
    case4("date", "date_part_on_ledger_dates",
          "SELECT date, date_part('dow', date) AS dow, date_part('isodow', date) AS isodow, "
          "date_part('week', date) AS week, date_part('isoyear', date) AS isoyear, "
          "date_part('quarter', date) AS quarter, date_part('epoch', date) AS epoch "
          "WHERE account = 'Expenses:Food:Groceries' AND date >= 2015-12-20 AND date < 2016-02-15 ORDER BY date",
          ordered=True,
          notes="date_part over the postings around a year boundary, where the ISO year and week differ from the "
                "calendar ones."),
    case4("date", "date_add_and_date_diff",
          "SELECT DISTINCT date_add(2016-02-28, 1) AS leap_day, date_add(2015-02-28, 1) AS march_first, "
          "date_add(2016-03-01, -1) AS back_to_leap_day, date_add(2016-12-31, 1) AS new_year, "
          "date_add(2016-01-31, 0) AS same, date_diff(2016-03-01, 2016-02-01) AS february, "
          "date_diff(2015-03-01, 2015-02-01) AS short_february, date_diff(2016-01-01, 2016-12-31) AS negative "
          "WHERE account = 'Expenses:Financial:Fees'",
          notes="date_add(date, days) adds days (negative days go back); date_diff(a, b) is the days from b to a."),
    case4("date", "date_add_and_diff_on_columns",
          "SELECT date, date_add(date, 30) AS later, date_add(date, -1) AS before, "
          "date_diff(date, 2016-01-01) AS since_new_year "
          "WHERE account = 'Expenses:Home:Rent' AND year = 2016 ORDER BY date",
          ordered=True),
    case4("date", "date_constructors",
          "SELECT DISTINCT date(2016, 2, 29) AS leap_day, date(2015, 2, 29) AS no_leap_day, date(2016, 13, 1) AS month_13, "
          "date(2016, 0, 1) AS month_0, date(0, 1, 1) AS year_0, date(10000, 1, 1) AS year_10000, "
          "date('2016-02-29') AS text, date('2016-2-9') AS short_text, date('2016-02- 9') AS space_day, "
          "date('016-02-09') AS short_year, date(' 2016-02-09') AS leading_space, date('2016-02-09x') AS trailing, "
          "date('20160209') AS compact, date('2015-02-29') AS invalid_text WHERE account = 'Expenses:Financial:Fees'",
          notes="date(y, m, d) and date(text) are NULL when there is no such day, or the year is outside 1 to 9999. "
                "date(text) parses like Python's strptime('%Y-%m-%d'): a four-digit year, a month and a day of one "
                "or two digits (the day may also be a space and one digit), and nothing else."),
    case4("date", "date_constructor_on_columns",
          "SELECT date, date(year, month, 1) AS first, date(year + 1, 1, 1) AS next_year "
          "WHERE account = 'Expenses:Home:Rent' AND year = 2016 AND month <= 3 ORDER BY date",
          ordered=True),

    # --- intervals ------------------------------------------------------------------------------
    case4("interval", "interval_values",
          "SELECT DISTINCT interval('1 month') AS month, interval('13 months') AS months_13, interval('-1 year') AS year, "
          "interval('+10 days') AS days, interval('1  days') AS two_spaces, interval('2 weeks') AS weeks, "
          "interval('1 Month') AS upper_case, interval(' 1 day') AS leading_space, interval('1 dayss') AS typo, "
          "interval('1 year') + interval('-1 month') AS months_11, interval('1 month') + interval('3 days') AS mixed "
          "WHERE account = 'Expenses:Financial:Fees'",
          notes="interval(text) accepts '<n> day[s]', '<n> month[s]' or '<n> year[s]' with an optional sign, and is "
                "NULL otherwise (upper case, surrounding spaces, other units). Intervals add up. The fixture encodes "
                "beanquery's relativedelta like zhang's interval text: the months as years and months with the "
                "sign of their total, then the days ('1 year 1 month', '11 months', '1 month 3 days'). "
                "ACCEPTED DEVIATION: zhang also accepts weeks (interval('2 weeks') is 14 days), which beanquery "
                "does not."),
    case4("interval", "date_interval_arithmetic",
          "SELECT DISTINCT 2016-01-31 + interval('1 month') AS leap_end, 2015-01-31 + interval('1 month') AS end, "
          "2016-03-31 - interval('1 month') AS back, interval('1 month') + 2016-01-31 AS commuted, "
          "2016-02-29 + interval('1 year') AS next_year, 2016-02-29 - interval('-1 year') AS minus_negative, "
          "2016-01-31 + (interval('1 month') + interval('1 day')) AS months_then_days, "
          "2016-01-31 - interval('1 month') - interval('1 month') AS twice, 2016-01-10 + interval('-10 days') AS days "
          "WHERE account = 'Expenses:Financial:Fees'",
          notes="Adding an interval moves by the months first, keeping the day unless the month is shorter (then "
                "its last day), then by the days. Subtracting adds the negated interval, one step at a time, so "
                "2016-01-31 minus one month twice is 2015-11-30."),
    case4("interval", "date_plus_interval_on_month_ends",
          "SELECT DISTINCT date, date + interval('1 month') AS next_month, date - interval('1 year') AS last_year, "
          "date + interval('-3 months') AS quarter_before "
          "WHERE account ~ '^Expenses' AND year = 2016 AND day(date) >= 29 ORDER BY date",
          ordered=True),
    case4("interval", "date_bin_months_and_years",
          "SELECT DISTINCT date, date_bin('1 month', date, 2016-01-01) AS month, date_bin('3 months', date, 2016-01-01) AS quarter, "
          "date_bin('1 year', date, 2015-07-01) AS fiscal_year, date_bin(interval('2 months'), date, 2015-12-01) AS two_months "
          "WHERE account ~ '^Expenses' AND year = 2016 AND day(date) >= 29 ORDER BY date",
          ordered=True,
          notes="date_bin(stride, date, origin) is the start of the bin of the date among bins of the stride laid from "
                "the origin; the stride may be an interval or its text. No date here is on a bin boundary (see "
                "date_bin_on_boundaries)."),
    case4("interval", "date_bin_on_boundaries",
          "SELECT DISTINCT date_bin('1 month', 2000-02-01, 2000-01-01) AS feb, "
          "date_bin(interval('1 month'), 2000-03-01, 2000-01-01) AS mar, date_bin('3 months', 2015-07-01, 2015-01-01) AS q3, "
          "date_bin('1 year', 2016-01-01, 2015-01-01) AS next_year, date_bin('1 month', 2015-01-01, 2015-01-01) AS origin, "
          "date_bin('1 month', 2014-12-01, 2015-01-01) AS month_before, date_bin('1 month', 2014-11-30, 2015-01-01) AS before, "
          "date_bin('7 days', 2015-01-12, 2015-01-05) AS week WHERE account = 'Expenses:Financial:Fees'",
          notes="ACCEPTED DEVIATION: with a stride of months or years, beanquery puts a date exactly on a bin boundary "
                "other than the origin into the previous bin (date_bin('1 month', 2000-02-01, 2000-01-01) is "
                "2000-01-01), while a stride of days starts a new bin there. In zhang a date on a boundary always "
                "starts its bin (2000-02-01), as date_trunc() does."),
    case4("interval", "date_bin_month_end_origin",
          "SELECT DISTINCT date_bin('1 month', 2015-03-30, 2015-01-31) AS after_drift, "
          "date_bin('1 month', 2015-03-29, 2015-01-31) AS drift_day, date_bin('1 month', 2015-03-28, 2015-01-31) AS boundary, "
          "date_bin('1 month', 2014-11-15, 2015-01-31) AS before_origin, date_bin('1 month', 2014-12-31, 2015-01-31) AS one_before, "
          "date_bin('1 month', 2015-01-31, 2015-01-31) AS origin, date_bin('1 year', 2017-02-28, 2016-02-29) AS leap_origin, "
          "date_bin('-1 month', 2015-03-30, 2015-01-31) AS negative WHERE account = 'Expenses:Financial:Fees'",
          notes="ACCEPTED DEVIATION: beanquery adds each stride to the previous bin, so from a month-end origin its bins "
                "drift (2015-01-31, 2015-02-28, 2015-03-28, ...; backwards 2014-12-31, 2014-11-30, 2014-10-30), and a "
                "date on a boundary falls in the previous bin. zhang lays the bins as origin + k strides "
                "(2015-02-28, 2015-03-31, ...; backwards 2014-11-30, 2014-10-31) and a boundary starts its bin. A "
                "negative stride is NULL in both."),
    case4("interval", "date_bin_days",
          "SELECT DISTINCT date, date_bin('7 days', date, 2015-01-05) AS week, date_bin('10 days', date, 2016-12-31) AS backwards, "
          "date_bin('1 day', date, 2000-01-01) AS day "
          "WHERE account = 'Expenses:Food:Groceries' AND date >= 2016-01-01 AND date < 2016-03-01 ORDER BY date",
          ordered=True,
          notes="A stride of days bins by whole strides from the origin, in both directions, and a date on a "
                "boundary starts its own bin (unlike strides of months)."),
    case4("interval", "date_bin_null_strides",
          "SELECT DISTINCT date_bin('-7 days', 2015-03-30, 2015-01-05) AS negative_days, "
          "date_bin(interval('2 fortnights'), 2015-03-30, 2015-01-05) AS unknown_unit, date_bin('-1 year', 2015-03-30, 2015-01-05) AS negative_year "
          "WHERE account = 'Expenses:Financial:Fees'",
          notes="A negative stride is NULL, and so is a NULL stride (interval('2 fortnights') is NULL). (beanquery rejects "
                "a NULL literal argument at compile time, since NULL has its own type there; zhang returns NULL.)"),

    # --- account and commodity directives -------------------------------------------------------
    case4("directives", "open_and_close_dates",
          "SELECT DISTINCT account, open_date(account) AS opened, close_date(account) AS closed "
          "WHERE account ~ 'Vanguard|Chase|Opening' ORDER BY account",
          ordered=True,
          notes="open_date(account) and close_date(account) read the account's open and close directives (the "
                "ledger closes no account, so close_date is NULL)."),
    case4("directives", "directive_functions_of_unknown_names",
          "SELECT DISTINCT open_date('Assets:Nope') AS opened, close_date('Assets:Nope') AS closed, "
          "open_meta('Assets:Nope', 'x') AS meta, commodity_meta('NOPE', 'name') AS commodity, "
          "open_date('assets:us:bofa:checking') AS lower_case WHERE account = 'Expenses:Financial:Fees'",
          notes="An unknown account or commodity is NULL; names are case-sensitive."),
    case4("directives", "open_meta_values",
          "SELECT DISTINCT account, open_meta(account, 'institution') AS institution, open_meta(account, 'number') AS number, "
          "open_meta(account, 'account') AS account_number, open_meta(account, 'nope') AS missing "
          "WHERE account ~ '^Assets:US:(BofA|Vanguard)' ORDER BY account",
          ordered=True,
          notes="open_meta(account, key) is a metadata value of the account's open directive, NULL when absent. "
                "Metadata is not inherited: Assets:US:BofA:Checking has no institution although Assets:US:BofA "
                "has. beanquery's one-argument open_meta(account) is a dict with filename and lineno, which zhang "
                "does not keep, so it is not covered."),
    case4("directives", "commodity_meta_values",
          "SELECT DISTINCT currency, commodity_meta(currency, 'name') AS name, currency_meta(currency, 'export') AS export, "
          "commodity_meta(currency, 'price') AS price WHERE account ~ '^Assets:US:(ETrade|Vanguard)' ORDER BY currency",
          ordered=True,
          notes="commodity_meta(currency, key), and its alias currency_meta, read the currency's commodity "
                "directive."),
    case4("directives", "directive_functions_on_tables",
          "SELECT name, commodity_meta(name, 'export') AS export, open_date('Assets:US:Vanguard:Cash') AS vanguard_cash "
          "FROM #commodities ORDER BY name",
          ordered=True,
          notes="The directive functions read the ledger, so they work on any table."),

    # --- errors -----------------------------------------------------------------------------------
    case4("error", "error_date_trunc_argument_order", "SELECT date_trunc(date, 'month')", expect="error",
          notes="date_trunc takes the field first."),
    case4("error", "error_interval_comparison",
          "SELECT DISTINCT interval('1 month') < interval('2 months') WHERE account = 'Expenses:Financial:Fees'",
          expect="error",
          notes="Intervals have no order. (beanquery also rejects = and != on intervals, which zhang accepts: see "
                "ACCEPTED_DEVIATIONS.)"),
    case4("error", "error_date_bin_int_stride", "SELECT date_bin(7, date, 2016-01-01)", expect="error"),
    case4("error", "error_open_date_of_a_date", "SELECT open_date(date)", expect="error"),
    case4("select", "offset_is_a_name",
          "SELECT date AS offset, account WHERE account ~ 'Fees' ORDER BY offset DESC LIMIT 2",
          ordered=True,
          notes="beanquery has no OFFSET, so offset is an ordinary name. zhang's OFFSET is only a keyword right after "
                "a LIMIT count, so the name keeps working."),
    case4("aggregate", "aggregate_written_twice",
          "SELECT account, sum(position) AS total, units(sum(position)) AS units, count(*) AS n, count(*) * 2 AS twice "
          "WHERE account ~ '^Expenses:Food' GROUP BY account ORDER BY account",
          ordered=True,
          notes="An aggregate written in several targets has the same value in each: zhang accumulates it once."),
    case4("aggregate", "aggregate_literal_scale",
          "SELECT str(max(number * 1.0)) AS one, str(max(number * 1.00)) AS two "
          "WHERE account ~ 'Expenses:Food' AND currency = 'USD'",
          notes="The scale of a decimal literal is part of the aggregate: the two maxima are the same number, with one "
                "and two more decimal places, so they are not the same aggregate. str() shows the scale, which the "
                "harness otherwise ignores when it compares numbers."),
    case4("date", "date_text_compared_with_a_date",
          "SELECT date, position WHERE account = 'Expenses:Home:Rent' AND (date = '2016-1-6' OR date = '2016-02- 3' "
          "OR (date >= '2016-3-6' AND date < '2016-4-5')) ORDER BY date",
          ordered=True,
          oracle_query="SELECT date, position WHERE account = 'Expenses:Home:Rent' AND (date = date('2016-1-6') "
                       "OR date = date('2016-02- 3') OR (date >= date('2016-3-6') AND date < date('2016-4-5'))) "
                       "ORDER BY date",
          notes="DELIBERATE DEVIATION FROM THE ORACLE: zhang reads a string compared with a date as a date, by the "
                "rule of date(text) (beanquery 0.2.0 rejects the comparison). The expected rows were generated "
                "from the same query with each string wrapped in date()."),
    case4("error", "error_date_text_that_date_does_not_read",
          "SELECT date WHERE account = 'Expenses:Home:Rent' AND date = ' 2016-01-06'", expect="error",
          notes="date(' 2016-01-06') is NULL (strptime takes no leading space), so the string is no date and the "
                "comparison is an error, in zhang as in beanquery."),
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
    if dtype is relativedelta:
        return "interval"
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


def enc_interval(value):
    """A relativedelta of years, months and days as zhang writes an interval: the total months as years and
    months (both with the sign of the total), then the days; zero parts are left out, zero is '0 days'."""
    if value.hours or value.minutes or value.seconds or value.microseconds or value.leapdays or value.weekday:
        raise TypeError(f"unmapped relativedelta {value!r}")
    months = value.years * 12 + value.months
    sign = -1 if months < 0 else 1
    years, months = sign * (abs(months) // 12), sign * (abs(months) % 12)

    def unit(n, name):
        return f"{n} {name}{'' if abs(n) == 1 else 's'}"
    parts = [unit(n, name) for n, name in ((years, "year"), (months, "month")) if n]
    if value.days or not parts:
        parts.append(unit(value.days, "day"))
    return " ".join(parts)


def enc_cell(value):
    if value is None:
        return None
    if isinstance(value, relativedelta):
        return enc_interval(value)
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
# CSV (see README.md, "CSV fixtures")
# ---------------------------------------------------------------------------

PLAIN_NUMBER = re.compile(r"-?[0-9]+(\.[0-9]+)?")


def csv_lines(conn, query, dcontext):
    """beanquery's numberified CSV output (as `bean-query -m -f csv`), split into lines.

    Returns (lines, quantized): `quantized` is true when numberify's rounding to the display
    precision changed a number, which a fixture must not depend on.
    """
    description, rows = run(conn, query)
    columns, numbered = numberify_results(description, rows, dcontext.build())
    _, exact = numberify_results(description, rows, None)
    quantized = any(a != b for row_a, row_b in zip(numbered, exact) for a, b in zip(row_a, row_b))
    out = io.StringIO()
    render_csv(columns, numbered, dcontext, out)
    text = out.getvalue()
    if not text.endswith("\r\n"):
        raise SystemExit(f"unexpected CSV line terminator in {text!r}")
    lines = text[:-2].split("\r\n")
    if any("\r" in line or "\n" in line for line in lines):
        raise SystemExit(f"a CSV cell contains a line break: {query}")
    return lines, quantized


def csv_canonical(lines):
    """Header and data rows with cells trimmed and plain numbers normalized (the harness rules)."""
    def cell(text):
        text = text.strip()
        return format(Decimal(text).normalize(), "f") if PLAIN_NUMBER.fullmatch(text) else text
    records = [[cell(c) for c in record] for record in csv.reader(lines)]
    return records[0], records[1:]


# ---------------------------------------------------------------------------
# Fixture writing
# ---------------------------------------------------------------------------

def dumps(value):
    return json.dumps(value, ensure_ascii=False)


def render_fixture(fixture):
    lines = ["{"]
    for key in ("name", "query", "phase", "kind", "ordered", "strict_names", "expect", "error_class"):
        if key in fixture:
            lines.append(f"  {dumps(key)}: {dumps(fixture[key])},")
    for key in ("columns", "rows", "csv"):
        if key not in fixture:
            continue
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


def synthetic_note(spec):
    """The note added to a ledger-dependent case that zhang's synthetic balance-check transactions change."""
    if spec["phase"] < 3:
        return ("Also sensitive to whether zhang's synthetic balance-check transactions (zero-amount postings) are "
                "part of the postings table.")
    return ("Also sensitive to zhang's synthetic balance-check transactions: beanquery has no such transactions "
            "(a balance directive is a balance entry), so they must not appear as transactions.")


def build_fixture(index, spec, conns, dcontext):
    base, shuffled, synthetic = conns
    query = spec["oracle_query"] or spec["query"]
    problems = []
    notes = spec["notes"]

    error_class = None
    lines = None
    if spec["expect"] == "csv":
        columns, rows = [], []
        lines, quantized = csv_lines(base, query, dcontext)
        detail = f"csv, {len(lines) - 1} rows"
        if quantized:
            problems.append("numberify's rounding to the display precision changes a number; pick other data")
        if len(lines) - 1 > MAX_ROWS:
            problems.append(f"{len(lines) - 1} rows exceeds MAX_ROWS={MAX_ROWS}")
        if " LIMIT " in f" {spec['query'].upper()} " and not spec["ordered"]:
            problems.append("LIMIT without a determining ORDER BY (mark ordered and order totally)")
        header, records = csv_canonical(lines)
        for label, conn in (("shuffled", shuffled), ("synthetic", synthetic)):
            other_header, other_records = csv_canonical(csv_lines(conn, query, dcontext)[0])
            if other_header == header and same_result(records, other_records, spec["ordered"]):
                continue
            if label == "shuffled":
                problems.append("result depends on tie-breaking between equal sort keys or same-day rows")
            elif spec["kind"] == ENGINE:
                problems.append("sensitive to zhang's synthetic balance-check postings; mark it ledger-dependent "
                                "or narrow the WHERE clause")
            else:
                extra = synthetic_note(spec)
                notes = f"{notes} {extra}".strip()
    elif spec["expect"] == "error":
        try:
            run(base, query)
        except tuple(ERROR_CLASSES) as exc:
            error_class = next(name for cls, name in ERROR_CLASSES.items() if isinstance(exc, cls))
            detail = f"{error_class}: {type(exc).__name__}: {exc}"
        except Exception as exc:
            raise SystemExit(f"case {spec['name']}: beanquery failed with {type(exc).__name__} ({exc}), which is "
                             "neither a ParseError nor a CompilationError; the oracle has no expectation for it")
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
                extra = synthetic_note(spec)
                notes = f"{notes} {extra}".strip()
    if spec["strict_names"] and spec["expect"] != "rows":
        problems.append("strict_names applies to rows fixtures only (csv headers are always compared)")
    if problems:
        raise SystemExit(f"case {spec['name']}: " + "; ".join(problems))

    fixture = {
        "name": spec["name"],
        "query": spec["query"],
        **({"phase": spec["phase"]} if spec["phase"] != 1 else {}),
        "kind": spec["kind"],
        "ordered": spec["ordered"],
        **({"strict_names": True} if spec["strict_names"] else {}),
        "expect": spec["expect"],
        **({"error_class": error_class} if error_class else {}),
        "columns": columns,
        "rows": rows,
        **({"csv": lines} if lines is not None else {}),
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
        filename, text, detail = build_fixture(index, spec, conns, options["dcontext"])
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
    by_kind = collections.Counter(spec["kind"] if spec["expect"] != "error" else "error" for spec in CASES)
    by_phase = collections.Counter(spec["phase"] for spec in CASES)
    print(f"{len(CASES)} cases; by phase: {dict(by_phase)}; by area: {dict(by_area)}; by kind: {dict(by_kind)}")
    for phase in sorted(by_phase):
        specs = [spec for spec in CASES if spec["phase"] == phase]
        counts = collections.Counter(
            "error" if spec["expect"] == "error" else f"{spec['kind']} {spec['expect']}" for spec in specs)
        print(f"  phase {phase}: {len(specs)} cases; {dict(counts)}")

    if args.table:
        print("\n| # | Case | Phase | Area | Kind | Ordered | Expect |")
        print("|---|---|---|---|---|---|---|")
        for (index, spec), detail in zip(enumerate(CASES, start=1), details):
            expect = "error" if spec["expect"] == "error" else detail
            print(f"| {index:03d} | `{spec['name']}` | {spec['phase']} | {spec['area']} | {spec['kind']} | "
                  f"{'yes' if spec['ordered'] else 'no'} | {expect} |")


if __name__ == "__main__":
    main()
