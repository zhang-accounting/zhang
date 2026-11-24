//! The compatibility mapping of the budget system in the query engine (RFC #698 §3.4, §3.7, §9.3): the legacy names of
//! the official default plugin `zhang:budget`, its tables, label column and named queries, which zhang declares for
//! every catalog ([`budget_manifest`]), and the wording the budget system has always had, kept verbatim for
//! `zhang:budget` alone ([`too_long`]). Any other provider gets the generic wording. Nothing here computes a budget:
//! the plugin does, and the predicate and the queries below are the text it declares, which a ledger it does not run
//! for lists without running them.

use std::fmt::Write as _;

use chrono::NaiveDate;
use zhang_ast::{Directive, Spanned};
use zhang_core::ledger::Ledger;

use super::series::{EndBy, Extent, SeriesSpec};
use super::snapshot::Cause;
use super::{ColumnDecl, DatePart, Fill, LabelDecl, Manifest, NamedQuery, PeriodUnit, RecordOrder, SeriesDecl, TableDecl, TableKind};
use crate::catalog::TableRef;
use crate::table::{date_of, entry_type, ledger_file, LedgerCache};
use crate::value::DataType;

/// The provider the legacy budget names and wording are granted to.
pub(crate) const BUDGET: &str = "zhang:budget";

/// The message of a series too long for the result: `total` periods in all, `longest` the partition with the most.
pub(crate) fn too_long(ledger: &Ledger, table: &TableRef, provider: &str, spec: &SeriesSpec, total: u64, longest: &Extent<'_>) -> String {
    if provider == BUDGET && table.name() == "budgets" {
        return budget_months(ledger, spec, total, longest);
    }
    let month = |date: NaiveDate| date.format("%Y-%m").to_string();
    // the partition by its key, as its first fact has it
    let mut key = String::new();
    if let Some(fact) = longest.partition.facts.first() {
        for (n, column) in spec.partition.iter().enumerate() {
            let name = table.columns().get(*column).map_or("", |it| it.name.as_ref());
            let cell = fact.cells.get(*column).map(ToString::to_string).unwrap_or_default();
            let _ = write!(key, "{} {} = {}", if n == 0 { "" } else { "," }, name, cell);
        }
    }
    let cause = match longest.end_by {
        EndBy::Query => " (the last month the query keeps)".to_owned(),
        EndBy::CurrentPeriod => match entry(ledger, longest.partition.first_cause) {
            Some(directive) => format!(
                ", the current month; check the date of the {} it starts with{}",
                entry_type(&directive.data),
                file(ledger, directive)
            ),
            None => ", the current month".to_owned(),
        },
        EndBy::Entry(cause) => entry(ledger, cause)
            .and_then(|directive| {
                let date = date_of(&directive.data)?;
                Some(format!(
                    " because of the {} dated {}{}; check that date",
                    entry_type(&directive.data),
                    date,
                    file(ledger, directive)
                ))
            })
            .unwrap_or_default(),
    };
    format!(
        "the #{} table would generate too many rows ({} months in all, more than the result size limit): the series of{} runs from {} until {}{}",
        table.name(),
        total,
        key,
        month(longest.first),
        month(longest.end),
        cause
    )
}

/// The `#budgets` wording: the budget with the longest series (by its partition column, `name`), and the directive
/// whose date sets its end.
fn budget_months(ledger: &Ledger, spec: &SeriesSpec, total: u64, longest: &Extent<'_>) -> String {
    let month = |date: NaiveDate| date.format("%Y-%m").to_string();
    let name = longest
        .partition
        .facts
        .first()
        .zip(spec.partition.first())
        .and_then(|(fact, column)| fact.cells.get(*column))
        .and_then(|cell| cell.as_str())
        .unwrap_or_default();
    let cause = match longest.end_by {
        EndBy::Query => " (the last month the query keeps)".to_owned(),
        // the current month ends it: the series is long because it starts early
        EndBy::CurrentPeriod => {
            let definition = entry(ledger, longest.partition.first_cause);
            format!(
                ", the current month; check the date of its budget directive{}",
                definition.map(|it| file(ledger, it)).unwrap_or_default()
            )
        }
        EndBy::Entry(cause) => entry(ledger, cause)
            .and_then(|directive| {
                let kind = match &directive.data {
                    Directive::Transaction(_) => "a transaction",
                    Directive::Budget(_) => "its budget directive",
                    Directive::BudgetAdd(_) => "a budget-add",
                    Directive::BudgetTransfer(_) => "a budget-transfer",
                    Directive::BudgetClose(_) => "a budget-close",
                    _ => return None,
                };
                let date = date_of(&directive.data)?;
                Some(format!(" because of {} dated {}{}; check that date", kind, date, file(ledger, directive)))
            })
            .unwrap_or_default(),
    };
    format!(
        "the #budgets table would generate too many rows ({} months in all, more than the result size limit): \
         budget '{}' runs from {} until {}{}",
        total,
        name,
        month(longest.first),
        month(longest.end),
        cause
    )
}

/// The directive of the entry `cause`.
fn entry(ledger: &Ledger, cause: Option<Cause>) -> Option<&Spanned<Directive>> {
    let entries = LedgerCache::of(ledger).entries(ledger);
    let row = entries.rows.get(cause?.seq as usize)?;
    ledger.directives.get(row.directive as usize)
}

/// ` (file)`, the file of `directive` within the ledger, or nothing.
fn file(ledger: &Ledger, directive: &Spanned<Directive>) -> String {
    directive
        .span
        .filename
        .as_deref()
        .map(|path| format!(" ({})", ledger_file(ledger, path).display()))
        .unwrap_or_default()
}

/// The predicate of the `budgets` column of `postings`, the one rule of membership in BQL, as `zhang:budget` declares
/// it: the row's account is one of the budget's (a fast test the index of the label column uses), its transaction was
/// folded after the budget's definition, the `budget` metadata of its account's `open` in effect at its date and time
/// names the budget (`open_meta_has`, which reads the `open` and the row's time in place), the budget is not closed by
/// then, and a price converts it to the budget's commodity.
pub(crate) const PREDICATE: &str = "account IN :accounts AND seq > :defined_seq \
    AND open_meta_has(account, 'budget', :label, date, time) \
    AND (:close_date IS NULL OR (:close_time IS NULL AND date <= :close_date) \
         OR (:close_time IS NOT NULL AND (date < :close_date OR (date = :close_date AND time <= :close_time)))) \
    AND currency(convert(position, :commodity, date)) = :commodity";

fn column(name: &str, ty: DataType, description: &str, fill: Option<Fill>) -> ColumnDecl {
    ColumnDecl {
        name: name.to_owned(),
        ty,
        description: description.to_owned(),
        fill,
    }
}

fn names(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

/// `budgets.month` without its order, and with a filter `budgets.budget_month`.
macro_rules! budgets_as_of_month {
    ($($filter:literal)?) => { concat!("SELECT name, last(alias) AS alias, last(category) AS category, last(currency) AS currency, \
              last(date) AS last_month, \
              CASE WHEN last(date) < :month THEN last(available) ELSE last(assigned) END AS assigned, \
              CASE WHEN last(date) < :month THEN 0 ELSE number(last(activity)) END AS activity, \
              last(available) AS available, last(closed) AS closed \
              FROM #budgets \
              WHERE ", $($filter, " AND ",)? "date <= :month GROUP BY name") };
}

/// The named queries of `zhang:budget`, those the budget pages of the web UI and the app run, which zhang's built-in
/// queries have always had, with their text unchanged. A month is the first day of the month (`2024-06-01`).
fn queries() -> Vec<NamedQuery> {
    use DataType::{Date, Str};
    let query = |name: &str, description: &str, bql: &str, params: &[(&str, DataType)]| NamedQuery {
        name: name.to_owned(),
        description: description.to_owned(),
        bql: bql.to_owned(),
        params: params.iter().map(|(name, ty)| ((*name).to_owned(), *ty)).collect(),
    };
    vec![
        query(
            "budgets.month",
            "Every budget as of a month (its first day): its last month in #budgets up to that month, carried over to the month when it is later.",
            concat!(budgets_as_of_month!(), " ORDER BY name"),
            &[("month", Date)],
        ),
        query(
            "budgets.budget",
            "One budget: its display name, category, commodity, the accounts whose postings are its activity, and the date and time of its close.",
            "SELECT name, alias, category, currency, accounts, close, close_time \
              FROM #budget_definitions \
              WHERE name = :name",
            &[("name", Str)],
        ),
        query(
            "budgets.budget_month",
            "One budget as of a month (its first day), as in budgets.month.",
            budgets_as_of_month!("name = :name"),
            &[("name", Str), ("month", Date)],
        ),
        query(
            "budgets.events",
            "What the budget-add and budget-transfer directives put into a budget in a month (its first day), newest first.",
            "SELECT date, time, timestamp, type, amount \
              FROM #budget_events \
              WHERE name = :name AND type != 'close' AND yearmonth(date) = :month \
              ORDER BY timestamp DESC",
            &[("name", Str), ("month", Date)],
        ),
        query(
            "budgets.postings",
            "The postings that count toward a budget in a month (its first day), newest first, with each account's balance after them: they add up to the month's activity.",
            "SELECT date, time, timestamp, account, id, payee, narration, units(position) AS units, \
              only(currency, account_balance) AS balance \
              WHERE yearmonth(date) = :month AND :name IN budgets \
              ORDER BY timestamp DESC",
            &[("name", Str), ("month", Date)],
        ),
    ]
}

/// The legacy names of `zhang:budget`: its tables, its label column and its named queries, as the budget tables and the
/// budget pages' queries have always described them, and as the bundled script declares them (a test of the server
/// checks that it does, `the_budget_script_declares_the_legacy_manifest`). zhang declares them for every catalog
/// ([`super::DECLARED`]), so that a client of a ledger the plugin does not run for, with plugins off or in the
/// playground, finds the tables and queries it always had, which answer that they are unavailable and why; when the
/// plugin runs, its own declaration takes their place, and serves its snapshot.
pub(crate) fn budget_manifest() -> Manifest {
    use DataType::{Amount, Bool, Date, Int, Set, Str};
    let carry = || Some(Fill::Carry);
    let budgets = TableDecl {
        name: "budgets".to_owned(),
        published_as: "budgets".to_owned(),
        description: "One row per budget per month, from the budget's first month through its last entry, the ledger's last month with a transaction or the current month, whichever is latest, with the assigned, activity and available amounts the budget pages show, in the budget's commodity; ordered by name, then month.".to_owned(),
        kind: TableKind::Series(SeriesDecl {
            period: "date".to_owned(),
            unit: PeriodUnit::Month,
            derive: vec![("year".to_owned(), DatePart::Year), ("month".to_owned(), DatePart::Month)],
            partition: names(&["name"]),
            extend_to_today: true,
        }),
        columns: vec![
            column("name", Str, "Name of the budget, as written in its directives.", carry()),
            column("alias", Str, "Display name of the budget (its alias metadata), or NULL if it has none.", carry()),
            column("category", Str, "Category the budget is grouped under (its category metadata), or NULL if it has none.", carry()),
            column("currency", Str, "Commodity the budget is kept in.", carry()),
            column("date", Date, "First day of the month.", None),
            column("year", Int, "Year of the month.", None),
            column("month", Int, "Month of the year, from 1 to 12.", None),
            column(
                "assigned",
                Amount,
                "Amount assigned to the budget for the month: the available amount carried over from the previous month plus what was added in the month.",
                Some(Fill::Previous("available".to_owned())),
            ),
            column(
                "added",
                Amount,
                "Amount the month's budget-add and budget-transfer directives put into the budget, converted to its commodity at their date; a transfer out counts as negative.",
                Some(Fill::Zero),
            ),
            column(
                "activity",
                Amount,
                "Amount the budget's accounts spent in the month: their booked postings, each converted to the budget's commodity at its date; postings no price converts, and those after the budget's close, are left out.",
                Some(Fill::Zero),
            ),
            column(
                "available",
                Amount,
                "Amount left at the end of the month (assigned minus activity); it carries over to the next month.",
                carry(),
            ),
            column(
                "accounts",
                Set,
                "Accounts whose postings count as the budget's activity: those with a budget metadata entry naming it.",
                carry(),
            ),
            column("closed", Bool, "Whether the budget was closed (budget-close) in or before the month.", carry()),
        ],
        wildcard: names(&["name", "date", "assigned", "activity", "available"]),
    };
    let events = TableDecl {
        name: "budget_events".to_owned(),
        published_as: "budget_events".to_owned(),
        description: "One row per effect of a budget directive, in ledger order: the amount a budget-add assigns, the amount a \
                      budget-transfer moves out of one budget and into another, and a budget-close."
            .to_owned(),
        kind: TableKind::Records(RecordOrder::Published),
        columns: vec![
            column("name", Str, "Name of the budget.", None),
            column("date", Date, "Date of the budget directive.", None),
            column(
                "time",
                Str,
                "Time of day of the budget directive in the ledger's timezone, as HH:MM:SS; 00:00:00 without a time.",
                None,
            ),
            column(
                "timestamp",
                Int,
                "Unix time in seconds of the budget directive's date and time in the ledger's timezone.",
                None,
            ),
            column(
                "type",
                Str,
                "What the directive does to the budget: 'assign' (budget-add), 'transfer_out' and 'transfer_in' (budget-transfer) or 'close' (budget-close).",
                None,
            ),
            column(
                "amount",
                Amount,
                "Amount put into the budget, as written; negative for a transfer out, NULL for a close.",
                None,
            ),
        ],
        wildcard: names(&["name", "date", "time", "timestamp", "type", "amount"]),
    };
    let definitions = TableDecl {
        name: "budget_definitions".to_owned(),
        published_as: "budget_definitions".to_owned(),
        description: "One row per budget, as its directives define it: its commodity, display name, category, the accounts of its activity and the date and time it is closed; ordered by name. It has no months, so it reads no transaction.".to_owned(),
        kind: TableKind::Records(RecordOrder::Columns(names(&["name"]))),
        columns: vec![
            column("name", Str, "Name of the budget, as written in its directives.", None),
            column("date", Date, "Date of the budget directive, from which the budget exists.", None),
            column("currency", Str, "Commodity the budget is kept in.", None),
            column("alias", Str, "Display name of the budget (its alias metadata), or NULL if it has none.", None),
            column("category", Str, "Category the budget is grouped under (its category metadata), or NULL if it has none.", None),
            column(
                "accounts",
                Set,
                "Accounts whose postings count as the budget's activity: those with a budget metadata entry naming it.",
                None,
            ),
            column(
                "close",
                Date,
                "Date of the budget's first budget-close, which closes it, or NULL while it is open.",
                None,
            ),
            column(
                "close_time",
                Str,
                "Time of day of the budget's first budget-close, as HH:MM:SS, until which the budget takes activity on its close day; NULL for a budget-close without a time, which leaves the budget open through its whole close day, or while it is open.",
                None,
            ),
        ],
        wildcard: names(&["name", "date", "currency", "alias", "category", "accounts", "close", "close_time"]),
    };
    let postings_budgets = LabelDecl {
        table: "postings".to_owned(),
        name: "budgets".to_owned(),
        published_as: "budgets".to_owned(),
        description: "The budgets the posting counts toward, as the budget pages count it: those the budget metadata of its \
        account's open in effect at its date and time names, defined before it and not closed by then, into \
        whose commodity a price converts it. The postings of a budget in a month add up to its activity in \
        #budgets. A zhang extension."
            .to_owned(),
        predicate: PREDICATE.to_owned(),
        params: vec![
            ("accounts".to_owned(), Set),
            ("defined_seq".to_owned(), Int),
            ("close_date".to_owned(), Date),
            ("close_time".to_owned(), Str),
            ("commodity".to_owned(), Str),
        ],
    };
    Manifest {
        provider: BUDGET.to_owned(),
        tables: vec![budgets, events, definitions],
        labels: vec![postings_budgets],
        queries: queries(),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_budgets_column_is_indexed_by_the_budgets_accounts() {
        let provider = &crate::extension::declared_providers()[0];
        let column = crate::extension::LabelColumn::compile(
            &std::sync::Arc::from(provider.manifest.provider.as_str()),
            provider.digest(),
            provider.source,
            &provider.manifest.labels[0],
        )
        .unwrap();
        let (indexed, param) = column.index.as_ref().expect("an index");
        assert_eq!((indexed.name(), param.to_string()), ("account", ":accounts".to_owned()));
    }
}

/// The legacy manifest as an analyzer declares it, which the example analyzer of `zhang-plugin-sdk` standing in for
/// `zhang:budget` reads (`zhang-plugin-sdk/examples/budget/manifest.json`), and the bundled script declares too: the
/// same tables, columns, descriptions, predicate and named queries, so that the catalog it gives a ledger is the one
/// zhang declares, served from its snapshot.
#[cfg(test)]
mod as_an_analyzer {
    use serde_json::{json, Value};

    use super::*;

    fn fill(fill: &Option<Fill>) -> Value {
        match fill {
            None => Value::Null,
            Some(Fill::Carry) => json!("carry"),
            Some(Fill::Zero) => json!("zero"),
            Some(Fill::Previous(column)) => json!({ "previous": column }),
        }
    }

    fn table(decl: &TableDecl) -> Value {
        let columns = decl
            .columns
            .iter()
            .map(|column| {
                let mut value = json!({"name": column.name, "type": column.ty.name(), "description": column.description});
                if column.fill.is_some() {
                    value["fill"] = fill(&column.fill);
                }
                value
            })
            .collect::<Vec<_>>();
        let mut value = json!({
            "name": decl.published_as, "kind": "records", "description": decl.description, "columns": columns, "wildcard": decl.wildcard,
        });
        match &decl.kind {
            TableKind::Records(RecordOrder::Published) => value["order"] = json!("published"),
            TableKind::Records(RecordOrder::Columns(columns)) => value["order"] = json!(columns),
            TableKind::Series(series) => {
                value["kind"] = json!("series");
                let derive = series
                    .derive
                    .iter()
                    .map(|(column, part)| {
                        let part = match part {
                            DatePart::Year => "year",
                            DatePart::Month => "month",
                        };
                        (column.clone(), json!(part))
                    })
                    .collect::<serde_json::Map<_, _>>();
                value["period"] = json!({"column": series.period, "unit": "month", "derive": derive});
                value["partition"] = json!(series.partition);
                value["extend_to_today"] = json!(series.extend_to_today);
            }
        }
        value
    }

    fn query(query: &NamedQuery) -> Value {
        let params = query
            .params
            .iter()
            .map(|(name, ty)| json!({"name": name, "type": ty.name()}))
            .collect::<Vec<_>>();
        json!({"name": query.name, "description": query.description, "bql": query.bql, "params": params})
    }

    #[test]
    fn the_budget_analyzer_example_declares_the_legacy_tables() {
        let manifest = budget_manifest();
        let labels = manifest
            .labels
            .iter()
            .map(|label| {
                let params = label
                    .params
                    .iter()
                    .map(|(name, ty)| (name.clone(), json!(ty.name())))
                    .collect::<serde_json::Map<_, _>>();
                json!({"table": label.table, "name": label.published_as, "description": label.description, "predicate": label.predicate, "params": params})
            })
            .collect::<Vec<_>>();
        let declared = json!({
            "analysis_abi": 1,
            "consumes": ["budget", "budget-add", "budget-transfer", "budget-close"],
            "open_meta": ["budget"],
            "diagnostics": [
                {"kind": "BudgetDoesNotExist"}, {"kind": "DefineDuplicatedBudget"}, {"kind": "BudgetClosed"},
                {"kind": "BudgetCommodityMismatch", "phase": "after_fold"}
            ],
            "tables": manifest.tables.iter().map(table).collect::<Vec<_>>(),
            "columns": labels,
            "queries": manifest.queries.iter().map(query).collect::<Vec<_>>(),
        });
        let text = format!("{}\n", serde_json::to_string_pretty(&declared).unwrap());
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../zhang-plugin-sdk/examples/budget/manifest.json");
        if matches!(std::env::var("UPDATE_GOLDEN").as_deref(), Ok("1" | "true")) {
            std::fs::write(path, &text).unwrap();
        }
        assert_eq!(std::fs::read_to_string(path).unwrap_or_default(), text, "UPDATE_GOLDEN=1 rewrites {path}");
        // and it reads back as the legacy manifest, but for the order of the parts of a period and of the parameters
        // of a label, which a JSON object does not keep
        let read: zhang_shared::plugin_abi::analysis::Manifest = serde_json::from_str(&text).unwrap();
        let converted = super::super::abi::manifest(BUDGET, &read, true).unwrap();
        let mut legacy = manifest.clone();
        for table in &mut legacy.tables {
            if let TableKind::Series(series) = &mut table.kind {
                series.derive.sort_by(|a, b| a.0.cmp(&b.0));
            }
        }
        for label in &mut legacy.labels {
            label.params.sort_by(|a, b| a.0.cmp(&b.0));
        }
        assert_eq!(format!("{:?}", converted), format!("{:?}", legacy));
    }
}
