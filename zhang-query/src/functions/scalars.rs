//! Scalar (row-level) functions. See the [`crate::functions`] module docs for how to add one.

use chrono::Datelike;

use super::{FunctionContext, ParamType, ReturnType, ScalarFunction};
use crate::value::{DataType, Value};

use DataType::*;
use ParamType::Exact;

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
