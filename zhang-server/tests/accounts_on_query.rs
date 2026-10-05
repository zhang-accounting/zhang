//! The account endpoints on the query engine (#479), checked on every fixture ledger: every `integration-tests`
//! ledger in each of its formats, the example ledger, the beancount oracle ledgers of
//! `extensions/beancount/tests/balance_assertions/`, and the ledgers of `tests/accounts_on_query/`. More ledgers can
//! be added with `ZHANG_ACCOUNTS_GOLDEN_EXTRA`, a `;`-separated list of `<dir>/<entry file>` paths.
//!
//! 1. **Against the store**, independently of the query engine. zhang records the order it processed the
//!    ledger in as the `sequence` of every transaction and checked assertion, and keeps every posting with
//!    its units. From these alone the test works out each account's journal (its postings and those of its
//!    sub-accounts, with their running balance, and its assertions, each where zhang checked it), its
//!    balance at the end of each day, its balances with and without sub-accounts, and its documents. The
//!    endpoints must return exactly that.
//! 2. **Pages**: the pages of every journal, put together, are the whole journal, and every page before the
//!    last one is full.
//!
//! The tests after the golden test check the endpoints and the journal order of the ledgers of
//! `tests/accounts_on_query/` with values worked out by hand.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::extract::{Path as UrlPath, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use bigdecimal::{BigDecimal, Zero};
use serde_json::{json, Map, Value};
use tokio::sync::RwLock;
use zhang_ast::{Directive, Spanned};
use zhang_core::clock::Clock;
use zhang_core::data_source::LocalFileSystemDataSource;
use zhang_core::data_type::text::ZhangDataType;
use zhang_core::data_type::DataType;
use zhang_core::ledger::{Ledger, LedgerProcessContext};
use zhang_core::store::DocumentType;
use zhang_server::request::AccountJournalRequest;
use zhang_server::routes::account::{get_account_balance_data, get_account_documents, get_account_info, get_account_journals, get_account_list};
use zhang_server::routes::Query as UrlQuery;
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
                dialect: zhang_core::data_type::Dialect::of(entry).unwrap(),
                visited_files: visited,
                data_source: source,
                clock: Clock::System,
            })
            .unwrap_or_else(|error| panic!("{}/{entry}: {error}", dir.display()))
        }
    };
    SharedLedger(Arc::new(RwLock::new(ledger)))
}

/// The status, the `X-Total-Count` header and the `data` of a response.
async fn respond_with_total(response: impl IntoResponse) -> (StatusCode, Option<u64>, Value) {
    let response = response.into_response();
    let status = response.status();
    let total = response.headers().get("X-Total-Count").map(|it| it.to_str().unwrap().parse::<u64>().unwrap());
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    (status, total, body.get("data").cloned().unwrap_or(body))
}

/// The status and the `data` of a response.
async fn respond(response: impl IntoResponse) -> (StatusCode, Value) {
    let (status, _, data) = respond_with_total(response).await;
    (status, data)
}

/// The whole journal of `account`.
async fn whole_journal(ledger: &SharedLedger, account: &str) -> (StatusCode, Value) {
    respond(get_account_journals(State(ledger.clone()), UrlPath((account.to_owned(),)), UrlQuery(Default::default())).await).await
}

/// Page `page` of `size` rows of the journal of `account`, with the number of rows of all its pages.
async fn journal_page(ledger: &SharedLedger, account: &str, page: u32, size: u32) -> (StatusCode, Option<u64>, Value) {
    let request = AccountJournalRequest {
        page: Some(page),
        size: Some(size),
    };
    respond_with_total(get_account_journals(State(ledger.clone()), UrlPath((account.to_owned(),)), UrlQuery(request)).await).await
}

/// Whether `text` is a decimal, such as `35.000`, or `0E-28`, as a zero of 28 decimals is written.
fn is_decimal(text: &str) -> bool {
    text.chars().any(|it| it.is_ascii_digit())
        && text.chars().all(|it| it.is_ascii_digit() || matches!(it, '.' | '-' | 'E'))
        && text.parse::<BigDecimal>().is_ok()
}

/// `value` with its decimal strings normalized, so `35` and `35.000` compare equal, and its objects
/// sorted by key.
fn canonical(value: &Value) -> Value {
    match value {
        Value::String(text) if is_decimal(text) => Value::String(decimal_string(&text.parse::<BigDecimal>().unwrap())),
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

/// A decimal as the canonical form writes it: without trailing zeros, zero as `0`.
fn decimal_string(number: &BigDecimal) -> String {
    if number.is_zero() {
        return "0".to_owned();
    }
    let normalized = number.normalized();
    if normalized.fractional_digit_count() < 0 {
        normalized.with_scale(0).to_string()
    } else {
        normalized.to_string()
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

#[derive(Default)]
struct Report {
    /// what disagrees with the store
    wrong: Vec<String>,
}

impl Report {
    /// `actual` must be `expected`, which the store says.
    fn expect(&mut self, ledger: &str, what: &str, account: &str, expected: &Value, actual: &Value) {
        if expected != actual {
            self.wrong.push(format!(
                "{ledger} {what} {account}\n  expected: {}\n  actual:   {}",
                short(expected),
                short(actual)
            ));
        }
    }
}

/// The fixture ledgers: every `integration-tests` ledger in each of its formats, the example ledger, the
/// beancount oracle ledgers, the ledgers of `tests/accounts_on_query/`, and those of
/// `ZHANG_ACCOUNTS_GOLDEN_EXTRA`.
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
    // the beancount oracle ledgers of pads, balances and document paths
    let oracle = root.join("extensions/beancount/tests/balance_assertions");
    let mut files = std::fs::read_dir(&oracle)
        .unwrap()
        .map(|it| it.unwrap().path())
        .filter(|it| it.extension().is_some_and(|extension| extension == "bean"))
        .collect::<Vec<_>>();
    files.sort();
    for file in files {
        let entry = file.file_name().unwrap().to_string_lossy().into_owned();
        fixtures.push((format!("balance_assertions/{entry}"), oracle.clone(), entry));
    }
    let own = root.join("zhang-server/tests/accounts_on_query");
    let mut files = std::fs::read_dir(&own).unwrap().map(|it| it.unwrap().path()).collect::<Vec<_>>();
    files.sort();
    for file in files {
        let entry = file.file_name().unwrap().to_string_lossy().into_owned();
        fixtures.push((format!("accounts_on_query/{entry}"), own.clone(), entry));
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

fn under(account: &str, ancestor: &str) -> bool {
    account == ancestor || account.strip_prefix(ancestor).is_some_and(|rest| rest.starts_with(':'))
}

// ---------------------------------------------------------------------------------------
// what the store says

/// A posting as zhang stored it.
struct StoredPosting {
    account: String,
    transaction: String,
    /// the place of its transaction in the order zhang processed the ledger
    sequence: i32,
    /// its date in the ledger's timezone
    date: String,
    number: BigDecimal,
    currency: String,
}

/// A balance assertion as zhang checked it.
struct StoredAssertion {
    account: String,
    /// the id zhang stored the check with, which /api/journals lists it with
    id: String,
    sequence: i32,
    asserted: (BigDecimal, String),
    balance: BigDecimal,
    passed: bool,
}

/// What the store of a ledger says, read without the query engine.
struct Stored {
    operating_currency: String,
    opened: BTreeSet<String>,
    /// the accounts of the `close` directives
    closed: BTreeSet<String>,
    /// in the order zhang processed them, the postings of a transaction in their order
    postings: Vec<StoredPosting>,
    assertions: Vec<StoredAssertion>,
    /// the `document` directives: account and path
    documents: Vec<(String, String)>,
}

impl Stored {
    async fn of(ledger: &SharedLedger) -> Stored {
        let guard = ledger.read().await;
        let store = guard.store.read().unwrap();
        let mut postings = store
            .postings
            .iter()
            .map(|posting| StoredPosting {
                account: posting.account.name().to_owned(),
                transaction: posting.trx_id.to_string(),
                sequence: posting.trx_sequence,
                date: posting.trx_datetime.naive_local().date().to_string(),
                number: posting.inferred_amount.number.clone(),
                currency: posting.inferred_amount.commodity.clone(),
            })
            .collect::<Vec<_>>();
        // stable: the postings of a transaction keep their order
        postings.sort_by_key(|posting| posting.sequence);
        let mut assertions = store
            .balance_assertions
            .iter()
            .map(|assertion| StoredAssertion {
                account: assertion.account.name().to_owned(),
                id: assertion.id.to_string(),
                sequence: assertion.sequence,
                asserted: (assertion.amount.number.clone(), assertion.amount.commodity.clone()),
                balance: assertion.balance.number.clone(),
                passed: assertion.passed,
            })
            .collect::<Vec<_>>();
        assertions.sort_by_key(|assertion| assertion.sequence);
        Stored {
            operating_currency: guard.options.operating_currency.clone(),
            opened: store.accounts.keys().cloned().collect(),
            closed: guard
                .directives
                .iter()
                .filter_map(|it| match &it.data {
                    Directive::Close(close) => Some(close.account.name().to_owned()),
                    _ => None,
                })
                .collect(),
            postings,
            assertions,
            documents: store
                .documents
                .iter()
                .map(|document| {
                    let DocumentType::Account(account) = &document.document_type;
                    (account.name().to_owned(), document.path.clone())
                })
                .collect(),
        }
    }

    /// The journal of `account` as the store says it, newest first, as the canonical rows the endpoint
    /// returns them: its postings and those of its sub-accounts, each with the running balance of the
    /// subtree in its currency, and the assertions on the account itself, each where zhang checked it,
    /// with the running balance there and the id of its check. Dates and descriptions are left out; a
    /// posting row has the id of its transaction.
    fn journal(&self, account: &str) -> Vec<Value> {
        // (sequence, then the postings of a transaction in their order and an assertion after them, row)
        let mut rows: Vec<((i32, usize), Value)> = vec![];
        let mut running: BTreeMap<&str, BigDecimal> = BTreeMap::new();
        let mut assertions = self.assertions.iter().filter(|it| it.account == account).peekable();
        let place = |rows: &mut Vec<((i32, usize), Value)>, running: &BTreeMap<&str, BigDecimal>, assertion: &StoredAssertion| {
            let (asserted, currency) = &assertion.asserted;
            let at = running.get(currency.as_str()).cloned().unwrap_or_else(BigDecimal::zero);
            rows.push((
                (assertion.sequence, 0),
                json!({
                    "account": assertion.account,
                    "id": assertion.id,
                    "units": format!("0 {currency}"),
                    "after": format!("{} {currency}", decimal_string(&at)),
                    "asserted": format!("{} {currency}", decimal_string(asserted)),
                    "checked": format!("{} {currency}", decimal_string(&assertion.balance)),
                    "passed": assertion.passed,
                }),
            ));
        };
        for (index, posting) in self.postings.iter().enumerate().filter(|(_, it)| under(&it.account, account)) {
            while let Some(assertion) = assertions.next_if(|it| it.sequence < posting.sequence) {
                place(&mut rows, &running, assertion);
            }
            let balance = running.entry(posting.currency.as_str()).or_insert_with(BigDecimal::zero);
            *balance += &posting.number;
            rows.push((
                (posting.sequence, index + 1),
                json!({
                    "account": posting.account,
                    "units": format!("{} {}", decimal_string(&posting.number), posting.currency),
                    "after": format!("{} {}", decimal_string(balance), posting.currency),
                    "transaction": posting.transaction,
                }),
            ));
        }
        for assertion in assertions {
            place(&mut rows, &running, assertion);
        }
        rows.sort_by_key(|(key, _)| std::cmp::Reverse(*key));
        rows.into_iter().map(|(_, row)| row).collect()
    }

    /// The balance of `account` and its sub-accounts at the end of each day with one of their postings, per
    /// currency, as the canonical history the endpoint returns.
    fn history(&self, account: &str) -> Value {
        let mut running: BTreeMap<&str, BigDecimal> = BTreeMap::new();
        let mut days: BTreeMap<&str, BTreeMap<&str, BigDecimal>> = BTreeMap::new();
        for posting in self.postings.iter().filter(|it| under(&it.account, account)) {
            let balance = running.entry(posting.currency.as_str()).or_insert_with(BigDecimal::zero);
            *balance += &posting.number;
            days.entry(posting.currency.as_str())
                .or_default()
                .insert(posting.date.as_str(), balance.clone());
        }
        let balance = days
            .into_iter()
            .map(|(currency, days)| {
                let days = days
                    .into_iter()
                    .map(|(date, balance)| json!({"date": date, "balance": {"number": decimal_string(&balance), "commodity": currency}}))
                    .collect::<Vec<_>>();
                (currency.to_owned(), Value::Array(days))
            })
            .collect::<Map<_, _>>();
        json!({ "balance": balance })
    }

    /// The units of the postings of the accounts `of` selects, per currency, a currency back at zero kept, and
    /// with the operating currency, as the canonical detail of the endpoints.
    fn units(&self, of: impl Fn(&str) -> bool) -> Value {
        let mut units: BTreeMap<&str, BigDecimal> = BTreeMap::new();
        units.insert(self.operating_currency.as_str(), BigDecimal::zero());
        for posting in self.postings.iter().filter(|it| of(&it.account)) {
            *units.entry(posting.currency.as_str()).or_insert_with(BigDecimal::zero) += &posting.number;
        }
        Value::Object(
            units
                .into_iter()
                .map(|(currency, number)| (currency.to_owned(), json!(decimal_string(&number))))
                .collect(),
        )
    }

    /// The `document` directives of `account` and its sub-accounts, in ledger order: account and path.
    fn documents(&self, account: &str) -> Value {
        json!(self
            .documents
            .iter()
            .filter(|(of, _)| under(of, account))
            .map(|(of, path)| json!([of, path]))
            .collect::<Vec<_>>())
    }
}

/// The rows of a canonical journal as [`Stored::journal`] writes them.
fn as_stored(journal: &[Value]) -> Vec<Value> {
    journal
        .iter()
        .map(|row| {
            let amount = |value: &Value| {
                let number = value["number"].as_str().unwrap();
                format!("{} {}", decimal_string(&number.parse().unwrap()), value["commodity"].as_str().unwrap())
            };
            if row["asserted"].is_null() {
                json!({
                    "account": row["account"],
                    "units": amount(&row["inferred_unit"]),
                    "after": amount(&row["account_after"]),
                    "transaction": row["trx_id"],
                })
            } else {
                json!({
                    "account": row["account"],
                    "id": row["trx_id"],
                    "units": amount(&row["inferred_unit"]),
                    "after": amount(&row["account_after"]),
                    "asserted": amount(&row["asserted"]),
                    "checked": amount(&row["checked_balance"]),
                    "passed": row["passed"],
                })
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------------------
// the check

async fn check(report: &mut Report, ledger_name: &str, ledger: &SharedLedger) {
    let stored = Stored::of(ledger).await;

    // the list
    let (status, list) = respond(get_account_list(State(ledger.clone())).await).await;
    assert_eq!(status, StatusCode::OK);
    let names = list
        .as_array()
        .unwrap()
        .iter()
        .map(|it| it["name"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    // every account with an `open` or `close` or a posting, by name
    let expected_names = stored
        .opened
        .iter()
        .chain(&stored.closed)
        .cloned()
        .chain(stored.postings.iter().map(|it| it.account.clone()))
        .collect::<BTreeSet<_>>();
    report.expect(ledger_name, "list names", "", &json!(expected_names.iter().collect::<Vec<_>>()), &json!(names));
    let accounts = list
        .as_array()
        .unwrap()
        .iter()
        .map(|it| (it["name"].as_str().unwrap().to_owned(), canonical(it)))
        .collect::<BTreeMap<_, _>>();
    for (name, account) in &accounts {
        // its own units and those with its sub-accounts, as the store has them
        report.expect(ledger_name, "list units", name, &stored.units(|it| it == name), &account["amount"]["detail"]);
        report.expect(
            ledger_name,
            "list units with sub-accounts",
            name,
            &stored.units(|it| under(it, name)),
            &account["balance_with_sub_accounts"],
        );
    }

    for account in universe(&[&list]) {
        let path = || UrlPath((account.clone(),));
        // what the endpoints of the page answer: a page for an account the store has, a 400 for a name that is no
        // account name, and a 404 for any other
        let expected_status = if !zhang_core::data_type::text::parser::is_valid_account_name(&account) {
            StatusCode::BAD_REQUEST
        } else if expected_names.contains(&account) {
            StatusCode::OK
        } else {
            StatusCode::NOT_FOUND
        };

        // the page
        let (status, page) = respond(get_account_info(State(ledger.clone()), path()).await).await;
        report.expect(ledger_name, "page status", &account, &json!(expected_status.as_u16()), &json!(status.as_u16()));
        if status == StatusCode::OK {
            let subtree_total = canonical(&page["amount_with_sub_accounts"]);
            report.expect(
                ledger_name,
                "page units with sub-accounts",
                &account,
                &stored.units(|it| under(it, &account)),
                &subtree_total["detail"],
            );
            // the value of the subtree is that of the accounts of the list in it
            let listed = accounts
                .iter()
                .filter(|(name, _)| under(name, &account))
                .map(|(_, it)| it["amount"]["calculated"]["number"].as_str().unwrap().parse::<BigDecimal>().unwrap())
                .fold(BigDecimal::zero(), |total, it| total + it);
            report.expect(
                ledger_name,
                "page value with sub-accounts",
                &account,
                &json!(decimal_string(&listed)),
                &subtree_total["calculated"]["number"],
            );
        }

        if expected_status != StatusCode::OK {
            let answers = [
                whole_journal(ledger, &account).await,
                respond(get_account_balance_data(State(ledger.clone()), path()).await).await,
                respond(get_account_documents(State(ledger.clone()), path()).await).await,
            ];
            let endpoints = [
                "GET /api/accounts/{a}/journals",
                "GET /api/accounts/{a}/balances",
                "GET /api/accounts/{a}/documents",
            ];
            for (endpoint, (status, _)) in endpoints.into_iter().zip(answers) {
                report.expect(ledger_name, endpoint, &account, &json!(expected_status.as_u16()), &json!(status.as_u16()));
            }
            continue;
        }

        // the journal
        let (status, journal) = whole_journal(ledger, &account).await;
        assert_eq!(status, StatusCode::OK, "{ledger_name} {account}");
        let journal = canonical(&journal).as_array().unwrap().clone();
        report.expect(ledger_name, "journal", &account, &json!(stored.journal(&account)), &json!(as_stored(&journal)));
        check_pages(report, ledger_name, ledger, &account, &journal).await;

        // the balance history
        let (status, history) = respond(get_account_balance_data(State(ledger.clone()), path()).await).await;
        assert_eq!(status, StatusCode::OK, "{ledger_name} {account}");
        report.expect(ledger_name, "history", &account, &canonical(&stored.history(&account)), &canonical(&history));

        // the documents
        let (status, documents) = respond(get_account_documents(State(ledger.clone()), path()).await).await;
        assert_eq!(status, StatusCode::OK, "{ledger_name} {account}");
        let listed = json!(canonical(&documents)
            .as_array()
            .unwrap()
            .iter()
            .map(|it| json!([it["account"], it["path"]]))
            .collect::<Vec<_>>());
        report.expect(ledger_name, "documents", &account, &stored.documents(&account), &listed);
    }
}

/// The pages of the journal of `account`, put together, are `journal`, the whole of it: with a size of 1,
/// 2 and 5 rows for a short journal, of 50 and 97 rows for a long one. Every page has the same total, the
/// number of rows of the journal; every page before the last one has `size` rows, the last one at least one,
/// and the page after it none.
async fn check_pages(report: &mut Report, ledger_name: &str, ledger: &SharedLedger, account: &str, journal: &[Value]) {
    let (_, total, _) = journal_page(ledger, account, 1, 1).await;
    let total = total.expect("a page has a total");
    let sizes: &[u32] = if total <= 60 { &[1, 2, 5] } else { &[50, 97] };
    for size in sizes {
        let mut rows = vec![];
        let pages = total.div_ceil(u64::from(*size)) as u32;
        for page in 1..=pages + 1 {
            let (status, page_total, data) = journal_page(ledger, account, page, *size).await;
            assert_eq!(status, StatusCode::OK, "{ledger_name} {account} page {page} of {size}");
            assert_eq!(page_total, Some(total), "{ledger_name} {account} page {page} of {size}");
            let data = canonical(&data).as_array().unwrap().clone();
            let expected = match page.cmp(&pages) {
                std::cmp::Ordering::Less => *size as usize,
                std::cmp::Ordering::Equal => (total - u64::from(*size) * u64::from(pages - 1)) as usize,
                std::cmp::Ordering::Greater => 0,
            };
            assert_eq!(data.len(), expected, "{ledger_name} {account} page {page} of {size}");
            rows.extend(data);
        }
        report.expect(ledger_name, "journal total", account, &json!(journal.len()), &json!(total));
        report.expect(ledger_name, &format!("pages of {size}"), account, &json!(journal), &json!(rows));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_account_endpoints_are_what_the_store_says() {
    let mut report = Report::default();
    for (name, dir, entry) in fixtures() {
        let ledger = load(&dir, &entry).await;
        check(&mut report, &name, &ledger).await;
    }
    assert!(report.wrong.is_empty(), "not what the store says:\n{}", report.wrong.join("\n"));
}

/// A ledger with one case of each change of #479, whose values are worked out by hand below.
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
  document: "receipts/split.pdf"
  Assets:Bank:Checking -3 CNY
    document: "receipts/checking.pdf"
  Assets:Bank:Checking -4 CNY
  Expenses:Food

2024-01-05 price CNY 20 JPY
2024-01-05 price USD 7 CNY
2024-01-05 price AAPL 12 USD
2999-01-01 price USD 100 CNY

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
}

/// An account's status is the account lifecycle's now, by the ledger's clock, the rule the ledger checks its directives
/// with: an account stays open through the day of a `close` with only a date, is closed after the time of a `close` with
/// a time, and is open again after a later `open`.
#[tokio::test]
async fn the_status_of_an_account_is_the_one_its_directives_are_checked_with() {
    let today = chrono::Utc::now().date_naive();
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("main.zhang"),
        format!(
            r#"option "timezone" "UTC"
1970-01-01 open Assets:Again
2000-01-01 close Assets:Again
2001-01-01 open Assets:Again
1970-01-01 open Assets:Gone
2000-01-01 close Assets:Gone
1970-01-01 open Assets:Today
{today} close Assets:Today
1970-01-01 open Assets:Midnight
{today} 00:00:00 close Assets:Midnight
"#
        ),
    )
    .unwrap();
    let ledger = load(dir.path(), "main.zhang").await;
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
            // opened again after its close
            ("Assets:Again", "Open"),
            ("Assets:Gone", "Close"),
            // closed today at 00:00, a time already past: the status is the one at this very moment
            ("Assets:Midnight", "Close"),
            // closed today, with only a date: open through today
            ("Assets:Today", "Open"),
        ]
    );
    for (account, status) in [
        ("Assets:Again", "Open"),
        ("Assets:Gone", "Close"),
        ("Assets:Midnight", "Close"),
        ("Assets:Today", "Open"),
    ] {
        let (_, page) = respond(get_account_info(State(ledger.clone()), UrlPath((account.to_owned(),))).await).await;
        assert_eq!(page["status"], json!(status), "{account}");
    }
}

/// Every endpoint of an account page answers a name that is no account, such as a parent without an account of its
/// own, with a 404, as the page does, and a name that is no account name with a 400 that says why.
#[tokio::test]
async fn a_name_that_is_no_account_is_a_404_and_one_that_is_no_account_name_a_400() {
    let (ledger, _dir) = differences().await;
    let statuses = |name: &'static str| {
        let ledger = ledger.clone();
        async move {
            let path = || UrlPath((name.to_owned(),));
            [
                respond(get_account_info(State(ledger.clone()), path()).await).await,
                whole_journal(&ledger, name).await,
                {
                    let (status, _, body) = journal_page(&ledger, name, 1, 10).await;
                    (status, body)
                },
                respond(get_account_balance_data(State(ledger.clone()), path()).await).await,
                respond(get_account_documents(State(ledger.clone()), path()).await).await,
            ]
        }
    };
    for name in ["Assets:Nowhere", "Expenses:Food:Coffee"] {
        for (status, body) in statuses(name).await {
            assert_eq!(status, StatusCode::NOT_FOUND, "{name}: {body}");
        }
    }
    for name in ["Assets", "Assets:Bad Name", "foo", "Assets::Bank"] {
        for (status, body) in statuses(name).await {
            assert_eq!(status, StatusCode::BAD_REQUEST, "{name}: {body}");
            assert!(
                body["message"].as_str().unwrap().starts_with(&format!("invalid account {name:?}")),
                "{name}: {body}"
            );
        }
    }
    // an account without `open` has a page
    for (status, body) in statuses("Assets:Ghost").await {
        assert_eq!(status, StatusCode::OK, "{body}");
    }
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
    // 10 AAPL at 12 USD, then 120 USD at 7 CNY: no AAPL price in CNY; the price of 2999 is not today's
    assert_eq!(value("Assets:Broker").await, (decimal("840"), json!("CNY")));
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
    let (_, journal) = whole_journal(&ledger, "Assets:Bank").await;
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

    // the documents of the subtree, in ledger order: its `document` directives, not the documents a transaction
    // names in its metadata or in that of a posting to the account
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

/// A ledger of `tests/accounts_on_query/`.
async fn fixture(name: &str) -> SharedLedger {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/accounts_on_query")
        .canonicalize()
        .unwrap();
    load(&dir, name).await
}

/// The rows of a journal as `account | payee | change | balance`, and for an assertion
/// `| = asserted, checked against, passed`.
fn lines(journal: &Value) -> Vec<String> {
    journal
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            let row = canonical(row);
            let amount = |value: &Value| format!("{} {}", value["number"].as_str().unwrap(), value["commodity"].as_str().unwrap());
            let mut line = format!(
                "{} | {} | {} | {}",
                row["account"].as_str().unwrap(),
                row["payee"].as_str().unwrap_or("NULL"),
                amount(&row["inferred_unit"]),
                amount(&row["account_after"])
            );
            if !row["asserted"].is_null() {
                line += &format!(" | = {}, {}, {}", amount(&row["asserted"]), amount(&row["checked_balance"]), row["passed"]);
            }
            line
        })
        .collect()
}

/// The ids of the assertions on `account`, in the order zhang checked them, newest first, and whether they
/// pad.
async fn assertion_ids(ledger: &SharedLedger, account: &str) -> Vec<(String, bool)> {
    let guard = ledger.read().await;
    let params = zhang_query::Params::new().push(account);
    let query = zhang_query::Query::compile_with_params(
        "SELECT id, pad IS NOT NULL FROM #balances WHERE account = $1 ORDER BY seq DESC",
        &params.types(),
    )
    .unwrap();
    let result = query.execute(&guard, &params).unwrap();
    result
        .rows
        .iter()
        .map(|row| (row[0].as_str().unwrap().to_owned(), row[1].as_bool().unwrap()))
        .collect()
}

/// A balance written with a time, as the web UI writes them, is checked after the transactions of its day
/// before that time: 90 after the lunch of midnight, 85 between the morning and the evening.
#[tokio::test]
async fn a_balance_with_a_time_stands_after_the_transactions_before_it() {
    let ledger = fixture("timed_balances.zhang").await;
    let (_, journal) = whole_journal(&ledger, "Assets:Cash").await;
    assert_eq!(
        lines(&journal),
        [
            "Assets:Cash | Shop | -5 CNY | 80 CNY",
            "Assets:Cash | Balance Check | 0 CNY | 85 CNY | = 85 CNY, 85 CNY, true",
            "Assets:Cash | Shop | -5 CNY | 85 CNY",
            "Assets:Cash | Balance Check | 0 CNY | 90 CNY | = 90 CNY, 90 CNY, true",
            "Assets:Cash | Shop | -10 CNY | 90 CNY",
            "Assets:Cash | Self | 100 CNY | 100 CNY",
        ]
    );
}

/// The assertions of a day with pads, on the parent of the padded accounts:
/// - on 02-01 the three balances of `Assets:Bank` stand in their order before the transaction of the day,
///   each checked against 100 CNY or 100 USD;
/// - on 03-01 `Assets:Bank:Checking` is padded by 111 to 200, then `Assets:Bank` by -111 back to 99; the
///   plain balance of 99 written after the pads stands after both paddings, and the `balance ... with pad`
///   of `Assets:Bank`, which zhang checks after every balance entry of the day, above it.
///
/// The sale of 12 AAPL booked against two lots is one row, the rows of a transaction newest first.
#[tokio::test]
async fn the_assertions_of_a_day_with_pads_stand_where_zhang_checks_them() {
    let ledger = fixture("pads_lots_and_prices.zhang").await;
    let (_, journal) = whole_journal(&ledger, "Assets:Bank").await;
    assert_eq!(
        lines(&journal),
        [
            "Assets:Bank:Checking | Shop | -1 CNY | 98 CNY",
            "Assets:Bank | Balance Check | 0 CNY | 99 CNY | = 99 CNY, 99 CNY, true",
            "Assets:Bank | Balance Check | 0 CNY | 99 CNY | = 99 CNY, 99 CNY, true",
            "Assets:Bank | Balance Pad | -111 CNY | 99 CNY",
            "Assets:Bank:Checking | Balance Pad | 111 CNY | 210 CNY",
            "Assets:Bank:Checking | Shop | -1 CNY | 99 CNY",
            "Assets:Bank | Balance Check | 0 USD | 100 USD | = 100 USD, 100 USD, true",
            "Assets:Bank | Balance Check | 0 CNY | 100 CNY | = 100 CNY, 100 CNY, true",
            "Assets:Bank | Balance Check | 0 CNY | 100 CNY | = 100 CNY, 100 CNY, true",
            "Assets:Bank:Checking | FX | -100 USD | 100 USD",
            "Assets:Bank:Checking | FX | 100 USD | 200 USD",
            "Assets:Bank | FX | 100 USD | 100 USD",
            "Assets:Bank:Checking:Deep | Self | 40 CNY | 100 CNY",
            "Assets:Bank:Checking | Self | 50 CNY | 60 CNY",
            "Assets:Bank | Self | 10 CNY | 10 CNY",
        ]
    );
    // the newest check is the `balance ... with pad`, the one below it the plain balance written after it
    let ids = assertion_ids(&ledger, "Assets:Bank").await;
    assert_eq!((ids[0].1, ids[1].1), (true, false));
    assert_eq!((&journal[1]["trx_id"], &journal[2]["trx_id"]), (&json!(ids[0].0), &json!(ids[1].0)));

    let (_, journal) = whole_journal(&ledger, "Assets:Bank:Checking").await;
    assert_eq!(
        lines(&journal),
        [
            "Assets:Bank:Checking | Shop | -1 CNY | 199 CNY",
            "Assets:Bank:Checking | Balance Check | 0 CNY | 200 CNY | = 200 CNY, 200 CNY, true",
            "Assets:Bank:Checking | Balance Pad | 111 CNY | 200 CNY",
            "Assets:Bank:Checking | Shop | -1 CNY | 89 CNY",
            "Assets:Bank:Checking | Balance Check | 0 CNY | 90 CNY | = 90 CNY, 90 CNY, true",
            "Assets:Bank:Checking | FX | -100 USD | 0 USD",
            "Assets:Bank:Checking | FX | 100 USD | 100 USD",
            "Assets:Bank:Checking:Deep | Self | 40 CNY | 90 CNY",
            "Assets:Bank:Checking | Self | 50 CNY | 50 CNY",
        ]
    );

    let (_, journal) = whole_journal(&ledger, "Assets:Broker").await;
    assert_eq!(
        lines(&journal),
        [
            "Assets:Broker | Broker | 180 USD | 180 USD",
            "Assets:Broker | Broker | -12 AAPL | 3 AAPL",
            "Assets:Broker | Broker | 5 AAPL | 15 AAPL",
            "Assets:Broker | Broker | 10 AAPL | 10 AAPL",
        ]
    );
}

/// After a `balance ... with pad`, the balance of the next day stands above it, and a failing check moved
/// nothing.
#[tokio::test]
async fn a_pad_between_two_checks_keeps_the_order_of_the_days() {
    let ledger = fixture("a_pad_between_checks.zhang").await;
    let (_, journal) = whole_journal(&ledger, "Assets:A").await;
    assert_eq!(
        lines(&journal),
        [
            "Assets:A | Balance Check | 0 CNY | 500 CNY | = 500 CNY, 500 CNY, true",
            "Assets:A | Balance Check | 0 CNY | 500 CNY | = 500 CNY, 500 CNY, true",
            "Assets:A | Balance Pad | 335 CNY | 500 CNY",
            "Assets:A | Balance Check | 0 CNY | 165 CNY | = 200 CNY, 165 CNY, false",
            "Assets:A | x | 165 CNY | 165 CNY",
        ]
    );
    // the newest is the balance of the 4th, then the pad's own check of the 3rd
    let dates = journal
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["datetime"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(&dates[..2], ["2024-01-04T00:00:00", "2024-01-03T00:00:00"]);
}

/// Two pads of sub-accounts that net to zero, then a plain balance and a `balance ... with pad` of the parent
/// on the same day: the plain balance after both paddings, the parent's pad (of nothing) checked last.
#[tokio::test]
async fn paddings_of_sub_accounts_come_before_the_checks_of_their_parent() {
    let ledger = fixture("pads_of_sub_accounts.zhang").await;
    let (_, journal) = whole_journal(&ledger, "Assets:A").await;
    assert_eq!(
        lines(&journal),
        [
            "Assets:A | Balance Check | 0 CNY | 100 CNY | = 100 CNY, 100 CNY, true",
            "Assets:A | Balance Check | 0 CNY | 100 CNY | = 100 CNY, 100 CNY, true",
            "Assets:A:C2 | Balance Pad | -50 CNY | 100 CNY",
            "Assets:A:C1 | Balance Pad | 50 CNY | 150 CNY",
            "Assets:A | x | 100 CNY | 100 CNY",
        ]
    );
    let ids = assertion_ids(&ledger, "Assets:A").await;
    assert_eq!((ids[0].1, ids[1].1), (true, false));
    assert_eq!((&journal[0]["trx_id"], &journal[1]["trx_id"]), (&json!(ids[0].0), &json!(ids[1].0)));
}

/// In New York, the transaction written at 02:30 on the day daylight saving starts, a time that does not
/// exist, is stored at 03:30 and listed in its place by the order it was written in, between the checks of
/// February and March; the failing check of March counts the sub-account.
#[tokio::test]
async fn the_journal_of_a_parent_follows_the_order_zhang_processed_the_ledger_in() {
    let ledger = fixture("pads_and_daylight_saving.zhang").await;
    let (_, journal) = whole_journal(&ledger, "Assets:Savings").await;
    assert_eq!(
        lines(&journal),
        [
            "Assets:Savings:Sub | dst | 1 CNY | 502 CNY",
            "Assets:Savings | Balance Check | 0 CNY | 501 CNY | = 500 CNY, 501 CNY, false",
            "Assets:Savings:Sub | dst | 1 CNY | 501 CNY",
            "Assets:Savings | Balance Check | 0 CNY | 500 CNY | = 500 CNY, 500 CNY, true",
            "Assets:Savings | Balance Check | 0 CNY | 500 CNY | = 500 CNY, 500 CNY, true",
            "Assets:Savings | Balance Pad | 335 CNY | 500 CNY",
            "Assets:Savings | x | 165 CNY | 165 CNY",
        ]
    );
    assert_eq!(journal[2]["datetime"], json!("2024-03-10T03:30:00"));
}

/// In London, which skips 01:00 to 02:00 on 2024-03-31, the transactions written at 01:10 and 01:40 are stored at
/// 02:10 and 02:40, after the one of 02:15, but zhang processes them by the time written: the balance of 7 checks
/// the two of the gap, and the cash ends at 4.
#[tokio::test]
async fn a_day_daylight_saving_skips_a_time_on_ends_with_the_posting_written_last() {
    let ledger = fixture("postings_in_a_daylight_saving_gap.zhang").await;
    let (_, journal) = whole_journal(&ledger, "Assets:Cash").await;
    assert_eq!(
        lines(&journal),
        [
            "Assets:Cash | Shop | -3 CNY | 4 CNY",
            "Assets:Cash | Balance Check | 0 CNY | 7 CNY | = 7 CNY, 7 CNY, true",
            "Assets:Cash | Shop | -2 CNY | 7 CNY",
            "Assets:Cash | Shop | -1 CNY | 9 CNY",
            "Assets:Cash | Self | 10 CNY | 10 CNY",
        ]
    );
    let path = || UrlPath(("Assets:Cash".to_owned(),));
    let (_, page) = respond(get_account_info(State(ledger.clone()), path()).await).await;
    assert_eq!(page["amount"]["detail"], json!({"CNY": "4"}));
    let (_, history) = respond(get_account_balance_data(State(ledger.clone()), path()).await).await;
    assert_eq!(
        history["balance"]["CNY"][1],
        json!({"date": "2024-03-31", "balance": {"number": "4", "commodity": "CNY"}})
    );
}

/// A transaction written without a narration has `""`; one written without any string has no payee either. An account without `open` has a journal like any other.
#[tokio::test]
async fn a_transaction_without_a_narration_has_an_empty_one() {
    let ledger = fixture("no_open_and_no_strings.zhang").await;
    let (_, journal) = whole_journal(&ledger, "Assets:Ghost").await;
    assert_eq!(
        lines(&journal),
        [
            "Assets:Ghost | NULL | 1 CNY | 8 CNY",
            "Assets:Ghost | Balance Check | 0 CNY | 7 CNY | = 7 CNY, 7 CNY, true",
            "Assets:Ghost | Ghost | 7 CNY | 7 CNY",
        ]
    );
    let narrations = journal.as_array().unwrap().iter().map(|row| row["narration"].clone()).collect::<Vec<_>>();
    assert_eq!(narrations, [json!(""), json!("Assets:Ghost"), json!("")]);
}

/// A page holds `size` rows from `(page - 1) * size`, counting a row per posting and per assertion, with the
/// number of rows of all the pages in `X-Total-Count`. The sale of 12 AAPL booked against two lots is one row.
#[tokio::test]
async fn a_journal_is_paged_by_rows() {
    let ledger = fixture("pads_lots_and_prices.zhang").await;
    let (_, total, page) = journal_page(&ledger, "Assets:Broker", 1, 1).await;
    // 180 USD, -12 AAPL, 5 AAPL and 10 AAPL
    assert_eq!(total, Some(4));
    assert_eq!(lines(&page), ["Assets:Broker | Broker | 180 USD | 180 USD"]);
    let (_, _, page) = journal_page(&ledger, "Assets:Broker", 2, 1).await;
    assert_eq!(lines(&page), ["Assets:Broker | Broker | -12 AAPL | 3 AAPL"]);
    let (_, _, page) = journal_page(&ledger, "Assets:Broker", 3, 1).await;
    assert_eq!(lines(&page), ["Assets:Broker | Broker | 5 AAPL | 15 AAPL"]);
    let (_, _, page) = journal_page(&ledger, "Assets:Broker", 2, 2).await;
    assert_eq!(
        lines(&page),
        ["Assets:Broker | Broker | 5 AAPL | 15 AAPL", "Assets:Broker | Broker | 10 AAPL | 10 AAPL"]
    );
    let (_, _, page) = journal_page(&ledger, "Assets:Broker", 5, 1).await;
    assert_eq!(lines(&page), Vec::<String>::new());

    // the assertions take their rows among the postings
    let (_, total, page) = journal_page(&ledger, "Assets:Bank", 2, 4).await;
    assert_eq!(total, Some(15));
    assert_eq!(
        lines(&page),
        [
            "Assets:Bank:Checking | Balance Pad | 111 CNY | 210 CNY",
            "Assets:Bank:Checking | Shop | -1 CNY | 99 CNY",
            "Assets:Bank | Balance Check | 0 USD | 100 USD | = 100 USD, 100 USD, true",
            "Assets:Bank | Balance Check | 0 CNY | 100 CNY | = 100 CNY, 100 CNY, true",
        ]
    );
    // the size defaults to 100, and the page to the first one
    let request = AccountJournalRequest { page: None, size: Some(3) };
    let (_, total, page) = respond_with_total(get_account_journals(State(ledger.clone()), UrlPath(("Assets:Bank".to_owned(),)), UrlQuery(request)).await).await;
    assert_eq!((total, page.as_array().unwrap().len()), (Some(15), 3));
    let request = AccountJournalRequest { page: Some(1), size: None };
    let (_, _, page) = respond_with_total(get_account_journals(State(ledger.clone()), UrlPath(("Assets:Bank".to_owned(),)), UrlQuery(request)).await).await;
    assert_eq!(page.as_array().unwrap().len(), 15);
    // without a page, the whole journal and no total
    let (_, total, _) =
        respond_with_total(get_account_journals(State(ledger.clone()), UrlPath(("Assets:Bank".to_owned(),)), UrlQuery(Default::default())).await).await;
    assert_eq!(total, None);
}

#[tokio::test]
async fn a_page_or_a_size_of_zero_is_a_bad_request() {
    let ledger = fixture("timed_balances.zhang").await;
    for (page, size, message) in [(0, 10, "page must be at least 1"), (1, 0, "size must be between 1 and 1000")] {
        let (status, _, body) = journal_page(&ledger, "Assets:Cash", page, size).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(body["message"], json!(message), "{body}");
    }
    // a page far past the end of the largest size is empty
    let (status, _, page) = journal_page(&ledger, "Assets:Cash", u32::MAX, 1000).await;
    assert_eq!((status, page), (StatusCode::OK, json!([])));
}

/// A page has at most 1000 rows: a larger size is a 400 that says so, whatever the page.
#[tokio::test]
async fn a_size_above_1000_is_a_bad_request() {
    let ledger = fixture("timed_balances.zhang").await;
    for (page, size) in [(1, 1001), (u32::MAX, u32::MAX)] {
        let (status, total, body) = journal_page(&ledger, "Assets:Cash", page, size).await;
        assert_eq!((status, total), (StatusCode::BAD_REQUEST, None), "{body}");
        assert_eq!(body["message"], json!("size must be between 1 and 1000"), "{body}");
    }
    let (status, total, page) = journal_page(&ledger, "Assets:Cash", 1, 1000).await;
    assert_eq!((status, total, page.as_array().unwrap().len()), (StatusCode::OK, Some(6), 6));
}

/// A sale of twenty lots is one row: the journal has the assertion, the sale and the twenty buys, 22 rows, and on
/// pages of five rows every page up to the fifth has rows, the sale on the first.
#[tokio::test]
async fn a_posting_of_many_lots_is_one_row() {
    let ledger = fixture("a_sale_of_many_lots.zhang").await;
    let (_, total, page) = journal_page(&ledger, "Assets:Broker", 1, 5).await;
    assert_eq!(total, Some(22));
    assert_eq!(
        lines(&page),
        [
            "Assets:Broker | Balance Check | 0 STK | 0 STK | = 0 STK, 0 STK, true",
            "Assets:Broker | Sell | -20 STK | 0 STK",
            "Assets:Broker | Buy | 1 STK | 20 STK",
            "Assets:Broker | Buy | 1 STK | 19 STK",
            "Assets:Broker | Buy | 1 STK | 18 STK",
        ]
    );
    for (page, rows) in [(2, 5), (3, 5), (4, 5), (5, 2), (6, 0)] {
        let (_, _, data) = journal_page(&ledger, "Assets:Broker", page, 5).await;
        assert_eq!(data.as_array().unwrap().len(), rows, "page {page}");
    }
    let (_, _, page) = journal_page(&ledger, "Assets:Broker", 5, 5).await;
    assert_eq!(lines(&page), ["Assets:Broker | Buy | 1 STK | 2 STK", "Assets:Broker | Buy | 1 STK | 1 STK"]);
}
