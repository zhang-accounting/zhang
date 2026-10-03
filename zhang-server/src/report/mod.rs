//! The report of the web UI, `GET /api/statistic/summary`, `/api/statistic/graph` and
//! `/api/statistic/{account_type}`, as built-in queries.
//!
//! Every figure is a named BQL query, listed with the other built-in queries and openable in
//! Explore, so anything the report shows can also be queried, and varied, by hand. The
//! functions here only bind the parameters and map the rows into the response types; the
//! figures themselves (balances, flows, valuation, bucketing, ranking) are the engine's.
//!
//! Ranges are ledger dates, both inclusive. Valuation follows #479: the summary and the
//! rankings are valued at the prices of the last day of the range, and each point of the
//! graph at the prices of its own last day in the range.

pub mod legacy;

use std::collections::{BTreeMap, HashMap};
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{DateTime, Datelike, Days, Months, NaiveDate, NaiveTime, TimeZone, Utc};
use chrono_tz::Tz;
use zhang_ast::amount::CalculatedAmount;
use zhang_ast::AccountType;
use zhang_core::domains::schemas::AccountJournalDomain;
use zhang_core::ledger::Ledger;
use zhang_query::{DataType, Inventory, Params, PriceMap, Value};

use crate::builtin::{calculated_amount, execute, BuiltinQuery, LedgerDateRange};
use crate::error::ServerError;
use crate::request::StatisticInterval;
use crate::response::{ReportRankItemEntity, StatisticGraphEntity, StatisticRankEntity, StatisticSummaryEntity};
use crate::routes::query::{execute_options, max_result_values};
use crate::ServerResult;

/// The net worth at the end of a day: the summary's balance, and the balance the graph starts
/// from on the day before its range.
pub const NET_WORTH: BuiltinQuery = BuiltinQuery {
    name: "report.net_worth",
    description: "The net worth, the balance of the assets and the liabilities, at the end of :to, valued in :currency at the prices of that day.",
    bql: "SELECT sum(position) AS balance, units(sum(position)) AS units, convert(sum(position), :currency, :to) AS value
WHERE (under(account, 'Assets') OR under(account, 'Liabilities')) AND date <= :to",
    params: &[("to", DataType::Date), ("currency", DataType::Str)],
};

/// The liabilities at the end of a day.
pub const LIABILITIES: BuiltinQuery = BuiltinQuery {
    name: "report.liabilities",
    description: "The balance of the liabilities at the end of :to, valued in :currency at the prices of that day.",
    bql: "SELECT units(sum(position)) AS units, convert(sum(position), :currency, :to) AS value
WHERE under(account, 'Liabilities') AND date <= :to",
    params: &[("to", DataType::Date), ("currency", DataType::Str)],
};

/// The income and the expenses of the range.
pub const FLOWS: BuiltinQuery = BuiltinQuery {
    name: "report.flows",
    description: "The income and the expenses from :from to :to, valued in :currency at the prices of :to.",
    bql: "SELECT root(account, 1) AS type, units(sum(position)) AS units, convert(sum(position), :currency, :to) AS value
WHERE (under(account, 'Income') OR under(account, 'Expenses')) AND date >= :from AND date <= :to
GROUP BY type
ORDER BY type",
    params: &[("from", DataType::Date), ("to", DataType::Date), ("currency", DataType::Str)],
};

/// The number of transactions of the range; padding transactions are not counted.
pub const TRANSACTION_COUNT: BuiltinQuery = BuiltinQuery {
    name: "report.transaction_count",
    description: "The number of transactions from :from to :to, without the padding transactions of balance ... with pad.",
    bql: "SELECT count(*) AS transactions
FROM #transactions
WHERE flag != 'P' AND date >= :from AND date <= :to",
    params: &[("from", DataType::Date), ("to", DataType::Date)],
};

/// The net worth at the end of every bucket of the graph that has postings.
pub const NET_WORTH_TREND: BuiltinQuery = BuiltinQuery {
    name: "report.net_worth_trend",
    description: "The net worth at the end of every day, week or month (:interval) from :from to :to that has postings, valued in :currency at the prices of its last day in the range.",
    bql: "SELECT date_bin(:interval, date, 2001-01-01) AS bucket, last(balance) AS balance, units(last(balance)) AS units,
  convert(last(balance), :currency, least(max(date_bin(:interval, date, 2001-01-01)) + interval(:interval) - 1, :to)) AS value
WHERE (under(account, 'Assets') OR under(account, 'Liabilities')) AND date <= :to
GROUP BY bucket
HAVING max(date) >= :from
ORDER BY bucket",
    params: &[
        ("from", DataType::Date),
        ("to", DataType::Date),
        ("interval", DataType::Str),
        ("currency", DataType::Str),
    ],
};

/// What every bucket of the graph changed by, per account type.
pub const CHANGES: BuiltinQuery = BuiltinQuery {
    name: "report.changes",
    description: "What each account type changed by in every day, week or month (:interval) from :from to :to, valued in :currency at the prices of its last day in the range.",
    bql: "SELECT date_bin(:interval, date, 2001-01-01) AS bucket, root(account, 1) AS type, units(sum(position)) AS units,
  convert(sum(position), :currency, least(max(date_bin(:interval, date, 2001-01-01)) + interval(:interval) - 1, :to)) AS value
WHERE date >= :from AND date <= :to
GROUP BY bucket, type
ORDER BY bucket, type",
    params: &[
        ("from", DataType::Date),
        ("to", DataType::Date),
        ("interval", DataType::Str),
        ("currency", DataType::Str),
    ],
};

/// What every account of one type changed by in the range.
pub const ACCOUNT_TOTALS: BuiltinQuery = BuiltinQuery {
    name: "report.account_totals",
    description: "What every account of the type :type changed by from :from to :to, valued in :currency at the prices of :to, smallest first.",
    bql: "SELECT account, units(sum(position)) AS units, convert(sum(position), :currency, :to) AS value
WHERE under(account, :type) AND date >= :from AND date <= :to
GROUP BY account
ORDER BY number(only(:currency, convert(sum(position), :currency, :to))), account",
    params: &[
        ("type", DataType::Str),
        ("from", DataType::Date),
        ("to", DataType::Date),
        ("currency", DataType::Str),
    ],
};

/// The ten largest postings of one account type in the range.
pub const TOP_POSTINGS: BuiltinQuery = BuiltinQuery {
    name: "report.top_postings",
    description:
        "The ten largest postings to accounts of the type :type from :from to :to by their value in :currency at the prices of :to, those without a price last.",
    bql: "SELECT date, time, timestamp, account, id, payee, narration, units(position) AS units,
  only(currency, account_balance) AS account_balance, convert(position, :currency, :to) AS value
WHERE under(account, :type) AND date >= :from AND date <= :to
ORDER BY currency(convert(position, :currency, :to)) = :currency DESC, number(possign(convert(position, :currency, :to), account)) DESC
LIMIT 10",
    params: &[
        ("type", DataType::Str),
        ("from", DataType::Date),
        ("to", DataType::Date),
        ("currency", DataType::Str),
    ],
};

/// `GET /api/statistic/summary`: the net worth and the liabilities at the end of the range,
/// and its income, expenses and number of transactions.
pub fn summary(ledger: &Ledger, range: &LedgerDateRange) -> ServerResult<StatisticSummaryEntity> {
    let currency = ledger.options.operating_currency.as_str();
    let at_end = Params::new().bind("to", range.to).bind("currency", currency);
    let net_worth = single(run(ledger, &NET_WORTH, at_end.clone())?);
    let liabilities = single(run(ledger, &LIABILITIES, at_end)?);
    let flows = by_type(run(ledger, &FLOWS, range.bind(Params::new().bind("currency", currency)))?);
    let count = run(ledger, &TRANSACTION_COUNT, range.bind(Params::new()))?;

    let figure = |figures: &HashMap<String, Figure>, account_type: AccountType| figures.get(&account_type.to_string()).cloned().unwrap_or_default();
    let timezone = &ledger.options.timezone;
    Ok(StatisticSummaryEntity {
        from: first_instant(range.from, timezone),
        to: last_instant(range.to, timezone),
        balance: net_worth.amount(currency),
        liability: liabilities.amount(currency),
        income: figure(&flows, AccountType::Income).amount(currency),
        expense: figure(&flows, AccountType::Expenses).amount(currency),
        // an aggregate query without rows has no row, not a zero
        transaction_number: count.into_iter().next().and_then(|mut row| row.take("transactions").as_int()).unwrap_or(0),
    })
}

/// The limits of building a graph, by default those of every query the server runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GraphLimits {
    /// at most this many values in the graph's points and changes: two per point or change
    /// (its day and its amount) and one per currency of its units
    pub max_values: u64,
    /// stop once building the graph, its queries included, has run this long
    pub timeout: Option<Duration>,
}

impl GraphLimits {
    /// The limits of `POST /api/query`: its result size limit and its time limit.
    pub fn server() -> GraphLimits {
        let options = execute_options(max_result_values());
        GraphLimits {
            max_values: options.max_result_values.unwrap_or(u64::MAX),
            timeout: options.timeout,
        }
    }

    /// The most points a graph may have: every point is at least two values.
    pub fn max_points(&self) -> u64 {
        self.max_values / POINT_VALUES
    }
}

/// The values of a point or a change besides those of its currencies: its day and its amount.
const POINT_VALUES: u64 = 2;

/// `GET /api/statistic/graph`: the net worth at the end of every day, week or month of the
/// range, and what each account type changed by in it, keyed by the bucket's first day. See
/// [`graph_rows`] and [`GraphRows::build`], which the endpoint runs apart so that the ledger is
/// only locked while the queries run.
pub fn graph(ledger: &Ledger, range: &LedgerDateRange, interval: &StatisticInterval) -> ServerResult<StatisticGraphEntity> {
    graph_rows(ledger, range, interval, GraphLimits::server())?.build()
}

/// What the graph reads from the ledger: the rows of its queries, and the ledger's price map to
/// value the buckets without postings. Building the graph from them needs no ledger.
pub struct GraphRows {
    range: LedgerDateRange,
    interval: StatisticInterval,
    currency: String,
    /// the balance at the end of the day before the range
    opening: Inventory,
    /// the closing balance (with its lots) and figure of every bucket with postings in the range
    closing: BTreeMap<NaiveDate, (Inventory, Figure)>,
    changes: HashMap<NaiveDate, HashMap<AccountType, CalculatedAmount>>,
    prices: Arc<PriceMap>,
    limits: GraphLimits,
    started: Instant,
}

/// Run the graph's queries: `report.net_worth_trend`, `report.net_worth` of the day before the
/// range and `report.changes`. A range with more buckets than the limits allow points is a 400 before
/// any query runs.
pub fn graph_rows(ledger: &Ledger, range: &LedgerDateRange, interval: &StatisticInterval, limits: GraphLimits) -> ServerResult<GraphRows> {
    let started = Instant::now();
    let points = bucket_count(range, interval);
    if points > limits.max_points() {
        return Err(ServerError::InvalidInput(format!(
            "the graph from {} to {} by {} would have {} points, and the server returns at most {} (half of the result size limit, ZHANG_QUERY_MAX_RESULT_VALUES); ask for weeks or months, or a shorter range",
            range.from,
            range.to,
            unit(interval),
            points,
            limits.max_points()
        )));
    }
    let currency = ledger.options.operating_currency.clone();
    let params = range.bind(Params::new().bind("interval", stride(interval)).bind("currency", currency.as_str()));

    let mut closing = BTreeMap::new();
    for mut row in run(ledger, &NET_WORTH_TREND, params.clone())? {
        if let Value::Date(bucket) = row.take("bucket") {
            closing.insert(bucket, (inventory(row.take("balance")), Figure::of(row.take("units"), row.take("value"))));
        }
    }
    let mut opening = Inventory::new();
    if let Some(day_before) = range.from.pred_opt() {
        for mut row in run(ledger, &NET_WORTH, Params::new().bind("to", day_before).bind("currency", currency.as_str()))? {
            opening = inventory(row.take("balance"));
        }
    }
    let mut changes: HashMap<NaiveDate, HashMap<AccountType, CalculatedAmount>> = HashMap::new();
    for mut row in run(ledger, &CHANGES, params)? {
        let (Value::Date(bucket), Value::Str(account_type)) = (row.take("bucket"), row.take("type")) else {
            continue;
        };
        let Ok(account_type) = AccountType::from_str(&account_type) else {
            continue;
        };
        let amount = Figure::of(row.take("units"), row.take("value")).amount(&currency);
        changes.entry(bucket).or_default().insert(account_type, amount);
    }
    Ok(GraphRows {
        range: *range,
        interval: *interval,
        currency,
        opening,
        closing,
        changes,
        prices: PriceMap::cached(ledger),
        limits,
        started,
    })
}

impl GraphRows {
    /// The graph: a point for every bucket of the range. A bucket with postings is the row of
    /// `report.net_worth_trend`. A bucket without postings has the balance of the bucket before
    /// (or the balance before the range), valued at its own last day in the range with the
    /// ledger's prices, as `report.net_worth` of that day values it; it has no changes.
    ///
    /// TODO(#479): the carrying over is the one part of the report outside the engine, since
    /// BQL has no series of dates to join the balances to, so "Open query" lists only the
    /// buckets with postings.
    ///
    /// The points count against the result size limit, and the time limit stops the loop.
    pub fn build(self) -> ServerResult<StatisticGraphEntity> {
        let GraphRows {
            range,
            interval,
            currency,
            opening,
            closing,
            changes,
            prices,
            limits,
            started,
        } = self;
        let too_slow = || {
            ServerError::InvalidInput(format!(
                "the graph from {} to {} by {} was stopped because it took longer than the {}s time limit; ask for weeks or months, or a shorter range",
                range.from,
                range.to,
                unit(&interval),
                limits.timeout.unwrap_or_default().as_secs()
            ))
        };
        let too_large = || {
            ServerError::InvalidInput(format!(
                "the graph from {} to {} by {} holds more than the {} values of the result size limit (ZHANG_QUERY_MAX_RESULT_VALUES); ask for weeks or months, or a shorter range",
                range.from,
                range.to,
                unit(&interval),
                limits.max_values
            ))
        };
        let late = || limits.timeout.is_some_and(|timeout| started.elapsed() > timeout);
        let mut values: u64 = changes.values().flat_map(HashMap::values).map(amount_values).sum();
        if values > limits.max_values {
            return Err(too_large());
        }

        let mut carried = opening;
        let mut balances = HashMap::new();
        for (index, bucket) in Buckets::of(&range, &interval).enumerate() {
            // the queries may have used up the time already: check before the first bucket too
            if index % 256 == 0 && late() {
                return Err(too_slow());
            }
            let amount = match closing.get(&bucket) {
                Some((balance, figure)) => {
                    carried = balance.clone();
                    figure.amount(&currency)
                }
                None => {
                    let value = carried.convert(&currency, &prices, Some(bucket_end(bucket, &interval).min(range.to)));
                    calculated_amount(&carried, &value, &currency)
                }
            };
            values += amount_values(&amount);
            if values > limits.max_values {
                return Err(too_large());
            }
            balances.insert(bucket, amount);
        }
        Ok(StatisticGraphEntity {
            from: range.from.and_time(NaiveTime::MIN),
            to: range.to.and_time(END_OF_DAY),
            balances,
            changes,
        })
    }
}

/// The values of a point or a change of the graph: its day, its amount and its currencies.
fn amount_values(amount: &CalculatedAmount) -> u64 {
    POINT_VALUES + amount.detail.len() as u64
}

/// The bucket size, for messages.
fn unit(interval: &StatisticInterval) -> &'static str {
    match interval {
        StatisticInterval::Day => "day",
        StatisticInterval::Week => "week",
        StatisticInterval::Month => "month",
    }
}

/// `GET /api/statistic/{account_type}`: what every account of the type changed by in the
/// range, and its ten largest postings.
pub fn rank(ledger: &Ledger, account_type: AccountType, range: &LedgerDateRange) -> ServerResult<StatisticRankEntity> {
    let currency = ledger.options.operating_currency.as_str();
    let params = range.bind(Params::new().bind("type", account_type.to_string()).bind("currency", currency));

    let mut detail = vec![];
    for mut row in run(ledger, &ACCOUNT_TOTALS, params.clone())? {
        if let Value::Str(account) = row.take("account") {
            let amount = Figure::of(row.take("units"), row.take("value")).amount(currency);
            detail.push(ReportRankItemEntity { account, amount });
        }
    }
    let top_transactions = run(ledger, &TOP_POSTINGS, params)?.into_iter().filter_map(top_posting).collect();

    Ok(StatisticRankEntity {
        from: range.from.and_time(NaiveTime::MIN),
        to: range.to.and_time(END_OF_DAY),
        detail,
        top_transactions,
    })
}

/// A row of `report.top_postings` as a journal item.
fn top_posting(mut row: Row) -> Option<AccountJournalDomain> {
    let (Value::Date(date), Value::Str(time), Value::Int(timestamp), Value::Str(account), Value::Str(id), Value::Amount(units), Value::Amount(account_balance)) = (
        row.take("date"),
        row.take("time"),
        row.take("timestamp"),
        row.take("account"),
        row.take("id"),
        row.take("units"),
        row.take("account_balance"),
    ) else {
        return None;
    };
    Some(AccountJournalDomain {
        datetime: date.and_time(NaiveTime::from_str(&time).unwrap_or(NaiveTime::MIN)),
        timestamp,
        account,
        trx_id: id,
        payee: text(row.take("payee")),
        narration: text(row.take("narration")),
        inferred_unit: units,
        account_after: account_balance,
        asserted: None,
        checked_balance: None,
        passed: None,
    })
}

/// The bin width of `report.net_worth_trend` and `report.changes` for an interval.
pub fn stride(interval: &StatisticInterval) -> &'static str {
    match interval {
        StatisticInterval::Day => "1 day",
        StatisticInterval::Week => "1 week",
        StatisticInterval::Month => "1 month",
    }
}

/// The first day of the bucket of `date`: the day itself, the Monday of its week or the first
/// of its month, as `date_bin(stride, date, 2001-01-01)` bins it (2001-01-01 is a Monday).
fn bucket_start(date: NaiveDate, interval: &StatisticInterval) -> NaiveDate {
    match interval {
        StatisticInterval::Day => date,
        StatisticInterval::Week => date - Days::new(u64::from(date.weekday().num_days_from_monday())),
        StatisticInterval::Month => date.with_day(1).unwrap_or(date),
    }
}

/// The first day of the bucket after the one that starts on `start`.
fn next_bucket(start: NaiveDate, interval: &StatisticInterval) -> Option<NaiveDate> {
    match interval {
        StatisticInterval::Day => start.checked_add_days(Days::new(1)),
        StatisticInterval::Week => start.checked_add_days(Days::new(7)),
        StatisticInterval::Month => start.checked_add_months(Months::new(1)),
    }
}

/// The last day of the bucket that starts on `start`.
fn bucket_end(start: NaiveDate, interval: &StatisticInterval) -> NaiveDate {
    next_bucket(start, interval).and_then(|next| next.pred_opt()).unwrap_or(NaiveDate::MAX)
}

/// The first days of the buckets that hold a day of the range, in order.
struct Buckets {
    next: Option<NaiveDate>,
    last: NaiveDate,
    interval: StatisticInterval,
}

impl Buckets {
    fn of(range: &LedgerDateRange, interval: &StatisticInterval) -> Buckets {
        Buckets {
            next: Some(bucket_start(range.from, interval)),
            last: bucket_start(range.to, interval),
            interval: *interval,
        }
    }
}

impl Iterator for Buckets {
    type Item = NaiveDate;

    fn next(&mut self) -> Option<NaiveDate> {
        let start = self.next.filter(|start| *start <= self.last)?;
        self.next = next_bucket(start, &self.interval);
        Some(start)
    }
}

/// How many buckets hold a day of the range.
fn bucket_count(range: &LedgerDateRange, interval: &StatisticInterval) -> u64 {
    let (first, last) = (bucket_start(range.from, interval), bucket_start(range.to, interval));
    let count = match interval {
        StatisticInterval::Day => (last - first).num_days(),
        StatisticInterval::Week => (last - first).num_days() / 7,
        StatisticInterval::Month => i64::from(last.year() - first.year()) * 12 + i64::from(last.month()) - i64::from(first.month()),
    };
    u64::try_from(count + 1).unwrap_or(0)
}

/// The time of the last second of a day, the end of a range in the responses.
const END_OF_DAY: NaiveTime = match NaiveTime::from_hms_opt(23, 59, 59) {
    Some(time) => time,
    None => NaiveTime::MIN,
};

/// The first instant of a ledger date: its midnight, or when a change of the clocks skips
/// midnight, the first time of the day that exists.
pub fn first_instant(date: NaiveDate, timezone: &Tz) -> DateTime<Utc> {
    (0..24 * 60)
        .find_map(|minute| {
            timezone
                .from_local_datetime(&(date.and_time(NaiveTime::MIN) + chrono::Duration::minutes(minute)))
                .earliest()
        })
        .map(|it| it.with_timezone(&Utc))
        .unwrap_or_else(|| Utc.from_utc_datetime(&date.and_time(NaiveTime::MIN)))
}

/// The last second of a ledger date, 23:59:59, or when a change of the clocks skips it, the
/// last second of the day that exists.
pub fn last_instant(date: NaiveDate, timezone: &Tz) -> DateTime<Utc> {
    (0..24 * 60 * 60)
        .step_by(60)
        .find_map(|seconds| {
            timezone
                .from_local_datetime(&(date.and_time(END_OF_DAY) - chrono::Duration::seconds(seconds)))
                .latest()
        })
        .map(|it| it.with_timezone(&Utc))
        .unwrap_or_else(|| Utc.from_utc_datetime(&date.and_time(END_OF_DAY)))
}

/// The rows of one of the report's queries, their cells taken by column name.
fn run(ledger: &Ledger, query: &BuiltinQuery, params: Params) -> ServerResult<Vec<Row>> {
    let result = execute(ledger, query.name, &params, false)?;
    let columns: HashMap<String, usize> = result.columns.iter().enumerate().map(|(index, column)| (column.name.clone(), index)).collect();
    Ok(result
        .rows
        .into_iter()
        .map(|cells| Row {
            columns: columns.clone(),
            cells,
        })
        .collect())
}

/// A row of a query result.
struct Row {
    columns: HashMap<String, usize>,
    cells: Vec<Value>,
}

impl Row {
    /// The cell of the column `name`; NULL if the query has no such column.
    fn take(&mut self, name: &str) -> Value {
        match self.columns.get(name).and_then(|index| self.cells.get_mut(*index)) {
            Some(cell) => std::mem::replace(cell, Value::Null),
            None => Value::Null,
        }
    }
}

/// The units and the value of a figure, as the queries return them.
#[derive(Debug, Clone, Default)]
struct Figure {
    units: Inventory,
    value: Inventory,
}

impl Figure {
    fn of(units: Value, value: Value) -> Figure {
        Figure {
            units: inventory(units),
            value: inventory(value),
        }
    }

    fn amount(&self, currency: &str) -> CalculatedAmount {
        calculated_amount(&self.units, &self.value, currency)
    }
}

/// The figure (`units`, `value`) of a query without groups; nothing when it matched no postings.
fn single(rows: Vec<Row>) -> Figure {
    rows.into_iter()
        .next()
        .map(|mut row| Figure::of(row.take("units"), row.take("value")))
        .unwrap_or_default()
}

/// The figures of a query grouped by account type (`type`, `units`, `value`), by type.
fn by_type(rows: Vec<Row>) -> HashMap<String, Figure> {
    rows.into_iter()
        .filter_map(|mut row| match row.take("type") {
            Value::Str(account_type) => Some((account_type, Figure::of(row.take("units"), row.take("value")))),
            _ => None,
        })
        .collect()
}

/// An inventory, amount or NULL cell as an inventory.
fn inventory(value: Value) -> Inventory {
    match value {
        Value::Inventory(inventory) => inventory,
        Value::Amount(amount) => {
            let mut inventory = Inventory::new();
            inventory.add_amount(&amount);
            inventory
        }
        _ => Inventory::new(),
    }
}

/// A text cell; NULL is none.
fn text(value: Value) -> Option<String> {
    match value {
        Value::Str(text) => Some(text),
        _ => None,
    }
}

#[cfg(test)]
mod test {
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::Duration;

    use chrono::NaiveDate;
    use chrono_tz::Tz;
    use zhang_core::clock::Clock;
    use zhang_core::data_source::LocalFileSystemDataSource;
    use zhang_core::data_type::text::ZhangDataType;
    use zhang_core::data_type::DataType;
    use zhang_core::ledger::{Ledger, LedgerProcessContext};

    use super::{first_instant, graph_rows, last_instant, GraphLimits};
    use crate::builtin::LedgerDateRange;
    use crate::request::StatisticInterval;

    fn ledger(content: &str) -> Ledger {
        let directives = ZhangDataType {}.transform(content.to_owned(), None).unwrap();
        Ledger::process(LedgerProcessContext {
            directives,
            entry: (PathBuf::from("."), "main.zhang".to_owned()),
            visited_files: vec![],
            data_source: Arc::new(LocalFileSystemDataSource::new(ZhangDataType {})),
            clock: Clock::System,
        })
        .unwrap()
    }

    fn date(text: &str) -> NaiveDate {
        NaiveDate::parse_from_str(text, "%Y-%m-%d").unwrap()
    }

    fn range(from: &str, to: &str) -> LedgerDateRange {
        LedgerDateRange {
            from: date(from),
            to: date(to),
        }
    }

    const LEDGER: &str = r#"
option "operating_currency" "CNY"
1970-01-01 commodity CNY
1970-01-01 commodity USD
1970-01-01 commodity JPY
1970-01-01 open Assets:Bank
1970-01-01 open Equity:Opening
2024-01-01 price USD 7 CNY
2024-01-01 "Opening" "three currencies"
  Assets:Bank 100 CNY
  Assets:Bank 10 USD
  Assets:Bank 1000 JPY
  Equity:Opening -100 CNY
  Equity:Opening -10 USD
  Equity:Opening -1000 JPY
"#;

    #[test]
    fn a_graph_with_more_points_than_the_limit_is_refused_before_its_queries() {
        let ledger = ledger(LEDGER);
        let limits = GraphLimits { max_values: 20, timeout: None };
        // 11 days are 22 values at least, over the limit of 20; 10 days are 20, within it
        let error = graph_rows(&ledger, &range("2024-01-01", "2024-01-11"), &StatisticInterval::Day, limits)
            .err()
            .unwrap();
        assert_eq!(
            error.to_string(),
            "the graph from 2024-01-01 to 2024-01-11 by day would have 11 points, and the server returns at most 10 (half of the result size limit, ZHANG_QUERY_MAX_RESULT_VALUES); ask for weeks or months, or a shorter range"
        );
        assert!(matches!(error, crate::error::ServerError::InvalidInput(_)));
        assert!(graph_rows(&ledger, &range("2024-01-01", "2024-01-10"), &StatisticInterval::Day, limits).is_ok());
        // the same range by week or month fits
        assert!(graph_rows(&ledger, &range("2024-01-01", "2024-01-11"), &StatisticInterval::Week, limits).is_ok());
        // a whole calendar by day, as the reviewer asked for, is refused with the server's limits
        let error = graph_rows(&ledger, &range("0001-01-01", "9999-12-31"), &StatisticInterval::Day, GraphLimits::server())
            .err()
            .unwrap();
        assert!(error.to_string().contains("would have 3652059 points"), "{}", error);
    }

    #[test]
    fn the_points_count_against_the_limit_with_their_currencies() {
        let ledger = ledger(LEDGER);
        // every point holds three currencies, 5 values, and so do the two changes of January 1
        // (Assets and Equity): 4 x 5 + 2 x 5 = 30 values; the carried points count too
        let limits = |max_values| GraphLimits { max_values, timeout: None };
        let rows = graph_rows(&ledger, &range("2024-01-01", "2024-01-04"), &StatisticInterval::Day, limits(30)).unwrap();
        assert_eq!(rows.build().unwrap().balances.len(), 4);
        let rows = graph_rows(&ledger, &range("2024-01-01", "2024-01-04"), &StatisticInterval::Day, limits(29)).unwrap();
        assert_eq!(
            rows.build().err().unwrap().to_string(),
            "the graph from 2024-01-01 to 2024-01-04 by day holds more than the 29 values of the result size limit (ZHANG_QUERY_MAX_RESULT_VALUES); ask for weeks or months, or a shorter range"
        );
    }

    #[test]
    fn building_the_graph_stops_at_the_time_limit() {
        let ledger = ledger(LEDGER);
        let limits = GraphLimits {
            max_values: u64::MAX,
            timeout: Some(Duration::ZERO),
        };
        let rows = graph_rows(&ledger, &range("2024-01-01", "2026-12-31"), &StatisticInterval::Day, limits).unwrap();
        assert_eq!(
            rows.build().err().unwrap().to_string(),
            "the graph from 2024-01-01 to 2026-12-31 by day was stopped because it took longer than the 0s time limit; ask for weeks or months, or a shorter range"
        );
        let limits = GraphLimits {
            max_values: u64::MAX,
            timeout: Some(Duration::from_secs(60)),
        };
        let rows = graph_rows(&ledger, &range("2024-01-01", "2026-12-31"), &StatisticInterval::Day, limits).unwrap();
        assert_eq!(rows.build().unwrap().balances.len(), 1096);
    }

    #[test]
    fn the_ends_of_a_day_exist_in_the_ledger_timezone() {
        let at = |instant: chrono::DateTime<chrono::Utc>| instant.to_rfc3339();
        let shanghai: Tz = "Asia/Shanghai".parse().unwrap();
        assert_eq!(at(first_instant(date("2025-04-01"), &shanghai)), "2025-03-31T16:00:00+00:00");
        assert_eq!(at(last_instant(date("2025-04-30"), &shanghai)), "2025-04-30T15:59:59+00:00");
        // in Santiago the clocks jump from 00:00 to 01:00 on 2024-09-08: the day starts at 01:00 (-03)
        let santiago: Tz = "America/Santiago".parse().unwrap();
        assert_eq!(at(first_instant(date("2024-09-08"), &santiago)), "2024-09-08T04:00:00+00:00");
        assert_eq!(at(last_instant(date("2024-09-08"), &santiago)), "2024-09-09T02:59:59+00:00");
        // on 2024-04-07 they go back from 00:00 to 23:00 the day before: the day starts after it
        assert_eq!(at(first_instant(date("2024-04-07"), &santiago)), "2024-04-07T04:00:00+00:00");
        assert_eq!(at(last_instant(date("2024-04-06"), &santiago)), "2024-04-07T03:59:59+00:00");
        // in New York the clocks skip 02:00 to 03:00 on 2025-03-09, so midnight exists
        let new_york: Tz = "America/New_York".parse().unwrap();
        assert_eq!(at(first_instant(date("2025-03-09"), &new_york)), "2025-03-09T05:00:00+00:00");
    }
}
