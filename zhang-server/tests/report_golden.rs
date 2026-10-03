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
use zhang_server::builtin::{calculated_amount, LedgerDateRange};
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
        // source does not expand the wildcard includes of `wildcard-include-directive-supportted`
        Err(error) if SKIPPED.iter().any(|name| case.dir.ends_with(name)) => {
            eprintln!("skipped {}: {}", case.name, error);
            None
        }
        Err(error) => panic!("{} should load: {error}", case.name),
    }
}

/// The fixtures that do not load from a plain directory.
const SKIPPED: [&str; 2] = ["examples", "wildcard-include-directive-supportted"];

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

    /// Why the old and the new balance of `types` differ: the old one adds up the opened
    /// accounts until `old_until` and values them the old way at `old_at`; the new one adds up
    /// every account until the end of `day` and values them at `day`. Every step that changes
    /// the figure must be one of the reasons, and the old figure must be reproduced.
    fn balance_reasons(&self, types: &[AccountType], old: &Fig, new: &Fig, old_until: DateTime<Utc>, old_at: DateTime<Utc>, day: NaiveDate) -> Reasons {
        let timezone = self.timezone();
        let end_of_day = end_of(day, &timezone);
        let opened = self.accounts(types, true);
        let all = self.accounts(types, false);
        let mut reasons = Reasons::new();
        let old_units = engine_units(self.ledger, &opened, old_until);
        if old_units != old.units || old_value(self.ledger, old, old_at.with_timezone(&timezone)) != old.calculated {
            return Reasons::from([Reason::Unexplained]);
        }
        let opened_at_day = engine_units(self.ledger, &opened, end_of_day);
        if opened_at_day != old_units {
            reasons.insert(Reason::UtcCutOff);
        }
        if engine_units(self.ledger, &all, end_of_day) != new.units {
            return Reasons::from([Reason::Unexplained]);
        }
        if opened_at_day != new.units {
            reasons.insert(Reason::AccountsWithoutOpen);
        }
        reasons.extend(self.valuation_reasons(new, old_at, end_of_day, Reason::EndOfRangePrices));
        reasons
    }

    /// Why the new value of the new units differs from what the old code would give them at
    /// `old_at`: the old method at the new date (`date_reason`), or prices it does not use.
    fn valuation_reasons(&self, new: &Fig, old_at: DateTime<Utc>, new_at: DateTime<Utc>, date_reason: Reason) -> Reasons {
        let timezone = self.timezone();
        if old_value(self.ledger, new, old_at.with_timezone(&timezone)) == new.calculated {
            return Reasons::new();
        }
        if old_value(self.ledger, new, new_at.with_timezone(&timezone)) == new.calculated {
            Reasons::from([date_reason])
        } else if old_value(self.ledger, new, old_at.with_timezone(&timezone)) == old_value(self.ledger, new, new_at.with_timezone(&timezone)) {
            Reasons::from([Reason::PricePaths])
        } else {
            Reasons::from([date_reason, Reason::PricePaths])
        }
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
        let end_of_range = end_of(self.range.to, &self.timezone());
        for (item, old, new, types) in [
            (
                "balance",
                &old.balance,
                &new.balance,
                Some(&[AccountType::Assets, AccountType::Liabilities][..]),
            ),
            ("liability", &old.liability, &new.liability, Some(&[AccountType::Liabilities][..])),
            ("income", &old.income, &new.income, None),
            ("expense", &old.expense, &new.expense, None),
        ] {
            let (old, new) = (Fig::of(old), Fig::of(new));
            if old == new {
                continue;
            }
            let reasons = match types {
                Some(types) => self.balance_reasons(types, &old, &new, self.to, self.to, self.range.to),
                None if old.units == new.units => self.valuation_reasons(&new, self.to, end_of_range, Reason::Unexplained),
                None => Reasons::new(),
            };
            self.found("summary", item, old.show(), new.show(), reasons);
        }
    }

    fn graph(&mut self) {
        let timezone = self.timezone();
        let old = legacy::graph(self.ledger, self.from, self.to).unwrap();
        let new = report::graph(self.ledger, &self.range, &StatisticInterval::Day).unwrap();
        let offset = self.to.with_timezone(&timezone).naive_local() - self.to.naive_utc();

        let old_days: BTreeSet<NaiveDate> = old.balances.keys().copied().collect();
        let new_days: BTreeSet<NaiveDate> = new.balances.keys().copied().collect();
        if old_days != new_days {
            let span = |days: &BTreeSet<NaiveDate>| {
                format!(
                    "{}..{}",
                    days.first().map(|d| d.to_string()).unwrap_or_default(),
                    days.last().map(|d| d.to_string()).unwrap_or_default()
                )
            };
            // the old days are those of the range's instants in UTC
            let utc_days: BTreeSet<NaiveDate> =
                std::iter::successors(Some(self.from.date_naive()), |d| d.succ_opt().filter(|next| *next <= self.to.date_naive())).collect();
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
                let reasons = if old_fig.units == new_fig.units && old_value(self.ledger, &old_fig, old_at.with_timezone(&timezone)) == old_fig.calculated {
                    self.valuation_reasons(&new_fig, old_at, end_of(day, &timezone), Reason::UtcCutOff)
                } else {
                    Reasons::new()
                };
                self.found("graph", format!("changes/{}/{}", day, account_type), old_fig.show(), new_fig.show(), reasons);
            }
        }
    }

    fn rank(&mut self, account_type: AccountType) {
        let endpoint = format!("rank {}", account_type);
        let old = legacy::rank(self.ledger, account_type, self.from, self.to).unwrap();
        let new = report::rank(self.ledger, account_type, &self.range).unwrap();
        let end_of_range = end_of(self.range.to, &self.timezone());
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
            let reasons = if old_fig.units == new_fig.units {
                self.valuation_reasons(&new_fig, self.to, end_of_range, Reason::Unexplained)
            } else {
                Reasons::new()
            };
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
        if old_rows != new_rows {
            let operating = &self.ledger.options.operating_currency;
            let foreign = {
                let store = self.ledger.store.read().unwrap();
                store.postings.iter().any(|it| {
                    it.account.account_type == account_type
                        && it.trx_datetime >= self.from
                        && it.trx_datetime <= self.to
                        && &it.inferred_amount.commodity != operating
                })
            };
            let numbers = |rows: &[AccountJournalDomain]| rows.iter().map(|it| it.inferred_unit.number.normalized()).collect::<Vec<_>>();
            let reasons = if foreign {
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

/// Every ledger of the comparison, in UTC.
fn all_ledgers() -> Vec<(String, Ledger)> {
    fixtures()
        .into_iter()
        .filter_map(|case| load(&case, Some("UTC")).map(|ledger| (case.name, ledger)))
        .chain([("hand".to_owned(), hand_ledger())])
        .collect()
}

/// Each point of the daily graph is the net worth of that day valued at that day's prices, as a
/// query for that day alone computes it, whether the day has postings or is carried over.
#[test]
fn every_day_is_valued_at_its_own_prices() {
    let query = Query::compile_with_params(
        "SELECT units(sum(position)), convert(sum(position), :currency, :day) WHERE (under(account, 'Assets') OR under(account, 'Liabilities')) AND date <= :day",
        &ParamTypes::new().bind("currency", DataType::Str).bind("day", DataType::Date),
    )
    .unwrap();
    for (name, ledger) in all_ledgers() {
        let currency = ledger.options.operating_currency.clone();
        for (label, range, _) in ranges(&ledger).into_iter().filter(|(_, range, _)| (range.to - range.from).num_days() <= 100) {
            let graph = report::graph(&ledger, &range, &StatisticInterval::Day).unwrap();
            for (day, amount) in &graph.balances {
                let params = Params::new().bind("currency", currency.as_str()).bind("day", *day);
                let result = query.execute_at(&ledger, &params, Utc::now().date_naive()).unwrap();
                let expected = match result.rows.first() {
                    Some(row) => {
                        let inventory = |value: &zhang_query::Value| match value {
                            zhang_query::Value::Inventory(inventory) => inventory.clone(),
                            _ => zhang_query::Inventory::new(),
                        };
                        calculated_amount(&inventory(&row[0]), &inventory(&row[1]), &currency)
                    }
                    None => CalculatedAmount::new(&currency),
                };
                assert_eq!(Fig::of(amount), Fig::of(&expected), "{} {} on {}", name, label, day);
            }
        }
    }
}

/// A week or a month of the graph is the net worth on its last day in the range, and what it
/// changed by adds up the days of the range in it.
#[test]
fn weeks_and_months_add_up_the_days() {
    for (name, ledger) in all_ledgers() {
        for (label, range, _) in ranges(&ledger) {
            let daily = report::graph(&ledger, &range, &StatisticInterval::Day).unwrap();
            for interval in [StatisticInterval::Week, StatisticInterval::Month] {
                let graph = report::graph(&ledger, &range, &interval).unwrap();
                let starts: BTreeSet<NaiveDate> = graph.balances.keys().copied().collect();
                // every day of the range is in exactly one bucket, the last that starts on or before it
                let bucket_of = |day: &NaiveDate| {
                    *starts
                        .range(..=*day)
                        .next_back()
                        .unwrap_or_else(|| panic!("{} {} {:?}: no bucket for {}", name, label, interval, day))
                };
                let mut last_day: BTreeMap<NaiveDate, NaiveDate> = BTreeMap::new();
                let mut changes: BTreeMap<(NaiveDate, String), BTreeMap<String, BigDecimal>> = BTreeMap::new();
                for day in daily.balances.keys() {
                    let bucket = bucket_of(day);
                    let start_ok = match interval {
                        StatisticInterval::Week => bucket.weekday() == chrono::Weekday::Mon && (*day - bucket).num_days() < 7,
                        StatisticInterval::Month => bucket.day() == 1 && bucket.month() == day.month() && bucket.year() == day.year(),
                        StatisticInterval::Day => unreachable!(),
                    };
                    assert!(start_ok, "{} {} {:?}: {} in the bucket of {}", name, label, interval, day, bucket);
                    let last = last_day.entry(bucket).or_insert(*day);
                    *last = (*last).max(*day);
                    for (account_type, amount) in daily.changes.get(day).into_iter().flatten() {
                        let sum = changes.entry((bucket, account_type.to_string())).or_default();
                        for (currency, number) in &amount.detail {
                            *sum.entry(currency.clone()).or_default() += number;
                        }
                    }
                }
                assert_eq!(starts, last_day.keys().copied().collect(), "{} {} {:?}: buckets", name, label, interval);
                for (bucket, last) in last_day {
                    assert_eq!(
                        Fig::of(&graph.balances[&bucket]),
                        Fig::of(&daily.balances[&last]),
                        "{} {} {:?}: {}",
                        name,
                        label,
                        interval,
                        bucket
                    );
                }
                let actual: BTreeMap<(NaiveDate, String), BTreeMap<String, BigDecimal>> = graph
                    .changes
                    .iter()
                    .flat_map(|(bucket, by_type)| {
                        by_type
                            .iter()
                            .map(move |(account_type, amount)| ((*bucket, account_type.to_string()), Fig::of(amount).units))
                    })
                    .collect();
                let expected: BTreeMap<_, _> = changes.into_iter().map(|(key, sum)| (key, units(sum))).collect();
                assert_eq!(actual, expected, "{} {} {:?}: changes", name, label, interval);
            }
        }
    }
}
