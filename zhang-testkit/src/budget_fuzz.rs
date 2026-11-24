//! Seeded ledgers for the differential fuzzing of budgets (RFC #698): small zhang ledgers, a function of their seed
//! alone, that use every budget directive and break the budgets in every way zhang reports, so that the bundled script
//! `zhang:budget`'s answers can be compared on them with the oracle's, the SDK's budget analyzer
//! ([`crate::budget_parity`]).
//!
//! A seed picks, among others:
//!
//! - the timezone, the operating currency and the clock of the load: in the ledger's months, after them, or before
//!   its last transaction;
//! - on some seeds, a timezone with daylight-saving changes, and directives of every timed kind (postings, budget
//!   definitions, adds, transfers and closes, account opens and closes) on its change days between 01:00 and 03:59:
//!   at times the change skips, times it repeats, and the times around them;
//! - budgets defined at a date or a time, some later than postings to their accounts, some twice (in the same or
//!   another commodity), some never defined but named by accounts, adds and transfers;
//! - `budget-add`s in the budget's commodity, in another one a price converts and in one none converts, zero,
//!   negative, with many digits, at a time; `budget-transfer`s between defined and undefined budgets, in other
//!   commodities, from a budget to itself; `budget-close`s with and without a time, twice, of an undefined budget;
//! - accounts of the five types linked to up to three budgets, closed and opened again the same day (at a time or not)
//!   or the next day with other budgets, and postings around the close; a budget only accounts of Income, Liabilities
//!   or Equity (and maybe Assets) count toward, whose postings possign negates;
//! - prices quoted either way, from a later date on, zero, or not at all (GBP and XAU never have one); shares held at
//!   cost, maybe on a budget's account, and sold from their lots (maybe lots at costs in two currencies, one that
//!   converts and one that does not, in one posting); postings at a price, implicit postings, zero postings, two postings of an account
//!   in one transaction; a `pad` and a `balance` on an account of a budget;
//! - on some seeds, amounts and prices of 9 to 28 decimals;
//! - postings at the very instant of a budget directive of their account's budget (a definition, an add, a close), and
//!   on the very day a budget is defined or closed;
//! - a month whose postings to a budget net to zero, on a budget with a posting no price converts (summed as an
//!   inventory);
//! - an account whose `open` names a budget per month, each defined in its month and most closed at its end (every
//!   posting of the account is in all of them), maybe another account in some of them, and the account opened again
//!   in a month with the later ones only;
//! - a date typo: a transaction or a budget directive decades ahead, maybe two directives of the budget that day, or
//!   a transaction in 9999 with a dozen budgets more, so that the unbounded queries of `#budgets` have too many months;
//! - a second file, and a `plugin "zhang:budget"` directive with options it does not have.
//!
//! A ledger is a header (its options and the directives that set up the rest) and blocks, one directive each, in one
//! of its two files: a smaller reproduction of a difference is the same ledger with fewer blocks
//! ([`FuzzLedger::with_blocks`]).

use std::collections::BTreeSet;
use std::fmt::Write as _;

use chrono::{DateTime, Datelike, LocalResult, Months, NaiveDate, NaiveTime, TimeZone, Utc};
use chrono_tz::Tz;
use zhang_core::clock::Clock;

use crate::ledger::Scratch;
use crate::XorShift;

/// the main file of a generated ledger
pub const MAIN: &str = "main.zhang";
/// the file the main one includes, when the seed splits the ledger
pub const INCLUDED: &str = "more.zhang";

/// the budget names the ledgers draw from: byte order and name order differ (`Zeta` and `alpha`), and `Ghost` is never
/// defined
const NAMES: &[&str] = &["Food", "Fun", "Rent", "Travel", "Zeta", "alpha", "Gifts-2024", "Ghost"];
/// the budget that only accounts of other types than Expenses count toward, when the seed has one ([`ROOTED_SHAPES`])
const ROOTED: &str = "Payback";
/// the accounts the budget [`ROOTED`] may have: Income alone or with Assets (which a budget of debit-normal accounts
/// only would sum unsigned, were Income debit-normal), Liabilities, Equity
const ROOTED_SHAPES: &[&[&str]] = &[
    &["Income:Refund"],
    &["Income:Refund", "Assets:Cash"],
    &["Liabilities:Card"],
    &["Liabilities:Card", "Assets:Cash"],
    &["Equity:Opening", "Income:Refund"],
    &["Income:Refund", "Liabilities:Card", "Equity:Opening"],
];
/// the expense accounts
const EXPENSES: &[&str] = &[
    "Expenses:Food",
    "Expenses:Fun",
    "Expenses:Rent",
    "Expenses:Travel",
    "Expenses:Gifts",
    "Expenses:Misc",
];
/// the accounts of the other types a budget may be linked to: their postings count with possign's sign
const OTHERS: &[&str] = &["Assets:Cash", "Liabilities:Card", "Income:Refund", "Equity:Opening"];
/// the account whose `open` names a budget per month, on a seed of many budgets on one account
/// ([`Generator::monthly_budgets`]), and the account in some of them
const MONTHLY: &str = "Expenses:Monthly";
const MONTHLY_TOO: &str = "Expenses:Monthly:Extra";
/// the commodities: CNY, USD, EUR and JPY have prices, GOLD is held at cost in USD, GBP and XAU never have a price
const COMMODITIES: &[&str] = &["CNY", "USD", "EUR", "JPY", "GBP", "GOLD", "XAU"];
const TIMEZONES: &[&str] = &["Asia/Shanghai", "UTC", "America/New_York", "Europe/Berlin", "Pacific/Auckland"];
/// the timezones of [`TIMEZONES`] with daylight-saving changes, each skipping an hour in spring and repeating one in
/// autumn between 01:00 and 03:00
const DST_TIMEZONES: &[&str] = &["America/New_York", "Europe/Berlin", "Pacific/Auckland"];

/// Everything a seed may exercise ([`FuzzLedger::exercised`]): the seeds of a run together exercise all of it.
pub const FEATURES: &[&str] = &[
    "a budget closed at a time",
    "a budget closed at a time a DST change skips",
    "a budget closed twice",
    "a budget defined after postings to its accounts",
    "a budget defined twice",
    "a budget named but never defined",
    "a budget of an account of another type than Expenses",
    "a budget of no Expenses account, with Income, Liabilities or Equity ones",
    "a budget-add of an undefined budget",
    "a budget-close of an undefined budget",
    "a clock before the last transaction",
    "a commodity without prices",
    "a date typo",
    "a date typo in 9999, too many months for #budgets",
    "a month whose postings net to zero, on a budget with a posting no price converts",
    "a negative amount",
    "a pad and a balance on an account of a budget",
    "a plugin directive naming zhang:budget",
    "a posting at the instant of a budget directive",
    "a posting at a price",
    "a posting no price converts",
    "a posting on the day a budget closes",
    "a posting on the day a budget is defined",
    "a price of 9 to 28 decimals",
    "a sale from lots at costs in two currencies",
    "a second file",
    "a time a DST change repeats",
    "a time a DST change skips",
    "a transfer between undefined budgets",
    "a transfer from a budget to itself",
    "a zero amount",
    "a zero posting",
    "a zero price",
    "an account closed and opened again the same day",
    "an account in some of the budgets of an account of many budgets",
    "an account named on many budgets' opens",
    "an account of many budgets opened again with fewer",
    "an account opened later",
    "an amount in another commodity",
    "an amount of 9 to 28 decimals",
    "an amount of many digits",
    "prices from a later month on",
    "shares held at cost and sold from their lots",
    "two directives of a budget at its last date",
    "two directives of a budget at its last date, which a too large #budgets names",
    "two postings of an account in one transaction",
];

/// A directive of the ledger and the file it is written in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    /// 0 for [`MAIN`], 1 for [`INCLUDED`]
    pub file: usize,
    /// the directive with its indented lines, ending with a newline
    pub text: String,
}

/// A generated ledger ([`FuzzLedger::generate`]).
#[derive(Debug, Clone)]
pub struct FuzzLedger {
    pub seed: u64,
    /// the options and the directives the rest needs (commodities, the plugin directive), in the main file
    pub header: String,
    pub blocks: Vec<Block>,
    /// the instant of the load
    pub now: DateTime<Utc>,
    /// every budget name the ledger has, defined or not, and one it never names
    pub budgets: Vec<String>,
    /// the first days of the months to ask about, in order
    pub months: Vec<NaiveDate>,
    /// what the seed exercises, for a report, and the test that the seeds exercise everything
    pub exercised: BTreeSet<&'static str>,
}

/// What a daylight-saving change does to a time of the day it happens on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Change {
    /// the clocks jump over it: zhang reads it as the instant an hour later
    Skipped,
    /// the clocks go through it twice: zhang reads it as the earlier instant
    Repeated,
    /// a time around the change that happens once
    Around,
}

/// What the timezone `timezone` does to the time `time` of `date`: `None` for a time of a day without a change.
fn change_at(timezone: Tz, date: NaiveDate, time: NaiveTime) -> Option<Change> {
    match timezone.from_local_datetime(&date.and_time(time)) {
        LocalResult::None => Some(Change::Skipped),
        LocalResult::Ambiguous(..) => Some(Change::Repeated),
        LocalResult::Single(_) => None,
    }
}

/// The times between 01:00 and 03:59 of the days from `first` to before `until` on which `timezone` changes its
/// clocks, each with what the change does to it: every quarter of an hour, and the last second before 02:00 and 03:00.
fn change_times(timezone: Tz, first: NaiveDate, until: NaiveDate) -> Vec<(NaiveDate, NaiveTime, Change)> {
    let mut times = (1..4)
        .flat_map(|hour| (0..4).map(move |quarter| NaiveTime::from_hms_opt(hour, 15 * quarter, 0).expect("a time")))
        .collect::<Vec<_>>();
    times.extend([1, 2].map(|hour| NaiveTime::from_hms_opt(hour, 59, 59).expect("a time")));
    times.sort();
    let mut found = vec![];
    for date in first.iter_days().take_while(|date| *date < until) {
        let changes = times.iter().map(|time| change_at(timezone, date, *time)).collect::<Vec<_>>();
        if changes.iter().any(Option::is_some) {
            found.extend(times.iter().zip(changes).map(|(time, change)| (date, *time, change.unwrap_or(Change::Around))));
        }
    }
    found
}

struct Generator {
    random: XorShift,
    seed: u64,
    first: NaiveDate,
    months: u32,
    /// the blocks, each with the date that orders it and a tie-breaker
    blocks: Vec<(NaiveDate, u64, Block)>,
    exercised: BTreeSet<&'static str>,
    /// the budgets the ledger defines, with their commodity
    defined: Vec<(&'static str, &'static str)>,
    /// the budgets an `open` links each account to, as (account, budget)
    links: Vec<(&'static str, &'static str)>,
    /// the instants of the budget directives, with their budget: where a posting is a tie
    instants: Vec<(NaiveDate, Option<NaiveTime>, &'static str)>,
    /// the first definition and the first close of each budget: the days a posting is a boundary of
    definition_days: Vec<(NaiveDate, Option<NaiveTime>, &'static str)>,
    close_days: Vec<(NaiveDate, Option<NaiveTime>, &'static str)>,
    /// whether the ledger is split in two files
    split: bool,
    /// the ledger's timezone
    timezone: Tz,
    /// on a seed of daylight-saving changes, the times of the change days of the ledger's months the timed directives
    /// may take ([`change_times`]); empty on other seeds
    changes: Vec<(NaiveDate, NaiveTime, Change)>,
    /// whether amounts and prices may have 9 to 28 decimals
    long_decimals: bool,
    /// the accounts that count toward [`ROOTED`], when the seed has that budget
    rooted: &'static [&'static str],
}

impl Generator {
    fn note(&mut self, what: &'static str) {
        self.exercised.insert(what);
    }

    /// a day of the ledger's months: of month `month` (from 0), a random day
    fn day_of(&mut self, month: u32) -> NaiveDate {
        let first = self.first + Months::new(month);
        let days = (first + Months::new(1) - first).num_days() as usize;
        first + chrono::Days::new(self.random.below(days) as u64)
    }

    fn any_day(&mut self) -> NaiveDate {
        let month = self.random.below(self.months as usize) as u32;
        self.day_of(month)
    }

    /// a time of day, or none, as the seed picks: midnight and the last second of a day come often
    fn time(&mut self, percent: usize) -> Option<NaiveTime> {
        if !self.random.chance(percent) {
            return None;
        }
        Some(match self.random.below(5) {
            0 => NaiveTime::MIN,
            1 => NaiveTime::from_hms_opt(23, 59, 59).expect("a time"),
            _ => NaiveTime::from_hms_opt(self.random.below(24) as u32, 15 * self.random.below(4) as u32, 0).expect("a time"),
        })
    }

    /// `date` and `time`, or `percent` times in a hundred on a seed of daylight-saving changes, a time of a change day
    /// from `not_before` on: mostly one the change skips or repeats, else one around it
    fn when(&mut self, date: NaiveDate, time: Option<NaiveTime>, percent: usize, not_before: NaiveDate) -> (NaiveDate, Option<NaiveTime>) {
        if self.changes.is_empty() || !self.random.chance(percent) {
            return (date, time);
        }
        let around = self.random.chance(25);
        let candidates = self
            .changes
            .iter()
            .filter(|(day, _, change)| *day >= not_before && (around || *change != Change::Around))
            .map(|(day, time, _)| (*day, *time))
            .collect::<Vec<_>>();
        if candidates.is_empty() {
            return (date, time);
        }
        let (day, time) = *self.random.pick(&candidates);
        (day, Some(time))
    }

    /// A date as zhang writes one, with its time when it has one, noting a time a daylight-saving change skips or
    /// repeats.
    fn stamp(&mut self, date: NaiveDate, time: Option<NaiveTime>) -> String {
        let Some(time) = time else {
            return date.to_string();
        };
        match change_at(self.timezone, date, time) {
            Some(Change::Skipped) => self.note("a time a DST change skips"),
            Some(Change::Repeated) => self.note("a time a DST change repeats"),
            _ => {}
        }
        format!("{date} {}", time.format("%H:%M:%S"))
    }

    fn push(&mut self, date: NaiveDate, text: String) {
        let tie = self.random.next() >> 1;
        let file = usize::from(self.split && self.random.chance(30));
        self.blocks.push((date, tie, Block { file, text }));
    }

    fn name(&mut self) -> &'static str {
        self.random.pick(NAMES)
    }

    /// `count` random digits
    fn digits(&mut self, count: usize) -> String {
        (0..count).map(|_| char::from(b'0' + self.random.below(10) as u8)).collect()
    }

    /// A number of `hundredths` written with 0 to 3 decimals, as the seed picks: `12`, `12.5`, `12.50`, `12.500`; or,
    /// on a seed of long decimals, with 9 to 28 (more digits than the hundredths, unless they are zero).
    fn number(&mut self, hundredths: i64) -> String {
        let (sign, magnitude) = if hundredths < 0 { ("-", -hundredths) } else { ("", hundredths) };
        let (whole, cents) = (magnitude / 100, magnitude % 100);
        if self.long_decimals && self.random.chance(30) {
            self.note("an amount of 9 to 28 decimals");
            let count = 7 + self.random.below(20);
            let more = if magnitude == 0 { "0".repeat(count) } else { self.digits(count) };
            return format!("{sign}{whole}.{cents:02}{more}");
        }
        match self.random.below(4) {
            0 if cents == 0 => format!("{sign}{whole}"),
            1 if cents % 10 == 0 => format!("{sign}{whole}.{}", cents / 10),
            3 => format!("{sign}{whole}.{cents:02}0"),
            _ => format!("{sign}{whole}.{cents:02}"),
        }
    }

    /// A price of `hundredths`: `7.10`, or on a seed of long decimals, maybe with 9 to 28 decimals.
    fn price(&mut self, hundredths: i64) -> String {
        let text = format!("{}.{:02}", hundredths / 100, hundredths % 100);
        if self.long_decimals && self.random.chance(40) {
            self.note("a price of 9 to 28 decimals");
            let count = 7 + self.random.below(20);
            return format!("{text}{}", self.digits(count));
        }
        text
    }

    /// an amount for a budget in `commodity`: mostly in it, else in another commodity, maybe one no price converts
    fn amount(&mut self, commodity: &'static str) -> String {
        let hundredths = match self.random.below(20) {
            0 => {
                self.note("a zero amount");
                0
            }
            1 => {
                self.note("a negative amount");
                -(100 + self.random.below(50_000) as i64)
            }
            2 => {
                self.note("an amount of many digits");
                1_234_567_890_123_456_789 + self.random.below(1000) as i64
            }
            _ => 100 + self.random.below(200_000) as i64,
        };
        let currency = match self.random.below(10) {
            0..=6 => commodity,
            _ => {
                let other = *self.random.pick(&COMMODITIES[..5]);
                if other != commodity {
                    self.note("an amount in another commodity");
                }
                other
            }
        };
        let text = if currency == "JPY" {
            (hundredths / 100).to_string()
        } else {
            self.number(hundredths)
        };
        format!("{text} {currency}")
    }

    fn options(&mut self, now: &mut DateTime<Utc>) -> String {
        // a seed of daylight-saving changes: a timezone that has them, and the timed directives on its change days
        let changes = self.random.chance(30);
        let timezone = *self.random.pick(if changes { DST_TIMEZONES } else { TIMEZONES });
        self.timezone = timezone.parse().expect("a timezone");
        if changes {
            self.changes = change_times(self.timezone, self.first, self.first + Months::new(self.months));
        }
        let operating = *self.random.pick(&["CNY", "USD"]);
        let mut header = format!("option \"operating_currency\" \"{operating}\"\noption \"timezone\" \"{timezone}\"\n");
        for commodity in COMMODITIES {
            writeln!(header, "1970-01-01 commodity {commodity}").expect("a string");
        }
        if self.random.chance(15) {
            self.note("a plugin directive naming zhang:budget");
            header.push_str("plugin \"zhang:budget\" \"monthly\"\n  timeout: \"5s\"\n");
            if self.random.chance(50) {
                header.push_str("plugin \"zhang:budget\"\n");
            }
        }
        if self.split {
            self.note("a second file");
            writeln!(header, "include \"{INCLUDED}\"").expect("a string");
        }
        // the clock: in the ledger's months, after them, or before the last transaction
        let last = self.first + Months::new(self.months);
        let day = match self.random.below(4) {
            0 => {
                self.note("a clock before the last transaction");
                self.first + Months::new(self.months / 2)
            }
            1 => last + Months::new(1 + self.random.below(14) as u32),
            _ => last - chrono::Days::new(1 + self.random.below(20) as u64),
        };
        *now = Utc.from_utc_datetime(&day.and_hms_opt(12, 0, 0).expect("noon"));
        header
    }

    fn prices(&mut self) {
        // USD in CNY, EUR quoted the other way round (CNY in EUR), JPY in CNY from a later month on, GOLD in USD; GBP
        // and XAU never
        let pairs: [(&str, &str, i64, u32); 4] = [("USD", "CNY", 710, 0), ("CNY", "EUR", 13, 0), ("JPY", "CNY", 5, 2), ("GOLD", "USD", 30_000, 0)];
        for (base, quote, rate, from) in pairs {
            if self.random.chance(10) {
                self.note("a commodity without prices");
                continue;
            }
            let from = if from > 0 && self.random.chance(50) {
                self.note("prices from a later month on");
                from
            } else {
                0
            };
            for month in from..self.months {
                if !self.random.chance(60) && month != from {
                    continue;
                }
                let date = self.day_of(month);
                let rate = if self.random.chance(3) {
                    self.note("a zero price");
                    "0".to_owned()
                } else {
                    let wobble = rate * (95 + self.random.below(11) as i64) / 100;
                    self.price(wobble)
                };
                self.push(date, format!("{date} price {base} {rate} {quote}\n"));
            }
        }
    }

    /// the `budget` metadata lines of an `open` of `account`
    fn budget_metas(&mut self, account: &'static str, at_most: usize) -> String {
        let mut text = String::new();
        let count = self.random.below(at_most + 1);
        let mut names = BTreeSet::new();
        for _ in 0..count {
            names.insert(self.name());
        }
        for name in names {
            self.links.push((account, name));
            if self.random.chance(20) && !name.contains('-') {
                writeln!(text, "  budget: {name}").expect("a string");
            } else {
                writeln!(text, "  budget: \"{name}\"").expect("a string");
            }
        }
        text
    }

    fn accounts(&mut self) {
        let before = self.first.pred_opt().expect("a date");
        for account in ["Assets:Bank", "Assets:Broker"] {
            let mut text = format!("{before} open {account}\n");
            if account == "Assets:Broker" {
                text.push_str("  booking_method: \"FIFO\"\n");
                // the account of the shares held at cost, sometimes a budget's
                if self.random.chance(50) {
                    let metas = self.budget_metas(account, 2);
                    text.push_str(&metas);
                }
            }
            self.push(before, text);
        }
        for account in EXPENSES.iter().chain(OTHERS).copied() {
            let rooted = self.rooted.contains(&account);
            let linked = EXPENSES.contains(&account) || self.random.chance(35);
            if (linked || rooted) && !EXPENSES.contains(&account) {
                self.note("a budget of an account of another type than Expenses");
            }
            // most accounts are open from the start; some from a later day, maybe at a time of a daylight-saving change
            let (opened, open_time) = if self.random.chance(15) {
                self.note("an account opened later");
                let day = self.any_day();
                self.when(day, None, 30, day)
            } else {
                (before, None)
            };
            let mut metas = if linked { self.budget_metas(account, 3) } else { String::new() };
            if rooted {
                self.links.push((account, ROOTED));
                writeln!(metas, "  budget: \"{ROOTED}\"").expect("a string");
            }
            let open = self.stamp(opened, open_time);
            self.push(opened, format!("{open} open {account}\n{metas}"));
            // closed, and opened again with other budgets the same day or the next
            if linked && self.random.chance(25) {
                let day = self.any_day().max(opened);
                let time = self.time(40);
                let (close, time) = self.when(day, time, 30, opened.succ_opt().expect("a date"));
                let again = if self.random.chance(70) {
                    self.note("an account closed and opened again the same day");
                    close
                } else {
                    close.succ_opt().expect("a date")
                };
                let text = format!("{} close {account}\n", self.stamp(close, time));
                self.push(close, text);
                let metas = self.budget_metas(account, 3);
                let reopen_time = if again == close { time.or(self.time(30)) } else { None };
                // after the close, in its file, among the blocks of its day
                let file = self.blocks.last().map_or(0, |(_, _, block)| block.file);
                let text = format!("{} open {account}\n{metas}", self.stamp(again, reopen_time));
                self.blocks.push((again, u64::MAX, Block { file, text }));
            }
        }
    }

    /// the `budget` directive of `name`, and maybe a second one
    fn define(&mut self, name: &'static str) {
        let commodity = *self.random.pick(&["CNY", "CNY", "USD", "EUR", "JPY", "GBP"]);
        let date = if self.random.chance(70) {
            self.first
        } else {
            self.note("a budget defined after postings to its accounts");
            self.any_day()
        };
        let time = self.time(20);
        let (date, time) = self.when(date, time, 10, self.first);
        self.instants.push((date, time, name));
        self.definition_days.push((date, time, name));
        let mut text = format!("{} budget {name} {commodity}\n", self.stamp(date, time));
        if self.random.chance(40) {
            writeln!(text, "  alias: \"The {name}\"").expect("a string");
        }
        if self.random.chance(40) {
            writeln!(text, "  category: \"{}\"", self.random.pick(&["Home", "Leisure"])).expect("a string");
        }
        if self.random.chance(20) {
            text.push_str("  note: \"kept\"\n");
        }
        self.push(date, text);
        self.defined.push((name, commodity));
        if self.random.chance(12) {
            self.note("a budget defined twice");
            let again = self.any_day();
            let other = *self.random.pick(&["CNY", "USD", commodity]);
            self.push(again, format!("{again} budget {name} {other}\n"));
        }
    }

    fn definitions(&mut self) {
        for name in NAMES.iter().copied().filter(|name| *name != "Ghost") {
            if self.random.chance(15) {
                self.note("a budget named but never defined");
                continue;
            }
            self.define(name);
        }
        if !self.rooted.is_empty() {
            self.note("a budget of no Expenses account, with Income, Liabilities or Equity ones");
            self.define(ROOTED);
        }
    }

    fn budget_directives(&mut self) {
        let defined = self.defined.clone();
        for month in 0..self.months {
            for (name, commodity) in &defined {
                if self.random.chance(60) {
                    let (day, time) = (self.day_of(month), self.time(15));
                    let (date, time) = self.when(day, time, 12, self.first);
                    self.instants.push((date, time, name));
                    let amount = self.amount(commodity);
                    let text = format!("{} budget-add {name} {amount}\n", self.stamp(date, time));
                    self.push(date, text);
                }
            }
            if self.random.chance(10) {
                self.note("a budget-add of an undefined budget");
                let date = self.day_of(month);
                self.push(date, format!("{date} budget-add Ghost 10 CNY\n"));
            }
            for _ in 0..self.random.below(3) {
                let (from, to) = (self.name(), self.name());
                if from == to {
                    self.note("a transfer from a budget to itself");
                }
                if !defined.iter().any(|(name, _)| *name == from) || !defined.iter().any(|(name, _)| *name == to) {
                    self.note("a transfer between undefined budgets");
                }
                let commodity = defined.iter().find(|(name, _)| *name == from).map_or("CNY", |(_, commodity)| commodity);
                let (day, time) = (self.day_of(month), self.time(15));
                let (date, time) = self.when(day, time, 12, self.first);
                let amount = self.amount(commodity);
                let text = format!("{} budget-transfer {from} {to} {amount}\n", self.stamp(date, time));
                self.push(date, text);
            }
        }
        // on a seed of daylight-saving changes, more closes, most of them at a time of a change day
        let (closing, at_a_change) = if self.changes.is_empty() { (30, 0) } else { (45, 60) };
        for (name, _) in &defined {
            if self.random.chance(closing) {
                let (day, time) = (self.any_day(), self.time(50));
                let (date, time) = self.when(day, time, at_a_change, self.first);
                if let Some(time) = time {
                    self.note("a budget closed at a time");
                    if change_at(self.timezone, date, time) == Some(Change::Skipped) {
                        self.note("a budget closed at a time a DST change skips");
                    }
                }
                self.instants.push((date, time, name));
                self.close_days.push((date, time, name));
                let text = format!("{} budget-close {name}\n", self.stamp(date, time));
                self.push(date, text);
                if self.random.chance(25) {
                    self.note("a budget closed twice");
                    let later = self.any_day();
                    self.push(later, format!("{later} budget-close {name}\n"));
                }
            }
        }
        if self.random.chance(10) {
            self.note("a budget-close of an undefined budget");
            let date = self.any_day();
            self.push(date, format!("{date} budget-close Ghost\n"));
        }
    }

    /// a posting of `hundredths` to `account`, in a commodity the seed picks, or the one `kind` picks
    fn posting(&mut self, account: &str, hundredths: i64, kind: Option<usize>) -> String {
        let kind = kind.unwrap_or_else(|| self.random.below(20));
        match kind {
            0..=10 => format!("  {account} {} CNY", self.number(hundredths)),
            11..=13 => format!("  {account} {} USD", self.number(hundredths)),
            14 => format!("  {account} {} EUR", self.number(hundredths)),
            15 => format!("  {account} {} JPY", hundredths),
            16 => {
                self.note("a posting no price converts");
                let currency = *self.random.pick(&["GBP", "XAU"]);
                format!("  {account} {} {currency}", self.number(hundredths))
            }
            17 => {
                self.note("a posting at a price");
                let units = self.number(hundredths);
                format!("  {account} {units} GBP @ {} CNY", self.price(910))
            }
            18 => {
                self.note("a zero posting");
                format!("  {account} 0 CNY")
            }
            _ => format!("  {account} {} USD", self.number(-hundredths)),
        }
    }

    /// the accounts an `open` links to the budget `name`
    fn accounts_of(&self, name: &str) -> Vec<&'static str> {
        self.links.iter().filter(|(_, it)| *it == name).map(|(account, _)| *account).collect()
    }

    fn transactions(&mut self) {
        let count = 20 + self.random.below(80);
        let accounts = EXPENSES.iter().chain(OTHERS).copied().collect::<Vec<_>>();
        for index in 0..count {
            let (day, time) = (self.any_day(), self.time(30));
            let (mut date, mut time) = self.when(day, time, 12, self.first);
            // a few at the very instant of a budget directive, to an account of its budget: the ties of a day
            let mut tied = None;
            if !self.instants.is_empty() && self.random.chance(15) {
                self.note("a posting at the instant of a budget directive");
                let (at, at_time, budget) = *self.random.pick(&self.instants);
                (date, time) = (at, at_time);
                let linked = self.accounts_of(budget);
                tied = (!linked.is_empty()).then(|| *self.random.pick(&linked));
            }
            let mut text = format!("{} * \"Shop {index}\" \"purchase {index}\"\n", self.stamp(date, time));
            // the postings of a transaction share a commodity, so that the implicit one balances them; a few mix
            let shared = self.random.below(20);
            for _ in 0..1 + self.random.below(3) {
                let account = tied.take().unwrap_or_else(|| *self.random.pick(&accounts));
                let hundredths = 100 + self.random.below(30_000) as i64;
                let posting = self.posting(account, hundredths, (shared > 1).then_some(shared));
                writeln!(text, "{posting}").expect("a string");
                if self.random.chance(5) {
                    self.note("two postings of an account in one transaction");
                    let again = self.posting(account, 150, Some(shared.max(2)));
                    writeln!(text, "{again}").expect("a string");
                }
            }
            text.push_str("  Assets:Bank\n");
            self.push(date, text);
        }
        self.boundary_postings();
        self.zero_month();
        // shares bought at cost, then sold from their lots into an expense: the implicit posting gets the booked cost
        if self.random.chance(40) {
            self.note("shares held at cost and sold from their lots");
            let (buy, sell) = (self.day_of(0), self.day_of(self.months - 1));
            let gift = *self.random.pick(EXPENSES);
            if self.random.chance(50) {
                self.push(buy, format!("{buy} * \"Broker\" \"buy\"\n  Assets:Broker 3 GOLD {{300 USD}}\n  Assets:Bank\n"));
                self.push(sell, format!("{sell} * \"Friend\" \"gift\"\n  Assets:Broker -2 GOLD {{}}\n  {gift}\n"));
            } else {
                // one lot at a cost in dollars, which converts, and one in pounds, which does not: the posting that
                // sells both is checked whole, through its first lot's cost currency
                self.note("a sale from lots at costs in two currencies");
                self.push(buy, format!("{buy} * \"Broker\" \"buy\"\n  Assets:Broker 1 GOLD {{300 USD}}\n  Assets:Bank\n"));
                let next = buy.succ_opt().expect("a date");
                self.push(
                    next,
                    format!("{next} * \"Broker\" \"buy\"\n  Assets:Broker 1 GOLD {{250 GBP}}\n  Assets:Bank\n"),
                );
                let text = format!("{sell} * \"Friend\" \"gift\"\n  Assets:Broker -2 GOLD {{}}\n  {gift} 300 USD\n  {gift} 250 GBP\n");
                self.push(sell, text);
            }
        }
        if self.random.chance(15) {
            self.note("a pad and a balance on an account of a budget");
            let date = self.any_day();
            let next = date.succ_opt().expect("a date");
            self.push(date, format!("{date} pad Assets:Cash Equity:Opening\n"));
            self.push(next, format!("{next} balance Assets:Cash 100 CNY\n"));
        }
        if self.random.chance(20) {
            self.note("a date typo");
            if self.random.chance(60) {
                // in the year 9999: a transaction then ends every budget's months there, and with a dozen budgets more
                // their months are more than a query may generate, so the unbounded queries of #budgets fail with an
                // error that names the longest series and the directive that ends it, here a budget's own later one
                self.note("a date typo in 9999, too many months for #budgets");
                for index in 0..12 {
                    let first = self.first;
                    self.push(first, format!("{first} budget Many-{index:02} CNY\n"));
                }
                let month = 1 + self.random.below(6) as u32;
                let date = NaiveDate::from_ymd_opt(9999, month, 1 + self.random.below(28) as u32).expect("a date");
                self.push(date, format!("{date} * \"Typo\" \"a date typo\"\n  Expenses:Food 1 CNY\n  Assets:Bank\n"));
                let later = NaiveDate::from_ymd_opt(9999, month + 6, 1 + self.random.below(28) as u32).expect("a date");
                if self.typo_budget_directives(later, 80) {
                    self.note("two directives of a budget at its last date, which a too large #budgets names");
                }
            } else {
                let date = self.any_day() + Months::new(12 * (10 + self.random.below(30) as u32));
                if self.random.chance(50) {
                    self.push(date, format!("{date} * \"Typo\" \"a date typo\"\n  Expenses:Food 1 CNY\n  Assets:Bank\n"));
                } else {
                    self.typo_budget_directives(date, 50);
                }
            }
        }
    }

    /// Postings on the very day a budget is defined or closed, to an account of the budget: before or after the
    /// directive, at no time (the start of the day, which a close at a date only lasts through) or at one, maybe its.
    fn boundary_postings(&mut self) {
        let days = (self.definition_days.iter().map(|it| (*it, "a posting on the day a budget is defined", 50)))
            .chain(self.close_days.iter().map(|it| (*it, "a posting on the day a budget closes", 80)))
            .collect::<Vec<_>>();
        for ((date, at, name), what, percent) in days {
            let linked = self.accounts_of(name);
            if linked.is_empty() || !self.random.chance(percent) {
                continue;
            }
            self.note(what);
            for index in 0..1 + self.random.below(2) {
                let time = match self.random.below(4) {
                    0 | 1 => None,
                    2 => at,
                    _ => self.time(100),
                };
                let account = *self.random.pick(&linked);
                let hundredths = 100 + self.random.below(30_000) as i64;
                let posting = self.posting(account, hundredths, None);
                let text = format!(
                    "{} * \"Shop\" \"on the day of {name} {index}\"\n{posting}\n  Assets:Bank\n",
                    self.stamp(date, time)
                );
                self.push(date, text);
            }
        }
    }

    /// A month whose postings to an account of a budget net to zero, the month after the ledger's (no other posting
    /// is in it), on a budget that also has a posting no price converts: its activity is then summed as an inventory,
    /// where a sum of zero has no position.
    fn zero_month(&mut self) {
        let candidates = self
            .defined
            .clone()
            .into_iter()
            .filter(|(name, _)| !self.accounts_of(name).is_empty())
            .collect::<Vec<_>>();
        if candidates.is_empty() || !self.random.chance(35) {
            return;
        }
        self.note("a month whose postings net to zero, on a budget with a posting no price converts");
        let (name, commodity) = *self.random.pick(&candidates);
        let linked = self.accounts_of(name);
        let account = *self.random.pick(&linked);
        // in the budget's commodity, or in one a price converts into it (at one rate: on one day)
        let currency = if self.random.chance(30) {
            commodity
        } else if commodity == "CNY" {
            *self.random.pick(&["USD", "EUR"])
        } else {
            "CNY"
        };
        let month = self.first + Months::new(self.months);
        let date = month + chrono::Days::new(self.random.below(28) as u64);
        let hundredths = 100 + self.random.below(30_000) as i64;
        let units = if currency == "JPY" {
            (hundredths / 100).to_string()
        } else {
            self.number(hundredths)
        };
        for (index, units) in [units.clone(), format!("-{units}")].into_iter().enumerate() {
            let time = self.time(30);
            let text = format!(
                "{} * \"Shop\" \"back and forth {index}\"\n  {account} {units} {currency}\n  Assets:Bank\n",
                self.stamp(date, time)
            );
            self.push(date, text);
        }
        let other = self.any_day();
        self.push(other, format!("{other} * \"Shop\" \"no price\"\n  {account} 1 XAU\n  Assets:Bank\n"));
    }

    /// a `budget-add` of the first budget at the date typo `date`, and `percent` times in a hundred its `budget-close`
    /// that day: which of the two ends its months is the one folded later; whether there are two
    fn typo_budget_directives(&mut self, date: NaiveDate, percent: usize) -> bool {
        let Some((name, commodity)) = self.defined.first().copied() else {
            return false;
        };
        self.push(date, format!("{date} budget-add {name} 1 {commodity}\n"));
        if !self.random.chance(percent) {
            return false;
        }
        self.note("two directives of a budget at its last date");
        self.push(date, format!("{date} budget-close {name}\n"));
        true
    }

    /// On some seeds, an account whose `open` names a budget per month of the ledger (`M-00`, `M-01`, ...), as a ledger
    /// that closes a budget per category every month: each is defined in its month (a few never are) and most are
    /// closed at its end, so every posting of the account is in all of them, before the definition of the later ones
    /// and after the close of the earlier ones. Maybe the `open` names a budget of the other accounts too, another
    /// account is in some of the monthly budgets, and the account is closed and opened again in a month with the
    /// budgets of the later months only. Its budgets, which the seed's draws come last for: the rest of the ledger is
    /// the same with them or without them.
    fn monthly_budgets(&mut self) -> Vec<String> {
        if !self.random.chance(25) {
            return vec![];
        }
        self.note("an account named on many budgets' opens");
        let before = self.first.pred_opt().expect("a date");
        let names = (0..self.months).map(|month| format!("M-{month:02}")).collect::<Vec<_>>();
        let metas = |names: &[String]| names.iter().map(|name| format!("  budget: \"{name}\"\n")).collect::<String>();
        let mut text = format!("{before} open {MONTHLY}\n{}", metas(&names));
        if self.random.chance(50) {
            let other = self.name();
            writeln!(text, "  budget: \"{other}\"").expect("a string");
        }
        self.push(before, text);
        let mut accounts = vec![MONTHLY];
        if self.random.chance(50) {
            self.note("an account in some of the budgets of an account of many budgets");
            let some = names.iter().filter(|_| self.random.chance(50)).cloned().collect::<Vec<_>>();
            self.push(before, format!("{before} open {MONTHLY_TOO}\n{}", metas(&some)));
            accounts.push(MONTHLY_TOO);
        }
        for (month, name) in (0..self.months).zip(&names) {
            let first = self.first + Months::new(month);
            let last = first + Months::new(1) - chrono::Days::new(1);
            if self.random.chance(90) {
                let commodity = *self.random.pick(&["CNY", "CNY", "CNY", "USD", "GBP"]);
                let day = if self.random.chance(75) { first } else { self.day_of(month) };
                let time = self.time(20);
                let (date, time) = self.when(day, time, 10, first);
                let text = format!("{} budget {name} {commodity}\n", self.stamp(date, time));
                self.push(date, text);
                if self.random.chance(60) {
                    let amount = self.amount(commodity);
                    self.push(last, format!("{last} budget-add {name} {amount}\n"));
                }
                match self.random.below(10) {
                    0..=4 => self.push(last, format!("{last} budget-close {name}\n")),
                    5..=7 => {
                        // at a time of its last day, between postings of that day
                        let (date, time) = self.when(last, NaiveTime::from_hms_opt(12, 0, 0), 20, last);
                        let text = format!("{} budget-close {name}\n", self.stamp(date, time));
                        self.push(date, text);
                        for at in [9, 18] {
                            let account = *self.random.pick(&accounts);
                            let hundredths = 125 + self.random.below(1000) as i64;
                            let posting = self.posting(account, hundredths, None);
                            let text = format!("{date} {at:02}:00:00 * \"Shop\" \"around {name}\"\n{posting}\n  Assets:Bank\n");
                            self.push(date, text);
                        }
                    }
                    _ => {}
                }
            }
            for _ in 0..1 + self.random.below(3) {
                let account = *self.random.pick(&accounts);
                let (day, time) = (self.day_of(month), self.time(30));
                let (date, time) = self.when(day, time, 10, first);
                let hundredths = 100 + self.random.below(30_000) as i64;
                let posting = self.posting(account, hundredths, None);
                let text = format!("{} * \"Shop\" \"monthly\"\n{posting}\n  Assets:Bank\n", self.stamp(date, time));
                self.push(date, text);
            }
        }
        if self.months > 2 && self.random.chance(40) {
            // closed in a month and opened again, that day or the next, with the budgets of the later months
            self.note("an account of many budgets opened again with fewer");
            let month = 1 + self.random.below(self.months as usize - 2) as u32;
            let day = self.day_of(month);
            let time = self.time(40);
            let text = format!("{} close {MONTHLY}\n", self.stamp(day, time));
            self.push(day, text);
            let again = if self.random.chance(50) { day } else { day.succ_opt().expect("a date") };
            let reopen_time = if again == day { time.or(self.time(30)) } else { None };
            let file = self.blocks.last().map_or(0, |(_, _, block)| block.file);
            let text = format!("{} open {MONTHLY}\n{}", self.stamp(again, reopen_time), metas(&names[month as usize..]));
            self.blocks.push((again, u64::MAX, Block { file, text }));
            let posting = self.posting(MONTHLY, 250, None);
            let text = format!(
                "{} * \"Shop\" \"reopened\"\n{posting}\n  Assets:Bank\n",
                self.stamp(again, Some(NaiveTime::from_hms_opt(23, 0, 0).expect("a time")))
            );
            self.push(again, text);
        }
        names
    }
}

impl FuzzLedger {
    /// The ledger of `seed`.
    pub fn generate(seed: u64) -> FuzzLedger {
        let mut random = XorShift(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let first = NaiveDate::from_ymd_opt(2023 + random.below(2) as i32, 1 + random.below(12) as u32, 1).expect("a date");
        let months = 4 + random.below(15) as u32;
        let split = random.chance(40);
        let long_decimals = random.chance(25);
        let rooted: &'static [&'static str] = if random.chance(45) { random.pick(ROOTED_SHAPES) } else { &[] };
        let mut generator = Generator {
            random,
            seed,
            first,
            months,
            blocks: vec![],
            exercised: BTreeSet::new(),
            defined: vec![],
            links: vec![],
            instants: vec![],
            definition_days: vec![],
            close_days: vec![],
            split,
            timezone: Tz::UTC,
            changes: vec![],
            long_decimals,
            rooted,
        };
        let mut now = Utc::now();
        let header = generator.options(&mut now);
        generator.prices();
        generator.accounts();
        generator.definitions();
        generator.budget_directives();
        generator.transactions();
        let monthly = generator.monthly_budgets();
        let Generator {
            seed,
            mut blocks,
            exercised,
            rooted,
            ..
        } = generator;
        // in date order, ties in the order the seed picked: zhang orders a day by time and then as written
        blocks.sort_by_key(|(date, tie, _)| (*date, *tie));
        let mut budgets = NAMES.iter().map(|it| (*it).to_owned()).collect::<Vec<_>>();
        if !rooted.is_empty() {
            budgets.push(ROOTED.to_owned());
        }
        budgets.extend(monthly);
        budgets.push("Nobody".to_owned());
        let last = (first + Months::new(months)).max(now.date_naive().with_day(1).expect("a first day"));
        let months = (0..)
            .map(|month| first - Months::new(1) + Months::new(month))
            .take_while(|month| *month <= last + Months::new(1))
            .collect();
        FuzzLedger {
            seed,
            header,
            blocks: blocks.into_iter().map(|(_, _, block)| block).collect(),
            now,
            budgets,
            months,
            exercised,
        }
    }

    /// The same ledger with the blocks `keep` says to keep.
    pub fn with_blocks(&self, keep: &[bool]) -> FuzzLedger {
        FuzzLedger {
            blocks: self
                .blocks
                .iter()
                .zip(keep)
                .filter(|(_, keep)| **keep)
                .map(|(block, _)| block.clone())
                .collect(),
            ..self.clone()
        }
    }

    /// Its files: the main one, and the included one when the header includes it.
    pub fn files(&self) -> Vec<(String, String)> {
        let mut files = vec![(MAIN.to_owned(), format!("{}\n", self.header)), (INCLUDED.to_owned(), String::new())];
        for block in &self.blocks {
            files[block.file].1.push_str(&block.text);
            files[block.file].1.push('\n');
        }
        if !self.header.contains(INCLUDED) {
            files.truncate(1);
        }
        files
    }

    /// Its files, as one text a report shows.
    pub fn text(&self) -> String {
        self.files().iter().map(|(name, text)| format!("--- {name}\n{text}")).collect()
    }

    /// The clock of its load.
    pub fn clock(&self) -> Clock {
        Clock::Fixed(self.now)
    }

    /// A directory holding its files.
    pub fn scratch(&self) -> Scratch {
        let files = self.files();
        let files = files.iter().map(|(name, text)| (name.as_str(), text.as_str())).collect::<Vec<_>>();
        Scratch::with_files(MAIN, &files)
    }
}

/// Running the seeds: each seed's ledger compared with the oracle and with the script ([`crate::budget_parity`]), and a
/// difference reduced to the fewest blocks that still show one.
#[cfg(feature = "server")]
mod run {
    use std::collections::BTreeSet;
    use std::fmt;
    use std::ops::RangeInclusive;
    use std::time::{Duration, Instant};

    use chrono::Datelike;
    use serde_json::{json, Value};

    use super::{FuzzLedger, MAIN};
    use crate::budget_parity::{compare_until, Difference, Report};

    /// the environment variable naming the seeds a run checks: `17`, `1..500` or `1..=500`
    pub const SEEDS_ENV: &str = "ZHANG_BUDGET_FUZZ_SEEDS";

    /// How long a difference is reduced at most.
    const SHRINKING: Duration = Duration::from_secs(180);

    /// The seeds [`SEEDS_ENV`] names, or `default` when it is unset; an error for a value it cannot read.
    pub fn seeds(default: RangeInclusive<u64>) -> Result<RangeInclusive<u64>, String> {
        let Ok(text) = std::env::var(SEEDS_ENV) else {
            return Ok(default);
        };
        let wrong = || format!("{SEEDS_ENV}={text:?}: expected a seed (17) or a range (1..500, 1..=500)");
        let number = |it: &str| it.trim().parse::<u64>().map_err(|_| wrong());
        match text.split_once("..") {
            None => number(&text).map(|seed| seed..=seed),
            Some((first, last)) => match last.strip_prefix('=') {
                Some(last) => Ok(number(first)?..=number(last)?),
                None => Ok(number(first)?..=number(last)?.checked_sub(1).ok_or_else(wrong)?),
            },
        }
    }

    /// What a client asks of a generated ledger's budgets: [`crate::budget_parity::requests`] over its months and
    /// budgets (an unknown one included), every table whole and by its wildcard, `#budgets` by year and up to a month
    /// (the queries whose bound stops the series early), the postings of each budget through the label column's fast
    /// path, the budgets' row metadata, and a month that is not a first day.
    pub fn requests(ledger: &FuzzLedger) -> Vec<Value> {
        let months = ledger.months.iter().map(ToString::to_string).collect::<Vec<_>>();
        let pages = [&months[0], &months[months.len() / 2], &months[months.len() - 2]].map(Clone::clone);
        let mut requests = crate::budget_parity::requests(&months, &ledger.budgets, &pages);
        for table in ["budgets", "budget_events", "budget_definitions"] {
            requests.push(json!({ "bql": format!("SELECT * FROM #{table}") }));
        }
        let years = ledger.months.iter().map(Datelike::year).collect::<BTreeSet<_>>();
        for year in years {
            requests.push(json!({ "bql": format!("SELECT name, date, assigned, added, activity, available, closed FROM #budgets WHERE year = {year}") }));
        }
        for month in [&months[1], &months[months.len() / 2]] {
            requests.push(json!({ "bql": format!("SELECT name, date, assigned, activity, available FROM #budgets WHERE date <= {month}") }));
            requests.push(json!({ "bql": format!("SELECT name, assigned, activity FROM #budgets WHERE yearmonth(date) = {month}") }));
        }
        for budget in &ledger.budgets {
            requests.push(json!({ "bql": format!("SELECT count(*) AS postings, sum(number) AS units FROM #postings WHERE '{budget}' IN budgets") }));
        }
        requests.push(json!({"bql": "SELECT name, meta('alias'), meta('note'), metas FROM #budget_definitions"}));
        requests.push(json!({"bql": "SELECT name, date, meta('category') FROM #budgets"}));
        let mid_month = ledger.months[1].with_day(15).map(|it| it.to_string()).unwrap_or_default();
        requests.push(json!({"builtin": "budgets.month", "params": {"month": mid_month}}));
        requests
    }

    /// A seed whose ledger the script answers otherwise than the oracle.
    #[derive(Debug, Clone)]
    pub struct Finding {
        pub seed: u64,
        pub exercised: BTreeSet<&'static str>,
        /// the first difference on the seed's ledger
        pub difference: Difference,
        /// the smallest ledger found that still shows a difference, and that one
        pub smallest: FuzzLedger,
        pub smallest_difference: Difference,
        pub blocks: usize,
        pub shrinking: Duration,
    }

    impl fmt::Display for Finding {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            writeln!(
                f,
                "seed {}: zhang:budget answers otherwise than the oracle (the seed exercises: {})",
                self.seed,
                self.exercised.iter().copied().collect::<Vec<_>>().join(", ")
            )?;
            writeln!(f, "  first difference: {}", self.difference)?;
            writeln!(
                f,
                "  on the smallest ledger found ({} of {} directives, {:.1?} to find): {}",
                self.smallest.blocks.len(),
                self.blocks,
                self.shrinking,
                self.smallest_difference
            )?;
            writeln!(
                f,
                "  again: {SEEDS_ENV}={} cargo nextest run -p zhang-server --test golden -E 'test(budget_fuzz)'",
                self.seed
            )?;
            write!(f, "{}", self.smallest.text())
        }
    }

    /// The first difference the testing guide does not allow between the oracle and the script on `ledger`,
    /// if any; `None` too when it does not load either way, which a smaller ledger may not.
    async fn first_regression(ledger: &FuzzLedger) -> Option<Difference> {
        let scratch = ledger.scratch();
        let report = compare_until(scratch.dir(), MAIN, ledger.clock(), &requests(ledger), true).await.ok()?;
        report.differences.into_iter().find(|it| it.allowed.is_none())
    }

    /// The fewest blocks of `ledger` found to still show a difference, removing runs of blocks, halved until single
    /// ones, for at most [`SHRINKING`].
    async fn shrink(mut ledger: FuzzLedger, mut difference: Difference) -> (FuzzLedger, Difference) {
        let deadline = Instant::now() + SHRINKING;
        let mut chunk = (ledger.blocks.len() / 2).max(1);
        loop {
            let (mut start, mut removed) = (0, false);
            while start < ledger.blocks.len() && Instant::now() < deadline {
                let keep = (0..ledger.blocks.len()).map(|at| at < start || at >= start + chunk).collect::<Vec<_>>();
                let smaller = ledger.with_blocks(&keep);
                match first_regression(&smaller).await {
                    Some(found) => (ledger, difference, removed) = (smaller, found, true),
                    None => start += chunk,
                }
            }
            if Instant::now() >= deadline || (chunk == 1 && !removed) {
                return (ledger, difference);
            }
            chunk = (chunk / 2).max(1);
        }
    }

    /// The ledger of `seed` compared with the oracle and with the script: what was compared, or the first difference with the
    /// smallest reproduction found.
    pub async fn check(seed: u64) -> Result<Report, Box<Finding>> {
        let ledger = FuzzLedger::generate(seed);
        let scratch = ledger.scratch();
        let report = match compare_until(scratch.dir(), MAIN, ledger.clock(), &requests(&ledger), false).await {
            Ok(report) => report,
            Err(error) => panic!("seed {seed}: the generated ledger does not load: {error}\n{}", ledger.text()),
        };
        let Some(difference) = report.regressions().next().cloned() else {
            return Ok(report);
        };
        let started = Instant::now();
        let (smallest, smallest_difference) = shrink(ledger.clone(), difference.clone()).await;
        Err(Box::new(Finding {
            seed,
            exercised: ledger.exercised.clone(),
            difference,
            smallest,
            smallest_difference,
            blocks: ledger.blocks.len(),
            shrinking: started.elapsed(),
        }))
    }
}

#[cfg(feature = "server")]
pub use run::{check, requests, seeds, Finding, SEEDS_ENV};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_seed_gives_one_ledger() {
        for seed in 1..20 {
            let (one, two) = (FuzzLedger::generate(seed), FuzzLedger::generate(seed));
            assert_eq!(one.text(), two.text());
            assert_eq!(one.now, two.now);
        }
        assert_ne!(FuzzLedger::generate(1).text(), FuzzLedger::generate(2).text());
    }

    #[test]
    fn a_ledger_with_fewer_blocks_keeps_its_header() {
        let ledger = FuzzLedger::generate(7);
        let none = ledger.with_blocks(&vec![false; ledger.blocks.len()]);
        assert!(none.blocks.is_empty());
        assert!(none.text().contains("option \"timezone\""));
    }
}
