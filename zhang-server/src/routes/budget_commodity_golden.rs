//! Golden tests of the budget endpoints: on every ledger of `integration-tests/` and `examples/`,
//! in both formats where there are both, every figure and posting the handlers answer is the
//! independent computation's ([`Reference`]); and small hand-worked ledgers (#479).

use std::collections::BTreeSet;
use std::future::Future;
use std::path::{Path as FsPath, PathBuf};
use std::str::FromStr;
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::response::IntoResponse;
use bigdecimal::{BigDecimal, Zero};
use chrono::{Datelike, Months, NaiveDate};
use serde::Serialize;
use serde_json::Value as Json;
use tokio::sync::RwLock;
use zhang_core::clock::Clock;
use zhang_core::data_source::LocalFileSystemDataSource;
use zhang_core::data_type::text::ZhangDataType;
use zhang_core::ledger::Ledger;

use super::budget_reference::{Figures, Reference};
use crate::request::{BudgetIntervalDetailRequest, BudgetListRequest};
use crate::response::ResponseWrapper;
use crate::routes::budget;
use crate::state::SharedLedger;
use crate::ServerResult;

/// What an endpoint answered.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Outcome {
    Json(Json),
    Status(u16),
    Panic,
}

/// Run a handler, catching a panic as [`Outcome::Panic`].
async fn call<T, F>(handler: F) -> Outcome
where
    T: Serialize + gotcha::Schematic + Send + 'static,
    F: Future<Output = ServerResult<ResponseWrapper<T>>> + Send + 'static,
{
    match tokio::spawn(handler).await {
        Err(_) => Outcome::Panic,
        Ok(Ok(wrapper)) => Outcome::Json(serde_json::to_value(&wrapper.data).expect("serializable")),
        Ok(Err(err)) => Outcome::Status(err.into_response().status().as_u16()),
    }
}

/// The endpoints tested, with their arguments.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
#[allow(clippy::enum_variant_names)]
pub(crate) enum Probe {
    BudgetList { month: Option<(u32, u32)> },
    BudgetInfo { name: String, month: Option<(u32, u32)> },
    BudgetInterval { name: String, year: u32, month: u32 },
}

impl Probe {
    fn month_of(month: &Option<(u32, u32)>) -> Option<NaiveDate> {
        month.and_then(|(year, month)| NaiveDate::from_ymd_opt(year as i32, month, 1))
    }

    /// the month the probe asks about, if it names one
    pub(crate) fn month(&self) -> Option<NaiveDate> {
        match self {
            Probe::BudgetList { month } | Probe::BudgetInfo { month, .. } => Probe::month_of(month),
            Probe::BudgetInterval { year, month, .. } => NaiveDate::from_ymd_opt(*year as i32, *month, 1),
        }
    }

    fn request(month: &Option<(u32, u32)>) -> Query<BudgetListRequest> {
        Query(BudgetListRequest {
            year: month.map(|it| it.0),
            month: month.map(|it| it.1),
        })
    }

    /// the handler's answer
    pub(crate) async fn run(&self, ledger: &SharedLedger) -> Outcome {
        let state = State(SharedLedger(ledger.0.clone()));
        match self.clone() {
            Probe::BudgetList { month } => call(budget::get_budget_list(state, Probe::request(&month))).await,
            Probe::BudgetInfo { name, month } => call(budget::get_budget_info(state, Path((name,)), Probe::request(&month))).await,
            Probe::BudgetInterval { name, year, month } => {
                let path = Path(BudgetIntervalDetailRequest {
                    budget_name: name,
                    year,
                    month,
                });
                call(budget::get_budget_interval_detail(state, path)).await
            }
        }
    }
}

/// A ledger to test on: its directory and main file.
#[derive(Debug, Clone)]
pub(crate) struct Fixture {
    pub(crate) dir: PathBuf,
    pub(crate) entry: String,
}

impl Fixture {
    pub(crate) fn name(&self) -> String {
        let dir = self.dir.file_name().map(|it| it.to_string_lossy().to_string()).unwrap_or_default();
        format!("{}/{}", dir, self.entry)
    }

    /// the ledger, or `None` if it cannot be loaded. The local data source does not expand wildcard
    /// includes and fails on a missing include, so a ledger that has either is loaded from a copy
    /// with its wildcards expanded and its missing includes empty ([`materialize`])
    pub(crate) async fn try_load(&self) -> Option<SharedLedger> {
        self.try_load_at(Clock::System).await
    }

    /// [`Fixture::try_load`] with the current time read from `clock`.
    pub(crate) async fn try_load_at(&self, clock: Clock) -> Option<SharedLedger> {
        let load = |dir: PathBuf| async move {
            let source: Arc<dyn zhang_core::data_source::DataSource> = if self.entry.ends_with(".bean") {
                Arc::new(LocalFileSystemDataSource::new(beancount::Beancount {}))
            } else {
                Arc::new(LocalFileSystemDataSource::new(ZhangDataType {}))
            };
            Ledger::load_with_clock(dir, self.entry.clone(), source, clock).ok()
        };
        let ledger = match load(self.dir.clone()).await {
            Some(ledger) => ledger,
            None => load(materialize(&self.dir)).await?,
        };
        Some(SharedLedger(Arc::new(RwLock::new(ledger))))
    }
}

/// Whether `name` matches `pattern`, whose `*` match any run of characters.
fn wildcard_match(pattern: &str, name: &str) -> bool {
    match pattern.split_once('*') {
        None => pattern == name,
        Some((prefix, rest)) => {
            name.starts_with(prefix)
                && (0..=name.len() - prefix.len()).any(|skip| name.is_char_boundary(prefix.len() + skip) && wildcard_match(rest, &name[prefix.len() + skip..]))
        }
    }
}

/// The files under `dir` that the include pattern matches, as paths relative to `dir`, sorted.
fn expand(dir: &FsPath, pattern: &str) -> Vec<String> {
    let mut matches = vec![String::new()];
    for segment in pattern.split('/') {
        matches = matches
            .into_iter()
            .flat_map(|base: String| {
                let path = dir.join(&base);
                let mut names = std::fs::read_dir(&path)
                    .map(|entries| {
                        entries
                            .filter_map(|it| it.ok())
                            .map(|it| it.file_name().to_string_lossy().to_string())
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                names.sort();
                names
                    .into_iter()
                    .filter(|name| wildcard_match(segment, name))
                    .map(|name| if base.is_empty() { name } else { format!("{}/{}", base, name) })
                    .collect::<Vec<_>>()
            })
            .collect();
    }
    matches.into_iter().filter(|it| dir.join(it).is_file()).collect()
}

/// A copy of the ledger directory `dir` in which every wildcard include lists the files it
/// matches, and every missing included file exists, empty.
fn materialize(dir: &FsPath) -> PathBuf {
    fn copy(from: &FsPath, to: &FsPath, files: &mut Vec<PathBuf>) {
        std::fs::create_dir_all(to).unwrap();
        for entry in std::fs::read_dir(from).unwrap().filter_map(|it| it.ok()) {
            let target = to.join(entry.file_name());
            if entry.path().is_dir() {
                copy(&entry.path(), &target, files);
            } else {
                std::fs::copy(entry.path(), &target).unwrap();
                files.push(target);
            }
        }
    }
    let target = tempfile::tempdir().unwrap().keep();
    let mut files = vec![];
    copy(dir, &target, &mut files);
    for file in files.into_iter().filter(|it| it.extension().is_some_and(|ext| ext == "zhang" || ext == "bean")) {
        let base = file.parent().unwrap().to_path_buf();
        let text = std::fs::read_to_string(&file).unwrap();
        let lines = text
            .lines()
            .flat_map(|line| {
                let pattern = line.trim().strip_prefix("include \"").and_then(|it| it.strip_suffix('"'));
                match pattern {
                    Some(pattern) if pattern.contains('*') => expand(&base, pattern).into_iter().map(|it| format!("include \"{}\"", it)).collect(),
                    Some(pattern) => {
                        let included = base.join(pattern);
                        if !included.exists() {
                            std::fs::create_dir_all(included.parent().unwrap()).unwrap();
                            std::fs::write(&included, "").unwrap();
                        }
                        vec![line.to_owned()]
                    }
                    None => vec![line.to_owned()],
                }
            })
            .collect::<Vec<_>>();
        std::fs::write(&file, lines.join("\n") + "\n").unwrap();
    }
    target
}

/// The main files of the ledgers in `dir`'s sub-directories (or of `dir` itself).
pub(crate) fn fixtures_in(dir: &FsPath) -> Vec<Fixture> {
    let mut dirs = vec![dir.to_path_buf()];
    if let Ok(entries) = std::fs::read_dir(dir) {
        let mut subs = entries
            .filter_map(|it| it.ok())
            .map(|it| it.path())
            .filter(|it| it.is_dir())
            .collect::<Vec<_>>();
        subs.sort();
        dirs.extend(subs);
    }
    dirs.into_iter()
        .flat_map(|dir| {
            ["main.zhang", "main.bean"]
                .into_iter()
                .filter(|entry| dir.join(entry).is_file())
                .map(|entry| Fixture {
                    dir: dir.clone(),
                    entry: entry.to_owned(),
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

/// Every ledger of `integration-tests/` and `examples/`.
pub(crate) fn repository_fixtures() -> Vec<Fixture> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut fixtures = fixtures_in(&root.join("integration-tests"));
    fixtures.extend(fixtures_in(&root.join("examples")));
    fixtures
}

/// The budget the probes ask about that no ledger defines.
const UNKNOWN_BUDGET: &str = "no-such-budget";

fn month_pair(date: NaiveDate) -> (u32, u32) {
    (date.year() as u32, date.month())
}

/// The probes of a ledger: every endpoint, for every budget, an unknown one, and
/// the months around the budgets' series: before the first, every month of it (sampled when it is
/// long), and after the last. A series that runs years past the current month, because of a date
/// typo, is probed up to a few months after the current one, and at its last month.
pub(crate) async fn probes(ledger: &SharedLedger) -> Vec<Probe> {
    let ledger = ledger.read().await;
    let store = ledger.store.read().unwrap();
    // Discover the probe domain from the fixture's directives, independently of the engine.
    let budgets = ledger
        .directives
        .iter()
        .filter_map(|directive| match &directive.data {
            zhang_ast::Directive::Budget(budget) => Some(budget.name.clone()),
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    let budget_dates = ledger
        .directives
        .iter()
        .filter_map(|directive| match &directive.data {
            zhang_ast::Directive::Budget(budget) => Some(budget.date.naive_date()),
            zhang_ast::Directive::BudgetAdd(add) => Some(add.date.naive_date()),
            zhang_ast::Directive::BudgetTransfer(transfer) => Some(transfer.date.naive_date()),
            zhang_ast::Directive::BudgetClose(close) => Some(close.date.naive_date()),
            _ => None,
        })
        .collect::<Vec<_>>();
    let first = budget_dates.iter().min().copied();
    let last_detail = budget_dates.iter().max().copied();
    let mut months = BTreeSet::new();
    let has_postings = |txn: &&zhang_core::store::TransactionDomain| matches!(&ledger.directives[txn.directive].data, zhang_ast::Directive::Transaction(it) if !it.postings.is_empty());
    let last_posting = store.transactions.values().filter(has_postings).map(|it| it.datetime.date_naive()).max();
    if let (Some(first), Some(last)) = (first, last_detail) {
        let first = first.with_day(1).unwrap();
        let last = last.with_day(1).unwrap();
        let last = last_posting.map(|it| it.with_day(1).unwrap()).map_or(last, |it| it.max(last));
        let start = first - Months::new(1);
        let end = last + Months::new(2);
        let current = ledger.today().with_day(1).unwrap();
        let typo = end > current + Months::new(36);
        if typo {
            months.insert(month_pair(last));
        }
        let end = if typo { current + Months::new(3) } else { end };
        let all = std::iter::successors(Some(start), |it| Some(*it + Months::new(1)))
            .take_while(|it| *it <= end)
            .collect::<Vec<_>>();
        let count = all.len();
        for (idx, month) in all.into_iter().enumerate() {
            if count <= 40 || idx < 3 || idx + 4 >= count || idx % 6 == 0 {
                months.insert(month_pair(month));
            }
        }
    }
    let mut probes = vec![Probe::BudgetList { month: None }];
    probes.extend(months.iter().map(|month| Probe::BudgetList { month: Some(*month) }));
    for name in budgets.iter().cloned().chain(std::iter::once(UNKNOWN_BUDGET.to_owned())) {
        probes.push(Probe::BudgetInfo {
            name: name.clone(),
            month: None,
        });
        for month in &months {
            probes.push(Probe::BudgetInfo {
                name: name.clone(),
                month: Some(*month),
            });
            probes.push(Probe::BudgetInterval {
                name: name.clone(),
                year: month.0,
                month: month.1,
            });
        }
    }
    probes
}

/// What the checks need to know of a ledger.
pub(crate) struct Context {
    /// the figures of the budgets as [`Reference`] computes them, independently of the handlers,
    /// or why it cannot
    reference: Result<Reference, String>,
    /// the current month in the ledger's timezone
    current_month: NaiveDate,
}

impl Context {
    pub(crate) async fn of(ledger: &SharedLedger) -> Context {
        let ledger = ledger.read().await;
        Context {
            reference: Reference::of(&ledger),
            current_month: ledger.today().with_day(1).unwrap(),
        }
    }
}

/// The number of an amount of the budget API, as a decimal rounded to 20 places: the reference
/// divides with more digits than the engine's 28 significant ones.
fn amount_number(json: &Json) -> Option<BigDecimal> {
    let number = BigDecimal::from_str(json["number"].as_str()?).ok()?;
    Some(number.with_scale_round(20, bigdecimal::RoundingMode::HalfEven))
}

/// Whether a budget's figures of the handler (an object of `GET /api/budgets` or
/// `GET /api/budgets/{name}`) are the independent computation's ([`Reference`]): the same
/// amounts by value and the same `closed`. A month before the budget's first is all zero and
/// open.
fn check_reference(new: &Json, name: &str, month: NaiveDate, context: &Context) -> Result<(), String> {
    let reference = context.reference.as_ref().map_err(|why| format!("no reference figures: {}", why))?;
    let expected = reference.figures(name, month).unwrap_or(Figures {
        assigned: BigDecimal::zero(),
        activity: BigDecimal::zero(),
        available: BigDecimal::zero(),
        closed: false,
    });
    let round = |number: BigDecimal| number.with_scale_round(20, bigdecimal::RoundingMode::HalfEven);
    let got = (
        amount_number(&new["assigned_amount"]),
        amount_number(&new["activity_amount"]),
        amount_number(&new["available_amount"]),
        new["closed"].as_bool(),
    );
    let want = (
        Some(round(expected.assigned)),
        Some(round(expected.activity)),
        Some(round(expected.available)),
        Some(expected.closed),
    );
    if got == want {
        Ok(())
    } else {
        Err(format!(
            "{} in {}: the handler gives {:?}, the reference {:?}",
            name,
            month.format("%Y-%m"),
            got,
            want
        ))
    }
}

/// Whether the postings of a month's detail (of the handler) are the reference's postings of
/// the budget in the month: those of the accounts whose `open` in effect at their date names it,
/// each with the units it books and its account's balance after it in ledger order.
fn check_postings(events: &[Json], name: &str, month: NaiveDate, context: &Context) -> Result<(), String> {
    let reference = context.reference.as_ref().map_err(|why| format!("no reference postings: {}", why))?;
    let number = |json: &Json| json["number"].as_str().and_then(|it| BigDecimal::from_str(it).ok());
    let mut got = events
        .iter()
        .filter(|it| it["type"] == "Posting")
        .map(|event| {
            (
                event["account"].as_str().unwrap_or_default().to_owned(),
                event["narration"].as_str().map(str::to_owned),
                number(&event["inferred_unit"]),
                event["inferred_unit"]["commodity"].as_str().unwrap_or_default().to_owned(),
                number(&event["account_after"]),
            )
        })
        .collect::<Vec<_>>();
    let mut want = reference
        .month_postings(name, month)
        .into_iter()
        .map(|posting| {
            (
                posting.account.clone(),
                posting.narration.clone(),
                Some(posting.units.number.clone()),
                posting.units.commodity.clone(),
                Some(posting.after.clone()),
            )
        })
        .collect::<Vec<_>>();
    got.sort();
    want.sort();
    if got == want {
        Ok(())
    } else {
        Err(format!(
            "{} in {}: the handler lists the postings {:?}, the reference {:?}",
            name,
            month.format("%Y-%m"),
            got,
            want
        ))
    }
}

/// Whether the handler's answer to `probe` is right: its figures and postings are the independent
/// computation's, an unknown budget is a 404, and a month so far ahead (a date typo) that the
/// budgets' months up to it are more than the result size limit is a 400.
pub(crate) fn check(probe: &Probe, outcome: &Outcome, context: &Context) -> Result<(), String> {
    let month = probe.month().unwrap_or(context.current_month);
    match (probe, outcome) {
        (Probe::BudgetList { .. }, Outcome::Json(Json::Array(budgets))) => {
            for budget in budgets {
                check_reference(budget, budget["name"].as_str().unwrap_or_default(), month, context)?;
            }
            Ok(())
        }
        (Probe::BudgetInfo { name, .. }, Outcome::Json(budget)) if name != UNKNOWN_BUDGET => check_reference(budget, name, month, context),
        (Probe::BudgetInterval { name, .. }, Outcome::Json(Json::Array(events))) if name != UNKNOWN_BUDGET => check_postings(events, name, month, context),
        (Probe::BudgetInfo { name, .. } | Probe::BudgetInterval { name, .. }, Outcome::Status(404)) if name == UNKNOWN_BUDGET => Ok(()),
        (Probe::BudgetList { .. } | Probe::BudgetInfo { .. }, Outcome::Status(400))
            if context
                .reference
                .as_ref()
                .is_ok_and(|reference| reference.months_until(month) > crate::routes::query::max_result_values()) =>
        {
            Ok(())
        }
        _ => Err(format!("unexpected answer {:?}", outcome)),
    }
}

#[cfg(test)]
mod test {
    use super::*;

    /// Every answer of the handlers on the repository's ledgers is right ([`check`]).
    #[tokio::test(flavor = "multi_thread")]
    async fn the_handlers_answer_the_reference_figures_on_every_ledger() {
        let mut wrong = vec![];
        for fixture in repository_fixtures() {
            let Some(ledger) = fixture.try_load().await else { continue };
            let context = Context::of(&ledger).await;
            for probe in probes(&ledger).await {
                if let Err(why) = check(&probe, &probe.run(&ledger).await, &context) {
                    wrong.push(format!("{} {:?}: {}", fixture.name(), probe, why));
                }
            }
        }
        assert!(wrong.is_empty(), "wrong answers:\n{}", wrong.join("\n"));
    }
}

/// Small ledgers whose figures are worked out by hand, and what the handlers answer.
#[cfg(test)]
mod worked_examples {
    use serde_json::json;

    use super::*;

    /// The ledger of `text` on 2025-06-15 (UTC): "this month" is June 2025.
    pub(super) async fn ledger_of(text: &str) -> SharedLedger {
        ledger_at(text, "2025-06-15T04:00:00Z").await
    }

    /// The ledger of `text` with its clock pinned at `instant` (RFC 3339).
    pub(super) async fn ledger_at(text: &str, instant: &str) -> SharedLedger {
        let dir = tempfile::tempdir().unwrap().keep();
        std::fs::write(dir.join("main.zhang"), text).unwrap();
        let instant = chrono::DateTime::parse_from_rfc3339(instant).unwrap().to_utc();
        Fixture {
            dir,
            entry: "main.zhang".to_owned(),
        }
        .try_load_at(Clock::Fixed(instant))
        .await
        .expect("the ledger loads")
    }

    pub(super) async fn json_answer(ledger: &SharedLedger, probe: Probe) -> Json {
        match probe.run(ledger).await {
            Outcome::Json(json) => json,
            other => panic!("{:?}: {:?}", probe, other),
        }
    }

    /// Asia/Shanghai is UTC+8. `trip` and `food` start in March 2025 with 1000 and 500 CNY.
    /// - The flight of 2025-04-02 costs 300 USD: 2100.0 CNY at the price of 2025-03-31 (7.0),
    ///   the latest one on that date. With the 40 CNY taxi, April's activity is 2140.0 CNY, and
    ///   with the 200 CNY added on 2025-04-10 April starts with 1200 CNY, so 1200 - 2140.0 =
    ///   -940.0 CNY is left.
    /// - `trip` is closed on 2025-05-02: closed from May, not before.
    /// - The budget-add of 2025-04-10 (00:00 +08:00, Unix 1744214400) is after the taxi of
    ///   2025-04-09 20:00 +08:00 (1744200000).
    pub(super) const BUDGETS: &str = r#"
option "operating_currency" "CNY"
option "timezone" "Asia/Shanghai"

1970-01-01 commodity CNY
1970-01-01 commodity USD

1970-01-01 open Assets:Bank
1970-01-01 open Assets:USBank
1970-01-01 open Expenses:Travel
  budget: trip
1970-01-01 open Expenses:Food
  budget: food
1970-01-01 open Expenses:Dining
  budget: food

2025-03-01 budget trip CNY
2025-03-01 budget food CNY
  alias: "Groceries"
2025-03-01 budget-add trip 1000 CNY
2025-03-01 budget-add food 500 CNY

2025-03-31 price USD 7.0 CNY
2025-04-30 price USD 7.3 CNY

2025-03-15 "Bistro" "dinner"
  Expenses:Dining 60 CNY
  Assets:Bank

2025-04-02 "Airline" "flight, in dollars"
  Expenses:Travel 300 USD
  Assets:USBank

2025-04-09 20:00:00 "Taxi" "late taxi"
  Expenses:Travel 40 CNY
  Assets:Bank

2025-04-10 budget-add trip 200 CNY

2025-05-02 budget-close trip
"#;

    /// The figures of a budget of the budget API, its numbers by value (`-940.0` is `-940`).
    pub(super) fn figures(json: &Json) -> (String, String, String, bool) {
        let number = |key: &str| {
            let number = BigDecimal::from_str(json[key]["number"].as_str().unwrap()).unwrap();
            zhang_query::decimal::to_plain_string(&number.normalized())
        };
        (
            number("assigned_amount"),
            number("activity_amount"),
            number("available_amount"),
            json["closed"].as_bool().unwrap(),
        )
    }

    pub(super) fn of(assigned: &str, activity: &str, available: &str, closed: bool) -> (String, String, String, bool) {
        (assigned.to_owned(), activity.to_owned(), available.to_owned(), closed)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn budget_activity_is_converted_and_closed_is_per_month() {
        let ledger = ledger_of(BUDGETS).await;
        let info = |month: (u32, u32)| Probe::BudgetInfo {
            name: "trip".to_owned(),
            month: Some(month),
        };
        // before the budget starts: nothing, and not closed yet
        let new = json_answer(&ledger, info((2025, 2))).await;
        assert_eq!(figures(&new), of("0", "0", "0", false));
        let new = json_answer(&ledger, info((2025, 3))).await;
        assert_eq!(figures(&new), of("1000", "0", "1000", false));
        let new = json_answer(&ledger, info((2025, 4))).await;
        assert_eq!(figures(&new), of("1200", "2140", "-940", false));
        let new = json_answer(&ledger, info((2025, 5))).await;
        assert_eq!(figures(&new), of("-940", "0", "-940", true));
        // after the current month (June 2025), the budget carries over
        let new = json_answer(&ledger, info((2026, 1))).await;
        assert_eq!(figures(&new), of("-940", "0", "-940", true));
        assert_eq!(new["related_accounts"], json!(["Expenses:Travel"]));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn budgets_are_listed_by_name_and_their_accounts_sorted() {
        let ledger = ledger_of(BUDGETS).await;
        let new = json_answer(&ledger, Probe::BudgetList { month: Some((2025, 4)) }).await;
        let names = new.as_array().unwrap().iter().map(|it| it["name"].clone()).collect::<Vec<_>>();
        assert_eq!(names, vec![json!("food"), json!("trip")]);
        assert_eq!(new[0]["alias"], json!("Groceries"));
        // food spent 60 CNY in March and nothing in April
        assert_eq!(figures(&new[0]), of("440", "0", "440", false));
        assert_eq!(figures(&new[1]), of("1200", "2140", "-940", false));
        // a month before every budget lists none
        let new = json_answer(&ledger, Probe::BudgetList { month: Some((2025, 2)) }).await;
        assert_eq!(new, json!([]));

        let new = json_answer(
            &ledger,
            Probe::BudgetInfo {
                name: "food".to_owned(),
                month: Some((2025, 3)),
            },
        )
        .await;
        assert_eq!(new["related_accounts"], json!(["Expenses:Dining", "Expenses:Food"]));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_months_events_are_ordered_by_their_time() {
        let ledger = ledger_of(BUDGETS).await;
        let probe = Probe::BudgetInterval {
            name: "trip".to_owned(),
            year: 2025,
            month: 4,
        };
        let new = json_answer(&ledger, probe).await;
        let summary = |json: &Json| {
            json.as_array()
                .unwrap()
                .iter()
                .map(|it| {
                    let what = match it["type"].as_str().unwrap() {
                        "BudgetEvent" => format!("{} {}", it["event_type"].as_str().unwrap(), it["amount"]["number"].as_str().unwrap()),
                        _ => format!("{} {}", it["narration"].as_str().unwrap(), it["inferred_unit"]["number"].as_str().unwrap()),
                    };
                    (it["timestamp"].as_i64().unwrap(), what)
                })
                .collect::<Vec<_>>()
        };
        let add = (1744214400, "AddAssignedAmount 200".to_owned());
        let taxi = (1744200000, "late taxi 40".to_owned());
        let flight = (1743523200, "flight, in dollars 300".to_owned());
        assert_eq!(summary(&new), vec![add, taxi, flight]);
        // the posting's local time, and its account's balance in its currency after it
        assert_eq!(new[1]["datetime"], json!("2025-04-09T20:00:00"));
        assert_eq!(new[1]["account_after"], json!({"number": "40", "commodity": "CNY"}));
        assert_eq!(new[2]["account_after"], json!({"number": "300", "commodity": "USD"}));
        // an unknown budget, and a month that does not exist
        let unknown = Probe::BudgetInterval {
            name: "nope".to_owned(),
            year: 2025,
            month: 4,
        };
        assert_eq!(unknown.run(&ledger).await, Outcome::Status(404));
        let response = budget::get_budget_interval_detail(
            State(SharedLedger(ledger.0.clone())),
            Path(BudgetIntervalDetailRequest {
                budget_name: "trip".to_owned(),
                year: 2025,
                month: 13,
            }),
        )
        .await;
        assert_eq!(response.err().map(|err| err.into_response().status().as_u16()), Some(400));
    }

    /// The ledger of #499, with a budget-close at a time on another budget.
    pub(super) const CLOSED_499: &str = r#"
option "operating_currency" "CNY"
1970-01-01 commodity CNY
1970-01-01 commodity USD
2024-01-01 open Assets:Cash
2024-01-01 open Expenses:Food
  budget: "Food"
2024-01-01 open Expenses:Fun
  budget: "Fun"

2024-01-01 budget Food CNY
2024-01-01 budget-add Food 1000 CNY
2024-01-02 budget-add Food 50 USD
2024-01-01 budget Fun CNY

2024-01-10 * "Lunch in CNY"
  Expenses:Food 100 CNY
  Assets:Cash

2024-01-11 * "Lunch abroad in USD"
  Expenses:Food 20 USD
  Assets:Cash

2024-03-01 budget-close Food

2024-04-05 * "Lunch after the budget is closed"
  Expenses:Food 30 CNY
  Assets:Cash

2024-04-10 12:00:00 budget-close Fun
2024-04-10 11:00:00 * "Cinema before the close"
  Expenses:Fun 5 CNY
  Assets:Cash
2024-04-10 13:00:00 * "Cinema after the close"
  Expenses:Fun 7 CNY
  Assets:Cash
"#;

    /// #499: the budget API never adds an amount in another commodity as a number, shows a budget
    /// open before its close, and counts and lists no posting after the close.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_closed_budget_takes_no_activity_after_its_close() {
        let ledger = ledger_of(CLOSED_499).await;
        let months = [
            (1, of("1000", "100", "900", false)),
            (2, of("900", "0", "900", false)),
            (3, of("900", "0", "900", true)),
            // the lunch of April is after the close
            (4, of("900", "0", "900", true)),
        ];
        for (month, expected) in months {
            let list = json_answer(&ledger, Probe::BudgetList { month: Some((2024, month)) }).await;
            let food = list.as_array().unwrap().iter().find(|it| it["name"] == "Food").unwrap();
            assert_eq!(food["assigned_amount"]["commodity"], "CNY");
            assert_eq!(figures(food), expected, "month {}", month);
        }
        let detail = |name: &str, month: u32| Probe::BudgetInterval {
            name: name.to_owned(),
            year: 2024,
            month,
        };
        let narrations = |json: &Json| {
            json.as_array()
                .unwrap()
                .iter()
                .filter_map(|it| it["narration"].as_str().map(str::to_owned))
                .collect::<Vec<_>>()
        };
        assert!(narrations(&json_answer(&ledger, detail("Food", 4)).await).is_empty());
        // the lunch in USD, which no price converts, does not count, so it is not listed either
        assert_eq!(narrations(&json_answer(&ledger, detail("Food", 1)).await), vec!["Lunch in CNY"]);
        // a budget-close at a time closes the budget at that time
        let info = json_answer(
            &ledger,
            Probe::BudgetInfo {
                name: "Fun".to_owned(),
                month: Some((2024, 4)),
            },
        )
        .await;
        assert_eq!(figures(&info), of("0", "5", "-5", true));
        assert_eq!(narrations(&json_answer(&ledger, detail("Fun", 4)).await), vec!["Cinema before the close"]);
    }

    /// 15 AAPL bought at 100 and 120 USD, 4 sold from the first lot; a loan in Liabilities. The
    /// latest USD price of AAPL is the 125 USD quoted at 10:00 on 2024-03-01.
    const COMMODITIES: &str = r#"
option "operating_currency" "USD"

1970-01-01 commodity USD
1970-01-01 commodity CNY
1970-01-01 commodity AAPL
  group: "Stocks"

1970-01-01 open Assets:Broker
1970-01-01 open Assets:Cash
1970-01-01 open Liabilities:Loan
1970-01-01 open Income:Gains

2024-01-02 "Broker" "buy"
  Assets:Broker 10 AAPL {100 USD}
  Assets:Cash -1000 USD

2024-02-01 "Broker" "buy more"
  Assets:Broker 5 AAPL {120 USD, 2024-02-01}
  Assets:Cash -600 USD

2024-03-01 "Broker" "sell"
  Assets:Broker -4 AAPL {100 USD}
  Assets:Cash 480 USD
  Income:Gains -80 USD

2024-04-01 "Bank" "loan"
  Assets:Cash 5000 USD
  Liabilities:Loan -5000 USD

2024-01-15 price AAPL 110 USD
2024-03-01 10:00:00 price AAPL 125 USD
2024-03-01 price AAPL 900 CNY
"#;

    /// What `GET /api/commodities/{name}` answers.
    async fn commodity(ledger: &SharedLedger, name: &str) -> Outcome {
        let state = State(SharedLedger(ledger.0.clone()));
        call(crate::routes::commodity::get_single_commodity(state, Path((name.to_owned(),)))).await
    }

    async fn commodity_json(ledger: &SharedLedger, name: &str) -> Json {
        match commodity(ledger, name).await {
            Outcome::Json(json) => json,
            other => panic!("{}: {:?}", name, other),
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn commodity_lots_are_assets_and_liabilities_holdings_in_order() {
        let ledger = ledger_of(COMMODITIES).await;
        let new = commodity_json(&ledger, "AAPL").await;
        assert_eq!(
            new["info"],
            json!({
                "name": "AAPL", "precision": 2, "prefix": null, "suffix": null, "rounding": "RoundDown", "group": "Stocks",
                "total_amount": "11",
                "latest_price_date": "2024-03-01T10:00:00", "latest_price_amount": "125", "latest_price_commodity": "USD"
            })
        );
        assert_eq!(
            new["lots"],
            json!([
                {"account": "Assets:Broker", "amount": "6", "cost": {"number": "100", "commodity": "USD"}, "price": null, "acquisition_date": "2024-01-02"},
                {"account": "Assets:Broker", "amount": "5", "cost": {"number": "120", "commodity": "USD"}, "price": null, "acquisition_date": "2024-02-01"},
            ])
        );
        assert_eq!(
            new["prices"],
            json!([
                {"datetime": "2024-01-15T00:00:00", "amount": {"number": "110", "commodity": "USD"}},
                {"datetime": "2024-03-01T00:00:00", "amount": {"number": "900", "commodity": "CNY"}},
                {"datetime": "2024-03-01T10:00:00", "amount": {"number": "125", "commodity": "USD"}},
            ])
        );

        // Income:Gains is not a holding of USD
        let new = commodity_json(&ledger, "USD").await;
        assert_eq!(
            new["lots"],
            json!([
                {"account": "Assets:Cash", "amount": "3880", "cost": null, "price": null, "acquisition_date": null},
                {"account": "Liabilities:Loan", "amount": "-5000", "cost": null, "price": null, "acquisition_date": null},
            ])
        );
        assert_eq!(new["info"]["total_amount"], json!("-1120"));

        // an unknown commodity
        assert_eq!(commodity(&ledger, "NOPE").await, Outcome::Status(404));
    }

    /// Lots that differ only by label are the store's lots (#498): the page keeps them apart too.
    #[tokio::test(flavor = "multi_thread")]
    async fn commodity_lots_keep_labelled_lots_apart() {
        let ledger = ledger_of(
            r#"
option "operating_currency" "USD"
1970-01-01 commodity USD
1970-01-01 commodity AAPL
1970-01-01 open Assets:Broker
1970-01-01 open Assets:Cash
1970-01-01 open Income:Gains

2024-01-02 "Broker" "buy"
  Assets:Broker 10 AAPL {100 USD, "a"}
  Assets:Broker 10 AAPL {100 USD, "b"}
  Assets:Broker 10 AAPL {100 USD}
  Assets:Cash -3000 USD

2024-03-01 "Broker" "sell from b"
  Assets:Broker -4 AAPL {, "b"}
  Assets:Cash 480 USD
  Income:Gains -80 USD
"#,
        )
        .await;
        let new = commodity_json(&ledger, "AAPL").await;
        assert_eq!(
            new["lots"],
            json!([
                {"account": "Assets:Broker", "amount": "10", "cost": {"number": "100", "commodity": "USD"}, "price": null, "acquisition_date": "2024-01-02"},
                {"account": "Assets:Broker", "amount": "10", "cost": {"number": "100", "commodity": "USD"}, "price": null, "acquisition_date": "2024-01-02", "label": "a"},
                {"account": "Assets:Broker", "amount": "6", "cost": {"number": "100", "commodity": "USD"}, "price": null, "acquisition_date": "2024-01-02", "label": "b"},
            ])
        );
        let new = commodity_json(&ledger, "USD").await;
        assert_eq!(
            new["lots"],
            json!([
                {"account": "Assets:Cash", "amount": "-2520", "cost": null, "price": null, "acquisition_date": null},
            ])
        );
    }
}

/// The budget pages with the ledger's clock pinned: the current month in the ledger's timezone,
/// a month after the last row, the pages against the queries they open, and a date typo.
#[cfg(test)]
mod fixed_clock {
    use std::collections::HashMap;

    use serde_json::json;
    use zhang_query::{Params, Value};

    use super::worked_examples::{figures, json_answer as new_json, ledger_at, ledger_of, of, BUDGETS, CLOSED_499};
    use super::*;
    use crate::cells::rows;
    use crate::request::{BuiltinParamValue, BuiltinQueryTextRequest, QueryRequest};
    use crate::routes::query;

    /// Shanghai is UTC+8: at 2024-03-31 16:30 UTC it is already April 1st there. Without a month,
    /// the budget pages show April, the ledger's current month: 500 CNY assigned in March, 100
    /// spent in March and 30 on April 1st, so April starts with 400 and spends 30. In UTC it would
    /// still be March (500 assigned, 100 spent).
    #[tokio::test(flavor = "multi_thread")]
    async fn the_current_month_is_the_ledger_timezones() {
        let ledger = ledger_at(
            r#"
option "operating_currency" "CNY"
option "timezone" "Asia/Shanghai"
1970-01-01 commodity CNY
1970-01-01 open Assets:Bank
1970-01-01 open Expenses:Food
  budget: food
2024-03-01 budget food CNY
2024-03-01 budget-add food 500 CNY
2024-03-10 "Market" "march"
  Expenses:Food 100 CNY
  Assets:Bank
2024-04-01 00:10:00 "Bakery" "first thing in april"
  Expenses:Food 30 CNY
  Assets:Bank
"#,
            "2024-03-31T16:30:00Z",
        )
        .await;
        let list = new_json(&ledger, Probe::BudgetList { month: None }).await;
        assert_eq!(figures(&list[0]), of("400", "30", "370", false));
        let info = new_json(
            &ledger,
            Probe::BudgetInfo {
                name: "food".to_owned(),
                month: None,
            },
        )
        .await;
        assert_eq!(figures(&info), of("400", "30", "370", false));
        // the query a page opens names the month it shows
        let march = new_json(&ledger, Probe::BudgetList { month: Some((2024, 3)) }).await;
        assert_eq!(figures(&march[0]), of("500", "100", "400", false));
    }

    /// A month after a budget's last row starts with what was available and spends nothing. With
    /// "today" in April 2025, the series ends in April: 1000 CNY added in March, 300 spent in
    /// April, so June starts with 700 (not April's 1000 assigned) and spends 0 (not April's 300).
    #[tokio::test(flavor = "multi_thread")]
    async fn a_month_after_the_last_row_carries_the_budget_over() {
        let ledger = ledger_at(
            r#"
option "operating_currency" "CNY"
1970-01-01 commodity CNY
1970-01-01 open Assets:Bank
1970-01-01 open Expenses:Food
  budget: food
2025-03-01 budget food CNY
2025-03-01 budget-add food 1000 CNY
2025-04-10 "Market" "april"
  Expenses:Food 300 CNY
  Assets:Bank
"#,
            "2025-04-20T04:00:00Z",
        )
        .await;
        let june = new_json(&ledger, Probe::BudgetList { month: Some((2025, 6)) }).await;
        assert_eq!(figures(&june[0]), of("700", "0", "700", false));
        // nothing spent, written as before
        assert_eq!(june[0]["activity_amount"], json!({"number": "0", "commodity": "CNY"}));
        let info = Probe::BudgetInfo {
            name: "food".to_owned(),
            month: Some((2025, 6)),
        };
        assert_eq!(figures(&new_json(&ledger, info).await), of("700", "0", "700", false));
        let april = new_json(&ledger, Probe::BudgetList { month: Some((2025, 4)) }).await;
        assert_eq!(figures(&april[0]), of("1000", "300", "700", false));
    }

    /// What a budget page shows is what the query it opens returns: the figures of `budgets.month`
    /// and `budgets.budget_month` exactly, as the query gives them, in every kind of month: before
    /// a budget, with entries, closed, without entries, the current one and later ones.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_budget_pages_show_what_their_queries_return() {
        let ledger = ledger_of(BUDGETS).await;
        let cell = |value: &Value| match value {
            Value::Amount(amount) => serde_json::to_value(amount).unwrap(),
            Value::Bool(it) => json!(it),
            Value::Str(it) => json!(it),
            Value::Null => Json::Null,
            other => panic!("{:?}", other),
        };
        // `activity` is a number in the budget's currency
        let activity = |row: &crate::cells::Row<'_>| match (row.get("activity").unwrap(), row.get("currency").unwrap()) {
            (Value::Decimal(number), Value::Str(currency)) => json!({"number": number.to_string(), "commodity": currency}),
            other => panic!("{:?}", other),
        };
        for (year, month) in [(2025, 2), (2025, 3), (2025, 4), (2025, 5), (2025, 6), (2025, 9), (2026, 1)] {
            let date = NaiveDate::from_ymd_opt(year, month, 1).unwrap();
            let pair = Some((year as u32, month));
            // what the queries return
            let (list, infos) = {
                let ledger = ledger.read().await;
                let result = crate::builtin::execute(&ledger, "budgets.month", &Params::new().bind("month", date), false).unwrap();
                let list = rows("budgets.month", &result)
                    .map(|row| {
                        json!({
                            "name": cell(row.get("name").unwrap()), "alias": cell(row.get("alias").unwrap()), "category": cell(row.get("category").unwrap()),
                            "closed": cell(row.get("closed").unwrap()), "assigned_amount": cell(row.get("assigned").unwrap()),
                            "activity_amount": activity(&row), "available_amount": cell(row.get("available").unwrap()),
                        })
                    })
                    .collect::<Vec<_>>();
                let infos = ["food", "trip"].map(|name| {
                    let params = Params::new().bind("name", name).bind("month", date);
                    let result = crate::builtin::execute(&ledger, "budgets.budget_month", &params, false).unwrap();
                    let figures = rows("budgets.budget_month", &result).next().map(|row| {
                        json!({
                            "closed": cell(row.get("closed").unwrap()), "assigned_amount": cell(row.get("assigned").unwrap()),
                            "activity_amount": activity(&row), "available_amount": cell(row.get("available").unwrap()),
                        })
                    });
                    (name, figures)
                });
                (list, infos)
            };
            // what the pages show
            assert_eq!(
                new_json(&ledger, Probe::BudgetList { month: pair }).await,
                Json::Array(list),
                "{}-{}",
                year,
                month
            );
            for (name, query) in infos {
                let mut page = new_json(
                    &ledger,
                    Probe::BudgetInfo {
                        name: name.to_owned(),
                        month: pair,
                    },
                )
                .await;
                match query {
                    Some(query) => {
                        let page = page.as_object_mut().unwrap();
                        page.retain(|key, _| ["closed", "assigned_amount", "activity_amount", "available_amount"].contains(&key.as_str()));
                        assert_eq!(Json::Object(page.clone()), query, "{} {}-{}", name, year, month);
                    }
                    // before the budget's first month the query has no row and the page shows nothing
                    None => assert_eq!(figures(&page), of("0", "0", "0", false), "{} {}-{}", name, year, month),
                }
            }
        }
    }

    /// The parameters of `budgets.postings` that the budget page's "Open query" on its activity
    /// sends, from the budget info it shows and its month (`budgetPostingsParams` in
    /// `frontend/src/components/budget/budget-query.ts`).
    fn postings_params(info: &Json, month: NaiveDate) -> HashMap<String, Option<BuiltinParamValue>> {
        let field = |key: &str| info.get(key).cloned().unwrap_or_else(|| panic!("the budget info has no {}: {}", key, info));
        let params = json!({
            "month": month.to_string(),
            "name": field("name"),
        });
        serde_json::from_value(params).unwrap()
    }

    /// The budget page's "Open query" on its activity is written out (not a 400 for a missing
    /// parameter) and runs to the postings the page lists, for an open budget and for budgets
    /// closed on a day and at a time; the budget info still carries the close (#684).
    #[tokio::test(flavor = "multi_thread")]
    async fn the_budget_pages_open_query_lists_the_postings_the_page_lists() {
        let open = ledger_of(BUDGETS).await;
        let closed = ledger_of(CLOSED_499).await;
        let cases = [
            // open: the dinner of March
            (&open, "food", (2025, 3), json!(null), json!(null), 1),
            // closed on 2024-03-01: the lunch of January in CNY (no price converts the one in USD)
            (&closed, "Food", (2024, 1), json!("2024-03-01"), json!(null), 1),
            // closed at 12:00 on 2024-04-10: the cinema before the close, not the one after
            (&closed, "Fun", (2024, 4), json!("2024-04-10"), json!("12:00:00"), 1),
        ];
        for (ledger, name, (year, month), close, close_time, postings) in cases {
            let info = new_json(
                ledger,
                Probe::BudgetInfo {
                    name: name.to_owned(),
                    month: Some((year, month)),
                },
            )
            .await;
            assert_eq!((&info["close"], &info["close_time"]), (&close, &close_time), "{}", name);

            let params = postings_params(&info, NaiveDate::from_ymd_opt(year as i32, month, 1).unwrap());
            let written = call(query::get_builtin_query_text(
                Path(("budgets.postings".to_owned(),)),
                axum::Json(BuiltinQueryTextRequest { params }),
            ))
            .await;
            let Outcome::Json(written) = written else {
                panic!("{}: {:?}", name, written);
            };
            let text = written["query"].as_str().unwrap().to_owned();

            let request = QueryRequest {
                query: text.clone(),
                count_total: None,
            };
            let response = query::run_query(State(SharedLedger(ledger.0.clone())), axum::Json(request))
                .await
                .into_response();
            assert_eq!(response.status().as_u16(), 200, "{}: {}", name, text);
            let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
            let result: Json = serde_json::from_slice(&bytes).unwrap();
            let id = result["data"]["columns"].as_array().unwrap().iter().position(|it| it["name"] == "id").unwrap();
            let ids = result["data"]["rows"].as_array().unwrap().iter().map(|row| row[id].clone()).collect::<Vec<_>>();

            // the postings the page lists, newest first like the query
            let detail = new_json(
                ledger,
                Probe::BudgetInterval {
                    name: name.to_owned(),
                    year,
                    month,
                },
            )
            .await;
            let listed = detail
                .as_array()
                .unwrap()
                .iter()
                .filter(|it| it["type"] != "BudgetEvent")
                .map(|it| it["trx_id"].clone())
                .collect::<Vec<_>>();
            assert_eq!(listed.len(), postings, "{}", name);
            assert_eq!(ids, listed, "{}: {}", name, text);
        }
    }

    /// A transaction dated 9999 by mistake in a ledger of twelve budgets: their months through
    /// it are more than the result size limit. The budget pages ask for one month and read the
    /// budgets' definitions without months, so they work; asking for a month in 9999 is a 400
    /// that names the cause.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_date_typo_far_ahead_leaves_the_budget_pages_working() {
        let mut text = String::from("option \"operating_currency\" \"CNY\"\n1970-01-01 commodity CNY\n1970-01-01 open Assets:Bank\n");
        text.push_str("1970-01-01 open Expenses:Food\n  budget: b00\n");
        for index in 0..12 {
            text.push_str(&format!("2025-01-01 budget b{:02} CNY\n", index));
        }
        text.push_str("2025-01-01 budget-add b00 100 CNY\n");
        text.push_str("2025-01-10 \"Market\" \"lunch\"\n  Expenses:Food 10 CNY\n  Assets:Bank\n");
        text.push_str("9999-01-01 \"Market\" \"a typo\"\n  Expenses:Food 1 CNY\n  Assets:Bank\n");
        let ledger = ledger_at(&text, "2025-03-15T04:00:00Z").await;
        // "this month" in Explore is bounded by today() too
        let this_month = zhang_query::Query::compile("SELECT name, available FROM #budgets WHERE date = yearmonth(today())")
            .unwrap()
            .execute(&*ledger.read().await, &Params::new())
            .unwrap();
        assert_eq!(this_month.rows.len(), 12);
        let list = new_json(&ledger, Probe::BudgetList { month: None }).await;
        assert_eq!(list.as_array().unwrap().len(), 12);
        assert_eq!(figures(&list[0]), of("90", "0", "90", false));
        let info = new_json(
            &ledger,
            Probe::BudgetInfo {
                name: "b00".to_owned(),
                month: None,
            },
        )
        .await;
        assert_eq!(info["related_accounts"], json!(["Expenses:Food"]));
        let interval = new_json(
            &ledger,
            Probe::BudgetInterval {
                name: "b00".to_owned(),
                year: 2025,
                month: 1,
            },
        )
        .await;
        assert_eq!(interval.as_array().unwrap().len(), 2);
        for probe in [
            Probe::BudgetInfo {
                name: "nope".to_owned(),
                month: None,
            },
            Probe::BudgetInterval {
                name: "nope".to_owned(),
                year: 2025,
                month: 1,
            },
        ] {
            assert_eq!(probe.run(&ledger).await, Outcome::Status(404), "{:?}", probe);
        }
        let response = budget::get_budget_list(
            State(SharedLedger(ledger.0.clone())),
            Query(BudgetListRequest {
                year: Some(9999),
                month: Some(1),
            }),
        )
        .await;
        let err = response.err().expect("too large");
        assert!(err.to_string().contains("too many rows"), "{}", err);
        assert_eq!(err.into_response().status().as_u16(), 400);
    }
}

/// Accounts named by several `budget` entries, or closed and opened again with another budget:
/// the pages and the independent computation they are checked against, on hand-verified figures.
#[cfg(test)]
mod budget_accounts {
    use serde_json::json;

    use super::worked_examples::{figures, json_answer as new_json, ledger_at, of};
    use super::*;

    fn narrations(detail: &Json) -> Vec<String> {
        detail
            .as_array()
            .unwrap()
            .iter()
            .filter(|it| it["type"] == "Posting")
            .map(|it| it["narration"].as_str().unwrap().to_owned())
            .collect()
    }

    /// `Expenses:B` is `b`'s until its close on June 30th, and `a`'s from its reopening on July
    /// 1st: the 5 CNY of March are `b`'s, the 7 CNY of July `a`'s. Each month lists the postings
    /// that count in the budget; both budgets name the account.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_reopened_account_counts_in_the_budget_of_its_open_at_each_date() {
        let ledger = ledger_at(
            r#"
option "operating_currency" "CNY"
1970-01-01 commodity CNY
1970-01-01 open Assets:Bank
1970-01-01 open Expenses:B
  budget: b
2024-02-01 budget a CNY
2024-02-01 budget b CNY
2024-03-10 "x" "b spend before close"
  Expenses:B 5 CNY
  Assets:Bank
2024-06-30 close Expenses:B
2024-07-01 open Expenses:B
  budget: a
2024-07-05 "y" "spend after reopen as a"
  Expenses:B 7 CNY
  Assets:Bank
"#,
            "2024-07-20T04:00:00Z",
        )
        .await;
        let july = new_json(&ledger, Probe::BudgetList { month: Some((2024, 7)) }).await;
        assert_eq!(figures(&july[0]), of("0", "7", "-7", false));
        assert_eq!(figures(&july[1]), of("-5", "0", "-5", false));
        let march = new_json(&ledger, Probe::BudgetList { month: Some((2024, 3)) }).await;
        assert_eq!(figures(&march[0]), of("0", "0", "0", false));
        assert_eq!(figures(&march[1]), of("0", "5", "-5", false));
        let detail = |name: &str, month: u32| Probe::BudgetInterval {
            name: name.to_owned(),
            year: 2024,
            month,
        };
        assert_eq!(narrations(&new_json(&ledger, detail("b", 3)).await), vec!["b spend before close"]);
        assert!(narrations(&new_json(&ledger, detail("a", 3)).await).is_empty());
        assert_eq!(narrations(&new_json(&ledger, detail("a", 7)).await), vec!["spend after reopen as a"]);
        assert!(narrations(&new_json(&ledger, detail("b", 7)).await).is_empty());
        for name in ["a", "b"] {
            let info = new_json(
                &ledger,
                Probe::BudgetInfo {
                    name: name.to_owned(),
                    month: Some((2024, 7)),
                },
            )
            .await;
            assert_eq!(info["related_accounts"], json!(["Expenses:B"]), "{name}");
        }
        // the independent computation agrees
        let reference = Reference::of(&*ledger.read().await).unwrap();
        let month = |m: u32| NaiveDate::from_ymd_opt(2024, m, 1).unwrap();
        assert_eq!(reference.figures("a", month(7)).map(|it| it.available.to_string()), Some("-7".to_owned()));
        assert_eq!(reference.figures("b", month(7)).map(|it| it.available.to_string()), Some("-5".to_owned()));
        assert_eq!(reference.month_postings("a", month(3)).len(), 0);
        assert_eq!(reference.month_postings("b", month(3)).len(), 1);
    }

    /// The budget page lists exactly the postings of a budget's activity, so the list adds up to
    /// the month's activity: not the market before the budget's definition, nor the dollars no
    /// price converts, and the lunch of 09:00 in `a`, the budget of the `open` in effect then,
    /// although the account names `b` from its reopening at 10:00 that day.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_posting_list_of_a_month_adds_up_to_its_activity() {
        let ledger = ledger_at(
            r#"
option "operating_currency" "CNY"
1970-01-01 commodity CNY
1970-01-01 commodity USD
1970-01-01 open Assets:Bank
1970-01-01 open Expenses:Food
  budget: food
1970-01-01 open Expenses:Lunch
  budget: a
2024-01-01 budget a CNY
2024-01-01 budget b CNY
2024-01-10 "Market" "before the budget"
  Expenses:Food 10 CNY
  Assets:Bank
2024-01-15 budget food CNY
2024-01-20 "Market" "after the budget"
  Expenses:Food 7 CNY
  Assets:Bank
2024-01-21 "Abroad" "no price converts it"
  Expenses:Food 3 USD
  Assets:Bank
2024-01-05 09:00:00 "Cafe" "lunch before the reopen"
  Expenses:Lunch 10 CNY
  Assets:Bank
2024-01-05 09:30:00 close Expenses:Lunch
2024-01-05 10:00:00 open Expenses:Lunch
  budget: b
2024-01-05 11:00:00 "Cafe" "lunch after the reopen"
  Expenses:Lunch 20 CNY
  Assets:Bank
"#,
            "2024-01-31T04:00:00Z",
        )
        .await;
        for (name, listed, activity) in [
            ("a", vec!["lunch before the reopen"], "10"),
            ("b", vec!["lunch after the reopen"], "20"),
            ("food", vec!["after the budget"], "7"),
        ] {
            let detail = new_json(
                &ledger,
                Probe::BudgetInterval {
                    name: name.to_owned(),
                    year: 2024,
                    month: 1,
                },
            )
            .await;
            assert_eq!(narrations(&detail), listed, "{}", name);
            let sum = detail
                .as_array()
                .unwrap()
                .iter()
                .filter(|it| it["type"] == "Posting")
                .map(|it| BigDecimal::from_str(it["inferred_unit"]["number"].as_str().unwrap()).unwrap())
                .sum::<BigDecimal>();
            let info = new_json(
                &ledger,
                Probe::BudgetInfo {
                    name: name.to_owned(),
                    month: Some((2024, 1)),
                },
            )
            .await;
            assert_eq!(figures(&info).1, activity, "{}", name);
            assert_eq!(zhang_query::decimal::to_plain_string(&sum.normalized()), activity, "{}", name);
        }
        // the independent computation agrees
        let reference = Reference::of(&*ledger.read().await).unwrap();
        let january = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
        for (name, count) in [("a", 1), ("b", 1), ("food", 1)] {
            assert_eq!(reference.month_postings(name, january).len(), count, "{}", name);
        }
    }

    /// A repeated identical `budget: a` names `a` once: its 10 CNY are spent once, leaving 90 of
    /// the 100 added, in the pages and in the independent computation.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_repeated_budget_entry_counts_once() {
        let ledger = ledger_at(
            r#"
option "operating_currency" "CNY"
1970-01-01 commodity CNY
1970-01-01 open Assets:Bank
1970-01-01 open Expenses:A
  budget: a
  budget: a
2024-02-01 budget a CNY
2024-02-02 budget-add a 100 CNY
2024-02-10 "x" "a spend"
  Expenses:A 10 CNY
  Assets:Bank
"#,
            "2024-02-20T04:00:00Z",
        )
        .await;
        let list = new_json(&ledger, Probe::BudgetList { month: Some((2024, 2)) }).await;
        assert_eq!(figures(&list[0]), of("100", "10", "90", false));
        let reference = Reference::of(&*ledger.read().await).unwrap();
        let figures = reference.figures("a", NaiveDate::from_ymd_opt(2024, 2, 1).unwrap()).unwrap();
        assert_eq!(
            (figures.activity.to_string(), figures.available.to_string()),
            ("10".to_owned(), "90".to_owned())
        );
    }
}
