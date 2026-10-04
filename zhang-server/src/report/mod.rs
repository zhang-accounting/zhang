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
use zhang_query::{DataType, ExecuteOptions, Inventory, Params, PriceMap, QueryErrorKind, QueryResult, Value};

use crate::builtin::{calculated_amount, compiled, execute, BuiltinQuery, LedgerDateRange};
use crate::cells::Columns;
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
    description: "The net worth at the end of every day, week or month (:interval) from :from to :to that has postings, valued in :currency at the prices of its last day in the range, after the opening balance on the day before :from.",
    bql: "SELECT date_bin(:interval, date, 2001-01-01) AS bucket, last(balance) AS balance, units(last(balance)) AS units,
  convert(last(balance), :currency, least(max(date_bin(:interval, date, 2001-01-01)) + interval(:interval) - 1, :to)) AS value
FROM OPEN ON :from
WHERE (under(account, 'Assets') OR under(account, 'Liabilities')) AND date <= :to
GROUP BY bucket
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
    check_calendar(range)?;
    let currency = ledger.options.operating_currency.as_str();
    let at_end = Params::new().bind("to", range.to).bind("currency", currency);
    let net_worth = single(run(ledger, &NET_WORTH, at_end.clone())?)?;
    let liabilities = single(run(ledger, &LIABILITIES, at_end)?)?;
    let flows = by_type(run(ledger, &FLOWS, range.bind(Params::new().bind("currency", currency)))?)?;
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
        transaction_number: match count.into_iter().next() {
            Some(mut row) => row.take("transactions")?.as_int().unwrap_or(0),
            None => 0,
        },
    })
}

/// The most points a graph has: about 137 years of days. A longer range by day is a 400 that
/// suggests weeks or months; no chart draws that many points.
pub const MAX_GRAPH_POINTS: u64 = 50_000;

/// The limits of building a graph, by default those of every query the server runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GraphLimits {
    /// at most this many points, checked before any query runs
    pub max_points: u64,
    /// at most this many values in the graph's points and changes: two per point or change
    /// (its day and its amount) and one per currency of its units
    pub max_values: u64,
    /// stop once building the graph, its queries included, has run this long
    pub timeout: Option<Duration>,
}

impl GraphLimits {
    /// [`MAX_GRAPH_POINTS`], and the limits of `POST /api/query`: its result size limit and its
    /// time limit.
    pub fn server() -> GraphLimits {
        let options = execute_options(max_result_values());
        GraphLimits {
            max_points: MAX_GRAPH_POINTS,
            max_values: options.max_result_values.unwrap_or(u64::MAX),
            timeout: options.timeout,
        }
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
    /// the closing balance (with its lots) and figure of every bucket of `report.net_worth_trend`:
    /// the buckets with postings in the range, and the bucket of the opening balance
    closing: BTreeMap<NaiveDate, (Inventory, Figure)>,
    changes: HashMap<NaiveDate, HashMap<AccountType, CalculatedAmount>>,
    prices: Arc<PriceMap>,
    limits: GraphLimits,
    started: Instant,
}

/// Run the graph's queries, `report.net_worth_trend` and `report.changes`, whose cost grows with
/// the range, not with the ledger's history. A range with more points than the limits allow,
/// or outside the years 1 to 9999, is a 400 before any query runs; a range whose queries go
/// over the result size limit or the time limit is a 400 in the graph's terms.
pub fn graph_rows(ledger: &Ledger, range: &LedgerDateRange, interval: &StatisticInterval, limits: GraphLimits) -> ServerResult<GraphRows> {
    graph_rows_since(ledger, range, interval, limits, Instant::now())
}

/// [`graph_rows`] for a graph that `started` building then: its queries share what is left of
/// its time limit.
fn graph_rows_since(ledger: &Ledger, range: &LedgerDateRange, interval: &StatisticInterval, limits: GraphLimits, started: Instant) -> ServerResult<GraphRows> {
    check_calendar(range)?;
    let points = bucket_count(range, interval);
    if points > limits.max_points {
        return Err(ServerError::InvalidInput(format!(
            "the graph from {} to {} by {} would have {} points, more than the {} a graph may have; ask for weeks or months, or a shorter range",
            range.from,
            range.to,
            unit(interval),
            points,
            limits.max_points
        )));
    }
    let currency = ledger.options.operating_currency.clone();
    let params = range.bind(Params::new().bind("interval", stride(interval)).bind("currency", currency.as_str()));
    let graph_error = |error: ServerError| match error {
        ServerError::QueryError(error) if error.kind == QueryErrorKind::TooLarge => too_large(range, interval, limits.max_values),
        ServerError::QueryError(error) if error.kind == QueryErrorKind::Timeout => too_slow(range, interval, limits.timeout),
        other => other,
    };

    let mut closing = BTreeMap::new();
    for mut row in run_within(ledger, &NET_WORTH_TREND, params.clone(), &limits, started).map_err(graph_error)? {
        if let Value::Date(bucket) = row.take("bucket")? {
            closing.insert(bucket, (inventory(row.take("balance")?), Figure::of(row.take("units")?, row.take("value")?)));
        }
    }
    let mut changes: HashMap<NaiveDate, HashMap<AccountType, CalculatedAmount>> = HashMap::new();
    for mut row in run_within(ledger, &CHANGES, params, &limits, started).map_err(graph_error)? {
        let (Value::Date(bucket), Value::Str(account_type)) = (row.take("bucket")?, row.take("type")?) else {
            continue;
        };
        let Ok(account_type) = AccountType::from_str(&account_type) else {
            continue;
        };
        let amount = Figure::of(row.take("units")?, row.take("value")?).amount(&currency);
        changes.entry(bucket).or_default().insert(account_type, amount);
    }
    Ok(GraphRows {
        range: *range,
        interval: *interval,
        currency,
        closing,
        changes,
        prices: PriceMap::cached(ledger),
        limits,
        started,
    })
}

/// The 400 of a graph whose points and currencies hold more values than the result size limit.
fn too_large(range: &LedgerDateRange, interval: &StatisticInterval, max_values: u64) -> ServerError {
    ServerError::InvalidInput(format!(
        "the graph from {} to {} by {} has too many points or currencies: it would hold more than the {} values of the result size limit (ZHANG_QUERY_MAX_RESULT_VALUES); ask for weeks or months, or a shorter range",
        range.from,
        range.to,
        unit(interval),
        max_values
    ))
}

/// The 400 of a graph that takes longer than the time limit.
fn too_slow(range: &LedgerDateRange, interval: &StatisticInterval, timeout: Option<Duration>) -> ServerError {
    ServerError::InvalidInput(format!(
        "the graph from {} to {} by {} was stopped because it took longer than the {}s time limit; ask for weeks or months, or a shorter range",
        range.from,
        range.to,
        unit(interval),
        timeout.unwrap_or_default().as_secs()
    ))
}

/// A report covers the years 1 to 9999, the calendar of the ledger; another range is a 400.
fn check_calendar(range: &LedgerDateRange) -> ServerResult<()> {
    let calendar = 1..=9999;
    if calendar.contains(&range.from.year()) && calendar.contains(&range.to.year()) {
        Ok(())
    } else {
        Err(ServerError::InvalidInput(format!(
            "a report covers the years 1 to 9999, not {} to {}",
            range.from, range.to
        )))
    }
}

impl GraphRows {
    /// The graph: a point for every bucket of the range. A bucket with postings is the row of
    /// `report.net_worth_trend`. A bucket without postings has the balance of the bucket before
    /// (or the opening balance of the range), valued at its own last day in the range with the
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
            closing,
            changes,
            prices,
            limits,
            started,
        } = self;
        let too_slow = || too_slow(&range, &interval, limits.timeout);
        let too_large = || too_large(&range, &interval, limits.max_values);
        let late = || limits.timeout.is_some_and(|timeout| started.elapsed() > timeout);
        let mut values: u64 = changes.values().flat_map(HashMap::values).map(amount_values).sum();
        if values > limits.max_values {
            return Err(too_large());
        }

        // the balance before the range: the bucket of the day before it, when that is an earlier
        // bucket (otherwise the first bucket starts with it)
        let first = bucket_start(range.from, &interval);
        let mut carried = closing.range(..first).next_back().map(|(_, (balance, _))| balance.clone()).unwrap_or_default();
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
    check_calendar(range)?;
    let currency = ledger.options.operating_currency.as_str();
    let params = range.bind(Params::new().bind("type", account_type.to_string()).bind("currency", currency));

    let mut detail = vec![];
    for mut row in run(ledger, &ACCOUNT_TOTALS, params.clone())? {
        if let Value::Str(account) = row.take("account")? {
            let amount = Figure::of(row.take("units")?, row.take("value")?).amount(currency);
            detail.push(ReportRankItemEntity { account, amount });
        }
    }
    let mut top_transactions = vec![];
    for row in run(ledger, &TOP_POSTINGS, params)? {
        top_transactions.extend(top_posting(row)?);
    }

    Ok(StatisticRankEntity {
        from: range.from.and_time(NaiveTime::MIN),
        to: range.to.and_time(END_OF_DAY),
        detail,
        top_transactions,
    })
}

/// A row of `report.top_postings` as a journal item.
fn top_posting(mut row: Row) -> ServerResult<Option<AccountJournalDomain>> {
    let (Value::Date(date), Value::Str(time), Value::Int(timestamp), Value::Str(account), Value::Str(id), Value::Amount(units), Value::Amount(account_balance)) = (
        row.take("date")?,
        row.take("time")?,
        row.take("timestamp")?,
        row.take("account")?,
        row.take("id")?,
        row.take("units")?,
        row.take("account_balance")?,
    ) else {
        return Ok(None);
    };
    Ok(Some(AccountJournalDomain {
        datetime: date.and_time(NaiveTime::from_str(&time).unwrap_or(NaiveTime::MIN)),
        timestamp,
        account,
        trx_id: id,
        payee: text(row.take("payee")?),
        narration: text(row.take("narration")?),
        inferred_unit: units,
        account_after: account_balance,
        asserted: None,
        checked_balance: None,
        passed: None,
    }))
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
        StatisticInterval::Week => date
            .checked_sub_days(Days::new(u64::from(date.weekday().num_days_from_monday())))
            .unwrap_or(date),
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
    Ok(rows(query.name, execute(ledger, query.name, &params, false)?))
}

/// [`run`] within the limits of a graph: its result size limit, and what is left of its time
/// limit since it `started`.
fn run_within(ledger: &Ledger, query: &BuiltinQuery, params: Params, limits: &GraphLimits, started: Instant) -> ServerResult<Vec<Row>> {
    let options = ExecuteOptions {
        today: None,
        timeout: limits.timeout.map(|timeout| timeout.saturating_sub(started.elapsed())),
        max_result_values: Some(limits.max_values),
        count_total: false,
    };
    Ok(rows(query.name, compiled(query.name)?.execute_with_options(ledger, &params, &options)?))
}

/// The rows of the result of the built-in query `query`, their cells taken by column name.
fn rows(query: &str, result: QueryResult) -> Vec<Row> {
    let columns = Columns::of(query, &result);
    result
        .rows
        .into_iter()
        .map(|cells| Row {
            columns: columns.clone(),
            cells,
        })
        .collect()
}

/// A row of a query result.
struct Row {
    columns: Columns,
    cells: Vec<Value>,
}

impl Row {
    /// The cell of the column `name`.
    fn take(&mut self, name: &str) -> ServerResult<Value> {
        self.columns.take(&mut self.cells, name)
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
fn single(rows: Vec<Row>) -> ServerResult<Figure> {
    match rows.into_iter().next() {
        Some(mut row) => Ok(Figure::of(row.take("units")?, row.take("value")?)),
        None => Ok(Figure::default()),
    }
}

/// The figures of a query grouped by account type (`type`, `units`, `value`), by type.
fn by_type(rows: Vec<Row>) -> ServerResult<HashMap<String, Figure>> {
    let mut figures = HashMap::new();
    for mut row in rows {
        if let Value::Str(account_type) = row.take("type")? {
            figures.insert(account_type, Figure::of(row.take("units")?, row.take("value")?));
        }
    }
    Ok(figures)
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

    /// No limits but those given.
    fn limits(max_points: u64, max_values: u64, timeout: Option<Duration>) -> GraphLimits {
        GraphLimits {
            max_points,
            max_values,
            timeout,
        }
    }

    #[test]
    fn a_graph_with_more_points_than_the_limit_is_refused_before_its_queries() {
        let ledger = ledger(LEDGER);
        let ten = limits(10, u64::MAX, None);
        let error = graph_rows(&ledger, &range("2024-01-01", "2024-01-11"), &StatisticInterval::Day, ten)
            .err()
            .unwrap();
        assert_eq!(
            error.to_string(),
            "the graph from 2024-01-01 to 2024-01-11 by day would have 11 points, more than the 10 a graph may have; ask for weeks or months, or a shorter range"
        );
        assert!(matches!(error, crate::error::ServerError::InvalidInput(_)));
        assert!(graph_rows(&ledger, &range("2024-01-01", "2024-01-10"), &StatisticInterval::Day, ten).is_ok());
        // the same range by week or month fits
        assert!(graph_rows(&ledger, &range("2024-01-01", "2024-01-11"), &StatisticInterval::Week, ten).is_ok());
        // the server's cap is 50,000 points, about 137 years of days
        assert_eq!(GraphLimits::server().max_points, 50_000);
        let error = graph_rows(&ledger, &range("1888-01-01", "2024-12-31"), &StatisticInterval::Day, GraphLimits::server())
            .err()
            .unwrap();
        assert!(error.to_string().contains("would have 50039 points, more than the 50000"), "{}", error);
        assert!(graph_rows(&ledger, &range("1900-01-01", "2024-12-31"), &StatisticInterval::Day, GraphLimits::server()).is_ok());
        let error = graph_rows(&ledger, &range("0001-01-01", "9999-12-31"), &StatisticInterval::Day, GraphLimits::server())
            .err()
            .unwrap();
        assert!(error.to_string().contains("would have 3652059 points, more than the 50000"), "{}", error);
    }

    #[test]
    fn a_range_outside_the_calendar_is_refused() {
        let ledger = ledger(LEDGER);
        // chrono's first date: its week would start before it
        let error = graph_rows(
            &ledger,
            &range("-262143-01-01", "-262143-01-10"),
            &StatisticInterval::Week,
            GraphLimits::server(),
        )
        .err()
        .unwrap();
        assert_eq!(error.to_string(), "a report covers the years 1 to 9999, not -262143-01-01 to -262143-01-10");
        assert!(matches!(error, crate::error::ServerError::InvalidInput(_)));
        let error = super::summary(&ledger, &range("2024-01-01", "+10000-01-01")).err().unwrap();
        assert_eq!(error.to_string(), "a report covers the years 1 to 9999, not 2024-01-01 to +10000-01-01");
        assert!(super::rank(&ledger, zhang_ast::AccountType::Expenses, &range("0000-12-31", "2024-01-01")).is_err());
        // the first and the last day of the calendar are fine
        assert!(graph_rows(&ledger, &range("0001-01-01", "0001-01-10"), &StatisticInterval::Week, GraphLimits::server()).is_ok());
        assert!(graph_rows(&ledger, &range("9999-12-20", "9999-12-31"), &StatisticInterval::Month, GraphLimits::server()).is_ok());
    }

    #[test]
    fn the_points_count_against_the_limit_with_their_currencies() {
        let ledger = ledger(LEDGER);
        // every point holds three currencies, 5 values, and so do the two changes of January 1
        // (Assets and Equity): 4 x 5 + 2 x 5 = 30 values; the carried points count too
        let rows = graph_rows(&ledger, &range("2024-01-01", "2024-01-04"), &StatisticInterval::Day, limits(u64::MAX, 30, None)).unwrap();
        assert_eq!(rows.build().unwrap().balances.len(), 4);
        let rows = graph_rows(&ledger, &range("2024-01-01", "2024-01-04"), &StatisticInterval::Day, limits(u64::MAX, 29, None)).unwrap();
        assert_eq!(
            rows.build().err().unwrap().to_string(),
            "the graph from 2024-01-01 to 2024-01-04 by day has too many points or currencies: it would hold more than the 29 values of the result size limit (ZHANG_QUERY_MAX_RESULT_VALUES); ask for weeks or months, or a shorter range"
        );
    }

    #[test]
    fn building_the_graph_stops_at_the_time_limit() {
        let ledger = ledger(LEDGER);
        let expired = limits(u64::MAX, u64::MAX, Some(Duration::ZERO));
        let message = "the graph from 2024-01-01 to 2026-12-31 by day was stopped because it took longer than the 0s time limit; ask for weeks or months, or a shorter range";
        // the queries stop at the limit
        let error = graph_rows(&ledger, &range("2024-01-01", "2026-12-31"), &StatisticInterval::Day, expired)
            .err()
            .unwrap();
        assert_eq!(error.to_string(), message);
        // and so does the loop over the buckets, once the queries are done
        let mut rows = graph_rows(&ledger, &range("2024-01-01", "2026-12-31"), &StatisticInterval::Day, GraphLimits::server()).unwrap();
        rows.limits = expired;
        assert_eq!(rows.build().err().unwrap().to_string(), message);
        // the queries share the time limit: once the graph has used it up, the next query stops
        let long_ago = std::time::Instant::now() - Duration::from_secs(2);
        let error = super::graph_rows_since(
            &ledger,
            &range("2024-01-01", "2024-01-03"),
            &StatisticInterval::Day,
            limits(u64::MAX, u64::MAX, Some(Duration::from_secs(1))),
            long_ago,
        )
        .err()
        .unwrap();
        assert_eq!(
            error.to_string(),
            "the graph from 2024-01-01 to 2024-01-03 by day was stopped because it took longer than the 1s time limit; ask for weeks or months, or a shorter range"
        );
        // checked at the first bucket, so a graph of a few buckets stops too
        let mut rows = graph_rows(&ledger, &range("2024-01-01", "2024-01-03"), &StatisticInterval::Day, GraphLimits::server()).unwrap();
        rows.limits = expired;
        assert!(rows.build().is_err());
        let rows = graph_rows(
            &ledger,
            &range("2024-01-01", "2026-12-31"),
            &StatisticInterval::Day,
            limits(u64::MAX, u64::MAX, Some(Duration::from_secs(60))),
        )
        .unwrap();
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
