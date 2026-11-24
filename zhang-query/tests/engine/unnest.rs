//! `unnest(set)`, a group key per element of a set (a zhang extension): a row counts in the group of each element, in
//! none without any, and the groups hold what a query per element filtering on membership holds.

use zhang_query::{DataType, ParamTypes, Params, QueryErrorKind, QueryResult, Value};
use zhang_testkit::budget_ledger::Spec;

const TAGGED: &str = r#"
option "operating_currency" "USD"
1970-01-01 open Assets:Cash
1970-01-01 open Expenses:Food
1970-01-01 open Expenses:Travel

2024-01-02 * "Lunch" #food #work
  Expenses:Food  12.50 USD
  Assets:Cash

2024-01-03 * "Train" #travel #work
  Expenses:Travel  30.00 USD
  Assets:Cash

2024-02-04 * "Dinner" #food
  Expenses:Food  20 USD
  Assets:Cash

2024-02-05 * "Cash"
  Expenses:Food  1.00 USD
  Assets:Cash
"#;

fn run(ledger: &zhang_core::ledger::Ledger, query: &str) -> QueryResult {
    crate::compile(query)
        .and_then(|compiled| compiled.execute(ledger, &Params::default()))
        .unwrap_or_else(|err| panic!("{}: {}", query, err))
}

fn compile_error(query: &str) -> zhang_query::QueryError {
    crate::compile(query).err().unwrap_or_else(|| panic!("{} compiles", query))
}

fn text(rows: &[Vec<Value>]) -> Vec<String> {
    rows.iter()
        .map(|row| row.iter().map(ToString::to_string).collect::<Vec<_>>().join(" | "))
        .collect()
}

#[test]
fn a_row_counts_in_the_group_of_each_element_of_the_set() {
    let ledger = zhang_testkit::ledger::load_text(TAGGED);
    let result = run(
        &ledger,
        "SELECT unnest(tags) AS tag, count(*) AS n, sum(number) AS total WHERE account ~ '^Expenses' GROUP BY tag ORDER BY tag",
    );
    assert_eq!(result.columns[0].name, "tag");
    assert_eq!(result.columns[0].ty, DataType::Str);
    // the posting without tags counts in no group
    assert_eq!(text(&result.rows), vec!["food | 2 | 32.50", "travel | 1 | 30.00", "work | 2 | 42.50"]);

    // implicitly grouped, by position, with another key, and a key that is not selected
    let implicit = run(&ledger, "SELECT unnest(tags) AS tag, count(*) WHERE account ~ '^Expenses' ORDER BY tag");
    assert_eq!(text(&implicit.rows), vec!["food | 2", "travel | 1", "work | 2"]);
    let by_month = run(
        &ledger,
        "SELECT yearmonth(date) AS month, unnest(tags), count(*) WHERE account ~ '^Expenses' GROUP BY 1, 2 ORDER BY 1, 2",
    );
    assert_eq!(
        text(&by_month.rows),
        vec![
            "2024-01-01 | food | 1",
            "2024-01-01 | travel | 1",
            "2024-01-01 | work | 2",
            "2024-02-01 | food | 1"
        ]
    );
    assert_eq!(by_month.columns[1].name, "unnest(tags)");
    let hidden = run(&ledger, "SELECT count(*) WHERE account ~ '^Expenses' GROUP BY unnest(tags) ORDER BY count(*)");
    assert_eq!(text(&hidden.rows), vec!["1", "2", "2"]);
}

#[test]
fn an_unnested_key_works_with_having_order_limit_and_pivot() {
    let ledger = zhang_testkit::ledger::load_text(TAGGED);
    let having = run(
        &ledger,
        "SELECT unnest(tags) AS tag, count(*) WHERE account ~ '^Expenses' GROUP BY tag HAVING count(*) > 1 AND unnest(tags) != 'food'",
    );
    assert_eq!(text(&having.rows), vec!["work | 2"]);
    // LIMIT without ORDER BY keeps the first groups, in the order of their first rows and the set's order within a row
    let all = run(&ledger, "SELECT unnest(tags) AS tag, count(*) WHERE account ~ '^Expenses' GROUP BY tag");
    assert_eq!(text(&all.rows), vec!["food | 2", "work | 2", "travel | 1"]);
    let first = run(&ledger, "SELECT unnest(tags) AS tag, count(*) WHERE account ~ '^Expenses' GROUP BY tag LIMIT 2");
    assert_eq!(first.rows, &all.rows[..2]);
    // a deferred last() of the running balance, read at the last row of each group
    let descending = run(
        &ledger,
        "SELECT unnest(tags) AS tag, last(balance) WHERE account = 'Expenses:Food' GROUP BY tag ORDER BY tag DESC",
    );
    assert_eq!(text(&descending.rows), vec!["work | 12.50 USD", "food | 32.50 USD"]);
    let pivot = run(
        &ledger,
        "SELECT unnest(tags) AS tag, month, count(*) WHERE account ~ '^Expenses' GROUP BY tag, month PIVOT BY tag, month",
    );
    assert_eq!((pivot.rows.len(), pivot.columns.len()), (3, 3));
    // the plan shows the key
    let explain = crate::compile("SELECT unnest(tags) AS tag, count(*) GROUP BY tag").unwrap().explain();
    assert!(explain.contains("target 0: tag = unnest(tags) : str\n"), "{}", explain);
}

#[test]
fn a_null_or_empty_set_puts_a_row_in_no_group() {
    let ledger = zhang_testkit::ledger::load_text(TAGGED);
    assert!(run(&ledger, "SELECT unnest(NULL) AS x, count(*) GROUP BY x").rows.is_empty());
    assert!(run(&ledger, "SELECT unnest(set()) AS x, count(*) GROUP BY x").rows.is_empty());
    let constant = run(&ledger, "SELECT unnest(set('a', 'b')) AS x, count(*) GROUP BY x ORDER BY x");
    assert_eq!(text(&constant.rows), vec!["a | 8", "b | 8"]);
    // a parameter, bound per execution
    let query = crate::compile_with_params(
        "SELECT unnest(:labels) AS label, count(*) WHERE account = 'Assets:Cash' GROUP BY label ORDER BY label",
        &ParamTypes::new().bind("labels", DataType::Set),
    )
    .unwrap();
    let labels = Value::Set(["x".to_owned(), "y".to_owned()].into());
    let result = query.execute(&ledger, &Params::new().bind("labels", labels)).unwrap();
    assert_eq!(text(&result.rows), vec!["x | 4", "y | 4"]);
    let result = query.execute(&ledger, &Params::new().bind("labels", Value::Null)).unwrap();
    assert!(result.rows.is_empty());
}

#[test]
fn unnest_is_only_a_whole_group_key() {
    for (query, message) in [
        (
            "SELECT unnest(tags)",
            "only allowed as a whole target or GROUP BY item that the query groups by",
        ),
        ("SELECT unnest(tags) AS tag, count(*) GROUP BY account", "only allowed as a whole target"),
        ("SELECT account, unnest(tags) AS tag", "only allowed as a whole target"),
        ("SELECT upper(unnest(tags)) AS tag, count(*) GROUP BY tag", "only allowed as a whole target"),
        ("SELECT count(*) WHERE unnest(tags) = 'food'", "only allowed as a whole target"),
        (
            "SELECT account, count(*) GROUP BY account ORDER BY unnest(tags)",
            "only allowed as a whole target",
        ),
        ("SELECT account, sum(length(unnest(tags))) GROUP BY account", "only allowed as a whole target"),
        (
            "SELECT unnest(tags) AS a, unnest(links) AS b, count(*) GROUP BY a, b",
            "a query can unnest one set only",
        ),
        ("SELECT unnest(account) AS a, count(*) GROUP BY a", "unnest"),
        (
            "SELECT unnest(sum(number)) AS a GROUP BY a",
            "aggregate function sum() is not allowed in unnest()",
        ),
        ("SELECT unnest(*) AS a, count(*) GROUP BY a", "unnest(*) is not supported"),
    ] {
        let err = compile_error(query);
        assert_eq!(err.kind, QueryErrorKind::Compile, "{query}: {}", err.message);
        assert!(err.message.contains(message), "{query}: {}", err.message);
    }
}

/// The budget analysis's use: the activity of every budget per month in one pass, grouped by the budgets of each
/// posting's account at its date and time, is what one query per budget testing its membership sums.
#[test]
fn grouping_by_the_budgets_of_postings_sums_what_a_query_per_budget_sums() {
    let spec = Spec::new(600, 12).with_months(18);
    let scratch = spec.generate().scratch();
    let ledger = scratch.ledger_at(spec.clock()).expect("the ledger loads");
    let activity = "sum(CASE WHEN currency = 'USD' THEN number ELSE number(convert(position, 'USD', date)) END)";
    let grouped = run(
        &ledger,
        &format!(
            "SELECT unnest(open_meta_values(account, 'budget', date, time)) AS budget, yearmonth(date) AS month, \
             count(*) AS n, {activity} AS activity GROUP BY budget, month ORDER BY budget, month"
        ),
    );
    let budgets = grouped.rows.iter().map(|row| row[0].to_string()).collect::<std::collections::BTreeSet<_>>();
    assert!(budgets.len() >= 10, "{budgets:?}");
    let per_budget = crate::compile_with_params(
        &format!(
            "SELECT :budget AS budget, yearmonth(date) AS month, count(*) AS n, {activity} AS activity \
             WHERE open_meta_has(account, 'budget', :budget, date, time) GROUP BY budget, month ORDER BY month"
        ),
        &ParamTypes::new().bind("budget", DataType::Str),
    )
    .unwrap();
    let mut expected = vec![];
    for budget in &budgets {
        let result = per_budget.execute(&ledger, &Params::new().bind("budget", budget.as_str())).unwrap();
        expected.extend(result.rows);
    }
    // every value, the scale of each sum included
    assert_eq!(text(&grouped.rows), text(&expected));
    assert_eq!(grouped.rows, expected);

    // without a time and with a time written in the query, the set read in place groups what a membership test of the
    // set as written selects
    for set in [
        "open_meta_values(account, 'budget', date)",
        "open_meta_values(account, 'budget', date, '12:00')",
    ] {
        let grouped = run(
            &ledger,
            &format!(
                "SELECT unnest({set}) AS budget, yearmonth(date) AS month, count(*), {activity} \
                 GROUP BY budget, month ORDER BY budget, month"
            ),
        );
        assert!(!grouped.rows.is_empty(), "{set}");
        let member = crate::compile_with_params(
            &format!(
                "SELECT :budget AS budget, yearmonth(date) AS month, count(*), {activity} WHERE :budget IN {set} \
                 GROUP BY budget, month ORDER BY month"
            ),
            &ParamTypes::new().bind("budget", DataType::Str),
        )
        .unwrap();
        let names = grouped.rows.iter().map(|row| row[0].to_string()).collect::<std::collections::BTreeSet<_>>();
        let mut expected = vec![];
        for budget in &names {
            expected.extend(member.execute(&ledger, &Params::new().bind("budget", budget.as_str())).unwrap().rows);
        }
        assert_eq!(text(&grouped.rows), text(&expected), "{set}");
    }
    // and fail alike on a time that is none
    let failure = |query: &str| {
        crate::compile(query)
            .unwrap()
            .execute(&ledger, &Params::default())
            .expect_err("the query fails")
            .message
    };
    assert_eq!(
        failure("SELECT unnest(open_meta_values(account, 'budget', date, narration)) AS b, count(*) GROUP BY b"),
        failure("SELECT open_meta_values(account, 'budget', date, narration)")
    );
}

/// A ledger whose transactions have zero to three tags and zero to two links each, and amounts of several scales.
fn tags_and_links_ledger() -> String {
    let mut random = zhang_testkit::XorShift(873);
    let mut text = String::from("1970-01-01 open Assets:Cash\n1970-01-01 open Expenses:Food\n1970-01-01 open Expenses:Travel\n\n");
    for day in 0..240 {
        let tags = ["food", "work", "trip", "kids", "gift"]
            .into_iter()
            .filter(|_| random.chance(30))
            .map(|tag| format!(" #{tag}"))
            .collect::<String>();
        let links = ["a1", "b2", "c3"]
            .into_iter()
            .filter(|_| random.chance(25))
            .map(|link| format!(" ^{link}"))
            .collect::<String>();
        let account = if random.chance(50) { "Expenses:Food" } else { "Expenses:Travel" };
        let amount = format!("{}.{:0width$}", random.below(500), random.below(100), width = 1 + random.below(2));
        text.push_str(&format!(
            "2024-{:02}-{:02} * \"Shop\"{tags}{links}\n  {account}  {amount} USD\n  Assets:Cash\n\n",
            1 + day / 28,
            1 + day % 28
        ));
    }
    text
}

/// The groups of `unnest(set)` against one query per element filtering on membership, which reads the set as a
/// membership test and never unnests it: same elements, same postings, same sums (scales included). The elements are
/// those the rows' sets hold, read from the rows themselves.
fn assert_unnest_is_one_query_per_element(ledger: &zhang_core::ledger::Ledger, set: &str) -> usize {
    let elements = run(ledger, &format!("SELECT {set}"))
        .rows
        .into_iter()
        .flat_map(|row| match row.into_iter().next() {
            Some(Value::Set(elements)) => elements,
            _ => Default::default(),
        })
        .collect::<std::collections::BTreeSet<_>>();
    let aggregates = "count(*), sum(number), sum(position), min(date), max(account)";
    let grouped = run(
        ledger,
        &format!("SELECT unnest({set}) AS element, {aggregates} GROUP BY element ORDER BY element"),
    );
    let per_element = crate::compile_with_params(
        &format!("SELECT :element AS element, {aggregates} WHERE :element IN {set}"),
        &ParamTypes::new().bind("element", DataType::Str),
    )
    .unwrap();
    let mut expected = vec![];
    for element in &elements {
        expected.extend(per_element.execute(ledger, &Params::new().bind("element", element.as_str())).unwrap().rows);
    }
    assert_eq!(text(&grouped.rows), text(&expected), "unnest({set})");
    assert_eq!(grouped.rows, expected, "unnest({set})");
    elements.len()
}

#[test]
fn unnest_groups_what_one_query_per_element_selects() {
    let fava = zhang_testkit::ledger::fava_demo_ledger();
    assert!(assert_unnest_is_one_query_per_element(&fava, "tags") >= 4);
    // the fava demo ledger has no links
    assert_eq!(assert_unnest_is_one_query_per_element(&fava, "links"), 0);
    let generated = zhang_testkit::ledger::load_text(&tags_and_links_ledger());
    assert_eq!(assert_unnest_is_one_query_per_element(&generated, "tags"), 5);
    assert_eq!(assert_unnest_is_one_query_per_element(&generated, "links"), 3);
}
