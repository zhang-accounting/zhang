#!/usr/bin/env python3
"""Generate the oracle of the metadata values beancount reads without quotes (#475): the
value Python beancount gives every metadata entry of ``ledger.bean``.

``oracle.json`` lists every directive in file order with its type, date, account or
narration when it has one, its own metadata and, for a transaction, the metadata of each
of its postings, every value as ``str(value)``. Beancount's automatic entries
(``filename``, ``lineno`` and the ``__...__`` keys) are left out. The fixture is checked
by ``extensions/beancount/tests/meta_values.rs``.

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
    # the entries written in the file, in file order: beancount adds a padding transaction
    entries = [entry for entry in entries if entry.meta.get("filename") == LEDGER and not entry.meta.get("__automatic__")]
    entries = [entry for entry in entries if not (isinstance(entry, data.Transaction) and entry.flag == "P")]
    entries.sort(key=lambda entry: entry.meta["lineno"])
    rows = []
    for entry in entries:
        row = {"type": type(entry).__name__, "date": str(entry.date), "meta": own_meta(entry.meta)}
        if isinstance(entry, data.Transaction):
            row["narration"] = entry.narration
            row["postings"] = [{"account": posting.account, "meta": own_meta(posting.meta)} for posting in entry.postings]
        elif hasattr(entry, "account"):
            row["account"] = entry.account
        rows.append(row)
    return rows


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
