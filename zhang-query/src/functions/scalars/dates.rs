//! Date functions: `month`, `day`, `quarter`, `weekday`, `yearmonth` and `today`
//! (`year` lives with the registry examples).

use chrono::{Datelike, NaiveDate, Weekday};

use crate::functions::FunctionContext;
use crate::value::Value;

fn date_arg(args: &[Value], function: &str) -> Result<NaiveDate, String> {
    args[0].as_date().ok_or_else(|| format!("{}() expects a date", function))
}

pub(super) fn month(args: &[Value], _ctx: &dyn FunctionContext) -> Result<Value, String> {
    Ok(Value::Int(date_arg(args, "month")?.month() as i64))
}

pub(super) fn day(args: &[Value], _ctx: &dyn FunctionContext) -> Result<Value, String> {
    Ok(Value::Int(date_arg(args, "day")?.day() as i64))
}

/// beanquery `quarter`: `YYYY-Qn`, e.g. `2024-Q1`.
pub(super) fn quarter(args: &[Value], _ctx: &dyn FunctionContext) -> Result<Value, String> {
    let date = date_arg(args, "quarter")?;
    Ok(Value::Str(format!("{:04}-Q{}", date.year(), (date.month() - 1) / 3 + 1)))
}

/// beanquery `weekday`: the three-letter English day name (`strftime('%a')` in the C locale).
pub(super) fn weekday(args: &[Value], _ctx: &dyn FunctionContext) -> Result<Value, String> {
    let name = match date_arg(args, "weekday")?.weekday() {
        Weekday::Mon => "Mon",
        Weekday::Tue => "Tue",
        Weekday::Wed => "Wed",
        Weekday::Thu => "Thu",
        Weekday::Fri => "Fri",
        Weekday::Sat => "Sat",
        Weekday::Sun => "Sun",
    };
    Ok(Value::from(name))
}

/// beanquery `yearmonth`: the first day of the date's month.
pub(super) fn yearmonth(args: &[Value], _ctx: &dyn FunctionContext) -> Result<Value, String> {
    let date = date_arg(args, "yearmonth")?;
    Ok(Value::Date(date.with_day(1).ok_or("yearmonth() got an invalid date")?))
}

pub(super) fn today(_args: &[Value], ctx: &dyn FunctionContext) -> Result<Value, String> {
    Ok(Value::Date(ctx.today()))
}

#[cfg(test)]
mod tests {
    use super::super::testing::*;
    use super::*;
    use crate::functions::TestContext;

    fn on(day: &str) -> Value {
        Value::Date(date(day))
    }

    #[test]
    fn month_and_day() {
        assert_eq!(call("month", vec![on("2024-03-31")]), Value::Int(3));
        assert_eq!(call("day", vec![on("2024-03-31")]), Value::Int(31));
        assert_eq!(call("day", vec![on("2024-02-29")]), Value::Int(29));
    }

    #[test]
    fn quarter_label() {
        assert_eq!(call("quarter", vec![on("2024-01-01")]), Value::from("2024-Q1"));
        assert_eq!(call("quarter", vec![on("2024-03-31")]), Value::from("2024-Q1"));
        assert_eq!(call("quarter", vec![on("2024-04-01")]), Value::from("2024-Q2"));
        assert_eq!(call("quarter", vec![on("2024-09-30")]), Value::from("2024-Q3"));
        assert_eq!(call("quarter", vec![on("2024-12-31")]), Value::from("2024-Q4"));
        assert_eq!(call("quarter", vec![on("0999-05-01")]), Value::from("0999-Q2"));
    }

    #[test]
    fn weekday_name() {
        assert_eq!(call("weekday", vec![on("2024-01-05")]), Value::from("Fri"));
        assert_eq!(call("weekday", vec![on("2024-01-07")]), Value::from("Sun"));
        assert_eq!(call("weekday", vec![on("2024-01-08")]), Value::from("Mon"));
    }

    #[test]
    fn yearmonth_truncates_to_the_month() {
        assert_eq!(call("yearmonth", vec![on("2024-02-29")]), on("2024-02-01"));
        assert_eq!(call("yearmonth", vec![on("2024-12-01")]), on("2024-12-01"));
    }

    #[test]
    fn today_comes_from_the_context() {
        assert_eq!(call("today", vec![]), on("2024-06-30"));
        let ctx = TestContext {
            today: date("2025-01-02"),
            ..TestContext::default()
        };
        assert_eq!(call_with(&ctx, "today", vec![]), on("2025-01-02"));
    }
}
