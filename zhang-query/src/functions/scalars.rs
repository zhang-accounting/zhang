//! Scalar (row-level) functions. See the [`crate::functions`] module docs for how to add one.

use chrono::Datelike;
use DataType::*;

pub(crate) use self::accounts::is_under;
#[cfg(test)]
pub(crate) use self::testing::TestContext;
use super::{Eval, ParamType, ReturnType, ScalarFunction};
use crate::value::{DataType, Value};

// the function library; its entries are registered in `SCALAR_FUNCTIONS` below
mod accounts;
mod amounts;
mod compare;
mod dates;
mod ledger;
mod meta;
mod search;
mod strings;
#[cfg(test)]
mod testing;
mod valuation;

/// The registry rows: `#[flags] name(param, ...) -> returns = implementation, "description";`, one per overload, with
/// the flags as in the [`crate::functions`] module docs.
macro_rules! scalars {
    ($($(#[$($flag:ident),+])? $name:ident($($param:tt)*) -> $returns:ident = $eval:path, $description:literal;)*) => {
        &[$(scalar!([$($($flag)+)?] Args, false; stringify!($name), params!([] $($param)*), returns!($returns), $description, $eval)),*]
    };
}

/// One registry row, its flags applied one by one to the implementation's kind and to `total`.
macro_rules! scalar {
    ([execution $($flag:ident)*] $kind:ident, $total:literal; $($row:tt)*) => { scalar!([$($flag)*] Execution, $total; $($row)*) };
    ([row $($flag:ident)*] $kind:ident, $total:literal; $($row:tt)*) => { scalar!([$($flag)*] Row, $total; $($row)*) };
    ([total $($flag:ident)*] $kind:ident, $total:literal; $($row:tt)*) => { scalar!([$($flag)*] $kind, true; $($row)*) };
    ([] $kind:ident, $total:literal; $name:expr, $params:expr, $returns:expr, $description:expr, $eval:path) => {
        ScalarFunction {
            name: $name,
            params: $params,
            returns: $returns,
            description: $description,
            eval: Eval::$kind($eval),
            total: $total,
        }
    };
}

/// The scalar function registry: one row per overload.
pub static SCALAR_FUNCTIONS: &[ScalarFunction] = scalars![
    // ---- dates ----
    year(Date) -> Int = year,
        "The year of a date.";
    // ---- accounts ----
    root(Str) -> Str = root,
        "The first component of an account name, e.g. root('Assets:Bank:Checking') = 'Assets'.";
    root(Str, Int) -> Str = root,
        "The first n components of an account name, e.g. root('Assets:Bank:Checking', 2) = 'Assets:Bank'.";
    // ---- function library: accounts ----
    parent(Str) -> Str = accounts::parent,
        "The parent of an account, e.g. parent('Assets:Bank:Checking') = 'Assets:Bank'; '' for a top-level account.";
    leaf(Str) -> Str = accounts::leaf,
        "The last component of an account name, e.g. leaf('Assets:Bank:Checking') = 'Checking'.";
    account_sortkey(Str) -> Str = accounts::account_sortkey,
        "A key that sorts accounts by type (Assets, Liabilities, Equity, Income, Expenses), then by name, e.g. account_sortkey('Expenses:Food') = '4-Expenses:Food'.";
    #[total] under(Str, Str) -> Bool = accounts::under,
        "Whether an account is the ancestor or one of its sub-accounts, e.g. under('Assets:Bank:Cash', 'Assets:Bank') is TRUE and under('Assets:Banking', 'Assets:Bank') FALSE. A zhang extension.";
    // ---- function library: account and commodity directives ----
    #[execution] open_date(Str) -> Date = ledger::open_date,
        "The date of the account's open directive; NULL if it has none.";
    #[execution] close_date(Str) -> Date = ledger::close_date,
        "The date of the account's close directive; NULL while it is open.";
    #[execution] open_meta(Str) -> Metas = ledger::open_meta,
        "The metadata of the account's open directive as (key, value) pairs, [] when it has none; NULL if the account has no open directive.";
    #[execution] open_meta(Str, Str) -> Str = ledger::open_meta,
        "A metadata value of the account's open directive, e.g. open_meta(account, 'institution'); NULL if absent.";
    #[execution] account_budgets(Str, Date) -> Set = ledger::account_budgets,
        "The budgets a posting of an account at the start of a date counts in, as a posting of that date without a time sees them: those the budget metadata of its latest open at or before then names, so an account closed and opened again with other budgets counts in those from its reopening on; empty before its first open. A zhang extension.";
    #[execution] account_status(Str, Date) -> Str = ledger::account_status,
        "Whether an account is 'open' or 'closed' at the start of a date, as a directive of that date without a time sees it: open from its open on, closed once its close takes effect, which a close with only a date does at the end of its day and a close with a time at that time, and open again after a later open; NULL when neither an open nor a close of it is in effect. A zhang extension.";
    #[execution] account_status(Str, Date, Str) -> Str = ledger::account_status,
        "Whether an account is 'open' or 'closed' at a date and a time of day, written HH:MM:SS or HH:MM as the time column holds it, e.g. account_status(account, date, time). A zhang extension.";
    #[execution] commodity_meta(Str) -> Metas = ledger::commodity_meta,
        "The metadata of the currency's commodity directive as (key, value) pairs, [] when it has none; NULL without a commodity directive.";
    #[execution] commodity_meta(Str, Str) -> Str = ledger::commodity_meta,
        "A metadata value of the currency's commodity directive, e.g. commodity_meta(currency, 'name'); NULL if absent.";
    #[execution] currency_meta(Str) -> Metas = ledger::commodity_meta,
        "The metadata of the currency's commodity directive; an alias of commodity_meta().";
    #[execution] currency_meta(Str, Str) -> Str = ledger::commodity_meta,
        "A metadata value of the currency's commodity directive; an alias of commodity_meta().";
    // ---- function library: dates ----
    month(Date) -> Int = dates::month,
        "The month of a date, 1 to 12.";
    day(Date) -> Int = dates::day,
        "The day of the month of a date, 1 to 31.";
    quarter(Date) -> Str = dates::quarter,
        "The year and quarter of a date as text, e.g. quarter(2024-05-01) = '2024-Q2'.";
    weekday(Date) -> Str = dates::weekday,
        "The three-letter English name of the day of the week, e.g. weekday(2024-01-05) = 'Fri'.";
    yearmonth(Date) -> Date = dates::yearmonth,
        "The first day of the date's month, e.g. yearmonth(2024-05-17) = 2024-05-01.";
    #[execution] today() -> Date = dates::today,
        "Today's date in the ledger's timezone.";
    #[execution] now() -> Str = dates::now,
        "The current time of day in the ledger's timezone as HH:MM:SS, as the time column holds it; today() is its date, so account_status(account, today(), now()) is whether an account is open now. A zhang extension.";
    date(Int, Int, Int) -> Date = dates::date_from_ymd,
        "The date of a year, month and day, e.g. date(2024, 2, 29); NULL if there is no such day.";
    date(Str) -> Date = dates::date_from_str,
        "The date written as YYYY-MM-DD (month and day may have one digit), e.g. date('2024-2-9'); NULL if it is not one.";
    date_add(Date, Int) -> Date = dates::date_add,
        "The date moved by a number of days, e.g. date_add(2024-02-28, 1) = 2024-02-29.";
    date_diff(Date, Date) -> Int = dates::date_diff,
        "The number of days from the second date to the first, e.g. date_diff(2024-03-01, 2024-02-01) = 29.";
    date_trunc(Str, Date) -> Date = dates::date_trunc,
        "The first day of the date's 'week' (Monday), 'month', 'quarter', 'year', 'decade', 'century' or 'millennium', e.g. date_trunc('month', 2024-05-17) = 2024-05-01; NULL for another field.";
    date_part(Str, Date) -> Int = dates::date_part,
        "A field of a date: 'weekday' or 'dow' (Monday 0), 'isoweekday' or 'isodow' (Monday 1), 'week' (ISO week), 'month', 'quarter', 'year', 'isoyear', 'decade', 'century', 'millennium' or 'epoch' (seconds); NULL for another field.";
    interval(Str) -> Interval = dates::interval,
        "An interval of days, weeks (a zhang extension), months or years, e.g. interval('3 months') or interval('-1 year'), to add to a date or bin by; NULL for any other text.";
    date_bin(Interval, Date, Date) -> Date = dates::date_bin,
        "The start of the bin of a date among the bins origin + k × interval, e.g. date_bin(interval('7 days'), date, 2024-01-01); a date on a boundary starts its bin.";
    date_bin(Str, Date, Date) -> Date = dates::date_bin,
        "date_bin() with the interval written as text, e.g. date_bin('1 month', date, 2024-01-01).";
    // ---- function library: valuation ----
    #[total] units(Position) -> Amount = valuation::units,
        "The units of a position, without its cost.";
    #[total] units(Inventory) -> Inventory = valuation::units,
        "The units of every lot of an inventory, without costs, merged per currency.";
    #[total] cost(Position) -> Amount = valuation::cost,
        "The total cost of a position (units × per-unit cost) in the cost currency; the units if it has no cost.";
    #[total] cost(Inventory) -> Inventory = valuation::cost,
        "The total cost of every lot of an inventory, merged per currency; lots without cost count as their units.";
    #[execution, total] convert(Amount, Str) -> Amount = valuation::convert,
        "Convert an amount to a currency at the latest price; unchanged if there is no price.";
    #[execution, total] convert(Amount, Str, Date) -> Amount = valuation::convert,
        "Convert an amount to a currency at the latest price on or before a date; unchanged if there is no price.";
    #[execution, total] convert(Position, Str) -> Amount = valuation::convert,
        "Convert the units of a position to a currency at the latest price, directly or via its cost currency; the units if there is no price.";
    #[execution, total] convert(Position, Str, Date) -> Amount = valuation::convert,
        "Convert the units of a position to a currency at the latest price on or before a date, directly or via its cost currency; the units if there is no price.";
    #[execution, total] convert(Inventory, Str) -> Inventory = valuation::convert,
        "Convert every lot of an inventory to a currency at the latest price, directly or via its cost currency; lots without a price keep their units.";
    #[execution, total] convert(Inventory, Str, Date) -> Inventory = valuation::convert,
        "Convert every lot of an inventory to a currency at the latest price on or before a date, directly or via its cost currency; lots without a price keep their units.";
    #[execution, total] value(Position) -> Amount = valuation::value,
        "The market value of a position in its cost currency at the latest price; the units if it has no cost or no price.";
    #[execution, total] value(Position, Date) -> Amount = valuation::value,
        "The market value of a position in its cost currency at the latest price on or before a date; the units if it has no cost or no price.";
    #[execution, total] value(Inventory) -> Inventory = valuation::value,
        "The market value of every lot of an inventory in its cost currency at the latest price; other lots keep their units.";
    #[execution, total] value(Inventory, Date) -> Inventory = valuation::value,
        "The market value of every lot of an inventory in its cost currency at the latest price on or before a date; other lots keep their units.";
    #[execution] getprice(Str, Str) -> Decimal = valuation::getprice,
        "The latest price of one unit of a base currency in a quote currency, e.g. getprice('AAPL', 'USD'); NULL if unknown.";
    #[execution] getprice(Str, Str, Date) -> Decimal = valuation::getprice,
        "The price of one unit of a base currency in a quote currency on or before a date; NULL if unknown.";
    // ---- function library: amounts ----
    number(Amount) -> Decimal = amounts::number,
        "The number of an amount, e.g. number(10.00 USD) = 10.00.";
    currency(Amount) -> Str = amounts::currency,
        "The currency of an amount, e.g. currency(10.00 USD) = 'USD'.";
    commodity(Amount) -> Str = amounts::currency,
        "The currency of an amount; an alias of currency().";
    #[total] only(Str, Inventory) -> Amount = amounts::only,
        "The total units of one currency in an inventory, e.g. only('USD', sum(position)); zero if absent.";
    #[total] filter_currency(Position, Str) -> Position = amounts::filter_currency,
        "The position if its units are in the currency, otherwise NULL.";
    #[total] filter_currency(Inventory, Str) -> Inventory = amounts::filter_currency,
        "The lots of an inventory whose units are in the currency.";
    // abs() and neg() of an integer can overflow, so they are not total
    abs(Int) -> Int = amounts::abs,
        "The absolute value of an integer.";
    #[total] abs(Decimal) -> Decimal = amounts::abs,
        "The absolute value of a number.";
    #[total] abs(Amount) -> Amount = amounts::abs,
        "The amount with a non-negative number.";
    #[total] abs(Position) -> Position = amounts::abs,
        "The position with non-negative units, keeping its cost.";
    #[total] abs(Inventory) -> Inventory = amounts::abs,
        "The inventory with the absolute value of every lot.";
    neg(Int) -> Int = amounts::neg,
        "The negated integer.";
    #[total] neg(Decimal) -> Decimal = amounts::neg,
        "The negated number.";
    #[total] neg(Amount) -> Amount = amounts::neg,
        "The negated amount.";
    #[total] neg(Position) -> Position = amounts::neg,
        "The position with negated units, keeping its cost.";
    #[total] neg(Inventory) -> Inventory = amounts::neg,
        "The inventory with every lot negated.";
    #[total] possign(Decimal, Str) -> Decimal = amounts::possign,
        "The number with its sign flipped unless the account is under Assets or Expenses, so balances read positive.";
    #[total] possign(Amount, Str) -> Amount = amounts::possign,
        "The amount with its sign flipped unless the account is under Assets or Expenses, so balances read positive.";
    #[total] possign(Position, Str) -> Position = amounts::possign,
        "The position with its sign flipped unless the account is under Assets or Expenses, so balances read positive.";
    #[total] possign(Inventory, Str) -> Inventory = amounts::possign,
        "The inventory with its sign flipped unless the account is under Assets or Expenses, so balances read positive.";
    // ---- function library: metadata ----
    #[row] meta(Str) -> Str = meta::meta,
        "A metadata value of the posting, as text; NULL if absent.";
    #[row] entry_meta(Str) -> Str = meta::entry_meta,
        "A metadata value of the transaction, as text; NULL if absent.";
    #[row] any_meta(Str) -> Str = meta::any_meta,
        "A metadata value of the posting, falling back to the transaction, as text; NULL if absent.";
    #[row] meta_values(Str) -> Set = meta::meta_values,
        "Every value of a metadata key of the posting, as a set (a repeated key has several); empty if absent. A zhang extension.";
    #[row] entry_meta_values(Str) -> Set = meta::entry_meta_values,
        "Every value of a metadata key of the transaction, as a set (a repeated key has several); empty if absent. A zhang extension.";
    // ---- function library: search (zhang extensions) ----
    #[total] icontains(Str, Str) -> Bool = search::icontains,
        "Whether the text contains the needle, ignoring case (Unicode lower-case), e.g. icontains(payee, 'cafe'). A zhang extension.";
    #[total] any_icontains(Set, Str) -> Bool = search::any_icontains,
        "Whether an element of the set contains the needle, ignoring case, e.g. any_icontains(tags, 'trip'). A zhang extension.";
    #[total] intersects(Set, Set) -> Bool = search::intersects,
        "Whether the two sets share an element, e.g. intersects(tags, :tags). A zhang extension.";
    set(Str...) -> Set = search::set,
        "A set of the given strings, e.g. intersects(tags, set('trip', 'food')); set() is the empty set. A zhang extension.";
    // ---- function library: comparison (zhang extensions) ----
    least(Bool, Bool) -> Bool = compare::least,
        "The smaller of two values, e.g. least(date_add(date, 6), :to); NULL if either is NULL. A zhang extension.";
    least(Int, Int) -> Int = compare::least,
        "The smaller of two values, e.g. least(date_add(date, 6), :to); NULL if either is NULL. A zhang extension.";
    least(Decimal, Decimal) -> Decimal = compare::least,
        "The smaller of two values, e.g. least(date_add(date, 6), :to); NULL if either is NULL. A zhang extension.";
    least(Str, Str) -> Str = compare::least,
        "The smaller of two values, e.g. least(date_add(date, 6), :to); NULL if either is NULL. A zhang extension.";
    least(Date, Date) -> Date = compare::least,
        "The smaller of two values, e.g. least(date_add(date, 6), :to); NULL if either is NULL. A zhang extension.";
    greatest(Bool, Bool) -> Bool = compare::greatest,
        "The larger of two values, e.g. greatest(date, :from); NULL if either is NULL. A zhang extension.";
    greatest(Int, Int) -> Int = compare::greatest,
        "The larger of two values, e.g. greatest(date, :from); NULL if either is NULL. A zhang extension.";
    greatest(Decimal, Decimal) -> Decimal = compare::greatest,
        "The larger of two values, e.g. greatest(date, :from); NULL if either is NULL. A zhang extension.";
    greatest(Str, Str) -> Str = compare::greatest,
        "The larger of two values, e.g. greatest(date, :from); NULL if either is NULL. A zhang extension.";
    greatest(Date, Date) -> Date = compare::greatest,
        "The larger of two values, e.g. greatest(date, :from); NULL if either is NULL. A zhang extension.";
    // ---- function library: strings ----
    #[total] str(any) -> Str = strings::str_,
        "Any value as text, e.g. str(position) = '10 AAPL {100 USD, 2024-01-01}'; booleans are TRUE/FALSE.";
    length(Str) -> Int = strings::length,
        "The number of characters in a string.";
    length(Set) -> Int = strings::length,
        "The number of elements in a set, e.g. length(tags).";
    maxwidth(Str, Int) -> Str = strings::maxwidth,
        "The text with its whitespace collapsed and, when longer than n characters, cut after a word with ' [...]' to fit n (as Python's textwrap.shorten), e.g. maxwidth('Paying the  rent', 12) = 'Paying [...]'.";
];

/// Whether the function `name` reads the row being evaluated (`#[row]`), as each overload of a metadata function does.
pub(crate) fn reads_the_row(name: &str) -> bool {
    SCALAR_FUNCTIONS.iter().any(|it| it.name == name && matches!(it.eval, Eval::Row(_)))
}

fn year(args: &[Value]) -> Result<Value, String> {
    let date = args[0].as_date().ok_or("year() expects a date")?;
    Ok(Value::Int(date.year() as i64))
}

fn root(args: &[Value]) -> Result<Value, String> {
    let account = args[0].as_str().ok_or("root() expects an account name")?;
    let n = match args.get(1) {
        Some(n) => n.as_int().ok_or("root() expects an integer depth")?,
        None => 1,
    };
    let n = usize::try_from(n).unwrap_or(0);
    Ok(Value::Str(account.split(':').take(n).collect::<Vec<_>>().join(":")))
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;

    use super::testing::call;
    use super::*;
    use crate::functions::resolve_scalar;

    #[test]
    fn year_of_date() {
        assert_eq!(call("year", vec![Value::Date(NaiveDate::from_ymd_opt(2024, 3, 1).unwrap())]), Value::Int(2024));
    }

    #[test]
    fn root_of_account() {
        assert_eq!(call("root", vec!["Assets:Bank:Checking".into()]), Value::from("Assets"));
        assert_eq!(call("root", vec!["Assets:Bank:Checking".into(), Value::Int(2)]), Value::from("Assets:Bank"));
        assert_eq!(call("root", vec!["Assets:Bank".into(), Value::Int(9)]), Value::from("Assets:Bank"));
        assert_eq!(call("root", vec!["Assets:Bank".into(), Value::Int(0)]), Value::from(""));
    }

    #[test]
    fn unknown_overload_lists_signatures() {
        let err = resolve_scalar("root", &[DataType::Int]).err().unwrap();
        assert!(err.contains("root(str) -> str"), "{}", err);
    }
}
