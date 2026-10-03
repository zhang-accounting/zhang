//! The current time, as one load sees it.
//!
//! A load reads its [`Clock`] at most once, on first use, through a [`LoadClock`]: every stage and plugin of the
//! load that asks for the time gets the same instant, so they all agree on "today". A load that never asks never
//! reads the clock, which matters on targets without one (`wasm32-unknown-unknown`, where the playground runs no
//! plugins). A stage that asks records [`ExtraInput::Clock`](crate::inputs::ExtraInput::Clock), so a server
//! knows the ledger goes stale when the date changes.

use std::sync::{Arc, OnceLock};

use chrono::{DateTime, Utc};

/// where a load reads the current time from
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Clock {
    /// the system clock
    #[default]
    System,
    /// always this instant, e.g. to reproduce a load or to test a plugin that reads the date
    Fixed(DateTime<Utc>),
}

impl Clock {
    /// the current time by this clock; [`Clock::System`] reads the system clock
    pub fn read(&self) -> DateTime<Utc> {
        match self {
            Clock::System => Utc::now(),
            Clock::Fixed(instant) => *instant,
        }
    }
}

/// the current time of one load: a [`Clock`] read at most once, on first use. Clones share the reading
#[derive(Debug, Clone, Default)]
pub struct LoadClock {
    clock: Clock,
    reading: Arc<OnceLock<DateTime<Utc>>>,
}

impl LoadClock {
    pub fn new(clock: Clock) -> Self {
        Self {
            clock,
            reading: Arc::default(),
        }
    }

    /// the clock this load reads
    pub fn clock(&self) -> Clock {
        self.clock
    }

    /// the current time of the load. The first call reads the clock; every later call, on this value or a clone,
    /// returns the same instant
    pub fn now(&self) -> DateTime<Utc> {
        *self.reading.get_or_init(|| self.clock.read())
    }

    /// the current time of the load if something read it already; never reads the clock
    pub fn reading(&self) -> Option<DateTime<Utc>> {
        self.reading.get().copied()
    }
}

#[cfg(test)]
mod test {
    use chrono::{DateTime, Utc};

    use super::{Clock, LoadClock};

    fn instant(rfc3339: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(rfc3339).unwrap().with_timezone(&Utc)
    }

    #[test]
    fn should_read_a_fixed_clock_as_its_instant() {
        let fixed = instant("2024-03-15T16:30:00Z");

        assert_eq!(Clock::Fixed(fixed).read(), fixed);
    }

    #[test]
    fn should_not_read_the_clock_before_first_use() {
        let clock = LoadClock::new(Clock::System);

        assert_eq!(clock.reading(), None);
        assert_eq!(clock.clone().reading(), None);
    }

    #[test]
    fn should_read_the_clock_once_and_share_the_reading_with_clones() {
        let clock = LoadClock::new(Clock::System);
        let clone = clock.clone();

        let first = clone.now();

        assert_eq!(clock.reading(), Some(first));
        std::thread::sleep(std::time::Duration::from_millis(5));
        assert_eq!(clock.now(), first, "a later call returns the first reading");
        assert_eq!(LoadClock::new(Clock::System).reading(), None, "a new load reads the clock again");
    }

    #[test]
    fn should_return_the_fixed_instant_for_a_load_on_a_fixed_clock() {
        let fixed = instant("2024-03-15T16:30:00Z");
        let clock = LoadClock::new(Clock::Fixed(fixed));

        assert_eq!(clock.now(), fixed);
        assert_eq!(clock.reading(), Some(fixed));
        assert_eq!(clock.clock(), Clock::Fixed(fixed));
    }
}
