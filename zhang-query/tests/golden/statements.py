"""Regenerate the beanquery oracle results used by tests/statements.rs: the BALANCES and
JOURNAL statements and the running `balance` column over the fava demo ledger.

Usage (needs `pip install beancount==3.2.3 beanquery==0.2.0`):

    python zhang-query/tests/golden/statements.py > zhang-query/tests/golden/statements.json

Values are encoded as in generate.py (the `POST /api/query` JSON encoding), one row per line.
"""
import json
import os
import sys

import beanquery

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from generate import LEDGER, TYPE_NAMES, encode  # noqa: E402

QUERIES = [
    "BALANCES",
    "BALANCES AT cost FROM year <= 2016 WHERE account ~ '^(Assets|Liabilities)'",
    "BALANCES AT value WHERE account ~ 'Vanguard|ETrade'",
    "JOURNAL 'Checking' FROM year = 2016",
    # 'Eating out ' narrations: JOURNAL collapses their whitespace with maxwidth()
    "JOURNAL 'Restaurant' FROM year = 2017 AND month <= 3",
    "JOURNAL 'ETrade:GLD' AT cost",
    'JOURNAL "ETrade" AT units FROM year = 2016',
    # the running balance is accumulated before ORDER BY sorts the rows
    "SELECT date, position, balance WHERE account ~ 'Checking' ORDER BY date DESC LIMIT 10",
    # ... and before GROUP BY, over every row WHERE selects
    "SELECT account, last(balance), count(*) WHERE account ~ 'ETrade:(GLD|VEA)' GROUP BY account ORDER BY account",
]


def main():
    connection = beanquery.connect("beancount:" + LEDGER)
    out = sys.stdout
    out.write("[\n")
    for idx, query in enumerate(QUERIES):
        cursor = connection.execute(query)
        columns = [
            {"name": column.name, "type": TYPE_NAMES.get(column.datatype.__name__, column.datatype.__name__)}
            for column in cursor.description
        ]
        rows = [[encode(cell) for cell in row] for row in cursor.fetchall()]
        out.write(' {"query": %s,\n' % json.dumps(query, ensure_ascii=False))
        out.write('  "columns": %s,\n' % json.dumps(columns, ensure_ascii=False))
        out.write('  "rows": [\n')
        out.write(",\n".join("   " + json.dumps(row, ensure_ascii=False) for row in rows))
        out.write("\n  ]}%s\n" % ("," if idx + 1 < len(QUERIES) else ""))
    out.write("]\n")


if __name__ == "__main__":
    main()
