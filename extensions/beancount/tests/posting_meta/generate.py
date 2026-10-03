#!/usr/bin/env python3
"""Generate the oracle of posting metadata: where Python beancount attaches the metadata
lines of the transactions in ``ledger.bean``.

``oracle.json`` lists every transaction in file order with its own metadata and the
metadata of each of its postings, every value as ``str(value)``. Beancount's automatic
entries (``filename``, ``lineno`` and the ``__...__`` keys) are left out. The fixture is
checked by ``extensions/beancount/tests/posting_meta.rs``.

Oracle version used: beancount 3.2.3 (Python 3.9).

Usage::

    python generate.py          # (re)write oracle.json
    python generate.py --check  # verify oracle.json is up to date
"""

import argparse
import json
import os
import sys

from beancount import loader
from beancount.core import data

HERE = os.path.dirname(os.path.abspath(__file__))
LEDGER = os.path.join(HERE, "ledger.bean")
ORACLE = os.path.join(HERE, "oracle.json")


def own_meta(meta):
    return {
        key: str(value)
        for key, value in (meta or {}).items()
        if key not in ("filename", "lineno") and not key.startswith("__")
    }


def oracle():
    entries, errors, _ = loader.load_file(LEDGER)
    if errors:
        sys.exit("beancount reports errors: {}".format([error.message for error in errors]))
    transactions = [entry for entry in entries if isinstance(entry, data.Transaction)]
    transactions.sort(key=lambda entry: entry.meta["lineno"])
    return [
        {
            "date": str(entry.date),
            "narration": entry.narration,
            "meta": own_meta(entry.meta),
            "postings": [{"account": posting.account, "meta": own_meta(posting.meta)} for posting in entry.postings],
        }
        for entry in transactions
    ]


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
