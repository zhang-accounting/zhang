//! The projector: the stage between the optimizer and the executor that decides which parts
//! of the rows of the query's table an execution builds.
//!
//! It takes the columns of the plan's table the optimized plan reads
//! ([`Plan::referenced_columns`]: targets, including the hidden GROUP BY / ORDER BY ones, the
//! filter and aggregate arguments) and turns them into a [`Projection`]. The row source
//! ([`crate::table::Dataset`]) then builds only what the projected columns read. A record
//! table computes each column when it is read, and its builder may skip work that only
//! unprojected columns need. For the `postings` table:
//!
//! - Lot booking runs over every posting held at cost: the lot a posting reduces depends on
//!   all earlier postings, and how a reduction splits across lots decides the rows themselves
//!   (their number and units), so even `SELECT count(*)` needs it. It runs once per loaded
//!   ledger, whatever the projection, and its rows are kept with the ledger
//!   ([`crate::table::LedgerCache`]). Only whether an execution's rows *carry* the cost of
//!   their lot depends on the projection (`position`, `cost_*`, `weight`), and so does the
//!   price annotation (`price`, `weight`).
//! - Transaction columns (`id`, `flag`, `payee`, `narration`, `description`, `tags`,
//!   `links`, `other_accounts`) are never copied up front: a row points at the stored
//!   transaction and a column reads it when it is evaluated.
//!
//! Evaluation is lazy per column as well: a column value exists only while an expression
//! reads it, and predicates over string and set columns read them in place, without
//! copying them into a [`Value`] ([`borrowed_str`], [`set_membership`]).
//!
//! The running `balance` is materialized late ([`plan_running`]): a row's balance is an
//! inventory of every open lot, so building one for every filtered row costs rows × lots.
//! Unless something needs the balances while scanning (ORDER BY or DISTINCT over them, a
//! GROUP BY key, an aggregate other than `first()` / `last()`), the targets that read it are
//! left empty by the scan, and once grouping, ORDER BY and LIMIT have chosen the result rows,
//! one replay of the filtered rows in ledger order evaluates them for those rows only. A
//! `first()` / `last()` over it remembers the row it picks and is evaluated there. A plan that
//! does not read `balance` keeps no running total at all.
//!
//! `account_balance`, the running balance of each posting's own account, is a running total
//! too ([`Running::AccountBalance`]), materialized the same way: the scan only keeps the
//! balances of the accounts (one inventory each) when the filter, ORDER BY, DISTINCT, a
//! GROUP BY key or an aggregate reads it, and otherwise the replay sums the rows of every
//! account up to the chosen rows, whatever the filter, and evaluates it for those rows only.
//! A row's value is a snapshot of its account's inventory, so only the rows a query holds
//! copy one, and the result budget counts them.

use std::fmt;

use crate::compiler::{AggregateCall, CExpr, Plan, Running, RunningPlan};
use crate::executor::Env;
use crate::functions::{AggregateKind, ParamType};
use crate::table::{Borrow, ColumnDef, Reads, RowRef, Table};
use crate::value::Value;

/// The columns of its table one plan reads, and the posting row parts they need.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Projection {
    table: &'static Table,
    /// bit `i` is set when `table.columns[i]` is read
    columns: u64,
    reads: Reads,
}

impl PartialEq for Projection {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self.table, other.table) && self.columns == other.columns && self.reads == other.reads
    }
}

impl Eq for Projection {}

/// Decide how the plan materializes its running totals (see the module docs).
pub(crate) fn plan_running(plan: &mut Plan) {
    let mut totals = vec![];
    // the filter can read the account balances (not `balance`, which it decides)
    let mut in_filter = vec![];
    if let Some(filter) = &plan.filter {
        collect_running(filter, &mut in_filter);
    }
    totals.extend(in_filter.iter().copied());
    for target in &plan.targets {
        collect_running(&target.expr, &mut totals);
    }
    for aggregate in &plan.aggregates {
        if let Some(arg) = &aggregate.arg {
            collect_running(arg, &mut totals);
        }
    }
    totals.sort();
    totals.dedup();
    if totals.is_empty() {
        plan.execution.running = RunningPlan::default();
        return;
    }
    let reads_running = |expr: &CExpr| {
        let mut found = vec![];
        collect_running(expr, &mut found);
        !found.is_empty()
    };
    let mut running = RunningPlan {
        totals,
        eager: !in_filter.is_empty(),
        ..RunningPlan::default()
    };
    match &plan.group_keys {
        None => {
            for (idx, target) in plan.targets.iter().enumerate() {
                if !reads_running(&target.expr) {
                    continue;
                }
                let sorted_on = plan.order.iter().any(|(key, _)| *key == idx);
                if idx < plan.visible && !plan.distinct && !sorted_on && infallible(&target.expr) {
                    running.deferred_targets.push(idx);
                } else {
                    running.eager = true;
                }
            }
        }
        Some(keys) => {
            running.eager |= keys.iter().any(|idx| reads_running(&plan.targets[*idx].expr));
            for (idx, aggregate) in plan.aggregates.iter().enumerate() {
                match &aggregate.arg {
                    Some(arg) if reads_running(arg) => {
                        if picks_a_row(aggregate) && never_null(arg) {
                            running.deferred_aggregates.push(idx);
                        } else {
                            running.eager = true;
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    plan.execution.running = running;
}

fn collect_running(expr: &CExpr, totals: &mut Vec<Running>) {
    if let CExpr::Running(total) = expr {
        totals.push(*total);
    }
    for child in expr.children() {
        collect_running(child, totals);
    }
}

fn picks_a_row(aggregate: &AggregateCall) -> bool {
    matches!(aggregate.function.kind, AggregateKind::First | AggregateKind::Last)
}

/// Functions that return a value, never NULL or an error, for arguments of their types,
/// as long as they take no integer (`abs` and `neg` can overflow one).
const TOTAL_FUNCTIONS: &[&str] = &[
    "units",
    "cost",
    "value",
    "convert",
    "str",
    "only",
    "filter_currency",
    "possign",
    "abs",
    "neg",
    "icontains",
    "any_icontains",
    "intersects",
    "under",
];

fn total_function(expr: &CExpr) -> bool {
    match expr {
        CExpr::Scalar { function, .. } => TOTAL_FUNCTIONS.contains(&function.name) && !function.params.contains(&ParamType::Exact(crate::value::DataType::Int)),
        _ => false,
    }
}

/// Whether evaluating the expression can never fail, so evaluating it for fewer rows
/// changes nothing but the work done.
pub(crate) fn infallible(expr: &CExpr) -> bool {
    let node = match expr {
        CExpr::Const(_) | CExpr::Column(_) | CExpr::Running(_) | CExpr::Param(_) | CExpr::WidenInt(_) | CExpr::Target(_) => true,
        CExpr::Not(_) | CExpr::And(_) | CExpr::Or(_) | CExpr::Compare { .. } | CExpr::InSet { .. } | CExpr::InList { .. } | CExpr::IsNull { .. } => true,
        CExpr::InConst { .. } | CExpr::StrTest { .. } | CExpr::Case { .. } => true,
        CExpr::Scalar { .. } => total_function(expr),
        CExpr::Aggregate(_) | CExpr::Neg(..) | CExpr::Arith { .. } | CExpr::Regex { .. } => false,
    };
    node && expr.children().into_iter().all(infallible)
}

/// Whether the expression is never NULL and never fails, so `first()` / `last()` over it
/// pick the first / last row of their group.
fn never_null(expr: &CExpr) -> bool {
    match expr {
        CExpr::Running(_) => true,
        CExpr::Const(value) => !value.is_null(),
        CExpr::Scalar { args, .. } => total_function(expr) && args.iter().all(never_null),
        _ => false,
    }
}

/// Build the projection of an optimized plan.
pub(crate) fn project(plan: &Plan) -> Projection {
    let referenced = plan.referenced_columns();
    Projection::of_columns(plan.table, plan.table.columns.iter().filter(|column| referenced.contains(column.name)))
}

impl Projection {
    /// Every column of the `postings` table: rows carry everything.
    #[cfg(test)]
    pub fn all() -> Projection {
        Projection::all_of(&crate::table::POSTINGS)
    }

    /// Every column of `table`.
    #[cfg(test)]
    pub fn all_of(table: &'static Table) -> Projection {
        Projection::of_columns(table, table.columns.iter())
    }

    fn of_columns<'c>(table: &'static Table, columns: impl Iterator<Item = &'c ColumnDef>) -> Projection {
        let mut projection = Projection {
            table,
            columns: 0,
            reads: Reads::default(),
        };
        for column in columns {
            projection.columns |= projection.bit(column);
            projection.reads.cost |= column.reads.cost;
            projection.reads.price |= column.reads.price;
        }
        projection
    }

    /// The table whose columns are projected.
    pub fn table(&self) -> &'static Table {
        self.table
    }

    /// Whether the column is projected; never for a column of another table.
    pub fn contains(&self, column: &ColumnDef) -> bool {
        self.index(column).is_some_and(|index| self.columns & (1 << index) != 0)
    }

    fn index(&self, column: &ColumnDef) -> Option<usize> {
        self.table.columns.iter().position(|it| std::ptr::eq(it, column))
    }

    fn bit(&self, column: &ColumnDef) -> u64 {
        1 << self.index(column).expect("a column of the projected table")
    }

    /// Whether booked rows keep the cost of their lot.
    pub fn keeps_cost(&self) -> bool {
        self.reads.cost
    }

    /// The same columns, with booked rows that keep the cost of their lot (the period
    /// modifiers sum balances at cost, whatever the query reads).
    pub fn with_cost(mut self) -> Projection {
        self.reads.cost = true;
        self
    }

    /// Whether rows keep the price annotation of their posting.
    pub fn keeps_price(&self) -> bool {
        self.reads.price
    }

    /// The projected column names, in name order (like [`Plan::referenced_columns`]).
    pub fn names(&self) -> Vec<&'static str> {
        let mut names = self
            .table
            .columns
            .iter()
            .filter(|column| self.contains(column))
            .map(|column| column.name)
            .collect::<Vec<_>>();
        names.sort_unstable();
        names
    }
}

/// `[account, position] (2 of 35 columns)`
impl fmt::Display for Projection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let names = self.names();
        write!(f, "[{}] ({} of {} columns)", names.join(", "), names.len(), self.table.columns.len())
    }
}

/// The string an operand reads, borrowed from the row, the plan or the parameters: a string
/// column that supports it ([`Borrow::Str`]), a string constant or a string parameter.
/// `Some(None)` is a NULL string; `None` when the operand has to be evaluated instead.
pub(crate) fn borrowed_str<'r>(expr: &'r CExpr, env: &Env<'r, '_>) -> Option<Option<&'r str>> {
    match expr {
        CExpr::Column(column) => match (column.borrow, env.data, env.row) {
            (Borrow::Str(get), Some(data), Some(RowRef::Posting(row))) => Some(get(data, row)),
            _ => None,
        },
        CExpr::Const(Value::Str(text)) => Some(Some(text)),
        CExpr::Param(param) => match env.params.get(param) {
            Some(Value::Str(text)) => Some(Some(text)),
            _ => None,
        },
        _ => None,
    }
}

/// A membership test of a set column of the current row ([`Borrow::Contains`]), which does
/// not build the set; `None` when the operand has to be evaluated instead.
pub(crate) fn set_membership<'r, 'a>(expr: &'r CExpr, env: &Env<'r, 'a>) -> Option<impl Fn(&str) -> bool + use<'r, 'a>> {
    match expr {
        CExpr::Column(column) => match (column.borrow, env.data, env.row) {
            (Borrow::Contains(contains), Some(data), Some(RowRef::Posting(row))) => Some(move |needle: &str| contains(data, row, needle)),
            _ => None,
        },
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;

    use chrono::NaiveDate;
    use zhang_core::data_source::LocalFileSystemDataSource;
    use zhang_core::data_type::text::ZhangDataType;
    use zhang_core::ledger::Ledger;

    use super::*;
    use crate::executor::{execute, Budget, RegexCache};
    use crate::params::Params;
    use crate::table::{column, Dataset, Limits, Record, Scope, COLUMNS};
    use crate::Query;

    fn load(dir: PathBuf) -> Ledger {
        let source = LocalFileSystemDataSource::new(ZhangDataType {});
        Ledger::load_with_data_source(dir, "main.zhang".to_owned(), Arc::new(source)).expect("cannot load ledger")
    }

    fn fava_demo_ledger() -> Ledger {
        load(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../integration-tests/fava-demo-ledger"))
    }

    fn load_text(content: &str) -> Ledger {
        let dir = tempfile::tempdir().expect("tempdir").into_path();
        std::fs::write(dir.join("main.zhang"), content).expect("write ledger");
        load(dir)
    }

    /// Lots (FIFO and LIFO, split reductions, `{}` and cost-only reductions, labels and
    /// dates), `@` / `@@` prices, prices for valuation, metadata, tags and links.
    const LEDGER: &str = r#"
option "operating_currency" "USD"

1970-01-01 commodity USD
1970-01-01 commodity EUR
1970-01-01 commodity AAPL

1970-01-01 open Assets:Bank
1970-01-01 open Assets:Fifo
1970-01-01 open Assets:Lifo
  booking_method: "LIFO"
1970-01-01 open Expenses:Food
1970-01-01 open Expenses:Travel
1970-01-01 open Income:Gains
1970-01-01 open Equity:Opening

2024-01-01 price EUR 1.10 USD
2024-02-15 price AAPL 120.00 USD
2024-03-01 price AAPL 150.00 USD

2024-01-05 * "Cafe" "lunch" #food ^receipt-1
  category: "meal"
  Expenses:Food    12.50 USD
  Assets:Bank

2024-02-01 * "Broker" "buy"
  Assets:Fifo     5 AAPL {100.00 USD}
  Assets:Lifo     5 AAPL {100.00 USD, 2024-01-15, "first"}
  Assets:Bank    -1000.00 USD

2024-02-10 * "Broker" "buy more"
  Assets:Fifo     5 AAPL {110.00 USD}
  Assets:Lifo     5 AAPL {110.00 USD}
  Assets:Bank    -1100.00 USD

2024-03-05 * "Broker" "sell"
  Assets:Fifo    -7 AAPL {} @ 150.00 USD
  Assets:Lifo    -2 AAPL {100.00 USD}
  Assets:Lifo    -6 AAPL {} @@ 900.00 USD
  Assets:Bank    2250.00 USD
  Income:Gains   -680.00 USD

2024-03-10 * "Trip" "hotel" #travel
  Expenses:Travel  100.00 EUR @@ 110.00 USD
  Assets:Bank     -110.00 USD

2024-04-02 balance Assets:Bank 2000.00 USD with pad Equity:Opening
"#;

    /// Queries covering every column, booking (lots and their costs), valuation, metadata,
    /// FROM + WHERE, in-place predicates, grouping, ordering, DISTINCT and LIMIT.
    const QUERIES: &[&str] = &[
        "SELECT *",
        "SELECT count(*)",
        "SELECT count(*), sum(number) WHERE account ~ 'Expenses:Food'",
        "SELECT account, number, cost_number, cost_currency, cost_date, cost_label, price, weight WHERE cost_number IS NOT NULL",
        "SELECT account, sum(position) GROUP BY account ORDER BY account",
        "SELECT account, units(sum(position)), cost(sum(position)), convert(units(sum(position)), 'USD') WHERE account ~ '^Assets' GROUP BY account ORDER BY account",
        "SELECT account, value(position), value(position, date), convert(position, 'USD', date), getprice(currency, 'USD', date) WHERE account ~ 'Assets'",
        "SELECT sum(weight), sum(cost(position)), count(*) WHERE number < 0",
        "SELECT payee, entry_meta('category'), any_meta('category'), meta('category') WHERE any_meta('category') IS NOT NULL",
        "SELECT payee, any_meta('title') WHERE entry_meta('title') IS NOT NULL",
        "SELECT date, account, position FROM year = 2016 WHERE account ~ 'Assets' AND number > 0",
        "SELECT date, account, position, price FROM month = 3 WHERE price IS NOT NULL OR cost_label = 'first'",
        "SELECT date, payee, account, position WHERE 'trip-chicago-2016' IN tags",
        "SELECT account, tags, links WHERE 'food' IN tags OR 'receipt-1' IN links OR 'travel' NOT IN tags",
        "SELECT account, other_accounts WHERE 'Assets:Bank' IN other_accounts",
        "SELECT id, flag, description, payee, narration WHERE payee ~ narration OR narration = 'buy' OR flag = 'P'",
        "SELECT account WHERE account = 'Assets:Lifo' OR currency = 'EUR' OR cost_currency = 'USD' OR cost_label = ''",
        "SELECT account, position WHERE account !~ 'Assets|Expenses' AND currency > 'A' AND payee <= 'Z'",
        "SELECT year, month, root(account, 2), sum(position) WHERE account ~ '^Expenses' GROUP BY 1, 2, 3 ORDER BY 1, 2, 3",
        "SELECT payee, sum(cost(position)) AS total WHERE account ~ '^Expenses' GROUP BY payee ORDER BY total DESC LIMIT 20",
        "SELECT DISTINCT account, cost_currency ORDER BY account",
        "SELECT account, position WHERE account ~ 'Expenses' LIMIT 7",
        "SELECT root(account, 1) AS r, first(date), last(position), min(number), max(weight) GROUP BY r ORDER BY count(*) DESC, r",
    ];

    /// Run `sql` over a dataset built for `projection` (or the query's own projection).
    fn run(ledger: &Ledger, sql: &str, projection: Option<Projection>) -> String {
        let query = Query::compile(sql).unwrap_or_else(|err| panic!("{sql}: {err}"));
        let store = ledger.store.read().unwrap();
        let today = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let data = Dataset::new(ledger, &store, today, projection.unwrap_or(query.projection));
        let rows = execute(&query.plan, &data, &Params::new(), None).unwrap_or_else(|err| panic!("{sql}: {}", err.message));
        // the Debug form keeps decimal scales, so equal strings are identical results
        format!("{rows:?}")
    }

    fn assert_pruning_keeps_results(ledger: &Ledger, queries: impl IntoIterator<Item = String>) {
        for sql in queries {
            let pruned = run(ledger, &sql, None);
            let full = run(ledger, &sql, Some(Projection::all()));
            assert_eq!(pruned, full, "{sql}");
        }
    }

    fn every_column() -> impl Iterator<Item = String> {
        COLUMNS.iter().map(|column| format!("SELECT {}", column.name))
    }

    #[test]
    fn pruning_keeps_results_on_the_fava_demo_ledger() {
        let ledger = fava_demo_ledger();
        assert_pruning_keeps_results(&ledger, QUERIES.iter().map(|it| it.to_string()).chain(every_column()));
    }

    #[test]
    fn pruning_keeps_results_with_lots_prices_and_metadata() {
        let ledger = load_text(LEDGER);
        // the ledger exercises split reductions: more rows than postings
        assert!(
            run(&ledger, "SELECT count(*)", None).contains("Int(19)"),
            "{}",
            run(&ledger, "SELECT count(*)", None)
        );
        assert_pruning_keeps_results(&ledger, QUERIES.iter().map(|it| it.to_string()).chain(every_column()));
    }

    /// Run `sql` over the rows of its table built for `projection` (or the query's own).
    fn run_table(ledger: &Ledger, sql: &str, projection: Option<Projection>) -> String {
        let query = Query::compile(sql).unwrap_or_else(|err| panic!("{sql}: {err}"));
        let store = ledger.store.read().unwrap();
        let today = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let mut budget = Budget::new(None);
        let data = Dataset::build(
            ledger,
            &store,
            today,
            projection.unwrap_or(query.projection),
            &Scope::All,
            None,
            &mut Limits::new(None, &mut budget),
        )
        .unwrap();
        let rows = execute(&query.plan, &data, &Params::new(), None).unwrap_or_else(|err| panic!("{sql}: {}", err.message));
        format!("{rows:?}")
    }

    #[test]
    fn record_tables_prune_without_changing_results() {
        let ledger = load_text(&format!(
            "{LEDGER}\n2024-04-03 balance Assets:Bank 1999.00 USD\n2024-04-04 note Assets:Bank \"x\"\n"
        ));
        let mut checked = 0;
        for table in crate::table::tables().iter().filter(|table| !table.is_postings()) {
            let all = Projection::all_of(table);
            let queries = table
                .columns
                .iter()
                .map(|column| format!("SELECT {} FROM #{}", column.name, table.name))
                .chain([format!("SELECT * FROM #{}", table.name), format!("SELECT count(*) FROM #{}", table.name)]);
            for sql in queries {
                assert_eq!(run_table(&ledger, &sql, None), run_table(&ledger, &sql, Some(all)), "{sql}");
                checked += 1;
            }
        }
        assert!(checked > 60, "{checked}");
    }

    #[test]
    fn balances_compute_the_true_balances_only_when_projected() {
        let ledger = load_text(&format!("{LEDGER}\n2024-04-03 balance Assets:Bank 1999.00 USD\n"));
        let store = ledger.store.read().unwrap();
        let today = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let actuals = |sql: &str| -> (usize, usize) {
            let mut budget = Budget::new(None);
            let data = Dataset::build(
                &ledger,
                &store,
                today,
                Query::compile(sql).unwrap().projection,
                &Scope::All,
                None,
                &mut Limits::new(None, &mut budget),
            )
            .unwrap();
            let computed = data
                .records
                .iter()
                .filter(|record| matches!(record, Record::Balance { check: Some(_), .. }))
                .count();
            (computed, data.records.len())
        };
        assert_eq!(actuals("SELECT date, amount FROM #balances").0, 0);
        for sql in [
            "SELECT date FROM #balances WHERE discrepancy IS NOT NULL",
            "SELECT actual FROM #balances",
            "SELECT count(*) FROM #balances WHERE passed",
        ] {
            let (computed, assertions) = actuals(sql);
            assert!(assertions >= 2, "{sql}");
            assert_eq!(computed, assertions, "{sql}");
        }
    }

    #[test]
    fn rows_drop_costs_and_prices_outside_the_projection() {
        let ledger = load_text(LEDGER);
        let store = ledger.store.read().unwrap();
        let today = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let rows_of = |sql: &str| {
            let projection = Query::compile(sql).unwrap().projection;
            let data = Dataset::new(&ledger, &store, today, projection);
            let costs = data.rows.iter().filter(|row| row.cost.is_some()).count();
            let prices = data.rows.iter().filter(|row| row.price.is_some()).count();
            (data.rows.len(), costs, prices)
        };
        // booking still splits the reductions, but the rows carry no cost or price
        assert_eq!(rows_of("SELECT account, number"), (19, 0, 0));
        assert_eq!(rows_of("SELECT cost_date"), (19, 9, 0));
        assert_eq!(rows_of("SELECT price"), (19, 0, 5));
        assert_eq!(rows_of("SELECT weight"), (19, 9, 5));
    }

    #[test]
    fn projection_follows_the_columns_the_plan_reads() {
        let projection = Query::compile("SELECT payee, sum(position) WHERE 'x' IN tags GROUP BY payee ORDER BY max(price)")
            .unwrap()
            .projection;
        assert_eq!(projection.names(), vec!["payee", "position", "price", "tags"]);
        assert!(projection.keeps_cost() && projection.keeps_price());
        assert!(projection.contains(column("tags").unwrap()) && !projection.contains(column("account").unwrap()));
        assert_eq!(projection.to_string(), "[payee, position, price, tags] (4 of 35 columns)");

        let projection = Query::compile("SELECT count(*), sum(number) WHERE account ~ 'Food'").unwrap().projection;
        assert!(!projection.keeps_cost() && !projection.keeps_price());
        assert_eq!(Query::compile("SELECT count(*)").unwrap().projection.to_string(), "[] (0 of 35 columns)");
        assert_eq!(Projection::all().names().len(), COLUMNS.len());
    }

    /// The in-place readers of a column give the same answer as its value.
    #[test]
    fn borrowed_columns_agree_with_their_values() {
        let ledger = load_text(LEDGER);
        let store = ledger.store.read().unwrap();
        let data = Dataset::new(&ledger, &store, NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(), Projection::all());
        let params = Params::new();
        let regexes = RegexCache::default();
        let impure = std::cell::Cell::new(false);
        let needles = ["food", "travel", "receipt-1", "Assets:Bank", "Income:Gains", "Expenses:Food", "x"];
        let mut checked = 0;
        for row in &data.rows {
            let env = Env {
                data: Some(&data),
                row: Some(RowRef::Posting(row)),
                aggregates: &[],
                cells: &[],
                running: None,
                params: &params,
                regexes: &regexes,
                impure: &impure,
            };
            for column in COLUMNS {
                let expr = CExpr::Column(column);
                let value = column.value(&data, RowRef::Posting(row));
                if let Some(text) = borrowed_str(&expr, &env) {
                    assert_eq!(text.map(|it| Value::Str(it.to_owned())).unwrap_or(Value::Null), value, "{}", column.name);
                    checked += 1;
                }
                if let Some(contains) = set_membership(&expr, &env) {
                    let Value::Set(set) = &value else { panic!("{} is not a set", column.name) };
                    for needle in needles {
                        assert_eq!(contains(needle), set.contains(needle), "{} {}", column.name, needle);
                    }
                    checked += 1;
                };
            }
        }
        assert_eq!(checked, data.rows.len() * 10);
    }
}
