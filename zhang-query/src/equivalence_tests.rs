//! The execution decisions of the optimizer and the projector (deferred running balances,
//! running sums for `units(balance)` / `cost(balance)`, LIMIT and OFFSET pushdown,
//! account-scoped scans, hashed `IN` lists, prepared string tests, and parameters bound as
//! constants) and the per-ledger cache change how much work an execution does, never its
//! result: every query here returns byte-identical rows (decimal scales included), and the same
//! total row count, with [`Query::compile_with_params`] and with [`Query::compile_naive`], with
//! and without its account scope, and on a cache that other queries built and on a fresh one.

use std::path::PathBuf;
use std::sync::Arc;

use chrono::NaiveDate;
use zhang_core::data_source::LocalFileSystemDataSource;
use zhang_core::data_type::text::ZhangDataType;
use zhang_core::ledger::Ledger;

use crate::{DataType, ExecuteOptions, ParamTypes, Params, Query, Value};

fn load(dir: PathBuf) -> Ledger {
    let source = LocalFileSystemDataSource::new(ZhangDataType {});
    Ledger::load_with_data_source(dir, "main.zhang".to_owned(), Arc::new(source)).expect("cannot load ledger")
}

fn load_text(content: &str) -> Ledger {
    let dir = tempfile::tempdir().expect("tempdir").keep();
    std::fs::write(dir.join("main.zhang"), content).expect("write ledger");
    load(dir)
}

fn fava_demo_ledger() -> Ledger {
    load(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../integration-tests/fava-demo-ledger"))
}

/// A deterministic ledger exercising the running sums: lots in two commodities with costs in
/// two currencies, numbers of several scales, zero costs, sales of lots that are not open
/// (so negative lots appear next to positive ones) and cash without cost.
fn random_lots_ledger(seed: u64, transactions: usize) -> String {
    let mut state = seed;
    let mut next = move |bound: u64| {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (state >> 33) % bound
    };
    let mut ledger = String::from(
        "option \"operating_currency\" \"USD\"\n\
         1970-01-01 commodity USD\n1970-01-01 commodity EUR\n1970-01-01 commodity STK\n1970-01-01 commodity ETF\n\
         1970-01-01 open Assets:Broker\n1970-01-01 open Assets:Short\n1970-01-01 open Assets:Cash\n1970-01-01 open Expenses:Fees\n\
         2020-01-01 price STK 12.5 USD\n2021-06-01 price ETF 3.25 EUR\n2021-06-01 price EUR 1.1 USD\n",
    );
    let numbers = ["1", "2", "0.5", "1.000", "3.25", "10", "0.125"];
    let costs = ["0", "1.5", "10.25", "11.50", "12", "0.333"];
    let start = NaiveDate::from_ymd_opt(2020, 1, 1).unwrap();
    for idx in 0..transactions {
        let date = start + chrono::Duration::days(next(700) as i64);
        let (commodity, currency) = if next(3) == 0 { ("ETF", "EUR") } else { ("STK", "USD") };
        let units = numbers[next(numbers.len() as u64) as usize];
        let cost = costs[next(costs.len() as u64) as usize];
        let total = crate::decimal::to_plain_string(&crate::decimal::mul(&units.parse().unwrap(), &cost.parse().unwrap()));
        let postings = match next(6) {
            0 | 1 => format!("  Assets:Broker {units} {commodity} {{{cost} {currency}}}\n  Assets:Cash -{total} {currency}"),
            // a sale of a lot that may not be open: the uncovered part opens a negative lot
            2 => format!("  Assets:Broker -{units} {commodity} {{{cost} {currency}}}\n  Assets:Cash {total} {currency}"),
            3 => format!("  Assets:Short -{units} {commodity} {{{cost} {currency}}}\n  Assets:Cash {total} {currency}"),
            _ => format!("  Assets:Cash -{units} {currency}\n  Expenses:Fees {units} {currency}"),
        };
        ledger.push_str(&format!("\n{date} * \"trade {idx}\"\n{postings}\n"));
    }
    ledger
}

/// Queries over every kind of decision: deferred targets and aggregates, eager running totals
/// (ORDER BY, DISTINCT and GROUP BY over balances, min/max/sum), the linear rewrites, top-k,
/// stopping the scan, the first groups, LIMIT 0, and plans without a balance.
const QUERIES: &[&str] = &[
    "JOURNAL",
    "JOURNAL AT units",
    "JOURNAL AT cost",
    "JOURNAL AT value",
    "JOURNAL 'Broker' AT cost",
    "JOURNAL 'Cash|Short' AT units FROM year >= 2016",
    "BALANCES",
    "BALANCES AT cost",
    "SELECT date, balance ORDER BY date DESC LIMIT 10",
    "SELECT date, account, position, balance, units(balance), cost(balance) WHERE account ~ 'Broker|Short'",
    "SELECT date, units(balance), cost(balance), value(balance) FROM year >= 2016 WHERE account ~ 'Vanguard|Broker'",
    "SELECT date, cost(balance) ORDER BY date DESC, account LIMIT 7",
    "SELECT account, last(balance), first(balance), count(*) GROUP BY account",
    "SELECT account, last(cost(balance)), first(units(balance)) GROUP BY account ORDER BY account LIMIT 3",
    "SELECT date, last(units(balance)) GROUP BY date ORDER BY date DESC LIMIT 5",
    "SELECT account, min(balance), max(cost(balance)), sum(units(balance)) GROUP BY account",
    "SELECT account, last(balance) GROUP BY account LIMIT 2",
    "SELECT year, count(*), last(cost(balance)) GROUP BY year LIMIT 1",
    "SELECT last(str(balance)), first(convert(balance, 'USD'))",
    "SELECT str(balance) AS b, count(*) GROUP BY b ORDER BY b LIMIT 4",
    "SELECT DISTINCT account, units(balance)",
    "SELECT DISTINCT account LIMIT 3",
    "SELECT DISTINCT currency ORDER BY currency DESC LIMIT 2",
    "SELECT str(balance), length(str(cost(balance))) LIMIT 7",
    "SELECT date, balance ORDER BY balance LIMIT 3",
    "SELECT date, balance ORDER BY 2 DESC, 1 LIMIT 3",
    "SELECT possign(balance, account), abs(balance), neg(cost(balance)) WHERE account ~ 'Liab|Income|Short'",
    "SELECT only('USD', balance), filter_currency(balance, 'STK') LIMIT 50",
    "SELECT units(balance) = cost(balance), balance IS NULL, 'x' IN tags FROM month = 1",
    "SELECT date, balance LIMIT 0",
    "SELECT date, balance ORDER BY date LIMIT 0",
    "SELECT account, last(balance) GROUP BY account LIMIT 0",
    "SELECT * ORDER BY date DESC, account LIMIT 25",
    "SELECT account, count(*) GROUP BY account LIMIT 2",
    "SELECT payee, sum(position) GROUP BY payee ORDER BY payee LIMIT 4",
    "SELECT account, balance FROM OPEN ON 2016-01-01 CLOSE ON 2017-01-01 WHERE account ~ 'Assets' LIMIT 20",
    "SELECT date, position LIMIT 5",
    // HAVING drops finished groups, so LIMIT never keeps only the first groups
    "SELECT account, count(*) GROUP BY account HAVING count(*) > 20 LIMIT 3",
    "SELECT account, last(balance), count(*) GROUP BY account HAVING count(*) > 5 AND last(balance) IS NOT NULL LIMIT 2",
    "SELECT year, account, sum(position) AS total GROUP BY 1, 2 HAVING count(*) > 1 PIVOT BY account, year LIMIT 10",
    "SELECT account, year, last(cost(balance)) AS b, count(*) AS n GROUP BY 1, 2 ORDER BY n DESC PIVOT BY account, year",
    // the other tables: top-k, stopping the scan, first groups, DISTINCT, HAVING and PIVOT BY
    "SELECT * FROM #entries ORDER BY date DESC, type LIMIT 7",
    "SELECT type, count(*) FROM #entries GROUP BY type LIMIT 2",
    "SELECT date, payee FROM #transactions LIMIT 5",
    "SELECT DISTINCT currency FROM #prices LIMIT 3",
    "SELECT currency, last(amount) FROM prices GROUP BY currency HAVING count(*) > 1 ORDER BY currency",
    "SELECT account, open.date, close FROM #accounts ORDER BY open.date DESC, account LIMIT 5",
    "SELECT account, year(date) AS y, count(*) AS n FROM #balances GROUP BY 1, 2 HAVING count(*) > 1 PIVOT BY account, y",
    "SELECT name, date FROM #commodities ORDER BY name DESC LIMIT 2",
    // top-k pages whose other targets are built for the kept rows only, also next to running totals
    "SELECT seq, type, id, date, time, payee, narration, tags, links, metas, accounts FROM #entries ORDER BY seq DESC LIMIT 9 OFFSET 4",
    "SELECT date, path, account, transaction_id FROM #documents ORDER BY seq DESC LIMIT 3",
    "SELECT id, account, payee, units(position), str(position), metas, entry_metas ORDER BY seq DESC, posting_index LIMIT 20 OFFSET 7",
    "SELECT date, account, balance, account_balance, payee, number / 0 ORDER BY date DESC, account LIMIT 6 OFFSET 2",
    // a CASE built for the kept rows of a top-k page, or while scanning when a branch can fail; CASE under OFFSET
    // without ORDER BY, whose skipped rows are not built
    "SELECT seq, payee, CASE WHEN flag = 'P' THEN 'pad' WHEN payee IS NULL THEN narration ELSE payee END AS who FROM #entries ORDER BY seq DESC LIMIT 5 OFFSET 3",
    "SELECT date, account, CASE WHEN number > 0 THEN number / 0 ELSE number END AS odd ORDER BY date DESC, account LIMIT 4 OFFSET 1",
    "SELECT date, CASE WHEN number < 0 THEN 'out' ELSE 'in' END AS way, units(balance) LIMIT 5 OFFSET 7",
    // a CASE inside an aggregate and an aggregate written twice, with HAVING, ORDER BY, LIMIT and OFFSET
    "SELECT account, sum(CASE WHEN number > 0 THEN number ELSE 0 END) AS inflow, sum(CASE WHEN number > 0 THEN number ELSE 0 END) * 2 AS twice GROUP BY account ORDER BY account LIMIT 3 OFFSET 2",
    "SELECT account, count(*) AS n, count(*) + 1 AS more, last(balance) GROUP BY account HAVING count(*) > 2 ORDER BY count(*) DESC, account LIMIT 3 OFFSET 1",
    "SELECT seq, least(seq, 10) AS low, greatest(seq, 10) AS high, CASE WHEN seq > 10 THEN 'late' END AS mark FROM #entries ORDER BY seq DESC LIMIT 4 OFFSET 2",
    // account-scoped scans, and the columns zhang adds
    "SELECT date, account, position, balance, account_balance WHERE account = 'Assets:Broker' OR account = 'Assets:US:BofA:Checking'",
    "SELECT date, account, balance, account_balance, seq, posting_index WHERE account IN ('Assets:Cash', 'Assets:US:Vanguard:Cash', 'Nowhere') ORDER BY seq DESC LIMIT 20",
    "SELECT account, last(account_balance), last(balance), count(*) WHERE year >= 2016 AND account IN ('Assets:Short', 'Liabilities:US:Chase:Slate') GROUP BY account",
    "SELECT date, account_balance, balance FROM year = 2016 WHERE account = 'Assets:US:BofA:Checking' AND number > 0",
    "SELECT date, units(account_balance), cost(balance) WHERE 'Assets:Broker' = account AND number < 0 LIMIT 5",
    // the account balances read while scanning (the filter, ORDER BY, DISTINCT, GROUP BY,
    // aggregates) and deferred to the rows a query keeps, with their linear rewrites
    "SELECT date, account, account_balance WHERE number(only('USD', units(account_balance))) < 0 LIMIT 10",
    "SELECT date, account FROM number(only('USD', cost(account_balance))) > 0 WHERE account ~ 'Assets' LIMIT 12",
    "SELECT account, account_balance ORDER BY account_balance DESC, date LIMIT 3",
    "SELECT DISTINCT account, units(account_balance) WHERE account ~ 'Broker|Cash|Vanguard' LIMIT 20",
    "SELECT account, sum(units(account_balance)), last(cost(account_balance)), first(account_balance), count(*) GROUP BY account",
    "SELECT str(units(account_balance)) AS u, count(*) GROUP BY u ORDER BY count(*) DESC, u LIMIT 3",
    "SELECT date, balance, account_balance, units(account_balance), cost(account_balance) WHERE account ~ 'Broker|Short|Checking' ORDER BY date DESC LIMIT 15",
    "SELECT date, account, cost(account_balance), units(balance) FROM year >= 2016 LIMIT 40",
    "SELECT account, last(units(account_balance)) GROUP BY account LIMIT 2",
    "SELECT count(*), sum(position) WHERE account = NULL",
    "JOURNAL 'Assets:Broker' AT cost",
    "SELECT account, account_balance, balanced, errors, time, timestamp, seq, posting_index FROM OPEN ON 2016-01-01 CLOSE ON 2017-01-01 WHERE account ~ 'Assets' LIMIT 30",
    "SELECT account, account_balance FROM CLOSE ON 2021-01-01 CLEAR WHERE account = 'Equity:Earnings:Current' OR account = 'Expenses:Fees'",
    "SELECT seq, time, timestamp, posting_index, balanced, errors ORDER BY seq DESC, posting_index LIMIT 12",
    "SELECT id, seq, time, timestamp, balanced, errors FROM #transactions ORDER BY seq DESC LIMIT 5",
    "SELECT seq, id, time, timestamp FROM #entries WHERE seq > 10 LIMIT 5",
    // OFFSET with every LIMIT strategy: top-k, stopping the scan (with DISTINCT), the first
    // groups, deferred balances, HAVING and PIVOT BY, and an offset past the end
    "SELECT date, balance ORDER BY date DESC LIMIT 10 OFFSET 5",
    "SELECT date, account, position, balance LIMIT 7 OFFSET 3",
    "SELECT DISTINCT account LIMIT 3 OFFSET 2",
    "SELECT DISTINCT account, currency ORDER BY account DESC LIMIT 4 OFFSET 1",
    "SELECT account, count(*) GROUP BY account LIMIT 2 OFFSET 3",
    "SELECT account, last(balance) GROUP BY account ORDER BY account LIMIT 3 OFFSET 2",
    "SELECT account, count(*) GROUP BY account HAVING count(*) > 5 LIMIT 2 OFFSET 1",
    "SELECT year, account, sum(position) AS total GROUP BY 1, 2 PIVOT BY account, year LIMIT 6 OFFSET 2",
    "SELECT date, position LIMIT 5 OFFSET 100000",
    "SELECT date, position ORDER BY date LIMIT 0 OFFSET 3",
    "SELECT * FROM #entries ORDER BY date DESC, type LIMIT 7 OFFSET 7",
    // hashed IN lists and prepared string tests over literals
    "SELECT account, count(*) WHERE account IN ('Assets:Cash', 'Assets:Broker', 'Expenses:Food:Coffee', NULL) GROUP BY account",
    "SELECT count(*) WHERE number IN (1, 2.0, 10.00, 0.5) OR number NOT IN (3.25, NULL)",
    "SELECT date, payee WHERE icontains(payee, 'TRADE') OR icontains(narration, 'café') OR any_icontains(tags, 'TRIP')",
    "SELECT account, count(*) WHERE under(account, 'Assets') OR under(account, 'Expenses:Food') GROUP BY account",
    "SELECT DISTINCT account WHERE NOT under(account, 'Assets:US') AND account IN ('Assets:US:BofA:Checking', 'Assets:Cash')",
];

/// Queries with parameters, run with [`PARAMS`]: the optimized execution binds them as
/// constants (compiling a pattern, hashing a set, lower-casing a needle once), the naive one
/// evaluates them for every row.
const PARAM_QUERIES: &[&str] = &[
    "SELECT date, payee WHERE payee ~ :keyword OR narration ~ :keyword OR account ~ :keyword",
    "SELECT DISTINCT id WHERE payee ~ :keyword OR str(tags) ~ :keyword ORDER BY id LIMIT :size OFFSET :offset",
    "SELECT account, count(*) WHERE account IN :accounts GROUP BY account",
    "SELECT count(*) WHERE account NOT IN :accounts AND :missing IN tags",
    "SELECT date, account WHERE account IN (:account, 'Assets:Cash', :missing) LIMIT :size",
    "SELECT date, payee WHERE icontains(payee, :needle) OR icontains(narration, :needle) ORDER BY date DESC LIMIT :size OFFSET :offset",
    "SELECT seq, type, id, payee, tags, metas FROM #entries WHERE icontains(payee, :needle) ORDER BY seq DESC LIMIT :size OFFSET :offset",
    "SELECT account, sum(position) WHERE under(account, :root) GROUP BY account ORDER BY account",
    "SELECT date, balance WHERE under(account, :root) ORDER BY date DESC LIMIT :size OFFSET :offset",
    "SELECT account, count(*) WHERE account ~ :empty OR :flag GROUP BY account LIMIT :size",
    "SELECT date, payee WHERE payee ~ :invalid LIMIT :size",
    "SELECT DISTINCT account LIMIT :size OFFSET :offset",
    "SELECT account, count(*) GROUP BY account LIMIT :size OFFSET :offset",
    // parameters that fold WHERE or HAVING to a constant: only TRUE drops it, FALSE and NULL
    // keep no row or group
    "SELECT date, account WHERE :flag",
    "SELECT date, account WHERE :none = 1 LIMIT :size",
    "SELECT count(*), sum(position) WHERE :flag AND account ~ 'Assets'",
    "SELECT account, count(*) GROUP BY account HAVING count(*) > 0 AND :flag",
    "SELECT account, count(*) GROUP BY account HAVING (count(*) > 0 OR :yes) AND :none = 1",
    "SELECT account, count(*) GROUP BY account HAVING (count(*) > 0 OR :yes) AND :flag",
];

fn params() -> Params {
    let accounts = [
        "Assets:Cash",
        "Assets:Broker",
        "Expenses:Food:Coffee",
        "Assets:US:BofA:Checking",
        "Liabilities:US:Chase:Slate",
    ];
    Params::new()
        .bind("keyword", "(?i)trade|coffee|Rent")
        .bind("accounts", Value::Set(accounts.iter().map(|it| it.to_string()).collect()))
        .bind("account", "Expenses:Fees")
        .bind("missing", Value::Null)
        .bind("needle", "TrAdE")
        .bind("root", "Assets")
        .bind("empty", "")
        .bind("flag", false)
        .bind("yes", true)
        .bind("none", Value::Null)
        .bind("invalid", "(")
        .bind("size", 9i64)
        .bind("offset", 4i64)
}

fn param_types() -> ParamTypes {
    let mut types = params().types();
    types = types.bind("missing", DataType::Str).bind("none", DataType::Int);
    types
}

fn options() -> ExecuteOptions {
    ExecuteOptions {
        today: Some(NaiveDate::from_ymd_opt(2026, 1, 1).unwrap()),
        timeout: None,
        max_result_values: None,
        count_total: true,
    }
}

/// The same result, compared by the `Debug` form of the rows, which shows decimal scales.
fn assert_same(sql: &str, expected: Result<crate::QueryResult, crate::QueryError>, actual: Result<crate::QueryResult, crate::QueryError>) {
    match (expected, actual) {
        (Ok(expected), Ok(actual)) => {
            assert_eq!(expected.columns, actual.columns, "{sql}");
            assert_eq!(format!("{:?}", expected.rows), format!("{:?}", actual.rows), "{sql}");
            assert_eq!(expected.total, actual.total, "{sql}");
        }
        (Err(expected), Err(actual)) => assert_eq!(expected, actual, "{sql}"),
        (expected, actual) => panic!("{sql}: {:?} vs {:?}", expected.map(|it| it.rows.len()), actual.map(|it| it.rows.len())),
    }
}

/// Run every query both ways and compare the rows.
fn assert_equivalent(ledger: &Ledger, queries: &[&str]) {
    assert_equivalent_with(ledger, queries, &ParamTypes::new(), &Params::new());
}

/// Run every query naively and optimized (with its parameters bound as constants, its account
/// scope and the total count), without its scope, and without the total, and compare.
fn assert_equivalent_with(ledger: &Ledger, queries: &[&str], types: &ParamTypes, params: &Params) {
    let options = options();
    for sql in queries {
        let compile = || Query::compile_with_params(sql, types).unwrap_or_else(|err| panic!("{sql}: {err}"));
        let run = |query: Query, options: &ExecuteOptions| query.execute_with_options(ledger, params, options);
        let naive = run(Query::compile_naive(sql, types).unwrap_or_else(|err| panic!("{sql}: {err}")), &options);
        let optimized = run(compile(), &options);
        if let Ok(optimized) = &optimized {
            assert!(optimized.total.is_some(), "{sql}");
        }
        if compile().plan.execution.scope.is_some() || compile().plan.execution.until.is_some() {
            assert_same(sql, run(unscoped(compile()), &options), optimized.clone());
        }
        // counting the rows changes nothing else
        let uncounted = run(
            compile(),
            &ExecuteOptions {
                count_total: false,
                ..options.clone()
            },
        );
        match (uncounted, &optimized) {
            (Ok(uncounted), Ok(optimized)) => {
                assert_eq!(format!("{:?}", uncounted.rows), format!("{:?}", optimized.rows), "{sql}");
                assert_eq!(uncounted.total, None, "{sql}");
            }
            (Err(uncounted), Err(optimized)) => assert_eq!(&uncounted, optimized, "{sql}"),
            // counting evaluates the filter past the window, where a row may fail that a scan
            // stopped by LIMIT never reaches
            (Ok(_), Err(_)) => {}
            (Err(uncounted), Ok(_)) => panic!("{sql}: only the uncounted execution fails: {uncounted}"),
        }
        assert_same(sql, naive, optimized);
    }
}

/// `query` reading every row instead of the rows of its account scope, and generating every
/// row instead of those up to its date bound.
fn unscoped(mut query: Query) -> Query {
    query.plan.execution.scope = None;
    query.plan.execution.until = None;
    query
}

#[test]
fn bound_parameters_keep_results() {
    let types = param_types();
    assert_equivalent_with(&fava_demo_ledger(), PARAM_QUERIES, &types, &params());
    for seed in 1..=3 {
        assert_equivalent_with(&load_text(&random_lots_ledger(seed, 120)), PARAM_QUERIES, &types, &params());
    }
    // other windows, and parameters that make the filters always or never hold
    for (size, offset, flag, root) in [
        (0i64, 0i64, true, ""),
        (1, 0, true, "Expenses"),
        (1000, 3, false, "Assets:US"),
        (5, 100_000, true, "Liabilities"),
    ] {
        let params = params().bind("size", size).bind("offset", offset).bind("flag", flag).bind("root", root);
        assert_equivalent_with(&fava_demo_ledger(), PARAM_QUERIES, &types, &params);
    }
}

/// The total counts every row before LIMIT and OFFSET, whatever cuts the scan short.
#[test]
fn totals_count_the_rows_before_the_window() {
    let ledger = fava_demo_ledger();
    let run = |sql: &str| Query::compile(sql).unwrap().execute_with_options(&ledger, &Params::new(), &options()).unwrap();
    for (sql, all) in [
        ("SELECT date, position LIMIT 5 OFFSET 2", "SELECT date, position"),
        ("SELECT date, position ORDER BY date DESC LIMIT 5", "SELECT date, position"),
        ("SELECT DISTINCT account LIMIT 3", "SELECT DISTINCT account"),
        ("SELECT DISTINCT account, units(balance) LIMIT 3", "SELECT DISTINCT account, units(balance)"),
        ("SELECT account, count(*) GROUP BY account LIMIT 2", "SELECT account, count(*) GROUP BY account"),
        (
            "SELECT account, count(*) GROUP BY account HAVING count(*) > 50 LIMIT 2",
            "SELECT account, count(*) GROUP BY account HAVING count(*) > 50",
        ),
        (
            "SELECT year, account, count(*) AS n GROUP BY 1, 2 PIVOT BY account, year LIMIT 4",
            "SELECT year, account, count(*) AS n GROUP BY 1, 2",
        ),
        (
            "SELECT date, balance WHERE account ~ 'Vanguard' ORDER BY date DESC LIMIT 2 OFFSET 1",
            "SELECT date WHERE account ~ 'Vanguard'",
        ),
        ("SELECT date LIMIT 0", "SELECT date"),
        ("SELECT * FROM #prices LIMIT 3 OFFSET 900", "SELECT * FROM #prices"),
    ] {
        let windowed = run(sql);
        let everything = run(all);
        assert_eq!(windowed.total, Some(everything.rows.len() as u64), "{sql}");
        assert_eq!(everything.total, Some(everything.rows.len() as u64), "{all}");
    }
}

/// Grouped queries whose LIMIT keeps their first groups: the rows of a posting's lots, of a day and of a month come
/// one after another, in one run, and the runs in the order of their keys; those of an account or a payee do not.
const GROUPED: &[&str] = &[
    "SELECT seq, posting_index, sum(number) AS units, first(currency), last(only(currency, units(balance))) GROUP BY seq, posting_index",
    "SELECT seq, posting_index, sum(number), first(account) WHERE account ~ 'Assets' GROUP BY seq, posting_index",
    "SELECT date, count(*), last(units(balance)), first(balance) GROUP BY date",
    "SELECT year, month, sum(position) GROUP BY year, month",
    "SELECT account, count(*), last(balance) GROUP BY account",
    "SELECT payee, narration, count(*) GROUP BY payee, narration",
];

/// LIMIT and OFFSET keep the groups of their window out of all the groups in the order of their first row, and the
/// total counts all of them, whether the groups come in runs, when only those of the window are built, or not, when
/// every group is built.
#[test]
fn a_window_of_groups_is_that_window_of_all_the_groups() {
    for ledger in [fava_demo_ledger(), load_text(&random_lots_ledger(7, 150))] {
        let run = |sql: &str, count_total: bool| {
            let options = ExecuteOptions { count_total, ..options() };
            Query::compile(sql).unwrap().execute_with_options(&ledger, &Params::new(), &options).unwrap()
        };
        for sql in GROUPED {
            let all = run(sql, false).rows;
            let n = all.len();
            assert!(n > 3, "{sql}");
            for (limit, offset) in [(0, 0), (1, 0), (3, 2), (5, n - 3), (10, n), (n + 5, 0), (7, n / 2)] {
                let windowed = format!("{sql} LIMIT {limit} OFFSET {offset}");
                let expected = all.iter().skip(offset).take(limit).collect::<Vec<_>>();
                let counted = run(&windowed, true);
                assert_eq!(format!("{:?}", counted.rows.iter().collect::<Vec<_>>()), format!("{expected:?}"), "{windowed}");
                assert_eq!(counted.total, Some(n as u64), "{windowed}");
                assert_eq!(format!("{:?}", run(&windowed, false).rows), format!("{:?}", counted.rows), "{windowed}");
            }
        }
    }
}

/// A page deep into groups that come in runs holds only its own groups: the lot rows of 800 postings grouped by
/// posting, page 141 of 5 within a budget of 200 values, which building every group would exceed.
#[test]
fn a_deep_page_of_groups_in_runs_holds_only_its_own_groups() {
    let ledger = load_text(&random_lots_ledger(3, 400));
    let grouped = "SELECT seq, posting_index, sum(number), last(only(currency, units(balance))) GROUP BY seq, posting_index";
    let run = |sql: &str| {
        let options = ExecuteOptions {
            max_result_values: Some(200),
            ..options()
        };
        Query::compile(sql).unwrap().execute_with_options(&ledger, &Params::new(), &options)
    };
    let page = run(&format!("{grouped} LIMIT 5 OFFSET 700")).unwrap();
    assert_eq!((page.rows.len(), page.total), (5, Some(800)));
    // sorted, every group is built
    let sorted = run(&format!("{grouped} ORDER BY seq LIMIT 5 OFFSET 700")).unwrap_err();
    assert_eq!(sorted.kind, crate::QueryErrorKind::TooLarge);
}

/// The fava demo ledger check of [`QUERIES`] runs as [`FAVA_DEMO_PARTS`] tests, so nextest runs them in parallel
/// instead of one test taking two thirds of the suite's wall time. Part `index` holds every `FAVA_DEMO_PARTS`th
/// query from `index` on; the interleaving spreads the slow queries, which sit next to each other in the list (the
/// two `ORDER BY balance` queries alone are two thirds of the time), over the parts, and every query is in exactly
/// one part ([`the_fava_demo_parts_hold_every_query_once`]).
const FAVA_DEMO_PARTS: usize = 8;

fn fava_demo_part(index: usize) -> Vec<&'static str> {
    QUERIES.iter().copied().skip(index).step_by(FAVA_DEMO_PARTS).collect()
}

#[test]
fn the_fava_demo_parts_hold_every_query_once() {
    let mut parts: Vec<&str> = (0..FAVA_DEMO_PARTS).flat_map(fava_demo_part).collect();
    let mut all = QUERIES.to_vec();
    parts.sort_unstable();
    all.sort_unstable();
    assert_eq!(parts, all);
}

#[test]
fn decisions_keep_results_on_the_fava_demo_ledger_part_1_of_8() {
    assert_equivalent(&fava_demo_ledger(), &fava_demo_part(0));
}

#[test]
fn decisions_keep_results_on_the_fava_demo_ledger_part_2_of_8() {
    assert_equivalent(&fava_demo_ledger(), &fava_demo_part(1));
}

#[test]
fn decisions_keep_results_on_the_fava_demo_ledger_part_3_of_8() {
    assert_equivalent(&fava_demo_ledger(), &fava_demo_part(2));
}

#[test]
fn decisions_keep_results_on_the_fava_demo_ledger_part_4_of_8() {
    assert_equivalent(&fava_demo_ledger(), &fava_demo_part(3));
}

#[test]
fn decisions_keep_results_on_the_fava_demo_ledger_part_5_of_8() {
    assert_equivalent(&fava_demo_ledger(), &fava_demo_part(4));
}

#[test]
fn decisions_keep_results_on_the_fava_demo_ledger_part_6_of_8() {
    assert_equivalent(&fava_demo_ledger(), &fava_demo_part(5));
}

#[test]
fn decisions_keep_results_on_the_fava_demo_ledger_part_7_of_8() {
    assert_equivalent(&fava_demo_ledger(), &fava_demo_part(6));
}

#[test]
fn decisions_keep_results_on_the_fava_demo_ledger_part_8_of_8() {
    assert_equivalent(&fava_demo_ledger(), &fava_demo_part(7));
}

#[test]
fn random_ledgers_hold_what_the_running_sums_must_handle() {
    let ledger = load_text(&random_lots_ledger(1, 120));
    let count = |sql: &str| {
        let rows = Query::compile(sql).unwrap().execute(&ledger, &Params::new()).unwrap().rows;
        rows[0][0].as_int().unwrap()
    };
    // 120 transactions of two postings; reductions across lots split into more rows
    assert!(count("SELECT count(*)") > 240);
    // zero costs, negative lots, reductions of open lots, several scales and currencies
    assert!(count("SELECT count(*) WHERE cost_number = 0") > 5);
    assert!(count("SELECT count(*) WHERE account ~ 'Short' AND cost_number IS NOT NULL") > 5);
    assert!(count("SELECT count(*) WHERE account ~ 'Broker' AND number < 0 AND cost_number IS NOT NULL") > 5);
    assert!(count("SELECT count(*) WHERE str(number) ~ '[.]000$'") > 5);
    assert!(count("SELECT count(*) WHERE cost_currency = 'EUR'") > 5);
    let mixed = Query::compile("SELECT units(balance) WHERE account ~ 'Broker|Short'").unwrap();
    let rows = mixed.execute(&ledger, &Params::new()).unwrap().rows;
    // the running balance holds lots of both signs in one currency at some point
    let balances = Query::compile("SELECT balance WHERE account ~ 'Broker|Short'")
        .unwrap()
        .execute(&ledger, &Params::new())
        .unwrap()
        .rows;
    assert!(balances.iter().any(|row| {
        let crate::Value::Inventory(inventory) = &row[0] else { return false };
        let signs = inventory
            .positions()
            .filter(|it| it.units.commodity == "STK")
            .map(|it| it.units.number > 0.into())
            .collect::<std::collections::HashSet<_>>();
        signs.len() == 2
    }));
    assert_eq!(rows.len(), balances.len());
}

#[test]
fn decisions_keep_results_with_mixed_lots_scales_and_zero_costs() {
    for seed in 1..=6 {
        assert_equivalent(&load_text(&random_lots_ledger(seed, 120)), QUERIES);
    }
}

/// The running sums give exactly what `units()` and `cost()` of the running balance give,
/// row by row, however the lots of a currency mix signs, scales and zero costs.
#[test]
fn running_sums_equal_the_functions_of_the_balance() {
    for seed in 1..=40 {
        let ledger = load_text(&random_lots_ledger(seed, 60));
        assert_equivalent(
            &ledger,
            &[
                "SELECT units(balance), cost(balance)",
                "SELECT units(balance), cost(balance) WHERE account ~ 'Broker'",
                "SELECT units(balance), cost(balance) WHERE account ~ 'Short|Cash'",
                "SELECT account, units(account_balance), cost(account_balance)",
                "SELECT units(account_balance), cost(account_balance), units(balance) WHERE account ~ 'Broker|Short' ORDER BY seq DESC LIMIT 30",
                "SELECT count(*) WHERE number(only('STK', units(account_balance))) > 1",
            ],
        );
    }
}

#[test]
fn explain_shows_the_decisions() {
    let explain = |sql: &str| Query::compile(sql).unwrap().explain();
    let journal = explain("JOURNAL 'Broker' AT cost");
    assert!(journal.contains("rewrite: cost(balance) -> running cost\n"), "{journal}");
    assert!(journal.contains("balance: deferred targets [6]\n"), "{journal}");
    let top = explain("SELECT date, balance ORDER BY date DESC LIMIT 10");
    assert!(top.contains("limit: 10 (top-k while scanning)\nbalance: deferred targets [1]\n"), "{top}");
    let page = Query::compile_with_params(
        "SELECT date, balance ORDER BY date DESC LIMIT :size OFFSET :offset",
        &ParamTypes::new().bind("size", DataType::Int).bind("offset", DataType::Int),
    )
    .unwrap()
    .explain();
    assert!(page.contains("limit: :size offset :offset (top-k while scanning)\n"), "{page}");
    // a top-k page builds its targets but the ORDER BY keys for the rows it keeps only
    let wide = explain("SELECT seq, payee, tags, metas FROM #entries ORDER BY seq DESC LIMIT 10");
    assert!(
        wide.contains("limit: 10 (top-k while scanning)\nlate targets: [1, 2, 3] (built for the kept rows only)\n"),
        "{wide}"
    );
    assert!(!explain("SELECT date, payee ORDER BY date DESC").contains("late targets"));
    assert!(!explain("SELECT date, number / 2 ORDER BY date DESC LIMIT 3").contains("late targets"));
    // a CASE is built late when every branch is infallible, while scanning when one can fail
    let case = explain("SELECT seq, CASE WHEN flag = 'P' THEN 'pad' ELSE payee END FROM #entries ORDER BY seq DESC LIMIT 10");
    assert!(case.contains("late targets: [1] (built for the kept rows only)\n"), "{case}");
    let failing = explain("SELECT seq, CASE WHEN seq > 1 THEN seq / 0 ELSE 0 END FROM #entries ORDER BY seq DESC LIMIT 10");
    assert!(!failing.contains("late targets"), "{failing}");
    assert!(explain("SELECT DISTINCT account LIMIT 3 OFFSET 6").contains("limit: 3 offset 6 (stops the scan)\n"));
    let grouped = explain("SELECT account, last(balance), min(balance) GROUP BY account LIMIT 2");
    assert!(
        grouped.contains("limit: 2 (first groups only)\nbalance: running while scanning, deferred agg#0\n"),
        "{grouped}"
    );
    assert!(explain("SELECT DISTINCT account LIMIT 3").contains("limit: 3 (stops the scan)\n"));
    assert!(explain("SELECT DISTINCT account ORDER BY account LIMIT 3").contains("limit: 3\n"));
    assert!(explain("SELECT date, balance ORDER BY balance").contains("balance: running while scanning\n"));
    // the account balances are deferred like the balance, and read while scanning by a filter
    let deferred = explain("SELECT date, account_balance ORDER BY date DESC LIMIT 1");
    assert!(deferred.contains("account_balance: deferred targets [1]\n"), "{deferred}");
    let filter = explain("SELECT date WHERE number(only('USD', units(account_balance))) > 0");
    assert!(
        filter.contains("rewrite: units(account_balance) -> running account units\n") && filter.contains("account_balance: running while scanning\n"),
        "{filter}"
    );
    let both = explain("SELECT balance, account_balance LIMIT 2");
    assert!(both.contains("balance, account_balance: deferred targets [0, 1]\n"), "{both}");
    // value() prices by date: it is not rewritten
    let value = explain("JOURNAL AT value");
    assert!(!value.contains("rewrite:") && value.contains("balance: deferred targets [6]\n"), "{value}");
    let scoped = explain("SELECT date WHERE year > 2020 AND (account = 'Assets:Bank' OR account IN ('Assets:Cash', NULL))");
    assert!(
        scoped.contains("scan: the rows of the accounts 'Assets:Bank', 'Assets:Cash', NULL\n"),
        "{scoped}"
    );
    assert!(!explain("SELECT date WHERE account ~ 'Assets:Bank'").contains("scan:"));
    // under(), with a literal (a prepared test) or a parameter, and a hashed IN list
    let under = explain("SELECT date WHERE under(account, 'Assets:US') OR account IN ('Assets:Cash', 'Expenses:Fees')");
    assert!(
        under.contains("scan: the rows of the accounts under 'Assets:US', 'Assets:Cash', 'Expenses:Fees'\n"),
        "{under}"
    );
    let under = Query::compile_with_params("SELECT date WHERE under(account, :root)", &ParamTypes::new().bind("root", DataType::Str))
        .unwrap()
        .explain();
    assert!(under.contains("scan: the rows of the accounts under :root\n"), "{under}");
}

/// A plan that does not read `balance` keeps no running total at all.
#[test]
fn plans_without_balance_keep_no_running_total() {
    for sql in [
        "SELECT *",
        "SELECT account, sum(position) GROUP BY account",
        "BALANCES",
        "SELECT date, position ORDER BY date LIMIT 3",
    ] {
        let query = Query::compile(sql).unwrap();
        assert!(!query.plan.execution.running.used(), "{sql}");
        assert!(query.plan.execution.running.totals.is_empty() && !query.plan.execution.running.eager, "{sql}");
        assert!(!query.explain().contains("balance"), "{sql}: {}", query.explain());
    }
    assert!(Query::compile("SELECT units(balance)").unwrap().plan.execution.running.used());
}

/// The data tables compute some columns only when a plan reads them (the true balance of
/// `#balances`, the figures of `#budgets`): top-k, stopping the scan, first groups, DISTINCT
/// and HAVING over them return what the naive plans return.
#[test]
fn decisions_keep_results_on_the_data_tables() {
    const DATA_QUERIES: &[&str] = &[
        "SELECT date, account, actual, passed, discrepancy FROM #balances ORDER BY date DESC LIMIT 5",
        "SELECT account, count(*) FROM #balances WHERE NOT passed GROUP BY account LIMIT 1",
        "SELECT DISTINCT passed FROM #balances LIMIT 1",
        "SELECT account, last(actual) FROM #balances GROUP BY account HAVING count(*) > 1 ORDER BY account",
        "SELECT * FROM #documents ORDER BY date DESC LIMIT 2",
        "SELECT source, count(*) FROM #documents GROUP BY source LIMIT 1",
        "SELECT DISTINCT transaction_id FROM #documents LIMIT 1",
        "SELECT kind, id, span_start, span_end FROM #errors ORDER BY span_start DESC LIMIT 1",
        "SELECT name, last(available), last(closed) FROM #budgets GROUP BY name LIMIT 1",
        "SELECT date, name, activity FROM #budgets ORDER BY activity DESC LIMIT 2",
        "SELECT DISTINCT name FROM #budgets WHERE number(available) < 0 LIMIT 1",
        "SELECT name, sum(added) FROM #budgets GROUP BY name HAVING count(*) > 2",
        "SELECT * FROM #budget_events ORDER BY timestamp DESC LIMIT 2",
        "SELECT type, count(*) FROM #budget_events GROUP BY type LIMIT 2",
    ];
    let ledger = load_text(
        r#"
option "operating_currency" "CNY"
1970-01-01 commodity CNY
1970-01-01 commodity USD
1970-01-01 open Assets:Bank
1970-01-01 open Equity:Open
1970-01-01 open Expenses:Food
  budget: food
1970-01-01 open Expenses:Travel
  budget: travel
2024-01-01 price USD 7 CNY
2024-01-01 budget food CNY
2024-01-01 budget travel CNY
2024-01-01 budget-add food 100 CNY
2024-01-02 * "Shop" "lunch"
  document: "a.pdf"
  Expenses:Food 30 CNY
    document: "b.pdf"
  Assets:Bank
2024-01-03 balance Assets:Bank -30 CNY
2024-01-04 balance Assets:Bank -20 CNY
2024-02-01 budget-transfer food travel 20 CNY
2024-02-02 * "Airline" "flight"
  Expenses:Travel 10 USD
  Assets:Bank -70 CNY
2024-02-03 document Assets:Bank "statement.pdf"
2024-02-04 balance Assets:Bank 0 CNY with pad Equity:Open
2024-03-01 budget-close travel
2024-03-02 * "Shop" "unbalanced"
  Expenses:Food 10 CNY
  Assets:Bank -9 CNY
"#,
    );
    assert_equivalent(&ledger, DATA_QUERIES);
    assert_equivalent(&fava_demo_ledger(), &DATA_QUERIES[..4]);
    // the data tables read the cache of the ledger too
    let mut ledger = ledger;
    assert_cache_keeps_results(&mut ledger, &DATA_QUERIES.iter().map(|sql| (*sql, Params::new())).collect::<Vec<_>>());
}

/// Queries whose filter scopes them to some accounts, with the parameters they are run with:
/// constants, string and set parameters, NULL, `OR`s, conjuncts before the scoping one, and the
/// columns that depend on the scope (the running balance, the account balance, LIMIT).
fn scoped_queries() -> Vec<(&'static str, Params)> {
    let account = |name: &str| Params::new().bind("account", name);
    let accounts = |names: &[&str]| Params::new().bind("accounts", Value::Set(names.iter().map(|it| it.to_string()).collect()));
    let root = |name: &str| Params::new().bind("root", name);
    vec![
        ("SELECT date, account, position, balance, account_balance WHERE account = :account", account("Assets:Broker")),
        ("SELECT date, account, position, balance, account_balance WHERE account = :account", account("Assets:US:BofA:Checking")),
        ("SELECT date, balance, account_balance WHERE account = :account ORDER BY date DESC LIMIT 3", account("Assets:Short")),
        ("SELECT count(*), last(balance) WHERE account = :account", Params::new().bind("account", Value::Null)),
        ("SELECT count(*) WHERE account = :account", account("Nowhere")),
        (
            "SELECT account, sum(position), last(account_balance) WHERE account IN :accounts GROUP BY account ORDER BY account",
            accounts(&["Assets:Cash", "Expenses:Fees", "Assets:US:Vanguard:Cash", "Income:US:Hoogle:Salary"]),
        ),
        ("SELECT date, account, balance WHERE account IN :accounts", accounts(&[])),
        ("SELECT date, account, balance WHERE account IN :accounts", Params::new().bind("accounts", Value::Null)),
        (
            "SELECT date, account, units(balance), account_balance WHERE year >= 2015 AND (account = :account OR account IN ('Assets:Cash', 'Expenses:Food:Restaurant')) LIMIT 40",
            account("Assets:Broker"),
        ),
        (
            "SELECT date, account, number WHERE number > 0 AND account IN (:account, 'Assets:Short', NULL) AND payee ~ 'trade|Hoogle'",
            account("Assets:US:BofA:Checking"),
        ),
        (
            "SELECT account, first(account_balance), count(*) WHERE 'Assets:Cash' = account OR account = :account GROUP BY account",
            account("Assets:US:Vanguard:Cash"),
        ),
        // under(), with a parameter and with a literal, and IN lists of constants
        (
            "SELECT date, account, balance, account_balance WHERE under(account, :root) LIMIT :size OFFSET 3",
            root("Assets:US").bind("size", 25i64),
        ),
        ("SELECT account, count(*), last(account_balance) WHERE under(account, :root) GROUP BY account", root("Assets")),
        ("SELECT count(*), last(balance) WHERE under(account, :root)", root("Assets:U")),
        ("SELECT count(*) WHERE under(account, :root)", Params::new().bind("root", Value::Null).bind("size", 1i64)),
        (
            "SELECT date, account, balance WHERE year >= 2016 AND (under(account, 'Assets:US:Vanguard') OR account IN ('Assets:Cash', 'Expenses:Fees', NULL))",
            Params::new(),
        ),
        (
            "SELECT DISTINCT account WHERE account IN ('Assets:Broker', 'Assets:Short') AND icontains(payee, 'trade')",
            Params::new(),
        ),
        // an account named both on its own and as an ancestor, or twice, is read once: its rows
        // twice would count twice in the running balance
        (
            "SELECT date, account, position, balance WHERE account = :account OR under(account, :account)",
            account("Assets:Cash"),
        ),
        (
            "SELECT date, account, position, balance WHERE account = :account OR under(account, :account)",
            account("Assets:US:BofA:Checking"),
        ),
        (
            "SELECT date, account, balance WHERE account IN ('Assets:Cash', 'Assets:Cash', 'Assets:US:BofA:Checking', 'Assets:US:BofA:Checking')",
            Params::new(),
        ),
        (
            "SELECT account, count(*), last(balance) WHERE account IN (:account, :account) OR under(account, :account) OR under(account, :account) GROUP BY account",
            account("Assets:US:Vanguard"),
        ),
    ]
}

fn compile_scoped(sql: &str) -> Query {
    let types = ParamTypes::new()
        .bind("account", DataType::Str)
        .bind("accounts", DataType::Set)
        .bind("root", DataType::Str)
        .bind("size", DataType::Int);
    Query::compile_with_params(sql, &types).unwrap_or_else(|err| panic!("{sql}: {err}"))
}

/// An account-scoped scan returns what a scan of every row returns: the scope only leaves out
/// rows the filter drops, and keeps every row of its accounts.
#[test]
fn account_scopes_keep_results() {
    let ledgers = [fava_demo_ledger(), load_text(&random_lots_ledger(3, 120))];
    for ledger in &ledgers {
        let mut with_rows = 0;
        for (sql, params) in scoped_queries() {
            let query = compile_scoped(sql);
            assert!(query.plan.execution.scope.is_some(), "{sql}");
            let scoped = query.execute_with_options(ledger, &params, &options());
            with_rows += scoped.as_ref().is_ok_and(|it| it.rows.iter().any(|row| row[0] != Value::Int(0))) as usize;
            let full = unscoped(compile_scoped(sql)).execute_with_options(ledger, &params, &options());
            assert_same(sql, full, scoped);
        }
        // the accounts of both ledgers are named, so most scopes hold rows
        assert!(with_rows >= 5, "{with_rows}");
        for sql in QUERIES {
            let query = Query::compile(sql).unwrap();
            if query.plan.execution.scope.is_some() {
                let scoped = query.execute_with_options(ledger, &Params::new(), &options());
                let full = unscoped(Query::compile(sql).unwrap()).execute_with_options(ledger, &Params::new(), &options());
                assert_same(sql, full, scoped);
            }
        }
    }
}

/// Filters that the rows of other accounts can pass read every row, and return what a scan of
/// every row returns: `!=`, `NOT`, `NOT IN` a list or a set, and a scoping conjunct after one
/// that may fail, which a scan of every row evaluates for the other accounts too (and stops at
/// its error there).
#[test]
fn filters_other_accounts_can_pass_read_every_row() {
    let account = |name: &str| Params::new().bind("account", name);
    let accounts = |names: &[&str]| Params::new().bind("accounts", Value::Set(names.iter().map(|it| it.to_string()).collect()));
    let queries = [
        ("SELECT date, account, balance WHERE account != 'Assets:Cash'", Params::new()),
        ("SELECT date, account, balance WHERE account != :account", account("Assets:US:BofA:Checking")),
        ("SELECT date, account, balance WHERE NOT account = :account", account("Assets:Cash")),
        (
            "SELECT date, account, balance WHERE account NOT IN ('Assets:Cash', 'Assets:US:BofA:Checking')",
            Params::new(),
        ),
        (
            "SELECT date, account, balance WHERE account NOT IN :accounts",
            accounts(&["Assets:Cash", "Assets:US:BofA:Checking"]),
        ),
        // the product overflows for accounts of 13 characters or more, not for Assets:Cash
        (
            "SELECT date, account WHERE length(account) * 800000000000000000 > 0 AND account = :account",
            account("Assets:Cash"),
        ),
    ];
    for ledger in [fava_demo_ledger(), load_text(&random_lots_ledger(3, 120))] {
        for (sql, params) in &queries {
            let query = compile_scoped(sql);
            assert!(query.plan.execution.scope.is_none(), "{sql}");
            let result = query.execute_with_options(&ledger, params, &options());
            let full = unscoped(compile_scoped(sql)).execute_with_options(&ledger, params, &options());
            assert_same(sql, full, result);
        }
        // a scan of every row reaches the accounts the product overflows for
        let (sql, params) = &queries[5];
        let error = compile_scoped(sql).execute_with_options(&ledger, params, &options()).unwrap_err();
        assert!(error.message.contains("integer overflow"), "{sql}: {}", error.message);
    }
}

/// Every query returns the same rows from a cache that the other queries built as from a fresh
/// one, which it builds itself.
fn assert_cache_keeps_results(ledger: &mut Ledger, queries: &[(&str, Params)]) {
    let run = |ledger: &Ledger, sql: &str, params: &Params| compile_scoped(sql).execute_with_options(ledger, params, &options());
    // every query on the cache the earlier ones built, in reverse order
    let warm = queries.iter().rev().map(|(sql, params)| run(ledger, sql, params)).collect::<Vec<_>>();
    assert!(ledger.derived.is_initialized());
    for ((sql, params), warm) in queries.iter().rev().zip(warm) {
        ledger.derived = Default::default();
        assert_same(sql, run(ledger, sql, params), warm);
    }
}

/// A query reads the same rows from a cache other queries built as from a fresh one: what the
/// cache holds does not depend on the query that built it.
#[test]
fn the_ledger_cache_keeps_results() {
    for mut ledger in [fava_demo_ledger(), load_text(&random_lots_ledger(5, 120))] {
        let queries = QUERIES.iter().map(|sql| (*sql, Params::new())).chain(scoped_queries()).collect::<Vec<_>>();
        assert_cache_keeps_results(&mut ledger, &queries);
    }
}

/// A table that generates its rows (the months of `#budgets`) generates none after the date
/// bound of its filter, and returns what generating every row returns: the bound only leaves
/// out rows the filter drops, and the months before it are computed from the same entries.
#[test]
fn date_bounds_keep_results() {
    let ledger = load_text(
        r#"
option "operating_currency" "CNY"
1970-01-01 commodity CNY
1970-01-01 commodity USD
1970-01-01 open Assets:Bank
1970-01-01 open Expenses:Food
  budget: food
1970-01-01 open Expenses:Travel
  budget: travel
2024-01-01 price USD 7 CNY
2024-01-01 budget food CNY
2024-03-01 budget travel CNY
2024-01-01 budget-add food 100 CNY
2024-01-10 * "Shop" "lunch"
  Expenses:Food 30 CNY
  Assets:Bank
2024-02-10 * "Shop" "dinner"
  Expenses:Food 10 USD
  Assets:Bank -70 CNY
2024-03-05 budget-transfer food travel 20 CNY
2024-04-02 * "Airline" "flight"
  Expenses:Travel 15 CNY
  Assets:Bank
2024-05-01 budget-close travel
"#,
    );
    let month = |m: u32| Params::new().bind("month", NaiveDate::from_ymd_opt(2024, m, 1).unwrap());
    let queries: Vec<(&str, Params)> = vec![
        ("SELECT name, date, assigned, activity, available, closed FROM #budgets WHERE date <= 2024-02-01", Params::new()),
        ("SELECT name, date, available FROM #budgets WHERE date < 2024-03-01", Params::new()),
        ("SELECT name, date, available FROM #budgets WHERE 2024-03-15 >= date AND name = 'food'", Params::new()),
        ("SELECT name, available FROM #budgets WHERE date = :month", month(3)),
        ("SELECT name, available FROM #budgets WHERE date = :month", month(1)),
        ("SELECT name, available FROM #budgets WHERE yearmonth(date) = :month", month(4)),
        ("SELECT name, available FROM #budgets WHERE yearmonth(date) < :month", month(3)),
        (
            "SELECT name, last(date), CASE WHEN last(date) < :month THEN last(available) ELSE last(assigned) END FROM #budgets WHERE date <= :month GROUP BY name ORDER BY name",
            month(6),
        ),
        ("SELECT name, count(*) FROM #budgets WHERE name != 'x' AND date <= :month GROUP BY name", month(2)),
        // today, in the documented idiom for this month
        ("SELECT name, date, available FROM #budgets WHERE date = yearmonth(today())", Params::new()),
        ("SELECT name, count(*) FROM #budgets WHERE date <= today() GROUP BY name", Params::new()),
        // a NULL bound holds for no row
        ("SELECT count(*) FROM #budgets WHERE date <= :month", Params::new().bind("month", Value::Null)),
        // a month before every budget
        ("SELECT name FROM #budgets WHERE date <= :month", Params::new().bind("month", NaiveDate::from_ymd_opt(2023, 12, 31).unwrap())),
    ];
    let types = ParamTypes::new().bind("month", DataType::Date);
    for (sql, params) in &queries {
        let compile = || Query::compile_with_params(sql, &types).unwrap_or_else(|err| panic!("{sql}: {err}"));
        assert!(compile().plan.execution.until.is_some(), "{sql}");
        assert_equivalent_with(&ledger, &[sql], &types, params);
        let bounded = compile().execute_with_options(&ledger, params, &options());
        let full = unscoped(compile()).execute_with_options(&ledger, params, &options());
        assert_same(sql, full, bounded);
    }
    // filters that keep later rows generate every row
    for sql in [
        "SELECT name FROM #budgets WHERE date >= 2024-02-01",
        "SELECT name FROM #budgets WHERE date != 2024-02-01",
        "SELECT name FROM #budgets WHERE date <= 2024-02-01 OR name = 'food'",
        "SELECT name FROM #budgets WHERE NOT date > 2024-02-01",
        // a conjunct that may fail before the bound: generating every row evaluates it on later rows
        "SELECT name FROM #budgets WHERE year * 1000 > 0 AND date <= 2024-02-01",
        // the other tables read their rows
        "SELECT date FROM #budget_events WHERE date <= 2024-02-01",
        "SELECT date WHERE date <= 2024-02-01",
    ] {
        let query = Query::compile(sql).unwrap_or_else(|err| panic!("{sql}: {err}"));
        assert!(query.plan.execution.until.is_none(), "{sql}");
    }
}
