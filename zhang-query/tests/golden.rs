//! Golden tests: the representative queries of issue #434 over
//! `integration-tests/fava-demo-ledger`, compared with results produced by beanquery
//! (`tests/golden/fava_demo.json`, regenerated with `tests/conformance/generate.py --set golden`)
//! through `zhang_testkit::oracle`.
//!
//! Numbers are compared numerically (`4.0 = 4.00`), inventory positions as sets, and the
//! column names and types exactly.

use std::path::PathBuf;
use std::str::FromStr;

use bigdecimal::BigDecimal;
use chrono::NaiveDate;
use zhang_query::{DataType, ParamTypes, Params, Query, Value};
use zhang_testkit::oracle::{load_query_list, run_case, Rules};

fn golden_case(index: usize) {
    let mut cases = load_query_list(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/fava_demo.json"));
    let case = &mut cases[index];
    // beanquery's column names are part of these results
    case.strict_names = true;
    let ledger = zhang_testkit::fixtures::fava_demo();
    run_case(&ledger, case, &Rules::on(NaiveDate::from_ymd_opt(2025, 1, 1).unwrap()), &[], &[]).assert_ok();
}

#[test]
fn monthly_expenses_by_category() {
    golden_case(0);
}

#[test]
fn spending_by_payee_top_20() {
    golden_case(1);
}

#[test]
fn postings_with_tag() {
    golden_case(2);
}

#[test]
fn holdings_at_cost_and_market_value() {
    golden_case(3);
}

#[test]
fn count_and_sum_of_food_expenses() {
    golden_case(4);
}

/// Drive the typed Rust API (no JSON): a compiled report query executed with bound
/// parameters, as an internal caller in zhang-server would.
#[test]
fn typed_api_with_parameters() {
    let ledger = zhang_testkit::ledger::fava_demo_ledger();
    let query = Query::compile_with_params(
        "SELECT year, month, root(account, 2) AS category, sum(position) AS total \
         WHERE account ~ :pattern AND date >= :from AND date < :to \
         GROUP BY year, month, category ORDER BY year, month, category",
        &ParamTypes::new()
            .bind("pattern", DataType::Str)
            .bind("from", DataType::Date)
            .bind("to", DataType::Date),
    )
    .unwrap();
    assert_eq!(
        query.columns().iter().map(|it| (it.name.as_str(), it.ty)).collect::<Vec<_>>(),
        vec![
            ("year", DataType::Int),
            ("month", DataType::Int),
            ("category", DataType::Str),
            ("total", DataType::Inventory)
        ]
    );

    let month = |year: i32, month: u32| {
        Params::new()
            .bind("pattern", "^Expenses")
            .bind("from", NaiveDate::from_ymd_opt(year, month, 1).unwrap())
            .bind("to", NaiveDate::from_ymd_opt(year, month + 1, 1).unwrap())
    };

    // the same compiled query runs for several parameter sets
    let january = query.execute(&ledger, &month(2016, 1)).unwrap();
    let categories = january.rows.iter().map(|row| row[2].clone()).collect::<Vec<_>>();
    assert_eq!(
        categories,
        [
            "Expenses:Financial",
            "Expenses:Food",
            "Expenses:Health",
            "Expenses:Home",
            "Expenses:Taxes",
            "Expenses:Transport"
        ]
        .map(Value::from)
        .to_vec()
    );
    let food = &january.rows[1];
    assert_eq!(food[0], Value::Int(2016));
    assert_eq!(food[1], Value::Int(1));
    let Value::Inventory(total) = &food[3] else {
        panic!("expected an inventory, got {:?}", food[3]);
    };
    let positions = total.positions().collect::<Vec<_>>();
    assert_eq!(positions.len(), 1);
    assert_eq!(positions[0].units.commodity, "USD");
    assert_eq!(positions[0].units.number, BigDecimal::from_str("515.05").unwrap());
    let Value::Inventory(taxes) = &january.rows[4][3] else { panic!() };
    assert_eq!(taxes.units().len(), 2);

    let february = query.execute(&ledger, &month(2016, 2)).unwrap();
    assert!(february.rows.iter().all(|row| row[1] == Value::Int(2)));
    assert!(!february.rows.is_empty());

    // a value of the wrong type is rejected before running
    let wrong = Params::new()
        .bind("pattern", 1)
        .bind("from", "2016-01-01")
        .bind("to", NaiveDate::from_ymd_opt(2016, 2, 1).unwrap());
    let err = query.execute(&ledger, &wrong).unwrap_err();
    assert!(err.message.contains(":pattern"), "{}", err);
}
