//! The testkit's own checks: the fixture corpus is enumerated as documented, loads once, and the helpers behave
//! as their callers expect. No zhang behaviour is asserted here.

use std::collections::HashSet;
use std::sync::Arc;

use zhang_core::data_type::Dialect;
use zhang_testkit::fixtures::{every_fixture_ledger, fava_demo, fixture_dir, fixture_ledger, integration_tests_dir, load_dir, oracle_ledgers};
use zhang_testkit::ledger::{load_ledger, load_text, Scratch};
use zhang_testkit::{golden, XorShift};

#[test]
fn every_fixture_ledger_lists_each_case_in_each_format_and_the_example() {
    let mut expected = vec![];
    let mut dirs = std::fs::read_dir(integration_tests_dir())
        .unwrap()
        .map(|it| it.unwrap().path())
        .filter(|it| it.is_dir())
        .collect::<Vec<_>>();
    dirs.sort();
    for dir in dirs {
        for entry in ["main.zhang", "main.bean"] {
            if dir.join(entry).is_file() {
                expected.push(format!("{}/{entry}", dir.file_name().unwrap().to_string_lossy()));
            }
        }
    }
    expected.push("examples/main.zhang".to_owned());
    let names = every_fixture_ledger().iter().map(|it| it.name.clone()).collect::<Vec<_>>();
    assert_eq!(names, expected);
    assert_eq!(names.iter().collect::<HashSet<_>>().len(), names.len(), "names are unique");
    assert!(names.contains(&"fava-demo-ledger/main.zhang".to_owned()));
    assert!(names.iter().any(|it| it.ends_with("/main.bean")), "the corpus has beancount ledgers too");
}

#[test]
fn a_fixture_ledger_knows_its_format_and_path() {
    let zhang = fixture_ledger("fava-demo-ledger", "main.zhang");
    assert_eq!(zhang.dialect, Dialect::Zhang);
    assert_eq!(zhang.path(), fixture_dir("fava-demo-ledger").join("main.zhang"));
    let bean = every_fixture_ledger().iter().find(|it| it.entry == "main.bean").expect("a beancount fixture");
    assert_eq!(bean.dialect, Dialect::Beancount);
    assert!(bean.path().is_file());
}

#[test]
fn a_fixture_ledger_loads_once_per_process() {
    let first = fava_demo();
    let second = fixture_ledger("fava-demo-ledger", "main.zhang").ledger();
    assert!(Arc::ptr_eq(&first, &second), "the same loaded ledger is shared");
    assert!(!first.directives.is_empty());
    let fresh = fixture_ledger("fava-demo-ledger", "main.zhang").load().unwrap();
    assert_eq!(fresh.directives.len(), first.directives.len(), "a fresh load reads the same ledger");
}

#[test]
fn load_dir_reads_each_format_with_its_parser_and_expands_wildcard_includes() {
    // the case `wildcard-include-directive-...` includes `data/*.zhang`
    let case = every_fixture_ledger()
        .iter()
        .find(|it| it.name.starts_with("wildcard-include-directive-"))
        .expect("the wildcard include fixture");
    let wildcard = load_dir(fixture_dir(case.dir.file_name().unwrap().to_str().unwrap()), &case.entry).unwrap();
    assert!(
        wildcard.visited_files.len() > 1,
        "the include with a wildcard was expanded: {:?}",
        wildcard.visited_files
    );
    let bean = every_fixture_ledger().iter().find(|it| it.entry == "main.bean").expect("a beancount fixture");
    assert_eq!(bean.load().unwrap().dialect, Dialect::Beancount);
}

#[test]
fn the_oracle_ledgers_are_the_bean_files_of_the_oracle_directory_in_order() {
    let names = oracle_ledgers().iter().map(|it| it.name.as_str()).collect::<Vec<_>>();
    assert!(!names.is_empty());
    assert!(names.iter().all(|it| it.starts_with("beancount-oracle/") && it.ends_with(".bean")), "{names:?}");
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(names, sorted);
    assert!(oracle_ledgers().iter().all(|it| it.dialect == Dialect::Beancount));
}

#[test]
fn text_and_scratch_ledgers_load_and_the_scratch_directory_is_removed() {
    const LEDGER: &str = "option \"operating_currency\" \"CNY\"\n2024-01-01 open Assets:Cash\n2024-01-01 open Expenses:Food\n2024-01-02 * \"lunch\"\n  Assets:Cash -10 CNY\n  Expenses:Food 10 CNY\n";
    let from_text = load_text(LEDGER);
    assert!(from_text.errors.is_empty(), "{:?}", from_text.errors);
    assert_eq!(from_text.dialect, Dialect::Zhang);

    let scratch = Scratch::zhang(LEDGER);
    assert_eq!(scratch.main(), "main.zhang");
    assert_eq!(scratch.read("main.zhang"), LEDGER);
    scratch.write("data/2024/01.zhang", "");
    assert!(scratch.dir().join("data/2024/01.zhang").is_file());
    let ledger = scratch.ledger().unwrap();
    assert_eq!(ledger.directives.len(), from_text.directives.len());
    assert_eq!(load_ledger(scratch.dir().to_path_buf(), "main.zhang").directives.len(), ledger.directives.len());
    let dir = scratch.dir().to_path_buf();
    drop(scratch);
    assert!(!dir.exists(), "the scratch directory is removed on drop");

    let bean = Scratch::beancount("2024-01-01 open Assets:Cash\n");
    assert_eq!(bean.ledger().unwrap().dialect, Dialect::Beancount);
}

#[test]
fn a_scratch_copy_of_a_fixture_is_a_separate_directory_with_the_same_files() {
    let fixture = fixture_ledger("fava-demo-ledger", "main.zhang");
    let copy = Scratch::copy_of(fixture);
    assert_ne!(copy.dir(), fixture.dir.as_path());
    assert_eq!(copy.main(), "main.zhang");
    assert_eq!(copy.read("main.zhang"), std::fs::read_to_string(fixture.path()).unwrap());
    assert_eq!(copy.ledger().unwrap().directives.len(), fixture.ledger().directives.len());
}

#[test]
fn xorshift_is_reproducible() {
    let mut a = XorShift(42);
    let mut b = XorShift(42);
    let first = (0..8).map(|_| a.next()).collect::<Vec<_>>();
    let second = (0..8).map(|_| b.next()).collect::<Vec<_>>();
    assert_eq!(first, second);
    assert!(first.iter().collect::<HashSet<_>>().len() > 1, "the sequence moves");
    let mut c = XorShift(7);
    assert!(c.below(10) < 10);
    assert_eq!(*c.pick(&["only"]), "only");
}

#[test]
fn a_golden_file_passes_when_equal_and_reads_back_as_json() {
    let scratch = Scratch::zhang("");
    let path = scratch.dir().join("expected.golden.json");
    let value = serde_json::json!({"a": [1, 2], "b": "text"});
    std::fs::write(&path, serde_json::to_string_pretty(&value).unwrap() + "\n").unwrap();
    golden::assert_json(&path, &value);
    assert_eq!(golden::read_json(&path), value);
    golden::assert_text(&path, &(serde_json::to_string_pretty(&value).unwrap() + "\n"));
}
