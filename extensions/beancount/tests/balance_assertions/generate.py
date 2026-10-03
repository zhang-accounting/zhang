#!/usr/bin/env python3
"""Generate the oracle of balance assertions and pads: what Python beancount makes of the
ledgers in this directory (a ledger may include files from a sub-directory).

``oracle.json`` holds, for every ``*.bean`` ledger:

- ``balances``: the units of every account in every currency once the ledger is loaded, the
  sum of the postings of that account alone (not of its sub-accounts), zeros left out;
- ``pads``: the padding transactions, each with its date, the padded account, the units it
  books there and the account it pads from, in ledger order;
- ``assertions``: every ``balance`` directive in ledger order, with the balance beancount
  checked it against (the account and its sub-accounts) and whether it passed;
- ``unused_pads``: the date and account of every ``pad`` beancount reports as unused;
- ``pads_with_cost``: the date and account of every ``balance`` whose pad beancount reports as
  padding a commodity held at cost;
- ``errors``: the other errors beancount reports;
- ``accepted_deviation``: why zhang deliberately differs from beancount on the ledger, or
  ``null`` when it agrees. Such a ledger is checked against zhang's own rules instead.

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
from beancount.ops.pad import PadError

HERE = os.path.dirname(os.path.abspath(__file__))
ORACLE = os.path.join(HERE, "oracle.json")

NO_INFERRED_TOLERANCE = (
    "zhang never infers a tolerance: an assertion without `~` must match exactly, and a pad brings the "
    "account to exactly the asserted amount. Beancount tolerates one unit of the last decimal place of "
    "the asserted amount, and pads nothing within it"
)

# the ledgers zhang deliberately checks differently, and why
ACCEPTED_DEVIATIONS = {
    "child_assertion_after_parent_pad": (
        "a pad serves the assertions on its own account only: beancount also lets an assertion on a sub-account "
        "use up the pad of its parent account, which then pads nothing for the parent's own assertion"
    ),
    "inferred_tolerance": NO_INFERRED_TOLERANCE,
    "nested_pads": (
        "a pad is sized from the balance with every padding before it: beancount sizes the pad of a parent account "
        "without the padding of its sub-accounts, and the parent's assertion then fails"
    ),
    "pad_within_tolerance": (
        "a pad brings the account to exactly the asserted amount: zhang pads the difference even within an "
        "explicit `~` tolerance, where beancount pads nothing and reports the pad unused"
    ),
}


def amount(number, currency):
    return {"number": str(number), "currency": currency}


def case(path):
    entries, errors, options = loader.load_file(path)
    unused_pads = [
        {"date": str(error.entry.date), "account": error.entry.account}
        for error in errors
        if isinstance(error, PadError) and error.message == "Unused Pad entry"
    ]
    balances_at = {(entry.meta["filename"], entry.meta["lineno"]): entry for entry in entries if isinstance(entry, data.Balance)}
    pads_with_cost = [
        {"date": str(balance.date), "account": balance.account}
        for balance in (
            balances_at[(error.source["filename"], error.source["lineno"])]
            for error in errors
            if isinstance(error, PadError) and error.message.startswith("Attempt to pad an entry with cost")
        )
    ]
    other_errors = [error.message for error in errors if not isinstance(error, (BalanceError, PadError))]

    # running units per (account, currency), postings of the account alone
    units = collections.defaultdict(lambda: collections.defaultdict(lambda: 0))
    pads, assertions = [], []
    for entry in entries:
        if isinstance(entry, data.Transaction):
            for posting in entry.postings:
                units[posting.account][posting.units.currency] += posting.units.number
            if entry.flag == "P":
                padded, source = entry.postings
                pads.append(
                    {
                        "date": str(entry.date),
                        "account": padded.account,
                        "units": amount(padded.units.number, padded.units.currency),
                        "from": source.account,
                    }
                )
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
    name = os.path.basename(path)[: -len(".bean")]
    return {
        "balances": balances,
        "pads": pads,
        "assertions": assertions,
        "unused_pads": unused_pads,
        "pads_with_cost": pads_with_cost,
        "errors": other_errors,
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
