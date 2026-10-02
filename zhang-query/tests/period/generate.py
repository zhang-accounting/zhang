#!/usr/bin/env python3
"""Generate the oracle fixtures of the FROM period modifiers (OPEN ON, CLOSE [ON], CLEAR).

The expected results in ``cases/*.json`` come from the official Python beanquery over the
shared ledger, exactly like the conformance suite in ``../conformance``: this script reuses
that generator's ledger loading, validation (determinism and synthetic balance-check
re-runs), encoding and fixture format, and only holds its own case list. The fixtures are
checked by ``zhang-query/tests/period.rs``.

Usage::

    python generate.py [LEDGER]          # (re)write cases/*.json
    python generate.py --check [LEDGER]  # verify cases/*.json are up to date
"""

import argparse
import inspect
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, "..", "conformance"))

import generate as conformance  # noqa: E402  (the conformance generator)
from generate import ENGINE, LEDGER, case  # noqa: E402

CASES_DIR = os.path.join(HERE, "cases")

CASES = [
    # --- reports ------------------------------------------------------------
    case("period", "income_statement_2016",
         "SELECT account, sum(position) FROM OPEN ON 2016-01-01 CLOSE ON 2017-01-01 "
         "WHERE account ~ '^(Income|Expenses)' GROUP BY 1 ORDER BY 1",
         ordered=True,
         notes="The income statement of 2016: OPEN moves earlier income and expenses to equity, CLOSE drops "
               "later entries."),
    case("period", "balance_sheet_at_2017_clear",
         "SELECT account, sum(position) FROM CLOSE ON 2017-01-01 CLEAR "
         "WHERE account ~ '^(Assets|Liabilities|Equity)' GROUP BY 1 ORDER BY 1",
         kind=LEDGER, ordered=True,
         notes="The balance sheet at 2017-01-01: CLEAR moves all income and expenses to Equity:Earnings:Current, "
               "and CLOSE adds the Equity:Conversions:Current entry. Holdings keep their lots."),
    case("period", "balance_sheet_totals_by_root",
         "SELECT root(account, 1) AS root, sum(cost(position)) AS total "
         "FROM OPEN ON 2016-01-01 CLOSE ON 2017-01-01 CLEAR GROUP BY 1 ORDER BY 1",
         ordered=True,
         notes="Per root account at cost: income and expenses net to nothing after CLEAR, and the whole "
               "period balances at cost except for the zero-priced conversion entry."),
    case("period", "open_holdings_with_lots",
         "SELECT account, sum(position) FROM OPEN ON 2017-01-01 "
         "WHERE account ~ '^Assets:US:(ETrade|Vanguard)' GROUP BY 1 ORDER BY 1",
         kind=LEDGER, ordered=True,
         notes="Holdings after OPEN: the summarization entries keep the booked lots (cost, cost date)."),
    case("period", "open_holdings_units_and_cost",
         "SELECT account, currency, sum(number) AS units, sum(cost(position)) AS book_value "
         "FROM OPEN ON 2016-06-01 WHERE account ~ '^Assets:US:ETrade' AND currency != 'USD' "
         "GROUP BY 1, 2 ORDER BY 1, 2",
         kind=LEDGER, ordered=True),
    case("period", "open_equity_accounts",
         "SELECT account, sum(position) FROM OPEN ON 2016-01-01 WHERE account ~ '^Equity' GROUP BY 1 ORDER BY 1",
         ordered=True,
         notes="Equity after OPEN: previous earnings, previous conversions and the opening balances."),

    # --- synthetic rows ------------------------------------------------------
    case("synthetic", "flags_counts_and_dates",
         "SELECT flag, count(*) AS n, min(date) AS first, max(date) AS last "
         "FROM OPEN ON 2016-01-01 CLOSE ON 2017-01-01 CLEAR GROUP BY 1 ORDER BY 1",
         kind=LEDGER, ordered=True,
         notes="S (summarization) rows are dated the day before OPEN, the C (conversion) entry the day before "
               "CLOSE, and the T (transfer) entries like the last entry of the period."),
    case("synthetic", "summarization_rows",
         "SELECT date, flag, payee, narration, description, account, position, price, weight, tags, links "
         "FROM OPEN ON 2016-01-01 WHERE flag = 'S'",
         kind=LEDGER,
         notes="Every summarization posting: one per lot of each account, against Equity:Opening-Balances at cost."),
    case("synthetic", "summarization_entries",
         "SELECT first(narration) AS narration, count(*) AS postings FROM OPEN ON 2016-01-01 WHERE flag = 'S' "
         "GROUP BY id",
         notes="One summarization entry per account; all its postings share one id."),
    case("synthetic", "transfer_rows",
         "SELECT date, flag, narration, account, position, other_accounts "
         "FROM CLOSE ON 2017-01-01 CLEAR WHERE flag = 'T'",
         notes="CLEAR: one transfer entry per income or expense account, against Equity:Earnings:Current."),
    case("synthetic", "conversion_row",
         "SELECT date, flag, payee, account, position, price, weight, cost_number, other_accounts "
         "FROM OPEN ON 2016-01-01 CLOSE ON 2017-01-01 WHERE flag = 'C'",
         notes="The conversion entry of CLOSE: the residual of the price conversions at cost, priced at zero in "
               "the conversion currency (NOTHING). Its narration lists the period's balance and is not compared."),
    case("synthetic", "bare_close_conversion",
         "SELECT date, flag, account, position, price FROM CLOSE WHERE flag = 'C'",
         notes="A bare CLOSE drops nothing and dates the conversion entry like the last entry of the ledger."),
    case("synthetic", "bare_clear",
         "SELECT flag, account, count(*) AS n, min(date) AS first, max(date) AS last, sum(position) AS total "
         "FROM CLEAR WHERE account ~ '^Equity' GROUP BY 1, 2 ORDER BY 1, 2",
         ordered=True,
         notes="CLEAR without CLOSE: the transfer entries are dated like the last entry of the ledger "
               "(here a price on 2017-09-08)."),
    case("synthetic", "clear_dated_like_last_entry",
         "SELECT date, flag, account, position FROM OPEN ON 2016-01-01 CLOSE ON 2016-01-02 CLEAR WHERE flag IN ('C', 'T')",
         notes="Without a conversion entry the transfer entries take the date of the last entry before CLOSE."),

    # --- composition and edges -------------------------------------------------
    case("compose", "from_expression_filters_after_the_period",
         "SELECT account, sum(position) FROM year = 2016 OPEN ON 2016-06-01 "
         "WHERE account ~ '^Assets:US:BofA' GROUP BY 1 ORDER BY 1",
         ordered=True,
         notes="The FROM expression filters the transformed rows, like WHERE: the opening balance on "
               "2016-05-31 still summarizes 2015."),
    case("compose", "from_account_filter_with_clear",
         "SELECT flag, count(*) AS n, sum(position) AS total FROM account ~ 'Expenses:Food' OPEN ON 2016-01-01 "
         "CLEAR GROUP BY 1 ORDER BY 1",
         ordered=True,
         notes="After OPEN the food accounts start at zero; CLEAR then empties them again."),
    case("compose", "open_before_the_ledger",
         "SELECT flag, count(*) AS n FROM OPEN ON 2010-01-01 GROUP BY 1 ORDER BY 1",
         kind=LEDGER, ordered=True,
         notes="Nothing to summarize: the postings are unchanged."),
    case("compose", "open_after_the_ledger",
         "SELECT flag, count(*) AS n, sum(cost(position)) AS total FROM OPEN ON 2030-01-01 GROUP BY 1 ORDER BY 1",
         ordered=True),
    case("compose", "close_before_the_ledger",
         "SELECT count(*) AS n FROM CLOSE ON 2010-01-01 CLEAR"),
    case("compose", "open_and_close_on_the_same_day",
         "SELECT flag, count(*) AS n FROM open on 2016-01-01 close on 2016-01-01 GROUP BY 1 ORDER BY 1",
         ordered=True,
         notes="Keywords are case-insensitive; CLOSE may equal OPEN."),

    # --- errors -------------------------------------------------------------
    case("error", "error_modifiers_out_of_order",
         "SELECT count(*) FROM CLEAR OPEN ON 2016-01-01", expect="error"),
    case("error", "error_open_on_string",
         "SELECT count(*) FROM OPEN ON '2016-01-01'", expect="error"),
    case("error", "error_close_before_open",
         "SELECT count(*) FROM OPEN ON 2017-01-01 CLOSE ON 2016-01-01", expect="error"),
]


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("ledger", nargs="?", default=conformance.DEFAULT_LEDGER)
    parser.add_argument("--check", action="store_true", help="verify fixtures instead of writing them")
    args = parser.parse_args()

    entries, options = conformance.load_ledger(args.ledger)
    conns = (conformance.connect(entries, options),
             conformance.connect(conformance.perturbed(entries), options),
             conformance.connect(conformance.with_synthetic_balance_checks(entries), options))
    print(f"oracle: beancount {conformance.beancount.__version__}, beanquery {conformance.beanquery.__version__}")

    # newer conformance generators also take the ledger's display context (for CSV cases)
    extra = (options["dcontext"],) if "dcontext" in inspect.signature(conformance.build_fixture).parameters else ()
    outputs = {}
    for index, spec in enumerate(CASES, start=1):
        filename, text, detail = conformance.build_fixture(index, spec, conns, *extra)
        outputs[filename] = text
        print(f"  {filename:<55} {spec['kind']:<17} {detail}")

    os.makedirs(CASES_DIR, exist_ok=True)
    existing = {name for name in os.listdir(CASES_DIR) if name.endswith(".json")}
    if args.check:
        changed = [name for name, text in outputs.items()
                   if not os.path.exists(os.path.join(CASES_DIR, name))
                   or open(os.path.join(CASES_DIR, name), encoding="utf-8").read() != text]
        stale = sorted(existing - set(outputs))
        if changed or stale:
            sys.exit(f"fixtures out of date: changed={changed} stale={stale}")
        print("fixtures are up to date")
    else:
        for name in existing - set(outputs):
            os.remove(os.path.join(CASES_DIR, name))
        for name, text in outputs.items():
            with open(os.path.join(CASES_DIR, name), "w", encoding="utf-8") as handle:
                handle.write(text)
    print(f"{len(CASES)} cases")


if __name__ == "__main__":
    main()
