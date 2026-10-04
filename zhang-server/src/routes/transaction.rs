use std::collections::HashMap;
use std::str::FromStr;

use axum::extract::{Multipart, Path, State};
use axum::Json;
use gotcha::api;
use indexmap::IndexSet;
use itertools::Itertools;
use log::info;
use uuid::Uuid;
use zhang_ast::{Date, Directive, Flag, Meta, Posting, SpanInfo, Transaction, ZhangString};
use zhang_core::data_type::text::parser::{is_valid_bare_meta_value, transaction_header_len};
use zhang_core::domains::schemas::TransactionInfoDomain;
use zhang_core::ledger::Ledger;
use zhang_core::utils::string_::{quote_as, QuoteStyle, StringExt};
use zhang_core::ZhangError;

use super::Query;
use crate::error::ServerError;
use crate::request::{CreateTransactionPostingRequest, CreateTransactionRequest, JournalRequest, MetaRequest};
use crate::response::{InfoForNewTransaction, JournalItemEntity, Pageable, ResponseWrapper};
use crate::state::{wrote, SharedLedger, SharedReloadSender};
use crate::{journals, validate, ApiResult, ServerResult};

/// The payees and the open accounts the new-transaction form suggests: the built-in queries
/// `journals.payees` and `journals.accounts`.
#[api(group = "transaction")]
// todo rename api
pub async fn get_info_for_new_transactions(ledger: State<SharedLedger>) -> ApiResult<InfoForNewTransaction> {
    ResponseWrapper::json(journals::info_for_new_transaction(&ledger).await?)
}

/// The journal: the transactions and the balance assertions, newest first. An assertion is listed in its place
/// among the transactions; it books nothing. The built-in query `journals.page`, with the postings and the checks
/// of a page from `journals.postings` and `journals.balance_checks`.
///
/// A page has 1 to 1000 rows (`size`, 100 by default); another size is a bad request, and a page past the last one is
/// empty.
#[api(group = "transaction")]
pub async fn get_journals(ledger: State<SharedLedger>, params: Query<JournalRequest>) -> ApiResult<Pageable<JournalItemEntity>> {
    ResponseWrapper::json(journals::journal(&ledger, params.0).await?)
}

/// Build the transaction a create or update request describes, rejecting with a
/// 400 any account, commodity, tag, link or flag that would be written unquoted and
/// not read back, and in a beancount ledger any new name beancount itself rejects.
/// `original` is the transaction an update replaces, as it was read from the ledger.
fn transaction_from_request(payload: CreateTransactionRequest, ledger: &Ledger, original: Option<&Transaction>) -> ServerResult<Directive> {
    let rules = validate::Rules::of(ledger);
    let original_postings = payload
        .postings
        .iter()
        .map(|posting| original.and_then(|original| original_posting(original, &payload.postings, posting)))
        .collect_vec();
    let mut postings = vec![];
    for (posting, original_posting) in payload.postings.into_iter().zip(original_postings) {
        let original_meta = original_posting.map(|it| &it.meta);
        if let Some(unit) = &posting.unit {
            validate::amount(unit, &rules)?;
        }
        postings.push(Posting {
            // a request carries no posting flag: the posting it edits keeps its own, such as `!`. The
            // exporter leaves out one the ledger's format would not read back, such as a `*` a plugin
            // set in a zhang ledger, where the line would read back as a comment
            flag: original_posting.and_then(|it| it.flag.clone()),
            account: validate::account(&posting.account, &rules)?,
            units: posting.unit,
            cost: None,
            price: None,
            comment: None,
            meta: metas_from_request(posting.metas.unwrap_or_default(), &rules, original_meta)?,
            written: None,
        });
    }

    let metas = metas_from_request(payload.metas, &rules, original.map(|it| &it.meta))?;
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
        payee: Some(ZhangString::quote(payload.payee)),
        narration: payload.narration.map(ZhangString::quote),
        tags: IndexSet::from_iter(payload.tags),
        links: IndexSet::from_iter(payload.links),
        postings,
        meta: metas,
    }))
}

/// The metadata of a request, every key checked by [`validate::meta_key`].
///
/// New and changed values are written quoted. A value `original`, the metadata the
/// request edits, already has unquoted under the same key is written unquoted again, so
/// an edit that leaves a number, date or boolean as it was does not turn it into a string
/// for beancount. Repeated pairs are matched in order: the n-th `key: value` of the request
/// takes the form of the n-th `key: value` of `original`, and one beyond those is new.
/// A value is only written unquoted when the parser reads it back as the same bare value:
/// `original` comes after the plugins, and a plugin can make an unquoted value of any
/// text, such as `from plugin`.
fn metas_from_request(metas: Vec<MetaRequest>, rules: &validate::Rules, original: Option<&Meta>) -> ServerResult<Meta> {
    let mut meta = Meta::default();
    let mut occurrences: HashMap<(String, String), usize> = HashMap::new();
    for MetaRequest { key, value } in metas {
        validate::meta_key(&key, rules)?;
        let occurrence = occurrences.entry((key.clone(), value.clone())).or_default();
        let original_form = original.and_then(|original| original.get_all(&key).into_iter().filter(|it| it.as_str() == value).nth(*occurrence));
        *occurrence += 1;
        let unchanged_bare = is_valid_bare_meta_value(&value) && matches!(original_form, Some(ZhangString::UnquoteString(_)));
        let value = if unchanged_bare {
            ZhangString::UnquoteString(value)
        } else {
            ZhangString::quote(value)
        };
        meta.insert(key, value);
    }
    Ok(meta)
}

/// The posting of `original` that the request posting `posting`, one of `requested`, edits:
/// the only posting to its account, or else the only one to its account with its units,
/// where `posting` is likewise the only one of `requested` (a split or repeated posting
/// matches nothing). `None` when there is no such single pair, so that every value of the
/// request posting counts as new.
fn original_posting<'a>(
    original: &'a Transaction, requested: &[CreateTransactionPostingRequest], posting: &CreateTransactionPostingRequest,
) -> Option<&'a Posting> {
    let same_account = original.postings.iter().filter(|it| it.account.name() == posting.account).collect_vec();
    let requested_same_account = requested.iter().filter(|it| it.account == posting.account).count();
    if let ([candidate], 1) = (same_account.as_slice(), requested_same_account) {
        return Some(candidate);
    }
    let same_units = same_account.into_iter().filter(|it| it.units == posting.unit).collect_vec();
    let requested_same_units = requested.iter().filter(|it| it.account == posting.account && it.unit == posting.unit).count();
    match (same_units.as_slice(), requested_same_units) {
        ([candidate], 1) => Some(candidate),
        _ => None,
    }
}

/// The transaction directive the stored transaction at `span` was read from.
fn original_transaction<'a>(ledger: &'a Ledger, span: &TransactionInfoDomain) -> Option<&'a Transaction> {
    ledger.directives.iter().find_map(|directive| match &directive.data {
        Directive::Transaction(transaction)
            if directive.span.start == span.span_start && directive.span.filename.as_deref() == Some(span.source_file.as_path()) =>
        {
            Some(transaction)
        }
        _ => None,
    })
}

#[api(group = "transaction")]
pub async fn create_new_transaction(
    ledger: State<SharedLedger>, reload_sender: State<SharedReloadSender>, Json(payload): Json<CreateTransactionRequest>,
) -> ApiResult<String> {
    let mut ledger = ledger.for_writing().await?;

    let trx = transaction_from_request(payload, &ledger, None)?;

    let appended = ledger.data_source.async_append(&ledger, vec![trx]).await;
    wrote(&mut ledger, &reload_sender, appended.map_err(ServerError::from))?;
    ResponseWrapper::json("Ok".to_string())
}

// TODO: handle multipart/form-data
#[api(group = "transaction")]
// todo(refact): use exporter to update transaction
pub async fn upload_transaction_document(
    ledger: State<SharedLedger>, reload_sender: State<SharedReloadSender>, path: Path<(String,)>, mut multipart: Multipart,
) -> ApiResult<String> {
    let Ok(transaction_id) = Uuid::from_str(&path.0 .0) else {
        return ResponseWrapper::bad_request();
    };
    // the files first, then the ledger, held to write
    let files = super::uploaded_files(&mut multipart).await?;
    let mut ledger = ledger.for_writing().await?;
    let mut operations = ledger.operations();
    let mut documents = vec![];

    let span_info = operations.transaction_span(&transaction_id)?;
    let Some(span_info) = span_info else {
        return Err(ServerError::NoSuchTransaction(transaction_id));
    };

    let written = async {
        // no file is saved for a transaction that is no longer where the ledger loaded it
        let source_file_path = span_info.source_file.to_string_lossy().to_string();
        ledger
            .data_source
            .async_get_unchanged(source_file_path, std::slice::from_ref(&span_info.span))
            .await?;
        for (file_name, content_buf) in files {
            let (v4, path) = super::attachment_path(&file_name);
            info!("uploading document `{}`(id={}) to transaction {}", file_name, v4, transaction_id);

            ledger.data_source.async_save(&ledger, path.clone(), &content_buf).await?;

            documents.push(path);
        }
        write_transaction_documents(&ledger, &span_info, &documents).await
    }
    .await;
    wrote(&mut ledger, &reload_sender, written.map_err(moved))?;
    ResponseWrapper::json("Ok".to_string())
}

/// `error`, for a write to a transaction: a file changed since the ledger was loaded may have moved the transaction,
/// which then has another id, as ids are derived from places
fn moved(error: ServerError) -> ServerError {
    match error {
        ServerError::CoreError(ZhangError::FileChanged(file)) => ServerError::Conflict(format!(
            "the file {file} changed since the ledger was loaded, so nothing was written. The transaction may be at \
             another place in it now, under another id: reopen the journal, and try again"
        )),
        other => other,
    }
}

/// Write `documents` into the ledger as `document` metadata of the transaction at `span`.
async fn write_transaction_documents(ledger: &Ledger, span: &TransactionInfoDomain, documents: &[String]) -> ServerResult<()> {
    // the source file may be zhang or beancount text: the beancount quote style is
    // read back exactly by both parsers, and by Python beancount too
    let lines = documents
        .iter()
        .map(|document| format!("  document: {}", quote_as(document, QuoteStyle::Beancount)))
        .collect_vec();
    let source_file_path = span.source_file.to_string_lossy().to_string();
    // the transaction must still be where the ledger loaded it
    let mut content = ledger
        .data_source
        .async_get_unchanged(source_file_path.clone(), std::slice::from_ref(&span.span))
        .await?;
    insert_transaction_metas(&mut content, span.span_start, span.span_end, &lines);
    ledger.data_source.async_save(ledger, source_file_path, content.as_bytes()).await?;
    Ok(())
}

/// Insert the metadata `lines` of the transaction at `span_start..span_end` of `content`
/// right under its header line. There they are the transaction's in zhang and in
/// beancount alike: after the postings beancount would read them as the last posting's.
/// The header line ends at the first line ending outside its quoted strings, since a
/// payee or narration may span several lines.
fn insert_transaction_metas(content: &mut String, span_start: usize, span_end: usize, lines: &[String]) {
    if lines.is_empty() {
        return;
    }
    let text = lines.iter().map(|line| format!("{line}\n")).join("");
    let span = &content[span_start..span_end];
    match transaction_header_len(span) {
        Some(header_len) => {
            let line_ending = if span[header_len..].starts_with("\r\n") { 2 } else { 1 };
            content.insert_str(span_start + header_len + line_ending, &text)
        }
        // a one-line directive, such as the `balance` of a balance check: write after it
        None => content.insert_str(span_end, &format!("\n{}", text.trim_end_matches('\n'))),
    }
}

#[api(group = "transaction")]
pub async fn update_single_transaction(
    ledger: State<SharedLedger>, reload_sender: State<SharedReloadSender>, path: Path<(String,)>, Json(payload): Json<CreateTransactionRequest>,
) -> ApiResult<()> {
    let Ok(transaction_id) = Uuid::from_str(&path.0 .0) else {
        return ResponseWrapper::bad_request();
    };
    let mut ledger = ledger.for_writing().await?;
    let mut operations = ledger.operations();

    let span_info = operations.transaction_span(&transaction_id)?;
    let Some(span_info) = span_info else {
        return Err(ServerError::NoSuchTransaction(transaction_id));
    };

    let trx = transaction_from_request(payload, &ledger, original_transaction(&ledger, &span_info))?;
    let txn_content = ledger.data_source.export(trx)?;
    let trx_content = String::from_utf8_lossy(&txn_content);
    let source_file_path = span_info.source_file.to_string_lossy().to_string();

    let written = async {
        // the transaction must still be where the ledger loaded it
        let mut content = ledger
            .data_source
            .async_get_unchanged(source_file_path.clone(), std::slice::from_ref(&span_info.span))
            .await?;
        content.replace_by_span(&SpanInfo::simple(span_info.span_start, span_info.span_end), &trx_content);
        ledger.data_source.async_save(&ledger, source_file_path, content.as_bytes()).await?;
        ServerResult::Ok(())
    }
    .await;
    wrote(&mut ledger, &reload_sender, written.map_err(moved))?;
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
    use zhang_ast::{Directive, Flag, SpanInfo, Spanned, Transaction};
    use zhang_core::data_source::LocalFileSystemDataSource;
    use zhang_core::data_type::text::ZhangDataType;
    use zhang_core::data_type::DataType;
    use zhang_core::domains::schemas::MetaType;
    use zhang_core::ledger::Ledger;
    use zhang_core::store::TransactionDomain;

    use super::{
        create_new_transaction, get_journals, insert_transaction_metas, metas_from_request, update_single_transaction, upload_transaction_document,
        write_transaction_documents,
    };
    use crate::request::{CreateTransactionPostingRequest, CreateTransactionRequest, FlagRequest, JournalRequest, MetaRequest};
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
                    metas: None,
                },
                CreateTransactionPostingRequest {
                    account: "Expenses:Food".to_owned(),
                    unit: Some(Amount::new(BigDecimal::from_str("5").unwrap(), "CNY")),
                    metas: None,
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

    fn meta(key: &str, value: &str) -> MetaRequest {
        MetaRequest {
            key: key.to_owned(),
            value: value.to_owned(),
        }
    }

    /// The journal items of `ledger`, as the API returns them.
    async fn journals(ledger: Ledger) -> serde_json::Value {
        let (ledger, _) = states(ledger);
        let params = JournalRequest {
            page: None,
            size: None,
            keyword: None,
            tags: None,
            links: None,
        };
        let response = get_journals(ledger, crate::routes::Query(params)).await.into_response();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
        body["data"]["records"].clone()
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
            "option \"operating_currency\" \"CNY\"\ninclude \"data/2024/01.zhang\"\n\
             1970-01-01 commodity CNY\n1970-01-01 open Assets:Cash\n1970-01-01 open Expenses:Food\n",
        )
        .unwrap();
        let data_file = dir.join("data/2024/01.zhang");
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
            (
                "metadata key",
                "Posting-Receipt",
                Box::new(|it| it.postings[1].metas = Some(vec![meta("Posting-Receipt", "1")])),
            ),
            (
                "metadata key",
                "posting receipt",
                Box::new(|it| it.postings[0].metas = Some(vec![meta("posting receipt", "1")])),
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
        let main = "include \"data/2024/01.bean\"\n1970-01-01 commodity CNY\n1970-01-01 open Assets:Cash\n1970-01-01 open Expenses:Food\n";
        std::fs::write(dir.join("main.bean"), main).unwrap();
        let data_file = dir.join("data/2024/01.bean");
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
            "include \"data/2024/01.bean\"\n1970-01-01 commodity CNY\n1970-01-01 open Assets:Cash\n\
             1970-01-01 open Expenses:Food\n1970-01-01 open Assets:银行\n\
             2023-06-01 * \"Shop\" \"trip\" #旅行\n  Receipt: \"1\"\n  Assets:Cash -1 CNY\n    Lot: \"7\"\n  Expenses:Food 1 CNY\n",
        )
        .unwrap();
        // the local file system data source appends to existing `.bean` files only
        std::fs::write(dir.join("data/2024/01.bean"), "").unwrap();
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
            (
                "used posting metadata key",
                StatusCode::OK,
                Box::new(|it| it.postings[0].metas = Some(vec![meta("Lot", "8")])),
            ),
            (
                "used metadata key on a posting",
                StatusCode::OK,
                Box::new(|it| it.postings[1].metas = Some(vec![meta("Receipt", "3")])),
            ),
            (
                "new posting metadata key",
                StatusCode::BAD_REQUEST,
                Box::new(|it| it.postings[0].metas = Some(vec![meta("Memo", "2")])),
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
        let written = std::fs::read_to_string(dir.join("data/2024/01.bean")).unwrap();
        assert!(
            written.contains("Assets:银行") && written.contains("#旅行") && written.contains("Receipt: \"2\""),
            "{written}"
        );
        assert!(written.contains("    Lot: \"8\"") && written.contains("    Receipt: \"3\""), "{written}");
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

    #[tokio::test]
    async fn posting_metadata_is_created_updated_and_read_back() {
        let (dir, data_file) = ledger_dir();

        // create
        let (ledger, reload) = states(load(&dir).await);
        let mut create = request("coffee", "note");
        create.postings[0].metas = Some(vec![meta("receipt", "r-1"), meta("my key", "say \"hi\"")]);
        let response = create_new_transaction(ledger, reload, Json(create)).await.into_response();
        assert_eq!(response.status(), StatusCode::OK);

        let written = assert_written_text_round_trips(&data_file);
        assert!(
            written.contains("  note: \"note\"\n  Assets:Cash -5 CNY\n    \"my key\": \"say \\\"hi\\\"\"\n    receipt: \"r-1\"\n  Expenses:Food 5 CNY"),
            "{written}"
        );
        let reloaded = load(&dir).await;
        let (created, _) = transaction(&reloaded);
        let items = journals(reloaded).await;
        assert_eq!(items[0]["metas"], serde_json::json!([{"key": "note", "value": "note"}]));
        assert_eq!(
            items[0]["postings"][0]["metas"],
            serde_json::json!([{"key": "my key", "value": "say \"hi\""}, {"key": "receipt", "value": "r-1"}])
        );
        assert_eq!(items[0]["postings"][1]["metas"], serde_json::json!([]));

        // update: the client sends the posting metadata back, changed
        let mut update = request("coffee", "note");
        update.postings[0].metas = Some(vec![meta("receipt", "r-2")]);
        update.postings[1].metas = Some(vec![meta("category", "lunch")]);
        let (ledger, reload) = states(load(&dir).await);
        let response = update_single_transaction(ledger, reload, Path((created.id.to_string(),)), Json(update))
            .await
            .into_response();
        assert_eq!(response.status(), StatusCode::OK);

        assert_written_text_round_trips(&data_file);
        let items = journals(load(&dir).await).await;
        assert_eq!(items.as_array().unwrap().len(), 1);
        assert_eq!(items[0]["metas"], serde_json::json!([{"key": "note", "value": "note"}]));
        assert_eq!(items[0]["postings"][0]["metas"], serde_json::json!([{"key": "receipt", "value": "r-2"}]));
        assert_eq!(items[0]["postings"][1]["metas"], serde_json::json!([{"key": "category", "value": "lunch"}]));

        std::fs::remove_dir_all(dir).ok();
    }

    #[tokio::test]
    async fn posting_metadata_reads_back_in_a_beancount_ledger() {
        let dir = std::env::temp_dir().join(format!("zhang-beancount-posting-meta-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let dir = dir.canonicalize().unwrap();
        let main = "1970-01-01 commodity CNY\n1970-01-01 open Assets:Cash\n1970-01-01 open Expenses:Food\n\n\
                    2024-01-15 * \"Bob\" \"coffee\"\n  Assets:Cash -5 CNY\n  Expenses:Food 5 CNY\n  memo: \"after\"\n";
        std::fs::write(dir.join("main.bean"), main).unwrap();
        let load = || async {
            let source = Arc::new(LocalFileSystemDataSource::new(beancount::Beancount {}));
            Ledger::async_load(dir.clone(), "main.bean".to_owned(), source).await.expect("load ledger")
        };

        // as in beancount, metadata after a posting is that posting's
        let ledger = load().await;
        let created = ledger.operations().read().transactions.values().next().cloned().unwrap();
        let items = journals(ledger).await;
        assert_eq!(items[0]["metas"], serde_json::json!([]));
        assert_eq!(items[0]["postings"][1]["metas"], serde_json::json!([{"key": "memo", "value": "after"}]));

        // an update writes it under its posting, where beancount reads it too
        let mut update = request("coffee", "note");
        update.metas = vec![];
        update.postings[1].metas = Some(vec![meta("memo", "after")]);
        let (ledger, reload) = states(load().await);
        let response = update_single_transaction(ledger, reload, Path((created.id.to_string(),)), Json(update))
            .await
            .into_response();
        assert_eq!(response.status(), StatusCode::OK);
        let written = std::fs::read_to_string(dir.join("main.bean")).unwrap();
        assert!(written.contains("  Expenses:Food 5 CNY\n    memo: \"after\""), "{written}");
        let items = journals(load().await).await;
        assert_eq!(items[0]["postings"][1]["metas"], serde_json::json!([{"key": "memo", "value": "after"}]));

        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn uploaded_documents_go_under_the_transaction_header() {
        let content = "2024-01-15 * \"Bob\" \"coffee\" ; a comment\n  Assets:Cash -5 CNY\n  Expenses:Food 5 CNY\n\n2024-01-16 open Assets:Bank\n";
        let end = content.find("\n\n").unwrap();
        let mut written = content.to_owned();
        let lines = vec!["  document: \"a.pdf\"".to_owned(), "  document: \"b.pdf\"".to_owned()];
        insert_transaction_metas(&mut written, 0, end, &lines);
        assert_eq!(
            written,
            "2024-01-15 * \"Bob\" \"coffee\" ; a comment\n  document: \"a.pdf\"\n  document: \"b.pdf\"\n  Assets:Cash -5 CNY\n  Expenses:Food 5 CNY\n\n2024-01-16 open Assets:Bank\n"
        );
        // there they are the transaction's in both formats
        for directives in [
            ZhangDataType {}.transform(written.clone(), None).unwrap(),
            beancount::Beancount {}.transform(written.clone(), None).unwrap(),
        ] {
            let Directive::Transaction(transaction) = &directives[0].data else {
                panic!("expected a transaction")
            };
            assert_eq!(transaction.meta.get_all("document").len(), 2);
            assert!(transaction.postings.iter().all(|posting| posting.meta.get_one("document").is_none()));
        }

        // nothing to insert leaves the text as it is
        let mut unchanged = content.to_owned();
        insert_transaction_metas(&mut unchanged, 0, end, &[]);
        assert_eq!(unchanged, content);
    }

    /// Documents are written under the header even when a quoted payee or narration
    /// spans several lines, in both formats, and the rest of the text is left as it is.
    #[tokio::test]
    async fn uploaded_documents_go_under_a_header_spanning_several_lines() {
        let cases = [
            ("Bob", "multi\nline narration", "\"Bob\" \"multi\nline narration\""),
            ("Bob\n  Assets:Cash -1 CNY", "coffee", "\"Bob\n  Assets:Cash -1 CNY\" \"coffee\""),
            ("Bob", "say \"hi\"\nthere", "\"Bob\" \"say \\\"hi\\\"\nthere\""),
        ];
        for main in ["main.zhang", "main.bean"] {
            for (payee, narration, strings) in cases {
                let dir = std::env::temp_dir().join(format!("zhang-document-header-{}", Uuid::new_v4()));
                std::fs::create_dir_all(&dir).unwrap();
                let dir = dir.canonicalize().unwrap();
                let opens = "1970-01-01 commodity CNY\n1970-01-01 open Assets:Cash\n1970-01-01 open Expenses:Food\n\n";
                let postings = "  Assets:Cash -5 CNY\n  Expenses:Food 5 CNY\n";
                let header = format!("2024-01-15 * {strings} ; a \"comment\"\n");
                std::fs::write(dir.join(main), format!("{opens}{header}{postings}")).unwrap();
                let load = || async {
                    let source: Arc<LocalFileSystemDataSource> = if main.ends_with(".bean") {
                        Arc::new(LocalFileSystemDataSource::new(beancount::Beancount {}))
                    } else {
                        Arc::new(LocalFileSystemDataSource::new(ZhangDataType {}))
                    };
                    Ledger::async_load(dir.clone(), main.to_owned(), source).await.expect("load ledger")
                };

                let ledger = load().await;
                let id = ledger.operations().read().transactions.values().next().unwrap().id;
                let span = ledger.operations().transaction_span(&id).unwrap().unwrap();
                write_transaction_documents(&ledger, &span, &["attachments/a.pdf".to_owned()]).await.unwrap();

                let written = std::fs::read_to_string(dir.join(main)).unwrap();
                assert_eq!(written, format!("{opens}{header}  document: \"attachments/a.pdf\"\n{postings}"), "{main}");
                let reloaded = load().await;
                let operations = reloaded.operations();
                let store = operations.read();
                assert!(store.errors.is_empty(), "{main} {:?}: {:?}", strings, store.errors);
                let transaction = store.transactions.values().next().unwrap();
                assert_eq!(transaction.payee.as_deref(), Some(payee), "{main}");
                assert_eq!(transaction.narration.as_deref(), Some(narration), "{main}");
                assert_eq!(transaction.postings.len(), 2, "{main}");
                assert!(transaction.postings.iter().all(|posting| posting.metas.is_empty()), "{main}");
                let documents = store
                    .documents
                    .iter()
                    .filter(|it| it.document_type.as_trx() == Some(transaction.id.to_string()))
                    .map(|it| it.path.clone())
                    .collect::<Vec<_>>();
                assert_eq!(documents, vec!["attachments/a.pdf"], "{main} {strings}");
                let metas = store.metas.iter().filter(|it| it.type_identifier == transaction.id.to_string()).count();
                assert_eq!(metas, 1, "the document is transaction metadata");
                drop(store);
                std::fs::remove_dir_all(dir).ok();
            }
        }
    }

    /// An edit that leaves a metadata value as it was writes it as it was: an unquoted
    /// number, date or boolean stays one for beancount. A changed value is quoted.
    #[tokio::test]
    async fn an_edit_keeps_unchanged_bare_metadata_values_bare() {
        let dir = std::env::temp_dir().join(format!("zhang-bare-meta-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let dir = dir.canonicalize().unwrap();
        let main = "1970-01-01 commodity CNY\n1970-01-01 open Assets:Cash\n1970-01-01 open Expenses:Food\n\n\
                    2024-01-15 * \"Bob\" \"coffee\"\n  rate: 1.5\n  day: 2024-01-15\n  paid: TRUE\n  note: \"n\"\n  \
                    Assets:Cash -5 CNY\n    rate: 2.5\n    day: 2024-01-14\n    cleared: FALSE\n  Expenses:Food 5 CNY\n    rate: 7\n";
        std::fs::write(dir.join("main.bean"), main).unwrap();
        let load = || async {
            let source = Arc::new(LocalFileSystemDataSource::new(beancount::Beancount {}));
            Ledger::async_load(dir.clone(), "main.bean".to_owned(), source).await.expect("load ledger")
        };

        let ledger = load().await;
        let id = ledger.operations().read().transactions.values().next().unwrap().id;
        // the client sends every value back as text, a few of them changed
        let mut update = request("coffee", "n");
        update.payee = "Bob".to_owned();
        update.metas = vec![meta("rate", "1.5"), meta("day", "2024-02-01"), meta("paid", "TRUE"), meta("note", "n")];
        update.postings[0].metas = Some(vec![meta("rate", "3.5"), meta("day", "2024-01-14"), meta("cleared", "FALSE")]);
        // the same text under a key the posting did not have unquoted
        update.postings[1].metas = Some(vec![meta("rate", "7"), meta("paid", "TRUE")]);
        let (ledger, reload) = states(ledger);
        let response = update_single_transaction(ledger, reload, Path((id.to_string(),)), Json(update))
            .await
            .into_response();
        assert_eq!(response.status(), StatusCode::OK);

        let written = std::fs::read_to_string(dir.join("main.bean")).unwrap();
        for line in [
            // unchanged: as they were
            "\n  rate: 1.5\n",
            "\n  paid: TRUE\n",
            "\n  note: \"n\"\n",
            "\n    day: 2024-01-14\n",
            "\n    cleared: FALSE\n",
            "\n    rate: 7\n",
            // changed or new: quoted
            "\n  day: \"2024-02-01\"\n",
            "\n    rate: \"3.5\"\n",
            "\n    paid: \"TRUE\"",
        ] {
            assert!(written.contains(line), "{line:?} in\n{written}");
        }
        let reloaded = load().await;
        assert!(reloaded.operations().read().errors.is_empty(), "{written}");
        let items = journals(reloaded).await;
        assert_eq!(
            items[0]["postings"][0]["metas"],
            serde_json::json!([{"key": "cleared", "value": "FALSE"}, {"key": "day", "value": "2024-01-14"}, {"key": "rate", "value": "3.5"}])
        );

        std::fs::remove_dir_all(dir).ok();
    }

    /// An unquoted value is only written back unquoted when it reads back as that value:
    /// a plugin can make an unquoted value of any text.
    #[test]
    fn only_values_that_read_back_bare_stay_bare() {
        use zhang_ast::{Meta, ZhangString};

        let mut original = Meta::default();
        for value in ["from plugin", "a: b", "", "1.5", "2024-01-15", "TRUE"] {
            original.insert(
                format!("k{}", original.clone().get_flatten().len()),
                ZhangString::UnquoteString(value.to_owned()),
            );
        }
        let request = original
            .clone()
            .get_flatten()
            .into_iter()
            .map(|(key, value)| meta(&key, value.as_str()))
            .collect::<Vec<_>>();
        let written = metas_from_request(request, &crate::validate::Rules::Zhang, Some(&original)).unwrap();
        let mut forms = written
            .get_flatten()
            .into_iter()
            .map(|(_, value)| match value {
                ZhangString::UnquoteString(bare) => format!("bare {bare}"),
                ZhangString::QuoteString(quoted) => format!("quoted {quoted}"),
            })
            .collect::<Vec<_>>();
        forms.sort();
        assert_eq!(
            forms,
            vec!["bare 1.5", "bare 2024-01-15", "bare TRUE", "quoted ", "quoted a: b", "quoted from plugin"]
        );
    }

    /// Write `ledger` as the `main.bean` of a new ledger, apply `update` to its only
    /// transaction and return the file written, after checking it reloads without errors.
    async fn edit_bean(ledger: &str, update: CreateTransactionRequest) -> String {
        edit_ledger("main.bean", ledger, update).await.0
    }

    /// Write `ledger` as the `main` file (`main.bean` or `main.zhang`) of a new ledger, apply
    /// `update` to its only transaction and return the file written, after checking it reloads
    /// without errors, with the ledger it reloads to.
    async fn edit_ledger(main: &str, ledger: &str, update: CreateTransactionRequest) -> (String, Ledger) {
        edit_loaded_ledger(main, ledger, |_| {}, update).await
    }

    /// [`edit_ledger`], with `prepare` applied to the loaded ledger before the update, as a plugin
    /// could have changed it.
    async fn edit_loaded_ledger(main: &str, ledger: &str, prepare: impl FnOnce(&mut Ledger), update: CreateTransactionRequest) -> (String, Ledger) {
        let dir = std::env::temp_dir().join(format!("zhang-edit-ledger-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let dir = dir.canonicalize().unwrap();
        let opens = "1970-01-01 commodity CNY\n1970-01-01 open Assets:Cash\n1970-01-01 open Expenses:Food\n\n";
        std::fs::write(dir.join(main), format!("{opens}{ledger}")).unwrap();
        let load = || async {
            let source = if main.ends_with(".bean") {
                Arc::new(LocalFileSystemDataSource::new(beancount::Beancount {}))
            } else {
                Arc::new(LocalFileSystemDataSource::new(ZhangDataType {}))
            };
            Ledger::async_load(dir.clone(), main.to_owned(), source).await.expect("load ledger")
        };
        let mut loaded = load().await;
        prepare(&mut loaded);
        let id = loaded.operations().read().transactions.values().next().unwrap().id;
        let (state, reload) = states(loaded);
        let response = update_single_transaction(state, reload, Path((id.to_string(),)), Json(update))
            .await
            .into_response();
        assert_eq!(response.status(), StatusCode::OK);
        let written = std::fs::read_to_string(dir.join(main)).unwrap();
        let reloaded = load().await;
        assert!(reloaded.operations().read().errors.is_empty(), "{written}");
        std::fs::remove_dir_all(dir).ok();
        (written, reloaded)
    }

    /// An edit keeps the flag of each posting it edits (#474): a request carries no posting
    /// flag, so the posting it is matched to, as for metadata, gives it.
    #[tokio::test]
    async fn an_edit_keeps_the_flags_of_the_postings() {
        let ledger = "2024-01-15 * \"Bob\" \"coffee\"\n  ! Assets:Cash -5 CNY\n    rate: 1.5\n  & Expenses:Food 5 CNY\n";
        for main in ["main.zhang", "main.bean"] {
            // amounts changed and postings reordered, each keeps its flag
            let update = edit(&[("Expenses:Food", 6, &[]), ("Assets:Cash", -6, &[("rate", "1.5")])]);
            let (written, reloaded) = edit_ledger(main, ledger, update).await;
            let postings = &written[written.find("\n  & Expenses:Food").expect(&written)..];
            assert_eq!(
                postings, "\n  & Expenses:Food 6 CNY\n  ! Assets:Cash -6 CNY\n    rate: 1.5\n",
                "{main}: {written}"
            );
            let flags = reloaded
                .operations()
                .read()
                .postings
                .iter()
                .map(|it| (it.account.name().to_owned(), it.flag.clone()))
                .collect::<Vec<_>>();
            assert_eq!(
                flags,
                vec![
                    ("Expenses:Food".to_owned(), Some(Flag::Custom("&".to_owned()))),
                    ("Assets:Cash".to_owned(), Some(Flag::Warning))
                ],
                "{main}"
            );

            // a posting the request splits matches none of the transaction: it is new, without a flag
            let update = edit(&[("Assets:Cash", -2, &[]), ("Assets:Cash", -3, &[]), ("Expenses:Food", 5, &[])]);
            let (written, _) = edit_ledger(main, ledger, update).await;
            let postings = &written[written.find("\n  Assets:Cash -2 CNY").expect(&written)..];
            assert_eq!(
                postings, "\n  Assets:Cash -2 CNY\n  Assets:Cash -3 CNY\n  & Expenses:Food 5 CNY\n",
                "{main}: {written}"
            );
        }
    }

    /// A posting flag `#`, which a zhang file reads as the start of a comment, can only come from a
    /// plugin in a zhang ledger. An edit then writes the posting without it, so that it stays a
    /// posting and the file keeps what it had. A `*` flag is written in a zhang ledger as in a
    /// beancount one, where `#` is written too, as beancount reads it.
    #[tokio::test]
    async fn an_edit_never_writes_a_posting_as_a_comment() {
        let ledger = "2024-01-15 * \"Bob\" \"coffee\"\n  Assets:Cash -5 CNY\n  Expenses:Food 5 CNY\n";
        // what a plugin could do: flag the postings `*` and `#`
        let flag_by_plugin = |ledger: &mut Ledger| {
            let transaction = ledger.directives.iter_mut().find_map(|it| match &mut it.data {
                Directive::Transaction(transaction) => Some(transaction),
                _ => None,
            });
            let postings = &mut transaction.unwrap().postings;
            postings[0].flag = Some(Flag::Okay);
            postings[1].flag = Some(Flag::Custom("#".to_owned()));
        };
        let update = || edit(&[("Assets:Cash", -6, &[]), ("Expenses:Food", 6, &[])]);

        let (written, reloaded) = edit_loaded_ledger("main.zhang", ledger, flag_by_plugin, update()).await;
        let postings = &written[written.find("\n  * Assets:Cash").expect(&written)..];
        assert_eq!(postings, "\n  * Assets:Cash -6 CNY\n  Expenses:Food 6 CNY\n", "{written}");
        let store = reloaded
            .operations()
            .read()
            .postings
            .iter()
            .map(|it| (it.flag.clone(), it.after_amount.number.to_string()))
            .collect::<Vec<_>>();
        assert_eq!(store, vec![(Some(Flag::Okay), "-6".to_owned()), (None, "6".to_owned())]);

        let (written, reloaded) = edit_loaded_ledger("main.bean", ledger, flag_by_plugin, update()).await;
        let postings = &written[written.find("\n  * Assets:Cash").expect(&written)..];
        assert_eq!(postings, "\n  * Assets:Cash -6 CNY\n  # Expenses:Food 6 CNY\n", "{written}");
        let flags = reloaded.operations().read().postings.iter().map(|it| it.flag.clone()).collect::<Vec<_>>();
        assert_eq!(flags, vec![Some(Flag::Okay), Some(Flag::Custom("#".to_owned()))]);
    }

    /// A posting of an [`edit`]: an account, a number of CNY and its metadata.
    type EditedPosting<'a> = (&'a str, i64, &'a [(&'a str, &'a str)]);

    /// An update request for a transaction `Bob` `coffee` with `postings`.
    fn edit(postings: &[EditedPosting]) -> CreateTransactionRequest {
        let mut update = request("coffee", "n");
        update.payee = "Bob".to_owned();
        update.metas = vec![];
        update.postings = postings
            .iter()
            .map(|(account, number, metas)| CreateTransactionPostingRequest {
                account: account.to_string(),
                unit: Some(Amount::new(BigDecimal::from(*number), "CNY")),
                metas: Some(metas.iter().map(|(key, value)| meta(key, value)).collect()),
            })
            .collect();
        update
    }

    /// The postings of the edited transaction in `written`, from its first posting on.
    fn postings_of(written: &str) -> &str {
        let start = ["\n  Assets", "\n  Expenses"].iter().filter_map(|it| written.find(it)).min().unwrap();
        &written[start..]
    }

    /// A request posting takes the original forms of the posting to the same account,
    /// wherever it is in the request, and none when that posting is not clear.
    #[tokio::test]
    async fn edited_postings_are_matched_by_account() {
        let ledger = "2024-01-15 * \"Bob\" \"coffee\"\n  Assets:Cash -5 CNY\n    rate: 1.5\n  Expenses:Food 5 CNY\n    rate: \"9\"\n";

        // the value of Cash sent for Food is a change for Food: quoted
        let written = edit_bean(ledger, edit(&[("Expenses:Food", 5, &[("rate", "1.5")]), ("Assets:Cash", -5, &[])])).await;
        assert_eq!(
            postings_of(&written),
            "\n  Expenses:Food 5 CNY\n    rate: \"1.5\"\n  Assets:Cash -5 CNY\n",
            "{written}"
        );

        // reordered, each value keeps its form
        let written = edit_bean(ledger, edit(&[("Expenses:Food", 5, &[("rate", "9")]), ("Assets:Cash", -5, &[("rate", "1.5")])])).await;
        assert_eq!(
            postings_of(&written),
            "\n  Expenses:Food 5 CNY\n    rate: \"9\"\n  Assets:Cash -5 CNY\n    rate: 1.5\n",
            "{written}"
        );

        // several postings to an account are told apart by their units
        let ledger = "2024-01-15 * \"Bob\" \"coffee\"\n  Assets:Cash -4 CNY\n    rate: 1.5\n  Assets:Cash -6 CNY\n    rate: 2.5\n  Expenses:Food 10 CNY\n";
        let written = edit_bean(
            ledger,
            edit(&[
                ("Assets:Cash", -6, &[("rate", "2.5")]),
                ("Assets:Cash", -4, &[("rate", "1.5")]),
                ("Expenses:Food", 10, &[]),
            ]),
        )
        .await;
        assert_eq!(
            postings_of(&written),
            "\n  Assets:Cash -6 CNY\n    rate: 2.5\n  Assets:Cash -4 CNY\n    rate: 1.5\n  Expenses:Food 10 CNY\n",
            "{written}"
        );

        // and when the units do not tell them apart either, every value is new
        let ledger = "2024-01-15 * \"Bob\" \"coffee\"\n  Assets:Cash -5 CNY\n    rate: 1.5\n  Assets:Cash -5 CNY\n    rate: 2.5\n  Expenses:Food 10 CNY\n";
        let written = edit_bean(
            ledger,
            edit(&[
                ("Assets:Cash", -5, &[("rate", "1.5")]),
                ("Assets:Cash", -5, &[("rate", "2.5")]),
                ("Expenses:Food", 10, &[]),
            ]),
        )
        .await;
        assert_eq!(
            postings_of(&written),
            "\n  Assets:Cash -5 CNY\n    rate: \"1.5\"\n  Assets:Cash -5 CNY\n    rate: \"2.5\"\n  Expenses:Food 10 CNY\n",
            "{written}"
        );
    }

    /// In a file with CRLF line endings the document line goes after the whole line
    /// ending of the header, never between its `\r` and `\n`.
    #[tokio::test]
    async fn uploaded_documents_go_under_a_crlf_header() {
        for main in ["main.zhang", "main.bean"] {
            let dir = std::env::temp_dir().join(format!("zhang-document-crlf-{}", Uuid::new_v4()));
            std::fs::create_dir_all(&dir).unwrap();
            let dir = dir.canonicalize().unwrap();
            let opens = "1970-01-01 commodity CNY\r\n1970-01-01 open Assets:Cash\r\n1970-01-01 open Expenses:Food\r\n\r\n";
            let header = "2024-01-15 * \"Bob\" \"coffee\"\r\n";
            let postings = "  Assets:Cash -5 CNY\r\n  Expenses:Food 5 CNY\r\n";
            std::fs::write(dir.join(main), format!("{opens}{header}{postings}")).unwrap();
            let load = || async {
                let source: Arc<LocalFileSystemDataSource> = if main.ends_with(".bean") {
                    Arc::new(LocalFileSystemDataSource::new(beancount::Beancount {}))
                } else {
                    Arc::new(LocalFileSystemDataSource::new(ZhangDataType {}))
                };
                Ledger::async_load(dir.clone(), main.to_owned(), source).await.expect("load ledger")
            };

            let ledger = load().await;
            let id = ledger.operations().read().transactions.values().next().unwrap().id;
            let span = ledger.operations().transaction_span(&id).unwrap().unwrap();
            write_transaction_documents(&ledger, &span, &["attachments/a.pdf".to_owned()]).await.unwrap();

            let written = std::fs::read_to_string(dir.join(main)).unwrap();
            assert_eq!(written, format!("{opens}{header}  document: \"attachments/a.pdf\"\n{postings}"), "{main}");
            let reloaded = load().await;
            let operations = reloaded.operations();
            let store = operations.read();
            assert!(store.errors.is_empty(), "{main}: {:?}", store.errors);
            let transaction = store.transactions.values().next().unwrap();
            assert_eq!(transaction.narration.as_deref(), Some("coffee"), "{main}");
            assert_eq!(transaction.postings.len(), 2, "{main}");
            let documents = store
                .documents
                .iter()
                .filter(|it| it.document_type.as_trx() == Some(transaction.id.to_string()))
                .count();
            assert_eq!(documents, 1, "{main}");
            drop(store);
            std::fs::remove_dir_all(dir).ok();
        }
    }

    /// Transactions of two files can start at the same offset: an edit takes the
    /// original forms of its values from the transaction of its own file.
    #[tokio::test]
    async fn an_edit_reads_the_original_of_its_own_file() {
        let dir = std::env::temp_dir().join(format!("zhang-edit-two-files-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let dir = dir.canonicalize().unwrap();
        let transaction =
            |payee: &str, rate: &str| format!("2024-01-15 * \"{payee}\" \"coffee\"\n  rate: {rate}\n  Assets:Cash -5 CNY\n  Expenses:Food 5 CNY\n");
        std::fs::write(
            dir.join("main.bean"),
            format!(
                "{}\ninclude \"other.bean\"\n1970-01-01 commodity CNY\n1970-01-01 open Assets:Cash\n1970-01-01 open Expenses:Food\n",
                transaction("Main", "1.5")
            ),
        )
        .unwrap();
        std::fs::write(dir.join("other.bean"), transaction("Other", "\"1.5\"")).unwrap();
        let load = || async {
            let source = Arc::new(LocalFileSystemDataSource::new(beancount::Beancount {}));
            Ledger::async_load(dir.clone(), "main.bean".to_owned(), source).await.expect("load ledger")
        };

        for (payee, file, written_rate) in [("Other", "other.bean", "  rate: \"1.5\"\n"), ("Main", "main.bean", "  rate: 1.5\n")] {
            let ledger = load().await;
            let (id, start) = {
                let operations = ledger.operations();
                let store = operations.read();
                let transaction = store.transactions.values().find(|it| it.payee.as_deref() == Some(payee)).unwrap();
                (transaction.id, transaction.span.start)
            };
            assert_eq!(start, 0, "both transactions start their file");
            let mut update = request("coffee", "n");
            update.payee = payee.to_owned();
            update.metas = vec![meta("rate", "1.5")];
            let (state, reload) = states(ledger);
            let response = update_single_transaction(state, reload, Path((id.to_string(),)), Json(update))
                .await
                .into_response();
            assert_eq!(response.status(), StatusCode::OK);
            let written = std::fs::read_to_string(dir.join(file)).unwrap();
            assert!(written.contains(written_rate), "{file}: {written}");
        }
        assert!(load().await.operations().read().errors.is_empty());

        std::fs::remove_dir_all(dir).ok();
    }

    /// A request posting is only matched when it is the only one with its account in
    /// the request too: splitting a posting makes every value of the parts new.
    #[tokio::test]
    async fn a_split_posting_is_not_matched() {
        let ledger = "2024-01-15 * \"Bob\" \"coffee\"\n  Assets:Cash -10 CNY\n    rate: 1.5\n  Expenses:Food 10 CNY\n";
        let written = edit_bean(
            ledger,
            edit(&[
                ("Assets:Cash", -4, &[("rate", "1.5")]),
                ("Assets:Cash", -6, &[("rate", "1.5")]),
                ("Expenses:Food", 10, &[]),
            ]),
        )
        .await;
        assert_eq!(
            postings_of(&written),
            "\n  Assets:Cash -4 CNY\n    rate: \"1.5\"\n  Assets:Cash -6 CNY\n    rate: \"1.5\"\n  Expenses:Food 10 CNY\n",
            "{written}"
        );
    }

    /// Nor when several request postings have the account and units of a single original
    /// posting: sending the same posting twice makes every value of both new.
    #[tokio::test]
    async fn identical_request_postings_are_not_matched() {
        let ledger = "2024-01-15 * \"Bob\" \"coffee\"\n  Assets:Cash -5 CNY\n    rate: 1.5\n  Assets:Cash -6 CNY\n  Expenses:Food 11 CNY\n";
        let written = edit_bean(
            ledger,
            edit(&[
                ("Assets:Cash", -5, &[("rate", "1.5")]),
                ("Assets:Cash", -5, &[("rate", "1.5")]),
                ("Expenses:Food", 10, &[]),
            ]),
        )
        .await;
        assert_eq!(
            postings_of(&written),
            "\n  Assets:Cash -5 CNY\n    rate: \"1.5\"\n  Assets:Cash -5 CNY\n    rate: \"1.5\"\n  Expenses:Food 10 CNY\n",
            "{written}"
        );
    }

    /// A repeated key keeps the form of each of its values only as many times as the
    /// original has that value: a value changed into another one's is new.
    #[tokio::test]
    async fn repeated_values_are_matched_by_occurrence() {
        let ledger = "2024-01-15 * \"Bob\" \"coffee\"\n  dup: 1.5\n  dup: 2.5\n  Assets:Cash -5 CNY\n    dup: 1.5\n    dup: 2.5\n  Expenses:Food 5 CNY\n";
        // 2.5 changed into 1.5, on the transaction and on the posting
        let mut update = edit(&[("Assets:Cash", -5, &[("dup", "1.5"), ("dup", "1.5")]), ("Expenses:Food", 5, &[])]);
        update.metas = vec![meta("dup", "1.5"), meta("dup", "1.5")];
        let written = edit_bean(ledger, update).await;
        assert!(written.contains("\n  dup: 1.5\n  dup: \"1.5\"\n"), "{written}");
        assert_eq!(
            postings_of(&written),
            "\n  Assets:Cash -5 CNY\n    dup: 1.5\n    dup: \"1.5\"\n  Expenses:Food 5 CNY\n",
            "{written}"
        );

        // unchanged, both keep their form
        let mut update = edit(&[("Assets:Cash", -5, &[("dup", "1.5"), ("dup", "2.5")]), ("Expenses:Food", 5, &[])]);
        update.metas = vec![meta("dup", "2.5"), meta("dup", "1.5")];
        let written = edit_bean(ledger, update).await;
        assert!(written.contains("\n  dup: 2.5\n  dup: 1.5\n"), "{written}");
        assert_eq!(
            postings_of(&written),
            "\n  Assets:Cash -5 CNY\n    dup: 1.5\n    dup: 2.5\n  Expenses:Food 5 CNY\n",
            "{written}"
        );
    }
    /// A document or an update of a transaction in a file changed since the ledger was loaded is refused with a 409,
    /// and nothing is written: the transaction is no longer where the ledger loaded it, and has another id. Tried
    /// again, the ledger is reloaded first: the old id is a 404, and the id of the journal reopened is written in the
    /// right place.
    #[tokio::test]
    async fn a_transaction_in_a_file_changed_since_the_load_is_not_written() {
        let dir = std::env::temp_dir().join(format!("zhang-stale-transaction-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let dir = dir.canonicalize().unwrap();
        let main = dir.join("main.bean");
        let opens = "1970-01-01 commodity CNY\n1970-01-01 open Assets:Cash\n1970-01-01 open Expenses:Food\n\n";
        let ledger = format!("{opens}2024-01-15 * \"Bob\" \"coffee\"\n  Assets:Cash -5 CNY\n  Expenses:Food 5 CNY\n");
        std::fs::write(&main, &ledger).unwrap();
        let source = Arc::new(LocalFileSystemDataSource::new(beancount::Beancount {}));
        let loaded = Ledger::async_load(dir.clone(), "main.bean".to_owned(), source).await.expect("load ledger");
        let id = loaded.operations().read().transactions.values().next().unwrap().id;
        let span = loaded.operations().transaction_span(&id).unwrap().unwrap();
        // an editor adds a line at the top, which the ledger has not loaded yet
        let edited = format!("; an editor adds this line\n{ledger}");
        std::fs::write(&main, &edited).unwrap();

        let refused = write_transaction_documents(&loaded, &span, &["attachments/a.pdf".to_owned()])
            .await
            .unwrap_err();
        assert_eq!(refused.into_response().status(), StatusCode::CONFLICT);
        assert_eq!(std::fs::read_to_string(&main).unwrap(), edited, "nothing is written");

        // through the API, the message tells the transaction may have moved, under another id
        let (ledger_state, reload) = states(loaded);
        let body = |response: axum::response::Response| async move {
            let status = response.status();
            let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
            (
                status,
                serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()["message"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned(),
            )
        };
        let upload = || {
            let request = axum::http::Request::builder()
                .method("POST")
                .header("content-type", "multipart/form-data; boundary=X")
                .body(axum::body::Body::from(
                    "--X\r\nContent-Disposition: form-data; name=\"file\"; filename=\"a.pdf\"\r\n\r\n%PDF\r\n--X--\r\n",
                ))
                .unwrap();
            async {
                <axum::extract::Multipart as axum::extract::FromRequest<()>>::from_request(request, &())
                    .await
                    .unwrap()
            }
        };
        let (status, message) = body(
            upload_transaction_document(ledger_state.clone(), reload.clone(), Path((id.to_string(),)), upload().await)
                .await
                .into_response(),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{message}");
        assert!(
            message.contains("main.bean changed since the ledger was loaded, so nothing was written"),
            "{message}"
        );
        assert!(message.contains("under another id: reopen the journal, and try again"), "{message}");
        assert_eq!(std::fs::read_to_string(&main).unwrap(), edited, "nothing is written");
        assert!(!dir.join("attachments").exists(), "no attachment is saved");

        // tried again with that id: the ledger is reloaded, and the transaction has another id now
        let update = || edit(&[("Assets:Cash", -6, &[]), ("Expenses:Food", 6, &[])]);
        let (status, message) = body(
            update_single_transaction(ledger_state.clone(), reload.clone(), Path((id.to_string(),)), Json(update()))
                .await
                .into_response(),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{message}");
        assert!(message.contains(&format!("there is no transaction {id} in the ledger")), "{message}");
        assert!(message.contains("Reopen the journal, and try again"), "{message}");
        assert_eq!(std::fs::read_to_string(&main).unwrap(), edited, "nothing is written");

        // with the id of the journal reopened, the update is written in the right place
        let id = ledger_state.read().await.operations().read().transactions.values().next().unwrap().id;
        let response = update_single_transaction(ledger_state, reload, Path((id.to_string(),)), Json(update()))
            .await
            .into_response();
        assert_eq!(response.status(), StatusCode::OK);
        let written = std::fs::read_to_string(&main).unwrap();
        assert!(written.starts_with("; an editor adds this line\n"), "{written}");
        assert!(written.contains("  Assets:Cash -6 CNY\n  Expenses:Food 6 CNY\n"), "{written}");
        assert!(!written.contains("5 CNY"), "{written}");
        std::fs::remove_dir_all(dir).ok();
    }
}
