#!/usr/bin/env python3
"""Generate the oracle fixtures of HAVING and PIVOT BY.

The expected results in ``cases/*.json`` come from the official Python beanquery over the
shared ledger, exactly like the conformance suite in ``../conformance``: this script reuses
that generator's ledger loading, validation (determinism, synthetic balance-check re-runs,
CSV rounding), encoding and fixture format, and only holds its own case list. The fixtures
are checked by ``zhang-query/tests/having_pivot.rs``, which also compares column names: the
names of pivoted columns are data.

The cases avoid what beanquery does not define: HAVING conditions that read a column
outside an aggregate (beanquery evaluates it on an arbitrary posting), NULL values in a
PIVOT BY column (beanquery cannot sort them) and missing pivot cells in CSV cases
(beanquery's numberify fails on them).

Usage::

    python generate.py [LEDGER]          # (re)write cases/*.json
    python generate.py --check [LEDGER]  # verify cases/*.json are up to date
"""

import argparse
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, "..", "conformance"))

import generate as conformance  # noqa: E402  (the conformance generator)
from generate import case  # noqa: E402

CASES_DIR = os.path.join(HERE, "cases")

CASES = [
    # --- HAVING --------------------------------------------------------------
    case("having", "having_categories_over_1000",
         "SELECT root(account, 2) AS category, sum(number) AS total WHERE account ~ '^Expenses' "
         "GROUP BY 1 HAVING sum(number) > 1000 ORDER BY 1",
         ordered=True,
         notes="HAVING keeps the groups whose aggregate passes; it is evaluated per finished group."),
    case("having", "having_aggregate_not_selected_with_limit",
         "SELECT account, sum(position) AS total WHERE account ~ '^Expenses:Food' "
         "GROUP BY account HAVING count(*) > 10 ORDER BY account LIMIT 2",
         ordered=True,
         notes="HAVING may use an aggregate that is not a target. LIMIT applies after HAVING."),
    case("having", "having_three_valued_logic",
         "SELECT account, count(*) AS n, max(number) AS largest WHERE account ~ '^Expenses:(Food|Home)' "
         "GROUP BY account HAVING NOT (sum(number) > 5000) OR max(number) / 0 > 1 ORDER BY account",
         ordered=True,
         notes="A group whose condition is NULL (here the division by zero) is dropped, like FALSE."),
    case("having", "having_keeps_late_groups",
         "SELECT year, count(*) AS n WHERE account ~ '^Expenses:Food' GROUP BY year HAVING count(*) < 180 LIMIT 2",
         ordered=True,
         notes="Without ORDER BY, groups come in the order of their first posting, and LIMIT keeps the first "
               "groups that pass HAVING: 2016 fails it and does not count, so 2017 is kept."),
    case("having", "having_with_order_by_aggregate",
         "SELECT year, root(account, 2) AS category, count(*) AS n WHERE account ~ '^Expenses' "
         "GROUP BY 1, 2 HAVING count(*) >= 100 ORDER BY sum(number) DESC LIMIT 4",
         ordered=True,
         notes="ORDER BY and LIMIT apply to the groups HAVING keeps."),

    # --- PIVOT BY ------------------------------------------------------------
    case("pivot", "pivot_category_by_year",
         "SELECT root(account, 2) AS category, year, sum(position) AS total WHERE account ~ '^Expenses' "
         "GROUP BY 1, 2 PIVOT BY category, year",
         ordered=True,
         notes="One row per value of the first column, sorted; one column per value of the second, sorted and "
               "named after the value. The first column is named '<first>/<second>'."),
    case("pivot", "pivot_monthly_food_by_year",
         "SELECT month, year, sum(number) AS total WHERE account ~ '^Expenses:Food' GROUP BY month, year "
         "PIVOT BY month, year",
         ordered=True,
         notes="A missing (month, year) pair is NULL: the ledger ends in September 2017."),
    case("pivot", "pivot_two_value_columns_by_index",
         "SELECT year, root(account, 2) AS category, count(*) AS n, sum(number) AS total "
         "WHERE account ~ '^Expenses:(Food|Home|Transport)' GROUP BY 1, 2 PIVOT BY 1, 2",
         ordered=True,
         notes="With several other targets each value gets one column per target, named '<value>/<target>'. "
               "Columns may be given by their 1-based index."),
    case("pivot", "pivot_after_having_order_and_limit",
         "SELECT year, root(account, 2) AS category, sum(number) AS total WHERE account ~ '^Expenses' "
         "GROUP BY 1, 2 HAVING sum(number) > 1000 ORDER BY 3 DESC PIVOT BY category, year LIMIT 8",
         ordered=True,
         notes="PIVOT BY reshapes the rows LIMIT leaves; it sorts them itself, so ORDER BY only decides which "
               "rows remain."),
    case("pivot", "pivot_last_row_of_a_pair_wins",
         "SELECT year, root(account, 2) AS category, month, count(*) AS n WHERE account ~ '^Expenses:(Food|Home)' "
         "GROUP BY 1, 2, 3 ORDER BY month DESC, year, category PIVOT BY year, category",
         ordered=True,
         notes="When several rows share a (first, second) pair, the last one in result order fills the cells: "
               "here the earliest month of each year."),
    case("pivot", "pivot_boolean_column_names",
         "SELECT root(account, 2) AS category, payee IS NULL AS no_payee, count(*) AS n "
         "WHERE account ~ '^Expenses' GROUP BY 1, 2 PIVOT BY category, no_payee",
         ordered=True,
         notes="Columns are named with Python's str() of the value: False and True."),
    case("pivot", "csv_pivot_inventory_columns",
         "SELECT account, year, sum(position) AS total "
         "WHERE account ~ '^Assets:US:(BofA:Checking|Vanguard:Cash|Hoogle:Vacation)' GROUP BY 1, 2 "
         "PIVOT BY account, year",
         expect="csv", ordered=True,
         notes="numberify splits each pivoted inventory column per currency: '2015 (USD)', '2015 (VACHR)'."),

    # --- errors ----------------------------------------------------------------
    case("error", "error_having_without_group_by",
         "SELECT sum(number) AS total WHERE account ~ '^Expenses' HAVING sum(number) > 10", expect="error",
         notes="HAVING is part of the GROUP BY clause."),
    case("error", "error_having_without_aggregate",
         "SELECT account, count(*) AS n GROUP BY account HAVING account ~ 'Food'", expect="error",
         notes="HAVING must use an aggregate function."),
    case("error", "error_having_target_name",
         "SELECT account, sum(number) AS total GROUP BY account HAVING total > 10", expect="error",
         notes="Names in HAVING are postings columns, never target aliases."),
    case("error", "error_pivot_second_column_not_grouped",
         "SELECT account, year, sum(number) AS total GROUP BY 1, 2 PIVOT BY account, total", expect="error"),
    case("error", "error_pivot_column_not_a_target",
         "SELECT account, year, sum(number) AS total GROUP BY 1, 2 PIVOT BY account, month", expect="error"),
    case("error", "error_pivot_same_column_twice",
         "SELECT account, year, sum(number) AS total GROUP BY 1, 2 PIVOT BY account, 1", expect="error"),
    case("error", "error_pivot_index_out_of_range",
         "SELECT account, year, sum(number) AS total GROUP BY 1, 2 PIVOT BY 1, 4", expect="error"),
    case("error", "error_pivot_three_columns",
         "SELECT account, year, sum(number) AS total GROUP BY 1, 2 PIVOT BY account, year, total", expect="error"),
    case("error", "error_pivot_expression",
         "SELECT account, year, sum(number) AS total GROUP BY 1, 2 PIVOT BY account, year + 1", expect="error"),
    case("error", "error_pivot_after_limit",
         "SELECT account, year, sum(number) AS total GROUP BY 1, 2 LIMIT 3 PIVOT BY account, year", expect="error"),
]


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("ledger", nargs="?", default=conformance.DEFAULT_LEDGER)
    parser.add_argument("--check", action="store_true", help="verify fixtures instead of writing them")
    args = parser.parse_args()

    entries, options = conformance.load_ledger(args.ledger)
    conns = (conformance.connect(entries, options),
             conformance.connect(conformance.perturbed(entries), options),
             conformance.connect(conformance.with_synthetic_balance_checks(entries), options))
    print(f"oracle: beancount {conformance.beancount.__version__}, beanquery {conformance.beanquery.__version__}")

    outputs = {}
    for index, spec in enumerate(CASES, start=1):
        filename, text, detail = conformance.build_fixture(index, spec, conns, options["dcontext"])
        outputs[filename] = text
        print(f"  {filename:<55} {spec['kind']:<17} {detail}")

    os.makedirs(CASES_DIR, exist_ok=True)
    existing = {name for name in os.listdir(CASES_DIR) if name.endswith(".json")}
    if args.check:
        changed = [name for name, text in outputs.items()
                   if not os.path.exists(os.path.join(CASES_DIR, name))
                   or open(os.path.join(CASES_DIR, name), encoding="utf-8").read() != text]
        stale = sorted(existing - set(outputs))
        if changed or stale:
            sys.exit(f"fixtures out of date: changed={changed} stale={stale}")
        print("fixtures are up to date")
    else:
        for name in existing - set(outputs):
            os.remove(os.path.join(CASES_DIR, name))
        for name, text in outputs.items():
            with open(os.path.join(CASES_DIR, name), "w", encoding="utf-8") as handle:
                handle.write(text)
    print(f"{len(CASES)} cases")


if __name__ == "__main__":
    main()
