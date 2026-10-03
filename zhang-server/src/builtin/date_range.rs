//! Report ranges as ledger dates (#479, decision 2).

use chrono::{DateTime, FixedOffset, NaiveDate};
use chrono_tz::Tz;
use zhang_query::Params;

use crate::error::ServerError;
use crate::ServerResult;

/// A range of ledger dates, both included: the `from` and `to` of a report.
///
/// A ledger date is a day of the ledger's own calendar, so a range means the same days in
/// every browser. Requests send `YYYY-MM-DD`; an RFC 3339 instant, what older clients send,
/// is still accepted and stands for the day it falls on in the ledger's timezone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LedgerDateRange {
    pub from: NaiveDate,
    pub to: NaiveDate,
}

impl LedgerDateRange {
    /// The range of a request's `from` and `to` (see [`ledger_date`]). A value that is neither
    /// form, or a `from` after `to`, is a 400.
    pub fn from_query(from: &str, to: &str, timezone: &Tz) -> ServerResult<LedgerDateRange> {
        let date = |name: &str, value: &str| ledger_date(value, timezone).map_err(|message| ServerError::InvalidInput(format!("`{}` {}", name, message)));
        let range = LedgerDateRange {
            from: date("from", from)?,
            to: date("to", to)?,
        };
        if range.from > range.to {
            return Err(ServerError::InvalidInput(format!(
                "the range starts on {} (`from`), after it ends on {} (`to`)",
                range.from, range.to
            )));
        }
        Ok(range)
    }

    /// `params` with the range bound to `:from` and `:to`, for a query such as
    /// `WHERE date >= :from AND date <= :to`.
    pub fn bind(&self, params: Params) -> Params {
        params.bind("from", self.from).bind("to", self.to)
    }
}

/// The ledger date a request value stands for: a date `YYYY-MM-DD` as it is, or the day an
/// RFC 3339 instant (such as `2024-01-31T16:00:00Z` or `2024-02-01T00:00:00+08:00`) falls on
/// in the ledger's `timezone`. The error says what was expected.
pub fn ledger_date(value: &str, timezone: &Tz) -> Result<NaiveDate, String> {
    let value = value.trim();
    if let Ok(date) = NaiveDate::parse_from_str(value, "%Y-%m-%d") {
        return Ok(date);
    }
    // the instants the endpoints took before, as chrono reads a `DateTime<Utc>` request field
    match value.parse::<DateTime<FixedOffset>>() {
        Ok(instant) => Ok(instant.with_timezone(timezone).date_naive()),
        Err(_) => Err(format!("must be a date (YYYY-MM-DD) or an RFC 3339 instant, got {:?}", value)),
    }
}

#[cfg(test)]
mod test {
    use chrono::NaiveDate;
    use chrono_tz::Tz;
    use zhang_query::{Params, Value};

    use super::{ledger_date, LedgerDateRange};

    fn date(text: &str) -> NaiveDate {
        NaiveDate::parse_from_str(text, "%Y-%m-%d").unwrap()
    }

    fn day_in(timezone: Tz, value: &str) -> NaiveDate {
        ledger_date(value, &timezone).unwrap_or_else(|err| panic!("{}: {}", value, err))
    }

    #[test]
    fn a_date_is_a_ledger_date_in_every_timezone() {
        for timezone in [
            Tz::UTC,
            Tz::Asia__Shanghai,
            Tz::America__New_York,
            Tz::Pacific__Kiritimati,
            Tz::Pacific__Pago_Pago,
        ] {
            assert_eq!(day_in(timezone, "2024-03-10"), date("2024-03-10"));
            assert_eq!(day_in(timezone, " 2024-02-29\n"), date("2024-02-29"));
        }
    }

    #[test]
    fn an_instant_is_the_day_it_falls_on_in_the_ledger_timezone() {
        let shanghai = Tz::Asia__Shanghai;
        // either side of midnight in Shanghai (UTC+8), which is 16:00 UTC
        assert_eq!(day_in(shanghai, "2024-01-31T15:59:59Z"), date("2024-01-31"));
        assert_eq!(day_in(shanghai, "2024-01-31T16:00:00Z"), date("2024-02-01"));
        assert_eq!(day_in(shanghai, "2024-01-31T23:59:59.999+08:00"), date("2024-01-31"));
        assert_eq!(day_in(shanghai, "2024-02-01T00:00:00+08:00"), date("2024-02-01"));
        // what the old frontend sent for the 1st, from a browser in the ledger's timezone
        assert_eq!(day_in(shanghai, "2024-01-01T00:00:01+08:00"), date("2024-01-01"));
        assert_eq!(day_in(shanghai, "2023-12-31T16:00:01.000Z"), date("2024-01-01"));
        // the same instant is another day in another timezone
        assert_eq!(day_in(Tz::UTC, "2024-01-31T16:00:00Z"), date("2024-01-31"));
        assert_eq!(day_in(Tz::America__Los_Angeles, "2024-02-01T07:59:59Z"), date("2024-01-31"));
        assert_eq!(day_in(Tz::America__Los_Angeles, "2024-02-01T08:00:00Z"), date("2024-02-01"));
        // chrono's other spelling of an instant, which the endpoints accepted before
        assert_eq!(day_in(shanghai, "2024-01-31 16:00:00Z"), date("2024-02-01"));
    }

    #[test]
    fn instants_around_daylight_saving_changes_keep_their_local_day() {
        let new_york = Tz::America__New_York;
        // spring forward on 2024-03-10: 02:00 EST (UTC-5) jumps to 03:00 EDT (UTC-4)
        assert_eq!(day_in(new_york, "2024-03-10T04:59:59Z"), date("2024-03-09"));
        assert_eq!(day_in(new_york, "2024-03-10T05:00:00Z"), date("2024-03-10"));
        assert_eq!(day_in(new_york, "2024-03-10T06:59:59Z"), date("2024-03-10")); // 01:59:59 EST
        assert_eq!(day_in(new_york, "2024-03-10T07:00:00Z"), date("2024-03-10")); // 03:00 EDT
        assert_eq!(day_in(new_york, "2024-03-11T03:59:59Z"), date("2024-03-10")); // 23:59:59 EDT
        assert_eq!(day_in(new_york, "2024-03-11T04:00:00Z"), date("2024-03-11"));
        // fall back on 2024-11-03: 02:00 EDT goes back to 01:00 EST, so 01:30 happens twice
        assert_eq!(day_in(new_york, "2024-11-03T03:59:59Z"), date("2024-11-02"));
        assert_eq!(day_in(new_york, "2024-11-03T04:00:00Z"), date("2024-11-03"));
        assert_eq!(day_in(new_york, "2024-11-03T05:30:00Z"), date("2024-11-03")); // 01:30 EDT
        assert_eq!(day_in(new_york, "2024-11-03T06:30:00Z"), date("2024-11-03")); // 01:30 EST
        assert_eq!(day_in(new_york, "2024-11-04T04:59:59Z"), date("2024-11-03")); // 23:59:59 EST
        assert_eq!(day_in(new_york, "2024-11-04T05:00:00Z"), date("2024-11-04"));
        // a midnight that does not exist: Santiago springs forward from 24:00 to 01:00
        assert_eq!(day_in(Tz::America__Santiago, "2024-09-08T03:59:59Z"), date("2024-09-07"));
        assert_eq!(day_in(Tz::America__Santiago, "2024-09-08T04:00:00Z"), date("2024-09-08"));
        // 01:00 -03
    }

    #[test]
    fn a_range_is_two_ledger_dates_in_order() {
        let shanghai = Tz::Asia__Shanghai;
        let range = LedgerDateRange::from_query("2024-01-01", "2024-01-31", &shanghai).unwrap();
        assert_eq!((range.from, range.to), (date("2024-01-01"), date("2024-01-31")));
        // a single day
        let range = LedgerDateRange::from_query("2023-12-31T16:00:00Z", "2024-01-01", &shanghai).unwrap();
        assert_eq!((range.from, range.to), (date("2024-01-01"), date("2024-01-01")));
        // the old frontend's month: its first second to its last, in the browser's timezone
        let range = LedgerDateRange::from_query("2024-01-01T00:00:01+08:00", "2024-01-31T23:59:59+08:00", &shanghai).unwrap();
        assert_eq!((range.from, range.to), (date("2024-01-01"), date("2024-01-31")));

        let params = range.bind(Params::new().bind("account", "Assets"));
        assert_eq!(params.get(&zhang_query::ParamRef::Named("from".into())), Some(&Value::Date(date("2024-01-01"))));
        assert_eq!(params.get(&zhang_query::ParamRef::Named("to".into())), Some(&Value::Date(date("2024-01-31"))));
        assert_eq!(params.types().len(), 3);
    }

    #[test]
    fn a_bad_range_is_a_400() {
        let error = |from: &str, to: &str| LedgerDateRange::from_query(from, to, &Tz::UTC).unwrap_err();
        assert_eq!(
            error("2024-02-01", "2024-01-31").to_string(),
            "the range starts on 2024-02-01 (`from`), after it ends on 2024-01-31 (`to`)"
        );
        assert_eq!(
            error("2024-02-30", "2024-03-01").to_string(),
            "`from` must be a date (YYYY-MM-DD) or an RFC 3339 instant, got \"2024-02-30\""
        );
        assert_eq!(
            error("2024-01-01", "2024-01-31T00:00:00").to_string(),
            "`to` must be a date (YYYY-MM-DD) or an RFC 3339 instant, got \"2024-01-31T00:00:00\""
        );
        assert!(matches!(error("", "2024-01-01"), crate::error::ServerError::InvalidInput(_)));
    }
}
