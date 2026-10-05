//! The journal, the suggestions of the new-transaction form, the documents and the errors of the
//! ledger as built-in queries of the query engine (#479): every response is one or more named BQL
//! queries plus a thin mapping into its existing shape.
//!
//! The queries are in the registry of built-in queries ([`crate::builtin`]), under `journals.`.
//! User input is only ever bound as parameters, never formatted into a query. The queries of one
//! response run under one read lock of the ledger, so they see the same ledger.

use std::collections::{BTreeSet, HashMap};
use std::str::FromStr;

use bigdecimal::BigDecimal;
use chrono::{DateTime, NaiveDateTime, Utc};
use itertools::Itertools;
use uuid::Uuid;
use zhang_ast::amount::Amount;
use zhang_ast::error::ErrorKind;
use zhang_ast::{Account, Directive, Flag, SpanInfo, Transaction};
use zhang_core::constants::BALANCE_CHECK_PAYEE;
use zhang_core::data_type::text::exporter::ZhangDataTypeExportable;
use zhang_core::data_type::text::parser::transaction_has_unexported_text;
use zhang_core::data_type::Dialect;
use zhang_core::ledger::Ledger;
use zhang_core::pipeline::AccountUse;
use zhang_core::utils::string_::QuoteStyle;
use zhang_query::{Params, QueryResult, Value};

use crate::builtin::{bind_instant, execute, ledger_now};
use crate::cells::{self, Row};
use crate::error::ServerError;
use crate::request::JournalRequest;
use crate::response::{
    DocumentEntity, ErrorEntity, InfoForNewDocument, InfoForNewTransaction, JournalBalanceCheckItemEntity, JournalBalanceItemEntity, JournalItemEntity,
    JournalTransactionItemEntity, JournalTransactionPostingEntity, MetaEntity, Pageable, SpanInfoEntity, WrittenPostingEntity,
};
use crate::routes::query::with_ledger;
use crate::state::SharedLedger;
use crate::ServerResult;

/// The built-in queries of the journal ([`crate::builtin::BUILTINS`]).
pub const JOURNAL: &str = "journals.page";
pub const JOURNAL_POSTINGS: &str = "journals.postings";
pub const JOURNAL_BALANCE_CHECKS: &str = "journals.balance_checks";
pub const PAYEES: &str = "journals.payees";
pub const OPEN_ACCOUNTS: &str = "journals.accounts";
/// the accounts with an `open` or `close` directive
const ACCOUNTS: &str = "accounts.list";
pub const DOCUMENTS: &str = "journals.documents";
pub const ERRORS: &str = "journals.errors";

/// A `set` cell as a list, in the set's order.
fn strings(set: Option<BTreeSet<String>>) -> Vec<String> {
    set.map(|set| set.into_iter().collect()).unwrap_or_default()
}

/// A `metas` cell as the response's metadata entries.
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

/// A `seq` as the `sequence` of a journal item.
fn sequence(seq: Option<i64>) -> i32 {
    seq.and_then(|seq| i32::try_from(seq).ok()).unwrap_or(i32::MAX)
}

/// The largest page a paged endpoint (`/api/journals`, `/api/errors`) returns.
pub const MAX_PAGE_SIZE: u32 = 1000;

/// The page, the page size and the offset of a paged request. A size outside 1 to
/// [`MAX_PAGE_SIZE`] is a bad request; a page past the last one is empty.
pub fn page_window(params: &JournalRequest) -> ServerResult<(u32, u32, i64)> {
    let page = params.page();
    let size = params.limit();
    if !(1..=MAX_PAGE_SIZE).contains(&size) {
        return Err(ServerError::InvalidInput(format!("size must be between 1 and {}", MAX_PAGE_SIZE)));
    }
    let offset = window_offset(page, size).ok_or_else(|| ServerError::InvalidInput(format!("page {} of {} rows is out of range", page, size)))?;
    Ok((page, size, offset))
}

/// The rows before page `page` (from 1) of `size` rows, when the page ends where the query engine
/// can count (within 64 bits); `None` otherwise. Within [`MAX_PAGE_SIZE`] it always does.
fn window_offset(page: u32, size: u32) -> Option<i64> {
    let offset = i64::try_from(u64::from(page.max(1) - 1) * u64::from(size)).ok()?;
    offset.checked_add(i64::from(size)).map(|_| offset)
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
    with_ledger(&ledger.0, move |ledger| {
        let page_rows = execute(ledger, JOURNAL, &query_params, true)?;
        let records = journal_items(ledger, &page_rows)?;
        Ok(Pageable::new(total(&page_rows), page, size, records))
    })
    .await
}

/// The journal items of the rows of a [`JOURNAL`] page, in their order: their postings and
/// balance checks are read with [`JOURNAL_POSTINGS`] and [`JOURNAL_BALANCE_CHECKS`].
fn journal_items(ledger: &Ledger, page: &QueryResult) -> ServerResult<Vec<JournalItemEntity>> {
    let ids_of = |kind: &str| -> ServerResult<BTreeSet<String>> {
        let mut ids = BTreeSet::new();
        for row in cells::rows(JOURNAL, page) {
            if row.str("type")?.as_deref() == Some(kind) {
                ids.extend(row.str("id")?);
            }
        }
        Ok(ids)
    };
    let transaction_ids = ids_of("transaction")?;
    let balance_ids = ids_of("balance")?;

    // as written: tags and links in their order, which the engine's sets sort and the edit form writes back as
    // listed, a narration that is absent, which the engine reads as '' as beancount does, and the cost, price and
    // comment of each posting line, which the engine holds booked
    let style = match ledger.dialect {
        Dialect::Beancount => QuoteStyle::Beancount,
        Dialect::Zhang => QuoteStyle::Zhang,
    };
    let written: HashMap<String, Written> = {
        let store = ledger
            .store
            .read()
            .map_err(|_| ServerError::InvalidInput("the ledger store is not readable".to_owned()))?;
        transaction_ids
            .iter()
            .filter_map(|id| {
                let transaction = store.transactions.get(&Uuid::from_str(id).ok()?)?;
                let written = Written {
                    narration: transaction.narration.clone(),
                    tags: transaction.tags.clone(),
                    links: transaction.links.clone(),
                    postings: written_transaction(ledger, &transaction.span).map(|directive| written_postings(directive, style)),
                    edit_drops_text: transaction_has_unexported_text(&transaction.span.content),
                };
                Some((id.clone(), written))
            })
            .collect()
    };
    let mut postings: HashMap<String, Vec<PostingRow>> = HashMap::new();
    if !transaction_ids.is_empty() {
        let result = execute(ledger, JOURNAL_POSTINGS, &Params::new().bind("ids", transaction_ids), false)?;
        for row in cells::rows(JOURNAL_POSTINGS, &result) {
            let posting = PostingRow::of(&row)?;
            postings.entry(row.str("id")?.unwrap_or_default()).or_default().push(posting);
        }
    }
    let mut checks: HashMap<String, BalanceAssertion> = HashMap::new();
    if !balance_ids.is_empty() {
        let result = execute(ledger, JOURNAL_BALANCE_CHECKS, &Params::new().bind("ids", balance_ids), false)?;
        for row in cells::rows(JOURNAL_BALANCE_CHECKS, &result) {
            checks.insert(row.str("id")?.unwrap_or_default(), BalanceAssertion::of(&row)?);
        }
    }

    cells::rows(JOURNAL, page)
        .map(|row| {
            let mut entry = EntryRow::of(&row)?;
            let mut postings = postings.remove(&entry.id).unwrap_or_default();
            if let Some(written) = written.get(&entry.id) {
                entry.narration = written.narration.clone();
                entry.tags = written.tags.clone();
                entry.links = written.links.clone();
                entry.edit_drops_text = written.edit_drops_text;
                // the postings of the journal are the written ones in their order ([`JOURNAL_POSTINGS`] groups the
                // booked legs by the posting they were written as); the written forms are attached only when they
                // line up with them, account by account
                if let Some(forms) = &written.postings {
                    let aligned = forms.len() == postings.len() && forms.iter().zip(&postings).all(|((account, _), row)| *account == row.posting.account);
                    if aligned {
                        for ((_, form), row) in forms.iter().zip(&mut postings) {
                            row.posting.written = Some(form.clone());
                        }
                    }
                }
            }
            let check = checks.remove(&entry.id);
            Ok(journal_item(entry, postings, check))
        })
        .collect()
}

/// What the journal shows of a transaction as it is written.
struct Written {
    narration: Option<String>,
    tags: Vec<String>,
    links: Vec<String>,
    /// the postings as written, each with its account, when the transaction's directive is in the ledger: not for
    /// one a plugin made
    postings: Option<Vec<(String, WrittenPostingEntity)>>,
    /// whether the text of the transaction has lines an edit drops ([`transaction_has_unexported_text`])
    edit_drops_text: bool,
}

/// The transaction directive of `ledger` read from `span`: the one starting where it starts, in its file. `None` for
/// a span in no file, such as a transaction a plugin made.
pub(crate) fn written_transaction<'a>(ledger: &'a Ledger, span: &SpanInfo) -> Option<&'a Transaction> {
    let file = span.filename.as_ref()?;
    ledger.directives.iter().find_map(|directive| match &directive.data {
        Directive::Transaction(transaction) if directive.span.start == span.start && directive.span.filename.as_ref() == Some(file) => Some(transaction),
        _ => None,
    })
}

/// The postings of `transaction` as written (#638: the ledger holds them booked), each with its account and the
/// cost, price and comment of its line in the ledger's syntax, as the exporter writes them in `style`.
fn written_postings(transaction: &Transaction, style: QuoteStyle) -> Vec<(String, WrittenPostingEntity)> {
    transaction
        .written_postings()
        .into_iter()
        .map(|posting| {
            let written = WrittenPostingEntity {
                cost: posting.cost.map(|cost| cost.export_as(style)),
                price: posting.price.map(|price| price.export_as(style)),
                comment: posting.comment,
            };
            (posting.account.name().to_owned(), written)
        })
        .collect()
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
    /// see [`Written::edit_drops_text`]; `false` until the written forms are read
    edit_drops_text: bool,
}

impl EntryRow {
    fn of(row: &Row<'_>) -> ServerResult<EntryRow> {
        Ok(EntryRow {
            id: row.str("id")?.unwrap_or_default(),
            seq: sequence(row.int("seq")?),
            kind: row.str("type")?.unwrap_or_default(),
            datetime: row.datetime("date", "time")?.unwrap_or_default(),
            flag: row.str("flag")?,
            payee: row.str("payee")?,
            narration: row.str("narration")?,
            tags: strings(row.set("tags")?),
            links: strings(row.set("links")?),
            metas: metas(row.get("metas")?),
            edit_drops_text: false,
        })
    }
}

/// A row of [`JOURNAL_POSTINGS`]: one posting as written.
struct PostingRow {
    balanced: bool,
    posting: JournalTransactionPostingEntity,
}

impl PostingRow {
    fn of(row: &Row<'_>) -> ServerResult<PostingRow> {
        let currency = row.str("currency")?.unwrap_or_default();
        let in_currency = |name| -> ServerResult<Amount> { Ok(Amount::new(row.decimal(name)?.unwrap_or_default(), currency.clone())) };
        let units = in_currency("number")?;
        // the cost of its lots when they all have the same; none when booking split it across lots of different costs
        let same = |min: &str, max: &str| -> ServerResult<bool> { Ok(row.get(min)? == row.get(max)?) };
        let every_lot_at_cost = same("lots", "lots_at_cost")?;
        let cost = match (row.decimal("cost_number")?, row.str("cost_currency")?) {
            (Some(number), Some(currency)) if every_lot_at_cost && same("cost_number", "max_cost_number")? && same("cost_currency", "max_cost_currency")? => {
                Some(Amount::new(number, currency))
            }
            _ => None,
        };
        Ok(PostingRow {
            balanced: row.bool("balanced")?.unwrap_or(true),
            posting: JournalTransactionPostingEntity {
                account: row.str("account")?.unwrap_or_default(),
                unit: (!row.bool("automatic")?.unwrap_or(false)).then(|| units.clone()),
                cost,
                inferred_unit: units,
                account_before: in_currency("balance_before")?,
                account_after: in_currency("balance_after")?,
                metas: metas(row.get("metas")?),
                written: None,
            },
        })
    }
}

/// A balance assertion as both journals describe it, the journal ([`JOURNAL_BALANCE_CHECKS`]) and an account's journal
/// (`accounts.balance_assertions`), from a row of the projection of `#balances` they share: one shape, with one meaning
/// of each figure.
pub(crate) struct BalanceAssertion {
    pub account: String,
    /// the asserted amount, as written
    pub asserted: Amount,
    /// the balance it was checked against: that of the account and all its sub-accounts where the assertion stands, in
    /// the asserted commodity
    pub checked_balance: Amount,
    /// the asserted amount minus the checked balance
    pub difference: Amount,
    /// the explicit tolerance (`~`) of the assertion; none for an exact one
    pub tolerance: Option<BigDecimal>,
    /// whether the checked balance is within the tolerance of the asserted amount, as the ledger checked it
    pub passed: bool,
}

impl BalanceAssertion {
    pub(crate) fn of(row: &Row<'_>) -> ServerResult<BalanceAssertion> {
        let asserted = row.amount("amount")?.unwrap_or_else(|| Amount::new(BigDecimal::from(0), ""));
        let zero = || Amount::new(BigDecimal::from(0), asserted.commodity.clone());
        let checked_balance = row.amount("actual")?.unwrap_or_else(zero);
        let difference = row
            .amount("difference")?
            .unwrap_or_else(|| Amount::new(&asserted.number - &checked_balance.number, asserted.commodity.clone()));
        Ok(BalanceAssertion {
            account: row.str("account")?.unwrap_or_default(),
            tolerance: row.decimal("tolerance")?,
            passed: row.bool("passed")?.unwrap_or(false),
            asserted,
            checked_balance,
            difference,
        })
    }
}

/// The journal item of a [`JOURNAL`] row, with the postings of a transaction or the check of a
/// balance assertion. This is the only place journal items are built:
///
/// - a `balance` entry (`balance`, or `balance ... with pad`) is a `BalanceCheck` item: payee
///   `Balance Check`, its account as the narration, and the [`BalanceAssertion`] as zhang's balance
///   check found it (`#balances`). Its one entry is no posting: the assertion books nothing, so the
///   balance before and after it is the checked balance, and it adds zero;
/// - a padding transaction (flag `P`) is a `BalancePad` item, with its postings;
/// - any other transaction is a `Transaction` item.
fn journal_item(entry: EntryRow, postings: Vec<PostingRow>, check: Option<BalanceAssertion>) -> JournalItemEntity {
    let id = Uuid::from_str(&entry.id).unwrap_or_default();
    if entry.kind == "balance" {
        let check = check.unwrap_or_else(|| {
            let nothing = Amount::new(BigDecimal::from(0), "");
            BalanceAssertion {
                account: String::new(),
                asserted: nothing.clone(),
                checked_balance: nothing.clone(),
                difference: nothing,
                tolerance: None,
                passed: false,
            }
        });
        let zero = Amount::new(BigDecimal::from(0), check.asserted.commodity.clone());
        return JournalItemEntity::BalanceCheck(JournalBalanceCheckItemEntity {
            id,
            sequence: entry.seq,
            datetime: entry.datetime,
            payee: BALANCE_CHECK_PAYEE.to_owned(),
            narration: Some(check.account.clone()),
            type_: Flag::BalanceCheck.to_string(),
            postings: vec![JournalTransactionPostingEntity {
                account: check.account,
                unit: None,
                cost: None,
                inferred_unit: zero,
                account_before: check.checked_balance.clone(),
                account_after: check.checked_balance.clone(),
                metas: vec![],
                written: None,
            }],
            asserted: check.asserted,
            checked_balance: check.checked_balance,
            difference: check.difference,
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
        edit_drops_text: entry.edit_drops_text,
    })
}

// ------------------------------------------------------------------------------------------------
// the new-transaction form

/// `GET /api/for-new-transaction`: the payees, and the accounts open at `at`, an instant read in the ledger's timezone,
/// or now.
pub async fn info_for_new_transaction(ledger: &SharedLedger, at: Option<DateTime<Utc>>) -> ServerResult<InfoForNewTransaction> {
    with_ledger(&ledger.0, move |ledger| {
        let first_column = |result: QueryResult| result.rows.iter().filter_map(|row| row[0].as_str().map(str::to_owned)).collect_vec();
        let at = match at {
            Some(at) => at.with_timezone(&ledger.options.timezone).naive_local(),
            None => ledger_now(ledger),
        };
        Ok(InfoForNewTransaction {
            payee: first_column(execute(ledger, PAYEES, &Params::new(), false)?),
            account_name: first_column(execute(ledger, OPEN_ACCOUNTS, &bind_instant(Params::new(), at), false)?),
        })
    })
    .await
}

// ------------------------------------------------------------------------------------------------
// documents

/// `GET /api/for-new-document`: the accounts a document written now may name, by the rule the ledger checks a `document`
/// with ([`AccountUse::Records`]): every account opened by now, closed ones included, but not one opened later or never.
/// The candidates are those of `accounts.list`, the accounts with an `open` or `close` directive.
pub async fn info_for_new_document(ledger: &SharedLedger) -> ServerResult<InfoForNewDocument> {
    with_ledger(&ledger.0, |ledger| {
        let now = ledger_now(ledger);
        let accounts = execute(ledger, ACCOUNTS, &bind_instant(Params::new(), now), false)?;
        let account_name = accounts
            .rows
            .iter()
            .filter_map(|row| row[0].as_str())
            .filter(|name| Account::from_str(name).is_ok_and(|account| ledger.account_reference_error(&account, now, AccountUse::Records).is_none()))
            .map(str::to_owned)
            .collect();
        Ok(InfoForNewDocument { account_name })
    })
    .await
}

/// `GET /api/documents`: every document, newest first.
pub async fn documents(ledger: &SharedLedger) -> ServerResult<Vec<DocumentEntity>> {
    with_ledger(&ledger.0, |ledger| {
        let result = execute(ledger, DOCUMENTS, &Params::new(), false)?;
        cells::rows(DOCUMENTS, &result).map(|row| DocumentEntity::of(&row)).collect()
    })
    .await
}

// ------------------------------------------------------------------------------------------------
// errors

/// `GET /api/errors`: one page of the ledger's errors.
pub async fn errors(ledger: &SharedLedger, params: JournalRequest) -> ServerResult<Pageable<ErrorEntity>> {
    let (page, size, offset) = page_window(&params)?;
    with_ledger(&ledger.0, move |ledger| {
        let result = execute(ledger, ERRORS, &Params::new().bind("size", i64::from(size)).bind("offset", offset), true)?;
        let records = cells::rows(ERRORS, &result)
            .map(|row| {
                let position = |name| -> ServerResult<Option<usize>> { Ok(row.int(name)?.and_then(|it| usize::try_from(it).ok())) };
                let span = match position("span_start")? {
                    Some(start) => Some(SpanInfoEntity {
                        start,
                        end: position("span_end")?.unwrap_or(start),
                        content: row.str("source")?.unwrap_or_default(),
                        filename: row.str("file")?,
                        line: position("line")?,
                        column: position("column")?,
                    }),
                    None => None,
                };
                Ok(ErrorEntity {
                    id: row.str("id")?.unwrap_or_default(),
                    span,
                    error_type: ErrorKind::from_str(row.str("kind")?.as_deref().unwrap_or_default()).unwrap_or(ErrorKind::PluginError),
                    metas: row.get("metas")?.as_metas().unwrap_or_default().iter().cloned().collect(),
                })
            })
            .collect::<ServerResult<Vec<_>>>()?;
        Ok(Pageable::new(total(&result), page, size, records))
    })
    .await
}

#[cfg(test)]
mod test {
    use super::{page_window, window_offset, MAX_PAGE_SIZE};
    use crate::request::JournalRequest;

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
    fn an_offset_must_leave_room_for_its_page() {
        assert_eq!(window_offset(1, 1), Some(0));
        assert_eq!(window_offset(u32::MAX, MAX_PAGE_SIZE), Some(i64::from(u32::MAX - 1) * i64::from(MAX_PAGE_SIZE)));
        // beyond a page size the endpoints accept: the last page whose end fits in 64 bits, one whose offset fits but
        // whose end does not, and one whose offset does not fit
        assert_eq!(window_offset(2_147_483_648, u32::MAX), Some(9_223_372_030_412_324_865));
        assert_eq!(window_offset(2_147_483_649, u32::MAX), None);
        assert_eq!(window_offset(2_147_483_650, u32::MAX), None);
        assert_eq!(window_offset(u32::MAX, u32::MAX), None);
    }

    #[test]
    fn pages_are_windows_of_the_rows() {
        assert_eq!(page_window(&request(None, None)).unwrap(), (1, 100, 0));
        assert_eq!(page_window(&request(Some(0), Some(10))).unwrap(), (1, 10, 0));
        assert_eq!(page_window(&request(Some(3), Some(50))).unwrap(), (3, 50, 100));
        // past the end of a u32, which the old journal wrapped around
        assert_eq!(page_window(&request(Some(42949674), Some(100))).unwrap(), (42949674, 100, 4294967300));
        assert_eq!(
            page_window(&request(Some(u32::MAX), Some(MAX_PAGE_SIZE))).unwrap(),
            (u32::MAX, MAX_PAGE_SIZE, 4_294_967_294_000)
        );
    }

    #[test]
    fn a_page_size_outside_1_to_1000_is_a_bad_request() {
        for size in [0, MAX_PAGE_SIZE + 1, u32::MAX] {
            let error = page_window(&request(Some(1), Some(size))).unwrap_err();
            assert_eq!(error.to_string(), "size must be between 1 and 1000", "{size}");
        }
        assert!(page_window(&request(Some(1), Some(1))).is_ok());
        assert!(page_window(&request(Some(1), Some(MAX_PAGE_SIZE))).is_ok());
    }
}
