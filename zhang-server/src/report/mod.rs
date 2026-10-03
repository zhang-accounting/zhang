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

use chrono::{DateTime, Datelike, Days, Months, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc};
use chrono_tz::Tz;
use zhang_ast::amount::CalculatedAmount;
use zhang_ast::AccountType;
use zhang_core::domains::schemas::AccountJournalDomain;
use zhang_core::ledger::Ledger;
use zhang_query::{DataType, Inventory, Params, PriceMap, Value};

use crate::builtin::{calculated_amount, execute, BuiltinQuery, LedgerDateRange};
use crate::request::StatisticInterval;
use crate::response::{ReportRankItemEntity, StatisticGraphEntity, StatisticRankEntity, StatisticSummaryEntity};
use crate::ServerResult;

/// The balances of the asset and of the liability accounts at the end of a day. The summary's
/// net worth is their sum, and the graph starts from them on the day before its range.
pub const BALANCES: BuiltinQuery = BuiltinQuery {
    name: "report.balances",
    description: "The balances of the assets and of the liabilities at the end of :to, valued in :currency at the prices of that day.",
    bql: "SELECT root(account, 1) AS type, sum(position) AS balance, units(sum(position)) AS units, convert(sum(position), :currency, :to) AS value
WHERE (under(account, 'Assets') OR under(account, 'Liabilities')) AND date <= :to
GROUP BY type
ORDER BY type",
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
pub const NET_WORTH: BuiltinQuery = BuiltinQuery {
    name: "report.net_worth",
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
    let balances = by_type(run(ledger, &BALANCES, Params::new().bind("to", range.to).bind("currency", currency))?);
    let flows = by_type(run(ledger, &FLOWS, range.bind(Params::new().bind("currency", currency)))?);
    let count = run(ledger, &TRANSACTION_COUNT, range.bind(Params::new()))?;

    let figure = |figures: &HashMap<String, Figure>, account_type: AccountType| figures.get(&account_type.to_string()).cloned().unwrap_or_default();
    let liabilities = figure(&balances, AccountType::Liabilities);
    let net_worth = figure(&balances, AccountType::Assets).plus(&liabilities);
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

/// `GET /api/statistic/graph`: the net worth at the end of every day, week or month of the
/// range, and what each account type changed by in it, keyed by the bucket's first day.
///
/// A bucket without postings has the balance of the one before (or the balance before the
/// range), valued at its own last day with the engine's conversion, as `report.net_worth`
/// would value it; it has no changes.
pub fn graph(ledger: &Ledger, range: &LedgerDateRange, interval: &StatisticInterval) -> ServerResult<StatisticGraphEntity> {
    let currency = ledger.options.operating_currency.as_str();
    let params = range.bind(Params::new().bind("interval", stride(interval)).bind("currency", currency));

    // the closing balance of every bucket with postings in the range
    let mut closing: BTreeMap<NaiveDate, (Inventory, Figure)> = BTreeMap::new();
    for mut row in run(ledger, &NET_WORTH, params.clone())? {
        if let Value::Date(bucket) = row.take("bucket") {
            closing.insert(bucket, (inventory(row.take("balance")), Figure::of(row.take("units"), row.take("value"))));
        }
    }

    // the balance before the range, carried into its first buckets until they have postings
    let mut carried = Inventory::new();
    if let Some(day_before) = range.from.pred_opt() {
        for mut row in run(ledger, &BALANCES, Params::new().bind("to", day_before).bind("currency", currency))? {
            carried.add_inventory(&inventory(row.take("balance")));
        }
    }
    let mut prices: Option<PriceMap> = None;
    let buckets = buckets(range, interval);
    let mut balances = HashMap::with_capacity(buckets.len());
    for bucket in buckets {
        let amount = match closing.get(&bucket) {
            Some((balance, figure)) => {
                carried = balance.clone();
                figure.amount(currency)
            }
            None if carried.is_empty() => Figure::default().amount(currency),
            None => {
                let prices = prices.get_or_insert_with(|| PriceMap::for_ledger(ledger));
                let value = carried.convert(currency, prices, Some(bucket_end(bucket, interval).min(range.to)));
                calculated_amount(&carried, &value, currency)
            }
        };
        balances.insert(bucket, amount);
    }

    let mut changes: HashMap<NaiveDate, HashMap<AccountType, CalculatedAmount>> = HashMap::new();
    for mut row in run(ledger, &CHANGES, params)? {
        let (Value::Date(bucket), Value::Str(account_type)) = (row.take("bucket"), row.take("type")) else {
            continue;
        };
        let Ok(account_type) = AccountType::from_str(&account_type) else {
            continue;
        };
        let amount = Figure::of(row.take("units"), row.take("value")).amount(currency);
        changes.entry(bucket).or_default().insert(account_type, amount);
    }

    Ok(StatisticGraphEntity {
        from: range.from.and_time(NaiveTime::MIN),
        to: range.to.and_time(END_OF_DAY),
        balances,
        changes,
    })
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

/// The bin width of `report.net_worth` and `report.changes` for an interval.
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
fn buckets(range: &LedgerDateRange, interval: &StatisticInterval) -> Vec<NaiveDate> {
    let last = bucket_start(range.to, interval);
    let mut buckets = vec![];
    let mut bucket = Some(bucket_start(range.from, interval));
    while let Some(start) = bucket.filter(|start| *start <= last) {
        buckets.push(start);
        bucket = next_bucket(start, interval);
    }
    buckets
}

/// The time of the last second of a day, the end of a range in the responses.
const END_OF_DAY: NaiveTime = match NaiveTime::from_hms_opt(23, 59, 59) {
    Some(time) => time,
    None => NaiveTime::MIN,
};

/// The first instant of a ledger date.
fn first_instant(date: NaiveDate, timezone: &Tz) -> DateTime<Utc> {
    instant(date.and_time(NaiveTime::MIN), timezone, true)
}

/// The last second of a ledger date.
fn last_instant(date: NaiveDate, timezone: &Tz) -> DateTime<Utc> {
    instant(date.and_time(END_OF_DAY), timezone, false)
}

fn instant(local: NaiveDateTime, timezone: &Tz, earliest: bool) -> DateTime<Utc> {
    let mapped = timezone.from_local_datetime(&local);
    let found = if earliest { mapped.earliest() } else { mapped.latest() };
    // a time skipped by a DST change is read as UTC rather than failing
    found.map(|it| it.with_timezone(&Utc)).unwrap_or_else(|| Utc.from_utc_datetime(&local))
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

    fn plus(&self, other: &Figure) -> Figure {
        let mut sum = self.clone();
        sum.units.add_inventory(&other.units);
        sum.value.add_inventory(&other.value);
        sum
    }

    fn amount(&self, currency: &str) -> CalculatedAmount {
        calculated_amount(&self.units, &self.value, currency)
    }
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
