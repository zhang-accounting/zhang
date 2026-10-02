#!/usr/bin/env python3
"""Generate the CSV export oracle fixtures from beanquery's command line.

Each case in ``CASES`` is run with ``bean-query -q -f csv -m`` (CSV output,
numberified) over the shared ledger, and the raw output is stored verbatim in
``cases/NN_<name>.json`` as ``{"name", "query", "notes", "csv"}``. The CSV is
kept inside JSON so that its CRLF line endings survive git.

``zhang-query/tests/export.rs`` compares zhang's ``to_csv`` against these
fixtures cell by cell; see that file for the comparison rules and the
documented differences.

Usage::

    python generate.py [LEDGER]          # (re)write cases/*.json
    python generate.py --check [LEDGER]  # fail if cases/*.json are out of date

``bean-query`` is looked up next to the running interpreter (the venv of
``tests/conformance/README.md``: beancount 3.2.3, beanquery 0.2.0).
"""

import argparse
import json
import os
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
REPO_ROOT = os.path.abspath(os.path.join(HERE, "..", "..", ".."))
DEFAULT_LEDGER = os.path.join(REPO_ROOT, "integration-tests", "fava-demo-ledger", "main.zhang")
CASES_DIR = os.path.join(HERE, "cases")


def case(name, query, notes=""):
    return {"name": name, "query": query, "notes": notes}


# Every column is aliased so that header names do not depend on how either
# tool names an unaliased expression. Every query is fully ordered.
CASES = [
    case("inventory_by_root_account",
         "SELECT root(account, 1) AS root, sum(position) AS balance GROUP BY root ORDER BY root",
         notes="inventories with many cost lots per currency: units are summed per currency, costs dropped"),
    case("holdings_units_cost_and_market_value",
         "SELECT account, units(sum(position)) AS units, cost(sum(position)) AS cost, "
         "value(sum(position)) AS market WHERE account ~ '^Assets:US' GROUP BY account ORDER BY account",
         notes="Assets:US:Federal:PreTax401k sums to an empty inventory, so all its split cells are empty; "
               "market depends on the price map"),
    case("yearly_inventory_cost_and_count",
         "SELECT year, account, sum(position) AS balance, sum(cost(position)) AS cost, count(*) AS postings "
         "WHERE account ~ '^Assets:US:Vanguard' GROUP BY year, account ORDER BY year, account",
         notes="three currencies with equal counts: ties are ordered by currency name descending"),
    case("posting_positions_and_amounts",
         "SELECT date, account, position, units(position) AS units, cost(position) AS cost "
         "WHERE account ~ 'Vanguard|Vacation' AND date >= 2017-05-01 AND date <= 2017-06-05 "
         "ORDER BY date, account, number",
         notes="position and amount columns with four currencies; the cost of held lots is dropped from "
               "position and is its own amount column through cost()"),
    case("scalars_sets_and_nulls",
         "SELECT date, payee, narration, account, tags, other_accounts, number, TRUE AS t, "
         "number > 0 AS positive, NULL AS nothing, cost_number, number / 3 AS third, "
         "'a,b \"c\"' AS quoted "
         "WHERE (account = 'Assets:US:Vanguard:Cash' AND date >= 2017-08-10) "
         "OR ('trip-chicago-2016' IN tags AND account ~ '^Expenses' AND date = 2016-11-18) "
         "ORDER BY date, number, account",
         notes="NULL payees and cost numbers, empty and multi-element sets (joined by ',' and quoted), "
               "booleans, a 28-digit quotient and a literal that needs quoting"),
    case("empty_result_drops_split_columns",
         "SELECT account, sum(position) AS balance WHERE account = 'Assets:Nowhere' GROUP BY account",
         notes="no rows: the inventory column has no currency, so only the header 'account' remains"),
]


def bean_query():
    path = os.path.join(os.path.dirname(sys.executable), "bean-query")
    return path if os.path.exists(path) else "bean-query"


def run(ledger, query):
    out = subprocess.run([bean_query(), "-q", "-f", "csv", "-m", ledger, query],
                         check=True, capture_output=True)
    return out.stdout.decode("utf-8")


def fixtures(ledger):
    for idx, spec in enumerate(CASES, start=1):
        fixture = dict(spec)
        fixture["csv"] = run(ledger, spec["query"])
        filename = "{:03d}_{}.json".format(idx, spec["name"])
        yield filename, json.dumps(fixture, ensure_ascii=False, indent=2) + "\n"


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("ledger", nargs="?", default=DEFAULT_LEDGER)
    parser.add_argument("--check", action="store_true", help="fail if cases/*.json are out of date")
    args = parser.parse_args()

    expected = dict(fixtures(args.ledger))
    if args.check:
        existing = set(os.listdir(CASES_DIR)) if os.path.isdir(CASES_DIR) else set()
        stale = sorted(existing - set(expected))
        changed = []
        for filename, content in expected.items():
            path = os.path.join(CASES_DIR, filename)
            if not os.path.exists(path) or open(path, encoding="utf-8").read() != content:
                changed.append(filename)
        if stale or changed:
            print("out of date: {}; stale: {}".format(changed, stale), file=sys.stderr)
            return 1
        print("{} fixtures up to date".format(len(expected)))
        return 0

    os.makedirs(CASES_DIR, exist_ok=True)
    for filename in os.listdir(CASES_DIR):
        if filename.endswith(".json") and filename not in expected:
            os.remove(os.path.join(CASES_DIR, filename))
    for filename, content in expected.items():
        with open(os.path.join(CASES_DIR, filename), "w", encoding="utf-8", newline="\n") as file:
            file.write(content)
    print("wrote {} fixtures to {}".format(len(expected), CASES_DIR))
    return 0


if __name__ == "__main__":
    sys.exit(main())
