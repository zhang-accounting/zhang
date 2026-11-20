//! The served ledger while it reloads: a reload, a write's own or the reload listener's, loads the next ledger with no
//! lock held and replaces the ledger served at once, so a reader never waits for a load; the writers still run one
//! after the other, each planning against the ledger its files hold, and a router plugin that may write runs between
//! them. The data source of these ledgers holds a load at a gate, after it read the files, for as long as a test needs:
//! what is asked meanwhile is answered from the ledger served, or waits for the writer before it.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use axum::extract::{Path as UrlPath, State};
use axum::http::{Method, StatusCode};
use axum::response::IntoResponse;
use axum::routing::any;
use axum::{Json, Router};
use serde_json::{json, Value};
use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use zhang_core::clock::Clock;
use zhang_core::data_source::{DataSource, LoadResult, SourceEntry};
use zhang_core::ledger::Ledger;
use zhang_core::plugin_policy::{PluginLevel, PluginPolicy};
use zhang_core::{ZhangError, ZhangResult};
use zhang_server::broadcast::{BroadcastEvent, Broadcaster};
use zhang_server::request::JournalRequest;
use zhang_server::routes::common::{get_basic_info, sse};
use zhang_server::routes::plugin_router::{route_to_plugin, PluginRouteState, ROUTE};
use zhang_server::routes::transaction::{create_new_transaction, delete_single_transaction, get_journals, update_single_transaction};
use zhang_server::routes::{IdempotencyKey, IfMatch, Query as UrlQuery, RequiredIfMatch};
use zhang_server::state::{SharedBroadcaster, SharedLedger, SharedReloadSender};
use zhang_server::{on_file_events, start_reload_listener, watch, ReloadSender};
use zhang_testkit::fixtures::data_source_for;
use zhang_testkit::http::{answer, json_request, respond, send, Answer};
use zhang_testkit::ledger::Scratch;

/// how long a request asked while a load is held may take: far longer than any answer from the ledger served, far
/// shorter than forever
const ANSWERED_WITHIN: Duration = Duration::from_secs(10);

/// how long a request that must wait for the writer before it is watched not to finish
const WAITS_FOR: Duration = Duration::from_millis(300);

const OPENS: &str = "option \"operating_currency\" \"CNY\"\n1970-01-01 commodity CNY\n1970-01-01 open Assets:Cash\n1970-01-01 open Expenses:Food\n";

/// A data source that reads and writes as `inner` does, except that a load, once it read the files, waits at a gate
/// while the gate is closed (at most a minute, so a failed test does not hang), and fails while `failing` is set.
struct Gated {
    inner: Arc<dyn DataSource>,
    closed: Mutex<bool>,
    opened: Condvar,
    /// the loads waiting at the gate
    waiting: AtomicUsize,
    failing: AtomicBool,
    /// the loads started
    loads: AtomicUsize,
}

impl Gated {
    fn new(inner: Arc<dyn DataSource>) -> Arc<Gated> {
        Arc::new(Gated {
            inner,
            closed: Mutex::new(false),
            opened: Condvar::new(),
            waiting: AtomicUsize::new(0),
            failing: AtomicBool::new(false),
            loads: AtomicUsize::new(0),
        })
    }

    fn close(&self) {
        *self.closed.lock().unwrap() = true;
    }

    fn open(&self) {
        *self.closed.lock().unwrap() = false;
        self.opened.notify_all();
    }

    /// once a load waits at the gate
    async fn a_load_waits(&self) {
        for _ in 0..1000 {
            if self.waiting.load(Ordering::SeqCst) > 0 {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("no load reached the gate");
    }
}

impl DataSource for Gated {
    fn export(&self, directive: zhang_ast::Directive) -> ZhangResult<Vec<u8>> {
        self.inner.export(directive)
    }

    fn get(&self, path: String) -> ZhangResult<Vec<u8>> {
        self.inner.get(path)
    }

    fn local_root(&self, entry: &std::path::Path) -> Option<std::path::PathBuf> {
        self.inner.local_root(entry)
    }

    fn get_limited(&self, path: String, max_len: u64) -> ZhangResult<Vec<u8>> {
        self.inner.get_limited(path, max_len)
    }

    fn list(&self, path: String, max_entries: usize) -> ZhangResult<Vec<SourceEntry>> {
        self.inner.list(path, max_entries)
    }

    fn get_to_replace(&self, path: String) -> ZhangResult<Option<Vec<u8>>> {
        self.inner.get_to_replace(path)
    }

    fn get_existing(&self, path: String) -> ZhangResult<Option<Vec<u8>>> {
        self.inner.get_existing(path)
    }

    fn load(&self, entry: String, endpoint: String) -> ZhangResult<LoadResult> {
        self.loads.fetch_add(1, Ordering::SeqCst);
        let loaded = self.inner.load(entry, endpoint);
        let closed = self.closed.lock().unwrap();
        if *closed {
            self.waiting.fetch_add(1, Ordering::SeqCst);
            let (closed, _) = self.opened.wait_timeout_while(closed, Duration::from_secs(60), |closed| *closed).unwrap();
            self.waiting.fetch_sub(1, Ordering::SeqCst);
            drop(closed);
        }
        if self.failing.load(Ordering::SeqCst) {
            return Err(ZhangError::CustomError("the load failed".to_owned()));
        }
        loaded
    }

    fn save(&self, ledger: &Ledger, path: String, content: &[u8]) -> ZhangResult<()> {
        self.inner.save(ledger, path, content)
    }
}

/// A ledger in a scratch directory, served as `zhang serve` serves it, with the reload listener running, through the
/// [`Gated`] source.
struct Served {
    scratch: Scratch,
    gate: Arc<Gated>,
    ledger: Arc<RwLock<Ledger>>,
    reload_sender: Arc<ReloadSender>,
    broadcaster: Arc<Broadcaster>,
    /// the plugin routes
    router: Router,
}

impl Served {
    /// `scratch`, served with its plugins bounded by `policy`
    fn new(scratch: Scratch, policy: PluginPolicy) -> Arc<Served> {
        let gate = Gated::new(data_source_for(scratch.main()));
        let source: Arc<dyn DataSource> = gate.clone();
        let ledger = Ledger::load_with_policy(scratch.dir().to_path_buf(), scratch.main().to_owned(), source, Clock::System, Arc::new(policy))
            .unwrap_or_else(|error| panic!("the ledger should load: {error}"));
        let ledger = Arc::new(RwLock::new(ledger));
        let (sender, receiver) = tokio::sync::mpsc::channel(8);
        let reload_sender = Arc::new(ReloadSender::new(sender));
        let broadcaster = Broadcaster::create();
        start_reload_listener(ledger.clone(), broadcaster.clone(), reload_sender.clone(), receiver);
        let router = Router::new()
            .route(ROUTE, any(route_to_plugin))
            .with_state(PluginRouteState::new(SharedLedger(ledger.clone()), reload_sender.clone()));
        Arc::new(Served {
            scratch,
            gate,
            ledger,
            reload_sender,
            broadcaster,
            router,
        })
    }

    /// `main.zhang` holding `content`, served
    fn with_main(content: &str) -> Arc<Served> {
        Served::new(Scratch::with_main("main.zhang", content), PluginPolicy::default())
    }

    fn shared(&self) -> State<SharedLedger> {
        State(SharedLedger(self.ledger.clone()))
    }

    fn sender(&self) -> State<SharedReloadSender> {
        State(SharedReloadSender(self.reload_sender.clone()))
    }

    /// `POST /api/transactions` of `request`, with the `Idempotency-Key` `key` when there is one
    async fn create(&self, key: Option<&str>, request: Value) -> Answer {
        let request = serde_json::from_value(request).expect("a create request");
        answer(create_new_transaction(self.shared(), self.sender(), IdempotencyKey(key.map(str::to_owned)), Json(request)).await).await
    }

    /// `PUT /api/transactions/{id}` of `request`, with `If-Match: <version>`
    async fn update(&self, id: &str, version: &str, request: Value) -> (StatusCode, Value) {
        let request = serde_json::from_value(request).expect("an update request");
        let if_match = IfMatch(Some(vec![version.to_owned()]));
        respond(update_single_transaction(self.shared(), self.sender(), UrlPath((id.to_owned(),)), if_match, Json(request)).await).await
    }

    /// `DELETE /api/transactions/{id}` with `If-Match: <version>`
    async fn delete(&self, id: &str, version: &str) -> (StatusCode, Value) {
        let if_match = RequiredIfMatch(vec![version.to_owned()]);
        respond(delete_single_transaction(self.shared(), self.sender(), UrlPath((id.to_owned(),)), if_match).await).await
    }

    /// the transactions of `GET /api/journals`, newest first
    async fn transactions(&self) -> Vec<Value> {
        let request = JournalRequest {
            page: None,
            size: Some(1000),
            keyword: None,
            tags: None,
            links: None,
        };
        let (status, body) = respond(get_journals(self.shared(), UrlQuery(request)).await).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let records = body["data"]["records"].as_array().expect("journal records").clone();
        records.into_iter().filter(|it| it["type"] == "Transaction").collect()
    }

    /// the payees of the journal, sorted
    async fn payees(&self) -> Vec<String> {
        let mut payees: Vec<String> = self
            .transactions()
            .await
            .iter()
            .map(|it| it["payee"].as_str().unwrap_or_default().to_owned())
            .collect();
        payees.sort();
        payees
    }

    /// the accounts the ledger served opens, sorted
    async fn accounts(&self) -> Vec<String> {
        let ledger = self.ledger.read().await;
        let mut accounts: Vec<String> = ledger
            .directives
            .iter()
            .filter_map(|it| match &it.data {
                zhang_ast::Directive::Open(open) => Some(open.account.name().to_owned()),
                _ => None,
            })
            .collect();
        accounts.sort();
        accounts
    }

    /// what the reads of a page ask, from the journal to `/api/info`, answered within [`ANSWERED_WITHIN`]: the payees
    /// of the journal
    async fn read_now(&self) -> Vec<String> {
        let read = async {
            let info = get_basic_info(self.shared(), self.sender()).await;
            assert!(info.is_ok(), "the basic info is answered");
            self.payees().await
        };
        tokio::time::timeout(ANSWERED_WITHIN, read)
            .await
            .expect("a read is answered while the ledger reloads")
    }

    fn read(&self, relative: &str) -> String {
        self.scratch.read(relative)
    }
}

/// a create request for a coffee of `amount` CNY paid to `payee` on 2024-01-15
fn coffee(payee: &str, amount: &str) -> Value {
    json!({
        "datetime": "2024-01-15T12:00:00",
        "payee": payee,
        "narration": "coffee",
        "postings": [{"account": "Assets:Cash", "unit": format!("-{amount} CNY")}, {"account": "Expenses:Food", "unit": null}],
        "metas": [],
        "tags": [],
        "links": [],
    })
}

/// `task` has not finished after [`WAITS_FOR`]: it waits
async fn waits<T>(task: &JoinHandle<T>, what: &str) {
    tokio::time::sleep(WAITS_FOR).await;
    assert!(!task.is_finished(), "{what} waits for the writer before it");
}

/// The reload a write runs before it answers: while it loads, the readers are answered from the ledger served, which
/// does not show the write yet, though its files do; the write answers once the ledger with it is served.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_read_while_a_write_reloads_is_answered_from_the_ledger_served() {
    let served = Served::with_main(OPENS);
    served.gate.close();
    let create = {
        let served = served.clone();
        tokio::spawn(async move { served.create(None, coffee("Cafe", "5")).await })
    };
    served.gate.a_load_waits().await;

    assert!(served.read("data/2024/01.zhang").contains("\"Cafe\""), "the write is in the files");
    assert!(served.read_now().await.is_empty(), "the ledger served is the one before the write");
    assert!(!create.is_finished(), "the write answers once its ledger is served");

    served.gate.open();
    let created = create.await.unwrap();
    assert_eq!(created.status, StatusCode::OK, "{}", created.body);
    let transactions = served.transactions().await;
    assert_eq!(transactions.len(), 1);
    assert_eq!(transactions[0]["id"], created.data()["id"], "the id answered is the one served");
}

/// A reload of the listener, as the file watcher asks for after an edit outside: the readers are answered from the
/// ledger served while it loads, and a write of the API sent meanwhile waits for it, writing nothing, then plans
/// against the ledger it loaded. Neither the edit nor the write is lost, and nothing is written twice.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_watcher_reload_and_an_api_write_keep_both() {
    let served = Served::with_main(OPENS);
    let main = served.read("main.zhang");
    // the edit outside, which the watcher sees
    served.scratch.write("main.zhang", &format!("{main}1970-01-01 open Expenses:Tea\n"));
    served.gate.close();
    served.reload_sender.reload();
    served.gate.a_load_waits().await;
    assert!(
        !served.accounts().await.contains(&"Expenses:Tea".to_owned()),
        "the ledger served is the one before the edit"
    );

    let create = {
        let served = served.clone();
        tokio::spawn(async move { served.create(None, coffee("Cafe", "5")).await })
    };
    waits(&create, "the write").await;
    assert!(served.read_now().await.is_empty());
    assert!(
        !served.scratch.dir().join("data/2024/01.zhang").exists(),
        "the write writes nothing while the reload runs"
    );

    served.gate.open();
    let created = create.await.unwrap();
    assert_eq!(created.status, StatusCode::OK, "{}", created.body);
    let main = served.read("main.zhang");
    assert!(main.contains("1970-01-01 open Expenses:Tea\n"), "the edit stays: {main}");
    assert_eq!(main.matches("include \"data/2024/01.zhang\"").count(), 1, "{main}");
    assert!(served.accounts().await.contains(&"Expenses:Tea".to_owned()), "the edit is served");
    assert_eq!(served.payees().await, vec!["Cafe"], "the write is served");
    // the watcher's reload after the write's own finds the files as they are
    served.reload_sender.reload_and_wait().await.unwrap();
    assert!(served.accounts().await.contains(&"Expenses:Tea".to_owned()));
    assert_eq!(served.payees().await, vec!["Cafe"]);
}

/// Two creates with one `Idempotency-Key`, the second sent while the first reloads: the second waits for the first's
/// reload, then finds the key in the ledger it loaded, writes nothing and answers the transaction written (#831).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_create_sent_again_while_the_first_reloads_writes_nothing() {
    let served = Served::with_main(OPENS);
    served.gate.close();
    let create = || {
        let served = served.clone();
        tokio::spawn(async move { served.create(Some("same-action"), coffee("Cafe", "5")).await })
    };
    let first = create();
    served.gate.a_load_waits().await;
    let second = create();
    waits(&second, "the create sent again").await;
    assert_eq!(served.read("data/2024/01.zhang").matches("idempotency-key").count(), 1);

    served.gate.open();
    let (first, second) = (first.await.unwrap(), second.await.unwrap());
    assert_eq!(first.status, StatusCode::OK, "{}", first.body);
    assert_eq!(second.status, StatusCode::OK, "{}", second.body);
    assert_eq!(first.data()["id"], second.data()["id"], "the second answers the transaction the first wrote");
    assert_eq!(second.header("Idempotency-Replayed").as_deref(), Some("true"));
    assert_eq!(served.read("data/2024/01.zhang").matches("idempotency-key").count(), 1, "written once");
    assert_eq!(served.transactions().await.len(), 1);
}

const TWINS: &str =
    "\n2024-01-02 * \"Cafe\" \"coffee\"\n  Assets:Cash -5 CNY\n  Expenses:Food\n\n2024-01-02 * \"Cafe\" \"coffee\"\n  Assets:Cash -5 CNY\n  Expenses:Food\n";

/// A delete of the first of two twins, and an update naming the same version sent while the delete reloads: the update
/// waits for the delete's reload, then finds the twin in the place and the id of the first, with the version the
/// delete recorded as a tombstone (#818), and is refused. The twin is as it was.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_update_of_a_twin_sent_while_its_delete_reloads_is_refused() {
    let served = Served::with_main(&format!("{OPENS}{TWINS}"));
    let oldest = served.transactions().await.pop().expect("a transaction");
    let (id, version) = (oldest["id"].as_str().unwrap().to_owned(), oldest["version"].as_str().unwrap().to_owned());
    served.gate.close();
    let delete = {
        let (served, id, version) = (served.clone(), id.clone(), version.clone());
        tokio::spawn(async move { served.delete(&id, &version).await })
    };
    served.gate.a_load_waits().await;
    let update = {
        let served = served.clone();
        let tea = json!({
            "datetime": "2024-01-02T12:00:00", "payee": "Cafe", "narration": "tea",
            "postings": [{"account": "Assets:Cash", "unit": "-4 CNY"}, {"account": "Expenses:Food", "unit": null}],
            "metas": [], "tags": [], "links": [],
        });
        tokio::spawn(async move { served.update(&id, &version, tea).await })
    };
    waits(&update, "the update").await;

    served.gate.open();
    let (deleted, body) = delete.await.unwrap();
    assert_eq!(deleted, StatusCode::OK, "{body}");
    let (updated, body) = update.await.unwrap();
    assert_eq!(updated, StatusCode::PRECONDITION_FAILED, "{body}");
    let text = served.read("main.zhang");
    assert_eq!(text.matches("\"Cafe\" \"coffee\"").count(), 1, "{text}");
    assert!(!text.contains("tea"), "{text}");
    assert_eq!(served.transactions().await.len(), 1);
}

/// a router plugin that previews and commits the batch of the request body, answering the commit, or the error raised
const ROUTER_WRITE: &str = r#"
local M = { name = "router-write", version = "0.1.0", writes = { "transaction" } }
function M.router(request)
  local ok, result = pcall(function()
    local batch = json.decode(request.body).directives
    local preview = zhang.write.preview(batch)
    return zhang.write.commit(batch, preview.token)
  end)
  if not ok then return { json = { error = result } } end
  return { json = result }
end
return M
"#;

/// a router plugin that writes nothing
const ROUTER_READ: &str = r#"
local M = { name = "router-read", version = "0.1.0" }
function M.router(request)
  return { json = { path = request.path } }
end
return M
"#;

/// `main.zhang` declaring the router plugins of [`ROUTER_WRITE`], granted transaction writes, and [`ROUTER_READ`],
/// served with plugin writes allowed
fn with_router_plugins() -> Arc<Served> {
    let scratch = Scratch::with_main("main.zhang", "");
    std::fs::write(scratch.dir().join("router_write.lua"), ROUTER_WRITE).unwrap();
    std::fs::write(scratch.dir().join("router_read.lua"), ROUTER_READ).unwrap();
    let main = format!(
        "option \"features.plugin\" \"true\"\noption \"operating_currency\" \"USD\"\n\
         plugin \"{}\"\n  timeout: \"20s\"\n  allowed_writes: \"transaction\"\n\
         plugin \"{}\"\n  timeout: \"20s\"\n\
         1970-01-01 open Assets:Cash\n1970-01-01 open Expenses:Food USD\n",
        scratch.dir().join("router_write.lua").display(),
        scratch.dir().join("router_read.lua").display(),
    );
    scratch.write("main.zhang", &main);
    let policy = PluginPolicy {
        level: PluginLevel::Write,
        ..PluginPolicy::default()
    };
    Served::new(scratch, policy)
}

/// a transaction of the plugins' ledger, paid to `payee` on `date`
fn purchase(payee: &str, date: &str) -> Value {
    json!({"datetime": date, "payee": payee, "narration": "coffee", "flag": "*",
        "postings": [{"account": "Expenses:Food", "unit": {"number": "4.50", "commodity": "USD"}}, {"account": "Assets:Cash"}],
        "tags": [], "links": [], "metas": []})
}

/// A write of the API reloads, and meanwhile a router plugin that may write is asked to commit to the same new monthly
/// file: the plugin waits for the write's reload, then plans against the ledger with the write, so the main file gets
/// one `include` of that file (#803). A plugin that may not write is answered meanwhile, as any reader.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_plugin_commit_sent_while_a_write_reloads_waits_for_it() {
    let served = with_router_plugins();
    served.gate.close();
    let create = {
        let served = served.clone();
        tokio::spawn(async move { served.create(None, purchase("Tea House", "2026-10-14T12:00:00")).await })
    };
    served.gate.a_load_waits().await;
    let plugin = {
        let served = served.clone();
        let mut directive = purchase("Blue Bottle", "2026-10-13T12:00:00");
        directive["kind"] = json!("transaction");
        let body = json!({"directives": [directive]});
        tokio::spawn(async move { send(&served.router, json_request(Method::POST, "/api/plugins/router-write/x", &[], Some(&body))).await })
    };
    waits(&plugin, "the plugin that may write").await;
    let reader = send(&served.router, json_request(Method::GET, "/api/plugins/router-read/x", &[], None));
    let reader = tokio::time::timeout(ANSWERED_WITHIN, reader)
        .await
        .expect("a plugin that may not write is answered while the ledger reloads");
    assert_eq!(reader.status, 200, "{}", reader.text());

    served.gate.open();
    let created = create.await.unwrap();
    assert_eq!(created.status, StatusCode::OK, "{}", created.body);
    let committed = plugin.await.unwrap();
    assert_eq!(committed.status, 200, "{}", committed.text());
    assert_eq!(committed.json()["entries"], 1, "{}", committed.text());
    served.reload_sender.reload_and_wait().await.unwrap();
    let main = served.read("main.zhang");
    assert_eq!(main.matches("include \"data/2026/10.zhang\"").count(), 1, "{main}");
    assert_eq!(served.payees().await, vec!["Blue Bottle", "Tea House"]);
}

/// A write whose own reload fails: it is answered, without an id, the failure is kept for `/api/info` and told to the
/// readers, and the ledger served stays the one before, stale, read on all along. The next write reloads it first, once
/// the files load again, and both writes are served.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_write_whose_reload_fails_is_answered_and_the_next_write_reloads_first() {
    let served = Served::with_main(OPENS);
    let mut events = served.broadcaster.new_client().await;
    served.gate.failing.store(true, Ordering::SeqCst);
    served.gate.close();
    let create = {
        let served = served.clone();
        tokio::spawn(async move { served.create(None, coffee("Cafe", "5")).await })
    };
    served.gate.a_load_waits().await;
    assert!(served.read_now().await.is_empty());
    served.gate.open();

    let created = create.await.unwrap();
    assert_eq!(created.status, StatusCode::OK, "{}", created.body);
    assert!(created.data()["id"].is_null(), "no id without the ledger with the write: {}", created.body);
    let failure = served.reload_sender.last_failure().expect("the failure is kept for /api/info");
    assert!(failure.message.contains("the load failed"), "{failure:?}");
    let mut told = false;
    while let Ok(Some(event)) = tokio::time::timeout(Duration::from_secs(5), events.recv()).await {
        if format!("{event:?}").contains("ReloadFailed") {
            told = true;
            break;
        }
    }
    assert!(told, "the readers are told the reload failed");
    assert!(served.ledger.read().await.stale, "the ledger served is behind the files");
    assert!(served.read_now().await.is_empty(), "the ledger served is the one before the write");

    served.gate.failing.store(false, Ordering::SeqCst);
    let second = served.create(None, coffee("Bakery", "3")).await;
    assert_eq!(second.status, StatusCode::OK, "{}", second.body);
    assert!(!second.data()["id"].is_null(), "{}", second.body);
    assert_eq!(served.payees().await, vec!["Bakery", "Cafe"]);
    assert!(served.reload_sender.last_failure().is_none(), "a reload that succeeds clears the failure");
    assert!(!served.ledger.read().await.stale);
}

/// An edit outside to the file a write newly includes, made while the write's reload loads: the file watcher looks at
/// its events once the write swapped its ledger in, which reads that file, so the edit is reloaded and served. Looked
/// at against the ledger served before, which does not read the file, the edit was no change, and it was lost
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_edit_during_a_reload_to_a_file_it_newly_includes_is_served() {
    let served = Served::with_main(OPENS);
    served.gate.close();
    let create = {
        let served = served.clone();
        tokio::spawn(async move { served.create(None, coffee("Cafe", "5")).await })
    };
    // the write created the file, included it, and its load read it
    served.gate.a_load_waits().await;
    let data = served.scratch.dir().join("data/2024/01.zhang");
    let written = std::fs::read_to_string(&data).unwrap();
    std::fs::write(
        &data,
        format!("{written}\n{}", "2024-01-20 * \"Outside\" \"edit\"\n  Assets:Cash -1 CNY\n  Expenses:Food\n"),
    )
    .unwrap();
    let watcher = {
        let served = served.clone();
        let edit = notify::Event::new(notify::EventKind::Modify(notify::event::ModifyKind::Data(notify::event::DataChange::Content))).add_path(data.clone());
        tokio::spawn(async move {
            let roots = watch::watch_roots(served.scratch.dir());
            on_file_events(&served.ledger, &served.reload_sender, &roots, &[Ok(edit)]).await
        })
    };
    waits(&watcher, "the watcher").await;

    served.gate.open();
    let created = create.await.unwrap();
    assert_eq!(created.status, StatusCode::OK, "{}", created.body);
    watcher.await.unwrap();
    let mut payees = vec![];
    for _ in 0..100 {
        payees = served.payees().await;
        if payees.len() == 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(payees, vec!["Cafe", "Outside"], "the edit is served");
}

/// An SSE client that never reads, as a suspended phone: the reloads tell it what its buffer holds and go on, and
/// neither the writers nor the readers wait for it
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_sse_client_that_does_not_read_holds_up_nothing() {
    let served = Served::with_main(OPENS);
    let _stalled = served.broadcaster.new_client().await;
    let work = async {
        // more reloads than its buffer holds events
        for _ in 0..15 {
            served.reload_sender.reload_and_wait().await.unwrap();
        }
        let created = served.create(None, coffee("Cafe", "5")).await;
        assert_eq!(created.status, StatusCode::OK, "{}", created.body);
        served.read_now().await
    };
    let payees = tokio::time::timeout(Duration::from_secs(60), work)
        .await
        .expect("the reloads, the write and the reads do not wait for the client");
    assert_eq!(payees, vec!["Cafe"]);
}

/// A delete of the first of two twins whose client goes away while the delete reloads: the delete is in the files,
/// and so is its tombstone, recorded with the write, so the delete sent again with the same version is refused, not
/// taken for the twin that has its place and its id now (#818)
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_delete_whose_client_goes_away_while_it_reloads_keeps_its_tombstone() {
    let served = Served::with_main(&format!("{OPENS}{TWINS}"));
    let oldest = served.transactions().await.pop().expect("a transaction");
    let (id, version) = (oldest["id"].as_str().unwrap().to_owned(), oldest["version"].as_str().unwrap().to_owned());
    served.gate.close();
    let delete = {
        let (served, id, version) = (served.clone(), id.clone(), version.clone());
        tokio::spawn(async move { served.delete(&id, &version).await })
    };
    served.gate.a_load_waits().await;
    // the client goes away: its request is dropped during the reload
    delete.abort();
    assert!(delete.await.unwrap_err().is_cancelled());
    assert_eq!(
        served.read("main.zhang").matches("\"Cafe\" \"coffee\"").count(),
        1,
        "the delete is in the files"
    );
    served.gate.open();

    // sent again, as a client sends a request whose answer it lost
    let (status, body) = served.delete(&id, &version).await;
    assert_eq!(status, StatusCode::PRECONDITION_FAILED, "{body}");
    assert_eq!(served.read("main.zhang").matches("\"Cafe\" \"coffee\"").count(), 1, "the twin stays");
    assert_eq!(served.transactions().await.len(), 1);
}

/// The reload the file watcher asks for after a write of the server, which reloaded the ledger before it answered,
/// loads nothing: the files hold what the ledger served was loaded from, byte for byte. An edit outside loads, and so
/// does a file written again as it was while the last reload failed, which clears the failure.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_watchers_reload_after_a_write_loads_only_an_edit_outside() {
    let served = Served::with_main(OPENS);
    let loads = || served.gate.loads.load(Ordering::SeqCst);
    // the watcher's reload, then one that must load, as a sync point: the listener runs them in turn
    let watched_then_forced = || async {
        served.reload_sender.reload_if_files_changed();
        served.reload_sender.reload_and_wait().await
    };
    let created = served.create(None, coffee("Cafe", "5")).await;
    assert_eq!(created.status, StatusCode::OK, "{}", created.body);
    let (before, generation) = (loads(), served.reload_sender.generation());

    watched_then_forced().await.unwrap();
    assert_eq!(loads(), before + 1, "the watcher's reload after the write loads nothing");
    assert_eq!(served.reload_sender.generation(), generation);

    // the same bytes written again: nothing to load
    let main = served.read("main.zhang");
    served.scratch.write("main.zhang", &main);
    watched_then_forced().await.unwrap();
    assert_eq!(loads(), before + 2);

    // an edit outside
    served.scratch.write("main.zhang", &format!("{main}1970-01-01 open Expenses:Tea\n"));
    served.reload_sender.reload_if_files_changed();
    served.reload_sender.reload_and_wait().await.unwrap();
    assert_eq!(loads(), before + 4, "the watcher's reload after an edit outside loads");
    assert!(served.accounts().await.contains(&"Expenses:Tea".to_owned()));

    // a reload that fails, then the files as the ledger served was loaded from: the failure is cleared by a load
    served.gate.failing.store(true, Ordering::SeqCst);
    served.reload_sender.reload_and_wait().await.expect_err("the load fails");
    served.gate.failing.store(false, Ordering::SeqCst);
    let edited = served.read("main.zhang");
    served.scratch.write("main.zhang", &edited);
    served.reload_sender.reload_if_files_changed();
    served.reload_sender.reload_and_wait().await.unwrap();
    assert_eq!(loads(), before + 7, "the watcher's reload loads while the last reload failed");
    assert!(served.reload_sender.last_failure().is_none());
}

/// An update whose client goes away while the update reloads: the version it replaced is a tombstone already,
/// recorded with the write, so once an edit outside restores the transaction as it was, the update sent again with that
/// version is refused (#818) rather than applied a second time
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_update_whose_client_goes_away_while_it_reloads_keeps_its_tombstone() {
    let coffee_text = "\n2024-01-02 * \"Cafe\" \"coffee\"\n  Assets:Cash -5 CNY\n  Expenses:Food\n";
    let served = Served::with_main(&format!("{OPENS}{coffee_text}"));
    let original = served.read("main.zhang");
    let coffee = served.transactions().await.pop().expect("a transaction");
    let (id, version) = (coffee["id"].as_str().unwrap().to_owned(), coffee["version"].as_str().unwrap().to_owned());
    let tea = || {
        json!({
            "datetime": "2024-01-02T12:00:00", "payee": "Cafe", "narration": "tea",
            "postings": [{"account": "Assets:Cash", "unit": "-4 CNY"}, {"account": "Expenses:Food", "unit": null}],
            "metas": [], "tags": [], "links": [],
        })
    };
    served.gate.close();
    let update = {
        let (served, id, version, tea) = (served.clone(), id.clone(), version.clone(), tea());
        tokio::spawn(async move { served.update(&id, &version, tea).await })
    };
    served.gate.a_load_waits().await;
    // the client goes away: its request is dropped during the reload
    update.abort();
    assert!(update.await.unwrap_err().is_cancelled());
    assert!(served.read("main.zhang").contains("\"tea\""), "the update is in the files");
    served.gate.open();
    let uuid = uuid::Uuid::parse_str(&id).unwrap();
    assert!(
        served.reload_sender.tombstones().contains(uuid, &version),
        "the version replaced is a tombstone"
    );

    // an edit outside undoes the update: the coffee is back, at its place, with its id and its version
    served.scratch.write("main.zhang", &original);
    let (status, body) = served.update(&id, &version, tea()).await;
    assert_eq!(status, StatusCode::PRECONDITION_FAILED, "{body}");
    assert_eq!(served.read("main.zhang"), original, "nothing is written");
}

/// The file watcher reports that it lost events (its queue overflowed: a rescan), or an error: any change may be
/// hidden, so the ledger is reloaded, though no event names a file of it
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_watcher_that_lost_events_reloads() {
    let served = Served::with_main(OPENS);
    let roots = watch::watch_roots(served.scratch.dir());
    let loads = || served.gate.loads.load(Ordering::SeqCst);
    let rescan = || Ok(notify::Event::new(notify::EventKind::Other).set_flag(notify::event::Flag::Rescan));
    let failed = || Err(notify::Error::generic("the watcher failed"));
    for (lost, account) in [(rescan(), "Expenses:Tea"), (failed(), "Expenses:Coffee")] {
        // an edit outside, which the watcher did not report
        let main = served.read("main.zhang");
        served.scratch.write("main.zhang", &format!("{main}1970-01-01 open {account}\n"));
        let before = loads();
        on_file_events(&served.ledger, &served.reload_sender, &roots, &[lost]).await;
        // a reload that loads, after the watcher's: the listener runs them in turn
        served.reload_sender.reload_and_wait().await.unwrap();
        assert_eq!(loads(), before + 2, "the watcher's reload loads, then the one after it");
        assert!(served.accounts().await.contains(&account.to_owned()), "the edit is served");
    }
    // nothing lost, nothing changed: no reload
    let before = loads();
    on_file_events(&served.ledger, &served.reload_sender, &roots, &[]).await;
    served.reload_sender.reload_and_wait().await.unwrap();
    assert_eq!(loads(), before + 1);
}

/// An SSE client that does not read and would miss a reload event is let go: its stream gives what its buffer holds,
/// then ends, so the client connects again and fetches anew
#[tokio::test]
async fn the_stream_of_an_sse_client_that_would_miss_a_reload_ends() {
    let broadcaster = Broadcaster::create();
    let stream = sse(State(SharedBroadcaster(broadcaster.clone()))).await.into_response();
    for _ in 0..10 {
        broadcaster.broadcast(BroadcastEvent::Reload).await;
    }
    assert_eq!(broadcaster.client_number().await, 0, "the client is let go");
    let body = tokio::time::timeout(Duration::from_secs(10), axum::body::to_bytes(stream.into_body(), usize::MAX))
        .await
        .expect("the stream ends")
        .expect("the stream");
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("\"type\":\"Connected\""), "{text}");
    // the buffer holds 10 events: `Connected`, a ping maybe, and the reloads
    let reloads = text.matches("\"type\":\"Reload\"").count();
    assert!((8..=9).contains(&reloads), "{text}");
}
