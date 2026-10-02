//! Turning wall-clock (local) times into time-zone-aware instants.

use chrono::{DateTime, LocalResult, NaiveDateTime, Offset, TimeDelta, TimeZone};

/// Resolve the wall-clock time `local` in `timezone` to a concrete instant. Never panics.
///
/// Ledgers record naive local times. Around a daylight-saving transition some local
/// times occur twice (an *overlap*, when clocks are set back) and some never occur at
/// all (a *gap*, when clocks are set forward), so chrono's
/// [`TimeZone::from_local_datetime`] answers [`LocalResult::Ambiguous`] or
/// [`LocalResult::None`] instead of a single instant, and unwrapping it panics.
///
/// This helper follows the `fold=0` rule of [PEP 495], the same rule Python's
/// `zoneinfo` applies:
///
/// - **unique**: the one matching instant.
/// - **ambiguous** (overlap): the earlier of the two instants. In `America/New_York`,
///   `2023-11-05 01:30` resolves to `01:30 EDT` (`05:30 UTC`), not `01:30 EST`.
/// - **non-existent** (gap): the time is read with the UTC offset in effect *before*
///   the transition, which lands it after the gap, shifted forward by the gap's length.
///   In `America/New_York`, `2023-03-12 02:30` resolves to `02:30 EST` = `07:30 UTC`,
///   shown as `03:30 EDT`.
///
/// A date-only value is converted at local midnight, which is a gap time in zones
/// that switch at midnight (e.g. `America/Santiago`), so dates go through here too.
///
/// [PEP 495]: https://peps.python.org/pep-0495/
pub fn resolve_local_datetime<Tz: TimeZone>(timezone: &Tz, local: &NaiveDateTime) -> DateTime<Tz> {
    match timezone.from_local_datetime(local) {
        LocalResult::Single(datetime) => datetime,
        LocalResult::Ambiguous(first, second) => first.min(second),
        LocalResult::None => resolve_gap(timezone, local),
    }
}

/// Resolve a `local` time that falls in a gap: read it with the offset in effect
/// before the transition that opened the gap.
fn resolve_gap<Tz: TimeZone>(timezone: &Tz, local: &NaiveDateTime) -> DateTime<Tz> {
    // The transition happened at the UTC instant `local - offset_after`, and every UTC
    // offset is shorter than a day, so the UTC instant `local - 1 day` lies before it.
    // Transitions are months apart, so the offset there is the one the gap began from.
    let offset_before = local
        .checked_sub_signed(TimeDelta::days(1))
        .map(|probe| timezone.offset_from_utc_datetime(&probe).fix());
    // Only out-of-range dates at the very ends of chrono's calendar fail the arithmetic;
    // read them as UTC rather than panic.
    let utc = offset_before.and_then(|offset| local.checked_sub_offset(offset)).unwrap_or(*local);
    timezone.from_utc_datetime(&utc)
}

#[cfg(test)]
mod test {
    use chrono::{LocalResult, NaiveDate, NaiveDateTime, TimeZone, Utc};
    use chrono_tz::Tz;

    use super::resolve_local_datetime;

    fn naive(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(year, month, day).unwrap().and_hms_opt(hour, minute, 0).unwrap()
    }

    fn utc(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> chrono::DateTime<Utc> {
        naive(year, month, day, hour, minute).and_utc()
    }

    #[test]
    fn gap_in_new_york_uses_the_offset_before_the_transition() {
        let tz = Tz::America__New_York;
        // clocks jump 02:00 EST -> 03:00 EDT on 2023-03-12, so 02:30 does not exist
        assert_eq!(tz.from_local_datetime(&naive(2023, 3, 12, 2, 30)), LocalResult::None);
        let resolved = resolve_local_datetime(&tz, &naive(2023, 3, 12, 2, 30));
        assert_eq!(resolved, utc(2023, 3, 12, 7, 30), "02:30 EST is 07:30 UTC");
        assert_eq!(resolved.naive_local(), naive(2023, 3, 12, 3, 30), "shown as 03:30 EDT");
    }

    #[test]
    fn overlap_in_new_york_takes_the_earlier_instant() {
        let tz = Tz::America__New_York;
        // clocks fall back 02:00 EDT -> 01:00 EST on 2023-11-05, so 01:30 happens twice
        assert!(matches!(tz.from_local_datetime(&naive(2023, 11, 5, 1, 30)), LocalResult::Ambiguous(..)));
        let resolved = resolve_local_datetime(&tz, &naive(2023, 11, 5, 1, 30));
        assert_eq!(resolved, utc(2023, 11, 5, 5, 30), "01:30 EDT, not 01:30 EST (06:30 UTC)");
        assert_eq!(resolved.naive_local(), naive(2023, 11, 5, 1, 30));
    }

    #[test]
    fn midnight_gap_in_santiago_for_a_date_only_value() {
        let tz = Tz::America__Santiago;
        // clocks jump 00:00 -04 -> 01:00 -03 on 2023-09-03, so that day has no midnight
        assert_eq!(tz.from_local_datetime(&naive(2023, 9, 3, 0, 0)), LocalResult::None);
        let date = crate::Date::Date(NaiveDate::from_ymd_opt(2023, 9, 3).unwrap());
        let resolved = date.to_timezone_datetime(&tz);
        assert_eq!(resolved, utc(2023, 9, 3, 4, 0), "00:00 -04 is 04:00 UTC");
        assert_eq!(resolved.naive_local(), naive(2023, 9, 3, 1, 0), "shown as 01:00 -03");
    }

    #[test]
    fn normal_times_are_unchanged() {
        let tz = Tz::America__New_York;
        let local = naive(2023, 7, 1, 12, 0);
        let resolved = resolve_local_datetime(&tz, &local);
        assert_eq!(resolved, utc(2023, 7, 1, 16, 0));
        assert_eq!(resolved.naive_local(), local);

        let resolved = resolve_local_datetime(&Tz::UTC, &local);
        assert_eq!(resolved, utc(2023, 7, 1, 12, 0));
    }
}
