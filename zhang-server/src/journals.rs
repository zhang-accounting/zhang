//! The journal, the suggestions of the new-transaction form, the documents and the errors of the
//! ledger as built-in queries of the query engine (#479): every response is one or more named BQL
//! queries plus a thin mapping into its existing shape.
//!
//! User input is only ever bound as parameters, never formatted into a query. The queries of one
//! response run under one read guard of the ledger, so they see the same ledger.
//!
//! Until the shared registry of built-in queries is in place, the queries of this module are
//! compiled and run here, once, with the limits of `/api/query`.

use std::collections::{BTreeSet, HashMap};
use std::path::Path;
use std::str::FromStr;
use std::sync::LazyLock;

use bigdecimal::BigDecimal;
use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
use itertools::Itertools;
use uuid::Uuid;
use zhang_ast::amount::Amount;
use zhang_ast::error::ErrorKind;
use zhang_ast::Flag;
use zhang_core::constants::BALANCE_CHECK_PAYEE;
use zhang_core::ledger::Ledger;
use zhang_query::{DataType, ExecuteOptions, ParamTypes, Params, Query, QueryResult, Value};

use crate::error::ServerError;
use crate::request::JournalRequest;
use crate::response::{
    DocumentEntity, ErrorEntity, InfoForNewTransaction, JournalBalanceCheckItemEntity, JournalBalanceItemEntity, JournalItemEntity,
    JournalTransactionItemEntity, JournalTransactionPostingEntity, MetaEntity, Pageable, SpanInfoEntity,
};
use crate::routes::query::{execute_options, max_result_values};
use crate::state::SharedLedger;
use crate::ServerResult;

/// A named, documented query the server runs, with the types of its parameters.
pub struct BuiltinQuery {
    pub name: &'static str,
    pub description: &'static str,
    pub bql: &'static str,
    pub params: &'static [(&'static str, DataType)],
}

/// One page of the journal: the transactions and the balance assertions, newest first.
pub const JOURNAL: BuiltinQuery = BuiltinQuery {
    name: "journal.page",
    description: "The journal: the transactions (padding transactions included) and the balance assertions, newest first, one \
                  page at a time. `keyword` keeps the transactions whose payee, narration, tags, links or accounts contain it, \
                  ignoring case, and the balance assertions whose accounts or the words Balance Check contain it; `tags` and \
                  `links` keep the transactions that have one of them, and no balance assertion. A NULL parameter keeps every \
                  row.",
    bql: "SELECT seq, type, id, date, time, flag, payee, narration, tags, links, metas
FROM #entries
WHERE (type = 'transaction'
       AND (:tags IS NULL OR intersects(tags, :tags))
       AND (:links IS NULL OR intersects(links, :links))
       AND (:keyword IS NULL OR icontains(payee, :keyword) OR icontains(narration, :keyword)
            OR any_icontains(tags, :keyword) OR any_icontains(links, :keyword) OR any_icontains(accounts, :keyword)))
   OR (type = 'balance' AND :tags IS NULL AND :links IS NULL
       AND (:keyword IS NULL OR icontains('Balance Check', :keyword) OR any_icontains(accounts, :keyword)))
ORDER BY seq DESC
LIMIT :size OFFSET :offset",
    params: &[
        ("keyword", DataType::Str),
        ("tags", DataType::Set),
        ("links", DataType::Set),
        ("size", DataType::Int),
        ("offset", DataType::Int),
    ],
};

/// The postings of some transactions of the journal.
pub const JOURNAL_POSTINGS: BuiltinQuery = BuiltinQuery {
    name: "journal.postings",
    description: "The postings of the transactions with the ids `ids`, one row per posting as written (the lots booking split \
                  it into added up), in ledger order: its units, whether they were inferred (`automatic`), the per-unit cost \
                  of its first lot, the balance of its account in the posting's currency before and after it, and whether its \
                  transaction balances.",
    bql: "SELECT id, posting_index, account, automatic, balanced,
       first(currency) AS currency,
       sum(number) AS number,
       first(cost_number) AS cost_number, first(cost_currency) AS cost_currency,
       number(last(only(currency, account_balance))) - sum(number) AS balance_before,
       number(last(only(currency, account_balance))) AS balance_after,
       first(metas) AS metas
WHERE id IN :ids
GROUP BY id, posting_index, account, automatic, balanced",
    params: &[("ids", DataType::Set)],
};

/// The balance assertions of the journal.
pub const JOURNAL_BALANCE_CHECKS: BuiltinQuery = BuiltinQuery {
    name: "journal.balance_checks",
    description: "The balance assertions with the ids `ids`: the asserted amount, the account's true balance, their difference \
                  and whether the assertion holds.",
    bql: "SELECT id, account, amount, tolerance, actual, passed,
       amount - actual AS difference,
       actual + (amount - actual) AS asserted
FROM #balances
WHERE id IN :ids",
    params: &[("ids", DataType::Set)],
};

/// The payees the new-transaction form suggests.
pub const PAYEES: BuiltinQuery = BuiltinQuery {
    name: "new_transaction.payees",
    description: "Every payee of the ledger's transactions, once and sorted, without those of the padding transactions.",
    bql: "SELECT DISTINCT payee
FROM #transactions
WHERE payee IS NOT NULL AND payee != '' AND flag != 'P'
ORDER BY payee",
    params: &[],
};

/// The accounts the new-transaction form offers.
pub const OPEN_ACCOUNTS: BuiltinQuery = BuiltinQuery {
    name: "new_transaction.accounts",
    description: "The accounts that are open: opened, and not closed. Sorted by name.",
    bql: "SELECT account
FROM #accounts
WHERE open IS NOT NULL AND close IS NULL
ORDER BY account",
    params: &[],
};

/// The documents page.
pub const DOCUMENTS: BuiltinQuery = BuiltinQuery {
    name: "documents.all",
    description: "Every document of the ledger, newest first: the document directives and the documents transactions and \
                  postings name in their metadata, with the path to download them by.",
    bql: "SELECT date, time, path, account, transaction_id
FROM #documents
ORDER BY seq DESC",
    params: &[],
};

/// One page of the ledger's errors.
pub const ERRORS: BuiltinQuery = BuiltinQuery {
    name: "errors.page",
    description: "The ledger's errors, one page at a time, by file and then by position in the file.",
    bql: "SELECT id, kind, file, span_start, span_end, source, metas
FROM #errors
LIMIT :size OFFSET :offset",
    params: &[("size", DataType::Int), ("offset", DataType::Int)],
};

/// Every built-in query of this module.
pub const BUILTINS: &[&BuiltinQuery] = &[
    &JOURNAL,
    &JOURNAL_POSTINGS,
    &JOURNAL_BALANCE_CHECKS,
    &PAYEES,
    &OPEN_ACCOUNTS,
    &DOCUMENTS,
    &ERRORS,
];

/// The built-in queries, each compiled once.
static COMPILED: LazyLock<HashMap<&'static str, Query>> = LazyLock::new(|| {
    BUILTINS
        .iter()
        .map(|builtin| {
            let types = builtin.params.iter().fold(ParamTypes::new(), |types, (name, ty)| types.bind(*name, *ty));
            let query = Query::compile_with_params(builtin.bql, &types)
                .unwrap_or_else(|error| panic!("the built-in query {} does not compile: {:?}", builtin.name, error));
            (builtin.name, query)
        })
        .collect()
});

/// Run a built-in query over `ledger` with the limits of `/api/query`; `count_total` also counts
/// its rows before `LIMIT` and `OFFSET`.
fn run(ledger: &Ledger, builtin: &BuiltinQuery, params: &Params, count_total: bool) -> ServerResult<QueryResult> {
    let query = COMPILED.get(builtin.name).expect("every built-in query is compiled");
    let options = ExecuteOptions {
        count_total,
        ..execute_options(max_result_values())
    };
    Ok(query.execute_with_options(ledger, params, &options)?)
}

/// Run `work` over the ledger under one read guard, off the async workers: the queries of one
/// response see the same ledger.
async fn with_ledger<T: Send + 'static>(ledger: &SharedLedger, work: impl FnOnce(&Ledger) -> ServerResult<T> + Send + 'static) -> ServerResult<T> {
    let guard = ledger.0.clone().read_owned().await;
    tokio::task::spawn_blocking(move || work(&guard)).await?
}

/// The columns of a result by name.
struct Columns(HashMap<String, usize>);

impl Columns {
    fn of(result: &QueryResult) -> Columns {
        Columns(result.columns.iter().enumerate().map(|(idx, column)| (column.name.clone(), idx)).collect())
    }

    fn get<'r>(&self, row: &'r [Value], name: &str) -> &'r Value {
        &row[*self.0.get(name).unwrap_or_else(|| panic!("the built-in query selects {}", name))]
    }
}

fn string(value: &Value) -> String {
    value.as_str().unwrap_or_default().to_owned()
}

fn optional_string(value: &Value) -> Option<String> {
    value.as_str().map(str::to_owned)
}

fn strings(value: &Value) -> Vec<String> {
    value.as_set().map(|set| set.iter().cloned().collect()).unwrap_or_default()
}

fn metas(value: &Value) -> Vec<MetaEntity> {
    value
        .as_metas()
        .unwrap_or_default()
        .iter()
        .map(|(key, value)| MetaEntity {
            key: key.clone(),
            value: value.clone(),
        })
        .collect()
}

fn amount(value: &Value) -> Option<Amount> {
    value.as_amount().cloned()
}

/// The date and time of a row, from its `date` and `time` (`HH:MM:SS`) columns.
fn datetime(date: &Value, time: &Value) -> NaiveDateTime {
    let date = date.as_date().unwrap_or(NaiveDate::MIN);
    let time = time
        .as_str()
        .and_then(|time| NaiveTime::parse_from_str(time, "%H:%M:%S").ok())
        .unwrap_or(NaiveTime::MIN);
    date.and_time(time)
}

/// A `seq` as the `sequence` of a journal item.
fn sequence(value: &Value) -> i32 {
    value.as_int().and_then(|seq| i32::try_from(seq).ok()).unwrap_or(i32::MAX)
}

/// The page, the page size and the offset of a paged request: a size of 0, or a page whose rows
/// lie beyond what an offset can count, is a bad request.
pub fn page_window(params: &JournalRequest) -> ServerResult<(u32, u32, i64)> {
    let page = params.page();
    let size = params.limit();
    if size == 0 {
        return Err(ServerError::InvalidInput("the page size must be at least 1".to_owned()));
    }
    let offset = u64::from(page - 1) * u64::from(size);
    match i64::try_from(offset) {
        Ok(offset) if offset.checked_add(i64::from(size)).is_some() => Ok((page, size, offset)),
        _ => Err(ServerError::InvalidInput(format!("page {} of {} rows is out of range", page, size))),
    }
}

fn total(result: &QueryResult) -> u32 {
    result.total.map_or(0, |total| u32::try_from(total).unwrap_or(u32::MAX))
}

// ------------------------------------------------------------------------------------------------
// the journal

/// The parameters of [`JOURNAL`] for a request. An empty keyword searches nothing.
fn journal_params(params: &JournalRequest, size: u32, offset: i64) -> Params {
    let set = |items: &Option<std::collections::HashSet<String>>| items.as_ref().map(|items| items.iter().cloned().collect::<BTreeSet<_>>());
    Params::new()
        .bind("keyword", params.keyword.clone().filter(|keyword| !keyword.is_empty()))
        .bind("tags", set(&params.tags))
        .bind("links", set(&params.links))
        .bind("size", i64::from(size))
        .bind("offset", offset)
}

/// `GET /api/journals`: one page of the journal.
pub async fn journal(ledger: &SharedLedger, params: JournalRequest) -> ServerResult<Pageable<JournalItemEntity>> {
    let (page, size, offset) = page_window(&params)?;
    let query_params = journal_params(&params, size, offset);
    with_ledger(ledger, move |ledger| {
        let page_rows = run(ledger, &JOURNAL, &query_params, true)?;
        let records = journal_items(ledger, &page_rows)?;
        Ok(Pageable::new(total(&page_rows), page, size, records))
    })
    .await
}

/// The journal items of the rows of a [`JOURNAL`] page, in their order: their postings and
/// balance checks are read with [`JOURNAL_POSTINGS`] and [`JOURNAL_BALANCE_CHECKS`].
fn journal_items(ledger: &Ledger, page: &QueryResult) -> ServerResult<Vec<JournalItemEntity>> {
    let columns = Columns::of(page);
    let ids_of = |kind: &str| -> BTreeSet<String> {
        page.rows
            .iter()
            .filter(|row| columns.get(row, "type").as_str() == Some(kind))
            .filter_map(|row| optional_string(columns.get(row, "id")))
            .collect()
    };
    let transaction_ids = ids_of("transaction");
    let balance_ids = ids_of("balance");

    let mut postings: HashMap<String, Vec<PostingRow>> = HashMap::new();
    if !transaction_ids.is_empty() {
        let result = run(ledger, &JOURNAL_POSTINGS, &Params::new().bind("ids", transaction_ids), false)?;
        let posting_columns = Columns::of(&result);
        for row in &result.rows {
            let posting = PostingRow::of(&posting_columns, row);
            postings.entry(string(posting_columns.get(row, "id"))).or_default().push(posting);
        }
    }
    let mut checks: HashMap<String, BalanceCheckRow> = HashMap::new();
    if !balance_ids.is_empty() {
        let result = run(ledger, &JOURNAL_BALANCE_CHECKS, &Params::new().bind("ids", balance_ids), false)?;
        let check_columns = Columns::of(&result);
        for row in &result.rows {
            checks.insert(string(check_columns.get(row, "id")), BalanceCheckRow::of(&check_columns, row));
        }
    }

    Ok(page
        .rows
        .iter()
        .map(|row| {
            let entry = EntryRow::of(&columns, row);
            let postings = postings.remove(&entry.id).unwrap_or_default();
            let check = checks.remove(&entry.id);
            journal_item(entry, postings, check)
        })
        .collect())
}

/// A row of [`JOURNAL`]: a transaction or a balance assertion.
struct EntryRow {
    id: String,
    seq: i32,
    kind: String,
    datetime: NaiveDateTime,
    flag: Option<String>,
    payee: Option<String>,
    narration: Option<String>,
    tags: Vec<String>,
    links: Vec<String>,
    metas: Vec<MetaEntity>,
}

impl EntryRow {
    fn of(columns: &Columns, row: &[Value]) -> EntryRow {
        EntryRow {
            id: string(columns.get(row, "id")),
            seq: sequence(columns.get(row, "seq")),
            kind: string(columns.get(row, "type")),
            datetime: datetime(columns.get(row, "date"), columns.get(row, "time")),
            flag: optional_string(columns.get(row, "flag")),
            payee: optional_string(columns.get(row, "payee")),
            narration: optional_string(columns.get(row, "narration")),
            tags: strings(columns.get(row, "tags")),
            links: strings(columns.get(row, "links")),
            metas: metas(columns.get(row, "metas")),
        }
    }
}

/// A row of [`JOURNAL_POSTINGS`]: one posting as written.
struct PostingRow {
    balanced: bool,
    posting: JournalTransactionPostingEntity,
}

impl PostingRow {
    fn of(columns: &Columns, row: &[Value]) -> PostingRow {
        let currency = string(columns.get(row, "currency"));
        let in_currency = |name| Amount::new(columns.get(row, name).as_decimal().unwrap_or_default(), currency.clone());
        let units = in_currency("number");
        let cost = match (columns.get(row, "cost_number").as_decimal(), columns.get(row, "cost_currency").as_str()) {
            (Some(number), Some(currency)) => Some(Amount::new(number, currency)),
            _ => None,
        };
        PostingRow {
            balanced: columns.get(row, "balanced").as_bool().unwrap_or(true),
            posting: JournalTransactionPostingEntity {
                account: string(columns.get(row, "account")),
                unit: (!columns.get(row, "automatic").as_bool().unwrap_or(false)).then(|| units.clone()),
                cost,
                inferred_unit: units,
                account_before: in_currency("balance_before"),
                account_after: in_currency("balance_after"),
                metas: metas(columns.get(row, "metas")),
            },
        }
    }
}

/// A row of [`JOURNAL_BALANCE_CHECKS`].
struct BalanceCheckRow {
    account: String,
    tolerance: Option<BigDecimal>,
    actual: Option<Amount>,
    difference: Option<Amount>,
    asserted: Option<Amount>,
    amount: Option<Amount>,
    passed: bool,
}

impl BalanceCheckRow {
    fn of(columns: &Columns, row: &[Value]) -> BalanceCheckRow {
        BalanceCheckRow {
            account: string(columns.get(row, "account")),
            tolerance: columns.get(row, "tolerance").as_decimal(),
            actual: amount(columns.get(row, "actual")),
            difference: amount(columns.get(row, "difference")),
            asserted: amount(columns.get(row, "asserted")),
            amount: amount(columns.get(row, "amount")),
            passed: columns.get(row, "passed").as_bool().unwrap_or(false),
        }
    }
}

/// The journal item of a [`JOURNAL`] row, with the postings of a transaction or the check of a
/// balance assertion. This is the only place journal items are built:
///
/// - a `balance` entry (`balance`, or `balance ... with pad`) is a `BalanceCheck` item: payee
///   `Balance Check`, its account as the narration, and one entry that describes the check, the
///   balance before (`account_before`), the asserted amount (`account_after`) and their
///   difference (`unit`, `inferred_unit`), as zhang's balance check found them (`#balances`);
/// - a padding transaction (flag `P`) is a `BalancePad` item, with its postings;
/// - any other transaction is a `Transaction` item.
fn journal_item(entry: EntryRow, postings: Vec<PostingRow>, check: Option<BalanceCheckRow>) -> JournalItemEntity {
    let id = Uuid::from_str(&entry.id).unwrap_or_default();
    if entry.kind == "balance" {
        let check = check.unwrap_or(BalanceCheckRow {
            account: String::new(),
            tolerance: None,
            actual: None,
            difference: None,
            asserted: None,
            amount: None,
            passed: false,
        });
        let asserted = check.asserted.or(check.amount).unwrap_or_else(|| Amount::new(BigDecimal::from(0), ""));
        let zero = || Amount::new(BigDecimal::from(0), asserted.commodity.clone());
        let difference = check.difference.unwrap_or_else(zero);
        return JournalItemEntity::BalanceCheck(JournalBalanceCheckItemEntity {
            id,
            sequence: entry.seq,
            datetime: entry.datetime,
            payee: BALANCE_CHECK_PAYEE.to_owned(),
            narration: Some(check.account.clone()),
            type_: Flag::BalanceCheck.to_string(),
            postings: vec![JournalTransactionPostingEntity {
                account: check.account,
                unit: Some(difference.clone()),
                cost: None,
                inferred_unit: difference,
                account_before: check.actual.unwrap_or_else(zero),
                account_after: asserted,
                metas: vec![],
            }],
            tolerance: check.tolerance,
            passed: check.passed,
        });
    }
    let is_balanced = postings.iter().all(|posting| posting.balanced);
    let postings = postings.into_iter().map(|posting| posting.posting).collect_vec();
    let flag = entry.flag.unwrap_or_else(|| Flag::Okay.to_string());
    if flag == Flag::BalancePad.to_string() {
        return JournalItemEntity::BalancePad(JournalBalanceItemEntity {
            id,
            sequence: entry.seq,
            datetime: entry.datetime,
            payee: entry.payee.unwrap_or_default(),
            narration: entry.narration,
            type_: flag,
            postings,
        });
    }
    JournalItemEntity::Transaction(JournalTransactionItemEntity {
        id,
        sequence: entry.seq,
        datetime: entry.datetime,
        payee: entry.payee.unwrap_or_default(),
        narration: entry.narration,
        tags: entry.tags,
        links: entry.links,
        flag,
        is_balanced,
        postings,
        metas: entry.metas,
    })
}

// ------------------------------------------------------------------------------------------------
// the new-transaction form

/// `GET /api/for-new-transaction`: the payees and the open accounts.
pub async fn info_for_new_transaction(ledger: &SharedLedger) -> ServerResult<InfoForNewTransaction> {
    with_ledger(ledger, |ledger| {
        let first_column = |result: QueryResult| result.rows.iter().filter_map(|row| optional_string(&row[0])).collect_vec();
        Ok(InfoForNewTransaction {
            payee: first_column(run(ledger, &PAYEES, &Params::new(), false)?),
            account_name: first_column(run(ledger, &OPEN_ACCOUNTS, &Params::new(), false)?),
        })
    })
    .await
}

// ------------------------------------------------------------------------------------------------
// documents

/// `GET /api/documents`: every document, newest first.
pub async fn documents(ledger: &SharedLedger) -> ServerResult<Vec<DocumentEntity>> {
    with_ledger(ledger, |ledger| {
        let result = run(ledger, &DOCUMENTS, &Params::new(), false)?;
        let columns = Columns::of(&result);
        Ok(result
            .rows
            .iter()
            .map(|row| {
                let path = string(columns.get(row, "path"));
                DocumentEntity {
                    datetime: datetime(columns.get(row, "date"), columns.get(row, "time")),
                    filename: Path::new(&path).file_name().map(|it| it.to_string_lossy().into_owned()).unwrap_or_default(),
                    extension: mime_guess::from_path(&path).first().map(|it| it.to_string()),
                    account: optional_string(columns.get(row, "account")),
                    trx_id: optional_string(columns.get(row, "transaction_id")),
                    path,
                }
            })
            .collect())
    })
    .await
}

// ------------------------------------------------------------------------------------------------
// errors

/// `GET /api/errors`: one page of the ledger's errors.
pub async fn errors(ledger: &SharedLedger, params: JournalRequest) -> ServerResult<Pageable<ErrorEntity>> {
    let (page, size, offset) = page_window(&params)?;
    with_ledger(ledger, move |ledger| {
        let result = run(ledger, &ERRORS, &Params::new().bind("size", i64::from(size)).bind("offset", offset), true)?;
        let columns = Columns::of(&result);
        let records = result
            .rows
            .iter()
            .map(|row| {
                let position = |name| columns.get(row, name).as_int().and_then(|it| usize::try_from(it).ok());
                let span = position("span_start").map(|start| SpanInfoEntity {
                    start,
                    end: position("span_end").unwrap_or(start),
                    content: string(columns.get(row, "source")),
                    filename: optional_string(columns.get(row, "file")),
                });
                ErrorEntity {
                    id: string(columns.get(row, "id")),
                    span,
                    error_type: ErrorKind::from_str(columns.get(row, "kind").as_str().unwrap_or_default()).unwrap_or(ErrorKind::PluginError),
                    metas: columns.get(row, "metas").as_metas().unwrap_or_default().iter().cloned().collect(),
                }
            })
            .collect();
        Ok(Pageable::new(total(&result), page, size, records))
    })
    .await
}

#[cfg(test)]
mod test {
    use std::collections::BTreeSet;

    use super::{page_window, BUILTINS, COMPILED};
    use crate::request::JournalRequest;

    /// Every built-in query compiles, and declares exactly the parameters its BQL uses.
    #[test]
    fn every_builtin_compiles_with_the_parameters_it_declares() {
        for builtin in BUILTINS {
            assert!(COMPILED.contains_key(builtin.name), "{}", builtin.name);
            let used: BTreeSet<&str> = builtin
                .bql
                .match_indices(':')
                .map(|(at, _)| {
                    let name = &builtin.bql[at + 1..];
                    &name[..name.find(|it: char| !(it.is_ascii_alphanumeric() || it == '_')).unwrap_or(name.len())]
                })
                .filter(|name| !name.is_empty())
                .collect();
            let declared: BTreeSet<&str> = builtin.params.iter().map(|(name, _)| *name).collect();
            assert_eq!(used, declared, "{}", builtin.name);
        }
        let names: BTreeSet<&str> = BUILTINS.iter().map(|it| it.name).collect();
        assert_eq!(names.len(), BUILTINS.len(), "built-in names are unique");
    }

    fn request(page: Option<u32>, size: Option<u32>) -> JournalRequest {
        JournalRequest {
            page,
            size,
            keyword: None,
            tags: None,
            links: None,
        }
    }

    #[test]
    fn pages_are_windows_of_the_rows() {
        assert_eq!(page_window(&request(None, None)).unwrap(), (1, 100, 0));
        assert_eq!(page_window(&request(Some(0), Some(10))).unwrap(), (1, 10, 0));
        assert_eq!(page_window(&request(Some(3), Some(50))).unwrap(), (3, 50, 100));
        // past the end of a u32, which the old journal wrapped around
        assert_eq!(page_window(&request(Some(42949674), Some(100))).unwrap(), (42949674, 100, 4294967300));
        assert_eq!(
            page_window(&request(Some(1 << 31), Some(u32::MAX))).unwrap().2,
            i64::from((1u32 << 31) - 1) * i64::from(u32::MAX)
        );
    }

    #[test]
    fn an_empty_page_size_and_a_page_no_offset_can_count_are_bad_requests() {
        assert!(page_window(&request(Some(1), Some(0))).is_err());
        assert!(page_window(&request(Some(u32::MAX), Some(u32::MAX))).is_err());
    }
}
