//! Date functions: `month`, `day`, `quarter`, `weekday`, `yearmonth` and `today`
//! (`year` lives with the registry examples), and beanquery 0.2.0's `date`, `date_add`,
//! `date_diff`, `date_trunc`, `date_part`, `interval` and `date_bin`.
//!
//! The beanquery functions follow beanquery (checked against beanquery 0.2.0 by the
//! conformance fixtures): an unknown field is NULL and an invalid date is NULL. Where
//! beanquery fails with an exception (a zero stride, an unparsable stride given as text),
//! the result is NULL, and NULL arguments give NULL (beanquery rejects a NULL literal).
//! Two deliberate differences: `date_bin` lays its bins from the origin without
//! beanquery's off-by-one at bin boundaries (see [`bin`]), and `interval` also accepts
//! weeks.
//!
//! Dates are those of beancount's calendar, years 1 to 9999: a function whose result would
//! fall outside, such as `date_add(9999-12-31, 1)` or `date_trunc('decade', 0002-12-15)`, is
//! NULL (beanquery raises an error).

use chrono::{Datelike, Duration, NaiveDate, Weekday};

use crate::functions::FunctionContext;
use crate::value::{calendar_value, in_calendar, Interval, Value};

pub(super) fn month(args: &[Value], _ctx: &dyn FunctionContext) -> Result<Value, String> {
    Ok(Value::Int(date_of(&args[0], "month")?.month() as i64))
}

pub(super) fn day(args: &[Value], _ctx: &dyn FunctionContext) -> Result<Value, String> {
    Ok(Value::Int(date_of(&args[0], "day")?.day() as i64))
}

/// beanquery `quarter`: `YYYY-Qn`, e.g. `2024-Q1`.
pub(super) fn quarter(args: &[Value], _ctx: &dyn FunctionContext) -> Result<Value, String> {
    let date = date_of(&args[0], "quarter")?;
    Ok(Value::Str(format!("{:04}-Q{}", date.year(), (date.month() - 1) / 3 + 1)))
}

/// beanquery `weekday`: the three-letter English day name (`strftime('%a')` in the C locale).
pub(super) fn weekday(args: &[Value], _ctx: &dyn FunctionContext) -> Result<Value, String> {
    let name = match date_of(&args[0], "weekday")?.weekday() {
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
    let date = date_of(&args[0], "yearmonth")?;
    Ok(Value::Date(date.with_day(1).ok_or("yearmonth() got an invalid date")?))
}

pub(super) fn today(_args: &[Value], ctx: &dyn FunctionContext) -> Result<Value, String> {
    Ok(Value::Date(ctx.today()))
}

fn int_arg(value: &Value, function: &str) -> Result<i64, String> {
    value.as_int().ok_or_else(|| format!("{}() expects an integer", function))
}

fn str_arg<'a>(value: &'a Value, function: &str) -> Result<&'a str, String> {
    value.as_str().ok_or_else(|| format!("{}() expects a string", function))
}

fn date_of(value: &Value, function: &str) -> Result<NaiveDate, String> {
    value.as_date().ok_or_else(|| format!("{}() expects a date", function))
}

/// A date of Python's calendar (years 1 to 9999), as `datetime.date` builds it.
fn python_date(year: i64, month: i64, day: i64) -> Option<NaiveDate> {
    if !(1..=9999).contains(&year) {
        return None;
    }
    NaiveDate::from_ymd_opt(year as i32, u32::try_from(month).ok()?, u32::try_from(day).ok()?)
}

/// beanquery `date(year, month, day)`: NULL when there is no such day (or the year is outside
/// 1 to 9999, Python's calendar).
pub(super) fn date_from_ymd(args: &[Value], _ctx: &dyn FunctionContext) -> Result<Value, String> {
    let [year, month, day] = [&args[0], &args[1], &args[2]].map(|it| int_arg(it, "date"));
    Ok(python_date(year?, month?, day?).map_or(Value::Null, Value::Date))
}

/// beanquery `date(text)`: Python's `strptime(text, '%Y-%m-%d')`, NULL when it does not
/// parse: a four-digit year, a month of one or two digits and a day of one or two digits
/// (or a space and one digit), nothing else around them.
pub(super) fn date_from_str(args: &[Value], _ctx: &dyn FunctionContext) -> Result<Value, String> {
    Ok(parse_python_date(str_arg(&args[0], "date")?).map_or(Value::Null, Value::Date))
}

fn parse_python_date(text: &str) -> Option<NaiveDate> {
    let mut parts = text.splitn(3, '-');
    let (year, month, day) = (parts.next()?, parts.next()?, parts.next()?);
    let digits = |part: &str| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit());
    if year.len() != 4 || !digits(year) {
        return None;
    }
    // %m: 1[0-2] | 0[1-9] | [1-9]
    if !(1..=2).contains(&month.len()) || !digits(month) || month == "0" || month == "00" {
        return None;
    }
    // %d: 3[01] | [12]\d | 0[1-9] | [1-9] | ' '[1-9]
    let day = match day.strip_prefix(' ') {
        Some(rest) if rest.len() == 1 => rest,
        Some(_) => return None,
        None => day,
    };
    if !(1..=2).contains(&day.len()) || !digits(day) || day == "0" || day == "00" {
        return None;
    }
    let (month, day) = (month.parse::<i64>().ok()?, day.parse::<i64>().ok()?);
    if month > 12 || day > 31 {
        return None;
    }
    python_date(year.parse().ok()?, month, day)
}

/// beanquery `date_add(date, days)`; NULL when the result is outside the calendar.
pub(super) fn date_add(args: &[Value], _ctx: &dyn FunctionContext) -> Result<Value, String> {
    let date = date_of(&args[0], "date_add")?;
    let days = int_arg(&args[1], "date_add")?;
    Ok(calendar_value(Duration::try_days(days).and_then(|days| date.checked_add_signed(days))))
}

/// beanquery `date_diff(a, b)`: the days from `b` to `a`.
pub(super) fn date_diff(args: &[Value], _ctx: &dyn FunctionContext) -> Result<Value, String> {
    let (a, b) = (date_of(&args[0], "date_diff")?, date_of(&args[1], "date_diff")?);
    Ok(Value::Int((a - b).num_days()))
}

/// beanquery `date_trunc(field, date)`: the first day of the date's week (Monday), month,
/// quarter, year, decade, century (years 1901, 2001, ...) or millennium (1001, 2001, ...);
/// NULL for any other field (fields are lower-case).
pub(super) fn date_trunc(args: &[Value], _ctx: &dyn FunctionContext) -> Result<Value, String> {
    let field = str_arg(&args[0], "date_trunc")?;
    let date = date_of(&args[1], "date_trunc")?;
    let (year, month) = (date.year(), date.month());
    let first = |year: i32, month: u32| NaiveDate::from_ymd_opt(year, month, 1);
    let truncated = match field {
        "week" => date.checked_sub_signed(Duration::days(date.weekday().num_days_from_monday() as i64)),
        "month" => first(year, month),
        "quarter" => first(year, month - (month - 1) % 3),
        "year" => first(year, 1),
        "decade" => first(year - year.rem_euclid(10), 1),
        "century" => first(year - (year - 1).rem_euclid(100), 1),
        "millennium" => first(year - (year - 1).rem_euclid(1000), 1),
        _ => None,
    };
    Ok(calendar_value(truncated))
}

/// beanquery `date_part(field, date)`: `weekday`/`dow` (Monday 0 to Sunday 6),
/// `isoweekday`/`isodow` (Monday 1 to Sunday 7), `week` (the ISO week), `month`, `quarter`,
/// `year`, `isoyear`, `decade`, `century`, `millennium` or `epoch` (seconds since
/// 1970-01-01); NULL for any other field.
pub(super) fn date_part(args: &[Value], _ctx: &dyn FunctionContext) -> Result<Value, String> {
    let field = str_arg(&args[0], "date_part")?;
    let date = date_of(&args[1], "date_part")?;
    let year = date.year() as i64;
    let part = match field {
        "weekday" | "dow" => date.weekday().num_days_from_monday() as i64,
        "isoweekday" | "isodow" => date.weekday().number_from_monday() as i64,
        "week" => date.iso_week().week() as i64,
        "month" => date.month() as i64,
        "quarter" => (date.month() as i64 - 1) / 3 + 1,
        "year" => year,
        "isoyear" => date.iso_week().year() as i64,
        "decade" => year.div_euclid(10),
        "century" => (year - 1).div_euclid(100) + 1,
        "millennium" => (year - 1).div_euclid(1000) + 1,
        "epoch" => (date - NaiveDate::from_ymd_opt(1970, 1, 1).expect("valid date")).num_days() * 86400,
        _ => return Ok(Value::Null),
    };
    Ok(Value::Int(part))
}

/// beanquery `interval(text)`: `<n> day[s]`, `<n> week[s]` (a zhang extension: seven days
/// each), `<n> month[s]` or `<n> year[s]`, with an optional sign and whitespace between
/// number and unit; NULL for anything else (other units, leading or trailing spaces, upper
/// case, a number out of range).
pub(super) fn interval(args: &[Value], _ctx: &dyn FunctionContext) -> Result<Value, String> {
    Ok(parse_interval(str_arg(&args[0], "interval")?).map_or(Value::Null, Value::Interval))
}

pub(crate) fn parse_interval(text: &str) -> Option<Interval> {
    let number_end = text.find(|c: char| c.is_whitespace())?;
    let (number, rest) = text.split_at(number_end);
    let unit = rest.trim_start();
    if unit.len() == rest.len() || unit.is_empty() {
        return None;
    }
    let digits = number.strip_prefix(['+', '-']).unwrap_or(number);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let number = number.parse::<i64>().ok()?;
    match unit.strip_suffix('s').unwrap_or(unit) {
        "day" => Some(Interval::new(0, number)),
        // a zhang extension: beanquery has no weeks
        "week" => Some(Interval::new(0, number.checked_mul(7)?)),
        "month" => Some(Interval::new(number, 0)),
        "year" => Some(Interval::new(number.checked_mul(12)?, 0)),
        _ => None,
    }
}

/// `date_bin(stride, source, origin)`: the start of the bin of `source` among the bins of
/// `stride` laid from `origin` (see [`bin`]).
pub(super) fn date_bin(args: &[Value], _ctx: &dyn FunctionContext) -> Result<Value, String> {
    let stride = match &args[0] {
        Value::Interval(stride) => *stride,
        Value::Str(text) => match parse_interval(text) {
            Some(stride) => stride,
            // beanquery fails here (it bins with a missing stride)
            None => return Ok(Value::Null),
        },
        _ => return Err("date_bin() expects an interval".to_owned()),
    };
    let (source, origin) = (date_of(&args[1], "date_bin")?, date_of(&args[2], "date_bin")?);
    Ok(bin(stride, source, origin).map_or(Value::Null, Value::Date))
}

/// The month index `year * 12 + month0` of a date.
fn month_index(date: NaiveDate) -> i64 {
    date.year() as i64 * 12 + date.month0() as i64
}

/// How many times [`bin`] may move its estimate. The estimate is at most two strides off
/// (see [`bin`]), so this is never reached; it only bounds the work for any input.
const BIN_SEARCH_STEPS: usize = 16;

/// `date_bin`: the bins are the origin moved by every whole number `k` of strides, each
/// computed from the origin at once (`origin + k × stride`, so bins from a month end stay
/// on month ends: 01-31, 02-29, 03-31, ...), and a date belongs to the last bin that starts
/// on or before it. A date on a boundary starts its bin, and dates before the origin fall
/// in bins laid backwards from it.
///
/// NULL for a stride that does not move forward (zero or negative), for a stride whose
/// months and days have opposite signs (`interval('2 months') - interval('61 days')`: its
/// bins need not grow with `k`, so a date has no single bin), and when a bin falls outside the
/// calendar.
///
/// Every other stride has months and days of one sign, so `origin + k × stride` grows with
/// `k`, and lies within six days of `k` times the stride's average length (at least 30 days
/// with months): the estimate below is at most two strides off, and the search takes a few
/// steps whatever the dates.
///
/// beanquery 0.2.0 walks the bins of a stride with months one stride at a time instead (so
/// they drift from a month end: 01-31, 02-29, 03-29, ...) and puts a date on a boundary into
/// the previous bin; zhang does neither (see the accepted deviations of the conformance
/// suite).
fn bin(stride: Interval, source: NaiveDate, origin: NaiveDate) -> Option<NaiveDate> {
    if stride.months == 0 {
        if stride.days <= 0 {
            return None;
        }
        let diff = (source - origin).num_days();
        let start = diff - diff.rem_euclid(stride.days);
        return origin.checked_add_signed(Duration::try_days(start)?).filter(|it| in_calendar(*it));
    }
    if stride.months.signum() * stride.days.signum() < 0 {
        return None;
    }
    let nth = |k: i64| Interval::new(stride.months.checked_mul(k)?, stride.days.checked_mul(k)?).add_to(origin);
    if nth(1)? <= origin {
        return None;
    }
    // an estimate of the bin, then the exact one: nth(k) <= source < nth(k + 1)
    let mut k = if stride.days == 0 {
        (month_index(source) - month_index(origin)).div_euclid(stride.months)
    } else {
        let length = stride.months as f64 * 30.436875 + stride.days as f64;
        ((source - origin).num_days() as f64 / length).floor() as i64
    };
    for _ in 0..BIN_SEARCH_STEPS {
        if nth(k)? > source {
            k -= 1;
        } else if nth(k + 1)? <= source {
            k += 1;
        } else {
            return nth(k).filter(|it| in_calendar(*it));
        }
    }
    None
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

    fn interval_of(text: &str) -> Value {
        call("interval", vec![text.into()])
    }

    #[test]
    fn date_trunc_and_date_part_fields() {
        let trunc = |field: &str, day: &str| call("date_trunc", vec![field.into(), on(day)]);
        assert_eq!(trunc("week", "2016-01-03"), on("2015-12-28"));
        assert_eq!(trunc("week", "2016-01-04"), on("2016-01-04"));
        assert_eq!(trunc("month", "2016-02-29"), on("2016-02-01"));
        assert_eq!(trunc("quarter", "2016-12-31"), on("2016-10-01"));
        assert_eq!(trunc("year", "2016-12-31"), on("2016-01-01"));
        assert_eq!(trunc("decade", "2009-05-05"), on("2000-01-01"));
        assert_eq!(trunc("century", "2000-12-31"), on("1901-01-01"));
        assert_eq!(trunc("millennium", "2001-01-01"), on("2001-01-01"));
        assert_eq!(trunc("day", "2016-01-01"), Value::Null);
        assert_eq!(trunc("Month", "2016-01-01"), Value::Null);
        let part = |field: &str, day: &str| call("date_part", vec![field.into(), on(day)]);
        assert_eq!(part("weekday", "2016-01-03"), Value::Int(6));
        assert_eq!(part("isodow", "2016-01-03"), Value::Int(7));
        assert_eq!(part("week", "2016-01-03"), Value::Int(53));
        assert_eq!(part("isoyear", "2016-01-03"), Value::Int(2015));
        assert_eq!(part("quarter", "2016-11-03"), Value::Int(4));
        assert_eq!(part("decade", "2009-05-05"), Value::Int(200));
        assert_eq!(part("century", "2000-01-03"), Value::Int(20));
        assert_eq!(part("millennium", "2001-01-03"), Value::Int(3));
        assert_eq!(part("epoch", "1969-12-31"), Value::Int(-86400));
        assert_eq!(part("day", "2016-01-03"), Value::Null);
    }

    #[test]
    fn date_constructors_follow_python() {
        let ymd = |y: i64, m: i64, d: i64| call("date", vec![Value::Int(y), Value::Int(m), Value::Int(d)]);
        assert_eq!(ymd(2016, 2, 29), on("2016-02-29"));
        assert_eq!(ymd(2015, 2, 29), Value::Null);
        assert_eq!(ymd(2016, 13, 1), Value::Null);
        assert_eq!(ymd(0, 1, 1), Value::Null);
        assert_eq!(ymd(10_000, 1, 1), Value::Null);
        assert_eq!(ymd(i64::MAX, i64::MIN, -1), Value::Null);
        for (text, expected) in [
            ("2016-02-29", Some("2016-02-29")),
            ("2016-2-9", Some("2016-02-09")),
            ("2016-02- 9", Some("2016-02-09")),
            ("2016-02-09", Some("2016-02-09")),
            ("2015-02-29", None),
            ("016-02-09", None),
            (" 2016-02-09", None),
            ("2016-02-09 ", None),
            ("2016-02-09x", None),
            ("2016-002-09", None),
            ("2016-00-09", None),
            ("2016-02-00", None),
            ("2016-02- 0", None),
            ("2016-02-  9", None),
            ("2016-13-01", None),
            ("20160209", None),
            ("+016-02-09", None),
            ("", None),
        ] {
            assert_eq!(call("date", vec![text.into()]), expected.map_or(Value::Null, on), "{text:?}");
        }
    }

    #[test]
    fn date_add_and_date_diff() {
        assert_eq!(call("date_add", vec![on("2016-02-28"), Value::Int(1)]), on("2016-02-29"));
        assert_eq!(call("date_add", vec![on("2016-03-01"), Value::Int(-1)]), on("2016-02-29"));
        assert_eq!(call("date_diff", vec![on("2016-01-01"), on("2016-12-31")]), Value::Int(-365));
        // outside the calendar (years 1 to 9999): NULL
        assert_eq!(call("date_add", vec![on("2016-01-01"), Value::Int(i64::MAX)]), Value::Null);
        assert_eq!(call("date_add", vec![on("9999-12-31"), Value::Int(1)]), Value::Null);
        assert_eq!(call("date_add", vec![on("0001-01-01"), Value::Int(-1)]), Value::Null);
        assert_eq!(call("date_add", vec![on("9999-12-30"), Value::Int(1)]), on("9999-12-31"));
        assert_eq!(call("date_trunc", vec!["decade".into(), on("0002-12-15")]), Value::Null);
        assert_eq!(call("date_trunc", vec!["week".into(), on("0001-01-01")]), on("0001-01-01"));
        assert_eq!(call("date_trunc", vec!["week".into(), on("0001-01-06")]), on("0001-01-01"));
        assert_eq!(call("date_trunc", vec!["century".into(), on("0099-12-31")]), on("0001-01-01"));
        assert_eq!(call("date_bin", vec!["1 year".into(), on("9999-06-01"), on("0001-07-01")]), on("9998-07-01"));
        assert_eq!(call("date_bin", vec!["1 year".into(), on("0001-03-01"), on("0002-06-01")]), Value::Null);
    }

    #[test]
    fn intervals_parse_like_beanquery_plus_weeks() {
        for (text, expected) in [
            ("1 month", Some((1, 0))),
            ("13 months", Some((13, 0))),
            ("-1 year", Some((-12, 0))),
            ("+10 days", Some((0, 10))),
            ("1  days", Some((0, 1))),
            ("1\tday", Some((0, 1))),
            ("2 weeks", Some((0, 14))),
            ("-1 week", Some((0, -7))),
            ("1 Month", None),
            (" 1 day", None),
            ("1 day ", None),
            ("1 dayss", None),
            ("1day", None),
            ("1 s", None),
            ("one day", None),
            ("1.5 days", None),
            ("99999999999999999999 days", None),
            ("999999999999999999 years", None),
            ("", None),
        ] {
            let expected = expected.map_or(Value::Null, |(months, days)| Value::Interval(Interval::new(months, days)));
            assert_eq!(interval_of(text), expected, "{text:?}");
        }
        assert_eq!(interval_of("13 months").to_string(), "1 year 1 month");
        assert_eq!(Interval::new(-13, -1).to_string(), "-1 year -1 month -1 day");
        assert_eq!(Interval::new(0, 0).to_string(), "0 days");
    }

    fn bin_of(stride: &str, source: &str, origin: &str) -> Value {
        call("date_bin", vec![stride.into(), on(source), on(origin)])
    }

    /// A stride whose months and days have opposite signs has no ordered bins: NULL, at once.
    #[test]
    fn date_bin_rejects_strides_of_mixed_signs() {
        let mixed = |months, days| {
            call(
                "date_bin",
                vec![Value::Interval(Interval::new(months, days)), on("2016-03-30"), on("0001-01-01")],
            )
        };
        assert_eq!(mixed(2, -61), Value::Null);
        assert_eq!(mixed(1, -1), Value::Null);
        assert_eq!(mixed(-1, 40), Value::Null);
        assert_eq!(mixed(-1, -1), Value::Null);
        // one sign: the last of origin + k × (1 month 1 day) on or before the date
        let origin = date("0001-01-01");
        let at = |k: i64| Interval::new(k, k).add_to(origin).unwrap();
        let k = (0..).find(|k| at(k + 1) > date("2016-03-30")).unwrap();
        assert_eq!(mixed(1, 1), Value::Date(at(k)));
        // a far origin takes as few steps as a near one
        let start = std::time::Instant::now();
        for _ in 0..1000 {
            assert_ne!(mixed(1, 3), Value::Null);
        }
        assert!(start.elapsed() < std::time::Duration::from_secs(5));
    }

    #[test]
    fn date_bin_starts_bins_on_their_boundary() {
        assert_eq!(bin_of("1 month", "2000-02-01", "2000-01-01"), on("2000-02-01"));
        assert_eq!(bin_of("1 month", "2000-03-01", "2000-01-01"), on("2000-03-01"));
        assert_eq!(bin_of("1 month", "2000-02-29", "2000-01-01"), on("2000-02-01"));
        assert_eq!(bin_of("1 month", "2000-01-01", "2000-01-01"), on("2000-01-01"));
        assert_eq!(bin_of("3 months", "2015-07-01", "2015-01-01"), on("2015-07-01"));
        assert_eq!(bin_of("3 months", "2015-06-30", "2015-01-01"), on("2015-04-01"));
        assert_eq!(bin_of("1 year", "2016-01-01", "2015-01-01"), on("2016-01-01"));
        // before the origin the bins go backwards
        assert_eq!(bin_of("1 month", "1999-12-31", "2000-01-01"), on("1999-12-01"));
        assert_eq!(bin_of("1 month", "1999-12-01", "2000-01-01"), on("1999-12-01"));
        assert_eq!(bin_of("1 year", "1990-06-01", "2000-03-01"), on("1990-03-01"));
        // origin + k strides: month ends stay month ends
        assert_eq!(bin_of("1 month", "2015-03-30", "2015-01-31"), on("2015-02-28"));
        assert_eq!(bin_of("1 month", "2015-03-31", "2015-01-31"), on("2015-03-31"));
        assert_eq!(bin_of("1 month", "2016-02-29", "2015-01-31"), on("2016-02-29"));
        assert_eq!(bin_of("1 month", "2014-11-15", "2015-01-31"), on("2014-10-31"));
        assert_eq!(bin_of("1 year", "2017-02-28", "2016-02-29"), on("2017-02-28"));
        assert_eq!(bin_of("1 year", "2020-02-29", "2016-02-29"), on("2020-02-29"));
        // days: floor division from the origin
        assert_eq!(bin_of("7 days", "2015-01-12", "2015-01-05"), on("2015-01-12"));
        assert_eq!(bin_of("7 days", "2015-01-11", "2015-01-05"), on("2015-01-05"));
        assert_eq!(bin_of("7 days", "2015-01-04", "2015-01-05"), on("2014-12-29"));
        assert_eq!(bin_of("2 weeks", "2015-01-19", "2015-01-05"), on("2015-01-19"));
        // strides that do not move forward, and text that is no interval
        for stride in ["0 days", "0 months", "-7 days", "-1 month", "1 fortnight"] {
            assert_eq!(bin_of(stride, "2015-03-30", "2015-01-05"), Value::Null, "{stride}");
        }
        // NULL in, NULL out
        assert_eq!(call("date_bin", vec![Value::Null, on("2015-03-30"), on("2015-01-05")]), Value::Null);
        assert_eq!(call("date_bin", vec!["1 month".into(), Value::Null, on("2015-01-05")]), Value::Null);
        assert_eq!(call("date_trunc", vec![Value::Null, on("2015-01-05")]), Value::Null);
        assert_eq!(call("date_add", vec![Value::Null, Value::Int(1)]), Value::Null);
        assert_eq!(call("interval", vec![Value::Null]), Value::Null);
    }

    /// `date_bin` against its definition: the largest `k` with `origin + k × stride <= source`,
    /// found by walking `k`, for strides of months, of days and of both.
    #[test]
    fn date_bin_is_the_last_bin_on_or_before_the_date() {
        let mut state = 7u64;
        let mut next = move |bound: i64| {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((state >> 33) % bound as u64) as i64
        };
        let base = date("2000-01-01");
        for _ in 0..3000 {
            let origin = base + Duration::days(next(3000));
            let source = base + Duration::days(next(3000));
            let stride = match next(3) {
                0 => Interval::new(next(25) + 1, 0),
                1 => Interval::new(0, next(60) + 1),
                _ => Interval::new(next(3) + 1, next(20)),
            };
            let at = |k: i64| Interval::new(stride.months * k, stride.days * k).add_to(origin).unwrap();
            // a stride is at least a day; mixed ones at least 22 days
            let shortest = if stride.months == 0 { stride.days } else { 22 };
            let mut k = -3001 / shortest - 1;
            assert!(at(k) <= source);
            while at(k + 1) <= source {
                k += 1;
            }
            let got = call("date_bin", vec![Value::Interval(stride), Value::Date(source), Value::Date(origin)]);
            assert_eq!(got, Value::Date(at(k)), "{stride} {source} {origin}");
        }
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
