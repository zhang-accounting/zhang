#!/usr/bin/env python3
"""Generate the `FROM #table` oracle cases from beanquery.

The expected results in ``oracle.json`` are produced by the official Python beanquery
running each query over a ledger, not by zhang. ``tests/tables.rs`` runs the same queries
with zhang and compares (decimals numerically, sets as sets, rows as multisets unless the
case is ordered).

Ledgers:

* ``fava``: ``integration-tests/fava-demo-ledger/main.zhang``, the shared conformance ledger.
* ``extra``: ``ledger/main.zhang`` next to this script, with the directives the fava demo
  ledger has none of (notes, documents, close, custom, query, a failing balance assertion,
  a tolerance, links, '!' flags, posting flags and metadata). beancount reports two errors on it, both
  expected: the document file does not exist, and one balance assertion fails.

Usage::

    python generate.py           # (re)write oracle.json
    python generate.py --check   # verify oracle.json is up to date

Requires beancount 3.2.3 and beanquery 0.2.0 (e.g. ``/tmp/bqvenv``).
"""

import argparse
import datetime
import json
import os
import sys
from decimal import Decimal

import beanquery
from beancount import loader
from beancount.core import amount
from beanquery.sources import beancount as bq_source

HERE = os.path.dirname(os.path.abspath(__file__))
REPO_ROOT = os.path.abspath(os.path.join(HERE, "..", "..", ".."))
LEDGERS = {
    "fava": os.path.join(REPO_ROOT, "integration-tests", "fava-demo-ledger", "main.zhang"),
    "extra": os.path.join(HERE, "ledger", "main.zhang"),
}
OUTPUT = os.path.join(HERE, "oracle.json")


def case(ledger, query, ordered=False):
    return dict(ledger=ledger, query=query, ordered=ordered)


# Every beanquery table: SELECT * (or its portable columns) with an ORDER BY, and a filter or
# an aggregate. `ordered` only when the order is fully determined.
CASES = [
    # entries
    case("fava", "SELECT type, date, year, month, day, flag, payee, narration, description, tags, links, accounts "
                 "FROM #entries WHERE date >= 2017-08-20 ORDER BY date, type"),
    case("fava", "SELECT type, count(*) AS n, min(date) AS first, max(date) AS last FROM #entries GROUP BY type ORDER BY type",
         ordered=True),
    case("extra", "SELECT type, date, flag, payee, narration, description, tags, links, accounts FROM #entries", ordered=True),
    case("extra", "SELECT date, meta('source') AS source FROM #entries WHERE meta('source') IS NOT NULL", ordered=True),
    # transactions
    case("fava", "SELECT * FROM #transactions WHERE date >= 2017-08-01 ORDER BY date, payee, narration"),
    case("fava", "SELECT year(date) AS y, flag, count(*) AS n, count(payee) AS payees FROM #transactions GROUP BY 1, 2 ORDER BY 1, 2",
         ordered=True),
    case("extra", "SELECT * FROM #transactions", ordered=True),
    case("extra", "SELECT payee, narration FROM #transactions WHERE 'payroll-1' IN links OR flag = '!' ORDER BY payee", ordered=True),
    # prices
    case("fava", "SELECT * FROM #prices WHERE currency = 'VHT' AND year(date) = 2016 ORDER BY date", ordered=True),
    case("fava", "SELECT currency, count(*) AS n, min(number(amount)) AS low, max(number(amount)) AS high, last(amount) AS latest "
                 "FROM #prices GROUP BY currency ORDER BY currency", ordered=True),
    case("extra", "SELECT * FROM #prices ORDER BY date, currency(amount)", ordered=True),
    # balances
    case("fava", "SELECT * FROM #balances WHERE account ~ 'Checking' AND year(date) = 2016 ORDER BY date", ordered=True),
    case("fava", "SELECT account, count(*) AS n, last(amount) AS latest FROM #balances GROUP BY account ORDER BY account",
         ordered=True),
    case("extra", "SELECT * FROM #balances ORDER BY date", ordered=True),
    case("extra", "SELECT date, discrepancy FROM #balances WHERE discrepancy IS NOT NULL", ordered=True),
    # notes
    case("extra", "SELECT * FROM #notes ORDER BY date", ordered=True),
    case("extra", "SELECT account, count(*) AS n FROM #notes WHERE comment ~ 'bank|less' GROUP BY account ORDER BY account",
         ordered=True),
    # events
    case("fava", "SELECT * FROM #events ORDER BY date", ordered=True),
    case("fava", "SELECT type, count(*) AS n, max(date) AS last FROM #events GROUP BY type ORDER BY type", ordered=True),
    case("extra", "SELECT * FROM #events WHERE type = 'location' ORDER BY date", ordered=True),
    # documents (the filename is an absolute path, compared in tests/tables.rs instead)
    case("extra", "SELECT date, account, tags, links FROM #documents ORDER BY date", ordered=True),
    case("extra", "SELECT account, count(*) AS n FROM #documents GROUP BY account", ordered=True),
    case("extra", "SELECT date, comment FROM #notes WHERE 't1' IN tags AND 'ln' IN links", ordered=True),
    case("extra", "SELECT type, tags, links FROM #entries WHERE type IN ('note', 'document') ORDER BY date", ordered=True),
    # accounts
    case("fava", "SELECT account, open.date, open.currencies, close.date FROM #accounts ORDER BY account", ordered=True),
    case("fava", "SELECT account, open.date FROM #accounts", ordered=True),
    case("fava", "SELECT root(account, 1) AS root, count(*) AS n, min(open.date) AS first FROM #accounts GROUP BY 1 ORDER BY 1",
         ordered=True),
    case("extra", "SELECT account, open.date, open.currencies, close.date FROM #accounts", ordered=True),
    case("extra", "SELECT account FROM #accounts WHERE close.date IS NOT NULL", ordered=True),
    # commodities
    case("fava", "SELECT date, name FROM #commodities ORDER BY date, name", ordered=True),
    case("fava", "SELECT name, meta('name') AS title FROM #commodities WHERE meta('export') ~ 'NYSE' ORDER BY name", ordered=True),
    case("extra", "SELECT date, name, meta('name') AS title FROM #commodities", ordered=True),
    # the postings table, named explicitly
    case("extra", "SELECT account, sum(units(position)) AS balance FROM #postings GROUP BY account ORDER BY account", ordered=True),
    # the flag of the posting itself, NULL when it has none
    case("extra", "SELECT date, flag, posting_flag, account, number FROM #postings ORDER BY date, account, number", ordered=True),
    case("extra", "SELECT posting_flag, count(*) AS n FROM #postings GROUP BY posting_flag ORDER BY posting_flag", ordered=True),
    case("extra", "SELECT payee, account FROM #postings WHERE posting_flag = '!' OR posting_flag IS NULL AND flag = '!' "
                  "ORDER BY payee, account", ordered=True),
]


def column_type(dtype):
    if dtype is bool:
        return "bool"
    if dtype is int:
        return "int"
    if dtype is Decimal:
        return "decimal"
    if dtype is str or dtype is object:
        return "str"
    if dtype is datetime.date:
        return "date"
    if dtype is amount.Amount:
        return "amount"
    name = str(dtype)
    if dtype in (set, frozenset, list) or "Set" in name or "frozenset" in name or "list" in name:
        return "set"
    if getattr(dtype, "__name__", "") == "Inventory":
        return "inventory"
    raise TypeError(f"unmapped beanquery datatype {dtype!r}")


def encode(value):
    if value is None:
        return None
    if isinstance(value, bool) or isinstance(value, int):
        return value
    if isinstance(value, Decimal):
        return format(value, "f")
    if isinstance(value, str):
        return value
    if isinstance(value, datetime.date):
        return value.isoformat()
    if isinstance(value, (set, frozenset, list)):
        return sorted(value)
    if isinstance(value, amount.Amount):
        return {"number": format(value.number, "f"), "currency": value.currency}
    if hasattr(value, "get_positions"):
        positions = []
        for position in value.get_positions():
            if position.cost is not None:
                raise TypeError("inventories at cost are not encoded")
            positions.append({"number": format(position.units.number, "f"), "currency": position.units.currency})
        return sorted(positions, key=lambda it: it["currency"])
    raise TypeError(f"unmapped value {value!r}")


def run(conn, query):
    cursor = conn.execute(query)
    columns = [{"name": column.name, "type": column_type(column.datatype)} for column in cursor.description]
    rows = [[encode(cell) for cell in row] for row in cursor.fetchall()]
    return columns, rows


def generate():
    connections = {}
    for name, path in LEDGERS.items():
        entries, errors, options = loader.load_file(path)
        if name == "fava" and errors:
            sys.exit(f"{path}: unexpected beancount errors: {errors}")
        conn = beanquery.Connection()
        bq_source.attach(conn, "beancount:", entries=entries, errors=errors, options=options)
        connections[name] = conn
    cases = []
    for spec in CASES:
        columns, rows = run(connections[spec["ledger"]], spec["query"])
        cases.append(dict(spec, columns=columns, rows=rows))
    return {"generator": "beancount 3.2.3, beanquery 0.2.0", "cases": cases}


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--check", action="store_true", help="fail if oracle.json is out of date")
    args = parser.parse_args()
    text = json.dumps(generate(), indent=1, ensure_ascii=False) + "\n"
    if args.check:
        with open(OUTPUT, encoding="utf-8") as current:
            if current.read() != text:
                sys.exit("oracle.json is out of date; run generate.py")
        print("oracle.json is up to date")
        return
    with open(OUTPUT, "w", encoding="utf-8") as output:
        output.write(text)
    print(f"wrote {len(CASES)} cases to {OUTPUT}")


if __name__ == "__main__":
    main()
