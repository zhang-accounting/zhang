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


def case(area, name, query, kind=ENGINE, ordered=False, expect="rows", notes="", oracle_query=None, phase=1):
    return dict(area=area, name=name, query=query, kind=kind, ordered=ordered, expect=expect,
                notes=notes, oracle_query=oracle_query, phase=phase)


def case2(area, name, query, **kwargs):
    """A Phase 2 case (issue #434): BALANCES, JOURNAL, the balance column, FROM OPEN/CLOSE/CLEAR, CSV."""
    return case(area, name, query, phase=2, **kwargs)


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
    for key in ("name", "query", "phase", "kind", "ordered", "expect", "error_class"):
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
                extra = ("Also sensitive to whether zhang's synthetic balance-check transactions (zero-amount "
                         "postings) are part of the postings table.")
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
                extra = ("Also sensitive to whether zhang's synthetic balance-check transactions (zero-amount "
                         "postings) are part of the postings table.")
                notes = f"{notes} {extra}".strip()
    if problems:
        raise SystemExit(f"case {spec['name']}: " + "; ".join(problems))

    fixture = {
        "name": spec["name"],
        "query": spec["query"],
        **({"phase": spec["phase"]} if spec["phase"] != 1 else {}),
        "kind": spec["kind"],
        "ordered": spec["ordered"],
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
