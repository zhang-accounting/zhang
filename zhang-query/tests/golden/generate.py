"""Regenerate the beanquery oracle results used by tests/golden.rs.

Usage (needs `pip install beancount beanquery`):

    python zhang-query/tests/golden/generate.py > zhang-query/tests/golden/fava_demo.json

Values are encoded like the `POST /api/query` JSON: decimals as strings, dates as
YYYY-MM-DD, sets as sorted arrays, amounts/positions/inventories as objects.
"""
import datetime
import decimal
import json
import os
import sys

import beanquery
from beancount.core.amount import Amount
from beancount.core.inventory import Inventory
from beancount.core.position import Position

HERE = os.path.dirname(os.path.abspath(__file__))
LEDGER = os.path.join(HERE, "..", "..", "..", "integration-tests", "fava-demo-ledger", "main.zhang")

QUERIES = [
    'SELECT year, month, root(account, 2), sum(position) WHERE account ~ "^Expenses" GROUP BY 1, 2, 3 ORDER BY 1, 2, 3',
    'SELECT payee, sum(cost(position)) AS total WHERE account ~ "^Expenses" GROUP BY payee ORDER BY total DESC LIMIT 20',
    "SELECT date, payee, account, position WHERE 'trip-chicago-2016' IN tags",
    'SELECT account, units(sum(position)) AS qty, cost(sum(position)) AS book, convert(units(sum(position)), "USD") AS market '
    'WHERE account ~ "^Assets" GROUP BY account ORDER BY account',
    'SELECT count(*), sum(number) WHERE account ~ "Expenses:Food"',
]

TYPE_NAMES = {"Decimal": "decimal", "Inventory": "inventory", "Position": "position", "Amount": "amount", "NoneType": "null"}


def amount(value):
    return {"number": str(value.number), "currency": value.currency}


def position(value):
    cost = value.cost
    return {
        "units": amount(value.units),
        "cost": None
        if cost is None
        else {
            "number": str(cost.number),
            "currency": cost.currency,
            "date": cost.date.isoformat() if cost.date else None,
            "label": cost.label,
        },
    }


def encode(value):
    if value is None or isinstance(value, (bool, int, str)):
        return value
    if isinstance(value, decimal.Decimal):
        return str(value)
    if isinstance(value, datetime.date):
        return value.isoformat()
    # Amount and Position are named tuples: test them before generic sequences
    if isinstance(value, Amount):
        return amount(value)
    if isinstance(value, Position):
        return position(value)
    if isinstance(value, Inventory):
        return {"positions": [position(p) for p in value]}
    if isinstance(value, (set, frozenset, list, tuple)):
        return sorted(value)
    raise TypeError(type(value))


def main():
    connection = beanquery.connect("beancount:" + LEDGER)
    cases = []
    for query in QUERIES:
        cursor = connection.execute(query)
        columns = [
            {"name": column.name, "type": TYPE_NAMES.get(column.datatype.__name__, column.datatype.__name__)}
            for column in cursor.description
        ]
        rows = [[encode(cell) for cell in row] for row in cursor.fetchall()]
        cases.append({"query": query, "columns": columns, "rows": rows})
    json.dump(cases, sys.stdout, indent=1, ensure_ascii=False)
    sys.stdout.write("\n")


if __name__ == "__main__":
    main()
