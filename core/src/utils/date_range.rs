use std::mem;

use chrono::{Duration, NaiveDate};

pub struct NaiveDateRange(NaiveDate, NaiveDate);

impl NaiveDateRange {
    pub fn new(from: NaiveDate, to: NaiveDate) -> Self {
        Self(from, to)
    }
}

impl Iterator for NaiveDateRange {
    type Item = NaiveDate;
    fn next(&mut self) -> Option<Self::Item> {
        if self.0 <= self.1 {
            let next = self.0 + Duration::days(1);
            Some(mem::replace(&mut self.0, next))
        } else {
            None
        }
    }
}
