#!/usr/bin/env python3
"""Generate the oracle of the commodities an ``open`` lists: where Python beancount reports a posting or a balance
assertion in a commodity that the ``open`` of its account does not list.

``oracle.json`` holds, for every ``*.bean`` ledger in this directory:

- ``errors``: every such error beancount reports, in the order it reports them, with the ``line`` of the directive
  it is reported on (a ``pad`` for its padding transaction), the ``account`` and the ``currency``. Beancount reports
  ``Invalid currency X for account 'A'`` for a posting and ``Invalid currency 'X' for Balance directive`` for an
  assertion;
- ``other_errors``: the other errors beancount reports, none for these ledgers;
- ``accepted_deviation``: why zhang deliberately differs from beancount on the ledger, or ``null`` when it agrees.

The fixture is checked by ``extensions/beancount/tests/open_commodities.rs``.

Oracle version used: beancount 3.2.3 (Python 3.9).

Usage::

    python generate.py          # (re)write oracle.json
    python generate.py --check  # verify oracle.json is up to date
"""

import argparse
import glob
import json
import os
import re
import sys

from beancount import loader
from beancount.core import data

HERE = os.path.dirname(os.path.abspath(__file__))
ORACLE = os.path.join(HERE, "oracle.json")

POSTING = re.compile(r"^Invalid currency (\S+) for account '([^']+)'$")
BALANCE = re.compile(r"^Invalid currency '(\S+)' for Balance directive")

# the ledgers zhang deliberately checks differently, and why
ACCEPTED_DEVIATIONS = {
    "split_reduction": (
        "a posting is reported once as written: beancount reports a reduction booked against several lots once for "
        "each lot"
    ),
}


def case(path):
    _, errors, _ = loader.load_file(path)
    found, other_errors = [], []
    for error in errors:
        posting = POSTING.match(error.message)
        balance = BALANCE.match(error.message)
        if posting:
            currency, account = posting.groups()
        elif balance and isinstance(error.entry, data.Balance):
            currency, account = balance.group(1), error.entry.account
        else:
            other_errors.append(error.message)
            continue
        found.append({"line": error.source["lineno"], "account": account, "currency": currency})
    name = os.path.basename(path)[: -len(".bean")]
    return {
        "errors": found,
        "other_errors": other_errors,
        "accepted_deviation": ACCEPTED_DEVIATIONS.get(name),
    }


def oracle():
    cases = {os.path.basename(path)[: -len(".bean")]: case(path) for path in sorted(glob.glob(os.path.join(HERE, "*.bean")))}
    unknown = set(ACCEPTED_DEVIATIONS) - set(cases)
    if unknown:
        sys.exit("accepted deviations of unknown ledgers: {}".format(sorted(unknown)))
    return cases


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
