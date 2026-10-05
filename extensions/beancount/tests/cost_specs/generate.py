#!/usr/bin/env python3
"""Generate the oracle of the cost spec forms of #497: how Python beancount books every transaction
of ``ledger.bean`` (a cost with only a date or only a label, components in any order, the compound
cost ``{P # T USD}`` and the merge-cost marker ``{*}``), what it reports, and the lots it ends with.

``oracle.json`` holds ``transactions`` (every transaction in file order with its line, date,
narration and booked postings: account, units, and the cost of the lot or ``null``), ``errors``
(the line and message of every error beancount reports; only "Cost merging is not supported yet"
is expected) and ``positions`` (the lots held at cost per account at the end, in inventory order).
Numbers are plain decimal strings. The fixture is checked by ``extensions/beancount/tests/cost_specs.rs``.

Oracle version used: beancount 3.2.3 (Python 3.9).

Usage::

    python generate.py          # (re)write oracle.json
    python generate.py --check  # verify oracle.json is up to date
"""

import argparse
import collections
import json
import os
import sys

from beancount import loader
from beancount.core import data, inventory

HERE = os.path.dirname(os.path.abspath(__file__))
LEDGER = os.path.join(HERE, "ledger.bean")
ORACLE = os.path.join(HERE, "oracle.json")

EXPECTED_ERRORS = {"Cost merging is not supported yet"}


def number(value):
    return format(value, "f")


def amount(value):
    return {"number": number(value.number), "currency": value.currency}


def cost(value):
    if value is None:
        return None
    return {"number": number(value.number), "currency": value.currency, "date": str(value.date), "label": value.label}


def oracle():
    entries, errors, _ = loader.load_file(LEDGER)
    unexpected = [error.message for error in errors if error.message not in EXPECTED_ERRORS]
    if unexpected:
        sys.exit("beancount reports unexpected errors: {}".format(unexpected))
    transactions = [entry for entry in entries if isinstance(entry, data.Transaction)]
    transactions.sort(key=lambda entry: entry.meta["lineno"])

    inventories = collections.defaultdict(inventory.Inventory)
    for entry in transactions:
        for posting in entry.postings:
            inventories[posting.account].add_position(posting)
    positions = {
        account: [{"units": amount(position.units), "cost": cost(position.cost)} for position in inv]
        for account, inv in sorted(inventories.items())
        if any(position.cost is not None for position in inv)
    }

    return {
        "transactions": [
            {
                "lineno": entry.meta["lineno"],
                "date": str(entry.date),
                "narration": entry.narration,
                "postings": [
                    {"account": posting.account, "units": amount(posting.units), "cost": cost(posting.cost)}
                    for posting in entry.postings
                ],
            }
            for entry in transactions
        ],
        "errors": [{"lineno": error.source["lineno"], "message": error.message} for error in errors],
        "positions": positions,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--check", action="store_true", help="verify oracle.json is up to date")
    args = parser.parse_args()
    text = json.dumps(oracle(), indent=2, ensure_ascii=False, sort_keys=True) + "\n"
    if args.check:
        with open(ORACLE, encoding="utf-8") as file:
            if file.read() != text:
                sys.exit("oracle.json is out of date: run generate.py")
        print("oracle.json is up to date")
        return
    with open(ORACLE, "w", encoding="utf-8") as file:
        file.write(text)
    print("wrote {}".format(ORACLE))


if __name__ == "__main__":
    main()
