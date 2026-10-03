#!/usr/bin/env python3
"""Generate the oracle of balance assertions and pads: what Python beancount makes of the
ledgers in this directory.

``oracle.json`` holds, for every ``*.bean`` ledger:

- ``balances``: the units of every account in every currency once the ledger is loaded, the
  sum of the postings of that account alone (not of its sub-accounts), zeros left out;
- ``pads``: the padding transactions, each with the padded account, the units it books there
  and the account it pads from, in ledger order;
- ``assertions``: every ``balance`` directive in ledger order, with the balance beancount
  checked it against (the account and its sub-accounts) and whether it passed;
- ``errors``: the other errors beancount reports, such as an unused pad.

Numbers are written as ``str(Decimal)``. The fixture is checked by
``extensions/beancount/tests/balance_assertions.rs``.

Oracle version used: beancount 3.2.3 (Python 3.9).

Usage::

    python generate.py          # (re)write oracle.json
    python generate.py --check  # verify oracle.json is up to date
"""

import argparse
import collections
import glob
import json
import os
import sys

from beancount import loader
from beancount.core import data
from beancount.core.interpolate import BalanceError
from beancount.ops.balance import get_balance_tolerance

HERE = os.path.dirname(os.path.abspath(__file__))
ORACLE = os.path.join(HERE, "oracle.json")


def amount(number, currency):
    return {"number": str(number), "currency": currency}


def case(path):
    entries, errors, options = loader.load_file(path)
    other_errors = [error.message for error in errors if not isinstance(error, BalanceError)]

    # running units per (account, currency), postings of the account alone
    units = collections.defaultdict(lambda: collections.defaultdict(lambda: 0))
    pads, assertions = [], []
    for entry in entries:
        if isinstance(entry, data.Transaction):
            for posting in entry.postings:
                units[posting.account][posting.units.currency] += posting.units.number
            if entry.flag == "P":
                padded, source = entry.postings
                pads.append({"account": padded.account, "units": amount(padded.units.number, padded.units.currency), "from": source.account})
        elif isinstance(entry, data.Balance):
            currency = entry.amount.currency
            # beancount checks the account and its sub-accounts
            balance = sum(
                (held[currency] for account, held in units.items() if account == entry.account or account.startswith(entry.account + ":")),
                0,
            )
            # beancount sets `diff_amount` on a failing assertion
            passed = entry.diff_amount is None
            assert passed == (abs(balance - entry.amount.number) <= get_balance_tolerance(entry, options)), entry
            assertions.append(
                {
                    "date": str(entry.date),
                    "account": entry.account,
                    "amount": amount(entry.amount.number, currency),
                    "balance": amount(balance, currency),
                    "passed": passed,
                }
            )
    balances = {
        account: {currency: str(number) for currency, number in sorted(held.items()) if number != 0}
        for account, held in sorted(units.items())
    }
    balances = {account: held for account, held in balances.items() if held}
    return {"balances": balances, "pads": pads, "assertions": assertions, "errors": other_errors}


def oracle():
    return {os.path.basename(path)[: -len(".bean")]: case(path) for path in sorted(glob.glob(os.path.join(HERE, "*.bean")))}


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
