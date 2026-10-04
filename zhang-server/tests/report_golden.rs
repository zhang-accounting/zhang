//! Golden comparison of the report (`/api/statistic/*`): the old hand-written computation,
//! `report::legacy`, against the built-in queries of `report`, on every fixture ledger.
//!
//! The old report is called the way the old web UI called it: with the first and the last
//! instant of each day of the range in the ledger's timezone. The new one gets the ledger dates.
//!
//! `report_diff` (ignored) prints every difference, for the review of a change; set
//! `REPORT_GOLDEN_LEDGERS` to a `:`-separated list of extra ledger directories to include them.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;

use bigdecimal::{BigDecimal, Zero};
use chrono::{DateTime, Datelike, Days, Months, NaiveDate, NaiveTime, TimeZone, Utc};
use serde_json::Value;
use zhang_ast::amount::{Amount, CalculatedAmount};
use zhang_ast::{AccountType, Flag};
use zhang_core::data_source::LocalFileSystemDataSource;
use zhang_core::data_type::text::ZhangDataType;
use zhang_core::domains::schemas::AccountJournalDomain;
use zhang_core::ledger::Ledger;
use zhang_core::utils::calculable::Calculable;
use zhang_query::{DataType, ParamTypes, Params, Query};
use zhang_server::builtin::{calculated_amount, execute, LedgerDateRange};
use zhang_server::report::{self, legacy};
use zhang_server::request::StatisticInterval;

/// A ledger to compare on.
#[derive(Debug, Clone)]
struct Case {
    name: String,
    dir: PathBuf,
    main: &'static str,
}

/// Every fixture: each `integration-tests/*` ledger in each format it has, and `examples`.
fn fixtures() -> Vec<Case> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut cases = vec![];
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(root.join("integration-tests"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_dir())
        .collect();
    dirs.sort();
    dirs.push(root.join("examples"));
    for dir in dirs {
        for main in ["main.zhang", "main.bean"] {
            if dir.join(main).exists() {
                let name = format!("{}/{}", dir.file_name().unwrap().to_string_lossy(), main);
                cases.push(Case { name, dir: dir.clone(), main });
            }
        }
    }
    cases
}

/// The extra ledgers of `REPORT_GOLDEN_LEDGERS`.
fn extra_ledgers() -> Vec<Case> {
    std::env::var("REPORT_GOLDEN_LEDGERS")
        .unwrap_or_default()
        .split(':')
        .filter(|it| !it.is_empty())
        .flat_map(|dir| {
            let dir = PathBuf::from(dir);
            ["main.zhang", "main.bean"]
                .into_iter()
                .filter(|main| dir.join(main).exists())
                .map(|main| Case {
                    name: format!("{}/{}", dir.display(), main),
                    dir: dir.clone(),
                    main,
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

/// A copy of the ledger's directory, with `option "timezone"` set when `timezone` is given, so
/// the comparison does not depend on the timezone of the machine.
fn load(case: &Case, timezone: Option<&str>) -> Option<Ledger> {
    let scratch = std::env::temp_dir().join(format!("zhang-report-golden-{}", uuid::Uuid::new_v4()));
    copy_dir(&case.dir, &scratch);
    let scratch = scratch.canonicalize().unwrap();
    if let Some(timezone) = timezone {
        let main = scratch.join(case.main);
        let content = std::fs::read_to_string(&main).unwrap();
        std::fs::write(&main, format!("option \"timezone\" \"{}\"\n{}", timezone, content)).unwrap();
    }
    let ledger = if case.main.ends_with(".bean") {
        Ledger::load_with_data_source(
            scratch.clone(),
            case.main.to_owned(),
            Arc::new(LocalFileSystemDataSource::new(beancount::Beancount {})),
        )
    } else {
        Ledger::load_with_data_source(
            scratch.clone(),
            case.main.to_owned(),
            Arc::new(LocalFileSystemDataSource::new(ZhangDataType {})),
        )
    };
    std::fs::remove_dir_all(&scratch).ok();
    match ledger {
        Ok(ledger) => Some(ledger),
        // `examples` includes a file that is not in the repository, and the local file data
        // source does not expand the wildcard includes of the wildcard include fixture
        Err(error)
            if SKIPPED
                .iter()
                .any(|prefix| case.dir.file_name().is_some_and(|name| name.to_string_lossy().starts_with(prefix))) =>
        {
            eprintln!("skipped {}: {}", case.name, error);
            None
        }
        Err(error) => panic!("{} should load: {error}", case.name),
    }
}

/// The fixtures that do not load from a plain directory, by the start of their name.
const SKIPPED: [&str; 2] = ["examples", "wildcard-include-directive-"];

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.path().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// The ranges to compare on: the whole ledger, the month of its last transaction, the three
/// months up to it, and the first day of that month.
fn ranges(ledger: &Ledger) -> Vec<(&'static str, LedgerDateRange, StatisticInterval)> {
    let timezone = ledger.options.timezone;
    let dates: BTreeSet<NaiveDate> = ledger
        .store
        .read()
        .unwrap()
        .transactions
        .values()
        .map(|trx| trx.datetime.with_timezone(&timezone).date_naive())
        .collect();
    let (Some(first), Some(last)) = (dates.first().copied(), dates.last().copied()) else {
        return vec![];
    };
    let month = last.with_day(1).unwrap();
    let month_end = month + Months::new(1) - Days::new(1);
    let quarter = month - Months::new(2);
    vec![
        ("ledger", LedgerDateRange { from: first, to: last }, StatisticInterval::Month),
        ("last month", LedgerDateRange { from: month, to: month_end }, StatisticInterval::Day),
        ("last 3 months", LedgerDateRange { from: quarter, to: month_end }, StatisticInterval::Week),
        ("first of the last month", LedgerDateRange { from: month, to: month }, StatisticInterval::Day),
    ]
}

/// The instants the old web UI sent for a range: the first and the last instant of its days, in
/// `browser`.
fn instants(range: &LedgerDateRange, browser: &chrono_tz::Tz) -> (DateTime<Utc>, DateTime<Utc>) {
    let from = browser.from_local_datetime(&range.from.and_time(NaiveTime::MIN)).earliest().unwrap();
    let to = browser
        .from_local_datetime(&range.to.and_hms_milli_opt(23, 59, 59, 999).unwrap())
        .latest()
        .unwrap();
    (from.with_timezone(&Utc), to.with_timezone(&Utc))
}

/// Why the new report differs from the old one, or that nothing explains it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Reason {
    /// decision 4: pads (flag `P`) are not transactions
    PadsCounted,
    /// #479 accounts: accounts without an `open` directive were left out of the totals
    AccountsWithoutOpen,
    /// decision 3 / #479 valuation: an inverse price, or a price via the cost currency, is used
    PricePaths,
    /// #479 report: the graph valued every day at the prices of the end of the range
    EndOfRangePrices,
    /// #479 report: the graph cut its days off at UTC midnight, not at the ledger's
    UtcCutOff,
    /// decision 5: the top 10 ranks by value in the operating currency, unconvertible last
    RankByValue,
    /// the old order of rows with equal values was arbitrary
    Ties,
    Unexplained,
}

type Reasons = BTreeSet<Reason>;

/// One difference between the old and the new report.
#[derive(Debug, Clone)]
struct Finding {
    ledger: String,
    endpoint: String,
    range: String,
    item: String,
    before: String,
    after: String,
    reasons: Reasons,
}

/// A figure in comparable form: its calculated number and its non-zero units per currency
/// (the engine's inventories drop a currency whose units add up to zero; the old code listed
/// it with 0), with numbers compared as decimals (`0.00` is `0`).
#[derive(Debug, Clone, PartialEq, Default)]
struct Fig {
    calculated: BigDecimal,
    units: BTreeMap<String, BigDecimal>,
}

impl Fig {
    fn of(amount: &CalculatedAmount) -> Fig {
        Fig {
            calculated: amount.calculated.number.normalized(),
            units: units(amount.detail.iter().map(|(currency, number)| (currency.clone(), number.clone()))),
        }
    }

    fn show(&self) -> String {
        let units = self
            .units
            .iter()
            .map(|(currency, number)| format!("{} {}", plain(number), currency))
            .collect::<Vec<_>>();
        format!("{} [{}]", plain(&self.calculated), units.join(", "))
    }

    fn amounts(&self) -> Vec<Amount> {
        self.units
            .iter()
            .map(|(currency, number)| Amount::new(number.clone(), currency.clone()))
            .collect()
    }
}

/// Units per currency, summed, without zeros.
fn units(amounts: impl IntoIterator<Item = (String, BigDecimal)>) -> BTreeMap<String, BigDecimal> {
    let mut sum: BTreeMap<String, BigDecimal> = BTreeMap::new();
    for (currency, number) in amounts {
        *sum.entry(currency).or_default() += number;
    }
    sum.into_iter()
        .filter(|(_, number)| !number.is_zero())
        .map(|(currency, number)| (currency, number.normalized()))
        .collect()
}

/// A number without trailing zeros or an exponent.
fn plain(number: &BigDecimal) -> String {
    let number = number.normalized();
    let (_, scale) = number.as_bigint_and_exponent();
    if scale < 0 {
        number.with_scale(0).to_string()
    } else {
        number.to_string()
    }
}

/// The value the old code gives `units` at `at`: direct prices only, as of that instant.
fn old_value(ledger: &Ledger, units: &Fig, at: DateTime<chrono_tz::Tz>) -> BigDecimal {
    let mut operations = ledger.operations();
    units.amounts().calculate(at, &mut operations).unwrap().calculated.number.normalized()
}

/// The last instant of a ledger date.
fn end_of(date: NaiveDate, timezone: &chrono_tz::Tz) -> DateTime<Utc> {
    timezone
        .from_local_datetime(&date.and_hms_opt(23, 59, 59).unwrap())
        .latest()
        .unwrap()
        .with_timezone(&Utc)
}

/// The engine's units of the postings to `accounts` up to an instant, for checking what the old
/// code added up.
fn engine_units(ledger: &Ledger, accounts: &BTreeSet<String>, until: DateTime<Utc>) -> BTreeMap<String, BigDecimal> {
    let query = Query::compile_with_params(
        "SELECT currency, sum(number) WHERE account IN :accounts AND timestamp <= :until GROUP BY currency",
        &ParamTypes::new().bind("accounts", DataType::Set).bind("until", DataType::Int),
    )
    .unwrap();
    let params = Params::new().bind("accounts", accounts.clone()).bind("until", until.timestamp());
    let result = query.execute_at(ledger, &params, Utc::now().date_naive()).unwrap();
    units(result.rows.into_iter().map(|row| match (&row[0], &row[1]) {
        (zhang_query::Value::Str(currency), zhang_query::Value::Decimal(number)) => (currency.clone(), number.clone()),
        other => panic!("unexpected row {:?}", other),
    }))
}

/// A figure computed by the engine apart from the report's queries: the units of the postings
/// dated `from` to `to` to the accounts whose first component is one of `types` (every account,
/// with an `open` or not), or to `account` alone, and their value in the operating currency at
/// the prices of `at`.
fn engine_figure(ledger: &Ledger, types: &[AccountType], account: Option<&str>, from: NaiveDate, to: NaiveDate, at: NaiveDate) -> Fig {
    let query = Query::compile_with_params(
        "SELECT units(sum(position)), convert(sum(position), :currency, :at) \
         WHERE root(account, 1) IN :types AND (:account IS NULL OR account = :account) AND date >= :from AND date <= :to",
        &ParamTypes::new()
            .bind("currency", DataType::Str)
            .bind("at", DataType::Date)
            .bind("types", DataType::Set)
            .bind("account", DataType::Str)
            .bind("from", DataType::Date)
            .bind("to", DataType::Date),
    )
    .unwrap();
    let currency = ledger.options.operating_currency.clone();
    let params = Params::new()
        .bind("currency", currency.as_str())
        .bind("at", at)
        .bind("types", types.iter().map(|it| it.to_string()).collect::<BTreeSet<_>>())
        .bind("account", account.map(str::to_owned))
        .bind("from", from)
        .bind("to", to);
    let result = query.execute_at(ledger, &params, Utc::now().date_naive()).unwrap();
    let inventory = |value: &zhang_query::Value| match value {
        zhang_query::Value::Inventory(inventory) => inventory.clone(),
        _ => zhang_query::Inventory::new(),
    };
    match result.rows.first() {
        Some(row) => Fig::of(&calculated_amount(&inventory(&row[0]), &inventory(&row[1]), &currency)),
        None => Fig::of(&CalculatedAmount::new(&currency)),
    }
}

/// The first day the engine can date.
fn day_one() -> NaiveDate {
    NaiveDate::from_ymd_opt(1, 1, 1).unwrap()
}

/// The postings of a flow: to the accounts whose first component is one of `types`, or to
/// `account` alone, from `from` to `to`.
#[derive(Clone, Copy)]
struct Flow<'a> {
    types: &'a [AccountType],
    account: Option<&'a str>,
    from: NaiveDate,
    to: NaiveDate,
}

struct Context<'a> {
    ledger: &'a Ledger,
    name: &'a str,
    range: LedgerDateRange,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    findings: Vec<Finding>,
}

impl Context<'_> {
    fn timezone(&self) -> chrono_tz::Tz {
        self.ledger.options.timezone
    }

    fn found(&mut self, endpoint: &str, item: impl Into<String>, before: impl Into<String>, after: impl Into<String>, reasons: Reasons) {
        let reasons = if reasons.is_empty() { Reasons::from([Reason::Unexplained]) } else { reasons };
        self.findings.push(Finding {
            ledger: self.name.to_owned(),
            endpoint: endpoint.to_owned(),
            range: format!("{}..{}", self.range.from, self.range.to),
            item: item.into(),
            before: before.into(),
            after: after.into(),
            reasons,
        });
    }

    /// The accounts of `types` with postings, all of them or only those with an `open`.
    fn accounts(&self, types: &[AccountType], opened_only: bool) -> BTreeSet<String> {
        let store = self.ledger.store.read().unwrap();
        store
            .postings
            .iter()
            .filter(|posting| types.contains(&posting.account.account_type))
            .map(|posting| posting.account.name().to_owned())
            .filter(|account| !opened_only || store.accounts.contains_key(account))
            .collect()
    }

    /// Whether the old figure is what the old code computes: its value, the old way, of its
    /// units at `old_at`.
    fn old_is_reproduced(&self, old: &Fig, old_at: DateTime<Utc>) -> bool {
        old_value(self.ledger, old, old_at.with_timezone(&self.timezone())) == old.calculated
    }

    /// Why the new value differs from the old one, when the new figure is `expected`, the
    /// engine's own computation of it at the new date `new_at`: the old way of valuing gives
    /// the new units another value at the new date than the engine (`PricePaths`: an inverse
    /// price or the cost currency), or another value at the old date than at the new one
    /// (`date_reason`). A new figure that is not the expected one is unexplained.
    fn valuation_reasons(&self, new: &Fig, expected: &Fig, old_at: DateTime<Utc>, new_at: DateTime<Utc>, date_reason: Reason) -> Reasons {
        if new != expected {
            return Reasons::from([Reason::Unexplained]);
        }
        let timezone = self.timezone();
        let old_way_then = old_value(self.ledger, new, old_at.with_timezone(&timezone));
        let old_way_now = old_value(self.ledger, new, new_at.with_timezone(&timezone));
        let mut reasons = Reasons::new();
        if old_way_now != new.calculated {
            reasons.insert(Reason::PricePaths);
        }
        if old_way_then != old_way_now {
            reasons.insert(date_reason);
        }
        reasons
    }

    /// Why the old and the new balance of `types` differ: the old one adds up the opened
    /// accounts until `old_until` and values them the old way at `old_at`; the new one must be
    /// the engine's balance of every account at the end of `day`, valued at `day`. Every step
    /// that changes the figure must be one of the reasons, and the old figure must be reproduced.
    fn balance_reasons(&self, types: &[AccountType], old: &Fig, new: &Fig, old_until: DateTime<Utc>, old_at: DateTime<Utc>, day: NaiveDate) -> Reasons {
        let end_of_day = end_of(day, &self.timezone());
        let opened = self.accounts(types, true);
        let old_units = engine_units(self.ledger, &opened, old_until);
        if old_units != old.units || !self.old_is_reproduced(old, old_at) {
            return Reasons::from([Reason::Unexplained]);
        }
        let expected = engine_figure(self.ledger, types, None, day_one(), day, day);
        if expected.units != new.units || engine_units(self.ledger, &self.accounts(types, false), end_of_day) != new.units {
            return Reasons::from([Reason::Unexplained]);
        }
        let mut reasons = Reasons::new();
        let opened_at_day = engine_units(self.ledger, &opened, end_of_day);
        if opened_at_day != old_units {
            reasons.insert(Reason::UtcCutOff);
        }
        if opened_at_day != new.units {
            reasons.insert(Reason::AccountsWithoutOpen);
        }
        reasons.extend(self.valuation_reasons(new, &expected, old_at, end_of_day, Reason::EndOfRangePrices));
        reasons
    }

    /// Why the old and the new value of a flow differ: the units must be the same, the old value
    /// reproduced at `old_at`, and the new one the engine's at the flow's last day.
    fn flow_reasons(&self, flow: Flow, old: &Fig, new: &Fig, old_at: DateTime<Utc>, date_reason: Reason) -> Reasons {
        if old.units != new.units || !self.old_is_reproduced(old, old_at) {
            return Reasons::from([Reason::Unexplained]);
        }
        let expected = engine_figure(self.ledger, flow.types, flow.account, flow.from, flow.to, flow.to);
        self.valuation_reasons(new, &expected, old_at, end_of(flow.to, &self.timezone()), date_reason)
    }

    fn summary(&mut self) {
        let old = legacy::summary(self.ledger, self.from, self.to).unwrap();
        let new = report::summary(self.ledger, &self.range).unwrap();
        if old.transaction_number != new.transaction_number {
            let pads = {
                let store = self.ledger.store.read().unwrap();
                store
                    .transactions
                    .values()
                    .filter(|trx| trx.flag == Flag::BalancePad && trx.datetime >= self.from && trx.datetime <= self.to)
                    .count() as i64
            };
            let reasons = if old.transaction_number - new.transaction_number == pads {
                Reasons::from([Reason::PadsCounted])
            } else {
                Reasons::new()
            };
            self.found(
                "summary",
                "transaction_number",
                old.transaction_number.to_string(),
                new.transaction_number.to_string(),
                reasons,
            );
        }
        let (from, to) = (self.range.from, self.range.to);
        for (item, old, new, types, balance) in [
            (
                "balance",
                &old.balance,
                &new.balance,
                &[AccountType::Assets, AccountType::Liabilities][..],
                true,
            ),
            ("liability", &old.liability, &new.liability, &[AccountType::Liabilities][..], true),
            ("income", &old.income, &new.income, &[AccountType::Income][..], false),
            ("expense", &old.expense, &new.expense, &[AccountType::Expenses][..], false),
        ] {
            let (old, new) = (Fig::of(old), Fig::of(new));
            if old == new {
                continue;
            }
            let reasons = if balance {
                self.balance_reasons(types, &old, &new, self.to, self.to, to)
            } else {
                let flow = Flow {
                    types,
                    account: None,
                    from,
                    to,
                };
                self.flow_reasons(flow, &old, &new, self.to, Reason::Unexplained)
            };
            self.found("summary", item, old.show(), new.show(), reasons);
        }
    }

    fn graph(&mut self) {
        let timezone = self.timezone();
        let old = legacy::graph(self.ledger, self.from, self.to).unwrap();
        let new = report::graph(self.ledger, &self.range, &StatisticInterval::Day).unwrap();
        let offset = self.to.with_timezone(&timezone).naive_local() - self.to.naive_utc();
        let span = |days: &BTreeSet<NaiveDate>| {
            format!(
                "{}..{}",
                days.first().map(|d| d.to_string()).unwrap_or_default(),
                days.last().map(|d| d.to_string()).unwrap_or_default()
            )
        };
        let days_of = |from: NaiveDate, to: NaiveDate| -> BTreeSet<NaiveDate> {
            std::iter::successors(Some(from), |d| d.succ_opt().filter(|next| *next <= to)).collect()
        };

        let old_days: BTreeSet<NaiveDate> = old.balances.keys().copied().collect();
        let new_days: BTreeSet<NaiveDate> = new.balances.keys().copied().collect();
        // the new days are those of the range, the old ones those of its instants in UTC
        if new_days != days_of(self.range.from, self.range.to) {
            self.found(
                "graph",
                "balances: days of the range",
                span(&days_of(self.range.from, self.range.to)),
                span(&new_days),
                Reasons::new(),
            );
        }
        if old_days != new_days {
            let utc_days = days_of(self.from.date_naive(), self.to.date_naive());
            let reasons = if !offset.is_zero() && utc_days == old_days {
                Reasons::from([Reason::UtcCutOff])
            } else {
                Reasons::new()
            };
            self.found("graph", "balances: days", span(&old_days), span(&new_days), reasons);
        }
        for day in old_days.intersection(&new_days) {
            let (old_fig, new_fig) = (Fig::of(&old.balances[day]), Fig::of(&new.balances[day]));
            if old_fig == new_fig {
                continue;
            }
            // the old day ends at 23:59:59 UTC, and every day is valued at the end of the range
            let old_until = Utc.from_utc_datetime(&day.and_hms_opt(23, 59, 59).unwrap());
            let reasons = self.balance_reasons(&[AccountType::Assets, AccountType::Liabilities], &old_fig, &new_fig, old_until, self.to, *day);
            self.found("graph", format!("balances/{}", day), old_fig.show(), new_fig.show(), reasons);
        }

        let days: BTreeSet<NaiveDate> = old.changes.keys().chain(new.changes.keys()).copied().collect();
        for day in days {
            let types: BTreeSet<String> = old
                .changes
                .get(&day)
                .into_iter()
                .chain(new.changes.get(&day))
                .flat_map(|it| it.keys().map(|t| t.to_string()))
                .collect();
            for account_type in types {
                let account_type = AccountType::from_str(&account_type).unwrap();
                let pick = |changes: &HashMap<NaiveDate, HashMap<AccountType, CalculatedAmount>>| {
                    changes.get(&day).and_then(|it| it.get(&account_type)).map(Fig::of).unwrap_or_default()
                };
                let (old_fig, new_fig) = (pick(&old.changes), pick(&new.changes));
                if old_fig == new_fig {
                    continue;
                }
                // the old change of a day is valued at 23:59:59 UTC of that day
                let old_at = Utc.from_utc_datetime(&day.and_hms_opt(23, 59, 59).unwrap());
                let flow = Flow {
                    types: &[account_type],
                    account: None,
                    from: day,
                    to: day,
                };
                let reasons = self.flow_reasons(flow, &old_fig, &new_fig, old_at, Reason::UtcCutOff);
                self.found("graph", format!("changes/{}/{}", day, account_type), old_fig.show(), new_fig.show(), reasons);
            }
        }
    }

    /// The ten largest postings of `account_type` in the range as the engine values them apart
    /// from `report.top_postings`: by value in the operating currency at the prices of the
    /// last day, income and liabilities negated, those without a price last, then in ledger
    /// order; as `date account units currency id`.
    fn expected_top(&self, account_type: AccountType) -> (Vec<String>, bool) {
        let query = Query::compile_with_params(
            "SELECT date, account, id, units(position), convert(position, :currency, :to) \
             WHERE root(account, 1) = :type AND date >= :from AND date <= :to",
            &ParamTypes::new()
                .bind("currency", DataType::Str)
                .bind("type", DataType::Str)
                .bind("from", DataType::Date)
                .bind("to", DataType::Date),
        )
        .unwrap();
        let currency = self.ledger.options.operating_currency.clone();
        let params = Params::new()
            .bind("currency", currency.as_str())
            .bind("type", account_type.to_string())
            .bind("from", self.range.from)
            .bind("to", self.range.to);
        let rows = query.execute_at(self.ledger, &params, Utc::now().date_naive()).unwrap().rows;
        let mut foreign = false;
        let mut postings: Vec<(bool, BigDecimal, String)> = rows
            .into_iter()
            .map(|row| match &row[..] {
                [zhang_query::Value::Date(date), zhang_query::Value::Str(account), zhang_query::Value::Str(id), zhang_query::Value::Amount(units), zhang_query::Value::Amount(value)] => {
                    foreign |= units.commodity != currency;
                    let positive = matches!(account_type, AccountType::Assets | AccountType::Expenses);
                    let signed = if positive { value.number.clone() } else { -value.number.clone() };
                    let text = format!("{} {} {} {} {}", date, account, plain(&units.number), units.commodity, id);
                    (value.commodity != currency, signed, text)
                }
                other => panic!("unexpected row {:?}", other),
            })
            .collect();
        // a stable sort keeps ledger order between equal values
        postings.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| b.1.cmp(&a.1)));
        (postings.into_iter().take(10).map(|(_, _, text)| text).collect(), foreign)
    }

    fn rank(&mut self, account_type: AccountType) {
        let endpoint = format!("rank {}", account_type);
        let old = legacy::rank(self.ledger, account_type, self.from, self.to).unwrap();
        let new = report::rank(self.ledger, account_type, &self.range).unwrap();
        let old_detail: BTreeMap<String, Fig> = old.detail.iter().map(|it| (it.account.clone(), Fig::of(&it.amount))).collect();
        let new_detail: BTreeMap<String, Fig> = new.detail.iter().map(|it| (it.account.clone(), Fig::of(&it.amount))).collect();
        for account in old_detail.keys().chain(new_detail.keys()).collect::<BTreeSet<_>>() {
            let (old_fig, new_fig) = (
                old_detail.get(account).cloned().unwrap_or_default(),
                new_detail.get(account).cloned().unwrap_or_default(),
            );
            if old_fig == new_fig {
                continue;
            }
            let flow = Flow {
                types: &[account_type],
                account: Some(account),
                from: self.range.from,
                to: self.range.to,
            };
            let reasons = self.flow_reasons(flow, &old_fig, &new_fig, self.to, Reason::Unexplained);
            self.found(&endpoint, format!("detail/{}", account), old_fig.show(), new_fig.show(), reasons);
        }

        let row = |it: &AccountJournalDomain| {
            format!(
                "{} {} {} {} {}",
                it.datetime.date(),
                it.account,
                plain(&it.inferred_unit.number),
                it.inferred_unit.commodity,
                it.trx_id
            )
        };
        let old_rows: Vec<String> = old.top_transactions.iter().map(row).collect();
        let new_rows: Vec<String> = new.top_transactions.iter().map(row).collect();
        let (expected, foreign) = self.expected_top(account_type);
        if new_rows != expected {
            self.found(
                &endpoint,
                "top_transactions: by value",
                expected.join("; "),
                new_rows.join("; "),
                Reasons::new(),
            );
        }
        if old_rows != new_rows {
            let numbers = |rows: &[AccountJournalDomain]| rows.iter().map(|it| it.inferred_unit.number.normalized()).collect::<Vec<_>>();
            let reasons = if new_rows != expected {
                Reasons::new()
            } else if foreign {
                Reasons::from([Reason::RankByValue])
            } else if numbers(&old.top_transactions) == numbers(&new.top_transactions) {
                Reasons::from([Reason::Ties])
            } else {
                Reasons::new()
            };
            self.found(&endpoint, "top_transactions", old_rows.join("; "), new_rows.join("; "), reasons);
        } else {
            for (old_row, new_row) in old.top_transactions.iter().zip(&new.top_transactions) {
                let (old_json, new_json) = (serde_json::to_value(old_row).unwrap(), serde_json::to_value(new_row).unwrap());
                let mut differences = vec![];
                diff("", &normalize(old_json), &normalize(new_json), &mut differences);
                for (path, before, after) in differences {
                    self.found(&endpoint, format!("top_transactions {}{}", row(old_row), path), before, after, Reasons::new());
                }
            }
        }
    }
}

/// Every difference between the old and the new report on one ledger.
fn findings(name: &str, ledger: &Ledger) -> Vec<Finding> {
    let mut all = vec![];
    for (_, range, _) in ranges(ledger) {
        let (from, to) = instants(&range, &ledger.options.timezone);
        let mut context = Context {
            ledger,
            name,
            range,
            from,
            to,
            findings: vec![],
        };
        context.summary();
        // the old graph takes a while on long ranges: it adds up every account for every day
        if (range.to - range.from).num_days() <= 100 {
            context.graph();
        }
        context.rank(AccountType::Expenses);
        context.rank(AccountType::Income);
        all.extend(context.findings);
    }
    all
}

/// JSON without differences of form: numbers as normalised decimals.
fn normalize(value: Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(map.into_iter().map(|(key, value)| (key, normalize(value))).collect()),
        Value::Array(items) => Value::Array(items.into_iter().map(normalize).collect()),
        Value::String(text) => match BigDecimal::from_str(&text) {
            Ok(number) => Value::String(plain(&number)),
            Err(_) => Value::String(text),
        },
        other => other,
    }
}

/// The paths at which two JSON values differ, with both values.
fn diff(path: &str, old: &Value, new: &Value, out: &mut Vec<(String, String, String)>) {
    match (old, new) {
        (Value::Object(a), Value::Object(b)) => {
            let keys: BTreeSet<&String> = a.keys().chain(b.keys()).collect();
            for key in keys {
                let child = format!("{}/{}", path, key);
                match (a.get(key), b.get(key)) {
                    (Some(x), Some(y)) => diff(&child, x, y, out),
                    (x, y) => out.push((child, show(x), show(y))),
                }
            }
        }
        (Value::Array(a), Value::Array(b)) if a.len() == b.len() => {
            for (index, (x, y)) in a.iter().zip(b).enumerate() {
                diff(&format!("{}[{}]", path, index), x, y, out);
            }
        }
        (a, b) if a == b => {}
        (a, b) => out.push((path.to_owned(), a.to_string(), b.to_string())),
    }
}

fn show(value: Option<&Value>) -> String {
    value.map(Value::to_string).unwrap_or_else(|| "-".to_owned())
}

fn reasons(reasons: &Reasons) -> String {
    reasons.iter().map(|it| format!("{:?}", it)).collect::<Vec<_>>().join("+")
}

/// Print every difference between the old and the new report, with its reasons: a summary per
/// endpoint and reasons, then every finding. `REPORT_GOLDEN_TIMEZONE` sets the ledgers'
/// timezone (default: each ledger's own).
#[test]
#[ignore]
fn report_diff() {
    let timezone = std::env::var("REPORT_GOLDEN_TIMEZONE").ok();
    let mut all = vec![];
    for case in fixtures().into_iter().chain(extra_ledgers()) {
        let Some(ledger) = load(&case, timezone.as_deref()) else {
            continue;
        };
        all.extend(findings(&case.name, &ledger));
    }
    let mut groups: BTreeMap<(String, String), Vec<&Finding>> = BTreeMap::new();
    for finding in &all {
        let endpoint = finding.endpoint.split(' ').next().unwrap().to_owned();
        groups.entry((endpoint, reasons(&finding.reasons))).or_default().push(finding);
    }
    let mut out = String::new();
    writeln!(out, "== summary: endpoint, reasons, count, ledgers, first example").unwrap();
    for ((endpoint, reason), findings) in &groups {
        let ledgers: BTreeSet<&str> = findings.iter().map(|it| it.ledger.as_str()).collect();
        let first = findings[0];
        writeln!(
            out,
            "{}\t{}\t{}\t{}\t{} {} {}: {} -> {}",
            endpoint,
            reason,
            findings.len(),
            ledgers.into_iter().collect::<Vec<_>>().join(","),
            first.ledger,
            first.range,
            first.item,
            first.before,
            first.after
        )
        .unwrap();
    }
    writeln!(out, "== findings").unwrap();
    for it in &all {
        writeln!(
            out,
            "{}\t{}\t{}\t{}\t{}\t{}\t{}",
            reasons(&it.reasons),
            it.ledger,
            it.endpoint,
            it.range,
            it.item,
            it.before,
            it.after
        )
        .unwrap();
    }
    println!("{}", out);
}

/// A small ledger with every case the old report got wrong, in a timezone east of UTC.
const HAND_LEDGER: &str = r#"option "operating_currency" "CNY"
option "timezone" "Asia/Shanghai"

1970-01-01 commodity CNY
1970-01-01 commodity USD
1970-01-01 commodity JPY
1970-01-01 commodity AAPL
1970-01-01 commodity HOUR

1970-01-01 open Assets:Bank
1970-01-01 open Assets:USBank
1970-01-01 open Assets:Broker
1970-01-01 open Assets:Travel
1970-01-01 open Assets:Savings
1970-01-01 open Liabilities:Card
1970-01-01 open Expenses:Food
1970-01-01 open Expenses:Travel
1970-01-01 open Expenses:Vacation
1970-01-01 open Income:Salary
1970-01-01 open Income:Vacation
1970-01-01 open Equity:Opening

2025-03-31 price USD 7.0 CNY
2025-03-31 price AAPL 200 USD
2025-03-31 price CNY 20 JPY
2025-04-30 price USD 7.3 CNY

2025-03-31 "Opening" "opening CNY"
  Assets:Bank 10000 CNY
  Equity:Opening

2025-03-31 "Opening" "opening USD"
  Assets:USBank 1000 USD
  Equity:Opening

2025-03-31 23:30:00 "Late" "late march dinner"
  Expenses:Food 40 CNY
  Assets:Bank

2025-04-01 "Landlord" "rent on the first"
  Expenses:Food 1630 CNY
  Assets:Bank

2025-04-02 "Airline" "US trip"
  Expenses:Travel 300 USD
  Assets:USBank

2025-04-03 "Broker" "buy AAPL"
  Assets:Broker 3 AAPL {150 USD}
  Assets:USBank -450 USD

2025-04-05 "Trip" "yen"
  Assets:Travel 2000 JPY @@ 100 CNY
  Assets:Bank -100 CNY

2025-04-06 "Employer" "vacation used"
  Expenses:Vacation 8 HOUR
  Income:Vacation -8 HOUR

2025-04-10 "Employer" "salary"
  Income:Salary -20000 CNY
  Assets:Bank

2025-04-12 "Card" "groceries on the card"
  Expenses:Food 200 CNY
  Liabilities:Card

2025-04-14 "Ghost" "an account without open"
  Assets:Ghost 7 CNY
  Equity:Opening

2025-04-15 balance Assets:Savings 500 CNY with pad Equity:Opening

2025-04-30 23:30:00 "Late" "late april dinner"
  Expenses:Food 60 CNY
  Assets:Bank

2025-05-01 "Landlord" "may rent"
  Expenses:Food 1630 CNY
  Assets:Bank
"#;

/// [`HAND_LEDGER`], loaded the way the server loads a ledger.
fn hand_ledger() -> Ledger {
    let dir = std::env::temp_dir().join(format!("zhang-report-hand-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.zhang"), HAND_LEDGER).unwrap();
    let case = Case {
        name: "hand".to_owned(),
        dir: dir.clone(),
        main: "main.zhang",
    };
    let ledger = load(&case, None).unwrap();
    std::fs::remove_dir_all(dir).ok();
    ledger
}

fn d(date: &str) -> NaiveDate {
    NaiveDate::from_str(date).unwrap()
}

fn number(text: &str) -> BigDecimal {
    BigDecimal::from_str(text).unwrap().normalized()
}

/// A figure from its calculated number and its units.
fn fig(calculated: &str, units: &[(&str, &str)]) -> Fig {
    Fig {
        calculated: number(calculated),
        units: units.iter().map(|(n, currency)| (currency.to_string(), number(n))).collect(),
    }
}

const APRIL: LedgerDateRange = LedgerDateRange {
    from: match NaiveDate::from_ymd_opt(2025, 4, 1) {
        Some(date) => date,
        None => panic!(),
    },
    to: match NaiveDate::from_ymd_opt(2025, 4, 30) {
        Some(date) => date,
        None => panic!(),
    },
};

/// The summary of April, checked by hand: everything on the 1st counts (#400), every account
/// counts, the net worth is valued at the prices of April 30 with an inverse price (JPY) and a
/// price via the cost currency (AAPL), and the pad is not a transaction.
#[test]
fn summary_of_the_hand_ledger() {
    let ledger = hand_ledger();
    let summary = report::summary(&ledger, &APRIL).unwrap();
    // CNY: 10000 - 40 - 1630 - 100 + 20000 - 60 (bank) + 500 (pad) - 200 (card) + 7 (no open);
    // 250 USD x 7.3 + 3 AAPL x 200 USD x 7.3 + 2000 JPY / 20
    assert_eq!(
        Fig::of(&summary.balance),
        fig("34782", &[("28477", "CNY"), ("250", "USD"), ("3", "AAPL"), ("2000", "JPY")])
    );
    assert_eq!(Fig::of(&summary.liability), fig("-200", &[("-200", "CNY")]));
    assert_eq!(Fig::of(&summary.income), fig("-20000", &[("-20000", "CNY"), ("-8", "HOUR")]));
    // 1630 + 200 + 60 CNY, 300 USD x 7.3; hours do not convert
    assert_eq!(Fig::of(&summary.expense), fig("4080", &[("1890", "CNY"), ("300", "USD"), ("8", "HOUR")]));
    // nine transactions in April, without the pad of April 15
    assert_eq!(summary.transaction_number, 9);
    assert_eq!(summary.from.to_rfc3339(), "2025-03-31T16:00:00+00:00");
    assert_eq!(summary.to.to_rfc3339(), "2025-04-30T15:59:59+00:00");

    // the old summary, as the old web UI asked for it, misses all of it
    let from = DateTime::parse_from_rfc3339("2025-04-01T00:00:01+08:00").unwrap().with_timezone(&Utc);
    let to = DateTime::parse_from_rfc3339("2025-04-30T23:59:59.999+08:00").unwrap().with_timezone(&Utc);
    let old = legacy::summary(&ledger, from, to).unwrap();
    assert_eq!(
        Fig::of(&old.balance),
        fig("30295", &[("28470", "CNY"), ("250", "USD"), ("3", "AAPL"), ("2000", "JPY")])
    );
    assert_eq!(Fig::of(&old.expense), fig("2450", &[("260", "CNY"), ("300", "USD"), ("8", "HOUR")]));
    assert_eq!(old.transaction_number, 9);
}

/// A range given as instants is read as the dates of those instants in the ledger's timezone.
#[test]
fn ranges_are_ledger_dates() {
    let timezone: chrono_tz::Tz = "Asia/Shanghai".parse().unwrap();
    let range = LedgerDateRange::from_query("2025-04-01", "2025-04-30", &timezone).unwrap();
    assert_eq!(range, APRIL);
    let range = LedgerDateRange::from_query("2025-03-31T16:00:00.000Z", "2025-04-30T15:59:59.999Z", &timezone).unwrap();
    assert_eq!(range, APRIL);
    assert!(LedgerDateRange::from_query("April", "2025-04-30", &timezone).is_err());
}

/// The ranking of April, checked by hand: by value in CNY at the prices of April 30, with the
/// hours, which do not convert, last.
#[test]
fn ranking_of_the_hand_ledger() {
    let ledger = hand_ledger();
    let expenses = report::rank(&ledger, AccountType::Expenses, &APRIL).unwrap();
    let detail: Vec<(String, Fig)> = expenses.detail.iter().map(|it| (it.account.clone(), Fig::of(&it.amount))).collect();
    assert_eq!(
        detail,
        vec![
            ("Expenses:Vacation".to_owned(), fig("0", &[("8", "HOUR")])),
            ("Expenses:Food".to_owned(), fig("1890", &[("1890", "CNY")])),
            ("Expenses:Travel".to_owned(), fig("2190", &[("300", "USD")])),
        ]
    );
    let top: Vec<String> = expenses
        .top_transactions
        .iter()
        .map(|it| {
            format!(
                "{} {} {} {} {}",
                it.datetime,
                it.account,
                plain(&it.inferred_unit.number),
                it.inferred_unit.commodity,
                it.narration.clone().unwrap_or_default()
            )
        })
        .collect();
    assert_eq!(
        top,
        vec![
            "2025-04-02 00:00:00 Expenses:Travel 300 USD US trip",
            "2025-04-01 00:00:00 Expenses:Food 1630 CNY rent on the first",
            "2025-04-12 00:00:00 Expenses:Food 200 CNY groceries on the card",
            "2025-04-30 23:30:00 Expenses:Food 60 CNY late april dinner",
            "2025-04-06 00:00:00 Expenses:Vacation 8 HOUR vacation used",
        ]
    );
    let rent = &expenses.top_transactions[1];
    assert_eq!(rent.payee.as_deref(), Some("Landlord"));
    assert_eq!(rent.timestamp, 1743436800);
    assert_eq!(
        (plain(&rent.account_after.number), rent.account_after.commodity.as_str()),
        ("1670".to_owned(), "CNY")
    );
    let trx_id = ledger
        .store
        .read()
        .unwrap()
        .transactions
        .values()
        .find(|trx| trx.narration.as_deref() == Some("rent on the first"))
        .unwrap()
        .id
        .to_string();
    assert_eq!(rent.trx_id, trx_id);

    // the query as "Open query" runs it: its value column is each posting at the prices of `to`
    let params = APRIL.bind(Params::new().bind("type", "Expenses").bind("currency", "CNY"));
    let result = execute(&ledger, "report.top_postings", &params, false).unwrap();
    let value = result.columns.iter().position(|column| column.name == "value").unwrap();
    let values: Vec<String> = result.rows.iter().map(|row| row[value].to_string()).collect();
    // 300 USD at 7.3, the price of April 30; hours have no price
    assert_eq!(values, vec!["2190.0 CNY", "1630 CNY", "200 CNY", "60 CNY", "8 HOUR"]);

    let income = report::rank(&ledger, AccountType::Income, &APRIL).unwrap();
    let top: Vec<String> = income
        .top_transactions
        .iter()
        .map(|it| format!("{} {}", it.account, plain(&it.inferred_unit.number)))
        .collect();
    assert_eq!(top, vec!["Income:Salary -20000", "Income:Vacation -8"]);

    // the old ranking, asked from the first second of April, misses the rent on the 1st and
    // ranks the 300 USD below 1630 CNY
    let from = DateTime::parse_from_rfc3339("2025-04-01T00:00:01+08:00").unwrap().with_timezone(&Utc);
    let to = DateTime::parse_from_rfc3339("2025-04-30T23:59:59.999+08:00").unwrap().with_timezone(&Utc);
    let old = legacy::rank(&ledger, AccountType::Expenses, from, to).unwrap();
    let top: Vec<String> = old.top_transactions.iter().map(|it| it.narration.clone().unwrap_or_default()).collect();
    assert_eq!(top, vec!["US trip", "groceries on the card", "late april dinner", "vacation used"]);
}

/// The graph of April, checked by hand: one point per day, week (from Monday) or month, each
/// valued at the prices of its last day (April 30 for the last), the days without postings
/// carried over from the day before.
#[test]
fn graph_of_the_hand_ledger() {
    let ledger = hand_ledger();
    let points = |interval: StatisticInterval| {
        let graph = report::graph(&ledger, &APRIL, &interval).unwrap();
        let mut balances: Vec<(NaiveDate, Fig)> = graph.balances.iter().map(|(day, amount)| (*day, Fig::of(amount))).collect();
        balances.sort_by_key(|(day, _)| *day);
        (balances, graph)
    };
    let units = |cny: &str| {
        vec![
            (cny.to_owned(), "CNY"),
            ("250".to_owned(), "USD"),
            ("3".to_owned(), "AAPL"),
            ("2000".to_owned(), "JPY"),
        ]
    };
    let with_units = |calculated: &str, cny: &str| {
        let units = units(cny);
        fig(calculated, &units.iter().map(|(n, c)| (n.as_str(), *c)).collect::<Vec<_>>())
    };

    let (days, graph) = points(StatisticInterval::Day);
    assert_eq!(days.len(), 30);
    assert_eq!(days[0], (d("2025-04-01"), fig("15330", &[("8330", "CNY"), ("1000", "USD")])));
    // April 29 has no postings: the balance of April 15 on, at the prices of April 29 (7.0)
    assert_eq!(days[28], (d("2025-04-29"), with_units("34587", "28537")));
    // April 30: the dinner, and the price of 7.3; the same as the summary
    assert_eq!(days[29], (d("2025-04-30"), with_units("34782", "28477")));
    let changes = &graph.changes[&d("2025-04-02")];
    assert_eq!(Fig::of(&changes[&AccountType::Expenses]), fig("2100", &[("300", "USD")]));
    assert_eq!(Fig::of(&changes[&AccountType::Assets]), fig("-2100", &[("-300", "USD")]));
    assert!(!graph.changes.contains_key(&d("2025-04-29")));
    assert_eq!(graph.from.to_string(), "2025-04-01 00:00:00");
    assert_eq!(graph.to.to_string(), "2025-04-30 23:59:59");

    let (weeks, graph) = points(StatisticInterval::Week);
    assert_eq!(
        weeks,
        vec![
            // March 31 to April 6, from April 1 on, at the prices of April 6
            (d("2025-03-31"), with_units("14280", "8230")),
            (d("2025-04-07"), with_units("34080", "28030")),
            (d("2025-04-14"), with_units("34587", "28537")),
            // no postings: carried over, at the prices of April 27
            (d("2025-04-21"), with_units("34587", "28537")),
            // April 28 to May 4, until April 30
            (d("2025-04-28"), with_units("34782", "28477")),
        ]
    );
    // the expenses of April 1 to 6, at the prices of April 6; hours do not convert
    assert_eq!(
        Fig::of(&graph.changes[&d("2025-03-31")][&AccountType::Expenses]),
        fig("3730", &[("1630", "CNY"), ("300", "USD"), ("8", "HOUR")])
    );

    let (months, graph) = points(StatisticInterval::Month);
    assert_eq!(months, vec![(d("2025-04-01"), with_units("34782", "28477"))]);
    assert_eq!(
        Fig::of(&graph.changes[&d("2025-04-01")][&AccountType::Expenses]),
        fig("4080", &[("1890", "CNY"), ("300", "USD"), ("8", "HOUR")])
    );
}

/// The reasons of the differences between the old and the new report, per ledger, on every
/// fixture and the hand ledger, with the fixtures' timezone set to `timezone`.
fn reasons_by_ledger(timezone: &str) -> (BTreeMap<String, Reasons>, Vec<Finding>) {
    let mut by_ledger: BTreeMap<String, Reasons> = BTreeMap::new();
    let mut unexplained = vec![];
    let ledgers = fixtures()
        .into_iter()
        .filter_map(|case| load(&case, Some(timezone)).map(|ledger| (case.name, ledger)))
        .chain([("hand".to_owned(), hand_ledger())]);
    for (name, ledger) in ledgers {
        for finding in findings(&name, &ledger) {
            if finding.reasons.contains(&Reason::Unexplained) {
                unexplained.push(finding.clone());
            }
            by_ledger.entry(name.clone()).or_default().extend(finding.reasons);
        }
    }
    (by_ledger, unexplained)
}

/// Every difference between the old and the new report is one of the bugs of #479 or one of
/// its decisions, and on each ledger exactly the expected ones show, in UTC, where the old
/// graph's day cut-off is right.
#[test]
fn every_difference_is_documented() {
    use Reason::*;
    let (actual, unexplained) = reasons_by_ledger("UTC");
    assert!(
        unexplained.is_empty(),
        "unexplained differences:\n{}",
        unexplained
            .iter()
            .map(|it| format!("{} {} {} {}: {} -> {}", it.ledger, it.endpoint, it.range, it.item, it.before, it.after))
            .collect::<Vec<_>>()
            .join("\n")
    );
    let expected: BTreeMap<String, Reasons> = [
        ("fava-demo-ledger/main.zhang", vec![EndOfRangePrices, RankByValue]),
        (
            "hand",
            vec![AccountsWithoutOpen, PricePaths, EndOfRangePrices, UtcCutOff, RankByValue, PadsCounted],
        ),
        ("pad_info_should_be_used_once_beancount/main.bean", vec![PadsCounted]),
    ]
    .into_iter()
    .map(|(name, reasons)| (name.to_owned(), reasons.into_iter().collect()))
    .collect();
    assert_eq!(actual, expected, "the reasons per ledger");
}

/// East of UTC, the old graph's days end at 08:00, so on every ledger with transactions it is
/// off by a day; apart from that, the differences are those of UTC.
#[test]
fn east_of_utc_the_old_graph_is_off_by_a_day() {
    let (utc, _) = reasons_by_ledger("UTC");
    let (east, unexplained) = reasons_by_ledger("Asia/Shanghai");
    assert!(unexplained.is_empty(), "unexplained differences: {:?}", unexplained);
    let with_transactions: BTreeSet<String> = fixtures()
        .into_iter()
        .filter_map(|case| {
            load(&case, Some("UTC"))
                .filter(|ledger| !ledger.store.read().unwrap().transactions.is_empty())
                .map(|_| case.name)
        })
        .chain(["hand".to_owned()])
        .collect();
    assert_eq!(east.keys().cloned().collect::<BTreeSet<_>>(), with_transactions);
    for (ledger, reasons) in east {
        assert!(reasons.contains(&Reason::UtcCutOff), "{}: {:?}", ledger, reasons);
        let others: Reasons = reasons.into_iter().filter(|it| *it != Reason::UtcCutOff).collect();
        let in_utc: Reasons = utc
            .get(&ledger)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|it| *it != Reason::UtcCutOff)
            .collect();
        assert_eq!(others, in_utc, "{}", ledger);
    }
}

/// A ledger whose weeks and months without postings see the price of the dollar change, and
/// with prices after the ends of [`CARRY_RANGES`]: the cases where the date a bucket is valued
/// at matters.
const CARRY_LEDGER: &str = r#"option "operating_currency" "CNY"
option "timezone" "Asia/Shanghai"

1970-01-01 commodity CNY
1970-01-01 commodity USD
1970-01-01 open Assets:Bank
1970-01-01 open Assets:USBank
1970-01-01 open Expenses:Travel
1970-01-01 open Equity:Opening

2025-03-31 price USD 7.00 CNY
2025-04-10 price USD 7.10 CNY
2025-04-18 price USD 7.15 CNY
2025-05-20 price USD 7.30 CNY
2025-06-20 price USD 7.40 CNY

2025-03-31 "Opening" "dollars"
  Assets:USBank 1000 USD
  Assets:Bank 500 CNY
  Equity:Opening -1000 USD
  Equity:Opening -500 CNY

2025-04-02 "Trip" "in the week of March 31"
  Expenses:Travel 100 USD
  Assets:USBank

2025-04-15 "Trip" "in the week of April 14"
  Expenses:Travel 50 USD
  Assets:USBank

2025-06-05 "Trip" "in June"
  Expenses:Travel 20 USD
  Assets:USBank
"#;

/// The ranges of [`CARRY_LEDGER`] whose buckets the valuation date of matters:
/// - April by week: the week of April 7 has no postings and the price changes on April 10, so
///   it is valued on April 13, not on April 7;
/// - April 1 to 9 by week: the same week is the last, valued on April 9, before the price of
///   April 10;
/// - April 1 to 16 by week: the last week has a posting in dollars and the price of April 18 is
///   after the range;
/// - April to June by month: May has no postings and the price changes on May 20;
/// - April 1 to May 10 by month: May is the last month, valued before the price of May 20;
/// - April 1 to June 10 by month: June has a posting in dollars, and the price of June 20 is
///   after the range.
const CARRY_RANGES: [(&str, &str, StatisticInterval); 6] = [
    ("2025-04-01", "2025-04-30", StatisticInterval::Week),
    ("2025-04-01", "2025-04-09", StatisticInterval::Week),
    ("2025-04-01", "2025-04-16", StatisticInterval::Week),
    ("2025-04-01", "2025-06-30", StatisticInterval::Month),
    ("2025-04-01", "2025-05-10", StatisticInterval::Month),
    ("2025-04-01", "2025-06-10", StatisticInterval::Month),
];

/// [`CARRY_LEDGER`], loaded the way the server loads a ledger.
fn carry_ledger() -> Ledger {
    let dir = std::env::temp_dir().join(format!("zhang-report-carry-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.zhang"), CARRY_LEDGER).unwrap();
    let case = Case {
        name: "carry".to_owned(),
        dir: dir.clone(),
        main: "main.zhang",
    };
    let ledger = load(&case, None).unwrap();
    std::fs::remove_dir_all(dir).ok();
    ledger
}

/// Every ledger of the comparison, in UTC, and the two hand ledgers.
fn all_ledgers() -> Vec<(String, Ledger)> {
    fixtures()
        .into_iter()
        .filter_map(|case| load(&case, Some("UTC")).map(|ledger| (case.name, ledger)))
        .chain([("hand".to_owned(), hand_ledger()), ("carry".to_owned(), carry_ledger())])
        .collect()
}

/// The ranges to check the buckets of a ledger on: those of [`ranges`] by day, week and month
/// (up to about three months by day and a year by week), and [`CARRY_RANGES`] for
/// [`CARRY_LEDGER`].
fn bucket_ranges(name: &str, ledger: &Ledger) -> Vec<(LedgerDateRange, StatisticInterval)> {
    // long ranges only by month, to keep the test quick: it runs six queries per bucket
    let mut all: Vec<(LedgerDateRange, StatisticInterval)> = ranges(ledger)
        .into_iter()
        .flat_map(|(_, range, _)| [StatisticInterval::Day, StatisticInterval::Week, StatisticInterval::Month].map(|interval| (range, interval)))
        .filter(|(range, interval)| match interval {
            StatisticInterval::Day => (range.to - range.from).num_days() <= 100,
            StatisticInterval::Week => (range.to - range.from).num_days() <= 400,
            StatisticInterval::Month => true,
        })
        .collect();
    if name == "carry" {
        all.extend(
            CARRY_RANGES
                .iter()
                .map(|(from, to, interval)| (LedgerDateRange { from: d(from), to: d(to) }, *interval)),
        );
    }
    all
}

/// The first and the last day of the bucket of `day`, computed apart from the report: the day,
/// its week from Monday to Sunday, or its month.
fn bucket_of(day: NaiveDate, interval: &StatisticInterval) -> (NaiveDate, NaiveDate) {
    match interval {
        StatisticInterval::Day => (day, day),
        StatisticInterval::Week => {
            let monday = day - Days::new(day.weekday().num_days_from_monday() as u64);
            (monday, monday + Days::new(6))
        }
        StatisticInterval::Month => {
            let first = day.with_day(1).unwrap();
            (first, first + Months::new(1) - Days::new(1))
        }
    }
}

const TYPES: [AccountType; 5] = [
    AccountType::Assets,
    AccountType::Liabilities,
    AccountType::Equity,
    AccountType::Income,
    AccountType::Expenses,
];

/// Every bucket of the graph, by day, week and month, is a bucket of the range, and its point is
/// the net worth at its last day in the range valued at the prices of that day, as a query for
/// that day alone computes it, whether the bucket has postings or is carried over; what each
/// account type changed by in it is valued at the same day.
#[test]
fn every_bucket_is_valued_at_its_last_day_in_the_range() {
    for (name, ledger) in all_ledgers() {
        for (range, interval) in bucket_ranges(&name, &ledger) {
            let graph = report::graph(&ledger, &range, &interval).unwrap();
            check_buckets(
                &format!("{} {}..{} {:?}", name, range.from, range.to, interval),
                &ledger,
                &range,
                &interval,
                &graph,
            );
        }
    }
}

/// Every bucket of `graph` is one of the range, its net worth that of a query for its last day
/// in the range alone, and what each account type changed by in it valued at that day.
fn check_buckets(label: &str, ledger: &Ledger, range: &LedgerDateRange, interval: &StatisticInterval, graph: &zhang_server::response::StatisticGraphEntity) {
    let expected: BTreeSet<NaiveDate> = std::iter::successors(Some(range.from), |day| day.succ_opt().filter(|next| *next <= range.to))
        .map(|day| bucket_of(day, interval).0)
        .collect();
    assert_eq!(graph.balances.keys().copied().collect::<BTreeSet<_>>(), expected, "{}: buckets", label);
    for start in expected {
        let last = bucket_of(start, interval).1.min(range.to);
        let net_worth = engine_figure(ledger, &[AccountType::Assets, AccountType::Liabilities], None, day_one(), last, last);
        assert_eq!(Fig::of(&graph.balances[&start]), net_worth, "{}: net worth of {}", label, start);
        for account_type in TYPES {
            let change = graph.changes.get(&start).and_then(|it| it.get(&account_type)).map(Fig::of).unwrap_or_default();
            let expected = engine_figure(ledger, &[account_type], None, start.max(range.from), last, last);
            assert_eq!(change, expected, "{}: {} of {}", label, account_type, start);
        }
    }
}

/// A ledger, loaded from its text the way the server loads a ledger.
fn ledger_of(name: &str, text: &str) -> Ledger {
    let dir = std::env::temp_dir().join(format!("zhang-report-{}-{}", name, uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.zhang"), text).unwrap();
    let case = Case {
        name: name.to_owned(),
        dir: dir.clone(),
        main: "main.zhang",
    };
    let ledger = load(&case, None).unwrap();
    std::fs::remove_dir_all(dir).ok();
    ledger
}

/// Ten years of investing: ten funds bought every month, each lot at its own cost (1,200 open
/// lots by the end), with a price of every fund every month, a salary every month and a coffee
/// every day.
fn invest_ledger() -> String {
    let mut text = String::from("option \"operating_currency\" \"USD\"\noption \"timezone\" \"UTC\"\n1970-01-01 commodity USD\n");
    text.push_str("1970-01-01 open Assets:Cash\n1970-01-01 open Assets:Broker\n1970-01-01 open Equity:Opening\n1970-01-01 open Expenses:Food\n1970-01-01 open Income:Salary\n");
    for fund in 0..10 {
        text.push_str(&format!("1970-01-01 commodity FUND{}\n", fund));
    }
    text.push_str("\n2015-01-01 * \"Opening\" \"cash\"\n  Assets:Cash 10000 USD\n  Equity:Opening\n");
    let mut day = d("2015-01-01");
    while day <= d("2024-12-31") {
        if day.day() == 1 {
            text.push_str(&format!("\n{} * \"Employer\" \"salary\"\n  Assets:Cash 5000 USD\n  Income:Salary\n", day));
            let month = (day.year() - 2015) * 12 + day.month() as i32;
            for fund in 0..10 {
                let price = 100 + fund + (month * 7 + fund * 3) % 50;
                text.push_str(&format!(
                    "\n{day} * \"Broker\" \"monthly buy FUND{fund}\"\n  Assets:Broker 1 FUND{fund} {{{price} USD}}\n  Assets:Cash -{price} USD\n{day} price FUND{fund} {price} USD\n"
                ));
            }
        }
        text.push_str(&format!("\n{} * \"Cafe\" \"coffee\"\n  Expenses:Food 4.50 USD\n  Assets:Cash\n", day));
        day = day.succ_opt().unwrap();
    }
    text
}

/// Fifteen years in seventy currencies: a wallet opened with all of them, each priced in dollars,
/// spending one unit of another currency every day.
fn fx_ledger() -> String {
    let mut text = String::from("option \"operating_currency\" \"USD\"\noption \"timezone\" \"UTC\"\n1970-01-01 commodity USD\n");
    text.push_str("1970-01-01 open Assets:Wallet\n1970-01-01 open Equity:Opening\n1970-01-01 open Expenses:Food\n");
    for currency in 0..70 {
        text.push_str(&format!("1970-01-01 commodity X{:02}\n", currency));
    }
    text.push_str("\n2010-01-01 * \"Opening\" \"wallets\"\n");
    for currency in 0..70 {
        text.push_str(&format!("  Assets:Wallet 100000 X{c:02}\n  Equity:Opening -100000 X{c:02}\n", c = currency));
    }
    for currency in 0..70 {
        text.push_str(&format!("2010-01-01 price X{:02} 1.5 USD\n", currency));
    }
    let mut day = d("2010-01-02");
    let mut index = 0;
    while day <= d("2024-12-31") {
        text.push_str(&format!(
            "\n{} * \"Shop\" \"spend\"\n  Expenses:Food 1 X{:02}\n  Assets:Wallet\n",
            day,
            index % 70
        ));
        if index % 30 == 0 {
            text.push_str(&format!("{} price X{:02} {}.{:02} USD\n", day, index % 70, 1 + index % 3, index % 100));
        }
        index += 1;
        day = day.succ_opt().unwrap();
    }
    text
}

/// Ledgers with a long history, of many lots and of many currencies: a month by day is graphed
/// right, and costs what the month holds, not what the history before it holds.
#[test]
fn a_graph_costs_what_its_range_holds() {
    use zhang_server::report::{graph_rows, GraphLimits};
    let december = LedgerDateRange {
        from: d("2024-12-01"),
        to: d("2024-12-31"),
    };
    let last_day = LedgerDateRange {
        from: d("2024-12-31"),
        to: d("2024-12-31"),
    };
    let decade = LedgerDateRange {
        from: d("2015-01-01"),
        to: d("2024-12-31"),
    };
    for (name, ledger, month_values) in [
        // every point holds 1,200 lots and ten funds
        ("invest", ledger_of("invest", &invest_ledger()), 100_000),
        // every point holds seventy currencies
        ("fx", ledger_of("fx", &fx_ledger()), 20_000),
    ] {
        for interval in [StatisticInterval::Day, StatisticInterval::Week, StatisticInterval::Month] {
            let graph = report::graph(&ledger, &december, &interval).unwrap_or_else(|error| panic!("{} {:?}: {}", name, interval, error));
            check_buckets(&format!("{} December {:?}", name, interval), &ledger, &december, &interval, &graph);
        }
        let within = |max_values| GraphLimits {
            max_points: 50_000,
            max_values,
            timeout: None,
        };
        let graph = graph_rows(&ledger, &december, &StatisticInterval::Day, within(month_values)).and_then(|rows| rows.build());
        assert_eq!(
            graph.map(|it| it.balances.len()).ok(),
            Some(31),
            "{}: December by day within {} values",
            name,
            month_values
        );
        let graph = graph_rows(&ledger, &last_day, &StatisticInterval::Day, within(month_values / 10)).and_then(|rows| rows.build());
        assert_eq!(
            graph.map(|it| it.balances.len()).ok(),
            Some(1),
            "{}: one day within {} values",
            name,
            month_values / 10
        );
        // what the queries hold counts, not only the points: a day of invest holds its 1,200 lots
        if name == "invest" {
            let error = graph_rows(&ledger, &last_day, &StatisticInterval::Day, within(1_000))
                .and_then(|rows| rows.build())
                .err()
                .unwrap();
            assert!(error.to_string().contains("has too many points or currencies"), "{}: {}", name, error);
        }
        // ten years by day hold a hundred times more, and are a 400 in the graph's terms
        let error = graph_rows(&ledger, &decade, &StatisticInterval::Day, within(month_values))
            .and_then(|rows| rows.build())
            .err()
            .unwrap();
        assert!(error.to_string().contains("has too many points or currencies"), "{}: {}", name, error);
        // by month they fit
        let graph = graph_rows(&ledger, &decade, &StatisticInterval::Month, within(month_values * 5)).and_then(|rows| rows.build());
        assert_eq!(graph.map(|it| it.balances.len()).ok(), Some(120), "{}: ten years by month", name);
    }
}

/// A week or a month of the graph agrees with the days in it: its point is the point of its last
/// day in the range, its value included, and what it changed by adds up the changes of its
/// days, valued at that last day.
#[test]
fn weeks_and_months_add_up_the_days() {
    for (name, ledger) in all_ledgers() {
        let mut checked: BTreeSet<(NaiveDate, NaiveDate)> = BTreeSet::new();
        for (range, _) in bucket_ranges(&name, &ledger) {
            if !checked.insert((range.from, range.to)) {
                continue;
            }
            let label = format!("{} {}..{}", name, range.from, range.to);
            let daily = report::graph(&ledger, &range, &StatisticInterval::Day).unwrap();
            for interval in [StatisticInterval::Week, StatisticInterval::Month] {
                let graph = report::graph(&ledger, &range, &interval).unwrap();
                let mut last_day: BTreeMap<NaiveDate, NaiveDate> = BTreeMap::new();
                let mut changes: BTreeMap<(NaiveDate, String), BTreeMap<String, BigDecimal>> = BTreeMap::new();
                for day in daily.balances.keys() {
                    let bucket = bucket_of(*day, &interval).0;
                    let last = last_day.entry(bucket).or_insert(*day);
                    *last = (*last).max(*day);
                    for (account_type, amount) in daily.changes.get(day).into_iter().flatten() {
                        let sum = changes.entry((bucket, account_type.to_string())).or_default();
                        for (currency, number) in &amount.detail {
                            *sum.entry(currency.clone()).or_default() += number;
                        }
                    }
                }
                assert_eq!(
                    graph.balances.keys().copied().collect::<BTreeSet<_>>(),
                    last_day.keys().copied().collect(),
                    "{} {:?}: buckets",
                    label,
                    interval
                );
                for (bucket, last) in &last_day {
                    assert_eq!(
                        Fig::of(&graph.balances[bucket]),
                        Fig::of(&daily.balances[last]),
                        "{} {:?}: {}",
                        label,
                        interval,
                        bucket
                    );
                }
                let actual: BTreeMap<(NaiveDate, String), Fig> = graph
                    .changes
                    .iter()
                    .flat_map(|(bucket, by_type)| {
                        by_type
                            .iter()
                            .map(move |(account_type, amount)| ((*bucket, account_type.to_string()), Fig::of(amount)))
                    })
                    .collect();
                let expected: BTreeMap<(NaiveDate, String), Fig> = changes
                    .into_iter()
                    .map(|((bucket, account_type), sum)| {
                        let account_type = AccountType::from_str(&account_type).unwrap();
                        let last = last_day[&bucket];
                        let valued = engine_figure(&ledger, &[account_type], None, bucket.max(range.from), last, last);
                        assert_eq!(valued.units, units(sum), "{} {:?}: units of {} {}", label, interval, bucket, account_type);
                        ((bucket, account_type.to_string()), valued)
                    })
                    .collect();
                assert_eq!(actual, expected, "{} {:?}: changes", label, interval);
            }
        }
    }
}

/// The handler's answers to a range it cannot graph and to an unknown account type are 400s
/// that say why.
#[tokio::test]
async fn bad_report_requests_are_bad_requests() {
    use axum::extract::{Path as UrlPath, Query as UrlQuery, State};
    use axum::response::IntoResponse;
    use zhang_server::request::{StatisticGraphRequest, StatisticRequest};
    use zhang_server::routes::statistics::{get_statistic_graph, get_statistic_rank_detail_by_account_type};
    use zhang_server::state::SharedLedger;

    let ledger = SharedLedger(Arc::new(tokio::sync::RwLock::new(carry_ledger())));
    let answer = |response: axum::response::Response| async move {
        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body: Value = serde_json::from_slice(&body).unwrap();
        (status.as_u16(), body["message"].as_str().unwrap_or_default().to_owned())
    };

    let request = StatisticGraphRequest {
        from: "0001-01-01".to_owned(),
        to: "9999-12-31".to_owned(),
        interval: StatisticInterval::Day,
    };
    let (status, message) = answer(get_statistic_graph(State(ledger.clone()), UrlQuery(request)).await.into_response()).await;
    assert_eq!(status, 400);
    assert!(
        message.contains("would have 3652059 points") && message.contains("ask for weeks or months"),
        "{}",
        message
    );

    // chrono's first date is outside the ledger's calendar
    let request = StatisticGraphRequest {
        from: "-262143-01-01".to_owned(),
        to: "-262143-01-10".to_owned(),
        interval: StatisticInterval::Week,
    };
    let (status, message) = answer(get_statistic_graph(State(ledger.clone()), UrlQuery(request)).await.into_response()).await;
    assert_eq!(
        (status, message.as_str()),
        (400, "a report covers the years 1 to 9999, not -262143-01-01 to -262143-01-10")
    );

    // by month the same range is fine
    let request = StatisticGraphRequest {
        from: "2025-01-01".to_owned(),
        to: "2025-12-31".to_owned(),
        interval: StatisticInterval::Month,
    };
    let (status, _) = answer(get_statistic_graph(State(ledger.clone()), UrlQuery(request)).await.into_response()).await;
    assert_eq!(status, 200);

    let request = StatisticRequest {
        from: "2025-04-01".to_owned(),
        to: "2025-04-30".to_owned(),
    };
    let response = get_statistic_rank_detail_by_account_type(State(ledger.clone()), UrlPath(("Foo".to_owned(),)), UrlQuery(request))
        .await
        .into_response();
    let (status, message) = answer(response).await;
    assert_eq!(status, 400);
    assert_eq!(
        message,
        "unknown account type \"Foo\": expected Assets, Liabilities, Equity, Income or Expenses"
    );
}
