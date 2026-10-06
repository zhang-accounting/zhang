//! Golden tests of the report (`/api/statistic/*`), the built-in queries of `report`: on every
//! fixture ledger against the query engine's own computation, and on small ledgers checked by
//! hand.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;

use bigdecimal::{BigDecimal, Zero};
use chrono::{Datelike, Days, Months, NaiveDate, Utc};
use serde_json::Value;
use zhang_ast::amount::CalculatedAmount;
use zhang_ast::{AccountType, Flag};
use zhang_core::data_source::LocalFileSystemDataSource;
use zhang_core::data_type::text::ZhangDataType;
use zhang_core::ledger::Ledger;
use zhang_query::{DataType, ParamTypes, Params, Query};
use zhang_server::builtin::{calculated_amount, execute, LedgerDateRange};
use zhang_server::report;
use zhang_server::request::StatisticInterval;

/// A ledger to test on.
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

/// A copy of the ledger's directory, with `option "timezone"` set when `timezone` is given, so
/// the tests do not depend on the timezone of the machine.
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

/// The ranges to test on: the whole ledger, the month of its last transaction, the three
/// months up to it, and the first day of that month.
fn ranges(ledger: &Ledger) -> Vec<(&'static str, LedgerDateRange, StatisticInterval)> {
    let timezone = ledger.options.timezone;
    let dates: BTreeSet<NaiveDate> = ledger
        .transactions()
        .into_iter()
        .map(|(_, trx)| trx.date.to_timezone_datetime(&timezone).date_naive())
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

/// A figure in comparable form: its calculated number and its non-zero units per currency
/// (the engine's inventories drop a currency whose units add up to zero), with numbers compared
/// as decimals (`0.00` is `0`).
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

/// The ten largest postings of `account_type` in `range` as the engine values them apart from
/// `report.top_postings`: by value in the operating currency at the prices of the last day,
/// income and liabilities negated, those without a price last, then in ledger order; as
/// `date account units currency id`.
fn expected_top(ledger: &Ledger, account_type: AccountType, range: &LedgerDateRange) -> Vec<String> {
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
    let currency = ledger.options.operating_currency.clone();
    let params = Params::new()
        .bind("currency", currency.as_str())
        .bind("type", account_type.to_string())
        .bind("from", range.from)
        .bind("to", range.to);
    let rows = query.execute_at(ledger, &params, Utc::now().date_naive()).unwrap().rows;
    let mut postings: Vec<(bool, BigDecimal, String)> = rows
        .into_iter()
        .map(|row| match &row[..] {
            [zhang_query::Value::Date(date), zhang_query::Value::Str(account), zhang_query::Value::Str(id), zhang_query::Value::Amount(units), zhang_query::Value::Amount(value)] => {
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
    postings.into_iter().take(10).map(|(_, _, text)| text).collect()
}

/// What is wrong with the report of one ledger on the ranges of [`ranges`]: a graph by day whose
/// days are not those of the range, or top postings that are not the engine's ([`expected_top`]).
fn wrong_answers(name: &str, ledger: &Ledger) -> Vec<String> {
    let mut wrong = vec![];
    for (_, range, _) in ranges(ledger) {
        let label = format!("{} {}..{}", name, range.from, range.to);
        if (range.to - range.from).num_days() <= 100 {
            let graph = report::graph(ledger, &range, &StatisticInterval::Day).unwrap();
            let days: BTreeSet<NaiveDate> = graph.balances.keys().copied().collect();
            let expected: BTreeSet<NaiveDate> = std::iter::successors(Some(range.from), |d| d.succ_opt().filter(|next| *next <= range.to)).collect();
            if days != expected {
                wrong.push(format!("{} graph: days {:?}", label, days));
            }
        }
        for account_type in [AccountType::Expenses, AccountType::Income] {
            let rank = report::rank(ledger, account_type, &range).unwrap();
            let rows: Vec<String> = rank
                .top_transactions
                .iter()
                .map(|it| {
                    format!(
                        "{} {} {} {} {}",
                        it.datetime.date(),
                        it.account,
                        plain(&it.inferred_unit.number),
                        it.inferred_unit.commodity,
                        it.trx_id
                    )
                })
                .collect();
            let expected = expected_top(ledger, account_type, &range);
            if rows != expected {
                wrong.push(format!(
                    "{} rank {}: {} instead of {}",
                    label,
                    account_type,
                    rows.join("; "),
                    expected.join("; ")
                ));
            }
        }
    }
    wrong
}

/// A small ledger with the cases the report must get right (#479), in a timezone east of UTC.
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
    assert_eq!(summary.from.to_string(), "2025-04-01 00:00:00");
    assert_eq!(summary.to.to_string(), "2025-04-30 23:59:59");
}

/// The summary, the graph and the rank of one range echo it alike, as the ledger's dates from their first to their
/// last second: the summary echoed the UTC instants of those seconds instead, `2025-03-31T16:00:00Z` to
/// `2025-04-30T15:59:59Z` for April in Shanghai
#[test]
fn every_report_echoes_its_range_as_the_ledgers_dates() {
    let ledger = hand_ledger();
    let summary = report::summary(&ledger, &APRIL).unwrap();
    let graph = report::graph(&ledger, &APRIL, &StatisticInterval::Day).unwrap();
    let rank = report::rank(&ledger, AccountType::Expenses, &APRIL).unwrap();
    let echo = |from: chrono::NaiveDateTime, to: chrono::NaiveDateTime| (from.to_string(), to.to_string());
    let april = ("2025-04-01 00:00:00".to_owned(), "2025-04-30 23:59:59".to_owned());
    assert_eq!(echo(summary.from, summary.to), april);
    assert_eq!(echo(graph.from, graph.to), april);
    assert_eq!(echo(rank.from, rank.to), april);
    // and in JSON, as the API answers
    let json = |value: serde_json::Value| (value["from"].clone(), value["to"].clone());
    let summary = json(serde_json::to_value(&summary).unwrap());
    assert_eq!(summary, (serde_json::json!("2025-04-01T00:00:00"), serde_json::json!("2025-04-30T23:59:59")));
    assert_eq!(json(serde_json::to_value(&graph).unwrap()), summary);
    assert_eq!(json(serde_json::to_value(&rank).unwrap()), summary);
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
    let transactions = ledger.transactions();
    let rent_on_the_first = |trx: &zhang_ast::Transaction| trx.narration.as_ref().map(|it| it.as_str()) == Some("rent on the first");
    let trx_id = transactions.into_iter().find(|(_, trx)| rent_on_the_first(trx)).unwrap().0.to_string();
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

/// On every fixture and the hand ledger, in UTC and east of it, every graph by day has the days
/// of its range and the top postings are the engine's.
#[test]
fn the_report_is_the_engines_on_every_ledger() {
    let mut wrong = vec![];
    for timezone in ["UTC", "Asia/Shanghai"] {
        let ledgers = fixtures()
            .into_iter()
            .filter_map(|case| load(&case, Some(timezone)).map(|ledger| (case.name, ledger)))
            .chain([("hand".to_owned(), hand_ledger())]);
        for (name, ledger) in ledgers {
            wrong.extend(wrong_answers(&format!("{} ({})", name, timezone), &ledger));
        }
    }
    assert!(wrong.is_empty(), "wrong answers:\n{}", wrong.join("\n"));
}

/// The summary of `range` that the engine computes apart from the report's queries
/// ([`engine_figure`]), and the store: the net worth and the liabilities at the end of the range
/// and the income and the expenses of the range, by units and by value at the prices of its last
/// day, and the number of the store's transactions dated in the range, padding transactions left
/// out. What differs from `report::summary`, as `item: expected -> actual`.
fn summary_differences(ledger: &Ledger, range: &LedgerDateRange) -> Vec<String> {
    let summary = report::summary(ledger, range).unwrap();
    let mut differences = vec![];
    for (item, actual, types, from) in [
        ("balance", &summary.balance, &[AccountType::Assets, AccountType::Liabilities][..], day_one()),
        ("liability", &summary.liability, &[AccountType::Liabilities][..], day_one()),
        ("income", &summary.income, &[AccountType::Income][..], range.from),
        ("expense", &summary.expense, &[AccountType::Expenses][..], range.from),
    ] {
        let expected = engine_figure(ledger, types, None, from, range.to, range.to);
        let actual = Fig::of(actual);
        if actual != expected {
            differences.push(format!("{}: {:?} -> {:?}", item, expected, actual));
        }
    }
    let timezone = ledger.options.timezone;
    let transactions = ledger
        .transactions()
        .into_iter()
        .map(|(_, trx)| trx)
        .filter(|trx| trx.flag != Some(Flag::BalancePad))
        .filter(|trx| {
            let date = trx.date.to_timezone_datetime(&timezone).date_naive();
            range.from <= date && date <= range.to
        })
        .count() as i64;
    if summary.transaction_number != transactions {
        differences.push(format!("transaction_number: {} -> {}", transactions, summary.transaction_number));
    }
    differences
}

/// On every fixture, in UTC and east of it, and on the two hand ledgers, the summary of every
/// range of [`ranges`] is the engine's and the store's ([`summary_differences`]).
#[test]
fn the_summary_is_the_engines_on_every_ledger() {
    let mut wrong = vec![];
    for timezone in ["UTC", "Asia/Shanghai"] {
        for case in fixtures() {
            if let Some(ledger) = load(&case, Some(timezone)) {
                for (label, range, _) in ranges(&ledger) {
                    for difference in summary_differences(&ledger, &range) {
                        wrong.push(format!("{} ({}) {} {}..{}: {}", case.name, timezone, label, range.from, range.to, difference));
                    }
                }
            }
        }
    }
    for (name, ledger) in [("hand", hand_ledger()), ("carry", carry_ledger())] {
        for (label, range, _) in ranges(&ledger).into_iter().chain([("april", APRIL, StatisticInterval::Day)]) {
            for difference in summary_differences(&ledger, &range) {
                wrong.push(format!("{} {} {}..{}: {}", name, label, range.from, range.to, difference));
            }
        }
    }
    assert!(wrong.is_empty(), "wrong summaries:\n{}", wrong.join("\n"));
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

/// Every fixture ledger, in UTC, and the two hand ledgers.
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
