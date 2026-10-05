#!/usr/bin/env python3
"""Generate the oracle of when an account is active: where Python beancount reports a reference to an account that is
not open, as ``Invalid reference to inactive account`` or ``Invalid reference to unknown account``.

``oracle.json`` holds, for every ``*.bean`` ledger in this directory:

- ``errors``: every such error beancount reports, in the order it reports them, with the ``line`` of the directive it
  is reported on and the ``account``;
- ``other_errors``: the other errors beancount reports, none for these ledgers;
- ``accepted_deviation``: why zhang deliberately differs from beancount on the ledger, or ``null`` when it agrees.

The fixture is checked by ``extensions/beancount/tests/active_accounts.rs``.

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

HERE = os.path.dirname(os.path.abspath(__file__))
ORACLE = os.path.join(HERE, "oracle.json")

INACTIVE = re.compile(r"^Invalid reference to (?:inactive|unknown) account '([^']+)'$")

# the ledgers zhang deliberately checks differently, and why
ACCEPTED_DEVIATIONS = {}


def case(path):
    _, errors, _ = loader.load_file(path)
    found, other_errors = [], []
    for error in errors:
        inactive = INACTIVE.match(error.message)
        if inactive:
            found.append({"line": error.source["lineno"], "account": inactive.group(1)})
        else:
            other_errors.append(error.message)
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
