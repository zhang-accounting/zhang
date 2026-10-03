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
//! graph at the prices of its own last day.

pub mod legacy;
mod shim;

use std::collections::{BTreeMap, HashMap};
use std::str::FromStr;

use chrono::{DateTime, Datelike, Days, Months, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc};
use chrono_tz::Tz;
use zhang_ast::amount::CalculatedAmount;
use zhang_ast::AccountType;
use zhang_core::domains::schemas::AccountJournalDomain;
use zhang_core::ledger::Ledger;
use zhang_query::{DataType, Inventory, Params, PriceMap, QueryResult, Value};

pub use self::shim::{calculated_amount, run, BuiltinQuery, LedgerDateRange};
use crate::request::StatisticInterval;
use crate::response::{ReportRankItemEntity, StatisticGraphEntity, StatisticRankEntity, StatisticSummaryEntity};
use crate::ServerResult;

/// The balances of the asset and liability accounts at the end of the range: the report's
/// net worth is their sum, and its liabilities the second row.
pub static BALANCES: BuiltinQuery = BuiltinQuery {
    name: "report.balances",
    description: "The balance of the assets and of the liabilities at the end of `to`, with their value in `currency` at the prices of that day. The report's net worth is their sum.",
    bql: "SELECT root(account, 1) AS type, units(sum(position)) AS units, convert(sum(position), :currency, :to) AS value
WHERE (under(account, 'Assets') OR under(account, 'Liabilities')) AND date <= :to
GROUP BY type
ORDER BY type",
    params: &[("to", DataType::Date), ("currency", DataType::Str)],
};

/// The income and the expenses of the range.
pub static FLOWS: BuiltinQuery = BuiltinQuery {
    name: "report.flows",
    description: "The income and the expenses from `from` to `to`, with their value in `currency` at the prices of `to`. Income is negative, as in the ledger.",
    bql: "SELECT root(account, 1) AS type, units(sum(position)) AS units, convert(sum(position), :currency, :to) AS value
WHERE (under(account, 'Income') OR under(account, 'Expenses')) AND date >= :from AND date <= :to
GROUP BY type
ORDER BY type",
    params: &[("from", DataType::Date), ("to", DataType::Date), ("currency", DataType::Str)],
};

/// The number of transactions of the range; padding transactions are not counted.
pub static TRANSACTION_COUNT: BuiltinQuery = BuiltinQuery {
    name: "report.transaction_count",
    description: "The number of transactions from `from` to `to`. The padding transactions of `balance ... with pad` (flag `P`) are not counted, and balance assertions are not transactions.",
    bql: "SELECT count(*) AS transactions
FROM #transactions
WHERE flag != 'P' AND date >= :from AND date <= :to",
    params: &[("from", DataType::Date), ("to", DataType::Date)],
};

/// The net worth at the end of every bucket of the graph that has postings.
pub static NET_WORTH: BuiltinQuery = BuiltinQuery {
    name: "report.net_worth",
    description: "The net worth (assets and liabilities) at the end of every day, week or month from `from` to `to` that has postings, with its value in `currency` at the prices of the bucket's last day (of `to` for the last bucket). `interval` is '1 day', '1 week' or '1 month': the bins from 2001-01-01, a Monday and the first of a month, are calendar days, weeks starting on Monday and months, each named by its first day. `OPEN ON :from` starts from the balances of the day before `from`, which are the row of the bucket of that day. The report carries a bucket without postings over from the previous one, valued at its own last day.",
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

/// What every bucket of the graph changed, per account type.
pub static CHANGES: BuiltinQuery = BuiltinQuery {
    name: "report.changes",
    description: "What each account type changed by in every day, week or month from `from` to `to`, with its value in `currency` at the prices of the bucket's last day (of `to` for the last bucket). The buckets are those of report.net_worth; the first one only counts the postings from `from` on.",
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
pub static ACCOUNT_TOTALS: BuiltinQuery = BuiltinQuery {
    name: "report.account_totals",
    description: "What every account of the type `type` (such as 'Expenses') changed by from `from` to `to`, with its value in `currency` at the prices of `to`, smallest value first.",
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
pub static TOP_POSTINGS: BuiltinQuery = BuiltinQuery {
    name: "report.top_postings",
    description: "The ten largest postings to accounts of the type `type` from `from` to `to`, by their value in `currency` at the prices of `to`: income and liabilities by their negated value, so the largest income comes first. Postings that no price converts to `currency` come last.",
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

/// Every built-in query of the report.
pub static QUERIES: &[&BuiltinQuery] = &[&BALANCES, &FLOWS, &TRANSACTION_COUNT, &NET_WORTH, &CHANGES, &ACCOUNT_TOTALS, &TOP_POSTINGS];

/// `GET /api/statistic/summary`: the net worth and the liabilities at the end of the range,
/// and its income, expenses and number of transactions.
pub fn summary(ledger: &Ledger, range: &LedgerDateRange) -> ServerResult<StatisticSummaryEntity> {
    let currency = ledger.options.operating_currency.as_str();
    let balances = by_type(run(ledger, &BALANCES, &Params::new().bind("to", range.to).bind("currency", currency))?);
    let flows = by_type(run(
        ledger,
        &FLOWS,
        &Params::new().bind("from", range.from).bind("to", range.to).bind("currency", currency),
    )?);
    let count = run(ledger, &TRANSACTION_COUNT, &Params::new().bind("from", range.from).bind("to", range.to))?;

    let figure = |figures: &HashMap<String, Figure>, account_type: AccountType| figures.get(&account_type.to_string()).cloned().unwrap_or_default();
    let assets = figure(&balances, AccountType::Assets);
    let liabilities = figure(&balances, AccountType::Liabilities);
    let net_worth = assets.plus(&liabilities);
    let timezone = &ledger.options.timezone;
    Ok(StatisticSummaryEntity {
        from: first_instant(range.from, timezone),
        to: last_instant(range.to, timezone),
        balance: net_worth.amount(currency),
        liability: liabilities.amount(currency),
        income: figure(&flows, AccountType::Income).amount(currency),
        expense: figure(&flows, AccountType::Expenses).amount(currency),
        // an aggregate query without rows has no row, not a zero
        transaction_number: count.rows.first().and_then(|row| row.first()).and_then(Value::as_int).unwrap_or(0),
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
    let params = Params::new()
        .bind("from", range.from)
        .bind("to", range.to)
        .bind("interval", stride(interval))
        .bind("currency", currency);

    // the closing balance of every bucket with postings (and of the bucket before the range)
    let mut closing: BTreeMap<NaiveDate, (Inventory, Figure)> = BTreeMap::new();
    for row in run(ledger, &NET_WORTH, &params)?.rows {
        let mut cells = row.into_iter();
        let (Some(Value::Date(bucket)), Some(balance), Some(units), Some(value)) = (cells.next(), cells.next(), cells.next(), cells.next()) else {
            continue;
        };
        closing.insert(bucket, (inventory(balance), Figure::of(units, value)));
    }

    let buckets = buckets(range, interval);
    let mut balances = HashMap::with_capacity(buckets.len());
    let mut carried = buckets
        .first()
        .and_then(|first| closing.range(..first).next_back())
        .map(|(_, (balance, _))| balance.clone())
        .unwrap_or_default();
    let mut prices: Option<PriceMap> = None;
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
                calculated_amount(&carried.units(), &value, currency)
            }
        };
        balances.insert(bucket, amount);
    }

    let mut changes: HashMap<NaiveDate, HashMap<AccountType, CalculatedAmount>> = HashMap::new();
    for row in run(ledger, &CHANGES, &params)?.rows {
        let mut cells = row.into_iter();
        let (Some(Value::Date(bucket)), Some(Value::Str(account_type)), Some(units), Some(value)) = (cells.next(), cells.next(), cells.next(), cells.next())
        else {
            continue;
        };
        let Ok(account_type) = AccountType::from_str(&account_type) else {
            continue;
        };
        changes
            .entry(bucket)
            .or_default()
            .insert(account_type, Figure::of(units, value).amount(currency));
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
    let params = Params::new()
        .bind("type", account_type.to_string())
        .bind("from", range.from)
        .bind("to", range.to)
        .bind("currency", currency);

    let detail = run(ledger, &ACCOUNT_TOTALS, &params)?
        .rows
        .into_iter()
        .filter_map(|row| {
            let mut cells = row.into_iter();
            let (Some(Value::Str(account)), Some(units), Some(value)) = (cells.next(), cells.next(), cells.next()) else {
                return None;
            };
            Some(ReportRankItemEntity {
                account,
                amount: Figure::of(units, value).amount(currency),
            })
        })
        .collect();

    let top_transactions = run(ledger, &TOP_POSTINGS, &params)?.rows.into_iter().filter_map(top_posting).collect();

    Ok(StatisticRankEntity {
        from: range.from.and_time(NaiveTime::MIN),
        to: range.to.and_time(END_OF_DAY),
        detail,
        top_transactions,
    })
}

/// A row of `report.top_postings` as a journal item.
fn top_posting(row: Vec<Value>) -> Option<AccountJournalDomain> {
    let mut cells = row.into_iter();
    let (
        Some(Value::Date(date)),
        Some(Value::Str(time)),
        Some(Value::Int(timestamp)),
        Some(Value::Str(account)),
        Some(Value::Str(id)),
        Some(payee),
        Some(narration),
        Some(Value::Amount(units)),
        Some(Value::Amount(account_balance)),
    ) = (
        cells.next(),
        cells.next(),
        cells.next(),
        cells.next(),
        cells.next(),
        cells.next(),
        cells.next(),
        cells.next(),
        cells.next(),
    )
    else {
        return None;
    };
    Some(AccountJournalDomain {
        datetime: date.and_time(NaiveTime::from_str(&time).unwrap_or(NaiveTime::MIN)),
        timestamp,
        account,
        trx_id: id,
        payee: text(payee),
        narration: text(narration),
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
/// of its month, as `date_bin(stride, date, 2001-01-01)` bins it.
fn bucket_start(date: NaiveDate, interval: &StatisticInterval) -> NaiveDate {
    match interval {
        StatisticInterval::Day => date,
        StatisticInterval::Week => date - Days::new(u64::from(date.weekday().num_days_from_monday())),
        StatisticInterval::Month => date.with_day(1).unwrap_or(date),
    }
}

/// The day after the bucket that starts on `start`.
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

/// The figures of a query grouped by account type, by type.
fn by_type(result: QueryResult) -> HashMap<String, Figure> {
    result
        .rows
        .into_iter()
        .filter_map(|row| {
            let mut cells = row.into_iter();
            let (Some(Value::Str(account_type)), Some(units), Some(value)) = (cells.next(), cells.next(), cells.next()) else {
                return None;
            };
            Some((account_type, Figure::of(units, value)))
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

/// A text cell; `NULL` is none.
fn text(value: Value) -> Option<String> {
    match value {
        Value::Str(text) => Some(text),
        _ => None,
    }
}
