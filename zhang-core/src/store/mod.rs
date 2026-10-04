use std::collections::{BTreeMap, HashMap, HashSet};

use bigdecimal::BigDecimal;
use chrono::{DateTime, NaiveDate};
use chrono_tz::Tz;
#[cfg(feature = "openapi")]
use gotcha_core::Schematic;
use indexmap::IndexMap;
use uuid::Uuid;
use zhang_ast::amount::Amount;
use zhang_ast::{Account, Flag, Meta, SpanInfo};

use crate::domains::schemas::{AccountDomain, CommodityDomain, ErrorDomain, MetaDomain, PriceDomain, QueryDomain};

#[derive(Default, serde::Serialize)]
pub struct Store {
    pub options: HashMap<String, String>,
    pub accounts: HashMap<String, AccountDomain>,
    pub commodities: IndexMap<String, CommodityDomain>,
    pub transactions: HashMap<Uuid, TransactionDomain>,
    /// in insertion order. The load appends with `Store::push_posting`, which keeps the index of
    /// `Store::last_posting_at` in step; nothing removes or reorders a posting
    pub postings: Vec<PostingDomain>,
    /// per account and commodity of `after_amount`, the positions in [`Store::postings`] of its postings, sorted by
    /// `(trx_datetime, position)`. A posting pushed to `postings` directly, not with [`Store::push_posting`], is not
    /// in it
    #[serde(skip)]
    posting_index: HashMap<String, HashMap<String, Vec<usize>>>,

    /// the `balance` assertions, in ledger order. They are not transactions and have no postings:
    /// an assertion changes no balance
    pub balance_assertions: Vec<BalanceAssertionDomain>,
    /// the ids of [`Store::balance_assertions`], which an id given to a transaction or an assertion avoids
    #[serde(skip)]
    pub(crate) balance_assertion_ids: HashSet<Uuid>,

    pub prices: Vec<PriceDomain>,

    pub budgets: HashMap<String, BudgetDomain>,

    // by account
    pub commodity_lots: HashMap<String, Vec<CommodityLotRecord>>,

    pub documents: Vec<DocumentDomain>,

    /// saved queries from `query` directives, in ledger order (by date, then source order)
    pub queries: Vec<QueryDomain>,

    pub metas: Vec<MetaDomain>,

    pub errors: Vec<ErrorDomain>,
}

impl Store {
    /// append a posting to [`Store::postings`] and index it. Every indexed position is lower than the new one, so the
    /// new position goes after those dated on or before the posting and before those dated after it, which keeps
    /// the positions sorted by `(trx_datetime, position)` when postings come out of datetime order
    pub(crate) fn push_posting(&mut self, posting: PostingDomain) {
        let position = self.postings.len();
        let datetime = posting.trx_datetime;
        let positions = self
            .posting_index
            .entry(posting.account.name().to_owned())
            .or_default()
            .entry(posting.after_amount.commodity.clone())
            .or_default();
        self.postings.push(posting);
        let postings = &self.postings;
        positions.insert(positions.partition_point(|&it| postings[it].trx_datetime <= datetime), position);
    }

    /// of the postings of `account` whose `after_amount` is in `commodity`, the latest dated on or before `datetime`,
    /// and of several dated the same, the last inserted: the last of a stable sort by datetime
    pub(crate) fn last_posting_at(&self, account: &str, commodity: &str, datetime: DateTime<Tz>) -> Option<&PostingDomain> {
        let positions = self.posting_index.get(account)?.get(commodity)?;
        let on_or_before = positions.partition_point(|&it| self.postings[it].trx_datetime <= datetime);
        on_or_before.checked_sub(1).map(|it| &self.postings[positions[it]])
    }
}

#[derive(Clone, serde::Serialize, Debug)]
pub struct TransactionDomain {
    pub id: Uuid,
    pub sequence: i32,
    pub datetime: DateTime<Tz>,
    pub flag: Flag,
    pub payee: Option<String>,
    pub narration: Option<String>,
    pub span: SpanInfo,
    pub tags: Vec<String>,
    pub links: Vec<String>,
    pub postings: Vec<PostingDomain>,
}

/// A `balance` assertion as the load checked it: the asserted amount next to the account's balance
/// where the assertion stands.
///
/// It records a check, it is not a transaction: it has no postings, so nothing that sums postings
/// books it, and no balance depends on it. A failing assertion is reported as an
/// `AccountBalanceCheckError` and changes no number; `balance ... with pad` (a `P` transaction)
/// is how a balance is corrected on purpose.
#[derive(Clone, serde::Serialize, Debug)]
pub struct BalanceAssertionDomain {
    /// derived from the span of the `balance` directive, like a transaction id
    pub id: Uuid,
    /// its place in the journal: assertions and transactions share one sequence
    pub sequence: i32,
    pub datetime: DateTime<Tz>,
    pub account: Account,
    /// the asserted amount
    pub amount: Amount,
    /// the explicit tolerance (`~`); `None` asserts the exact amount
    pub tolerance: Option<BigDecimal>,
    /// the account's balance in the asserted currency where the assertion stands: the sum of the
    /// postings of the account and all its sub-accounts before it, as in beancount
    pub balance: Amount,
    /// whether `balance` is within `tolerance` of `amount`
    pub passed: bool,
    pub span: SpanInfo,
}

#[derive(Clone, serde::Serialize, Debug)]
pub struct PostingDomain {
    pub id: Uuid,
    pub trx_id: Uuid,
    pub trx_sequence: i32,
    pub trx_datetime: DateTime<Tz>,
    pub account: Account,
    pub unit: Option<Amount>,
    pub cost: Option<Amount>,
    pub inferred_amount: Amount,
    pub previous_amount: Amount,
    pub after_amount: Amount,
    /// metadata of the posting, sorted by key (the values of a repeated key in ledger
    /// order). The transaction's own metadata is in [`Store::metas`].
    pub metas: Vec<PostingMetaDomain>,
}

/// One metadata entry of a posting, its value as plain text like [`MetaDomain`]'s.
#[derive(Clone, serde::Serialize, Debug, PartialEq, Eq)]
pub struct PostingMetaDomain {
    pub key: String,
    pub value: String,
}

impl PostingMetaDomain {
    /// The entries of `meta`, sorted by key; the values of a repeated key keep their order.
    pub fn of(meta: Meta) -> Vec<PostingMetaDomain> {
        let mut metas = meta
            .get_flatten()
            .into_iter()
            .map(|(key, value)| PostingMetaDomain {
                key,
                value: value.to_plain_string(),
            })
            .collect::<Vec<_>>();
        metas.sort_by(|a, b| a.key.cmp(&b.key));
        metas
    }
}

#[derive(Clone, serde::Serialize)]
pub enum DocumentType {
    Trx(Uuid),
    Account(Account),
}

impl DocumentType {
    pub fn match_account(&self, account_name: &str) -> bool {
        match self {
            DocumentType::Trx(_) => false,
            DocumentType::Account(acc) => acc.name().eq(account_name),
        }
    }
    pub fn as_account(&self) -> Option<String> {
        match self {
            DocumentType::Trx(_) => None,
            DocumentType::Account(account) => Some(account.name().to_owned()),
        }
    }
    pub fn as_trx(&self) -> Option<String> {
        match self {
            DocumentType::Trx(id) => Some(id.to_string()),
            DocumentType::Account(_) => None,
        }
    }
}

#[derive(Clone, serde::Serialize)]
pub struct DocumentDomain {
    pub datetime: DateTime<Tz>,
    pub document_type: DocumentType,
    pub filename: Option<String>,
    pub path: String,
    /// where the document may be instead, looked at when nothing is at `path`: a `document` of a beancount ledger on
    /// a remote source, whose files are not looked at on load, may be at its path relative to the ledger's root, as
    /// earlier versions wrote it, rather than relative to its file, as beancount reads it
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alternate: Option<String>,
}

#[derive(Default, Clone, Debug, serde::Serialize, PartialEq)]
pub struct CommodityLotRecord {
    pub commodity: String,
    pub amount: BigDecimal,

    pub cost: Option<Amount>,

    // acquisition date
    pub acquisition_date: Option<NaiveDate>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct BudgetDomain {
    pub name: String,
    pub alias: Option<String>,
    pub category: Option<String>,
    pub closed: bool,
    pub detail: BTreeMap<u32, BudgetIntervalDetail>,
    pub commodity: String,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct BudgetIntervalDetail {
    /// year and month pair, calculated as `year*100+month`, E.G. `202312`
    pub date: u32,
    pub assigned_amount: Amount,
    // todo: budget event for addition, transfer and close
    pub events: Vec<BudgetEvent>,
    pub activity_amount: Amount,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct BudgetEvent {
    pub datetime: DateTime<Tz>,
    pub timestamp: i64,
    pub amount: Amount,
    pub event_type: BudgetEventType,
}

#[derive(Clone, Debug, serde::Serialize)]
#[cfg_attr(feature = "openapi", derive(Schematic))]
pub enum BudgetEventType {
    AddAssignedAmount,
    Transfer,
}

#[cfg(test)]
mod test {
    use std::str::FromStr;

    use uuid::uuid;
    use zhang_ast::Account;

    use crate::store::DocumentType;

    #[test]
    fn should_match_document_type() {
        let document_type = DocumentType::Trx(uuid!("67e55044-10b1-426f-9247-bb680e5fe0c8"));
        assert!(!document_type.match_account("any"));

        let account_type = DocumentType::Account(Account::from_str("Assets:A").unwrap());

        assert!(account_type.match_account("Assets:A"));
        assert!(!account_type.match_account("Assets:A:B"));
        assert!(!account_type.match_account("Assets:C"));
    }

    #[test]
    fn should_return_account() {
        let document_type = DocumentType::Trx(uuid!("67e55044-10b1-426f-9247-bb680e5fe0c8"));
        assert_eq!(None, document_type.as_account());

        let account_type = DocumentType::Account(Account::from_str("Assets:A").unwrap());
        assert_eq!(account_type.as_account(), Some("Assets:A".to_owned()));
    }

    #[test]
    fn should_return_trx() {
        let uuid = uuid!("67e55044-10b1-426f-9247-bb680e5fe0c8");
        let document_type = DocumentType::Trx(uuid);
        assert_eq!(Some(uuid.to_string()), document_type.as_trx());

        let account_type = DocumentType::Account(Account::from_str("Assets:A").unwrap());
        assert_eq!(account_type.as_trx(), None);
    }
}

#[cfg(test)]
mod posting_index_test {
    use bigdecimal::{BigDecimal, Zero};
    use chrono::{DateTime, Duration, TimeZone};
    use chrono_tz::Tz;
    use itertools::Itertools;
    use uuid::Uuid;
    use zhang_ast::amount::Amount;
    use zhang_ast::{Flag, Meta, SpanInfo};

    use crate::domains::Operations;
    use crate::store::Store;

    /// the day balance as found before the postings were indexed: scan every posting, keep those of the account
    /// and commodity dated on or before `datetime`, sort them by datetime (a stable sort) and take the last
    fn scan(store: &Store, account: &str, commodity: &str, datetime: DateTime<Tz>) -> Option<BigDecimal> {
        store
            .postings
            .iter()
            .filter(|posting| posting.account.name() == account)
            .filter(|posting| posting.after_amount.commodity == commodity)
            .filter(|posting| posting.trx_datetime <= datetime)
            .sorted_by_key(|posting| posting.trx_datetime)
            .next_back()
            .map(|posting| posting.after_amount.number.clone())
    }

    /// After every transaction, the indexed lookup finds the posting a scan of every posting finds, at every datetime
    /// of the ledger and between them. The transactions come out of datetime order, many postings share a datetime
    /// (also within one transaction), and they spread over an account, its sub-account and another account, each in
    /// several commodities. Every posting has its own number, so a lookup that finds another posting fails
    #[test]
    fn the_day_balance_lookup_finds_what_a_scan_of_every_posting_finds() {
        let timezone = Tz::Asia__Shanghai;
        let mut operations = Operations {
            timezone,
            store: Default::default(),
        };
        let accounts = ["Assets:Bank", "Assets:Bank:Card", "Expenses:Food"];
        let commodities = ["CNY", "USD", "BTC"];
        // a few datetimes, two of them a second apart, so that many postings share one
        let datetimes = [(1, 1, 0, 0), (1, 1, 0, 1), (1, 2, 12, 0), (2, 1, 0, 0), (3, 1, 0, 0)]
            .map(|(month, day, hour, second)| timezone.with_ymd_and_hms(2024, month, day, hour, 0, second).unwrap());
        // every datetime, an hour before and an hour after it: before all, between and after all of them
        let targets = datetimes
            .iter()
            .flat_map(|datetime| [*datetime - Duration::hours(1), *datetime, *datetime + Duration::hours(1)])
            .collect_vec();

        // xorshift: the same sequence on every run
        let mut state = 0x2545_f491_4f6c_dd1d_u64;
        let mut below = |bound: usize| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state % bound as u64) as usize
        };
        let mut latest = datetimes[0];
        let mut out_of_order = 0;
        for sequence in 0..120 {
            let id = Uuid::from_u128(sequence as u128 + 1);
            let datetime = datetimes[below(datetimes.len())];
            if datetime < latest {
                out_of_order += 1;
            }
            latest = latest.max(datetime);
            operations
                .insert_transaction(&id, sequence, datetime, Flag::Okay, None, None, vec![], vec![], &SpanInfo::default())
                .unwrap();
            for posting_idx in 0..1 + below(3) {
                let account = accounts[below(accounts.len())];
                let commodity = commodities[below(commodities.len())];
                let zero = Amount::new(BigDecimal::zero(), commodity);
                let after = Amount::new(BigDecimal::from(sequence * 10 + posting_idx as i32), commodity);
                operations
                    .insert_transaction_posting(&id, posting_idx, account, None, None, zero.clone(), zero, after, Meta::default())
                    .unwrap();
            }

            for account in accounts {
                for commodity in commodities.into_iter().chain(["EUR"]) {
                    for target in targets.iter().copied() {
                        let found = operations.account_target_day_balance(account, target, commodity).unwrap();
                        let scanned = scan(&operations.read(), account, commodity, target);
                        assert_eq!(
                            found.map(|it| it.number),
                            scanned,
                            "{account} in {commodity} at {target}, after transaction {sequence}"
                        );
                    }
                }
            }
        }

        let store = operations.read();
        let sharing_a_datetime = store
            .postings
            .iter()
            .map(|posting| (posting.account.name(), &posting.after_amount.commodity, posting.trx_datetime))
            .duplicates()
            .count();
        assert!(out_of_order > 0, "some transactions come out of datetime order");
        assert!(sharing_a_datetime > 0, "some postings of an account and commodity share a datetime");
    }
}
