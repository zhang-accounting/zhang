//! Scalar (row-level) functions. See the [`crate::functions`] module docs for how to add one.

use chrono::Datelike;
use DataType::*;
use ParamType::Exact;

use super::{FunctionContext, ParamType, ReturnType, ScalarFunction};
use crate::value::{DataType, Value};

// the function library; its entries are registered in `SCALAR_FUNCTIONS` below
mod accounts;
mod amounts;
mod dates;
mod meta;
mod strings;
#[cfg(test)]
mod testing;
pub(crate) mod valuation;

/// The scalar function registry: one entry per overload.
pub static SCALAR_FUNCTIONS: &[ScalarFunction] = &[
    // ---- dates ----
    ScalarFunction {
        name: "year",
        params: &[Exact(Date)],
        returns: ReturnType::Exact(Int),
        description: "The year of a date.",
        eval: year,
    },
    // ---- accounts ----
    ScalarFunction {
        name: "root",
        params: &[Exact(Str)],
        returns: ReturnType::Exact(Str),
        description: "The first component of an account name, e.g. root('Assets:Bank:Checking') = 'Assets'.",
        eval: root,
    },
    ScalarFunction {
        name: "root",
        params: &[Exact(Str), Exact(Int)],
        returns: ReturnType::Exact(Str),
        description: "The first n components of an account name, e.g. root('Assets:Bank:Checking', 2) = 'Assets:Bank'.",
        eval: root,
    },
    // ---- function library: accounts ----
    ScalarFunction {
        name: "parent",
        params: &[Exact(Str)],
        returns: ReturnType::Exact(Str),
        description: "The parent of an account, e.g. parent('Assets:Bank:Checking') = 'Assets:Bank'; '' for a top-level account.",
        eval: accounts::parent,
    },
    ScalarFunction {
        name: "leaf",
        params: &[Exact(Str)],
        returns: ReturnType::Exact(Str),
        description: "The last component of an account name, e.g. leaf('Assets:Bank:Checking') = 'Checking'.",
        eval: accounts::leaf,
    },
    ScalarFunction {
        name: "account_sortkey",
        params: &[Exact(Str)],
        returns: ReturnType::Exact(Str),
        description: "A key that sorts accounts by type (Assets, Liabilities, Equity, Income, Expenses), then by name, e.g. account_sortkey('Expenses:Food') = '4-Expenses:Food'.",
        eval: accounts::account_sortkey,
    },
    // ---- function library: dates ----
    ScalarFunction {
        name: "month",
        params: &[Exact(Date)],
        returns: ReturnType::Exact(Int),
        description: "The month of a date, 1 to 12.",
        eval: dates::month,
    },
    ScalarFunction {
        name: "day",
        params: &[Exact(Date)],
        returns: ReturnType::Exact(Int),
        description: "The day of the month of a date, 1 to 31.",
        eval: dates::day,
    },
    ScalarFunction {
        name: "quarter",
        params: &[Exact(Date)],
        returns: ReturnType::Exact(Str),
        description: "The year and quarter of a date as text, e.g. quarter(2024-05-01) = '2024-Q2'.",
        eval: dates::quarter,
    },
    ScalarFunction {
        name: "weekday",
        params: &[Exact(Date)],
        returns: ReturnType::Exact(Str),
        description: "The three-letter English name of the day of the week, e.g. weekday(2024-01-05) = 'Fri'.",
        eval: dates::weekday,
    },
    ScalarFunction {
        name: "yearmonth",
        params: &[Exact(Date)],
        returns: ReturnType::Exact(Date),
        description: "The first day of the date's month, e.g. yearmonth(2024-05-17) = 2024-05-01.",
        eval: dates::yearmonth,
    },
    ScalarFunction {
        name: "today",
        params: &[],
        returns: ReturnType::Exact(Date),
        description: "Today's date in the ledger's timezone.",
        eval: dates::today,
    },
    // ---- function library: valuation ----
    ScalarFunction {
        name: "units",
        params: &[Exact(Position)],
        returns: ReturnType::Exact(Amount),
        description: "The units of a position, without its cost.",
        eval: valuation::units,
    },
    ScalarFunction {
        name: "units",
        params: &[Exact(Inventory)],
        returns: ReturnType::Exact(Inventory),
        description: "The units of every lot of an inventory, without costs, merged per currency.",
        eval: valuation::units,
    },
    ScalarFunction {
        name: "cost",
        params: &[Exact(Position)],
        returns: ReturnType::Exact(Amount),
        description: "The total cost of a position (units × per-unit cost) in the cost currency; the units if it has no cost.",
        eval: valuation::cost,
    },
    ScalarFunction {
        name: "cost",
        params: &[Exact(Inventory)],
        returns: ReturnType::Exact(Inventory),
        description: "The total cost of every lot of an inventory, merged per currency; lots without cost count as their units.",
        eval: valuation::cost,
    },
    ScalarFunction {
        name: "convert",
        params: &[Exact(Amount), Exact(Str)],
        returns: ReturnType::Exact(Amount),
        description: "Convert an amount to a currency at the latest price; unchanged if there is no price.",
        eval: valuation::convert,
    },
    ScalarFunction {
        name: "convert",
        params: &[Exact(Amount), Exact(Str), Exact(Date)],
        returns: ReturnType::Exact(Amount),
        description: "Convert an amount to a currency at the latest price on or before a date; unchanged if there is no price.",
        eval: valuation::convert,
    },
    ScalarFunction {
        name: "convert",
        params: &[Exact(Position), Exact(Str)],
        returns: ReturnType::Exact(Amount),
        description: "Convert the units of a position to a currency at the latest price, directly or via its cost currency; the units if there is no price.",
        eval: valuation::convert,
    },
    ScalarFunction {
        name: "convert",
        params: &[Exact(Position), Exact(Str), Exact(Date)],
        returns: ReturnType::Exact(Amount),
        description: "Convert the units of a position to a currency at the latest price on or before a date, directly or via its cost currency; the units if there is no price.",
        eval: valuation::convert,
    },
    ScalarFunction {
        name: "convert",
        params: &[Exact(Inventory), Exact(Str)],
        returns: ReturnType::Exact(Inventory),
        description: "Convert every lot of an inventory to a currency at the latest price, directly or via its cost currency; lots without a price keep their units.",
        eval: valuation::convert,
    },
    ScalarFunction {
        name: "convert",
        params: &[Exact(Inventory), Exact(Str), Exact(Date)],
        returns: ReturnType::Exact(Inventory),
        description: "Convert every lot of an inventory to a currency at the latest price on or before a date, directly or via its cost currency; lots without a price keep their units.",
        eval: valuation::convert,
    },
    ScalarFunction {
        name: "value",
        params: &[Exact(Position)],
        returns: ReturnType::Exact(Amount),
        description: "The market value of a position in its cost currency at the latest price; the units if it has no cost or no price.",
        eval: valuation::value,
    },
    ScalarFunction {
        name: "value",
        params: &[Exact(Position), Exact(Date)],
        returns: ReturnType::Exact(Amount),
        description: "The market value of a position in its cost currency at the latest price on or before a date; the units if it has no cost or no price.",
        eval: valuation::value,
    },
    ScalarFunction {
        name: "value",
        params: &[Exact(Inventory)],
        returns: ReturnType::Exact(Inventory),
        description: "The market value of every lot of an inventory in its cost currency at the latest price; other lots keep their units.",
        eval: valuation::value,
    },
    ScalarFunction {
        name: "value",
        params: &[Exact(Inventory), Exact(Date)],
        returns: ReturnType::Exact(Inventory),
        description: "The market value of every lot of an inventory in its cost currency at the latest price on or before a date; other lots keep their units.",
        eval: valuation::value,
    },
    ScalarFunction {
        name: "getprice",
        params: &[Exact(Str), Exact(Str)],
        returns: ReturnType::Exact(Decimal),
        description: "The latest price of one unit of a base currency in a quote currency, e.g. getprice('AAPL', 'USD'); NULL if unknown.",
        eval: valuation::getprice,
    },
    ScalarFunction {
        name: "getprice",
        params: &[Exact(Str), Exact(Str), Exact(Date)],
        returns: ReturnType::Exact(Decimal),
        description: "The price of one unit of a base currency in a quote currency on or before a date; NULL if unknown.",
        eval: valuation::getprice,
    },
    // ---- function library: amounts ----
    ScalarFunction {
        name: "number",
        params: &[Exact(Amount)],
        returns: ReturnType::Exact(Decimal),
        description: "The number of an amount, e.g. number(10.00 USD) = 10.00.",
        eval: amounts::number,
    },
    ScalarFunction {
        name: "currency",
        params: &[Exact(Amount)],
        returns: ReturnType::Exact(Str),
        description: "The currency of an amount, e.g. currency(10.00 USD) = 'USD'.",
        eval: amounts::currency,
    },
    ScalarFunction {
        name: "commodity",
        params: &[Exact(Amount)],
        returns: ReturnType::Exact(Str),
        description: "The currency of an amount; an alias of currency().",
        eval: amounts::currency,
    },
    ScalarFunction {
        name: "only",
        params: &[Exact(Str), Exact(Inventory)],
        returns: ReturnType::Exact(Amount),
        description: "The total units of one currency in an inventory, e.g. only('USD', sum(position)); zero if absent.",
        eval: amounts::only,
    },
    ScalarFunction {
        name: "filter_currency",
        params: &[Exact(Position), Exact(Str)],
        returns: ReturnType::Exact(Position),
        description: "The position if its units are in the currency, otherwise NULL.",
        eval: amounts::filter_currency,
    },
    ScalarFunction {
        name: "filter_currency",
        params: &[Exact(Inventory), Exact(Str)],
        returns: ReturnType::Exact(Inventory),
        description: "The lots of an inventory whose units are in the currency.",
        eval: amounts::filter_currency,
    },
    ScalarFunction {
        name: "abs",
        params: &[Exact(Int)],
        returns: ReturnType::Exact(Int),
        description: "The absolute value of an integer.",
        eval: amounts::abs,
    },
    ScalarFunction {
        name: "abs",
        params: &[Exact(Decimal)],
        returns: ReturnType::Exact(Decimal),
        description: "The absolute value of a number.",
        eval: amounts::abs,
    },
    ScalarFunction {
        name: "abs",
        params: &[Exact(Amount)],
        returns: ReturnType::Exact(Amount),
        description: "The amount with a non-negative number.",
        eval: amounts::abs,
    },
    ScalarFunction {
        name: "abs",
        params: &[Exact(Position)],
        returns: ReturnType::Exact(Position),
        description: "The position with non-negative units, keeping its cost.",
        eval: amounts::abs,
    },
    ScalarFunction {
        name: "abs",
        params: &[Exact(Inventory)],
        returns: ReturnType::Exact(Inventory),
        description: "The inventory with the absolute value of every lot.",
        eval: amounts::abs,
    },
    ScalarFunction {
        name: "neg",
        params: &[Exact(Int)],
        returns: ReturnType::Exact(Int),
        description: "The negated integer.",
        eval: amounts::neg,
    },
    ScalarFunction {
        name: "neg",
        params: &[Exact(Decimal)],
        returns: ReturnType::Exact(Decimal),
        description: "The negated number.",
        eval: amounts::neg,
    },
    ScalarFunction {
        name: "neg",
        params: &[Exact(Amount)],
        returns: ReturnType::Exact(Amount),
        description: "The negated amount.",
        eval: amounts::neg,
    },
    ScalarFunction {
        name: "neg",
        params: &[Exact(Position)],
        returns: ReturnType::Exact(Position),
        description: "The position with negated units, keeping its cost.",
        eval: amounts::neg,
    },
    ScalarFunction {
        name: "neg",
        params: &[Exact(Inventory)],
        returns: ReturnType::Exact(Inventory),
        description: "The inventory with every lot negated.",
        eval: amounts::neg,
    },
    ScalarFunction {
        name: "possign",
        params: &[Exact(Decimal), Exact(Str)],
        returns: ReturnType::Exact(Decimal),
        description: "The number with its sign flipped unless the account is under Assets or Expenses, so balances read positive.",
        eval: amounts::possign,
    },
    ScalarFunction {
        name: "possign",
        params: &[Exact(Amount), Exact(Str)],
        returns: ReturnType::Exact(Amount),
        description: "The amount with its sign flipped unless the account is under Assets or Expenses, so balances read positive.",
        eval: amounts::possign,
    },
    ScalarFunction {
        name: "possign",
        params: &[Exact(Position), Exact(Str)],
        returns: ReturnType::Exact(Position),
        description: "The position with its sign flipped unless the account is under Assets or Expenses, so balances read positive.",
        eval: amounts::possign,
    },
    ScalarFunction {
        name: "possign",
        params: &[Exact(Inventory), Exact(Str)],
        returns: ReturnType::Exact(Inventory),
        description: "The inventory with its sign flipped unless the account is under Assets or Expenses, so balances read positive.",
        eval: amounts::possign,
    },
    // ---- function library: metadata ----
    ScalarFunction {
        name: "meta",
        params: &[Exact(Str)],
        returns: ReturnType::Exact(Str),
        description: "A metadata value of the posting, as text; NULL if absent.",
        eval: meta::meta,
    },
    ScalarFunction {
        name: "entry_meta",
        params: &[Exact(Str)],
        returns: ReturnType::Exact(Str),
        description: "A metadata value of the transaction, as text; NULL if absent.",
        eval: meta::entry_meta,
    },
    ScalarFunction {
        name: "any_meta",
        params: &[Exact(Str)],
        returns: ReturnType::Exact(Str),
        description: "A metadata value of the posting, falling back to the transaction, as text; NULL if absent.",
        eval: meta::any_meta,
    },
    // ---- function library: strings ----
    ScalarFunction {
        name: "str",
        params: &[ParamType::Any],
        returns: ReturnType::Exact(Str),
        description: "Any value as text, e.g. str(position) = '10 AAPL {100 USD, 2024-01-01}'; booleans are TRUE/FALSE.",
        eval: strings::str_,
    },
    ScalarFunction {
        name: "length",
        params: &[Exact(Str)],
        returns: ReturnType::Exact(Int),
        description: "The number of characters in a string.",
        eval: strings::length,
    },
    ScalarFunction {
        name: "length",
        params: &[Exact(Set)],
        returns: ReturnType::Exact(Int),
        description: "The number of elements in a set, e.g. length(tags).",
        eval: strings::length,
    },
    ScalarFunction {
        name: "maxwidth",
        params: &[Exact(Str), Exact(Int)],
        returns: ReturnType::Exact(Str),
        description: "The text with its whitespace collapsed and, when longer than n characters, cut after a word with ' [...]' to fit n (as Python's textwrap.shorten), e.g. maxwidth('Paying the  rent', 12) = 'Paying [...]'.",
        eval: strings::maxwidth,
    },
];

fn year(args: &[Value], _ctx: &dyn FunctionContext) -> Result<Value, String> {
    let date = args[0].as_date().ok_or("year() expects a date")?;
    Ok(Value::Int(date.year() as i64))
}

fn root(args: &[Value], _ctx: &dyn FunctionContext) -> Result<Value, String> {
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

    use super::*;
    use crate::functions::{resolve_scalar, TestContext};

    fn call(name: &str, args: Vec<Value>) -> Value {
        let types = args.iter().map(Value::data_type).collect::<Vec<_>>();
        let resolved = resolve_scalar(name, &types).unwrap();
        (resolved.function.eval)(&args, &TestContext::default()).unwrap()
    }

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
