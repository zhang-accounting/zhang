use std::str::FromStr;

use axum::extract::{Multipart, Path, State};
use axum::Json;
use gotcha::api;
use indexmap::IndexSet;
use itertools::Itertools;
use log::info;
use uuid::Uuid;
use zhang_ast::error::ErrorKind;
use zhang_ast::{Date, Directive, Flag, Meta, Posting, SpanInfo, Transaction, ZhangString};
use zhang_core::constants::TXN_ID;
use zhang_core::domains::schemas::MetaType;
use zhang_core::ledger::Ledger;
use zhang_core::store::TransactionDomain;
use zhang_core::utils::string_::{quote_as, QuoteStyle, StringExt};

use super::Query;
use crate::request::{CreateTransactionRequest, JournalRequest};
use crate::response::{
    InfoForNewTransaction, JournalBalanceItemEntity, JournalItemEntity, JournalTransactionItemEntity, JournalTransactionPostingEntity, Pageable,
    ResponseWrapper,
};
use crate::state::{SharedLedger, SharedReloadSender};
use crate::{validate, ApiResult, ServerResult};

#[api(group = "transaction")]
// todo rename api
pub async fn get_info_for_new_transactions(ledger: State<SharedLedger>) -> ApiResult<InfoForNewTransaction> {
    let guard = ledger.read().await;
    let mut operations = guard.operations();

    let all_open_accounts = operations.all_open_accounts()?;
    let account_names = all_open_accounts.into_iter().map(|it| it.name).collect_vec();

    ResponseWrapper::json(InfoForNewTransaction {
        payee: operations.all_payees()?,
        account_name: account_names,
    })
}

#[api(group = "transaction")]
pub async fn get_journals(ledger: State<SharedLedger>, params: Query<JournalRequest>) -> ApiResult<Pageable<JournalItemEntity>> {
    let ledger = ledger.read().await;
    let mut operations = ledger.operations();
    let params = params.0;

    let store = operations.read();

    let total_count = store
        .transactions
        .values()
        .filter(|it| it.match_keywords(params.keyword.as_ref(), &params.tags, &params.links))
        .count();

    let journals: Vec<TransactionDomain> = store
        .transactions
        .values()
        .filter(|it| it.match_keywords(params.keyword.as_ref(), &params.tags, &params.links))
        .sorted_by_key(|it| -it.sequence)
        .skip(params.offset() as usize)
        .take(params.limit() as usize)
        .cloned()
        .collect_vec();

    drop(store);
    let mut ret = vec![];
    for journal_item in journals {
        let item = match journal_item.flag {
            Flag::BalancePad => {
                let postings = journal_item.postings.into_iter().map(JournalTransactionPostingEntity::from).collect_vec();
                JournalItemEntity::BalancePad(JournalBalanceItemEntity {
                    id: journal_item.id,
                    sequence: journal_item.sequence,
                    datetime: journal_item.datetime.naive_local(),
                    payee: journal_item.payee.unwrap_or_default(),
                    narration: journal_item.narration,
                    type_: journal_item.flag.to_string(),
                    postings,
                })
            }
            Flag::BalanceCheck => {
                let postings = journal_item.postings.into_iter().map(JournalTransactionPostingEntity::from).collect_vec();
                JournalItemEntity::BalanceCheck(JournalBalanceItemEntity {
                    id: journal_item.id,
                    sequence: journal_item.sequence,
                    datetime: journal_item.datetime.naive_local(),
                    payee: journal_item.payee.unwrap_or_default(),
                    narration: journal_item.narration,
                    type_: journal_item.flag.to_string(),
                    postings,
                })
            }
            _ => {
                let postings = journal_item.postings.into_iter().map(JournalTransactionPostingEntity::from).collect_vec();
                let metas = operations
                    .metas(MetaType::TransactionMeta, journal_item.id.to_string())
                    .unwrap()
                    .into_iter()
                    .map(|it| it.into())
                    .collect();
                let has_unbalanced_error = operations
                    .errors_by_meta(TXN_ID, &journal_item.id.to_string())?
                    .iter()
                    .any(|error| error.error_type == ErrorKind::UnbalancedTransaction);

                JournalItemEntity::Transaction(JournalTransactionItemEntity {
                    id: journal_item.id,
                    sequence: journal_item.sequence,
                    datetime: journal_item.datetime.naive_local(),
                    payee: journal_item.payee.unwrap_or_default(),
                    narration: journal_item.narration,
                    tags: journal_item.tags,
                    links: journal_item.links,
                    flag: journal_item.flag.to_string(),
                    is_balanced: !has_unbalanced_error,
                    postings,
                    metas,
                })
            }
        };
        ret.push(item);
    }
    ret.sort_by_key(|item| item.sequence());
    ret.reverse();
    ResponseWrapper::json(Pageable::new(total_count as u32, params.page(), params.limit(), ret))
}

/// Build the transaction a create or update request describes, rejecting with a
/// 400 any account, commodity, tag, link or flag that would be written unquoted and
/// not read back, and in a beancount ledger any new name beancount itself rejects.
fn transaction_from_request(payload: CreateTransactionRequest, ledger: &Ledger) -> ServerResult<Directive> {
    let rules = validate::Rules::of(ledger);
    let mut postings = vec![];
    for posting in payload.postings {
        if let Some(unit) = &posting.unit {
            validate::amount(unit, &rules)?;
        }
        postings.push(Posting {
            flag: None,
            account: validate::account(&posting.account, &rules)?,
            units: posting.unit,
            cost: None,
            price: None,
            comment: None,
            meta: Meta::default(),
        });
    }

    let mut metas = Meta::default();
    for meta in payload.metas {
        validate::meta_key(&meta.key, &rules)?;
        metas.insert(meta.key, meta.value.to_quote());
    }
    for tag in &payload.tags {
        validate::tag(tag, &rules)?;
    }
    for link in &payload.links {
        validate::link(link, &rules)?;
    }
    let flag = payload.flag.map(Flag::from).unwrap_or(Flag::Okay);
    validate::flag(&flag.to_string())?;

    let time = payload.datetime.with_timezone(&ledger.options.timezone).naive_local();
    Ok(Directive::Transaction(Transaction {
        date: Date::Datetime(time),
        flag: Some(flag),
        payee: Some(payload.payee.to_quote()),
        narration: payload.narration.map(|it| it.to_quote()),
        tags: IndexSet::from_iter(payload.tags),
        links: IndexSet::from_iter(payload.links),
        postings,
        meta: metas,
    }))
}

#[api(group = "transaction")]
pub async fn create_new_transaction(
    ledger: State<SharedLedger>, reload_sender: State<SharedReloadSender>, Json(payload): Json<CreateTransactionRequest>,
) -> ApiResult<String> {
    let ledger = ledger.read().await;

    let trx = transaction_from_request(payload, &ledger)?;

    ledger.data_source.async_append(&ledger, vec![trx]).await?;
    reload_sender.reload();
    ResponseWrapper::json("Ok".to_string())
}

// TODO: handle multipart/form-data
#[api(group = "transaction")]
// todo(refact): use exporter to update transaction
pub async fn upload_transaction_document(
    ledger: State<SharedLedger>, reload_sender: State<SharedReloadSender>, path: Path<(String,)>, mut multipart: Multipart,
) -> ApiResult<String> {
    let transaction_id = Uuid::from_str(&path.0 .0).expect("invalid txn id");
    let ledger = ledger.read().await;
    let mut operations = ledger.operations();
    let entry = &ledger.entry.0;
    let mut documents = vec![];

    let span_info = operations.transaction_span(&transaction_id)?;
    let Some(span_info) = span_info else {
        return ResponseWrapper::bad_request();
    };

    while let Some(field) = multipart.next_field().await.unwrap() {
        let _name = field.name().unwrap().to_string();
        let file_name = field.file_name().unwrap().to_string();
        let _content_type = field.content_type().unwrap().to_string();

        let v4 = Uuid::new_v4();
        let buf = entry.join("attachments").join(v4.to_string()).join(&file_name);
        let striped_buf = buf.strip_prefix(entry).unwrap();
        let striped_path_string = striped_buf.to_string_lossy().to_string();
        info!("uploading document `{}`(id={}) to transaction {}", file_name, v4, transaction_id);
        let content_buf = field.bytes().await.unwrap();

        ledger.data_source.async_save(&ledger, striped_path_string, &content_buf).await?;

        let path = match buf.strip_prefix(entry) {
            Ok(relative_path) => relative_path.to_str().unwrap(),
            Err(_) => buf.to_str().unwrap(),
        };

        documents.push(ZhangString::QuoteString(path.to_string()));
    }

    // the source file may be zhang or beancount text: the beancount quote style is
    // read back exactly by both parsers, and by Python beancount too
    let metas_content = documents
        .into_iter()
        .map(|document| format!("  document: {}", quote_as(document.as_str(), QuoteStyle::Beancount)))
        .join("\n");

    let source_file_path = span_info.source_file.to_string_lossy().to_string();
    let mut content = String::from_utf8(ledger.data_source.async_get(source_file_path.clone()).await?).unwrap();
    content.insert(span_info.span_end, '\n');
    content.insert_str(span_info.span_end + 1, &metas_content);
    ledger.data_source.async_save(&ledger, source_file_path, content.as_bytes()).await?;
    reload_sender.reload();
    ResponseWrapper::json("Ok".to_string())
}

#[api(group = "transaction")]
pub async fn update_single_transaction(
    ledger: State<SharedLedger>, reload_sender: State<SharedReloadSender>, path: Path<(String,)>, Json(payload): Json<CreateTransactionRequest>,
) -> ApiResult<()> {
    let Ok(transaction_id) = Uuid::from_str(&path.0 .0) else {
        return ResponseWrapper::bad_request();
    };
    let ledger = ledger.read().await;
    let mut operations = ledger.operations();

    let span_info = operations.transaction_span(&transaction_id)?;
    let Some(span_info) = span_info else {
        return ResponseWrapper::bad_request();
    };

    let trx = transaction_from_request(payload, &ledger)?;
    let txn_content = ledger.data_source.export(trx)?;
    let trx_content = String::from_utf8_lossy(&txn_content);
    let source_file_path = span_info.source_file.to_string_lossy().to_string();

    let mut content = String::from_utf8(ledger.data_source.async_get(source_file_path.clone()).await?).unwrap();
    content.replace_by_span(&SpanInfo::simple(span_info.span_start, span_info.span_end), &trx_content);

    ledger.data_source.async_save(&ledger, source_file_path, content.as_bytes()).await?;
    reload_sender.reload();
    ResponseWrapper::json(())
}

/// Transactions written through the API read back exactly (issue #442).
#[cfg(test)]
mod string_round_trip_test {
    use std::path::{Path as FsPath, PathBuf};
    use std::str::FromStr;
    use std::sync::Arc;

    use axum::extract::{Path, State};
    use axum::http::StatusCode;
    use axum::response::IntoResponse;
    use axum::Json;
    use bigdecimal::BigDecimal;
    use chrono::{TimeZone, Utc};
    use tokio::sync::{mpsc, RwLock};
    use uuid::Uuid;
    use zhang_ast::amount::Amount;
    use zhang_ast::{Directive, SpanInfo, Spanned, Transaction};
    use zhang_core::data_source::LocalFileSystemDataSource;
    use zhang_core::data_type::text::ZhangDataType;
    use zhang_core::data_type::DataType;
    use zhang_core::domains::schemas::MetaType;
    use zhang_core::ledger::Ledger;
    use zhang_core::store::TransactionDomain;

    use super::{create_new_transaction, update_single_transaction};
    use crate::request::{CreateTransactionPostingRequest, CreateTransactionRequest, FlagRequest, MetaRequest};
    use crate::state::{SharedLedger, SharedReloadSender};
    use crate::ReloadSender;

    /// A change to a create request.
    type Change = Box<dyn Fn(&mut CreateTransactionRequest)>;

    const PAYEE: &str = "Bob's \"café\" \\ `shop`";

    fn request(narration: &str, note: &str) -> CreateTransactionRequest {
        CreateTransactionRequest {
            datetime: Utc.with_ymd_and_hms(2024, 1, 15, 12, 0, 0).unwrap(),
            payee: PAYEE.to_owned(),
            flag: None,
            narration: Some(narration.to_owned()),
            postings: vec![
                CreateTransactionPostingRequest {
                    account: "Assets:Cash".to_owned(),
                    unit: Some(Amount::new(BigDecimal::from_str("-5").unwrap(), "CNY")),
                },
                CreateTransactionPostingRequest {
                    account: "Expenses:Food".to_owned(),
                    unit: Some(Amount::new(BigDecimal::from_str("5").unwrap(), "CNY")),
                },
            ],
            metas: vec![MetaRequest {
                key: "note".to_owned(),
                value: note.to_owned(),
            }],
            tags: vec![],
            links: vec![],
        }
    }

    async fn load(dir: &FsPath) -> Ledger {
        let source = Arc::new(LocalFileSystemDataSource::new(ZhangDataType {}));
        Ledger::async_load(dir.to_path_buf(), "main.zhang".to_owned(), source)
            .await
            .expect("load ledger")
    }

    fn states(ledger: Ledger) -> (State<SharedLedger>, State<SharedReloadSender>) {
        let (sender, _) = mpsc::channel(1);
        (
            State(SharedLedger(Arc::new(RwLock::new(ledger)))),
            State(SharedReloadSender(Arc::new(ReloadSender(sender)))),
        )
    }

    /// The only transaction of the ledger, with its `note` metadata.
    fn transaction(ledger: &Ledger) -> (TransactionDomain, String) {
        let operations = ledger.operations();
        assert!(operations.read().errors.is_empty(), "ledger errors: {:?}", operations.read().errors);
        let transactions = operations.read().transactions.values().cloned().collect::<Vec<_>>();
        assert_eq!(transactions.len(), 1);
        let transaction = transactions.into_iter().next().unwrap();
        let note = operations
            .metas(MetaType::TransactionMeta, transaction.id.to_string())
            .unwrap()
            .into_iter()
            .find(|meta| meta.key == "note")
            .expect("note meta")
            .value;
        (transaction, note)
    }

    /// The written transaction parses back and exports to exactly the written text.
    fn assert_written_text_round_trips(file: &FsPath) -> String {
        let written = std::fs::read_to_string(file).unwrap();
        let data_type = ZhangDataType {};
        let directives = data_type.transform(written.clone(), None).expect("written file parses");
        let transactions: Vec<Transaction> = directives
            .into_iter()
            .filter_map(|it| match it.data {
                Directive::Transaction(transaction) => Some(transaction),
                _ => None,
            })
            .collect();
        assert_eq!(transactions.len(), 1, "{written}");
        let exported = data_type.export(Spanned::new(Directive::Transaction(transactions[0].clone()), SpanInfo::default()));
        assert!(
            written.contains(&exported),
            "re-export differs from the written text:\n{written}\n---\n{exported}"
        );
        written
    }

    /// A ledger directory whose January 2024 data file, where the API appends the
    /// test transactions, exists already: the local file system data source appends
    /// to existing files only. Returns the directory and the data file.
    fn ledger_dir() -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!("zhang-transaction-strings-{}", Uuid::new_v4()));
        std::fs::create_dir_all(dir.join("data/2024")).unwrap();
        let dir = dir.canonicalize().unwrap();
        std::fs::write(
            dir.join("main.zhang"),
            "option \"operating_currency\" \"CNY\"\ninclude \"data/2024/1.zhang\"\n\
             1970-01-01 commodity CNY\n1970-01-01 open Assets:Cash\n1970-01-01 open Expenses:Food\n",
        )
        .unwrap();
        let data_file = dir.join("data/2024/1.zhang");
        std::fs::write(&data_file, "").unwrap();
        (dir, data_file)
    }

    #[tokio::test]
    async fn created_and_updated_transactions_read_back_exactly() {
        let (dir, data_file) = ledger_dir();

        // create
        let (ledger, reload) = states(load(&dir).await);
        let note = "a\u{a0}b\u{2028}c 😀 你好";
        let response = create_new_transaction(ledger, reload, Json(request("coffee $5", note))).await.into_response();
        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));

        let written = assert_written_text_round_trips(&data_file);
        assert!(written.contains("\"coffee $5\""), "`$` is written raw:\n{written}");
        let reloaded = load(&dir).await;
        let (created, created_note) = transaction(&reloaded);
        assert_eq!(created.payee.as_deref(), Some(PAYEE));
        assert_eq!(created.narration.as_deref(), Some("coffee $5"));
        assert_eq!(created_note, note);

        // update in place
        let narration = "tea $3 ~ '\\d+' \"x\"\nsecond line";
        let (ledger, reload) = states(reloaded);
        let response = update_single_transaction(ledger, reload, Path((created.id.to_string(),)), Json(request(narration, "`$`")))
            .await
            .into_response();
        assert_eq!(response.status(), StatusCode::OK);

        assert_written_text_round_trips(&data_file);
        let (updated, updated_note) = transaction(&load(&dir).await);
        assert_eq!(updated.payee.as_deref(), Some(PAYEE));
        assert_eq!(updated.narration.as_deref(), Some(narration));
        assert_eq!(updated_note, "`$`");

        std::fs::remove_dir_all(dir).ok();
    }

    #[tokio::test]
    async fn a_transaction_without_narration_keeps_its_payee() {
        let (dir, data_file) = ledger_dir();
        let (ledger, reload) = states(load(&dir).await);
        let request = CreateTransactionRequest {
            narration: None,
            ..request("unused", "note")
        };
        let response = create_new_transaction(ledger, reload, Json(request)).await.into_response();
        assert_eq!(response.status(), StatusCode::OK);

        assert_written_text_round_trips(&data_file);
        let (created, _) = transaction(&load(&dir).await);
        assert_eq!(created.payee.as_deref(), Some(PAYEE));
        assert_eq!(created.narration.as_deref(), Some(""));

        std::fs::remove_dir_all(dir).ok();
    }

    #[tokio::test]
    async fn names_written_unquoted_read_back() {
        let (dir, data_file) = ledger_dir();
        let (ledger, reload) = states(load(&dir).await);
        let mut request = request("coffee", "note");
        request.flag = Some(FlagRequest::Warning);
        request.tags = vec!["trip-2024".to_owned(), "旅行".to_owned(), "a#b".to_owned()];
        request.links = vec!["inv-1".to_owned()];
        request.metas.push(MetaRequest {
            key: "receipt-no".to_owned(),
            value: "1".to_owned(),
        });
        // a zhang ledger takes any metadata key, quoting one that is not a bare word
        request.metas.push(MetaRequest {
            key: "receipt no".to_owned(),
            value: "2".to_owned(),
        });
        let response = create_new_transaction(ledger, reload, Json(request)).await.into_response();
        assert_eq!(response.status(), StatusCode::OK);

        assert_written_text_round_trips(&data_file);
        let reloaded = load(&dir).await;
        let (created, _) = transaction(&reloaded);
        assert_eq!(created.flag.to_string(), "!");
        assert_eq!(created.tags, vec!["trip-2024", "旅行", "a#b"]);
        assert_eq!(created.links, vec!["inv-1"]);
        let receipt = reloaded.operations().metas(MetaType::TransactionMeta, created.id.to_string()).unwrap();
        assert!(receipt.iter().any(|meta| meta.key == "receipt-no" && meta.value == "1"), "{receipt:?}");
        assert!(receipt.iter().any(|meta| meta.key == "receipt no" && meta.value == "2"), "{receipt:?}");

        std::fs::remove_dir_all(dir).ok();
    }

    #[tokio::test]
    async fn names_that_would_not_read_back_are_rejected() {
        let (dir, data_file) = ledger_dir();
        let cases: Vec<(&str, Change)> = vec![
            ("tag \"two words\"", Box::new(|it| it.tags.push("two words".to_owned()))),
            ("tag \"\"", Box::new(|it| it.tags.push(String::new()))),
            ("link \"a:b\"", Box::new(|it| it.links.push("a:b".to_owned()))),
            (
                "account \"Assets:My Bank\"",
                Box::new(|it| it.postings[0].account = "Assets:My Bank".to_owned()),
            ),
            ("account \"Assets\"", Box::new(|it| it.postings[0].account = "Assets".to_owned())),
            (
                "commodity \"US D\"",
                Box::new(|it| it.postings[0].unit = Some(Amount::new(BigDecimal::from(1), "US D"))),
            ),
            ("flag \"a\"", Box::new(|it| it.flag = Some(FlagRequest::Custom('a')))),
        ];
        for (rejected, change) in cases {
            let mut request = request("coffee", "note");
            change(&mut request);
            let (ledger, reload) = states(load(&dir).await);
            let response = create_new_transaction(ledger, reload, Json(request)).await.into_response();
            assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{rejected}");
            let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
            let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
            let message = body["message"].as_str().unwrap();
            assert!(message.starts_with(&format!("invalid {rejected}: ")), "{message}");
            assert_eq!(std::fs::read_to_string(&data_file).unwrap(), "", "nothing is written for {rejected}");
        }

        // updates are checked the same way
        let (ledger, reload) = states(load(&dir).await);
        let response = create_new_transaction(ledger, reload, Json(request("coffee", "note"))).await.into_response();
        assert_eq!(response.status(), StatusCode::OK);
        let written = std::fs::read_to_string(&data_file).unwrap();
        let reloaded = load(&dir).await;
        let (created, _) = transaction(&reloaded);
        let mut bad = request("coffee", "note");
        bad.tags.push("two words".to_owned());
        let (ledger, reload) = states(reloaded);
        let response = update_single_transaction(ledger, reload, Path((created.id.to_string(),)), Json(bad))
            .await
            .into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(std::fs::read_to_string(&data_file).unwrap(), written);

        std::fs::remove_dir_all(dir).ok();
    }

    /// Names zhang's parsers read but beancount 3.2.3 rejects (checked against it),
    /// each set on a create request, with the kind of name.
    fn names_only_zhang_reads() -> Vec<(&'static str, &'static str, Change)> {
        vec![
            (
                "metadata key",
                "receipt no",
                Box::new(|it| {
                    it.metas.push(MetaRequest {
                        key: "receipt no".to_owned(),
                        value: "1".to_owned(),
                    })
                }),
            ),
            (
                "metadata key",
                "Receipt",
                Box::new(|it| {
                    it.metas.push(MetaRequest {
                        key: "Receipt".to_owned(),
                        value: "1".to_owned(),
                    })
                }),
            ),
            ("tag", "旅行", Box::new(|it| it.tags.push("旅行".to_owned()))),
            ("link", "a+b", Box::new(|it| it.links.push("a+b".to_owned()))),
            ("account", "Assets:银行", Box::new(|it| it.postings[0].account = "Assets:银行".to_owned())),
            (
                "commodity",
                "usd",
                Box::new(|it| it.postings[0].unit = Some(Amount::new(BigDecimal::from(1), "usd"))),
            ),
        ]
    }

    #[tokio::test]
    async fn a_beancount_ledger_rejects_names_beancount_cannot_read() {
        // the ledger format comes from the main file's extension
        let dir = std::env::temp_dir().join(format!("zhang-beancount-names-{}", Uuid::new_v4()));
        std::fs::create_dir_all(dir.join("data/2024")).unwrap();
        let dir = dir.canonicalize().unwrap();
        let main = "include \"data/2024/1.bean\"\n1970-01-01 commodity CNY\n1970-01-01 open Assets:Cash\n1970-01-01 open Expenses:Food\n";
        std::fs::write(dir.join("main.bean"), main).unwrap();
        let data_file = dir.join("data/2024/1.bean");
        std::fs::write(&data_file, "").unwrap();
        let load = || async {
            let source = Arc::new(LocalFileSystemDataSource::new(beancount::Beancount {}));
            Ledger::async_load(dir.clone(), "main.bean".to_owned(), source).await.expect("load ledger")
        };

        let mut cases = names_only_zhang_reads();
        cases.push((
            "metadata key",
            ";path",
            Box::new(|it| {
                it.metas.push(MetaRequest {
                    key: ";path".to_owned(),
                    value: "1".to_owned(),
                })
            }),
        ));
        for (kind, name, change) in cases {
            let mut create = request("coffee", "note");
            change(&mut create);
            let (ledger, reload) = states(load().await);
            let response = create_new_transaction(ledger, reload, Json(create)).await.into_response();
            assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{name:?}");
            let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
            let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
            let message = body["message"].as_str().unwrap();
            assert!(message.starts_with(&format!("invalid {kind} {name:?}: beancount ")), "{message}");
            assert_eq!(std::fs::read_to_string(&data_file).unwrap(), "", "nothing is written for {name:?}");
            assert_eq!(std::fs::read_to_string(dir.join("main.bean")).unwrap(), main);
        }

        std::fs::remove_dir_all(dir).ok();
    }

    #[tokio::test]
    async fn a_beancount_ledger_takes_names_it_already_has() {
        // a zhang user with a `.bean` file and Chinese names keeps writing them; only
        // new names must be ones beancount accepts
        let dir = std::env::temp_dir().join(format!("zhang-beancount-known-names-{}", Uuid::new_v4()));
        std::fs::create_dir_all(dir.join("data/2024")).unwrap();
        let dir = dir.canonicalize().unwrap();
        std::fs::write(
            dir.join("main.bean"),
            "include \"data/2024/1.zhang\"\n1970-01-01 commodity CNY\n1970-01-01 open Assets:Cash\n\
             1970-01-01 open Expenses:Food\n1970-01-01 open Assets:银行\n\
             2023-06-01 * \"Shop\" \"trip\" #旅行\n  Receipt: \"1\"\n  Assets:Cash -1 CNY\n  Expenses:Food 1 CNY\n",
        )
        .unwrap();
        // the local file system data source appends to existing `.zhang` files only
        std::fs::write(dir.join("data/2024/1.zhang"), "").unwrap();
        let load = || async {
            let source = Arc::new(LocalFileSystemDataSource::new(beancount::Beancount {}));
            Ledger::async_load(dir.clone(), "main.bean".to_owned(), source).await.expect("load ledger")
        };

        let cases: Vec<(&str, StatusCode, Change)> = vec![
            (
                "opened account",
                StatusCode::OK,
                Box::new(|it| it.postings[0].account = "Assets:银行".to_owned()),
            ),
            ("used tag", StatusCode::OK, Box::new(|it| it.tags.push("旅行".to_owned()))),
            (
                "used metadata key",
                StatusCode::OK,
                Box::new(|it| {
                    it.metas.push(MetaRequest {
                        key: "Receipt".to_owned(),
                        value: "2".to_owned(),
                    })
                }),
            ),
            ("new tag", StatusCode::BAD_REQUEST, Box::new(|it| it.tags.push("出差".to_owned()))),
            (
                "new account",
                StatusCode::BAD_REQUEST,
                Box::new(|it| it.postings[0].account = "Assets:现金".to_owned()),
            ),
            (
                "new metadata key",
                StatusCode::BAD_REQUEST,
                Box::new(|it| {
                    it.metas.push(MetaRequest {
                        key: "Memo".to_owned(),
                        value: "2".to_owned(),
                    })
                }),
            ),
        ];
        for (case, status, change) in cases {
            let mut create = request("coffee", "note");
            change(&mut create);
            let (ledger, reload) = states(load().await);
            let response = create_new_transaction(ledger, reload, Json(create)).await.into_response();
            assert_eq!(response.status(), status, "{case}");
        }
        let written = std::fs::read_to_string(dir.join("data/2024/1.zhang")).unwrap();
        assert!(
            written.contains("Assets:银行") && written.contains("#旅行") && written.contains("Receipt: \"2\""),
            "{written}"
        );
        assert!(!written.contains("出差") && !written.contains("现金") && !written.contains("Memo"), "{written}");

        std::fs::remove_dir_all(dir).ok();
    }

    #[tokio::test]
    async fn a_zhang_ledger_takes_names_beancount_cannot_read() {
        for (_, name, change) in names_only_zhang_reads() {
            let (dir, data_file) = ledger_dir();
            let mut create = request("coffee", "note");
            change(&mut create);
            let (ledger, reload) = states(load(&dir).await);
            let response = create_new_transaction(ledger, reload, Json(create)).await.into_response();
            assert_eq!(response.status(), StatusCode::OK, "{name:?}");
            let written = assert_written_text_round_trips(&data_file);
            assert!(written.contains(name), "{name:?}: {written}");
            std::fs::remove_dir_all(dir).ok();
        }
    }

    #[tokio::test]
    async fn quoted_metadata_keys_survive_an_update() {
        let (dir, data_file) = ledger_dir();
        std::fs::write(
            &data_file,
            "2024-01-15 * \"Bob\" \"coffee\"\n  note: \"n\"\n  \"my key\": \"v\"\n  \";path\": \"b\"\n  Assets:Cash -5 CNY\n  Expenses:Food 5 CNY\n",
        )
        .unwrap();
        let reloaded = load(&dir).await;
        let (created, _) = transaction(&reloaded);
        let keys = |ledger: &Ledger, id: String| {
            let mut metas = ledger
                .operations()
                .metas(MetaType::TransactionMeta, id)
                .unwrap()
                .into_iter()
                .map(|meta| (meta.key, meta.value))
                .collect::<Vec<_>>();
            metas.sort();
            metas
        };
        let pairs = |pairs: &[(&str, &str)]| pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect::<Vec<_>>();
        assert_eq!(
            keys(&reloaded, created.id.to_string()),
            pairs(&[(";path", "b"), ("my key", "v"), ("note", "n")])
        );

        // the client sends the metadata back as it got it, with one more key
        let mut update = request("coffee", "n");
        for (key, value) in [("my key", "v"), (";path", "b"), ("receipt no", "1")] {
            update.metas.push(MetaRequest {
                key: key.to_owned(),
                value: value.to_owned(),
            });
        }
        let (ledger, reload) = states(reloaded);
        let response = update_single_transaction(ledger, reload, Path((created.id.to_string(),)), Json(update))
            .await
            .into_response();
        assert_eq!(response.status(), StatusCode::OK);

        let written = assert_written_text_round_trips(&data_file);
        assert!(written.contains("  \";path\": \"b\"\n  \"my key\": \"v\"\n"), "{written}");
        let reloaded = load(&dir).await;
        let (updated, _) = transaction(&reloaded);
        assert_eq!(
            keys(&reloaded, updated.id.to_string()),
            pairs(&[(";path", "b"), ("my key", "v"), ("note", "n"), ("receipt no", "1")])
        );

        std::fs::remove_dir_all(dir).ok();
    }
}
