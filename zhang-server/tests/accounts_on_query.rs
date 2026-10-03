//! The account endpoints on the query engine (#479), compared with the hand-written implementation they
//! replace (`legacy_*`, kept until wave 3), on every fixture ledger: every `integration-tests` ledger in each
//! of its formats and the example ledger. More ledgers can be added with `ZHANG_ACCOUNTS_GOLDEN_EXTRA`, a
//! `;`-separated list of `<dir>/<entry file>` paths, and `ZHANG_ACCOUNTS_GOLDEN_REPORT=1` prints every
//! difference.
//!
//! Every difference must be one #479 expects, and each kind is checked against the data, not taken on
//! trust:
//! - accounts without `open` are listed (a bug of the hand-written list);
//! - the order is deterministic: accounts by name, journals newest first by transaction then posting,
//!   balance histories by date (the hand-written ones came in hash map order);
//! - valuation uses the engine's price lookup, with inverse rates and the cost currency (decision 3);
//! - `trx_id` is the id of the transaction, not of the posting (a bug), and an assertion row's is its id;
//! - a parent account's page is its subtree (decision 6).
//!
//! The tests after the golden diff check each of these differences on a small ledger, with values worked
//! out by hand.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::extract::{Path as UrlPath, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use bigdecimal::BigDecimal;
use serde_json::{json, Map, Value};
use tokio::sync::RwLock;
use zhang_ast::{Directive, Spanned};
use zhang_core::clock::Clock;
use zhang_core::data_source::LocalFileSystemDataSource;
use zhang_core::data_type::text::ZhangDataType;
use zhang_core::data_type::DataType;
use zhang_core::ledger::{Ledger, LedgerProcessContext};
use zhang_server::routes::account::{
    get_account_balance_data, get_account_documents, get_account_info, get_account_journals, get_account_list, legacy_get_account_balance_data,
    legacy_get_account_documents, legacy_get_account_info, legacy_get_account_journals, legacy_get_account_list,
};
use zhang_server::state::SharedLedger;

/// Whether a file name matches a pattern of `include`, where `*` stands for any part of a name.
fn matches(pattern: &str, name: &str) -> bool {
    let parts = pattern.split('*').collect::<Vec<_>>();
    let (first, last) = (parts[0], parts[parts.len() - 1]);
    if parts.len() == 1 {
        return pattern == name;
    }
    if !name.starts_with(first) || !name[first.len()..].ends_with(last) || name.len() < first.len() + last.len() {
        return false;
    }
    let mut rest = &name[first.len()..name.len() - last.len()];
    for part in &parts[1..parts.len() - 1] {
        match rest.find(part) {
            Some(at) => rest = &rest[at + part.len()..],
            None => return false,
        }
    }
    true
}

/// The files an `include` names, its wildcards expanded, in name order.
fn included(base: &Path, pattern: &str) -> Vec<PathBuf> {
    let mut paths = vec![base.to_path_buf()];
    for component in pattern.split('/') {
        paths = paths
            .into_iter()
            .flat_map(|path| {
                if !component.contains('*') {
                    return vec![path.join(component)];
                }
                let mut entries = std::fs::read_dir(&path)
                    .map(|entries| {
                        entries
                            .filter_map(|entry| entry.ok())
                            .filter(|entry| matches(component, &entry.file_name().to_string_lossy()))
                            .map(|entry| entry.path())
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                entries.sort();
                entries
            })
            .collect();
    }
    paths.into_iter().filter(|it| it.is_file()).collect()
}

/// The directives of a ledger file and of the files it includes, wildcards expanded, as the server's
/// data source reads them.
fn directives(file: &Path, data_type: &dyn DataType<Carrier = String>, visited: &mut Vec<PathBuf>) -> Vec<Spanned<Directive>> {
    if visited.contains(&file.to_path_buf()) {
        return vec![];
    }
    visited.push(file.to_path_buf());
    let content = std::fs::read_to_string(file).unwrap_or_else(|error| panic!("{}: {error}", file.display()));
    let mut directives = data_type.transform(content, Some(file.to_string_lossy().into_owned())).unwrap();
    let includes = directives
        .iter()
        .filter_map(|it| match &it.data {
            Directive::Include(include) => Some(include.file.clone().to_plain_string()),
            _ => None,
        })
        .collect::<Vec<_>>();
    for include in includes {
        for path in included(file.parent().unwrap(), &include) {
            directives.extend(self::directives(&path, data_type, visited));
        }
    }
    directives
}

async fn load(dir: &Path, entry: &str) -> SharedLedger {
    let (source, data_type): (Arc<LocalFileSystemDataSource>, Box<dyn DataType<Carrier = String>>) = if entry.ends_with(".bean") {
        (
            Arc::new(LocalFileSystemDataSource::new(beancount::Beancount {})),
            Box::new(beancount::Beancount {}),
        )
    } else {
        (Arc::new(LocalFileSystemDataSource::new(ZhangDataType {})), Box::new(ZhangDataType {}))
    };
    let ledger = match Ledger::async_load(dir.to_path_buf(), entry.to_owned(), source.clone()).await {
        Ok(ledger) => ledger,
        // the local data source does not expand the wildcards of `include`
        Err(_) => {
            let mut visited = vec![];
            let directives = directives(&dir.join(entry), data_type.as_ref(), &mut visited);
            Ledger::process(LedgerProcessContext {
                directives,
                entry: (dir.to_path_buf(), entry.to_owned()),
                visited_files: visited,
                data_source: source,
                clock: Clock::System,
            })
            .unwrap_or_else(|error| panic!("{}/{entry}: {error}", dir.display()))
        }
    };
    SharedLedger(Arc::new(RwLock::new(ledger)))
}

/// The status and the `data` of a response.
async fn respond(response: impl IntoResponse) -> (StatusCode, Value) {
    let response = response.into_response();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    (status, body.get("data").cloned().unwrap_or(body))
}

fn is_decimal(text: &str) -> bool {
    text.chars().any(|it| it.is_ascii_digit()) && text.chars().all(|it| it.is_ascii_digit() || it == '.' || it == '-') && text.parse::<BigDecimal>().is_ok()
}

/// `value` with its decimal strings normalized, so `35` and `35.000` compare equal, and its objects
/// sorted by key.
fn canonical(value: &Value) -> Value {
    match value {
        Value::String(text) if is_decimal(text) => Value::String(text.parse::<BigDecimal>().unwrap().normalized().to_string()),
        Value::Array(items) => Value::Array(items.iter().map(canonical).collect()),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, value)| (key.clone(), canonical(value)))
                .collect::<BTreeMap<_, _>>()
                .into_iter()
                .collect::<Map<_, _>>(),
        ),
        other => other.clone(),
    }
}

fn short(value: &Value) -> String {
    let text = value.to_string();
    if text.chars().count() > 240 {
        format!("{}…", text.chars().take(240).collect::<String>())
    } else {
        text
    }
}

/// What makes a difference between the hand-written endpoints and the engine's expected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Reason {
    /// #479 bug: accounts without `open` are missing from the list and totals
    ListedWithoutOpen,
    /// #479 slice A: the order is deterministic
    DeterministicOrder,
    /// #479 decision 3: valuation uses the engine's price lookup
    Valuation,
    /// #479 bug: account journals return a posting id as `trx_id`
    TransactionId,
    /// #479 decision 6: a parent account's page is its subtree
    Subtree,
    /// no expected difference explains it
    Unexplained,
}

impl Reason {
    fn describe(self) -> &'static str {
        match self {
            Reason::ListedWithoutOpen => "bug: accounts without `open` were missing",
            Reason::DeterministicOrder => "deterministic order (was hash map order / posting order within a transaction)",
            Reason::Valuation => "decision 3: engine valuation (inverse rates, via the cost currency, latest of both directions)",
            Reason::TransactionId => "bug: `trx_id` was the posting id; an assertion row has its entry id",
            Reason::Subtree => "decision 6: a parent account's page is its subtree",
            Reason::Unexplained => "UNEXPLAINED",
        }
    }
}

struct Difference {
    ledger: String,
    endpoint: &'static str,
    account: String,
    reason: Reason,
    before: String,
    after: String,
}

#[derive(Default)]
struct Report {
    differences: Vec<Difference>,
    compared: usize,
}

impl Report {
    fn add(&mut self, ledger: &str, endpoint: &'static str, account: &str, reason: Reason, before: &Value, after: &Value) {
        self.differences.push(Difference {
            ledger: ledger.to_owned(),
            endpoint,
            account: account.to_owned(),
            reason,
            before: short(before),
            after: short(after),
        });
    }
}

/// The fixture ledgers: every `integration-tests` ledger in each of its formats, the example ledger, and those
/// of `ZHANG_ACCOUNTS_GOLDEN_EXTRA`.
fn fixtures() -> Vec<(String, PathBuf, String)> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").canonicalize().unwrap();
    let mut fixtures = vec![];
    let mut dirs = std::fs::read_dir(root.join("integration-tests"))
        .unwrap()
        .map(|it| it.unwrap().path())
        .filter(|it| it.is_dir())
        .collect::<Vec<_>>();
    dirs.sort();
    dirs.push(root.join("examples"));
    for dir in dirs {
        for entry in ["main.zhang", "main.bean"] {
            if dir.join(entry).exists() {
                let name = format!("{}/{entry}", dir.file_name().unwrap().to_string_lossy());
                fixtures.push((name, dir.clone(), entry.to_owned()));
            }
        }
    }
    for extra in std::env::var("ZHANG_ACCOUNTS_GOLDEN_EXTRA")
        .unwrap_or_default()
        .split(';')
        .filter(|it| !it.is_empty())
    {
        let path = PathBuf::from(extra);
        let dir = path.parent().unwrap().to_path_buf();
        fixtures.push((extra.to_owned(), dir, path.file_name().unwrap().to_string_lossy().into_owned()));
    }
    fixtures
}

/// The account names of a list, and every ancestor of them (`Assets` and `Assets:US` for
/// `Assets:US:Bank`), which may be parents without an account of their own.
fn universe(lists: &[&Value]) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for list in lists {
        for account in list.as_array().unwrap() {
            let name = account["name"].as_str().unwrap();
            for (at, _) in name.match_indices(':') {
                names.insert(name[..at].to_owned());
            }
            names.insert(name.to_owned());
        }
    }
    names
}

/// The fields in which two objects differ.
fn differing_fields(before: &Value, after: &Value) -> BTreeSet<String> {
    let empty = Map::new();
    let before = before.as_object().unwrap_or(&empty);
    let after = after.as_object().unwrap_or(&empty);
    before
        .keys()
        .chain(after.keys())
        .filter(|key| before.get(*key) != after.get(*key))
        .cloned()
        .collect()
}

/// Whether an account holds anything but the operating currency, which only a price values.
fn holds_other_currencies(account: &Value, operating_currency: &str) -> bool {
    account["amount"]["detail"]
        .as_object()
        .is_some_and(|detail| detail.iter().any(|(currency, number)| currency != operating_currency && number != "0"))
}

/// Why an account of the list or the page differs, `before` and `after` canonical.
fn account_reason(before: &Value, after: &Value, operating_currency: &str) -> Reason {
    let fields = differing_fields(before, after);
    let valuation_only = fields.iter().all(|field| field == "amount")
        && before["amount"]["detail"] == after["amount"]["detail"]
        && holds_other_currencies(after, operating_currency);
    if valuation_only {
        Reason::Valuation
    } else {
        Reason::Unexplained
    }
}

/// The rows of a hand-written account journal with the `trx_id` of each posting row made the id of its
/// transaction; `None` when a posting id is unknown.
fn with_transaction_ids(rows: &[Value], transaction_of: &HashMap<String, String>) -> Option<Vec<Value>> {
    rows.iter()
        .map(|row| {
            let mut row = row.clone();
            if row["asserted"].is_null() {
                row["trx_id"] = Value::String(transaction_of.get(row["trx_id"].as_str()?)?.clone());
            } else {
                row["trx_id"] = Value::Null;
            }
            Some(row)
        })
        .collect()
}

fn without_assertion_ids(rows: &[Value]) -> Vec<Value> {
    rows.iter()
        .map(|row| {
            let mut row = row.clone();
            if !row["asserted"].is_null() {
                row["trx_id"] = Value::Null;
            }
            row
        })
        .collect()
}

fn sorted(mut rows: Vec<Value>) -> Vec<Value> {
    rows.sort_by_key(|row| row.to_string());
    rows
}

fn without(rows: &[Value], field: &str) -> Vec<Value> {
    rows.iter()
        .map(|row| {
            let mut row = row.clone();
            row.as_object_mut().unwrap().remove(field);
            row
        })
        .collect()
}

/// Why an account journal differs, both canonical. A leaf account's journal must be the same but for the ids
/// and the order of the rows of one transaction; a parent account's is its subtree's, so its own postings and
/// its assertions must be those of before, and each assertion row stands where the running balance is the
/// balance it was checked against.
fn journal_reason(account: &str, before: &[Value], after: &[Value], transaction_of: &HashMap<String, String>) -> Reason {
    let Some(before) = with_transaction_ids(before, transaction_of) else {
        return Reason::Unexplained;
    };
    let after = without_assertion_ids(after);
    let subtree = after.iter().any(|row| row["account"] != account);
    if !subtree {
        if before == after {
            return Reason::TransactionId;
        }
        if sorted(before) == sorted(after) {
            return Reason::DeterministicOrder;
        }
        return Reason::Unexplained;
    }
    let own = |rows: &[Value]| {
        sorted(without(
            &rows
                .iter()
                .filter(|row| row["account"] == account && row["asserted"].is_null())
                .cloned()
                .collect::<Vec<_>>(),
            "account_after",
        ))
    };
    let assertions = |rows: &[Value]| {
        sorted(without(
            &rows.iter().filter(|row| !row["asserted"].is_null()).cloned().collect::<Vec<_>>(),
            "account_after",
        ))
    };
    let checked_where_they_stand = after
        .iter()
        .filter(|row| !row["asserted"].is_null())
        .all(|row| row["account_after"] == row["checked_balance"]);
    if own(&before) == own(&after) && assertions(&before) == assertions(&after) && checked_where_they_stand {
        Reason::Subtree
    } else {
        Reason::Unexplained
    }
}

/// The history of a canonical balance history response, each currency's days in date order.
fn by_date(history: &Value) -> Value {
    let mut history = history.clone();
    if let Some(currencies) = history["balance"].as_object_mut() {
        for days in currencies.values_mut() {
            if let Some(days) = days.as_array_mut() {
                days.sort_by_key(|day| day["date"].as_str().unwrap_or_default().to_owned());
            }
        }
    }
    history
}

/// The end of day balances a (canonical) journal implies: per currency and day, the balance after the newest
/// posting row of the day.
fn history_of_journal(journal: &[Value]) -> Value {
    let mut days: BTreeMap<String, BTreeMap<String, Value>> = BTreeMap::new();
    for row in journal.iter().rev().filter(|row| row["asserted"].is_null()) {
        let currency = row["account_after"]["commodity"].as_str().unwrap().to_owned();
        let day = row["datetime"].as_str().unwrap()[..10].to_owned();
        days.entry(currency).or_default().insert(day, row["account_after"].clone());
    }
    let balance = days
        .into_iter()
        .map(|(currency, days)| {
            let days = days
                .into_iter()
                .map(|(date, balance)| json!({"date": date, "balance": balance}))
                .collect::<Vec<_>>();
            (currency, Value::Array(days))
        })
        .collect::<Map<_, _>>();
    json!({ "balance": balance })
}

async fn compare(report: &mut Report, ledger_name: &str, ledger: &SharedLedger) {
    let (operating_currency, opened, transaction_of) = {
        let guard = ledger.read().await;
        let store = guard.store.read().unwrap();
        let transaction_of = store
            .postings
            .iter()
            .map(|posting| (posting.id.to_string(), posting.trx_id.to_string()))
            .collect::<HashMap<_, _>>();
        let opened = store.accounts.keys().cloned().collect::<BTreeSet<_>>();
        (guard.options.operating_currency.clone(), opened, transaction_of)
    };

    // the list
    let (status, before_list) = respond(legacy_get_account_list(State(ledger.clone())).await).await;
    assert_eq!(status, StatusCode::OK);
    let (status, after_list) = respond(get_account_list(State(ledger.clone())).await).await;
    assert_eq!(status, StatusCode::OK);
    let names = |list: &Value| {
        list.as_array()
            .unwrap()
            .iter()
            .map(|it| it["name"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>()
    };
    let before_names = names(&before_list);
    let after_names = names(&after_list);
    report.compared += 1;
    let mut sorted_before = before_names.clone();
    sorted_before.sort();
    if after_names.windows(2).any(|pair| pair[0] >= pair[1]) {
        report.add(
            ledger_name,
            "GET /api/accounts",
            "",
            Reason::Unexplained,
            &json!(before_names),
            &json!(after_names),
        );
    } else if sorted_before != before_names {
        report.add(
            ledger_name,
            "GET /api/accounts",
            "(order)",
            Reason::DeterministicOrder,
            &json!(before_names.iter().take(4).collect::<Vec<_>>()),
            &json!(after_names.iter().take(4).collect::<Vec<_>>()),
        );
    }
    let by_name = |list: &Value| {
        list.as_array()
            .unwrap()
            .iter()
            .map(|it| (it["name"].as_str().unwrap().to_owned(), canonical(it)))
            .collect::<BTreeMap<_, _>>()
    };
    let before_accounts = by_name(&before_list);
    let after_accounts = by_name(&after_list);
    for (name, after) in &after_accounts {
        match before_accounts.get(name) {
            None => {
                let reason = if opened.contains(name) {
                    Reason::Unexplained
                } else {
                    Reason::ListedWithoutOpen
                };
                report.add(ledger_name, "GET /api/accounts", name, reason, &Value::Null, after);
            }
            Some(before) if before != after => {
                report.add(
                    ledger_name,
                    "GET /api/accounts",
                    name,
                    account_reason(before, after, &operating_currency),
                    before,
                    after,
                );
            }
            Some(_) => {}
        }
    }
    for (name, before) in &before_accounts {
        if !after_accounts.contains_key(name) {
            report.add(ledger_name, "GET /api/accounts", name, Reason::Unexplained, before, &Value::Null);
        }
    }

    for account in universe(&[&before_list, &after_list]) {
        let path = || UrlPath((account.clone(),));
        report.compared += 4;

        // the page
        let (before_status, mut before) = respond(legacy_get_account_info(State(ledger.clone()), path()).await).await;
        let (after_status, mut after) = respond(get_account_info(State(ledger.clone()), path()).await).await;
        // the subtree total is new
        for page in [&mut before, &mut after] {
            if let Some(page) = page.as_object_mut() {
                page.remove("amount_with_sub_accounts");
            }
        }
        let (before, after) = (canonical(&before), canonical(&after));
        match (before_status, after_status) {
            (StatusCode::OK, StatusCode::OK) if before != after => {
                report.add(
                    ledger_name,
                    "GET /api/accounts/{a}",
                    &account,
                    account_reason(&before, &after, &operating_currency),
                    &before,
                    &after,
                );
            }
            (StatusCode::OK, StatusCode::OK) => {}
            (StatusCode::NOT_FOUND, StatusCode::OK) if !opened.contains(&account) && after_accounts.contains_key(&account) => {
                report.add(ledger_name, "GET /api/accounts/{a}", &account, Reason::ListedWithoutOpen, &json!("404"), &after);
            }
            (StatusCode::NOT_FOUND, StatusCode::NOT_FOUND) => {}
            _ => report.add(
                ledger_name,
                "GET /api/accounts/{a}",
                &account,
                Reason::Unexplained,
                &json!(before_status.as_u16()),
                &json!(after_status.as_u16()),
            ),
        }

        // the journal
        let (before_status, before) = respond(legacy_get_account_journals(State(ledger.clone()), path()).await).await;
        let (after_status, after) = respond(get_account_journals(State(ledger.clone()), path()).await).await;
        assert_eq!((before_status, after_status), (StatusCode::OK, StatusCode::OK), "{ledger_name} {account}");
        let before_journal = canonical(&before).as_array().unwrap().clone();
        let after_journal = canonical(&after).as_array().unwrap().clone();
        if before_journal != after_journal {
            let reason = journal_reason(&account, &before_journal, &after_journal, &transaction_of);
            let first_difference = before_journal
                .iter()
                .zip(&after_journal)
                .find(|(before, after)| before != after)
                .map(|(before, after)| (before.clone(), after.clone()))
                .unwrap_or_else(|| (json!(before_journal.len()), json!(after_journal.len())));
            report.add(
                ledger_name,
                "GET /api/accounts/{a}/journals",
                &account,
                reason,
                &first_difference.0,
                &first_difference.1,
            );
        }

        // the balance history
        let (before_status, before) = respond(legacy_get_account_balance_data(State(ledger.clone()), path()).await).await;
        let (after_status, after) = respond(get_account_balance_data(State(ledger.clone()), path()).await).await;
        assert_eq!((before_status, after_status), (StatusCode::OK, StatusCode::OK), "{ledger_name} {account}");
        let (before, after) = (canonical(&before), canonical(&after));
        // the engine's history is in date order, and the end of day balances of its journal
        assert_eq!(by_date(&after), after, "{ledger_name} {account}: the history is in date order");
        assert_eq!(
            history_of_journal(&after_journal),
            after,
            "{ledger_name} {account}: the history follows the journal"
        );
        if before != after {
            let subtree = after_journal.iter().any(|row| row["account"] != account.as_str());
            let reason = if by_date(&before) == after {
                Reason::DeterministicOrder
            } else if subtree {
                Reason::Subtree
            } else {
                Reason::Unexplained
            };
            report.add(ledger_name, "GET /api/accounts/{a}/balances", &account, reason, &before, &after);
        }

        // the documents
        let (before_status, before) = respond(legacy_get_account_documents(State(ledger.clone()), path()).await).await;
        let (after_status, after) = respond(get_account_documents(State(ledger.clone()), path()).await).await;
        assert_eq!((before_status, after_status), (StatusCode::OK, StatusCode::OK), "{ledger_name} {account}");
        let (before, after) = (canonical(&before), canonical(&after));
        if before != after {
            let own = after
                .as_array()
                .unwrap()
                .iter()
                .filter(|it| it["account"] == account.as_str())
                .cloned()
                .collect::<Vec<_>>();
            let reason = if Value::Array(own) == before { Reason::Subtree } else { Reason::Unexplained };
            report.add(ledger_name, "GET /api/accounts/{a}/documents", &account, reason, &before, &after);
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_account_endpoints_differ_from_the_hand_written_ones_only_as_479_expects() {
    let mut report = Report::default();
    for (name, dir, entry) in fixtures() {
        let ledger = load(&dir, &entry).await;
        compare(&mut report, &name, &ledger).await;
    }

    let mut summary: BTreeMap<(&str, Reason), (usize, BTreeSet<&str>)> = BTreeMap::new();
    for difference in &report.differences {
        let entry = summary.entry((difference.endpoint, difference.reason)).or_default();
        entry.0 += 1;
        entry.1.insert(difference.ledger.as_str());
    }
    println!("{} responses compared, {} differ", report.compared, report.differences.len());
    for ((endpoint, reason), (count, ledgers)) in &summary {
        println!(
            "{endpoint} | {} | {count} | {}",
            reason.describe(),
            ledgers.iter().cloned().collect::<Vec<_>>().join(", ")
        );
    }
    if std::env::var("ZHANG_ACCOUNTS_GOLDEN_REPORT").is_ok() {
        for difference in &report.differences {
            println!(
                "\n[{}] {} {} {}\n  before: {}\n  after:  {}",
                difference.reason.describe(),
                difference.ledger,
                difference.endpoint,
                difference.account,
                difference.before,
                difference.after
            );
        }
    }
    let unexplained = report
        .differences
        .iter()
        .filter(|it| it.reason == Reason::Unexplained)
        .map(|it| format!("{} {} {}\n  before: {}\n  after:  {}", it.ledger, it.endpoint, it.account, it.before, it.after))
        .collect::<Vec<_>>();
    assert!(unexplained.is_empty(), "unexplained differences:\n{}", unexplained.join("\n"));
}

/// A ledger with one case of each expected difference, whose values are worked out by hand below.
const DIFFERENCES: &str = r#"option "operating_currency" "CNY"
1970-01-01 commodity CNY
1970-01-01 commodity USD
1970-01-01 commodity JPY
1970-01-01 commodity AAPL
1970-01-01 open Assets:Bank
1970-01-01 open Assets:Bank:Checking
1970-01-01 open Assets:Bank:Savings
1970-01-01 open Assets:Travel
1970-01-01 open Assets:Broker
1970-01-01 open Equity:Open
1970-01-01 open Expenses:Food
2024-01-01 close Assets:Old

2024-01-01 * "Self" "opening"
  Assets:Bank 5 CNY
  Assets:Bank:Checking 60 CNY
  Assets:Bank:Savings 40 CNY
  Equity:Open -105 CNY

2024-01-02 * "Trip" "yen"
  Assets:Travel 2000 JPY
  Equity:Open -2000 JPY

2024-01-03 * "Broker" "buy"
  Assets:Broker 10 AAPL {10 USD}
  Equity:Open -100 USD

2024-01-04 * "Ghost" "an account without open"
  Assets:Ghost 7 CNY
  Equity:Open

2024-01-05 * "Split" "two postings to one account"
  Assets:Bank:Checking -3 CNY
  Assets:Bank:Checking -4 CNY
  Expenses:Food

2024-01-05 price CNY 20 JPY
2024-01-05 price USD 7 CNY
2024-01-05 price AAPL 12 USD

2024-01-06 document Assets:Bank:Savings "statements/savings.pdf"
2024-01-06 document Assets:Bank "bank.pdf"

2024-01-07 balance Assets:Bank 98 CNY
"#;

async fn differences() -> (SharedLedger, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("main.zhang"), DIFFERENCES).unwrap();
    (load(dir.path(), "main.zhang").await, dir)
}

fn number(value: &Value) -> BigDecimal {
    value.as_str().unwrap_or_else(|| panic!("{value} is a decimal string")).parse().unwrap()
}

fn decimal(text: &str) -> BigDecimal {
    text.parse().unwrap()
}

#[tokio::test]
async fn accounts_without_open_are_listed_by_name() {
    let (ledger, _dir) = differences().await;
    let (_, list) = respond(get_account_list(State(ledger.clone())).await).await;
    let listed = list
        .as_array()
        .unwrap()
        .iter()
        .map(|it| (it["name"].as_str().unwrap(), it["status"].as_str().unwrap()))
        .collect::<Vec<_>>();
    assert_eq!(
        listed,
        vec![
            ("Assets:Bank", "Open"),
            ("Assets:Bank:Checking", "Open"),
            ("Assets:Bank:Savings", "Open"),
            ("Assets:Broker", "Open"),
            // posted to without an `open`
            ("Assets:Ghost", "Open"),
            // closed without an `open`
            ("Assets:Old", "Close"),
            ("Assets:Travel", "Open"),
            ("Equity:Open", "Open"),
            ("Expenses:Food", "Open"),
        ]
    );
    let ghost = list.as_array().unwrap().iter().find(|it| it["name"] == "Assets:Ghost").unwrap();
    assert_eq!(ghost["amount"]["detail"], json!({"CNY": "7"}));

    // its page: dated by its first posting, as it has no `open`
    let (status, page) = respond(get_account_info(State(ledger.clone()), UrlPath(("Assets:Ghost".to_owned(),))).await).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        (&page["date"], &page["type"], &page["status"]),
        (&json!("2024-01-04T00:00:00"), &json!("Assets"), &json!("Open"))
    );
    // a parent that is no account has no page
    let (status, _) = respond(get_account_info(State(ledger), UrlPath(("Assets".to_owned(),))).await).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn holdings_are_valued_with_inverse_prices_and_through_their_cost_currency() {
    let (ledger, _dir) = differences().await;
    let value = |name: &'static str| {
        let ledger = ledger.clone();
        async move {
            let (_, page) = respond(get_account_info(State(ledger), UrlPath((name.to_owned(),))).await).await;
            (
                number(&page["amount"]["calculated"]["number"]),
                page["amount"]["calculated"]["commodity"].clone(),
            )
        }
    };
    // 2000 JPY at the inverse of `price CNY 20 JPY`: 2000 / 20
    assert_eq!(value("Assets:Travel").await, (decimal("100"), json!("CNY")));
    // 10 AAPL at 12 USD, then 120 USD at 7 CNY: no AAPL price in CNY
    assert_eq!(value("Assets:Broker").await, (decimal("840"), json!("CNY")));
    // the hand-written valuation found no direct price for either
    let (_, page) = respond(legacy_get_account_info(State(ledger.clone()), UrlPath(("Assets:Travel".to_owned(),))).await).await;
    assert_eq!(number(&page["amount"]["calculated"]["number"]), decimal("0"));
}

#[tokio::test]
async fn a_parent_account_shows_its_subtree() {
    let (ledger, _dir) = differences().await;
    let path = || UrlPath(("Assets:Bank".to_owned(),));

    // its own balance, and that of its subtree: 5 + 60 + 40 - 3 - 4
    let (_, page) = respond(get_account_info(State(ledger.clone()), path()).await).await;
    assert_eq!(page["amount"]["detail"], json!({"CNY": "5"}));
    assert_eq!(page["balance_with_sub_accounts"], json!({"CNY": "98"}));
    assert_eq!(number(&page["amount_with_sub_accounts"]["calculated"]["number"]), decimal("98"));
    assert_eq!(page["has_sub_accounts"], true);

    // the journal of the subtree, newest first, by transaction then posting, with the running balance of the subtree;
    // the assertion on the account is checked against the balance where it stands
    let (_, journal) = respond(get_account_journals(State(ledger.clone()), path()).await).await;
    let rows = journal
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            (
                row["account"].as_str().unwrap(),
                row["payee"].as_str().unwrap(),
                number(&row["inferred_unit"]["number"]),
                number(&row["account_after"]["number"]),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        rows,
        vec![
            ("Assets:Bank", "Balance Check", decimal("0"), decimal("98")),
            ("Assets:Bank:Checking", "Split", decimal("-4"), decimal("98")),
            ("Assets:Bank:Checking", "Split", decimal("-3"), decimal("102")),
            ("Assets:Bank:Savings", "Self", decimal("40"), decimal("105")),
            ("Assets:Bank:Checking", "Self", decimal("60"), decimal("65")),
            ("Assets:Bank", "Self", decimal("5"), decimal("5")),
        ]
    );
    let check = &journal[0];
    assert_eq!(
        (
            number(&check["asserted"]["number"]),
            number(&check["checked_balance"]["number"]),
            &check["passed"]
        ),
        (decimal("98"), decimal("98"), &json!(true))
    );
    // `trx_id` is the transaction's: the postings of a transaction share it
    let ids = journal
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["trx_id"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(ids[1], ids[2]);
    assert_eq!((ids[3], ids[4]), (ids[5], ids[5]));
    assert_ne!(ids[1], ids[3]);
    let transaction = {
        let guard = ledger.read().await;
        let store = guard.store.read().unwrap();
        store
            .transactions
            .values()
            .find(|it| it.payee.as_deref() == Some("Split"))
            .unwrap()
            .id
            .to_string()
    };
    assert_eq!(ids[1], transaction);

    // its balance at the end of each day with a posting
    let (_, history) = respond(get_account_balance_data(State(ledger.clone()), path()).await).await;
    assert_eq!(
        canonical(&history),
        canonical(&json!({"balance": {"CNY": [
            {"date": "2024-01-01", "balance": {"number": "105", "commodity": "CNY"}},
            {"date": "2024-01-05", "balance": {"number": "98", "commodity": "CNY"}},
        ]}}))
    );

    // the documents of the subtree, in ledger order
    let (_, documents) = respond(get_account_documents(State(ledger), path()).await).await;
    let documents = documents
        .as_array()
        .unwrap()
        .iter()
        .map(|it| (it["account"].as_str().unwrap(), it["filename"].as_str().unwrap(), it["path"].as_str().unwrap()))
        .collect::<Vec<_>>();
    assert_eq!(
        documents,
        vec![
            ("Assets:Bank:Savings", "savings.pdf", "statements/savings.pdf"),
            ("Assets:Bank", "bank.pdf", "bank.pdf")
        ]
    );
}
