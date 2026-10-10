//! The journal and new-transaction endpoints on every ledger of `integration-tests` and `examples`, in each format it
//! has, the fava demo ledger, the survey probes and review ledgers in `tests/fixtures/journals`, and the beancount
//! ledgers of the balance assertion oracle in `extensions/beancount/tests/balance_assertions`: the running balances of
//! the journal follow their chains, its pages of every size are windows of the whole journal, and the suggestions of
//! the new-transaction form are sorted. The endpoints are checked with hand-verified values in `journals_engine.rs`.
//!
//! `ZHANG_GOLDEN_EXTRA=dir[:entry],...` checks more ledgers (such as a large generated one).

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::Path;

use axum::extract::State;
use axum::response::IntoResponse;
use serde_json::Value;
use zhang_query::Params;
use zhang_server::request::JournalRequest;
use zhang_server::routes::transaction::get_journals;
use zhang_server::routes::Query as UrlQuery;
use zhang_server::state::SharedLedger;
use zhang_testkit::fixtures::{every_fixture_ledger, oracle_ledgers, FixtureLedger};
use zhang_testkit::http::fixture_shared;

/// Every ledger of `integration-tests` in each format it has, the survey probes, the example ledger, the beancount
/// oracle ledgers, and the ledgers of `ZHANG_GOLDEN_EXTRA`.
fn fixtures() -> Vec<FixtureLedger> {
    let (examples, mut fixtures): (Vec<_>, Vec<_>) = every_fixture_ledger().iter().cloned().partition(|it| it.name.starts_with("examples/"));
    let probes = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/journals");
    let mut dirs = std::fs::read_dir(&probes)
        .unwrap()
        .map(|it| it.unwrap().path())
        .filter(|it| it.is_dir())
        .collect::<Vec<_>>();
    dirs.sort();
    for dir in dirs {
        for entry in ["main.zhang", "main.bean"] {
            if dir.join(entry).exists() {
                fixtures.push(FixtureLedger::new(
                    format!("{}/{}", dir.file_name().unwrap().to_string_lossy(), entry),
                    dir.clone(),
                    entry,
                ));
            }
        }
    }
    fixtures.extend(examples);
    // the beancount ledgers of the balance assertion oracle: pads, balances and document paths as beancount reads them
    fixtures.extend(oracle_ledgers().iter().cloned());
    if let Ok(extra) = std::env::var("ZHANG_GOLDEN_EXTRA") {
        for item in extra.split(',').filter(|it| !it.is_empty()) {
            let (dir, entry) = item.split_once(':').unwrap_or((item, "main.zhang"));
            fixtures.push(FixtureLedger::new(format!("{}/{}", dir, entry), dir, entry));
        }
    }
    fixtures
}

async fn json(response: impl IntoResponse) -> Value {
    let response = response.into_response();
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    if status.is_success() {
        body
    } else {
        serde_json::json!({ "status": status.as_u16(), "body": body })
    }
}

/// A search of the journal.
#[derive(Clone, Debug, Default)]
struct Search {
    keyword: Option<String>,
    tags: Option<Vec<String>>,
    links: Option<Vec<String>>,
}

impl Search {
    fn request(&self, page: u32, size: u32) -> JournalRequest {
        let set = |items: &Option<Vec<String>>| items.as_ref().map(|items| items.iter().cloned().collect::<HashSet<_>>());
        JournalRequest {
            page: Some(page),
            size: Some(size),
            keyword: self.keyword.clone(),
            tags: set(&self.tags),
            links: set(&self.links),
        }
    }
}

/// A page of the journal.
async fn journal_page(ledger: &SharedLedger, request: JournalRequest) -> Value {
    json(get_journals(State(ledger.clone()), UrlQuery(request)).await).await
}

/// Every record of the journal, page after page of 100.
async fn journal(ledger: &SharedLedger) -> Vec<Value> {
    let mut records = vec![];
    let mut page = 1;
    loop {
        let response = journal_page(ledger, Search::default().request(page, 100)).await;
        let data = &response["data"];
        records.extend(data["records"].as_array().cloned().unwrap_or_default());
        if u64::from(page) >= data["total_page"].as_u64().unwrap_or(0) {
            break;
        }
        page += 1;
    }
    records
}

fn number(value: &Value) -> Option<bigdecimal::BigDecimal> {
    value.as_str().and_then(|it| it.parse().ok())
}

/// The postings, `(item id, posting index)`, of the transactions and paddings of a whole journal (newest first) whose
/// account's running balance in their currency broke its chain at them or before: a posting's balance before is not
/// the balance after the previous posting of the account in that currency, or its balance after is not its balance
/// before plus its units. Both together leave no deviation that is consistent with the chain.
fn broken_chains(journal: &[Value]) -> BTreeSet<(String, usize)> {
    let mut after: BTreeMap<(String, String), bigdecimal::BigDecimal> = BTreeMap::new();
    let mut broken_accounts: BTreeSet<(String, String)> = BTreeSet::new();
    let mut broken = BTreeSet::new();
    for item in journal.iter().rev().filter(|it| it["type"] != "BalanceCheck") {
        for (index, posting) in item["postings"].as_array().into_iter().flatten().enumerate() {
            let account = (
                posting["account"].as_str().unwrap_or_default().to_owned(),
                posting["account_before"]["commodity"].as_str().unwrap_or_default().to_owned(),
            );
            let before = number(&posting["account_before"]["number"]).unwrap_or_default();
            let this_after = number(&posting["account_after"]["number"]).unwrap_or_default();
            let units = number(&posting["inferred_unit"]["number"]).unwrap_or_default();
            let previous = after.get(&account).cloned().unwrap_or_default();
            let same_currency = posting["inferred_unit"]["commodity"] == posting["account_before"]["commodity"];
            if before != previous || !same_currency || &this_after - &before != units {
                broken_accounts.insert(account.clone());
            }
            if broken_accounts.contains(&account) {
                broken.insert((item["id"].as_str().unwrap_or_default().to_owned(), index));
            }
            after.insert(account, this_after);
        }
    }
    broken
}

/// On every ledger, the journal's running balances follow their chains ([`broken_chains`]), its pages of every size
/// are windows of the whole journal, and the payees and accounts the new-transaction form suggests are sorted, each
/// once.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_journal_and_the_new_transaction_suggestions_hold_on_every_ledger() {
    for fixture in fixtures() {
        let ledger = fixture_shared(&fixture);
        let name = fixture.name.as_str();
        let everything = journal(&ledger).await;
        let broken = broken_chains(&everything);
        assert!(broken.is_empty(), "{}: the running balances break: {:?}", name, broken);

        // pages of every size are windows of the same journal
        let total = everything.len() as u32;
        for (page, size) in [(1, 1), (2, 1), (total.max(1), 1), (1, 7), (2, 7), (3, 7), (total + 2, 100), (2, 100)] {
            let response = journal_page(&ledger, Search::default().request(page, size)).await;
            let records = response["data"]["records"].as_array().cloned().unwrap_or_default();
            let from = ((page - 1) * size) as usize;
            let window = everything.iter().skip(from).take(size as usize).cloned().collect::<Vec<_>>();
            assert_eq!(records, window, "{} page={} size={}", name, page, size);
        }

        // the form's suggestions (`journals.payees`, `journals.accounts` at the ledger's current instant) are sorted. A
        // ledger without any account has no `ledger.now` row (the query reads `#accounts`), and no account to list.
        let now = zhang_server::builtin::run(&ledger, "ledger.now", Params::new()).await.unwrap();
        let accounts_at = match &now.rows[..] {
            [row] => Some(Params::new().bind("date", row[0].clone()).bind("time", row[1].clone())),
            [] => None,
            other => panic!("{}: ledger.now gave {:?}", name, other),
        };
        let queries = [("journals.payees", Some(Params::new())), ("journals.accounts", accounts_at)];
        for (query, params) in queries.into_iter().filter_map(|(query, params)| Some((query, params?))) {
            let values = zhang_server::builtin::run(&ledger, query, params)
                .await
                .unwrap()
                .rows
                .into_iter()
                .filter_map(|row| row[0].as_str().map(str::to_owned))
                .collect::<Vec<_>>();
            assert!(values.windows(2).all(|it| it[0] < it[1]), "{} {}: not sorted: {:?}", name, query, values);
        }
    }
}

/// A journal (newest first) of one account, from its postings' `(units, before, after)`, oldest first.
fn chain(postings: &[(i64, i64, i64)]) -> Vec<Value> {
    let amount = |number: i64| serde_json::json!({"number": number.to_string(), "commodity": "CNY"});
    postings
        .iter()
        .enumerate()
        .rev()
        .map(|(idx, (units, before, after))| {
            serde_json::json!({
                "type": "Transaction",
                "id": format!("t{}", idx),
                "postings": [{
                    "account": "Assets:Cash",
                    "inferred_unit": amount(*units),
                    "account_before": amount(*before),
                    "account_after": amount(*after),
                }],
            })
        })
        .collect()
}

/// The chain check catches a running balance that deviates consistently with the chain: from one posting on, every
/// balance is off by the same amount, so each balance before is the balance after the previous posting, but that
/// posting's balance after is not its balance before plus its units.
#[test]
fn a_running_balance_that_deviates_consistently_breaks_the_chain() {
    assert!(broken_chains(&chain(&[(-1, 0, -1), (-1, -1, -2), (-1, -2, -3)])).is_empty());
    let deviating = chain(&[(-1, 0, -1), (-1, -1, -3), (-1, -3, -4)]);
    let broken = broken_chains(&deviating);
    assert_eq!(broken, BTreeSet::from([("t1".to_owned(), 0), ("t2".to_owned(), 0)]));
    // a balance before that does not follow the previous balance after, as the old journal at a daylight saving gap
    let skipped = chain(&[(-1, 0, -1), (-1, -1, -2), (-1, -1, -2), (-1, -2, -3)]);
    assert_eq!(broken_chains(&skipped), BTreeSet::from([("t2".to_owned(), 0), ("t3".to_owned(), 0)]));
}
