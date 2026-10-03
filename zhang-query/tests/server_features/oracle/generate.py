#!/usr/bin/env python3
"""Generate the beanquery oracle of the date and directive-metadata functions that
``zhang-query/tests/server_features.rs`` checks (issue #479, track L of the wave 1 spec).

Every case runs one query over ``main.zhang`` with beanquery and records the column types
and the rows. ``oracle.json`` lists the cases in this file's order; the Rust test runs the
same query with zhang and compares column types and rows, in order. Column names are not
compared (they are advisory, as in the conformance suite).

Cells are written the way ``zhang_query::Value`` displays them, so the Rust side compares
``Value::to_string()`` with the oracle text:

* ``None`` -> ``NULL``; booleans -> ``TRUE`` / ``FALSE``;
* dates -> ``YYYY-MM-DD``; ints and strings as they are.

Only these types occur: the queries never select a raw ``interval`` value, whose rendering
beanquery leaves to Python (``relativedelta(months=+1)``).

Oracle version used: beancount 3.2.3, beanquery 0.2.0 (Python 3.9, ``/tmp/bqvenv``).

Usage::

    python generate.py          # (re)write oracle.json
    python generate.py --check  # verify oracle.json is up to date
"""

import argparse
import datetime
import json
import os
import sys

import beanquery
from beancount import loader
from beanquery.sources import beancount as bq_source

HERE = os.path.dirname(os.path.abspath(__file__))
LEDGER = os.path.join(HERE, "main.zhang")
ORACLE = os.path.join(HERE, "oracle.json")


def case(area, name, query, notes=""):
    return dict(area=area, name=name, query=query, notes=notes)


# Every query is ordered, so the rows are compared as a sequence. `SELECT DISTINCT date, ...`
# evaluates the functions once per distinct posting date of the ledger.
CASES = [
    # --- date_trunc -----------------------------------------------------------------------
    case("date_trunc", "date_trunc_every_field",
         "SELECT DISTINCT date, date_trunc('week', date), date_trunc('month', date), "
         "date_trunc('quarter', date), date_trunc('year', date), date_trunc('decade', date), "
         "date_trunc('century', date), date_trunc('millennium', date) ORDER BY date",
         notes="week truncates to the Monday on or before the date; century and millennium start in years "
               "ending in 01 (2000-02-29 truncates to 1901-01-01 and 1001-01-01)."),
    case("date_trunc", "date_trunc_unknown_field_is_null",
         "SELECT DISTINCT date, date_trunc('day', date), date_trunc('MONTH', date), date_trunc('', date) "
         "ORDER BY date",
         notes="beanquery has no 'day' field and the field names are case-sensitive: NULL."),

    # --- date_part ------------------------------------------------------------------------
    case("date_part", "date_part_calendar_fields",
         "SELECT DISTINCT date, date_part('weekday', date), date_part('dow', date), "
         "date_part('isoweekday', date), date_part('isodow', date), date_part('week', date), "
         "date_part('month', date), date_part('quarter', date), date_part('year', date), "
         "date_part('isoyear', date) ORDER BY date",
         notes="weekday/dow is Monday=0; isoweekday/isodow Monday=1; week and isoyear are ISO 8601 "
               "(2021-01-03 is week 53 of 2020, 2024-12-30 week 1 of 2025)."),
    case("date_part", "date_part_long_fields_and_epoch",
         "SELECT DISTINCT date, date_part('decade', date), date_part('century', date), "
         "date_part('millennium', date), date_part('epoch', date) ORDER BY date",
         notes="epoch is seconds since 1970-01-01 (negative before it); century and millennium count "
               "from years ending in 01."),
    case("date_part", "date_part_unknown_field_is_null",
         "SELECT DISTINCT date, date_part('day', date), date_part('Year', date), date_part('', date) "
         "ORDER BY date",
         notes="beanquery has no 'day' field and the field names are case-sensitive: NULL."),

    # --- date_add, date_diff ----------------------------------------------------------------
    case("date_add", "date_add_days_across_month_and_year_ends",
         "SELECT DISTINCT date, date_add(date, 1), date_add(date, -1), date_add(date, 0), "
         "date_add(date, 365), date_add(date, -366) ORDER BY date"),
    case("date_diff", "date_diff_in_days_both_signs",
         "SELECT DISTINCT date, date_diff(date, 2020-01-01), date_diff(2020-01-01, date), "
         "date_diff(date, date), date_diff(date, 1970-01-01) ORDER BY date"),

    # --- date(y, m, d) ----------------------------------------------------------------------
    case("date_ymd", "date_from_parts_of_each_date",
         "SELECT DISTINCT date, date(year, month, 1), date(year, 12, 31), date(year, 2, 29) ORDER BY date",
         notes="date(year, 2, 29) is NULL in common years."),
    case("date_ymd", "date_from_invalid_parts_is_null",
         "SELECT DISTINCT date(2024, 2, 29), date(2023, 2, 29), date(2100, 2, 29), date(2000, 2, 29), "
         "date(2024, 4, 31), date(2024, 13, 1), date(2024, 0, 1), date(2024, 1, 0), date(0, 1, 1), "
         "date(10000, 1, 1), date(1, 1, 1), date(9999, 12, 31) WHERE account = 'Assets:Wallet'",
         notes="Invalid dates are NULL, so are years outside 1..9999 (Python's date range)."),

    # --- interval and date arithmetic -------------------------------------------------------
    case("interval", "interval_months_clamp_to_the_month_end",
         "SELECT DISTINCT date, date + interval('1 month'), date - interval('1 month'), "
         "date + interval('-1 month'), interval('1 month') + date, date + interval('12 months') "
         "ORDER BY date",
         notes="Adding months keeps the day when it exists and clamps it to the month's last day "
               "otherwise (2020-01-31 + 1 month = 2020-02-29)."),
    case("interval", "interval_years_days_and_signs",
         "SELECT DISTINCT date, date + interval('1 year'), date - interval('1 year'), "
         "date + interval('-2 years'), date + interval('10 days'), date - interval('+10 days'), "
         "date + interval('-1 day') ORDER BY date",
         notes="2000-02-29 + 1 year = 2001-02-28; 2024-02-29 - 1 year = 2023-02-28."),
    case("interval", "interval_sum_versus_repeated_addition",
         "SELECT DISTINCT date, date + interval('1 month') + interval('1 month'), "
         "date + (interval('1 month') + interval('1 month')), date + interval('2 months') ORDER BY date",
         notes="Each addition clamps: (2020-01-31 + 1 month) + 1 month = 2020-03-29, but "
               "2020-01-31 + (1 month + 1 month) = 2020-03-31."),
    case("interval", "interval_parsing",
         "SELECT DISTINCT interval('1 day') IS NULL, interval('3 days') IS NULL, interval('+3 days') IS NULL, "
         "interval('-3 days') IS NULL, interval('1  month') IS NULL, interval('2 years') IS NULL, "
         "interval('1 week') IS NULL, interval('1 Month') IS NULL, interval(' 1 day') IS NULL, "
         "interval('1day') IS NULL, interval('one day') IS NULL, interval('1.5 days') IS NULL "
         "WHERE account = 'Assets:Wallet'",
         notes="beanquery 0.2.0 accepts '<signed int> <day|month|year>[s]' only, case-sensitively: "
               "'1 week' is NULL even though its interval() has a branch for weeks (the regular "
               "expression never lets it through)."),
    case("interval", "interval_null_propagates",
         "SELECT DISTINCT date, date + interval('1 week'), date - interval('bogus') ORDER BY date",
         notes="An interval that does not parse is NULL, and so is a date plus NULL."),

    # --- date_bin ---------------------------------------------------------------------------
    case("date_bin", "date_bin_days",
         "SELECT DISTINCT date, date_bin('7 days', date, 2020-01-06), date_bin('1 day', date, 2000-01-01), "
         "date_bin('10 days', date, 2024-01-01), date_bin('-7 days', date, 2020-01-06) ORDER BY date",
         notes="Day strides bin by whole days from the origin, also before it; a negative stride is NULL."),
    case("date_bin", "date_bin_months_and_years_inside_bins",
         "SELECT DISTINCT date, date_bin('1 month', date, 1960-01-15), date_bin('3 months', date, 1960-01-15), "
         "date_bin('1 year', date, 1960-07-01), date_bin(interval('1 month'), date, 1960-01-15) "
         "ORDER BY date",
         notes="Origins in the middle of a month, so that no posting date falls on a bin boundary "
               "(see date_bin_month_boundaries for those)."),
    case("date_bin", "date_bin_months_origin_after_the_dates",
         "SELECT DISTINCT date, date_bin('1 month', date, 2030-01-01), date_bin('1 year', date, 2030-01-01) "
         "ORDER BY date",
         notes="Before the origin, beanquery steps back from the origin until it reaches the date."),
    case("date_bin", "date_bin_negative_month_stride_is_null",
         "SELECT DISTINCT date, date_bin('-1 month', date, 2020-01-01), date_bin('-1 year', date, 2020-01-01) "
         "ORDER BY date",
         notes="A negative stride is NULL (beanquery: 'FIXME: this should raise'). A zero stride is not "
               "here: beanquery 0.2.0 raises ZeroDivisionError for it."),
    case("date_bin_boundary", "date_bin_month_boundaries",
         "SELECT DISTINCT date, date_bin('1 month', date, 2000-01-01), date_bin('1 year', date, 2000-01-01), "
         "date_bin('1 month', date, 2020-01-31) ORDER BY date",
         notes="BEANQUERY QUIRK, flagged for the lead: with a month or year stride a date exactly on a "
               "later bin boundary falls into the previous bin (date_bin('1 month', 2000-02-01, "
               "2000-01-01) is 2000-01-01 in beanquery 0.2.0; the loop stops when the next boundary is "
               ">= the date). With an origin on the 31st, the boundaries drift (2020-01-31, 2020-02-29, "
               "2020-03-29, ...) because each step adds one month to the previous boundary."),

    # --- open_date, close_date, open_meta, commodity_meta ------------------------------------
    case("directive_meta", "open_and_close_dates_of_each_account",
         "SELECT DISTINCT account, open_date(account), close_date(account), "
         "date_diff(close_date(account), open_date(account)), date_trunc('year', close_date(account)) "
         "ORDER BY account",
         notes="close_date is NULL for an account that is not closed, and NULL propagates."),
    case("directive_meta", "open_meta_by_key",
         "SELECT DISTINCT account, open_meta(account, 'owner'), open_meta(account, 'bank'), "
         "open_meta(account, 'category'), open_meta(account, 'nosuchkey') ORDER BY account",
         notes="The metadata of the open directive; NULL for a key it does not have."),
    case("directive_meta", "commodity_meta_by_key",
         "SELECT DISTINCT currency, commodity_meta(currency, 'name'), commodity_meta(currency, 'symbol') "
         "ORDER BY currency",
         notes="The metadata of the commodity directive."),
    case("directive_meta", "unknown_accounts_and_commodities_are_null",
         "SELECT DISTINCT open_date('Assets:Nope'), close_date('Assets:Nope'), open_meta('Assets:Nope', 'owner'), "
         "open_date('Assets'), commodity_meta('XYZ', 'name'), commodity_meta('JPY', 'name'), "
         "commodity_meta('cny', 'name'), open_date('assets:bank') WHERE account = 'Assets:Wallet'",
         notes="An account or commodity without a directive is NULL, and so is a commodity directive "
               "without the key. Names are case-sensitive; a parent account without its own open is "
               "unknown."),
    case("directive_meta", "directive_metadata_in_filters",
         "SELECT account, count(*) WHERE open_meta(account, 'owner') = 'alice' OR close_date(account) IS NOT NULL "
         "GROUP BY account ORDER BY account"),
]


def cell(value):
    if value is None:
        return "NULL"
    if value is True:
        return "TRUE"
    if value is False:
        return "FALSE"
    if isinstance(value, datetime.date):
        return value.isoformat()
    if isinstance(value, (int, str)):
        return str(value)
    raise TypeError("unexpected value {!r} ({})".format(value, type(value).__name__))


def column_type(datatype):
    if datatype is bool:
        return "bool"
    if datatype is int:
        return "int"
    if datatype is str:
        return "str"
    if datatype is datetime.date:
        return "date"
    if datatype is object:
        # open_meta / commodity_meta with a key are dynamically typed; zhang metadata is text
        return "str"
    raise TypeError("unexpected column type {!r}".format(datatype))


def oracle():
    entries, errors, options = loader.load_file(LEDGER)
    if errors:
        sys.exit("beancount reports errors: {}".format([error.message for error in errors]))
    conn = beanquery.Connection()
    bq_source.attach(conn, "beancount:", entries=entries, errors=[], options=options)
    names = set()
    out = []
    for spec in CASES:
        if spec["name"] in names:
            sys.exit("duplicate case name {}".format(spec["name"]))
        names.add(spec["name"])
        cursor = conn.execute(spec["query"])
        rows = [[cell(value) for value in row] for row in cursor.fetchall()]
        if not rows:
            sys.exit("case {} returns no rows".format(spec["name"]))
        out.append(dict(spec, columns=[column_type(column.datatype) for column in cursor.description], rows=rows))
    return out


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--check", action="store_true", help="verify oracle.json is up to date")
    args = parser.parse_args()
    text = json.dumps(oracle(), indent=1, ensure_ascii=False) + "\n"
    if args.check:
        with open(ORACLE, encoding="utf-8") as file:
            if file.read() != text:
                sys.exit("oracle.json is out of date: run generate.py")
        print("oracle.json is up to date")
        return
    with open(ORACLE, "w", encoding="utf-8") as file:
        file.write(text)
    print("wrote {} ({} cases)".format(ORACLE, len(CASES)))


if __name__ == "__main__":
    main()
