//! The query type system: [`DataType`], the typed runtime [`Value`] and the accounting
//! types [`Position`], [`Cost`] and [`Inventory`].
//!
//! Amounts reuse [`zhang_ast::amount::Amount`] (re-exported as [`crate::Amount`]) so values
//! flow between the ledger and the engine without conversion.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::hash::{Hash, Hasher};
use std::ops::Neg;
use std::sync::Arc;

use bigdecimal::{BigDecimal, Zero};
use chrono::{Datelike, Duration, NaiveDate};
use zhang_ast::amount::Amount;

use crate::decimal::to_plain_string;

/// The static type of an expression or a result column.
///
/// The lower-case names returned by [`DataType::name`] are the type names used by the
/// HTTP API and in function signatures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum DataType {
    /// The type of the `NULL` literal; it is compatible with every other type.
    Null,
    Bool,
    /// 64-bit signed integer.
    Int,
    /// Exact decimal number.
    Decimal,
    Str,
    Date,
    /// A set of strings, e.g. `tags` and `links`.
    Set,
    Amount,
    Position,
    Inventory,
    /// A calendar interval of months and days, as `interval('1 month')` builds it.
    Interval,
    /// Metadata as an ordered list of `(key, value)` pairs, e.g. the `metas` column.
    Metas,
}

impl DataType {
    pub fn name(&self) -> &'static str {
        match self {
            DataType::Null => "null",
            DataType::Bool => "bool",
            DataType::Int => "int",
            DataType::Decimal => "decimal",
            DataType::Str => "str",
            DataType::Date => "date",
            DataType::Set => "set",
            DataType::Amount => "amount",
            DataType::Position => "position",
            DataType::Inventory => "inventory",
            DataType::Interval => "interval",
            DataType::Metas => "metas",
        }
    }

    pub fn is_numeric(&self) -> bool {
        matches!(self, DataType::Int | DataType::Decimal)
    }
}

impl fmt::Display for DataType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// The cost basis of a lot: per-unit cost number and currency, acquisition date and label.
///
/// Ordered by number, currency, date, label.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Cost {
    /// per-unit cost
    pub number: BigDecimal,
    pub currency: String,
    pub date: Option<NaiveDate>,
    pub label: Option<String>,
}

impl fmt::Display for Cost {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{{{} {}", to_plain_string(&self.number), self.currency)?;
        if let Some(date) = self.date {
            write!(f, ", {}", date)?;
        }
        if let Some(label) = &self.label {
            write!(f, ", \"{}\"", label)?;
        }
        f.write_str("}")
    }
}

/// Units of a commodity, optionally held at a [`Cost`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Position {
    pub units: Amount,
    pub cost: Option<Cost>,
}

impl Position {
    pub fn new(units: Amount, cost: Option<Cost>) -> Self {
        Position { units, cost }
    }

    /// The total cost of the position (`units × cost.number` in the cost currency, in the decimal
    /// context like beanquery's, as a cost may be a 28-digit quotient), or the units themselves when
    /// the position is not held at cost.
    pub fn at_cost(&self) -> Amount {
        match &self.cost {
            Some(cost) => Amount::new(crate::decimal::mul_in_context(&self.units.number, &cost.number), cost.currency.clone()),
            None => self.units.clone(),
        }
    }
}

impl Hash for Position {
    fn hash<H: Hasher>(&self, state: &mut H) {
        hash_amount(&self.units, state);
        self.cost.hash(state);
    }
}

impl Neg for Position {
    type Output = Position;

    fn neg(self) -> Self::Output {
        Position {
            units: self.units.neg(),
            cost: self.cost,
        }
    }
}

impl fmt::Display for Position {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", to_plain_string(&self.units.number), self.units.commodity)?;
        if let Some(cost) = &self.cost {
            write!(f, " {}", cost)?;
        }
        Ok(())
    }
}

/// A multiset of positions keyed by (units currency, cost): adding a position merges it
/// into the lot with the same key, and lots that net to zero disappear.
///
/// Positions iterate (and serialise) sorted by units currency, then cost, with the
/// no-cost lot first.
///
/// Clones share their lots until one of them changes (copy on write), so a clone is cheap:
/// the running `balance` hands every row a clone of the same inventory.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct Inventory {
    lots: Arc<Lots>,
}

type Lots = BTreeMap<(String, Option<Cost>), BigDecimal>;

impl Inventory {
    pub fn new() -> Self {
        Inventory::default()
    }

    pub fn add_position(&mut self, position: &Position) {
        self.add(&position.units, position.cost.as_ref());
    }

    /// [`Inventory::add_position`] for a position the caller no longer needs: its currency
    /// and cost move into the inventory instead of being copied.
    pub(crate) fn add_owned_position(&mut self, position: Position) {
        let Position { units, cost } = position;
        let key = (units.commodity, cost);
        if units.number.is_zero() && !self.lots.contains_key(&key) {
            // nothing changes, so a shared inventory stays shared
            return;
        }
        // (adding zero to a lot still widens the scale of its number)
        let lots = Arc::make_mut(&mut self.lots);
        match lots.get_mut(&key) {
            Some(existing) => {
                *existing += &units.number;
                if existing.is_zero() {
                    lots.remove(&key);
                }
            }
            None => {
                lots.insert(key, units.number);
            }
        }
    }

    /// Add units without cost.
    pub fn add_amount(&mut self, amount: &Amount) {
        self.add(amount, None);
    }

    pub fn add_inventory(&mut self, other: &Inventory) {
        for ((currency, cost), number) in other.lots.iter() {
            self.add_number(currency, cost.as_ref(), number);
        }
    }

    fn add(&mut self, units: &Amount, cost: Option<&Cost>) {
        self.add_number(&units.commodity, cost, &units.number);
    }

    fn add_number(&mut self, currency: &str, cost: Option<&Cost>, number: &BigDecimal) {
        let key = (currency.to_owned(), cost.cloned());
        if number.is_zero() && !self.lots.contains_key(&key) {
            return;
        }
        let lots = Arc::make_mut(&mut self.lots);
        let remove = match lots.get_mut(&key) {
            Some(existing) => {
                *existing += number;
                existing.is_zero()
            }
            None => {
                lots.insert(key.clone(), number.clone());
                false
            }
        };
        if remove {
            lots.remove(&key);
        }
    }

    /// The number of the lot with this (units currency, cost) key, if it is open.
    pub(crate) fn lot(&self, key: &(String, Option<Cost>)) -> Option<&BigDecimal> {
        self.lots.get(key)
    }

    pub fn is_empty(&self) -> bool {
        self.lots.is_empty()
    }

    /// number of lots
    pub fn len(&self) -> usize {
        self.lots.len()
    }

    /// The positions, sorted by units currency then cost.
    pub fn positions(&self) -> impl Iterator<Item = Position> + '_ {
        self.lots.iter().map(|((currency, cost), number)| Position {
            units: Amount::new(number.clone(), currency.clone()),
            cost: cost.clone(),
        })
    }

    /// The units currency and number of every lot, in the order of
    /// [`Inventory::positions`], without copying them.
    pub(crate) fn lot_units(&self) -> impl Iterator<Item = (&str, &BigDecimal)> + '_ {
        self.lots.iter().map(|((currency, _), number)| (currency.as_str(), number))
    }

    /// Reduce every position to an amount with `f` and sum the results into a new
    /// inventory without costs (beancount's `Inventory.reduce`).
    pub fn reduce(&self, mut f: impl FnMut(&Position) -> Amount) -> Inventory {
        let mut ret = Inventory::new();
        for position in self.positions() {
            ret.add_amount(&f(&position));
        }
        ret
    }

    /// The units of every lot, merged per currency.
    pub fn units(&self) -> Inventory {
        // like `reduce`, without copying the costs it drops
        let mut ret = Inventory::new();
        for (currency, number) in self.lot_units() {
            ret.add_number(currency, None, number);
        }
        ret
    }

    /// The cost of every lot, merged per currency.
    pub fn at_cost(&self) -> Inventory {
        self.reduce(Position::at_cost)
    }
}

impl Neg for Inventory {
    type Output = Inventory;

    fn neg(self) -> Self::Output {
        Inventory {
            lots: Arc::new(Arc::unwrap_or_clone(self.lots).into_iter().map(|(key, number)| (key, -number)).collect()),
        }
    }
}

impl FromIterator<Position> for Inventory {
    fn from_iter<T: IntoIterator<Item = Position>>(iter: T) -> Self {
        let mut inventory = Inventory::new();
        for position in iter {
            inventory.add_position(&position);
        }
        inventory
    }
}

impl fmt::Display for Inventory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (idx, position) in self.positions().enumerate() {
            if idx > 0 {
                f.write_str(", ")?;
            }
            write!(f, "{}", position)?;
        }
        Ok(())
    }
}

/// A calendar interval: a number of months (a year counts as twelve) and a number of days,
/// either of which may be negative. It is beanquery's `relativedelta` as `interval()` builds
/// it, and it moves a date the same way: by the months first, keeping the day of the month
/// unless the month it lands in is shorter (then its last day), and then by the days.
///
/// Intervals are not ordered: `1 month` is not comparable with `30 days`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Interval {
    pub months: i64,
    pub days: i64,
}

impl Interval {
    pub fn new(months: i64, days: i64) -> Self {
        Interval { months, days }
    }

    /// `date` moved by the interval: by the months first (the day of the month is kept,
    /// or the last day of a shorter month is taken), then by the days. `None` when the
    /// result is not a representable date.
    pub fn add_to(&self, date: NaiveDate) -> Option<NaiveDate> {
        let date = if self.months == 0 {
            date
        } else {
            let index = (date.year() as i64)
                .checked_mul(12)?
                .checked_add(date.month0() as i64)?
                .checked_add(self.months)?;
            let year = i32::try_from(index.div_euclid(12)).ok()?;
            let month = index.rem_euclid(12) as u32 + 1;
            let day = date.day().min(days_in_month(year, month)?);
            NaiveDate::from_ymd_opt(year, month, day)?
        };
        date.checked_add_signed(Duration::try_days(self.days)?)
    }

    /// `date` moved back by the interval (beanquery's `date - relativedelta`): by the
    /// negated months first, then by the negated days.
    pub fn subtract_from(&self, date: NaiveDate) -> Option<NaiveDate> {
        self.checked_neg()?.add_to(date)
    }

    pub fn checked_neg(&self) -> Option<Interval> {
        Some(Interval::new(self.months.checked_neg()?, self.days.checked_neg()?))
    }

    pub fn checked_add(&self, other: &Interval) -> Option<Interval> {
        Some(Interval::new(self.months.checked_add(other.months)?, self.days.checked_add(other.days)?))
    }

    pub fn checked_sub(&self, other: &Interval) -> Option<Interval> {
        self.checked_add(&other.checked_neg()?)
    }
}

/// The dates a query computes: years 1 to 9999, the calendar of beancount (Python's
/// `datetime.date`). A date function or date arithmetic whose result falls outside is NULL.
pub(crate) fn in_calendar(date: NaiveDate) -> bool {
    (1..=9999).contains(&date.year())
}

/// `date` as a value when it is in the calendar ([`in_calendar`]); NULL otherwise.
pub(crate) fn calendar_value(date: Option<NaiveDate>) -> Value {
    date.filter(|date| in_calendar(*date)).map_or(Value::Null, Value::Date)
}

/// A date of Python's calendar (years 1 to 9999), as `datetime.date` builds it; `None` when
/// there is no such day.
pub(crate) fn python_date(year: i64, month: i64, day: i64) -> Option<NaiveDate> {
    if !(1..=9999).contains(&year) {
        return None;
    }
    NaiveDate::from_ymd_opt(year as i32, u32::try_from(month).ok()?, u32::try_from(day).ok()?)
}

/// The date a text names, or `None`: the one text-to-date rule of the engine. `date(text)`, a
/// string compared with a date, a bare date literal and a `date` parameter given as text all
/// read text with it.
///
/// It is beanquery's `date(text)`, Python's `strptime(text, '%Y-%m-%d')`: a four-digit year, a
/// month of one or two digits and a day of one or two digits (or a space and one digit), and
/// nothing else around them, in the years 1 to 9999.
///
/// ```
/// use chrono::NaiveDate;
/// use zhang_query::value::parse_date;
///
/// assert_eq!(parse_date("2024-2-9"), NaiveDate::from_ymd_opt(2024, 2, 9));
/// assert_eq!(parse_date("2024-02- 9"), NaiveDate::from_ymd_opt(2024, 2, 9));
/// assert_eq!(parse_date(" 2024-02-09"), None);
/// assert_eq!(parse_date("+2024-02-09"), None);
/// assert_eq!(parse_date("24-02-09"), None);
/// assert_eq!(parse_date("2023-02-29"), None);
/// ```
pub fn parse_date(text: &str) -> Option<NaiveDate> {
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

/// The number of days of a month; `None` for a month outside the calendar.
pub(crate) fn days_in_month(year: i32, month: u32) -> Option<u32> {
    let first = NaiveDate::from_ymd_opt(year, month, 1)?;
    let next = if month == 12 {
        NaiveDate::from_ymd_opt(year.checked_add(1)?, 1, 1)?
    } else {
        NaiveDate::from_ymd_opt(year, month + 1, 1)?
    };
    Some((next - first).num_days() as u32)
}

/// `1 year 2 months -3 days`: the months as years and months (both with the sign of the
/// months), then the days; zero parts are left out, and the zero interval is `0 days`.
impl fmt::Display for Interval {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let unit = |n: i64, singular: &str| format!("{} {}{}", n, singular, if n.abs() == 1 { "" } else { "s" });
        let years = self.months / 12;
        let months = self.months % 12;
        let mut parts = vec![];
        if years != 0 {
            parts.push(unit(years, "year"));
        }
        if months != 0 {
            parts.push(unit(months, "month"));
        }
        if self.days != 0 || parts.is_empty() {
            parts.push(unit(self.days, "day"));
        }
        f.write_str(&parts.join(" "))
    }
}

/// Metadata as `(key, value)` pairs in order: the value of the `metas` columns and of
/// `open_meta(account)`.
pub type Metas = Vec<(String, String)>;

/// `key: value` pairs joined by `; `, the text of a [`Value::Metas`].
pub(crate) fn metas_to_string(metas: &[(String, String)]) -> String {
    metas.iter().map(|(key, value)| format!("{}: {}", key, value)).collect::<Vec<_>>().join("; ")
}

/// A typed runtime value. Any value may be `Null`.
#[derive(Debug, Clone)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Decimal(BigDecimal),
    Str(String),
    Date(NaiveDate),
    Set(BTreeSet<String>),
    Amount(Amount),
    Position(Position),
    Inventory(Inventory),
    Interval(Interval),
    Metas(Metas),
}

impl Value {
    pub fn data_type(&self) -> DataType {
        match self {
            Value::Null => DataType::Null,
            Value::Bool(_) => DataType::Bool,
            Value::Int(_) => DataType::Int,
            Value::Decimal(_) => DataType::Decimal,
            Value::Str(_) => DataType::Str,
            Value::Date(_) => DataType::Date,
            Value::Set(_) => DataType::Set,
            Value::Amount(_) => DataType::Amount,
            Value::Position(_) => DataType::Position,
            Value::Inventory(_) => DataType::Inventory,
            Value::Interval(_) => DataType::Interval,
            Value::Metas(_) => DataType::Metas,
        }
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(it) => Some(*it),
            _ => None,
        }
    }
    pub fn as_int(&self) -> Option<i64> {
        match self {
            Value::Int(it) => Some(*it),
            _ => None,
        }
    }
    /// The value as a decimal; integers are widened.
    pub fn as_decimal(&self) -> Option<BigDecimal> {
        match self {
            Value::Int(it) => Some(BigDecimal::from(*it)),
            Value::Decimal(it) => Some(it.clone()),
            _ => None,
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(it) => Some(it),
            _ => None,
        }
    }
    pub fn as_date(&self) -> Option<NaiveDate> {
        match self {
            Value::Date(it) => Some(*it),
            _ => None,
        }
    }
    pub fn as_set(&self) -> Option<&BTreeSet<String>> {
        match self {
            Value::Set(it) => Some(it),
            _ => None,
        }
    }
    pub fn as_amount(&self) -> Option<&Amount> {
        match self {
            Value::Amount(it) => Some(it),
            _ => None,
        }
    }
    pub fn as_inventory(&self) -> Option<&Inventory> {
        match self {
            Value::Inventory(it) => Some(it),
            _ => None,
        }
    }
    pub fn as_metas(&self) -> Option<&[(String, String)]> {
        match self {
            Value::Metas(it) => Some(it),
            _ => None,
        }
    }

    /// The total order used by `ORDER BY`, `min()` and `max()`.
    ///
    /// `NULL` sorts before everything else; integers and decimals compare numerically;
    /// amounts compare by (currency, number); positions and inventories follow
    /// beancount's position sort key (common currencies first, then cost, then units).
    pub fn sort_cmp(&self, other: &Value) -> Ordering {
        match (self, other) {
            (Value::Null, Value::Null) => Ordering::Equal,
            (Value::Null, _) => Ordering::Less,
            (_, Value::Null) => Ordering::Greater,
            (Value::Bool(a), Value::Bool(b)) => a.cmp(b),
            (Value::Int(a), Value::Int(b)) => a.cmp(b),
            (Value::Int(a), Value::Decimal(b)) => BigDecimal::from(*a).cmp(b),
            (Value::Decimal(a), Value::Int(b)) => a.cmp(&BigDecimal::from(*b)),
            (Value::Decimal(a), Value::Decimal(b)) => a.cmp(b),
            (Value::Str(a), Value::Str(b)) => a.cmp(b),
            (Value::Date(a), Value::Date(b)) => a.cmp(b),
            (Value::Set(a), Value::Set(b)) => a.cmp(b),
            (Value::Amount(a), Value::Amount(b)) => a.commodity.cmp(&b.commodity).then_with(|| a.number.cmp(&b.number)),
            (Value::Position(a), Value::Position(b)) => position_sort_cmp(a, b),
            (Value::Inventory(a), Value::Inventory(b)) => {
                let mut left = a.positions().collect::<Vec<_>>();
                let mut right = b.positions().collect::<Vec<_>>();
                left.sort_by(position_sort_cmp);
                right.sort_by(position_sort_cmp);
                for (l, r) in left.iter().zip(right.iter()) {
                    let ord = position_sort_cmp(l, r);
                    if ord != Ordering::Equal {
                        return ord;
                    }
                }
                left.len().cmp(&right.len())
            }
            // intervals are not comparable in BQL; this order only makes sorting total
            (Value::Interval(a), Value::Interval(b)) => (a.months, a.days).cmp(&(b.months, b.days)),
            (Value::Metas(a), Value::Metas(b)) => a.cmp(b),
            // mixed types never appear in a single typed column; order by type for totality
            (a, b) => a.data_type().cmp(&b.data_type()),
        }
    }
}

/// Currencies beancount orders first when sorting positions.
const COMMON_CURRENCIES: [&str; 8] = ["USD", "EUR", "JPY", "CAD", "GBP", "AUD", "NZD", "CHF"];

/// beancount's rank of a currency when sorting positions: the common currencies in their
/// order, then every other currency by the length of its name in characters (Python's `len`).
fn currency_rank(currency: &str) -> usize {
    COMMON_CURRENCIES
        .iter()
        .position(|it| *it == currency)
        .unwrap_or(COMMON_CURRENCIES.len() + currency.chars().count())
}

/// beancount's position sort key as a comparison: (currency rank, cost number, cost currency,
/// units number). The currency name, cost date and label take no part, so positions that
/// compare equal keep their order under a stable sort, as beancount prints an inventory.
pub(crate) fn position_sort_key_cmp(a: &Position, b: &Position) -> Ordering {
    let zero = BigDecimal::zero();
    let (a_cost_number, a_cost_currency) = a.cost.as_ref().map(|c| (&c.number, c.currency.as_str())).unwrap_or((&zero, ""));
    let (b_cost_number, b_cost_currency) = b.cost.as_ref().map(|c| (&c.number, c.currency.as_str())).unwrap_or((&zero, ""));
    currency_rank(&a.units.commodity)
        .cmp(&currency_rank(&b.units.commodity))
        .then_with(|| a_cost_number.cmp(b_cost_number))
        .then_with(|| a_cost_currency.cmp(b_cost_currency))
        .then_with(|| a.units.number.cmp(&b.units.number))
}

/// [`position_sort_key_cmp`] with the currency name and the cost as final tie-breakers so the
/// order is total (`ORDER BY`, `min()`, `max()` and `str()`).
pub(crate) fn position_sort_cmp(a: &Position, b: &Position) -> Ordering {
    position_sort_key_cmp(a, b)
        .then_with(|| a.units.commodity.cmp(&b.units.commodity))
        .then_with(|| a.cost.cmp(&b.cost))
}

fn hash_amount<H: Hasher>(amount: &Amount, state: &mut H) {
    amount.number.hash(state);
    amount.commodity.hash(state);
}

/// Structural equality; integers and decimals compare numerically (`1 = 1.00`).
impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Value::Null, Value::Null) => true,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Int(a), Value::Int(b)) => a == b,
            (Value::Int(a), Value::Decimal(b)) | (Value::Decimal(b), Value::Int(a)) => &BigDecimal::from(*a) == b,
            (Value::Decimal(a), Value::Decimal(b)) => a == b,
            (Value::Str(a), Value::Str(b)) => a == b,
            (Value::Date(a), Value::Date(b)) => a == b,
            (Value::Set(a), Value::Set(b)) => a == b,
            (Value::Amount(a), Value::Amount(b)) => a == b,
            (Value::Position(a), Value::Position(b)) => a == b,
            (Value::Inventory(a), Value::Inventory(b)) => a == b,
            (Value::Interval(a), Value::Interval(b)) => a == b,
            (Value::Metas(a), Value::Metas(b)) => a == b,
            _ => false,
        }
    }
}

impl Eq for Value {}

impl Hash for Value {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            Value::Null => 0u8.hash(state),
            Value::Bool(it) => it.hash(state),
            // integers hash like the equal decimal so that `1 = 1.0` keeps Hash consistent
            Value::Int(it) => BigDecimal::from(*it).hash(state),
            Value::Decimal(it) => it.hash(state),
            Value::Str(it) => it.hash(state),
            Value::Date(it) => it.hash(state),
            Value::Set(it) => it.hash(state),
            Value::Amount(it) => hash_amount(it, state),
            Value::Position(it) => it.hash(state),
            Value::Inventory(it) => it.hash(state),
            Value::Interval(it) => it.hash(state),
            Value::Metas(it) => it.hash(state),
        }
    }
}

/// Human-readable rendering, used by `str()`.
impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Null => f.write_str("NULL"),
            Value::Bool(it) => f.write_str(if *it { "TRUE" } else { "FALSE" }),
            Value::Int(it) => write!(f, "{}", it),
            Value::Decimal(it) => f.write_str(&to_plain_string(it)),
            Value::Str(it) => f.write_str(it),
            Value::Date(it) => write!(f, "{}", it.format("%Y-%m-%d")),
            Value::Set(it) => f.write_str(&it.iter().map(String::as_str).collect::<Vec<_>>().join(", ")),
            Value::Amount(it) => write!(f, "{} {}", to_plain_string(&it.number), it.commodity),
            Value::Position(it) => write!(f, "{}", it),
            Value::Inventory(it) => write!(f, "{}", it),
            Value::Interval(it) => write!(f, "{}", it),
            Value::Metas(it) => f.write_str(&metas_to_string(it)),
        }
    }
}

macro_rules! impl_from {
    ($t:ty, $variant:ident) => {
        impl From<$t> for Value {
            fn from(value: $t) -> Self {
                Value::$variant(value)
            }
        }
    };
}

impl_from!(bool, Bool);
impl_from!(i64, Int);
impl_from!(BigDecimal, Decimal);
impl_from!(String, Str);
impl_from!(NaiveDate, Date);
impl_from!(BTreeSet<String>, Set);
impl_from!(Amount, Amount);
impl_from!(Position, Position);
impl_from!(Inventory, Inventory);
impl_from!(Interval, Interval);
impl_from!(Metas, Metas);

impl From<i32> for Value {
    fn from(value: i32) -> Self {
        Value::Int(value as i64)
    }
}

impl From<&str> for Value {
    fn from(value: &str) -> Self {
        Value::Str(value.to_owned())
    }
}

impl<T: Into<Value>> From<Option<T>> for Value {
    fn from(value: Option<T>) -> Self {
        value.map(Into::into).unwrap_or(Value::Null)
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;

    fn amount(n: &str, c: &str) -> Amount {
        Amount::new(BigDecimal::from_str(n).unwrap(), c)
    }

    fn cost(n: &str, c: &str) -> Option<Cost> {
        Some(Cost {
            number: BigDecimal::from_str(n).unwrap(),
            currency: c.to_owned(),
            date: NaiveDate::from_ymd_opt(2024, 1, 1),
            label: None,
        })
    }

    #[test]
    fn inventory_merges_lots_and_drops_zero_positions() {
        let mut inventory = Inventory::new();
        inventory.add_position(&Position::new(amount("10", "AAPL"), cost("100", "USD")));
        inventory.add_position(&Position::new(amount("5", "AAPL"), cost("100.00", "USD")));
        inventory.add_position(&Position::new(amount("1", "AAPL"), cost("120", "USD")));
        inventory.add_amount(&amount("4.00", "USD"));
        assert_eq!(inventory.len(), 3);
        inventory.add_position(&Position::new(amount("-1", "AAPL"), cost("120", "USD")));
        assert_eq!(inventory.len(), 2);
        assert_eq!(inventory.to_string(), "15 AAPL {100 USD, 2024-01-01}, 4.00 USD");
        assert_eq!(inventory.units().to_string(), "15 AAPL, 4.00 USD");
        assert_eq!(inventory.at_cost().to_string(), "1504.00 USD");
    }

    #[test]
    fn clones_share_their_lots_until_one_changes() {
        let mut inventory = Inventory::new();
        inventory.add_position(&Position::new(amount("10", "AAPL"), cost("100", "USD")));
        let snapshot = inventory.clone();
        assert!(Arc::ptr_eq(&inventory.lots, &snapshot.lots));
        // adding zero to a missing lot changes nothing
        inventory.add_amount(&amount("0", "USD"));
        assert!(Arc::ptr_eq(&inventory.lots, &snapshot.lots));
        inventory.add_amount(&amount("1", "USD"));
        assert!(!Arc::ptr_eq(&inventory.lots, &snapshot.lots));
        assert_eq!(snapshot.to_string(), "10 AAPL {100 USD, 2024-01-01}");
        assert_eq!(inventory.to_string(), "10 AAPL {100 USD, 2024-01-01}, 1 USD");
        // adding zero to a lot widens its scale, as before
        inventory.add_amount(&amount("0.00", "USD"));
        assert_eq!(inventory.to_string(), "10 AAPL {100 USD, 2024-01-01}, 1.00 USD");
        assert_eq!(-inventory.clone(), -inventory);
    }

    #[test]
    fn positions_rank_other_currencies_by_characters_like_beancount() {
        // beancount ranks a currency outside the common ones by `len(currency)`, which counts
        // characters, not bytes: a three-character CJK name sorts with "CNY", before "ABCD".
        let position = |currency: &str| Value::Position(Position::new(amount("1", currency), None));
        assert_eq!(position("人民币").sort_cmp(&position("ABCD")), Ordering::Less);
        assert_eq!(position("AB").sort_cmp(&position("人民币")), Ordering::Less);
        assert_eq!(position("USD").sort_cmp(&position("人民币")), Ordering::Less);
        // a tie on the key is broken by the currency name, so the order stays total
        assert_eq!(position("CNY").sort_cmp(&position("人民币")), Ordering::Less);
        assert_eq!(
            position_sort_key_cmp(&Position::new(amount("1", "CNY"), None), &Position::new(amount("1", "人民币"), None)),
            Ordering::Equal
        );
    }

    #[test]
    fn null_sorts_first_and_numbers_compare_across_types() {
        assert_eq!(Value::Null.sort_cmp(&Value::Int(1)), Ordering::Less);
        assert_eq!(Value::Int(2).sort_cmp(&Value::Decimal(BigDecimal::from_str("1.5").unwrap())), Ordering::Greater);
        assert_eq!(Value::Int(1), Value::Decimal(BigDecimal::from_str("1.00").unwrap()));
    }

    #[test]
    fn intervals_move_by_months_then_days() {
        let day = |text: &str| NaiveDate::parse_from_str(text, "%Y-%m-%d").unwrap();
        let month = Interval::new(1, 0);
        assert_eq!(month.add_to(day("2024-01-31")), Some(day("2024-02-29")));
        assert_eq!(month.add_to(day("2023-01-31")), Some(day("2023-02-28")));
        assert_eq!(month.subtract_from(day("2024-03-31")), Some(day("2024-02-29")));
        assert_eq!(Interval::new(-12, 0).add_to(day("2024-02-29")), Some(day("2023-02-28")));
        assert_eq!(Interval::new(1, 1).add_to(day("2024-01-31")), Some(day("2024-03-01")));
        assert_eq!(Interval::new(0, -1).add_to(day("2024-03-01")), Some(day("2024-02-29")));
        assert_eq!(Interval::new(i64::MAX, 0).add_to(day("2024-01-01")), None);
        assert_eq!(Interval::new(0, i64::MIN).add_to(day("2024-01-01")), None);
        assert_eq!(Interval::new(13, -3).to_string(), "1 year 1 month -3 days");
        assert_eq!(Interval::new(-1, 1).to_string(), "-1 month 1 day");
        assert_eq!(Interval::new(24, 0).to_string(), "2 years");
        assert_eq!(Value::Interval(Interval::new(1, 0)).data_type(), DataType::Interval);
        assert_eq!(Interval::new(1, 2).checked_sub(&Interval::new(1, 2)), Some(Interval::default()));
        assert_eq!(Interval::new(i64::MIN, 0).checked_neg(), None);
    }

    #[test]
    fn metas_render_as_pairs() {
        let metas = Value::Metas(vec![("a".into(), "1".into()), ("a".into(), "2".into()), ("b".into(), "x: y".into())]);
        assert_eq!(metas.to_string(), "a: 1; a: 2; b: x: y");
        assert_eq!(Value::Metas(vec![]).to_string(), "");
        assert_eq!(metas.data_type(), DataType::Metas);
    }

    #[test]
    fn inventories_order_by_common_currency_first() {
        let mut a = Inventory::new();
        a.add_amount(&amount("10", "USD"));
        a.add_amount(&amount("99999", "IRAUSD"));
        let mut b = Inventory::new();
        b.add_amount(&amount("20", "USD"));
        assert_eq!(Value::Inventory(a).sort_cmp(&Value::Inventory(b)), Ordering::Less);
    }
}
